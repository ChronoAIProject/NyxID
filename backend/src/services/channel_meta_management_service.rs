//! Meta management policy and customer setup orchestration. HTTP is owned by handlers.
use super::{
    channel_meta_service::{self, MetaService, Started},
    nyxbot_channel_setup_service as setup,
};
use crate::{
    AppState,
    errors::{AppError, AppResult},
};
use bson::doc;
use serde::Deserialize;

#[derive(Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CustomerAction {
    Check,
    Assign,
    PrepareModel,
    EnableGuests,
    OwnerLink,
}

pub struct CustomerSetupResult {
    pub channel_id: String,
    pub agent_id: Option<String>,
    pub guests_enabled: bool,
    /// Existing personal/org readiness projections keep their established wire shape.
    pub readiness: serde_json::Value,
    pub owner_link_code: Option<zeroize::Zeroizing<String>>,
    pub owner_link_expires_at: Option<String>,
}

pub async fn customer_setup(
    state: &AppState,
    actor: &str,
    id: &str,
    agent_id: &str,
    action: CustomerAction,
) -> AppResult<CustomerSetupResult> {
    use crate::{
        errors::AppError,
        models::nyxbot_channel::NyxbotChannel,
        services::{nyxbot_chat_settings_service::AccessSettings, org_channel_service as org},
    };
    use CustomerAction::*;

    let (owner, bot) =
        super::channel_bot_service::resolve_bot_owner_for_write(&state.db, actor, id).await?;
    channel_meta_service::adapter(&bot.platform)?;
    crate::services::assistant_nyxagent::require_enabled(&state.db, actor).await?;
    let specialist = state
        .db
        .collection::<crate::models::assistant_agent::AssistantAgent>("assistant_agents")
        .find_one(doc! {"_id":agent_id,"user_id":&owner,"kind":"specialist","destroyed_at":null})
        .await?
        .ok_or_else(|| {
            AppError::ValidationError(
                "Select a support specialist owned by this inbox's person or organization".into(),
            )
        })?;
    let existing = state
        .db
        .collection::<NyxbotChannel>("nyxbot_channels")
        .find_one(
            doc! {"channel_bot_id":&bot.id,"status":{"$in":["active","pending"]},
            "$or":[{"user_id":actor},{"bot_owner_id":&owner},{"org_specialist.org_id":&owner}]},
        )
        .await?;
    if action == OwnerLink
        && (owner != actor
            || existing
                .as_ref()
                .is_none_or(|row| row.agent_id.as_deref() != Some(specialist.id.as_str())))
    {
        return Err(AppError::ValidationError("Assign this specialist first. Organization members verify through the member-link flow.".into()));
    }
    let mut issued_link = None;
    let mut row = match existing {
        Some(mut row) => {
            if row.user_id != actor
                && row
                    .org_specialist
                    .as_ref()
                    .is_none_or(|b| b.org_id != owner)
            {
                return Err(AppError::NotFound("Channel agent not found".into()));
            }
            let reusable = super::nyxbot_channel_query_service::reusable_meta(
                &state.db,
                &row.user_id,
                &owner,
                &row,
                &state.config.base_url,
            )
            .await?;
            if !reusable {
                if action != Assign {
                    return Err(AppError::Conflict(
                        "Assign this specialist again to repair its inbox route before continuing"
                            .into(),
                    ));
                }
                let (repaired, link) =
                    setup::connect_meta(state, actor, None, &bot.id, &specialist, true, Some(&row))
                        .await?;
                issued_link = Some(link);
                row = repaired;
            } else if row.agent_id.as_deref() != Some(specialist.id.as_str()) {
                if action != Assign {
                    return Err(AppError::Conflict("The selected specialist differs from the committed inbox assignment. Explicitly assign it before continuing.".into()));
                }
                (row, _) = setup::mutate_meta_assignment(
                    state,
                    actor,
                    &row,
                    Some(&specialist),
                    None,
                    None,
                )
                .await?;
                setup::after_link(state, actor, &row, &specialist).await;
            }
            row
        }
        None if action == Assign => {
            let (row, link) =
                setup::connect_meta(state, actor, None, &bot.id, &specialist, true, None).await?;
            issued_link = Some(link);
            row
        }
        None => {
            return Err(AppError::ValidationError(
                "Assign a support specialist before checking or enabling customer conversations"
                    .into(),
            ));
        }
    };
    if action == OwnerLink {
        let code = super::assistant_wake_service::watch::new_link_code();
        let expires_at = crate::services::nyxbot_channel_continuity_service::rotate_owner_link(
            &state.db,
            &row,
            &{
                use sha2::Digest;
                hex::encode(sha2::Sha256::digest(code.as_bytes()))
            },
            &super::nyxbot_channel_query_service::direct_callback(&state.config.base_url, &row.id),
        )
        .await?;
        issued_link = Some(setup::LinkInstructions {
            code,
            expires_at,
            url: None,
        });
    }
    let readiness = if row.org_specialist.is_some() {
        if action == PrepareModel {
            return Err(AppError::ValidationError("Organization guest model setup runs under verified guest funding. Enable the ready organization guest binding, then test from a separate customer account; setup diagnostics appear on this channel.".into()));
        }
        let inputs = org::management::view_inputs(&state.db, actor, &row).await?;
        if action == EnableGuests {
            if !inputs.ready {
                return Err(AppError::ValidationError("Organization guest grants are not ready. Enable nyxbot:org-guest-grants-v1 after all serving replicas are ready.".into()));
            }
            let already_open = row.private_chats.as_deref() == Some("everyone")
                && row
                    .org_specialist
                    .as_ref()
                    .is_some_and(|binding| binding.guests.enabled);
            if !already_open {
                row = org::management::enable_customer_dms(
                    &state.db,
                    actor,
                    &row,
                    org::management::GuestSettings {
                        enabled: true,
                        service_ids: None,
                        daily_credits: None,
                        expected_revision: row
                            .org_specialist
                            .as_ref()
                            .map(|b| b.guests.revision.clone()),
                    },
                    Some(&super::nyxbot_channel_query_service::direct_callback(
                        &state.config.base_url,
                        &row.id,
                    )),
                )
                .await?;
            }
        }
        inputs.render(&row, row.org_specialist.as_ref().ok_or_else(org::denied)?)
    } else {
        if action == PrepareModel {
            setup::prepare_guest_model(state, actor, Some(&specialist.id), None).await?;
        }
        let facts = setup::audience_readiness_for_channel(state, actor, &row).await?;
        if action == EnableGuests {
            if facts.guest_readiness.status != "ready" {
                return Err(AppError::ValidationError(facts.guest_readiness.fix.unwrap_or_else(||"Customer model setup is pending. Prepare the guest model, then check readiness again.".into())));
            }
            setup::update_confirmed_access(
                state,
                actor,
                &row,
                &AccessSettings {
                    private_chats: Some("everyone".into()),
                    private_chat_posts: Some(false),
                    apply_to_existing: false,
                },
            )
            .await?;
        }
        serde_json::to_value(facts)
            .map_err(|_| AppError::Internal("Unable to report guest readiness".into()))?
    };
    let committed = if action == Assign || (action == EnableGuests && row.org_specialist.is_some())
    {
        row
    } else {
        state
        .db
        .collection::<NyxbotChannel>("nyxbot_channels")
        .find_one(doc! {"_id":&row.id, "channel_bot_id": &bot.id, "agent_id": &specialist.id,
            "status": {"$in":["pending","active"]}, "$or":[{"user_id":actor},{"org_specialist.org_id":&owner}]})
        .await?
        .ok_or_else(|| AppError::Conflict("Channel assignment changed".into()))?
    };
    let guests_enabled = committed.private_chats.as_deref() == Some("everyone")
        && committed
            .org_specialist
            .as_ref()
            .is_none_or(|b| b.guests.enabled);
    Ok(CustomerSetupResult {
        channel_id: committed.id,
        agent_id: committed.agent_id,
        guests_enabled,
        readiness,
        owner_link_code: issued_link
            .as_ref()
            .filter(|_| committed.org_specialist.is_none())
            .map(|link| link.code.clone()),
        owner_link_expires_at: issued_link
            .as_ref()
            .map(|link| link.expires_at.to_rfc3339()),
    })
}

pub async fn bot_delivery(state: &AppState, actor: &str, id: &str, enabled: bool) -> AppResult<()> {
    let (_, bot) =
        super::channel_bot_service::resolve_bot_owner_for_write(&state.db, actor, id).await?;
    channel_meta_service::adapter(&bot.platform)?;
    channel_meta_service::set_bot_delivery(&state.db, &bot, actor, enabled).await?;
    if enabled {
        let current = super::channel_bot_service::get_bot(&state.db, id).await?;
        MetaService::from(state).verify(&current).await?;
    }
    Ok(())
}

pub async fn start(
    state: &AppState,
    actor: &str,
    platform: &str,
    label: &str,
    target_org_id: Option<&str>,
    reconnect: Option<&str>,
    setup_token: Option<&str>,
) -> AppResult<Started> {
    if let Some(token) = setup_token {
        let claim =
            super::channel_connect_link_service::claim(&state.db, actor, token, "meta_oauth")
                .await?;
        let result = async {
            if claim.link.platform != platform || reconnect.is_some() {
                return Err(AppError::Forbidden(
                    "Setup link does not match this connection".into(),
                ));
            }
            let started = MetaService::from(state)
                .start(
                    actor,
                    &claim.link.user_id,
                    platform,
                    &claim.link.label,
                    claim.link.bot_id.as_deref(),
                )
                .await?;
            state
                .db
                .collection::<bson::Document>(crate::models::channel_meta::SESSIONS)
                .update_one(
                    doc! {"_id": &started.session_id, "actor_id": actor},
                    doc! {"$set": {"connect_link_id": &claim.link.id}},
                )
                .await?;
            super::channel_connect_link_service::pin_connection(
                &state.db,
                &claim,
                &started.session_id,
            )
            .await?;
            Ok(started)
        }
        .await;
        super::channel_connect_link_service::release(&state.db, &claim, result.is_err()).await?;
        return result;
    }
    let owner = if let Some(org) = target_org_id {
        if !super::org_service::resolve_owner_access(&state.db, actor, org)
            .await?
            .can_write()
        {
            return Err(AppError::OrgRoleInsufficient(
                "you must be an admin of the target org to create a channel bot under it".into(),
            ));
        }
        org
    } else {
        actor
    };
    MetaService::from(state)
        .start(actor, owner, platform, label, reconnect)
        .await
}

/// Keep provider retries out of unrelated managed-onboarding rate buckets.
pub async fn oauth_limit(db: &mongodb::Database, actor: &str, operation: &str) -> AppResult<()> {
    let limiter =
        crate::mw::rate_limit::PerKeyRateLimiter::with_db(db.clone(), "channel_meta_oauth", 12, 60);
    if !limiter
        .check_shared(&format!("{actor}:{operation}"))
        .await?
    {
        return Err(AppError::RateLimited);
    }
    Ok(())
}

pub async fn platform_delivery(
    db: &mongodb::Database,
    platform: &str,
    enabled: bool,
) -> AppResult<()> {
    channel_meta_service::adapter(platform)?;
    channel_meta_service::set_platform_delivery(db, platform, enabled).await
}

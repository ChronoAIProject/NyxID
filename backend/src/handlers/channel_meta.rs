use crate::{
    AppState,
    errors::AppResult,
    mw::auth::AuthUser,
    services::channel_meta_service::{self, MetaService},
};
use axum::{
    Json,
    extract::{Path, State},
};
use bson::doc;
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Start {
    pub label: String,
    pub target_org_id: Option<String>,
    pub reconnect_bot_id: Option<String>,
    pub setup_token: Option<zeroize::Zeroizing<String>>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Exchange {
    pub state: zeroize::Zeroizing<String>,
    pub code: Option<zeroize::Zeroizing<String>>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Finish {
    pub asset_id: String,
    pub exclusive_routing_verified: bool,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Delivery {
    pub enabled: bool,
}

async fn oauth_limit(state: &AppState, auth: &AuthUser, operation: &str) -> AppResult<()> {
    // OAuth has three bounded requests (start, exchange, finish), and a
    // provider retry must not consume the shared onboarding budget used by
    // other managed platforms.
    let limiter = crate::mw::rate_limit::PerKeyRateLimiter::with_db(
        state.db.clone(),
        "channel_meta_oauth",
        12,
        60,
    );
    if !limiter
        .check_shared(&format!("{}:{operation}", auth.user_id))
        .await?
    {
        return Err(crate::errors::AppError::RateLimited);
    }
    Ok(())
}
pub async fn platform_delivery(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(platform): Path<String>,
    Json(input): Json<Delivery>,
) -> AppResult<Json<serde_json::Value>> {
    super::admin_helpers::require_admin(&state, &auth).await?;
    channel_meta_service::adapter(&platform)?;
    channel_meta_service::set_platform_delivery(&state.db, &platform, input.enabled).await?;
    crate::services::audit_service::log_for_user(
        state.db.clone(),
        &auth,
        "channel_platform_delivery_changed",
        Some(serde_json::json!({"platform":platform,"enabled":input.enabled})),
    );
    Ok(Json(
        serde_json::json!({"platform":platform,"enabled":input.enabled,"resume_requires_bot_verification":true}),
    ))
}
pub async fn bot_delivery(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(input): Json<Delivery>,
) -> AppResult<Json<serde_json::Value>> {
    let (_, bot) =
        super::channel_bots::resolve_bot_owner_for_write(&state, &auth.user_id.to_string(), &id)
            .await?;
    channel_meta_service::adapter(&bot.platform)?;
    channel_meta_service::set_bot_delivery(&state.db, &bot, input.enabled).await?;
    if input.enabled {
        let current = crate::services::channel_bot_service::get_bot(&state.db, &id).await?;
        MetaService::from(&state).verify(&current).await?;
    }
    crate::services::audit_service::log_for_user(
        state.db.clone(),
        &auth,
        "channel_bot_delivery_changed",
        Some(serde_json::json!({"bot_id":id,"enabled":input.enabled})),
    );
    Ok(Json(serde_json::json!({"id":id,"enabled":input.enabled})))
}

#[derive(Deserialize)]
pub struct SignedRequest {
    pub signed_request: zeroize::Zeroizing<String>,
}
pub async fn deauthorize(
    State(state): State<AppState>,
    Path(platform): Path<String>,
    axum::Form(input): axum::Form<SignedRequest>,
) -> AppResult<Json<serde_json::Value>> {
    Ok(Json(
        channel_meta_service::accept_provider_request(
            &MetaService::from(&state),
            &platform,
            &input.signed_request,
            false,
        )
        .await?,
    ))
}
pub async fn data_deletion(
    State(state): State<AppState>,
    Path(platform): Path<String>,
    axum::Form(input): axum::Form<SignedRequest>,
) -> AppResult<Json<serde_json::Value>> {
    Ok(Json(
        channel_meta_service::accept_provider_request(
            &MetaService::from(&state),
            &platform,
            &input.signed_request,
            true,
        )
        .await?,
    ))
}
pub async fn deletion_status(
    State(state): State<AppState>,
    Path((platform, id)): Path<(String, String)>,
) -> AppResult<Json<serde_json::Value>> {
    Ok(Json(
        channel_meta_service::request_status(&state.db, &platform, &id).await?,
    ))
}

#[derive(Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CustomerAction {
    Check,
    Assign,
    PrepareModel,
    EnableGuests,
    OwnerLink,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CustomerSetup {
    pub agent_id: String,
    pub action: CustomerAction,
}

pub async fn customer_setup(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(input): Json<CustomerSetup>,
) -> AppResult<Json<serde_json::Value>> {
    use crate::{
        errors::AppError,
        models::nyxbot_channel::NyxbotChannel,
        services::{nyxbot_chat_settings_service::AccessSettings, org_channel_service as org},
    };
    use CustomerAction::*;
    super::login_client_context::require_first_party_human(&auth)?;
    let actor = auth.user_id.to_string();
    let (owner, bot) =
        super::channel_bots::resolve_bot_owner_for_write(&state, &actor, &id).await?;
    channel_meta_service::adapter(&bot.platform)?;
    crate::services::assistant_nyxagent::require_enabled(&state.db, &actor).await?;
    let specialist = state
        .db
        .collection::<crate::models::assistant_agent::AssistantAgent>("assistant_agents")
        .find_one(
            doc! {"_id":&input.agent_id,"user_id":&owner,"kind":"specialist","destroyed_at":null},
        )
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
            "$or":[{"user_id":&actor},{"bot_owner_id":&owner},{"org_specialist.org_id":&owner}]},
        )
        .await?;
    if input.action == OwnerLink
        && (owner != actor
            || existing
                .as_ref()
                .is_none_or(|row| row.agent_id.as_deref() != Some(specialist.id.as_str())))
    {
        return Err(AppError::ValidationError("Assign this specialist first. Organization members verify through the member-link flow.".into()));
    }
    let mut issued_link = None;
    let row = match existing {
        Some(mut row) => {
            if row.agent_id.as_deref() != Some(specialist.id.as_str()) {
                if input.action != Assign {
                    return Err(AppError::Conflict("The selected specialist differs from the committed inbox assignment. Explicitly assign it before continuing.".into()));
                }
                super::nyxbot::link(&state, &actor, &row.id, &specialist).await?;
                row = super::nyxbot::load_channel(&state, &actor, &row.id).await?;
            }
            row
        }
        None if input.action == Assign => {
            let (row, link) =
                super::nyxbot::connect(&state, &actor, None, &bot.id, &specialist).await?;
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
    if input.action == OwnerLink {
        let code = super::nyxbot::new_link_code();
        let expires_at = crate::services::nyxbot_channel_continuity_service::rotate_owner_link(
            &state.db,
            &row,
            &{
                use sha2::Digest;
                hex::encode(sha2::Sha256::digest(code.as_bytes()))
            },
        )
        .await?;
        issued_link = Some(super::nyxbot::LinkInstructions {
            code,
            expires_at,
            url: None,
        });
    }
    let readiness = if row.org_specialist.is_some() {
        let facts = org::management::view(&state.db, &actor, &row).await?;
        if input.action == EnableGuests {
            if facts["grants_ready"] != true {
                return Err(AppError::ValidationError("Organization guest grants are not ready. Enable nyxbot:org-guest-grants-v1 after all serving replicas are ready.".into()));
            }
            let already_open = row.private_chats.as_deref() == Some("everyone")
                && row
                    .org_specialist
                    .as_ref()
                    .is_some_and(|binding| binding.guests.enabled);
            if !already_open {
                org::management::enable_customer_dms(
                    &state.db,
                    &actor,
                    &row,
                    org::management::GuestSettings {
                        enabled: true,
                        service_ids: None,
                        daily_credits: None,
                        expected_revision: facts["guest_revision"].as_str().map(str::to_owned),
                    },
                )
                .await?;
            }
        }
        if input.action == PrepareModel {
            return Err(AppError::ValidationError("Organization guest model setup runs under verified guest funding. Enable the ready organization guest binding, then test from a separate customer account; setup diagnostics appear on this channel.".into()));
        }
        facts
    } else {
        if input.action == PrepareModel {
            super::nyxbot::prepare_guest_model(&state, &actor, Some(&specialist.id), None).await?;
        }
        let facts = super::nyxbot::chats::audience_readiness(&state, &actor, &row.id).await?;
        if input.action == EnableGuests {
            if facts.guest_readiness.status != "ready" {
                return Err(AppError::ValidationError(facts.guest_readiness.fix.unwrap_or_else(||"Customer model setup is pending. Prepare the guest model, then check readiness again.".into())));
            }
            super::nyxbot::chats::update_access(
                &state,
                &actor,
                &row.id,
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
    let committed = state
        .db
        .collection::<NyxbotChannel>("nyxbot_channels")
        .find_one(doc! {"_id":&row.id, "channel_bot_id": &bot.id, "agent_id": &specialist.id,
            "status": {"$in":["pending","active"]}, "$or":[{"user_id":&actor},{"org_specialist.org_id":&owner}]})
        .await?
        .ok_or_else(|| AppError::Conflict("Channel assignment changed".into()))?;
    let guests_enabled = committed.private_chats.as_deref() == Some("everyone")
        && committed
            .org_specialist
            .as_ref()
            .is_none_or(|b| b.guests.enabled);
    Ok(Json(serde_json::json!({"channel_id":committed.id,
        "agent_id":committed.agent_id,
        "guests_enabled":guests_enabled,
        "readiness":readiness,
        "owner_link_code":issued_link.as_ref().filter(|_|committed.org_specialist.is_none()).map(|l|l.code.as_str()),
        "owner_link_expires_at":issued_link.as_ref().map(|l|l.expires_at.to_rfc3339())})))
}

pub async fn start(
    State(s): State<AppState>,
    auth: AuthUser,
    Path(platform): Path<String>,
    Json(input): Json<Start>,
) -> AppResult<Json<channel_meta_service::Started>> {
    super::login_client_context::require_first_party_human(&auth)?;
    oauth_limit(&s, &auth, &platform).await?;
    let actor = auth.user_id.to_string();
    if let Some(token) = input.setup_token.as_deref() {
        let claim = crate::services::channel_connect_link_service::claim(
            &s.db,
            &actor,
            token,
            "meta_oauth",
        )
        .await?;
        let result = async {
            if claim.link.platform != platform || input.reconnect_bot_id.is_some() {
                return Err(crate::errors::AppError::Forbidden(
                    "Setup link does not match this connection".into(),
                ));
            }
            let started = MetaService::from(&s)
                .start(
                    &actor,
                    &claim.link.user_id,
                    &platform,
                    &claim.link.label,
                    claim.link.bot_id.as_deref(),
                )
                .await?;
            s.db.collection::<bson::Document>(crate::models::channel_meta::SESSIONS)
                .update_one(
                    bson::doc! {"_id":&started.session_id,"actor_id":&actor},
                    bson::doc! {"$set":{"connect_link_id":&claim.link.id}},
                )
                .await?;
            crate::services::channel_connect_link_service::pin_connection(
                &s.db,
                &claim,
                &started.session_id,
            )
            .await?;
            Ok(Json(started))
        }
        .await;
        crate::services::channel_connect_link_service::release(&s.db, &claim, result.is_err())
            .await?;
        return result;
    }
    let owner =
        super::channel_bots::resolve_create_owner(&s, &actor, input.target_org_id.as_deref())
            .await?;
    Ok(Json(
        MetaService::from(&s)
            .start(
                &actor,
                &owner,
                &platform,
                &input.label,
                input.reconnect_bot_id.as_deref(),
            )
            .await?,
    ))
}
pub async fn exchange(
    State(s): State<AppState>,
    auth: AuthUser,
    Path(platform): Path<String>,
    Json(input): Json<Exchange>,
) -> AppResult<Json<channel_meta_service::Progress>> {
    super::login_client_context::require_first_party_human(&auth)?;
    oauth_limit(&s, &auth, &platform).await?;
    Ok(Json(
        MetaService::from(&s)
            .exchange(
                &auth.user_id.to_string(),
                &platform,
                &input.state,
                input.code.as_ref().map(|s| s.as_str()),
            )
            .await?,
    ))
}
pub async fn progress(
    State(s): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<channel_meta_service::Progress>> {
    super::login_client_context::require_first_party_human(&auth)?;
    Ok(Json(
        MetaService::from(&s)
            .progress(&auth.user_id.to_string(), &id)
            .await?,
    ))
}
#[derive(Serialize)]
pub struct Connected {
    pub id: String,
    pub platform: String,
    pub label: String,
    pub platform_bot_username: String,
    pub status: String,
    pub credential_source: String,
    pub webhook_ingestion: bool,
    pub webhook_url: String,
}
pub async fn finish(
    State(s): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(input): Json<Finish>,
) -> AppResult<Json<Connected>> {
    super::login_client_context::require_first_party_human(&auth)?;
    oauth_limit(&s, &auth, "finish").await?;
    let bot = MetaService::from(&s)
        .finish(
            &auth.user_id.to_string(),
            &id,
            &input.asset_id,
            input.exclusive_routing_verified,
        )
        .await?;
    Ok(Json(Connected {
        webhook_url: crate::services::channel_bot_service::webhook_url(&s.config.base_url, &bot),
        id: bot.id,
        platform: bot.platform,
        label: bot.label,
        platform_bot_username: bot.platform_bot_username,
        status: bot.status,
        credential_source: bot.credential_source,
        webhook_ingestion: true,
    }))
}

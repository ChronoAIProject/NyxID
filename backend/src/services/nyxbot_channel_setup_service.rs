//! Shared channel assignment and customer model orchestration.
use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::{
        channel_bot::ChannelBot,
        nyxbot_channel::{
            COLLECTION_NAME as CHANNELS, NyxbotChannel, THREADS_COLLECTION_NAME as THREADS,
        },
    },
    services::{assistant_nyxagent as engine, channel_bot_service},
};
use crate::{
    models::nyxbot_gateway_release::ReleaseReason,
    services::nyxbot_gateway_release_service as releases,
};
use mongodb::bson::doc;
use serde_json::{Value, json};
use uuid::Uuid;

pub(crate) async fn manages_bot_owner(
    state: &AppState,
    owner: &str,
    bot_owner: &str,
) -> AppResult<bool> {
    if owner == bot_owner {
        return Ok(true);
    }
    Ok(matches!(
        crate::services::org_service::resolve_owner_access(&state.db, owner, bot_owner).await?,
        crate::services::org_service::OwnerAccess::AsOrgAdmin { .. }
    ))
}

pub(crate) async fn accessible_bot(
    state: &AppState,
    owner: &str,
    bot_id: &str,
) -> AppResult<ChannelBot> {
    let bot = channel_bot_service::get_bot(&state.db, bot_id).await?;
    if manages_bot_owner(state, owner, &bot.user_id).await? {
        Ok(bot)
    } else {
        Err(AppError::ChannelBotNotFound(bot_id.to_string()))
    }
}

pub(crate) async fn org_access_with(
    state: &AppState,
    row: &NyxbotChannel,
    checked: Option<&crate::services::platform_bot_authority::PersonalRouteCheck>,
) -> AppResult<crate::services::org_channel_service::OrgAccess> {
    use crate::services::org_channel_service::OrgAccess;
    if !crate::services::platform_bot_authority::permits_with(
        &state.db,
        &row.channel_bot_id,
        checked,
    )
    .await?
    {
        return Ok(OrgAccess::Lost);
    }
    if let Some(org) = row.bot_owner_id.as_deref()
        && !manages_bot_owner(state, &row.user_id, org).await?
    {
        return Ok(OrgAccess::Lost);
    }
    crate::services::org_channel_service::transport_access(&state.db, row).await
}

pub(crate) async fn load_channel(
    state: &AppState,
    owner: &str,
    id: &str,
) -> AppResult<NyxbotChannel> {
    if Uuid::parse_str(id).is_err() {
        return Err(AppError::NotFound("Channel agent not found".into()));
    }
    let row = state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .find_one(doc! {"_id": id})
        .await?
        .ok_or_else(|| AppError::NotFound("Channel agent not found".into()))?;
    if row.user_id != owner {
        let binding = row
            .org_specialist
            .as_ref()
            .ok_or_else(|| AppError::NotFound("Channel agent not found".into()))?;
        crate::services::org_channel_service::require_admin(&state.db, owner, &binding.org_id)
            .await?;
    }
    Ok(row)
}

pub(crate) async fn link(
    state: &AppState,
    owner: &str,
    channel_id: &str,
    agent: &crate::models::assistant_agent::AssistantAgent,
) -> AppResult<Value> {
    crate::services::org_channel_service::require_target(&state.db, owner, agent).await?;
    if agent.destroyed_at.is_some() {
        return Err(AppError::Conflict("That agent was destroyed".into()));
    }
    let row = load_channel(state, owner, channel_id).await?;
    let bot = accessible_bot(state, owner, &row.channel_bot_id).await?;
    let binding =
        crate::services::org_channel_service::link_target(&state.db, owner, &bot, agent).await?;
    if binding.is_some() != row.org_specialist.is_some() {
        return Err(AppError::Conflict("Disconnect and reconnect this bot to change between a personal and organization specialist".into()));
    }
    crate::services::platform_bot_authority::require_personal_routes(
        &state.db,
        &row.channel_bot_id,
    )
    .await?;
    if row.agent_id.as_deref() == Some(agent.id.as_str())
        || (row.agent_id.is_none() && agent.is_nyxbot())
    {
        return Ok(json!({"channel_agent_id": row.id, "agent": agent.name, "changed": false}));
    }
    let callback = (row.transport == "direct"
        && super::channel_adapters::meta::is_meta(&row.platform))
    .then(|| super::nyxbot_channel_query_service::direct_callback(&state.config.base_url, &row.id));
    #[cfg(test)]
    if callback.is_some() {
        super::durable_wake_tests::after("meta_relink_before_commit").await;
    }
    crate::services::channel_thread_follow_service::relink(
        &state.db,
        &row.user_id,
        owner,
        &row.id,
        &agent.id,
        callback.as_deref(),
    )
    .await?;
    let open_group = state
        .db
        .collection::<bson::Document>(THREADS)
        .find_one(doc! {
            "channel_id":&row.id,"user_id":owner,"kind":{"$in":["group","channel"]},
            "$or":[{"members":"everyone"},{"members":null,"owner_seen":true}]
        })
        .projection(doc! {"_id":1})
        .await?
        .is_some();
    if row.org_specialist.is_none()
        && (row.private_chats.as_deref() == Some("everyone") || open_group)
    {
        prepare_guest_model(state, owner, Some(&agent.id), None).await?;
    }
    audit(
        state,
        owner,
        "nyxbot_channel_linked",
        json!({
            "channel_agent_id": &row.id, "agent_id": &agent.id,
        }),
    )
    .await;
    Ok(json!({"channel_agent_id": row.id, "agent": agent.name, "changed": true}))
}

pub(crate) async fn prepare_guest_model(
    state: &AppState,
    owner: &str,
    agent_id: Option<&str>,
    conversation: Option<&str>,
) -> AppResult<()> {
    let agent = match agent_id {
        Some(id) => crate::services::assistant_team_service::agent(&state.db, owner, id).await?,
        None => crate::services::assistant_team_service::ensure_nyxbot(&state.db, owner).await?,
    };
    let profile = if let Some(id) = conversation {
        engine::get(&state.db, owner, id)
            .await
            .ok()
            .map(|r| r.model)
    } else {
        None
    };
    let profile = match profile {
        Some(profile) => profile,
        None => {
            crate::services::assistant_profile_routing::model_for(
                &state.db,
                crate::services::assistant_profile_routing::RouteRole::Channel,
                &agent.model,
            )
            .await
        }
    };
    crate::services::assistant_guest_model_setup::backoff::reset(
        &state.db,
        owner,
        &agent.id,
        Some(&profile),
    )
    .await?;
    crate::services::assistant_guest_model_setup::kick(state, owner, &agent.id, &profile, false)
        .await?;
    Ok(())
}

pub(crate) async fn audit(state: &AppState, owner: &str, event: &str, data: Value) {
    let _ = crate::services::audit_service::log_actor_event(
        state.db.clone(),
        &crate::services::audit_service::AuditActor {
            user_id: owner.into(),
            ip_address: None,
            user_agent: None,
            api_key_id: None,
            api_key_name: None,
        },
        event,
        Some(data),
    )
    .await;
}

/// Meta uses the same connection lifecycle as normal and restored connections.
pub(crate) async fn connect_meta(
    state: &AppState,
    owner: &str,
    source_conversation_id: Option<&str>,
    bot_id: &str,
    agent: &crate::models::assistant_agent::AssistantAgent,
    allow_existing: bool,
    repair_channel: Option<&NyxbotChannel>,
) -> AppResult<(NyxbotChannel, LinkInstructions)> {
    use super::nyxbot_channel_connection_service::{
        self as connections, ConnectionRequest, RelayConnection,
    };
    crate::services::channel_meta_service::adapter(
        &accessible_bot(state, owner, bot_id).await?.platform,
    )?;
    connections::connect(
        state,
        ConnectionRequest {
            owner,
            source_conversation_id,
            bot_id,
            agent,
            allow_existing,
            restoration: false,
            restoration_claim_id: None,
            repair_channel,
        },
        &RelayConnection,
    )
    .await
}
pub use super::nyxbot_channel_connection_service::LinkInstructions;

pub(crate) async fn audience_readiness(
    state: &AppState,
    owner: &str,
    channel_id: &str,
) -> AppResult<crate::services::assistant_service_execution::readiness::AudienceReadiness> {
    let row = load_channel(state, owner, channel_id).await?;
    if let Some(org) = row.bot_owner_id.as_deref()
        && !manages_bot_owner(state, owner, org).await?
    {
        return Err(AppError::Forbidden("org_access_lost".into()));
    }
    crate::services::assistant_service_execution::readiness::audience_readiness(
        &state.db,
        &state.encryption_keys,
        owner,
        row.agent_id.as_deref(),
    )
    .await
}

pub(crate) async fn update_access(
    state: &AppState,
    owner: &str,
    channel_id: &str,
    settings: &crate::services::nyxbot_chat_settings_service::AccessSettings,
) -> AppResult<Value> {
    let row = load_channel(state, owner, channel_id).await?;
    update_access_inner(state, owner, row, settings, false).await
}

pub(crate) async fn update_confirmed_access(
    state: &AppState,
    owner: &str,
    row: &NyxbotChannel,
    settings: &crate::services::nyxbot_chat_settings_service::AccessSettings,
) -> AppResult<Value> {
    update_access_inner(state, owner, row.clone(), settings, true).await
}

async fn update_access_inner(
    state: &AppState,
    owner: &str,
    row: NyxbotChannel,
    settings: &crate::services::nyxbot_chat_settings_service::AccessSettings,
    confirm_assignment: bool,
) -> AppResult<Value> {
    settings.validate()?;
    if let Some(org) = row.bot_owner_id.as_deref()
        && !manages_bot_owner(state, owner, org).await?
    {
        return Err(AppError::Forbidden("org_access_lost".into()));
    }
    if settings.private_chats.as_deref() == Some("everyone")
        || settings.private_chat_posts == Some(true)
    {
        crate::services::platform_bot_authority::require_personal_routes(
            &state.db,
            &row.channel_bot_id,
        )
        .await?;
    }
    // Read before mutation: diagnostics must not turn a committed change into a failure.
    let mut audience =
        if settings.private_chats.as_deref() == Some("everyone") && row.org_specialist.is_none() {
            Some(
                crate::services::assistant_service_execution::readiness::audience_readiness(
                    &state.db,
                    &state.encryption_keys,
                    owner,
                    row.agent_id.as_deref(),
                )
                .await?,
            )
        } else {
            None
        };
    let applied = if confirm_assignment {
        crate::services::nyxbot_chat_settings_service::update_confirmed_as(
            &state.db, &row, owner, settings,
        )
        .await?
    } else {
        crate::services::nyxbot_chat_settings_service::update_as(&state.db, &row, owner, settings)
            .await?
    };
    audit(
        state,
        owner,
        "nyxbot_channel_access_updated",
        json!({"channel_agent_id":&row.id,
        "private_chats":settings.private_chats,"private_chat_posts":settings.private_chat_posts,
        "apply_to_existing":settings.apply_to_existing,"applied_count":applied}),
    )
    .await;
    let mut result = json!({"channel_agent_id":row.id,
        "guest_access_description": if row.org_specialist.is_some() {
            crate::services::org_channel_service::management::ORG_GUEST_EXPLANATION
        } else {
            crate::services::org_channel_service::management::PERSONAL_GUEST_EXPLANATION
        },
        "private_chats":settings.private_chats.as_deref().or(row.private_chats.as_deref()).unwrap_or("owner"),
        "private_chat_posts":settings.private_chat_posts.or(row.private_chat_posts).unwrap_or(false),"applied_count":applied});
    if let Some(access) = settings
        .private_chats
        .as_deref()
        .filter(|_| row.org_specialist.is_none())
    {
        let prepared = access != "everyone"
            || prepare_guest_model(state, owner, row.agent_id.as_deref(), None)
                .await
                .is_ok();
        let readiness = if prepared {
            crate::services::assistant_service_execution::guest_readiness(
                &state.db,
                &state.encryption_keys,
                owner,
                row.agent_id.as_deref(),
                None,
            )
            .await
            .ok()
        } else {
            None
        };
        let readiness = readiness.unwrap_or(
            crate::services::assistant_service_execution::GuestReadiness {
                status: "not_ready",
                reason: Some("readiness_unavailable".into()),
                fix: Some(
                    "Access was saved. Refresh customer model readiness before relying on replies."
                        .into(),
                ),
            },
        );
        result["guest_readiness"] = json!(&readiness);
        if let Some(facts) = &mut audience {
            facts.guest_readiness = readiness;
            result["audience_readiness"] = json!(facts);
        }
    }
    Ok(result)
}

pub(crate) async fn release_org_channel(state: &AppState, row: &NyxbotChannel) -> AppResult<()> {
    if row.bot_owner_id.is_none() {
        return Ok(());
    }
    let ids = releases::retire_connection(
        state,
        row,
        ReleaseReason::OrgAccessLost,
        "failed",
        Some("org_access_lost"),
    )
    .await?;
    releases::try_inline(state, &ids).await;
    Ok(())
}

pub(crate) async fn disconnect(state: &AppState, owner: &str, id: &str) -> AppResult<Value> {
    let row = load_channel(state, owner, id).await?;
    let ids =
        releases::retire_connection(state, &row, ReleaseReason::Disconnect, "disconnected", None)
            .await?;
    finish_disconnect(state, owner, &row, &ids).await
}

pub(crate) async fn disconnect_for_repair(
    state: &AppState,
    actor: &str,
    expected: &NyxbotChannel,
    meta_callback: Option<&str>,
) -> AppResult<Value> {
    let ids = releases::retire_for_repair(state, actor, expected, meta_callback).await?;
    finish_disconnect(state, actor, expected, &ids).await
}

async fn finish_disconnect(
    state: &AppState,
    owner: &str,
    row: &NyxbotChannel,
    ids: &[String],
) -> AppResult<Value> {
    let gateway_released = if ids.is_empty() {
        row.transport != "gateway"
    } else {
        releases::try_inline(state, ids).await
    };
    audit(
        state,
        owner,
        "nyxbot_channel_disconnected",
        json!({"channel_agent_id": &row.id, "gateway_released": gateway_released}),
    )
    .await;
    Ok(json!({"disconnected": row.id, "platform": row.platform,
        "gateway_released": gateway_released}))
}

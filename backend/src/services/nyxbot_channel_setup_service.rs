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
use futures::TryStreamExt;
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
    crate::services::channel_thread_follow_service::relink(
        &state.db,
        &row.user_id,
        owner,
        &row.id,
        &agent.id,
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

/// Direct Meta connections share the normal route, key, retirement and
/// continuity services. Meta never creates a native gateway installation.
pub(crate) async fn connect_meta(
    state: &AppState,
    owner: &str,
    source_conversation_id: Option<&str>,
    bot_id: &str,
    agent: &crate::models::assistant_agent::AssistantAgent,
    allow_existing: bool,
) -> AppResult<(NyxbotChannel, LinkInstructions)> {
    use crate::models::nyxbot_gateway_release::ReleaseReason;
    use crate::services::{
        channel_routing_service, key_service, nyxbot_channel_continuity_service as continuity,
        nyxbot_gateway_release_service as releases,
    };
    use chrono::Utc;
    crate::services::org_channel_service::require_target(&state.db, owner, agent).await?;
    if agent.destroyed_at.is_some() {
        return Err(AppError::Conflict("That agent was destroyed".into()));
    }
    let bot = accessible_bot(state, owner, bot_id).await?;
    crate::services::channel_meta_service::adapter(&bot.platform)?;
    let org_binding =
        crate::services::org_channel_service::link_target(&state.db, owner, &bot, agent).await?;
    crate::services::platform_bot_authority::require_personal_routes(&state.db, &bot.id).await?;
    if !bot.is_active {
        return Err(AppError::ValidationError(
            "That channel bot is not active".into(),
        ));
    }
    let bot_owner_id = bot.user_id.clone();
    let mut rebuilt_from = None;
    if let Some(existing) =
        crate::services::nyxbot_channel_query_service::active_for_bot(&state.db, owner, &bot.id)
            .await?
    {
        if !allow_existing {
            return Err(AppError::Conflict(
                "This bot already has a selected agent".into(),
            ));
        }
        if crate::services::nyxbot_channel_query_service::reusable(
            &state.db,
            owner,
            &bot_owner_id,
            &existing,
        )
        .await?
        {
            link(state, owner, &existing.id, agent).await?;
            let code = crate::services::assistant_wake_service::watch::new_link_code();
            continuity::restore_existing(
                &state.db,
                &existing.id,
                owner,
                agent.is_nyxbot(),
                &crate::crypto::token::hash_token(&code),
            )
            .await?;
            let row = load_channel(state, owner, &existing.id).await?;
            let link = link_instructions(&row, code)?;
            return Ok((row, link));
        }
        let ids = releases::retire_connection(
            state,
            &existing,
            ReleaseReason::Disconnect,
            "disconnected",
            None,
        )
        .await?;
        let gateway_released = if ids.is_empty() {
            existing.transport != "gateway"
        } else {
            releases::try_inline(state, &ids).await
        };
        audit(
            state,
            owner,
            "nyxbot_channel_disconnected",
            json!({"channel_agent_id": &existing.id, "gateway_released": gateway_released}),
        )
        .await;
        rebuilt_from = Some(existing);
    }
    if channel_routing_service::list_conversations(&state.db, &bot_owner_id, Some(&bot.id))
        .await?
        .iter()
        .any(|row| row.default_agent && row.is_active)
    {
        return Err(AppError::Conflict("This bot already routes to another agent. Remove that route first (nyxid__delete_channel_route), then connect NyxBot.".into()));
    }
    let id = Uuid::new_v4().to_string();
    let label = engine::excerpt(&bot.label, 60);
    let callback = format!(
        "{}/api/v1/nyxbot/relay/{id}",
        state.config.base_url.trim_end_matches('/')
    );
    let route_key = key_service::create_api_key(
        &state.db,
        &bot_owner_id,
        &format!("NyxBot channel route {}", engine::identifier(&label)),
        "read proxy",
        crate::services::api_key_validation::NewKeyExpiry::NeverExpires(
            crate::services::api_key_validation::NonExpiringReason::ManagedNyxbot,
        ),
        Some("NyxBot channel route key. Managed by NyxBot; disconnect NyxBot to remove."),
        Some(&[]),
        Some(&[]),
        Some(false),
        Some(false),
        Some(false),
        None,
        None,
        Some("generic"),
        Some(&callback),
    )
    .await?;
    let now = Utc::now();
    let platform = bot.platform.clone();
    let row = NyxbotChannel {
        notification_epoch: 0,
        execution_policy_revision: 0,
        org_specialist: org_binding.clone(),
        gateway_source: None,
        gateway_native_phase: None,
        gateway_native_activation: None,
        gateway_native_activation_detail: None,
        gateway_native_activation_operation: None,
        gateway_native_delivery: None,
        gateway_intake: None,
        gateway_bot_mentions: Default::default(),
        gateway_threads: Default::default(),
        follow_capacity_revision: 0,
        follow_binding_generation: 0,
        id: id.clone(),
        user_id: owner.into(),
        channel_bot_id: bot.id.clone(),
        bot_owner_id: (bot_owner_id != owner).then(|| bot_owner_id.clone()),
        platform: platform.clone(),
        bot_label: label.clone(),
        bot_username: Some(bot.platform_bot_username.clone()).filter(|name| !name.is_empty()),
        transport: "direct".into(),
        status: "pending".into(),
        last_error: None,
        route_api_key_id: route_key.id.clone(),
        route_id: None,
        agent_api_key_id: None,
        agent_key_ciphertext: None,
        gateway_channel_id: None,
        gateway_record_id: None,
        gateway_setup_release_id: None,
        continuity_from_channel_id: None,
        continuity_to_channel_id: None,
        continuity_restored_at: None,
        gateway_version: None,
        gateway_no_progress: false,
        binding_id: None,
        gateway_groups: None,
        gateway_groups_retry_at: None,
        gateway_bot_id: None,
        gateway_attempted_at: None,
        relay_attempted_at: None,
        gateway_fallback_at: None,
        pending_agent_api_key_id: None,
        pending_route_api_key_id: None,
        // Meta sender identities require an inbox-specific owner challenge.
        // Continuity activation may carry a verified same-inbox predecessor.
        owner_sender_ids: Vec::new(),
        link_code_hash: None,
        link_code_expires_at: None,
        source_conversation_id: source_conversation_id.map(str::to_owned),
        agent_id: Some(agent.id.clone()),
        private_chats: org_binding.as_ref().map(|_| "everyone".to_owned()),
        private_chat_posts: None,
        delivery_status: None,
        delivery_error: None,
        delivery_failed_at: None,
        delivery_seen_at: None,
        delivery_checked_at: None,
        delivery_notified_at: None,
        org_restore_next_attempt_at: None,
        org_restore_attempts: 0,
        org_restore_rule_version: 0,
        org_restore_last_reason: None,
        org_restore_claim_id: None,
        org_restore_lease_until: None,
        org_restored_at: None,
        created_at: now,
        updated_at: now,
    };
    let insertion = crate::services::transaction_service::run_db_only(
        &state.db,
        &(&state.db, &row),
        |tx, &(db, row)| {
            Box::pin(async move {
                crate::services::platform_bot_authority::fence_personal_routes(
                    db,
                    &row.channel_bot_id,
                    row.bot_owner_id.as_deref().unwrap_or(&row.user_id),
                    tx,
                )
                .await?;
                if db
                    .collection::<NyxbotChannel>(CHANNELS)
                    .find_one(doc! {
                        "channel_bot_id": &row.channel_bot_id,
                        "status": {"$in": ["pending", "active"]},
                    })
                    .session(&mut *tx)
                    .await?
                    .is_some()
                {
                    return Err(AppError::Conflict(
                        "This bot already has a selected agent".into(),
                    ));
                }
                db.collection::<NyxbotChannel>(CHANNELS)
                    .insert_one(row)
                    .session(&mut *tx)
                    .await?;
                if row.org_specialist.is_some() {
                    crate::services::org_channel_service::fence_management(
                        db,
                        tx,
                        row,
                        &row.user_id,
                    )
                    .await?;
                    crate::services::org_channel_service::management::restore_in_session(
                        db, tx, row,
                    )
                    .await?;
                }
                Ok(())
            })
        },
    )
    .await;
    if let Err(error) = insertion {
        let _ = key_service::delete_api_key(&state.db, &bot_owner_id, &route_key.id).await;
        return Err(error);
    }
    let route = channel_routing_service::create_conversation(
        &state.db,
        &bot_owner_id,
        Some(&bot.id),
        &bot.platform,
        "*",
        "private",
        None,
        &route_key.id,
        true,
        false,
    )
    .await;
    let route = match route {
        Ok(route) => route,
        Err(_) => {
            let current = load_channel(state, owner, &id).await?;
            retire_failed_setup(state, &current, "route_create_failed").await?;
            audit(
                state,
                owner,
                "nyxbot_channel_connect_failed",
                json!({
                    "channel_agent_id": id, "platform": platform,
                    "error_code": "route_create_failed",
                }),
            )
            .await;
            return Err(AppError::ValidationError(
                "Could not connect this bot. Try again. (route_create_failed)".into(),
            ));
        }
    };
    match org_access_with(state, &row, None).await? {
        crate::services::org_channel_service::OrgAccess::Holds => {}
        crate::services::org_channel_service::OrgAccess::Lost => {
            let mut revoked = load_channel(state, owner, &id).await?;
            revoked.route_id = Some(route.id.clone());
            let ids = releases::retire_connection(
                state,
                &revoked,
                ReleaseReason::OrgAccessLost,
                "failed",
                Some("org_access_lost"),
            )
            .await?;
            releases::try_inline(state, &ids).await;
            return Err(AppError::Forbidden("org_access_lost".into()));
        }
        crate::services::org_channel_service::OrgAccess::Unavailable => {
            return Err(AppError::SecurityReadinessUnavailable);
        }
    }
    let code = crate::services::assistant_wake_service::watch::new_link_code();
    let link_hash = crate::crypto::token::hash_token(&code);
    let activation = continuity::activate(
        &state.db,
        owner,
        &id,
        &route.id,
        rebuilt_from.as_ref().map(|row| row.id.as_str()),
        agent.is_nyxbot(),
        &link_hash,
    )
    .await;
    // A failed read after an uncertain commit leaves the durable reservation
    // for recovery; it must not trigger destructive rollback of a live route.
    let current = load_channel(state, owner, &id).await?;
    if let Err(error) = activation
        && (current.status != "active" || current.link_code_hash.as_deref() != Some(&link_hash))
    {
        retire_failed_setup(state, &current, "setup_interrupted").await?;
        return Err(error);
    }
    let link = link_instructions(&current, code)?;
    audit(
        state,
        owner,
        "nyxbot_channel_connected",
        json!({
            "channel_agent_id": id, "channel_bot_id": bot.id,
            "platform": platform, "transport": "direct",
        }),
    )
    .await;
    Ok((current, link))
}

async fn retire_failed_setup(
    state: &AppState,
    row: &NyxbotChannel,
    code: &'static str,
) -> AppResult<()> {
    use crate::models::nyxbot_gateway_release::ReleaseReason;
    use crate::services::nyxbot_gateway_release_service as releases;
    let ids =
        releases::retire_connection(state, row, ReleaseReason::SetupFailed, "failed", Some(code))
            .await?;
    releases::try_inline(state, &ids).await;
    Ok(())
}

pub(crate) struct LinkInstructions {
    pub code: zeroize::Zeroizing<String>,
    pub url: Option<String>,
    pub expires_at: chrono::DateTime<chrono::Utc>,
}

fn link_instructions(
    row: &NyxbotChannel,
    code: zeroize::Zeroizing<String>,
) -> AppResult<LinkInstructions> {
    Ok(LinkInstructions {
        code,
        url: None,
        expires_at: row
            .link_code_expires_at
            .ok_or_else(|| AppError::Internal("Owner link unavailable".into()))?,
    })
}

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
    settings.validate()?;
    let row = load_channel(state, owner, channel_id).await?;
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
    let applied =
        crate::services::nyxbot_chat_settings_service::update_as(&state.db, &row, owner, settings)
            .await?;
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

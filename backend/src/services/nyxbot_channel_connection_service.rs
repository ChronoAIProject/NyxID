//! One connection lifecycle for relay, gateway and restoration entry points.
use crate::models::nyxbot_gateway_release::ReleaseReason;
use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::{
        channel_bot::ChannelBot,
        nyxbot_channel::{COLLECTION_NAME as CHANNELS, NyxbotChannel},
    },
    services::{
        assistant_nyxagent::{excerpt, identifier},
        channel_platform::canonical_platform,
        channel_routing_service, feature_flag_service, key_service,
        nyxbot_channel_setup_service::{
            accessible_bot, audit, link, load_channel, org_access_with, release_org_channel,
        },
        nyxbot_gateway_release_service as releases,
        nyxbot_gateway_service::gateway_base,
        org_channel_service::OrgAccess,
    },
};
use chrono::Utc;
use mongodb::bson::{self, doc};
use serde_json::{Value, json};
use uuid::Uuid;
use zeroize::Zeroizing;

fn bot_owner(row: &NyxbotChannel) -> &str {
    row.bot_owner_id.as_deref().unwrap_or(&row.user_id)
}
fn sha256_hex(value: impl AsRef<[u8]>) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(value.as_ref()))
}
fn new_link_code() -> Zeroizing<String> {
    crate::services::assistant_wake_service::watch::new_link_code()
}
async fn active_for_bot(
    state: &AppState,
    owner: &str,
    bot: &str,
) -> AppResult<Option<NyxbotChannel>> {
    crate::services::nyxbot_channel_query_service::active_for_bot(&state.db, owner, bot).await
}

#[cfg(test)]
pub(crate) struct ConnectInsertGate {
    pub(crate) bot_id: String,
    pub(crate) arrived: std::sync::Arc<tokio::sync::Barrier>,
    pub(crate) release: std::sync::Arc<tokio::sync::Notify>,
}
#[cfg(test)]
tokio::task_local! {
    pub(crate) static CONNECT_INSERT_GATE: std::sync::Arc<ConnectInsertGate>;
}
#[cfg(test)]
tokio::task_local! {
    pub(crate) static RESTORE_CONNECT_FAIL_STAGE: u8;
}

#[cfg(test)]
pub(crate) const RESTORE_FAIL_ROUTE: u8 = 1;
#[cfg(test)]
pub(crate) const RESTORE_FAIL_ACTIVATION: u8 = 2;

pub(crate) struct ConnectionRequest<'a> {
    pub owner: &'a str,
    pub source_conversation_id: Option<&'a str>,
    pub bot_id: &'a str,
    pub agent: &'a crate::models::assistant_agent::AssistantAgent,
    pub allow_existing: bool,
    pub restoration: bool,
    pub restoration_claim_id: Option<&'a str>,
    pub repair_channel: Option<&'a NyxbotChannel>,
}

/// Legacy gateway effects are supplied by the existing transport implementation.
/// The service owns connection policy, transactions and lifecycle for every path.
#[async_trait::async_trait]
pub(crate) trait ConnectionTransport: Send + Sync {
    async fn connect_gateway(
        &self,
        state: &AppState,
        row: &NyxbotChannel,
        bot: &ChannelBot,
        agent_key: &str,
        route_key: &str,
    ) -> Result<String, &'static str>;
    async fn fall_back_to_direct(
        &self,
        state: &AppState,
        row: &NyxbotChannel,
        bot: &ChannelBot,
        callback: &str,
    ) -> Result<String, &'static str>;
    async fn sync_gateway_groups(
        &self,
        state: &AppState,
        row: &NyxbotChannel,
        force: bool,
    ) -> AppResult<Option<&'static str>>;
}

/// Meta has no gateway transport. Its direct path still uses the shared
/// connection orchestration, including all live authority and continuity gates.
pub(crate) struct RelayConnection;
#[async_trait::async_trait]
impl ConnectionTransport for RelayConnection {
    async fn connect_gateway(
        &self,
        _: &AppState,
        _: &NyxbotChannel,
        _: &ChannelBot,
        _: &str,
        _: &str,
    ) -> Result<String, &'static str> {
        Err("gateway_configuration_required")
    }
    async fn fall_back_to_direct(
        &self,
        _: &AppState,
        _: &NyxbotChannel,
        _: &ChannelBot,
        _: &str,
    ) -> Result<String, &'static str> {
        Err("gateway_configuration_required")
    }
    async fn sync_gateway_groups(
        &self,
        _: &AppState,
        _: &NyxbotChannel,
        _: bool,
    ) -> AppResult<Option<&'static str>> {
        Ok(None)
    }
}

pub struct LinkInstructions {
    pub code: Zeroizing<String>,
    pub url: Option<String>,
    pub expires_at: chrono::DateTime<Utc>,
}

/// The linking person's gateway rollout flag; Telegram always uses the gateway.
pub(crate) fn gateway_flag(platform: &str) -> Option<&'static str> {
    let platform = canonical_platform(platform);
    feature_flag_service::NYXBOT_GATEWAY_FLAGS
        .iter()
        .find(|(listed, _)| *listed == platform)
        .map(|(_, flag)| *flag)
}

pub(crate) async fn gateway_enabled(
    state: &AppState,
    owner: &str,
    platform: &str,
) -> AppResult<bool> {
    if canonical_platform(platform) == "telegram" {
        return Ok(true);
    }
    match gateway_flag(platform) {
        Some(flag) => feature_flag_service::personal_flag_enabled(&state.db, owner, flag).await,
        None => Ok(false),
    }
}

pub(crate) fn link_url(row: &NyxbotChannel, code: &str) -> Option<String> {
    match (row.platform.as_str(), row.bot_username.as_deref()) {
        ("telegram", Some(username)) if !username.is_empty() => {
            Some(format!("https://t.me/{username}?start={code}"))
        }
        _ => None,
    }
}

/// Owner-verified senders from NyxID's own Telegram notification link.
pub(crate) async fn known_owner_senders(
    state: &AppState,
    owner: &str,
    platform: &str,
    bot_id: &str,
) -> Vec<String> {
    if platform != "telegram" {
        return Vec::new();
    }
    let mut ids: Vec<String> = state
        .db
        .collection::<crate::models::notification_channel::NotificationChannel>(
            crate::models::notification_channel::COLLECTION_NAME,
        )
        .find_one(doc! {"user_id": owner})
        .await
        .ok()
        .flatten()
        .and_then(|row| row.telegram_chat_id)
        .map(|id| vec![id.to_string()])
        .unwrap_or_default();
    if let Some(creator) = bot_creator(state, owner, bot_id).await
        && !ids.contains(&creator)
    {
        ids.push(creator);
    }
    ids
}

/// The Telegram account that created this bot through NyxID's in-Telegram
/// bot creation, when `owner` started that creation and bound that account
/// with their own setup challenge (`start_update_id`, set only by the
/// challenge's `/start`): Telegram confirmed the account created the bot, for
/// this NyxID account. It is the owner's own account, as surely as one that
/// opened the owner-verification link. A bot claimed with a code is not: the
/// code is transferable, and its creator is someone else.
pub(crate) async fn bot_creator(state: &AppState, owner: &str, bot_id: &str) -> Option<String> {
    use crate::models::telegram_bot_request::COLLECTION_NAME as REQUESTS;
    state
        .db
        .collection::<bson::Document>(REQUESTS)
        .find_one(doc! {"_id": bot_id, "actor_user_id": owner,
        "status": {"$in": ["provisioning", "connected"]},
        "start_update_id": {"$ne": null}})
        .projection(doc! {"telegram_user_id": 1})
        .await
        .ok()
        .flatten()
        .and_then(|request| request.get_i64("telegram_user_id").ok())
        .map(|id| id.to_string())
}

/// The linking person's gateway provider/event-tool key. Provider authentication
/// binds its personal owner to NyxbotChannel.user_id, including for org bots.
/// Channel management uses the separate creator_bearer, never this API key.
pub(crate) async fn create_gateway_agent_key(
    state: &AppState,
    owner: &str,
    label: &str,
) -> AppResult<key_service::CreatedApiKey> {
    key_service::create_api_key(
        &state.db,
        owner,
        &format!("NyxBot gateway agent {}", identifier(label)),
        "read proxy",
        crate::services::api_key_validation::NewKeyExpiry::NeverExpires(
            crate::services::api_key_validation::NonExpiringReason::ManagedNyxbot,
        ),
        Some("Authenticates the Agent Event Gateway to NyxBot. Managed by NyxBot."),
        Some(&[]),
        Some(&[]),
        Some(false),
        Some(false),
        Some(false),
        None,
        None,
        Some("generic"),
        None,
    )
    .await
}

pub(crate) fn gateway_diagnostic_audit(mut details: Value, row: &NyxbotChannel) -> Value {
    details["transport"] = json!(row.transport);
    if let Some(code) = &row.gateway_native_activation {
        details["gateway_native_activation"] = json!(code);
    }
    if let Some(code) = &row.gateway_native_activation_detail {
        details["gateway_native_activation_detail"] = json!(code);
    }
    if let Some(operation) = &row.gateway_native_activation_operation {
        details["gateway_native_activation_operation"] = json!(operation);
    }
    details
}

pub(crate) async fn connect_direct(
    state: &AppState,
    row: &NyxbotChannel,
    bot: &ChannelBot,
) -> Result<String, &'static str> {
    channel_routing_service::create_conversation(
        &state.db,
        bot_owner(row),
        Some(&bot.id),
        &bot.platform,
        "*",
        "private",
        None,
        &row.route_api_key_id,
        true,
        false,
    )
    .await
    .map(|route| route.id)
    .map_err(|_| "route_create_failed")
}

pub(crate) async fn connect(
    state: &AppState,
    request: ConnectionRequest<'_>,
    effects: &dyn ConnectionTransport,
) -> AppResult<(NyxbotChannel, LinkInstructions)> {
    let ConnectionRequest {
        owner,
        source_conversation_id,
        bot_id,
        agent,
        allow_existing,
        restoration,
        restoration_claim_id,
        repair_channel,
    } = request;

    crate::services::org_channel_service::require_target(&state.db, owner, agent).await?;
    if agent.destroyed_at.is_some() {
        return Err(AppError::Conflict("That agent was destroyed".into()));
    }
    let bot: ChannelBot = accessible_bot(state, owner, bot_id).await?;
    // Every NyxBot entry point uses the same direct Meta policy, including
    // ordinary Connect, assistant tools and organization restoration.
    let guarded_default = super::channel_adapters::meta::is_meta(&bot.platform);
    let org_binding =
        crate::services::org_channel_service::link_target(&state.db, owner, &bot, agent).await?;
    crate::services::platform_bot_authority::require_personal_routes(&state.db, &bot.id).await?;
    // An org bot's route and route key belong to the org.
    let bot_owner_id = bot.user_id.clone();
    if !bot.is_active {
        return Err(AppError::ValidationError(
            "That channel bot is not active".into(),
        ));
    }
    // Owners verified on a connection being rebuilt stay verified: the bot
    // and their chat-app account are the same.
    let mut rebuilt_from = None;
    let existing = if let Some(expected) = repair_channel {
        let current = load_channel(state, owner, &expected.id).await?;
        if current.channel_bot_id != bot.id
            || current.execution_policy_revision != expected.execution_policy_revision
            || current.route_api_key_id != expected.route_api_key_id
            || current.status != expected.status
        {
            return Err(AppError::Conflict(
                "Channel assignment changed during repair".into(),
            ));
        }
        Some(current)
    } else {
        active_for_bot(state, owner, &bot.id).await?
    };
    if let Some(existing) = existing {
        if !allow_existing {
            return Err(AppError::Conflict(
                "This bot already has a selected agent".into(),
            ));
        }
        let reusable = if guarded_default {
            super::nyxbot_channel_query_service::reusable_meta(
                &state.db,
                owner,
                &bot_owner_id,
                &existing,
                &state.config.base_url,
            )
            .await?
        } else {
            super::nyxbot_channel_query_service::reusable(
                &state.db,
                owner,
                &bot_owner_id,
                &existing,
            )
            .await?
        };
        if reusable {
            if repair_channel.is_some() {
                return Err(AppError::Conflict(
                    "Channel repair already completed; check the current assignment".into(),
                ));
            }
            #[cfg(test)]
            if guarded_default {
                super::durable_wake_tests::after("meta_reuse_before_link").await;
            }
            let code = new_link_code();
            let hash = sha256_hex(code.as_bytes());
            let existing = if guarded_default && existing.transport == "direct" {
                let changed = existing.agent_id.as_deref() != Some(agent.id.as_str())
                    && !(existing.agent_id.is_none() && agent.is_nyxbot());
                let (current, _) = super::nyxbot_channel_setup_service::mutate_meta_assignment(
                    state,
                    owner,
                    &existing,
                    Some(agent),
                    None,
                    Some((&hash, agent.is_nyxbot())),
                )
                .await?;
                if changed {
                    super::nyxbot_channel_setup_service::after_link(state, owner, &current, agent)
                        .await;
                }
                #[cfg(test)]
                super::durable_wake_tests::after("meta_reuse_after_commit").await;
                current
            } else {
                link(state, owner, &existing.id, agent).await?;
                crate::services::nyxbot_channel_continuity_service::restore_existing(
                    &state.db,
                    &existing.id,
                    owner,
                    agent.is_nyxbot(),
                    &hash,
                    None,
                )
                .await?;
                load_channel(state, owner, &existing.id).await?
            };
            let link = LinkInstructions {
                url: link_url(&existing, &code),
                code,
                expires_at: existing
                    .link_code_expires_at
                    .ok_or_else(|| AppError::Internal("Owner link unavailable".into()))?,
            };
            return Ok((existing, link));
        }
        // A half-created or broken connection (e.g. a key was deleted) is
        // released first, then rebuilt from scratch.
        rebuilt_from = Some(existing.clone());
        #[cfg(test)]
        super::durable_wake_tests::after("channel_repair_before_retirement").await;
        let callback = (guarded_default && existing.transport == "direct").then(|| {
            super::nyxbot_channel_query_service::direct_callback(
                &state.config.base_url,
                &existing.id,
            )
        });
        super::nyxbot_channel_setup_service::disconnect_for_repair(
            state,
            owner,
            &existing,
            callback.as_deref(),
        )
        .await?;
    }
    // Never silently replace another agent's default route.
    let routes =
        channel_routing_service::list_conversations(&state.db, &bot_owner_id, Some(&bot.id))
            .await?;
    if routes
        .iter()
        .any(|route| route.default_agent && route.is_active)
    {
        return Err(AppError::Conflict(
            "This bot already routes to another agent. Remove that route first \
            (nyxid__delete_channel_route), then connect NyxBot."
                .into(),
        ));
    }
    let platform = canonical_platform(&bot.platform).to_owned();
    // The gateway creator/provider belongs to the linking person; the bot's
    // route remains owned by the org when this is an organization bot.
    // Platforms newly enabled for the gateway fall back to NyxID's relay when
    // the gateway cannot take them (Telegram keeps failing loudly).
    let wants_gateway = gateway_enabled(state, owner, &platform).await?;
    let may_fall_back = wants_gateway && platform != "telegram";
    let now = Utc::now();
    let id = Uuid::new_v4().to_string();
    let label = excerpt(&bot.label, 60);
    let direct_callback =
        super::nyxbot_channel_query_service::direct_callback(&state.config.base_url, &id);
    let gateway_url = if wants_gateway {
        match gateway_base(state).await {
            Ok(url) => Some(url),
            Err(_) if may_fall_back => None,
            Err(error) => return Err(error),
        }
    } else {
        None
    };
    let gateway = gateway_url.is_some();
    let callback = match gateway_url.as_deref() {
        Some(url) => format!("{url}/callbacks/pending"),
        None => direct_callback.clone(),
    };
    let route_key = key_service::create_api_key(
        &state.db,
        &bot_owner_id,
        &format!("NyxBot channel route {}", identifier(&label)),
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
    let mut agent_key_raw: Option<Zeroizing<String>> = None;
    let mut agent_key_id = None;
    let mut agent_key_ciphertext = None;
    if gateway {
        let agent = create_gateway_agent_key(state, owner, &label).await?;
        agent_key_ciphertext = Some(
            state
                .encryption_keys
                .encrypt(agent.full_key.as_bytes())
                .await?,
        );
        agent_key_id = Some(agent.id.clone());
        agent_key_raw = Some(Zeroizing::new(agent.full_key));
    }
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
        transport: if gateway { "gateway" } else { "direct" }.into(),
        status: "pending".into(),
        last_error: None,
        route_api_key_id: route_key.id.clone(),
        route_id: None,
        agent_api_key_id: agent_key_id.clone(),
        agent_key_ciphertext,
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
        gateway_attempted_at: may_fall_back.then_some(now),
        relay_attempted_at: None,
        gateway_fallback_at: (may_fall_back && !gateway).then_some(now),
        pending_agent_api_key_id: None,
        pending_route_api_key_id: None,
        owner_sender_ids: if org_binding.is_some() {
            Vec::new()
        } else {
            known_owner_senders(state, owner, &platform, &bot.id).await
        },
        link_code_hash: None,
        link_code_expires_at: None,
        source_conversation_id: source_conversation_id.map(str::to_owned),
        agent_id: Some(agent.id.clone()),
        private_chats: org_binding.as_ref().map(|binding| {
            if binding.guests.enabled {
                "everyone"
            } else {
                "owner"
            }
            .to_owned()
        }),
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
        org_restore_claim_id: restoration_claim_id.map(str::to_owned),
        org_restore_lease_until: None,
        org_restored_at: restoration.then_some(now),
        created_at: now,
        updated_at: now,
    };
    let db = state.db.clone();
    let inserted = row.clone();
    #[cfg(test)]
    if let Ok(gate) = CONNECT_INSERT_GATE.try_with(std::sync::Arc::clone)
        && gate.bot_id == inserted.channel_bot_id
    {
        gate.arrived.wait().await;
        gate.release.notified().await;
    }
    let mut session = db.client().start_session().await?;
    let saved = session
        .start_transaction()
        .and_run2(async move |session| {
            let result: AppResult<()> = async {
                crate::services::platform_bot_authority::fence_personal_routes(
                    &db,
                    &inserted.channel_bot_id,
                    bot_owner(&inserted),
                    session,
                )
                .await?;
                // The bot write fence serializes concurrent connection creation.
                // Automatic completion may never replace an explicit connection.
                if db
                    .collection::<NyxbotChannel>(CHANNELS)
                    .find_one(doc! {"channel_bot_id":&inserted.channel_bot_id,
                    "status":{"$in":["pending","active"]}})
                    .session(&mut *session)
                    .await?
                    .is_some()
                {
                    return Err(AppError::Conflict(
                        "This bot already has a selected agent".into(),
                    ));
                }
                db.collection::<NyxbotChannel>(CHANNELS)
                    .insert_one(&inserted)
                    .session(&mut *session)
                    .await?;
                if inserted.org_specialist.is_some() {
                    crate::services::org_channel_service::fence_management(
                        &db,
                        session,
                        &inserted,
                        &inserted.user_id,
                    )
                    .await?;
                    crate::services::org_channel_service::management::restore_in_session(
                        &db, session, &inserted,
                    )
                    .await?;
                }
                Ok(())
            }
            .await;
            crate::services::api_key_mutation_service::transaction_result(result)
        })
        .await
        .map_err(crate::services::api_key_mutation_service::map_transaction_error);
    if let Err(error) = saved {
        let _ = key_service::delete_api_key(&state.db, &bot_owner_id, &route_key.id).await;
        if let Some(key) = &agent_key_id {
            let _ = key_service::delete_api_key(&state.db, owner, key).await;
        }
        return Err(error);
    }
    // Exercise the real restoration cleanup after the provisional row and route
    // key exist. The hook only fails; cleanup_org_restore_provisional owns the
    // rollback of the product state.
    #[cfg(test)]
    if restoration
        && RESTORE_CONNECT_FAIL_STAGE
            .try_with(|stage| *stage == RESTORE_FAIL_ROUTE)
            .unwrap_or(false)
    {
        return Err(AppError::ValidationError("route_creation_failed".into()));
    }
    let row = if org_binding.is_some() {
        load_channel(state, owner, &id).await?
    } else {
        row
    };
    let mut meta_route_error = None;
    let mut outcome = if gateway {
        effects
            .connect_gateway(
                state,
                &row,
                &bot,
                agent_key_raw
                    .as_deref()
                    .map(String::as_str)
                    .unwrap_or_default(),
                &route_key.full_key,
            )
            .await
    } else if guarded_default {
        #[cfg(test)]
        super::durable_wake_tests::after("meta_default_before_insert").await;
        match channel_routing_service::create_meta_default(
            &state.db,
            bot_owner(&row),
            &bot.id,
            &bot.platform,
            &row.route_api_key_id,
            &direct_callback,
        )
        .await
        {
            Ok(route) => Ok(route.id),
            Err(error) => {
                meta_route_error = Some(error);
                Err("meta_route_creation_failed")
            }
        }
    } else {
        connect_direct(state, &row, &bot).await
    };
    // The gateway does not take this platform yet: NyxID's relay does.
    if may_fall_back
        && gateway
        && let Err(code) = outcome
    {
        tracing::info!(code, platform = %platform, "NyxBot channel stays on NyxID's relay");
        if platform == "discord" {
            state
                .db
                .collection::<NyxbotChannel>(CHANNELS)
                .update_one(
                    doc! {"_id":&row.id},
                    doc! {"$set":{"gateway_native_activation":code}},
                )
                .await?;
        }
        outcome = effects
            .fall_back_to_direct(state, &row, &bot, &direct_callback)
            .await;
    }
    if let Ok(route_id) = &outcome {
        match org_access_with(state, &row, None).await? {
            OrgAccess::Holds => {}
            OrgAccess::Lost => {
                let mut revoked = load_channel(state, owner, &id).await?;
                revoked.route_id = Some(route_id.clone());
                Box::pin(release_org_channel(state, &revoked)).await?;
                return Err(AppError::Forbidden("org_access_lost".into()));
            }
            OrgAccess::Unavailable => return Err(AppError::SecurityReadinessUnavailable),
        }
    }
    match outcome {
        Ok(route_id) => {
            let code = new_link_code();
            #[cfg(test)]
            if restoration
                && RESTORE_CONNECT_FAIL_STAGE
                    .try_with(|stage| *stage == RESTORE_FAIL_ACTIVATION)
                    .unwrap_or(false)
            {
                return Err(AppError::ValidationError("activation_failed".into()));
            }
            #[cfg(test)]
            if guarded_default {
                super::durable_wake_tests::after("meta_default_before_activation").await;
            }
            let activation = crate::services::nyxbot_channel_continuity_service::activate(
                &state.db,
                owner,
                &id,
                crate::services::nyxbot_channel_continuity_service::ActivationBinding {
                    route_id: &route_id,
                    predecessor_id: rebuilt_from.as_ref().map(|row| row.id.as_str()),
                    default_agent: agent.is_nyxbot(),
                    link_hash: &sha256_hex(code.as_bytes()),
                    expected_callback: guarded_default.then_some(direct_callback.as_str()),
                },
            )
            .await;
            let row = match activation {
                Ok(row) => row,
                Err(error) => {
                    // The commit may have succeeded. A failed reread leaves the
                    // durable reservation to recovery, never destructive rollback.
                    let current = load_channel(state, owner, &id).await?;
                    if current.status != "active" {
                        let ids = releases::retire_connection(
                            state,
                            &current,
                            ReleaseReason::SetupFailed,
                            "failed",
                            Some("setup_interrupted"),
                        )
                        .await?;
                        releases::try_inline(state, &ids).await;
                        return Err(error);
                    }
                    if current.link_code_hash.as_deref() != Some(&sha256_hex(code.as_bytes())) {
                        return Err(AppError::Conflict(
                            "Owner link was superseded; check the current assignment".into(),
                        ));
                    }
                    current
                }
            };
            let row = if guarded_default {
                row
            } else {
                load_channel(state, owner, &id).await?
            };
            #[cfg(test)]
            if guarded_default {
                super::durable_wake_tests::after("meta_new_connect_after_commit").await;
            }
            if !restoration {
                audit(
                    state,
                    owner,
                    "nyxbot_channel_connected",
                    gateway_diagnostic_audit(
                        json!({
                            "channel_agent_id": &id, "channel_bot_id": &bot.id,
                            "platform": &platform, "transport": &row.transport,
                        }),
                        &row,
                    ),
                )
                .await;
                if let Some(creator) = bot_creator(state, owner, &bot.id).await
                    && row.owner_sender_ids.contains(&creator)
                {
                    audit(
                        state,
                        owner,
                        "nyxbot_channel_owner_linked",
                        json!({"channel_agent_id": &id, "platform": &platform,
                            "method": "bot_creator"}),
                    )
                    .await;
                }
            }
            if let Some(code) = effects.sync_gateway_groups(state, &row, true).await? {
                tracing::warn!(code, "NyxBot gateway group admission not restored");
            }
            let row = if guarded_default {
                row
            } else {
                load_channel(state, owner, &id).await?
            };
            let link = LinkInstructions {
                url: link_url(&row, &code),
                code,
                expires_at: row
                    .link_code_expires_at
                    .ok_or_else(|| AppError::Internal("Owner link unavailable".into()))?,
            };
            Ok((row, link))
        }
        Err("org_access_unavailable" | "storage_unavailable") => {
            Err(AppError::SecurityReadinessUnavailable)
        }
        Err(code) => {
            let cleanup: AppResult<()> = async {
                let current = load_channel(state, owner, &id).await?;
                let ids = releases::retire_connection(
                    state,
                    &current,
                    ReleaseReason::SetupFailed,
                    "failed",
                    Some(code),
                )
                .await?;
                releases::try_inline(state, &ids).await;
                audit(
                    state,
                    owner,
                    "nyxbot_channel_connect_failed",
                    gateway_diagnostic_audit(
                        json!({
                            "channel_agent_id": &id, "platform": &platform, "error_code": code,
                        }),
                        &current,
                    ),
                )
                .await;
                Ok(())
            }
            .await;
            if let Some(error) = meta_route_error {
                if let Err(cleanup_error) = cleanup {
                    tracing::warn!(channel_id = %id, error = ?cleanup_error,
                        "Failed Meta setup remains pending cleanup");
                }
                return Err(error);
            }
            cleanup?;
            Err(AppError::ValidationError(match code {
                "gateway_provider_not_registered" => {
                    "The Agent Event Gateway has not registered NyxBot as a provider yet \
                    (operator step). Telegram bots cannot use NyxBot until it is."
                        .into()
                }
                // Keep bounded codes through tool/watch callers. Their presentation
                // layer supplies guidance without parsing an error's prose.
                "gateway_not_authenticated"
                | "gateway_permission_required"
                | "gateway_configuration_required"
                | "gateway_unavailable" => code.into(),
                other => format!("Could not connect this bot. Try again. ({other})"),
            }))
        }
    }
}

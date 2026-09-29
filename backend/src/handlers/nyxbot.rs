//! NyxBot as the agent behind the owner's channel bots.
//!
//! Telegram bots go through the Agent Event Gateway (catalog slug `cmaeg`):
//! NyxID creates the gateway channel with the owner's dedicated agent key and
//! answers as the gateway's `nyxbot` provider (Agent Card, bindings,
//! conversations, Responses SSE, event context). Platforms the gateway relay
//! does not support yet (Lark, Feishu, Discord, Slack, WhatsApp, ...) use
//! NyxID's own relay directly. Both paths run the same NyxBot turn: one
//! orchestrator conversation per chat and sender, owned by the bot owner, with
//! Full access. Only senders verified as the owner reach it; everyone else gets
//! a short refusal and no turn.
use axum::{
    Json,
    body::{Body, Bytes},
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};
use chrono::{Duration as ChronoDuration, Utc};
use futures::TryStreamExt;
use mongodb::bson::{self, doc};
use serde::Serialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{convert::Infallible, time::Duration};
use subtle::ConstantTimeEq;
use tokio::sync::broadcast;
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::{
        api_key::ApiKey,
        assistant_conversation::{ChannelOrigin, TurnOrigin},
        channel_bot::ChannelBot,
        nyxbot_channel::{
            COLLECTION_NAME as CHANNELS, EVENTS_COLLECTION_NAME as EVENTS, NyxbotChannel,
            NyxbotEvent, NyxbotThread, NyxbotWatch, THREADS_COLLECTION_NAME as THREADS,
            WATCHES_COLLECTION_NAME as WATCHES,
        },
    },
    services::{
        assistant_nyxagent::{self as engine, TurnStart, excerpt, identifier},
        channel_bot_service, channel_routing_service, key_service,
    },
};

use super::assistant_team::{Pool, Started, start_server_turn};

pub const GATEWAY_SLUG: &str = "cmaeg";
pub const PROVIDER_SLUG: &str = "nyxbot";
pub const LINK_CODE_TTL_HOURS: i64 = 24;
pub const EVENT_RETENTION_HOURS: i64 = 24;
pub const MAX_CHANNEL_REPLY_CHARS: usize = 8000;
const GATEWAY_TIMEOUT_SECS: u64 = 20;

fn canonical_platform(platform: &str) -> &str {
    match platform {
        "telegram-new" => "telegram",
        other => other,
    }
}

/// Who owns the channel bot (and its route and route key): the org for an
/// org bot, else the channel's owner.
/// NyxBot reaches bots on these platforms through the Agent Event Gateway
/// (`NYXBOT_GATEWAY_PLATFORMS`); every other bot uses NyxID's relay.
fn gateway_supports(state: &AppState, platform: &str) -> bool {
    let platform = canonical_platform(platform);
    state
        .config
        .nyxbot_gateway_platforms
        .iter()
        .any(|listed| canonical_platform(listed) == platform)
}

/// A gateway refusal of a newly listed platform is not retried sooner than
/// this (the sweep tries daily).
const GATEWAY_RETRY_HOURS: i64 = 23;

pub(crate) fn bot_owner(row: &NyxbotChannel) -> &str {
    row.bot_owner_id.as_deref().unwrap_or(&row.user_id)
}

/// Whether `owner` may link and manage `bot`: their own bot, or a bot of an
/// organization they administer (the rule for managing org bots).
async fn manages_bot_owner(state: &AppState, owner: &str, bot_owner: &str) -> AppResult<bool> {
    if owner == bot_owner {
        return Ok(true);
    }
    Ok(matches!(
        crate::services::org_service::resolve_owner_access(&state.db, owner, bot_owner).await?,
        crate::services::org_service::OwnerAccess::AsOrgAdmin { .. }
    ))
}

/// A channel bot the owner may link: theirs or one of an org they
/// administer. Anything else is not-found-shaped.
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

/// A bot named by id or by its label (case-insensitive) among the owner's
/// bots and those of the orgs they administer.
pub(crate) async fn resolve_bot_ref(
    state: &AppState,
    owner: &str,
    reference: &str,
) -> AppResult<ChannelBot> {
    let reference = reference.trim();
    // Only the canonical hyphenated form can be a stored `_id`.
    if reference.len() == 36 && Uuid::parse_str(reference).is_ok() {
        return accessible_bot(state, owner, reference).await;
    }
    let wanted = reference.to_lowercase();
    let bots = channel_bot_service::list_all_bots(&state.db, owner).await?;
    let matches: Vec<&ChannelBot> = bots
        .iter()
        .filter(|bot| bot.label.trim().to_lowercase() == wanted)
        .collect();
    match matches.as_slice() {
        [bot] => Ok((*bot).clone()),
        [] => Err(AppError::NotFound(format!(
            "No channel bot is labelled '{}'. Available: {}",
            excerpt(reference, 60),
            bots.iter()
                .take(20)
                .map(|bot| format!(
                    "{} ({}, {})",
                    excerpt(bot.label.trim(), 40),
                    bot.platform,
                    bot.id
                ))
                .collect::<Vec<_>>()
                .join("; ")
        ))),
        several => Err(AppError::ValidationError(format!(
            "Several channel bots are labelled '{}'; use one of their ids: {}",
            excerpt(reference, 60),
            several
                .iter()
                .map(|bot| format!("{} ({})", bot.id, bot.platform))
                .collect::<Vec<_>>()
                .join(", ")
        ))),
    }
}

/// Whether an org bot's channel may still act: its owner must still
/// administer the org. Personal channels always may. Database errors are
/// errors, never a denial.
pub(crate) async fn org_access_holds(state: &AppState, row: &NyxbotChannel) -> AppResult<bool> {
    match row.bot_owner_id.as_deref() {
        Some(org) => manages_bot_owner(state, &row.user_id, org).await,
        None => Ok(true),
    }
}

/// The owner no longer administers the org that owns the channel's bot:
/// fail the link once and remove its org-owned route and route key, so the
/// org's bot is free for its admins and no traffic reaches this agent.
pub(crate) async fn release_org_channel(state: &AppState, row: &NyxbotChannel) -> AppResult<()> {
    let Some(org) = row.bot_owner_id.as_deref() else {
        return Ok(());
    };
    let failed = state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .update_one(
            doc! {"_id": &row.id, "status": {"$in": ["pending", "active"]}},
            doc! {"$set": {"status": "failed", "last_error": "org_access_lost",
            "updated_at": bson::DateTime::now()},
            "$unset": {"link_code_hash": "", "link_code_expires_at": ""}},
        )
        .await?
        .modified_count
        == 1;
    if let Some(route_id) = row.route_id.as_deref() {
        match channel_routing_service::delete_conversation(&state.db, route_id, org).await {
            Ok(()) | Err(AppError::NotFound(_)) => {}
            Err(error) => return Err(error),
        }
    }
    match key_service::delete_api_key(&state.db, org, &row.route_api_key_id).await {
        Ok(()) | Err(AppError::NotFound(_)) => {}
        Err(error) => return Err(error),
    }
    if failed {
        audit(
            state,
            &row.user_id,
            "nyxbot_channel_org_access_lost",
            json!({"channel_agent_id": &row.id, "platform": &row.platform}),
        )
        .await;
    }
    Ok(())
}

fn sha256_hex(value: impl AsRef<[u8]>) -> String {
    hex::encode(Sha256::digest(value.as_ref()))
}

// ---------------------------------------------------------------------------
// Gateway client (server-to-server, the owner's dedicated agent key as bearer)
// ---------------------------------------------------------------------------

/// The gateway's `/v1` base from the `cmaeg` catalog row: no new environment
/// variable, and the same origin the NyxID proxy already uses.
async fn gateway_base(state: &AppState) -> AppResult<String> {
    let row =
        crate::services::assistant_service::resolve_admin_service_by_slug(&state.db, GATEWAY_SLUG)
            .await
            .map_err(|_| AppError::ValidationError("gateway_unavailable".into()))?;
    let base = row.base_url.trim_end_matches('/').to_owned();
    let parsed = url::Url::parse(&base)
        .map_err(|_| AppError::ValidationError("gateway_unavailable".into()))?;
    let local = matches!(parsed.host_str(), Some("localhost" | "127.0.0.1" | "[::1]"));
    if !(parsed.scheme() == "https" || (local && parsed.scheme() == "http")) {
        return Err(AppError::ValidationError("gateway_unavailable".into()));
    }
    Ok(base)
}

/// Acting service recorded on creator tokens (`act.sub`).
const CREATOR_CLIENT_ID: &str = "nyxbot";
/// A creator token covers one channel-management sequence.
const CREATOR_TOKEN_TTL_SECS: i64 = 120;

/// The owner's bearer for gateway channel management. The gateway resolves
/// the creator with `GET /api/v1/users/me`, which refuses API keys, and requires
/// `profile.metadata.owner.subject` to match. NyxID therefore mints a
/// short-lived delegated token carrying only `account:read`, the narrowest
/// credential that endpoint accepts. It is sent only to the gateway and is
/// never logged or stored.
fn creator_bearer(state: &AppState, owner: &str) -> AppResult<Zeroizing<String>> {
    let user_id =
        Uuid::parse_str(owner).map_err(|_| AppError::Internal("Invalid channel owner".into()))?;
    crate::crypto::jwt::generate_delegated_access_token(
        &state.jwt_keys,
        &state.config,
        &user_id,
        crate::mw::auth::ACCOUNT_READ_SCOPE,
        CREATOR_CLIENT_ID,
        CREATOR_TOKEN_TTL_SECS,
        None,
    )
    .map(Zeroizing::new)
}

struct GatewayResponse {
    status: u16,
    body: Value,
}

async fn gateway_call(
    state: &AppState,
    method: reqwest::Method,
    path: &str,
    bearer: &str,
    body: Option<&Value>,
    idempotency_key: Option<&str>,
) -> AppResult<GatewayResponse> {
    let base = gateway_base(state).await?;
    let mut request = state
        .http_client
        .request(method, format!("{base}{path}"))
        .bearer_auth(bearer)
        .timeout(Duration::from_secs(GATEWAY_TIMEOUT_SECS));
    if let Some(body) = body {
        request = request.json(body);
    }
    if let Some(key) = idempotency_key {
        request = request.header("Idempotency-Key", key);
    }
    let response = request
        .send()
        .await
        .map_err(|_| AppError::ValidationError("gateway_unavailable".into()))?;
    let status = response.status().as_u16();
    let bytes = response.bytes().await.unwrap_or_default();
    let body =
        serde_json::from_slice(&bytes[..bytes.len().min(2 * 1024 * 1024)]).unwrap_or(Value::Null);
    Ok(GatewayResponse { status, body })
}

/// Stable, prose-free code for a failed gateway call.
fn gateway_error_code(response: &GatewayResponse) -> &'static str {
    let code = response.body["code"].as_str().unwrap_or_default();
    let detail = response.body["detail"].as_str().unwrap_or_default();
    match (response.status, code) {
        (400, "invalid_request") if detail.contains("provider") => {
            "gateway_provider_not_registered"
        }
        (400, _) => "gateway_rejected_request",
        (401, _) | (403, _) => "gateway_not_authenticated",
        (409, _) => "gateway_conflict",
        (502, _) => "gateway_provider_rejected",
        _ => "gateway_unavailable",
    }
}

// ---------------------------------------------------------------------------
// Connect, list, disconnect
// ---------------------------------------------------------------------------

#[derive(Serialize)]
pub struct ChannelAgentResponse {
    id: String,
    channel_bot_id: String,
    platform: String,
    bot_label: String,
    bot_username: Option<String>,
    transport: String,
    status: String,
    last_error: Option<String>,
    owner_linked: bool,
    /// The agent this bot reaches; `None` means the owner's NyxBot.
    agent_id: Option<String>,
    /// The organization that owns the bot; `None` for the owner's own bot.
    org_id: Option<String>,
    /// Who may talk to the agent in private chats: `owner` or `everyone`.
    private_chats: String,
    /// `ok` or `failing` once a message has been judged; `None` before.
    delivery_status: Option<String>,
    /// Stable code of the newest lost message (see `delivery_reason`).
    delivery_error: Option<String>,
    /// Plain words for `delivery_error`.
    delivery_reason: Option<String>,
    delivery_failed_at: Option<chrono::DateTime<Utc>>,
    created_at: chrono::DateTime<Utc>,
}
impl From<&NyxbotChannel> for ChannelAgentResponse {
    fn from(row: &NyxbotChannel) -> Self {
        Self {
            id: row.id.clone(),
            channel_bot_id: row.channel_bot_id.clone(),
            platform: row.platform.clone(),
            bot_label: row.bot_label.clone(),
            bot_username: row.bot_username.clone(),
            transport: row.transport.clone(),
            status: row.status.clone(),
            last_error: row.last_error.clone(),
            owner_linked: !row.owner_sender_ids.is_empty(),
            agent_id: row.agent_id.clone(),
            org_id: row.bot_owner_id.clone(),
            private_chats: row.private_chats.clone().unwrap_or_else(|| "owner".into()),
            delivery_status: row.delivery_status.clone(),
            delivery_error: row.delivery_error.clone(),
            delivery_reason: row
                .delivery_error
                .as_deref()
                .map(|code| status::failure_reason(code, &row.transport)),
            delivery_failed_at: row.delivery_failed_at,
            created_at: row.created_at,
        }
    }
}

/// A fresh one-time code the owner sends from the chat app to prove the
/// sender account is theirs.
pub struct LinkInstructions {
    pub code: Zeroizing<String>,
    pub url: Option<String>,
    pub expires_at: chrono::DateTime<Utc>,
}

fn new_link_code() -> Zeroizing<String> {
    use rand::Rng;
    const ALPHABET: &[u8] = b"abcdefghjkmnpqrstuvwxyz23456789";
    let mut rng = rand::thread_rng();
    let tail: String = (0..12)
        .map(|_| ALPHABET[rng.gen_range(0..ALPHABET.len())] as char)
        .collect();
    Zeroizing::new(format!("nyxlink_{tail}"))
}

fn link_url(row: &NyxbotChannel, code: &str) -> Option<String> {
    match (row.platform.as_str(), row.bot_username.as_deref()) {
        ("telegram", Some(username)) if !username.is_empty() => {
            Some(format!("https://t.me/{username}?start={code}"))
        }
        _ => None,
    }
}

async fn refresh_link_code(state: &AppState, row: &NyxbotChannel) -> AppResult<LinkInstructions> {
    let code = new_link_code();
    let expires_at = Utc::now() + ChronoDuration::hours(LINK_CODE_TTL_HOURS);
    state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .update_one(
            doc! {"_id": &row.id, "user_id": &row.user_id},
            doc! {"$set": {"link_code_hash": sha256_hex(code.as_bytes()),
            "link_code_expires_at": bson::DateTime::from_chrono(expires_at),
            "updated_at": bson::DateTime::now()}},
        )
        .await?;
    Ok(LinkInstructions {
        url: link_url(row, &code),
        code,
        expires_at,
    })
}

async fn load_channel(state: &AppState, owner: &str, id: &str) -> AppResult<NyxbotChannel> {
    if Uuid::parse_str(id).is_err() {
        return Err(AppError::NotFound("Channel agent not found".into()));
    }
    state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .find_one(doc! {"_id": id, "user_id": owner})
        .await?
        .ok_or_else(|| AppError::NotFound("Channel agent not found".into()))
}

async fn active_for_bot(
    state: &AppState,
    owner: &str,
    bot_id: &str,
) -> AppResult<Option<NyxbotChannel>> {
    Ok(state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .find_one(doc! {"user_id": owner, "channel_bot_id": bot_id,
        "status": {"$in": ["pending", "active"]}})
        .await?)
}

/// Owner-verified senders from NyxID's own Telegram notification link.
async fn known_owner_senders(state: &AppState, owner: &str, platform: &str) -> Vec<String> {
    if platform != "telegram" {
        return Vec::new();
    }
    state
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
        .unwrap_or_default()
}

/// The channel's gateway agent key. It is the creator bearer on channel
/// management (the gateway resolves the owner through `GET /api/v1/users/me`)
/// and the bearer the gateway presents to NyxID as provider.
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
        None,
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

/// Link one of the owner's channel bots to an agent (NyxBot or a specialist).
/// Idempotent: an active connection is relinked to `agent` if needed and
/// returns itself with a fresh link code.
pub async fn connect(
    state: &AppState,
    owner: &str,
    source_conversation_id: Option<&str>,
    bot_id: &str,
    agent: &crate::models::assistant_agent::AssistantAgent,
) -> AppResult<(NyxbotChannel, LinkInstructions)> {
    if agent.destroyed_at.is_some() {
        return Err(AppError::Conflict("That agent was destroyed".into()));
    }
    let bot: ChannelBot = accessible_bot(state, owner, bot_id).await?;
    // An org bot's route and route key belong to the org.
    let bot_owner_id = bot.user_id.clone();
    if !bot.is_active {
        return Err(AppError::ValidationError(
            "That channel bot is not active".into(),
        ));
    }
    // Owners verified on a connection being rebuilt stay verified: the bot
    // and their chat-app account are the same.
    let mut verified_owners: Vec<String> = Vec::new();
    let mut rebuilt_from = None;
    if let Some(existing) = active_for_bot(state, owner, &bot.id).await? {
        let keys_alive =
            key_service::get_api_key(&state.db, bot_owner(&existing), &existing.route_api_key_id)
                .await
                .is_ok_and(|key| key.is_active)
                && match existing.agent_api_key_id.as_deref() {
                    Some(agent) => key_service::get_api_key(&state.db, owner, agent)
                        .await
                        .is_ok_and(|key| key.is_active),
                    None => true,
                };
        // A channel whose messages stopped arriving, whose bot changed owner,
        // or whose route is gone is rebuilt from scratch.
        let route_alive = match existing.route_id.as_deref() {
            Some(route_id) => {
                state
                    .db
                    .collection::<bson::Document>(
                        crate::models::channel_conversation::COLLECTION_NAME,
                    )
                    .count_documents(doc! {"_id": route_id, "user_id": &bot_owner_id,
                    "is_active": true})
                    .await?
                    > 0
            }
            None => false,
        };
        // A bot whose platform the gateway now relays moves there, unless the
        // gateway refused it recently.
        let wanted = if gateway_supports(state, &bot.platform) && bot_owner_id == owner {
            "gateway"
        } else {
            "direct"
        };
        let transport_ok = existing.transport == wanted
            || (wanted == "gateway"
                && existing.gateway_fallback_at.is_some_and(|at| {
                    at > Utc::now() - ChronoDuration::hours(GATEWAY_RETRY_HOURS)
                }));
        let healthy = existing.delivery_status.as_deref() != Some("failing")
            && bot_owner(&existing) == bot_owner_id
            && route_alive
            && transport_ok;
        if existing.status == "active" && keys_alive && healthy {
            link(state, owner, &existing.id, agent).await?;
            let existing = load_channel(state, owner, &existing.id).await?;
            let link = refresh_link_code(state, &existing).await?;
            return Ok((existing, link));
        }
        // A half-created or broken connection (e.g. a key was deleted) is
        // released first, then rebuilt from scratch.
        verified_owners = existing.owner_sender_ids.clone();
        rebuilt_from = Some(existing.clone());
        disconnect(state, owner, &existing.id).await?;
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
    // The gateway binds a channel to one person; org bots use NyxID's relay.
    // Platforms newly listed for the gateway fall back to NyxID's relay when
    // the gateway cannot take them (Telegram keeps failing loudly).
    let wants_gateway = gateway_supports(state, &platform) && bot_owner_id == owner;
    let may_fall_back = wants_gateway && platform != "telegram";
    let now = Utc::now();
    let id = Uuid::new_v4().to_string();
    let label = excerpt(&bot.label, 60);
    let direct_callback = format!(
        "{}/api/v1/nyxbot/relay/{id}",
        state.config.base_url.trim_end_matches('/')
    );
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
        None,
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
        gateway_version: None,
        binding_id: None,
        gateway_groups: None,
        gateway_groups_retry_at: None,
        gateway_bot_id: None,
        gateway_attempted_at: may_fall_back.then_some(now),
        gateway_fallback_at: (may_fall_back && !gateway).then_some(now),
        owner_sender_ids: {
            let mut ids = known_owner_senders(state, owner, &platform).await;
            for id in verified_owners {
                if !ids.contains(&id) {
                    ids.push(id);
                }
            }
            ids
        },
        link_code_hash: None,
        link_code_expires_at: None,
        source_conversation_id: source_conversation_id.map(str::to_owned),
        agent_id: Some(agent.id.clone()),
        private_chats: None,
        delivery_status: None,
        delivery_error: None,
        delivery_failed_at: None,
        delivery_seen_at: None,
        delivery_checked_at: None,
        delivery_notified_at: None,
        created_at: now,
        updated_at: now,
    };
    state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .insert_one(&row)
        .await?;
    let mut outcome = if gateway {
        connect_gateway(
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
    } else {
        connect_direct(state, &row, &bot).await
    };
    // The gateway does not take this platform yet: NyxID's relay does.
    if may_fall_back
        && gateway
        && let Err(code) = outcome
    {
        tracing::info!(code, platform = %platform, "NyxBot channel stays on NyxID's relay");
        outcome = fall_back_to_direct(
            state,
            owner,
            &row,
            &bot,
            &direct_callback,
            agent_key_id.as_deref(),
        )
        .await;
    }
    match outcome {
        Ok(route_id) => {
            state
                .db
                .collection::<NyxbotChannel>(CHANNELS)
                .update_one(
                    doc! {"_id": &id},
                    doc! {"$set": {"status": "active", "route_id": &route_id,
                    "updated_at": bson::DateTime::now()}},
                )
                .await?;
            audit(
                state,
                owner,
                "nyxbot_channel_connected",
                json!({
                    "channel_agent_id": &id, "channel_bot_id": &bot.id,
                    "platform": &platform, "transport": &row.transport,
                }),
            )
            .await;
            // A rebuilt connection keeps its chats, their settings and threads.
            if let Some(previous) = rebuilt_from.as_ref() {
                let reaches = |row: &NyxbotChannel| row.agent_id.clone();
                let agent_changed = reaches(previous).unwrap_or_default() != agent.id
                    && !(previous.agent_id.is_none() && agent.is_nyxbot());
                chats::carry_over(state, owner, previous, &id, agent_changed).await?;
                if let Some(access) = previous.private_chats.as_deref() {
                    state
                        .db
                        .collection::<NyxbotChannel>(CHANNELS)
                        .update_one(doc! {"_id": &id}, doc! {"$set": {"private_chats": access}})
                        .await?;
                }
            }
            let row = load_channel(state, owner, &id).await?;
            if let Some(code) = chats::sync_gateway_groups(state, &row, true).await? {
                tracing::warn!(code, "NyxBot gateway group admission not restored");
            }
            let row = load_channel(state, owner, &id).await?;
            let link = refresh_link_code(state, &row).await?;
            Ok((row, link))
        }
        Err(code) => {
            // Best-effort rollback of everything this attempt created.
            let _ = key_service::delete_api_key(&state.db, &bot_owner_id, &route_key.id).await;
            if let Some(agent) = &agent_key_id {
                let _ = key_service::delete_api_key(&state.db, owner, agent).await;
            }
            state
                .db
                .collection::<NyxbotChannel>(CHANNELS)
                .update_one(
                    doc! {"_id": &id},
                    doc! {"$set": {"status": "failed", "last_error": code,
                    "updated_at": bson::DateTime::now()},
                    "$unset": {"agent_key_ciphertext": ""}},
                )
                .await?;
            audit(
                state,
                owner,
                "nyxbot_channel_connect_failed",
                json!({
                    "channel_agent_id": &id, "platform": &platform, "error_code": code,
                }),
            )
            .await;
            Err(AppError::ValidationError(match code {
                "gateway_provider_not_registered" => {
                    "The Agent Event Gateway has not registered NyxBot as a provider yet \
                    (operator step). Telegram bots cannot use NyxBot until it is."
                        .into()
                }
                other => format!("Connecting NyxBot failed: {other}"),
            }))
        }
    }
}

/// Gateway group admission unless one of the channel's chats answers every
/// message (see `chats::sync_gateway_groups`).
pub(crate) const GATEWAY_GROUPS_DEFAULT: &str = "mention_or_reply_to_bot";

fn gateway_policy(
    state: &AppState,
    row: &NyxbotChannel,
    bot: &ChannelBot,
    route_ids: &[String],
    groups: &str,
) -> Value {
    let username = bot.platform_bot_username.trim_start_matches('@');
    let platform = canonical_platform(&row.platform);
    let mut source = json!({
        "type": "nyxid_relay",
        "issuer": state.config.jwt_issuer,
        "key_id": row.route_api_key_id,
        "route_ids": route_ids,
        "platform": platform,
        "admission": {"type": "scoped", "senders": {"type": "open"},
            "chats": {"type": "open"}, "groups": groups},
    });
    // Telegram recognises mentions by username; other platforms by the bot's
    // own user ID, when NyxID could look it up.
    if platform == "telegram" {
        let valid_username = (5..=32).contains(&username.len())
            && username
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_');
        if valid_username {
            source["bot_username"] = json!(username);
        }
    } else if let Some(bot_id) = row.gateway_bot_id.as_deref() {
        source["bot_id"] = json!(bot_id);
    }
    let name: String = format!("NyxBot {}", row.bot_label)
        .chars()
        .take(60)
        .collect();
    json!({
        "schema_version": 1,
        "name": name,
        "profile": {
            "schema_version": 1,
            "metadata": {"name": name, "description": "NyxBot, the owner's NyxID assistant",
                "owner": {"subject": row.user_id, "kind": "human"}, "provider": PROVIDER_SLUG},
            "model": {"access": "managed", "service": engine::SERVICE_SLUG,
                "api_type": "responses", "model": engine::DEFAULT_MODEL,
                "thinking_effort": "medium"},
            "permission": {"action_mode": "autonomous", "service_scope": []},
            "context": {"system_prompt": "", "skills": [], "mcp_servers": []},
        },
        "provider": {"slug": PROVIDER_SLUG},
        "sources": [source],
        "partition": {"version": 1, "key": "conversation_and_sender"},
        "reply": {"auto_final": true, "ack_after_ms": 2500, "sinks": [{"type": "source"}]},
    })
}

/// Create the gateway channel, point the route key at it, create the route,
/// then attach the route. Returns the route ID or a stable error code.
async fn connect_gateway(
    state: &AppState,
    row: &NyxbotChannel,
    bot: &ChannelBot,
    agent_key: &str,
    route_key: &str,
) -> Result<String, &'static str> {
    if !(state.config.jwt_issuer.starts_with("https://")
        || state.config.jwt_issuer.starts_with("http://localhost"))
    {
        return Err("issuer_unsupported");
    }
    let creator = creator_bearer(state, &row.user_id).map_err(|_| "creator_unavailable")?;
    let creator = creator.as_str();
    let record_id = Uuid::new_v4().to_string();
    // Other platforms pin the bot's own user ID so mentions of it are known.
    let mut row = row.clone();
    if row.platform != "telegram"
        && let Some(bot_id) = bot_user_id(state, bot).await
    {
        let _ = state
            .db
            .collection::<NyxbotChannel>(CHANNELS)
            .update_one(
                doc! {"_id": &row.id},
                doc! {"$set": {"gateway_bot_id": &bot_id}},
            )
            .await;
        row.gateway_bot_id = Some(bot_id);
    }
    let row = &row;
    let mut body = gateway_policy(state, row, bot, &[], GATEWAY_GROUPS_DEFAULT);
    body["record_id"] = json!(record_id);
    body["credentials"] = json!({"agent_key": agent_key, "channel_key": route_key});
    let created = gateway_call(
        state,
        reqwest::Method::POST,
        "/channels",
        creator,
        Some(&body),
        Some(&record_id),
    )
    .await
    .map_err(|_| "gateway_unavailable")?;
    if created.status != 201 && created.status != 200 {
        return Err(gateway_error_code(&created));
    }
    let channel_id = created.body["channel_id"]
        .as_str()
        .ok_or("gateway_invalid_response")?;
    let callback = created.body["endpoints"]["nyxid_callback_url"]
        .as_str()
        .ok_or("gateway_invalid_response")?;
    let version = created.body["version"]
        .as_i64()
        .ok_or("gateway_invalid_response")?;
    let binding_id = created.body["provider"]["binding_id"]
        .as_str()
        .map(str::to_owned);
    let db = &state.db;
    db.collection::<NyxbotChannel>(CHANNELS)
        .update_one(
            doc! {"_id": &row.id},
            doc! {"$set": {"gateway_channel_id": channel_id, "gateway_record_id": &record_id,
            "gateway_version": version, "binding_id": binding_id}},
        )
        .await
        .map_err(|_| "storage_unavailable")?;
    let failed = |code: &'static str| async move {
        // Release the gateway channel before surfacing the failure.
        let _ = gateway_call(
            state,
            reqwest::Method::DELETE,
            &format!(
                "/channels/{}?expected_version={version}",
                urlencode(channel_id)
            ),
            creator,
            None,
            None,
        )
        .await;
        code
    };
    if key_service::update_api_key_scope_with_scope_authorization(
        db,
        &row.user_id,
        None,
        &row.route_api_key_id,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        Some(Some(callback)),
        None,
    )
    .await
    .is_err()
    {
        return Err(failed("route_key_update_failed").await);
    }
    let route = match channel_routing_service::create_conversation(
        db,
        &row.user_id,
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
    {
        Ok(route) => route,
        Err(_) => return Err(failed("route_create_failed").await),
    };
    let mut update = gateway_policy(
        state,
        row,
        bot,
        std::slice::from_ref(&route.id),
        GATEWAY_GROUPS_DEFAULT,
    );
    update["expected_version"] = json!(version);
    let attached = gateway_call(
        state,
        reqwest::Method::PUT,
        &format!("/channels/{}", urlencode(channel_id)),
        creator,
        Some(&update),
        None,
    )
    .await;
    match attached {
        Ok(response) if response.status == 200 => {
            let version = response.body["version"].as_i64().unwrap_or(version + 1);
            let _ = db
                .collection::<NyxbotChannel>(CHANNELS)
                .update_one(
                    doc! {"_id": &row.id},
                    // A new gateway channel starts with the default admission.
                    doc! {"$set": {"gateway_version": version},
                    "$unset": {"gateway_groups": ""}},
                )
                .await;
            Ok(route.id)
        }
        Ok(response) => {
            let _ = channel_routing_service::delete_conversation(db, &route.id, &row.user_id).await;
            Err(failed(gateway_error_code(&response)).await)
        }
        Err(_) => {
            let _ = channel_routing_service::delete_conversation(db, &route.id, &row.user_id).await;
            Err(failed("gateway_unavailable").await)
        }
    }
}

/// The bot's own user ID on its platform (best effort).
async fn bot_user_id(state: &AppState, bot: &ChannelBot) -> Option<String> {
    let lookup = async {
        let adapter = crate::services::channel_adapters::resolve_adapter(
            &bot.platform,
            &state.token_exchange_cache,
        )?;
        let token = crate::services::channel_credentials::resolve_bot_token(
            &state.db,
            &state.encryption_keys,
            adapter.as_ref(),
            bot,
        )
        .await?;
        adapter
            .bot_user_id(
                &state.http_client,
                &crate::services::channel_platform::BotCredentials {
                    billing: None,
                    token: &token,
                    platform_bot_id: Some(&bot.platform_bot_id),
                    platform_secrets: None,
                },
            )
            .await
    };
    match tokio::time::timeout(Duration::from_secs(5), lookup).await {
        Ok(Ok(id)) => id,
        _ => None,
    }
}

/// Move one personal bot still on NyxID's relay onto the gateway once its
/// platform is listed in `NYXBOT_GATEWAY_PLATFORMS`: at most one per sweep,
/// and each bot at most daily (a refusal leaves it on NyxID's relay). The
/// rebuild keeps its verified owners, chats, settings and private-chat access.
pub(crate) async fn switch_to_gateway(state: &AppState) -> AppResult<()> {
    let listed: Vec<&str> = state
        .config
        .nyxbot_gateway_platforms
        .iter()
        .map(|platform| canonical_platform(platform))
        .filter(|platform| *platform != "telegram")
        .collect();
    if listed.is_empty() {
        return Ok(());
    }
    let due = bson::DateTime::from_chrono(Utc::now() - ChronoDuration::hours(GATEWAY_RETRY_HOURS));
    let filter = doc! {"status": "active", "transport": "direct",
    "bot_owner_id": bson::Bson::Null, "platform": {"$in": &listed},
    "$or": [{"gateway_attempted_at": bson::Bson::Null},
        {"gateway_attempted_at": {"$lt": due}}]};
    let channels = state.db.collection::<NyxbotChannel>(CHANNELS);
    // Claim it, so replicas do not rebuild the same bot at once.
    let Some(row) = channels
        .find_one_and_update(
            filter,
            doc! {"$set": {"gateway_attempted_at": bson::DateTime::now()}},
        )
        .await?
    else {
        return Ok(());
    };
    let agent = match row.agent_id.as_deref() {
        Some(agent_id) => {
            crate::services::assistant_team_service::agent(&state.db, &row.user_id, agent_id)
                .await?
        }
        None => {
            crate::services::assistant_team_service::ensure_nyxbot(&state.db, &row.user_id).await?
        }
    };
    let (moved, _) = connect(
        state,
        &row.user_id,
        row.source_conversation_id.as_deref(),
        &row.channel_bot_id,
        &agent,
    )
    .await?;
    audit(
        state,
        &row.user_id,
        "nyxbot_channel_transport_checked",
        json!({"channel_agent_id": &moved.id, "platform": &moved.platform,
            "transport": &moved.transport}),
    )
    .await;
    Ok(())
}

/// Turn a channel whose gateway setup failed into a NyxID relay channel: its
/// route key calls NyxID, the gateway agent key goes.
async fn fall_back_to_direct(
    state: &AppState,
    owner: &str,
    row: &NyxbotChannel,
    bot: &ChannelBot,
    callback: &str,
    agent_key: Option<&str>,
) -> Result<String, &'static str> {
    if let Some(agent_key) = agent_key {
        let _ = key_service::delete_api_key(&state.db, owner, agent_key).await;
    }
    key_service::update_api_key_scope_with_scope_authorization(
        &state.db,
        bot_owner(row),
        None,
        &row.route_api_key_id,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        Some(Some(callback)),
        None,
    )
    .await
    .map_err(|_| "route_key_update_failed")?;
    state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .update_one(
            doc! {"_id": &row.id},
            doc! {"$set": {"transport": "direct",
                "gateway_fallback_at": bson::DateTime::now()},
            "$unset": {"agent_api_key_id": "", "agent_key_ciphertext": "",
                "gateway_channel_id": "", "gateway_record_id": "", "gateway_version": "",
                "binding_id": "", "gateway_bot_id": ""}},
        )
        .await
        .map_err(|_| "storage_unavailable")?;
    let mut direct = row.clone();
    direct.transport = "direct".into();
    direct.agent_api_key_id = None;
    connect_direct(state, &direct, bot).await
}

async fn connect_direct(
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

fn urlencode(value: &str) -> String {
    url::form_urlencoded::byte_serialize(value.as_bytes()).collect()
}

async fn decrypt_agent_key(
    state: &AppState,
    row: &NyxbotChannel,
) -> AppResult<Option<Zeroizing<String>>> {
    let Some(ciphertext) = row.agent_key_ciphertext.as_ref() else {
        return Ok(None);
    };
    let bytes = Zeroizing::new(state.encryption_keys.decrypt(ciphertext).await?);
    Ok(Some(Zeroizing::new(
        String::from_utf8(bytes.to_vec())
            .map_err(|_| AppError::Internal("Channel key unavailable".into()))?,
    )))
}

/// Stop NyxBot answering a bot: delete the gateway channel (gateway path),
/// the route and both keys. Chats stay in the owner's history.
pub async fn disconnect(state: &AppState, owner: &str, id: &str) -> AppResult<Value> {
    let row = load_channel(state, owner, id).await?;
    // Release the gateway channel as its creator. If the gateway refuses, the
    // orphan has no route, key or provider binding left to reach, so local
    // cleanup proceeds regardless.
    let mut gateway_released = row.transport != "gateway";
    if row.transport == "gateway"
        && let (Some(channel_id), Some(version)) =
            (row.gateway_channel_id.as_deref(), row.gateway_version)
    {
        let creator = creator_bearer(state, owner)?;
        gateway_released = gateway_call(
            state,
            reqwest::Method::DELETE,
            &format!(
                "/channels/{}?expected_version={version}",
                urlencode(channel_id)
            ),
            creator.as_str(),
            None,
            None,
        )
        .await
        .is_ok_and(|response| matches!(response.status, 204 | 200 | 404));
    }
    if let Some(route_id) = row.route_id.as_deref() {
        match channel_routing_service::delete_conversation(&state.db, route_id, bot_owner(&row))
            .await
        {
            Ok(()) | Err(AppError::NotFound(_)) => {}
            Err(error) => return Err(error),
        }
    }
    // The route key belongs to the bot's owner (an org for org bots); the
    // gateway agent key (personal bots only) to the channel's owner.
    for (key, key_owner) in [
        (Some(row.route_api_key_id.as_str()), bot_owner(&row)),
        (row.agent_api_key_id.as_deref(), owner),
    ] {
        let Some(key) = key else { continue };
        match key_service::delete_api_key(&state.db, key_owner, key).await {
            Ok(()) | Err(AppError::NotFound(_)) => {}
            Err(error) => return Err(error),
        }
    }
    state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .update_one(
            doc! {"_id": &row.id, "user_id": owner},
            doc! {"$set": {"status": "disconnected", "updated_at": bson::DateTime::now()},
            "$unset": {"agent_key_ciphertext": "", "link_code_hash": ""}},
        )
        .await?;
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

pub async fn list(state: &AppState, owner: &str) -> AppResult<Vec<NyxbotChannel>> {
    Ok(state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .find(doc! {"user_id": owner, "status": {"$ne": "disconnected"}})
        .sort(doc! {"created_at": -1})
        .limit(50)
        .await?
        .try_collect()
        .await?)
}

fn link_json(row: &NyxbotChannel, link: &LinkInstructions) -> Value {
    json!({
        "code": link.code.as_str(),
        "url": link.url,
        "expires_at": link.expires_at,
        "instructions": match &link.url {
            Some(_) => "Open the link on the phone or computer where you use Telegram and \
                press Start. That links your Telegram account as NyxBot's owner.".to_owned(),
            None => format!("Send this code to the bot once from your own {} account: {}",
                row.platform, link.code.as_str()),
        },
    })
}

pub(crate) async fn connect_tool(
    state: &AppState,
    owner: &str,
    source_conversation_id: &str,
    bot_id: &str,
    agent: &crate::models::assistant_agent::AssistantAgent,
) -> AppResult<(Value, bool)> {
    let (row, link) = connect(state, owner, Some(source_conversation_id), bot_id, agent).await?;
    // Linking an existing bot answers a setup link this chat handed out for
    // the same platform: it is no longer waiting for a new bot.
    state
        .db
        .collection::<NyxbotWatch>(WATCHES)
        .update_many(
            doc! {"user_id": owner, "kind": "channel_bot", "conversation_id": source_conversation_id,
            "platform": &row.platform, "status": "pending"},
            doc! {"$set": {"status": "done", "channel_bot_id": &row.channel_bot_id}},
        )
        .await?;
    let mut value = json!({
        "channel_agent": ChannelAgentResponse::from(&row),
        "agent": agent.name,
        "link": link_json(&row, &link),
        "note": if row.owner_sender_ids.is_empty() {
            "Give the user the link (or code). Until they use it, the bot answers nobody."
        } else {
            "The user's linked account already reaches the bot; the link adds another."
        },
    });
    // The same app registered as two NyxID bots: its events reach only one.
    if let Some(twin) = status::twin_bot(state, &row).await? {
        value["duplicate_bot"] = json!({"id": twin.id, "label": excerpt(twin.label.trim(), 60)});
        let point = if status::manual_webhook(&row.platform) {
            format!(
                "point the app's event subscription at this bot ({}) and publish a new app version",
                status::webhook_url(state, &row)
            )
        } else {
            "keep only one of them".to_owned()
        };
        value["duplicate_bot_note"] = json!(format!(
            "Another NyxID bot, {} ({}), is registered for the same app, and the app delivers \
            events to only one of them. Either {point}, or link the agent to that bot instead \
            and delete the duplicate.",
            identifier(&twin.label),
            twin.id,
        ));
    }
    // Chat-specific routes win over NyxBot's default route: say so now,
    // instead of the owner's messages silently going elsewhere.
    let others = other_routes(state, &row).await?;
    if others > 0 {
        value["other_routes"] = json!(others);
        value["other_routes_note"] = json!(format!(
            "This bot also has {others} chat-specific route(s) to other agents from an earlier \
            setup. Messages from those chats go there, not to {}; if one is the user's own chat \
            with the bot, list them with nyxid__list_channel_routes and, with the user's OK, \
            remove it with nyxid__delete_channel_route.",
            identifier(&agent.name)
        ));
    }
    Ok((value, false))
}

/// Active routes on the channel's bot other than its own: they take the
/// chats they name before NyxBot's default route sees them.
async fn other_routes(state: &AppState, row: &NyxbotChannel) -> AppResult<u64> {
    let mut filter = doc! {"user_id": bot_owner(row), "channel_bot_id": &row.channel_bot_id,
    "is_active": true};
    if let Some(route_id) = row.route_id.as_deref() {
        filter.insert("_id", doc! {"$ne": route_id});
    }
    Ok(state
        .db
        .collection::<bson::Document>(crate::models::channel_conversation::COLLECTION_NAME)
        .count_documents(filter)
        .await?)
}

/// A pending setup link stays usable this long.
const SETUP_WATCH_TTL_MINUTES: i64 = 120;
/// A connect link can finish this long after it expires (OAuth finalization).
const CONNECT_WATCH_GRACE_MINUTES: i64 = 30;

/// The onboarding page for a platform the owner can register now. Telegram
/// prefers creation inside Telegram (`telegram-new`) when an administrator has
/// configured it, else the bot-token form.
async fn setup_platform(state: &AppState, requested: &str) -> AppResult<(String, String)> {
    let entries = crate::services::channel_platform_catalog_service::list(
        &state.db,
        &state.token_exchange_cache,
    )
    .await?;
    let available =
        |entry: &crate::services::channel_platform_catalog_service::PlatformCatalogEntry| {
            entry.registration.enabled
                && entry
                    .platform_credentials
                    .as_ref()
                    .is_none_or(|(_, configured)| *configured)
        };
    let requested = requested.trim().to_ascii_lowercase();
    let candidates: Vec<&str> = match requested.as_str() {
        "telegram" => vec!["telegram-new", "telegram"],
        other => vec![other],
    };
    for candidate in candidates {
        if let Some(entry) = entries
            .iter()
            .find(|entry| entry.platform == candidate && available(entry))
        {
            return Ok((entry.platform.clone(), entry.display_name.clone()));
        }
    }
    let names: Vec<&str> = entries
        .iter()
        .filter(|entry| available(entry))
        .map(|entry| canonical_platform(&entry.platform))
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    Err(AppError::ValidationError(format!(
        "That channel is not available. Available: {}",
        names.join(", ")
    )))
}

/// Start creating a channel bot: the owner opens NyxID's onboarding page (bot
/// secrets never pass through chat), and the new bot is linked to `agent`
/// automatically once it exists.
pub(crate) async fn setup_link_tool(
    state: &AppState,
    owner: &str,
    source_conversation_id: &str,
    platform: &str,
    label: Option<&str>,
    agent: &crate::models::assistant_agent::AssistantAgent,
) -> AppResult<(Value, bool)> {
    if agent.destroyed_at.is_some() {
        return Err(AppError::Conflict("That agent was destroyed".into()));
    }
    let (platform, display_name) = setup_platform(state, platform).await?;
    let label = label
        .map(|label| excerpt(label.trim(), 60))
        .filter(|label| !label.is_empty());
    let mut url = format!(
        "{}/channel-bots/connect/{}",
        state.config.frontend_url.trim_end_matches('/'),
        urlencode(&platform)
    );
    if let Some(label) = &label {
        url.push_str(&format!("?label={}", urlencode(label)));
    }
    let now = Utc::now();
    let family = canonical_platform(&platform).to_owned();
    // One pending link per owner and platform: a new link replaces the last.
    state
        .db
        .collection::<NyxbotWatch>(WATCHES)
        .update_one(
            doc! {"user_id": owner, "kind": "channel_bot", "platform": &family,
            "status": "pending"},
            doc! {
                "$set": {
                    "agent_id": &agent.id,
                    "conversation_id": source_conversation_id,
                    "created_at": bson::DateTime::from_chrono(now),
                    "expires_at": bson::DateTime::from_chrono(
                        now + ChronoDuration::minutes(SETUP_WATCH_TTL_MINUTES)
                    ),
                },
                "$setOnInsert": {"_id": Uuid::new_v4().to_string()},
            },
        )
        .upsert(true)
        .await?;
    let steps = if platform == "telegram-new" {
        "Open the link, check the bot name, and press Continue in Telegram. Telegram creates \
        the bot; no token to copy."
    } else if platform == "telegram" {
        "Create a bot with @BotFather in Telegram, then open the link and paste its token there \
        (never in this chat)."
    } else {
        "Open the link and follow the steps on that page. Enter any keys or secrets there, \
        never in this chat."
    };
    Ok((
        json!({
            "url": url,
            "platform": display_name,
            "steps": steps,
            "links_to": agent.name,
            "expires_in_minutes": SETUP_WATCH_TTL_MINUTES,
            "note": "Give the user this link and end your turn. Do not ask them to reply when \
                done: once the bot exists NyxID links it and wakes you with the \
                owner-verification step to pass on.",
        }),
        false,
    ))
}

/// Watch a hosted connect link a chat minted, so the chat resumes by itself
/// when the user finishes (or declines) instead of waiting for "connected".
pub(crate) async fn watch_connect_link(
    db: &mongodb::Database,
    owner: &str,
    conversation_id: &str,
    connect_link_id: &str,
) -> AppResult<()> {
    let link = db
        .collection::<crate::models::connect_link::ConnectLink>(
            crate::models::connect_link::COLLECTION_NAME,
        )
        .find_one(doc! {"_id": connect_link_id, "user_id": owner})
        .await?
        .ok_or_else(|| AppError::NotFound("Connect link not found".into()))?;
    let now = Utc::now();
    db.collection::<NyxbotWatch>(WATCHES)
        .insert_one(NyxbotWatch {
            id: Uuid::new_v4().to_string(),
            user_id: owner.into(),
            kind: "connect_link".into(),
            conversation_id: conversation_id.into(),
            status: "pending".into(),
            platform: None,
            agent_id: None,
            channel_bot_id: None,
            connect_link_id: Some(connect_link_id.into()),
            last_error: None,
            checked_at: None,
            created_at: now,
            expires_at: link.expires_at.max(now)
                + ChronoDuration::minutes(CONNECT_WATCH_GRACE_MINUTES),
        })
        .await?;
    Ok(())
}

/// Claim a pending watch once across replicas.
async fn claim(state: &AppState, watch: &NyxbotWatch, extra: bson::Document) -> AppResult<bool> {
    let mut set = doc! {"status": "claimed"};
    set.extend(extra);
    Ok(state
        .db
        .collection::<NyxbotWatch>(WATCHES)
        .update_one(
            doc! {"_id": &watch.id, "status": "pending"},
            doc! {"$set": set},
        )
        .await?
        .modified_count
        == 1)
}

async fn settle(state: &AppState, watch: &NyxbotWatch, error: Option<&'static str>) {
    let _ = state
        .db
        .collection::<NyxbotWatch>(WATCHES)
        .update_one(
            doc! {"_id": &watch.id},
            doc! {"$set": {"status": if error.is_some() { "failed" } else { "done" },
            "last_error": error}},
        )
        .await;
}

async fn wake_with(state: &AppState, watch: &NyxbotWatch, kind: &str, text: String) {
    super::assistant_team::notify(
        state,
        &watch.user_id,
        &watch.conversation_id,
        vec![crate::services::assistant_team_service::event(
            kind, text, None,
        )],
    )
    .await;
}

/// Resolve pending watches: link bots created from setup links, and report
/// finished connect links, waking the thread that is waiting.
pub(crate) async fn process_watches(state: &AppState) -> AppResult<()> {
    let watches: Vec<NyxbotWatch> = state
        .db
        .collection::<NyxbotWatch>(WATCHES)
        .find(doc! {"status": "pending", "expires_at": {"$gt": bson::DateTime::now()}})
        .sort(doc! {"checked_at": 1, "created_at": 1})
        .limit(100)
        .await?
        .try_collect()
        .await?;
    let ids: Vec<&str> = watches.iter().map(|watch| watch.id.as_str()).collect();
    state
        .db
        .collection::<NyxbotWatch>(WATCHES)
        .update_many(
            doc! {"_id": {"$in": ids}},
            doc! {"$set": {"checked_at": bson::DateTime::now()}},
        )
        .await?;
    for watch in watches {
        resolve(state, &watch).await;
    }
    Ok(())
}

async fn resolve(state: &AppState, watch: &NyxbotWatch) {
    let result = match watch.kind.as_str() {
        "channel_bot" => channel_bot_watch(state, watch).await,
        "connect_link" => connect_link_watch(state, watch).await,
        _ => Ok(()),
    };
    if let Err(error) = result {
        tracing::debug!(%error, "NyxBot watch deferred");
    }
}

/// React to live changes at once instead of on the next sweep: a watched
/// connect link that finished, or a new (or reactivated) bot for an owner
/// with a pending setup link. The 15-second sweep remains the backstop.
pub fn spawn_live_dispatch(state: AppState) {
    use crate::services::assistant_live::LiveEvent;
    let mut events = state.assistant_live.subscribe();
    tokio::spawn(async move {
        loop {
            let event = match events.recv().await {
                Ok(event) => event,
                Err(broadcast::error::RecvError::Lagged(_)) => LiveEvent::Resync,
                Err(broadcast::error::RecvError::Closed) => break,
            };
            let filter = match event {
                LiveEvent::ConnectLink {
                    id,
                    user_id,
                    status,
                } if status != "pending" => {
                    doc! {"user_id": user_id, "kind": "connect_link", "connect_link_id": id}
                }
                LiveEvent::ChannelBot {
                    user_id,
                    active: true,
                    ..
                } => doc! {"user_id": user_id, "kind": "channel_bot"},
                LiveEvent::Resync => {
                    let state = state.clone();
                    tokio::spawn(async move {
                        if let Err(error) = process_watches(&state).await {
                            tracing::debug!(%error, "NyxBot watch resync deferred");
                        }
                    });
                    continue;
                }
                _ => continue,
            };
            // Linking may call the gateway: never hold up the next event.
            let state = state.clone();
            tokio::spawn(async move {
                if let Err(error) = resolve_matching(&state, filter).await {
                    tracing::debug!(%error, "NyxBot live watch deferred");
                }
            });
        }
    });
}

/// Resolve the pending watches matching `filter` now. Claims are atomic,
/// so replicas reacting to the same change never act twice.
async fn resolve_matching(state: &AppState, mut filter: bson::Document) -> AppResult<()> {
    filter.insert("status", "pending");
    filter.insert("expires_at", doc! {"$gt": bson::DateTime::now()});
    let watches: Vec<NyxbotWatch> = state
        .db
        .collection::<NyxbotWatch>(WATCHES)
        .find(filter)
        .sort(doc! {"created_at": 1})
        .limit(20)
        .await?
        .try_collect()
        .await?;
    for watch in watches {
        resolve(state, &watch).await;
    }
    Ok(())
}

async fn channel_bot_watch(state: &AppState, watch: &NyxbotWatch) -> AppResult<()> {
    let (Some(platform), Some(agent_id)) = (watch.platform.as_deref(), watch.agent_id.as_deref())
    else {
        return Ok(());
    };
    let owner = watch.user_id.as_str();
    let mut candidate = None;
    for bot in channel_bot_service::list_bots(&state.db, owner).await? {
        // Newest first; only bots created after the link was issued.
        if bot.created_at < watch.created_at - ChronoDuration::seconds(5) {
            break;
        }
        if canonical_platform(&bot.platform) == platform
            && active_for_bot(state, owner, &bot.id).await?.is_none()
        {
            candidate = Some(bot);
            break;
        }
    }
    let Some(bot) = candidate else {
        return Ok(());
    };
    if !claim(state, watch, doc! {"channel_bot_id": &bot.id}).await? {
        return Ok(());
    }
    let outcome =
        match crate::services::assistant_team_service::agent(&state.db, owner, agent_id).await {
            Ok(agent) => connect_tool(state, owner, &watch.conversation_id, &bot.id, &agent)
                .await
                .map(|(value, _)| (agent, value)),
            Err(error) => Err(error),
        };
    let text = match &outcome {
        Ok((agent, value)) => format!(
            "The {} channel bot {} the user just created is now linked to {}. Give the user \
            its owner-verification step: {}",
            identifier(&bot.platform),
            identifier(&bot.label),
            identifier(&agent.name),
            value["link"]
        ),
        Err(error) => format!(
            "The {} channel bot {} was created but could not be linked ({}). Try \
            nyxid__connect_channel_bot with bot_id {}.",
            identifier(&bot.platform),
            identifier(&bot.label),
            error_code(error),
            bot.id
        ),
    };
    settle(state, watch, outcome.as_ref().err().map(error_code)).await;
    wake_with(state, watch, "channel_bot_linked", text).await;
    Ok(())
}

async fn connect_link_watch(state: &AppState, watch: &NyxbotWatch) -> AppResult<()> {
    use crate::models::connect_link::{COLLECTION_NAME as LINKS, ConnectLink, ConnectLinkStatus};
    let Some(link_id) = watch.connect_link_id.as_deref() else {
        return Ok(());
    };
    let Some(link) = state
        .db
        .collection::<ConnectLink>(LINKS)
        .find_one(doc! {"_id": link_id, "user_id": &watch.user_id})
        .await?
    else {
        settle(state, watch, Some("not_found")).await;
        return Ok(());
    };
    let text = match link.status {
        ConnectLinkStatus::Pending => return Ok(()),
        ConnectLinkStatus::Completed => format!(
            "The user finished connecting {} (connect_link_id {link_id}). Continue the task \
            that needed it now; do not ask them to confirm.",
            identifier(&link.service_slug)
        ),
        ConnectLinkStatus::Cancelled => format!(
            "The user declined connecting {} (connect_link_id {link_id}).",
            identifier(&link.service_slug)
        ),
        // An unused link that ran out needs no reply from anyone.
        ConnectLinkStatus::Expired => {
            if claim(state, watch, doc! {}).await? {
                settle(state, watch, Some("expired")).await;
            }
            return Ok(());
        }
    };
    if !claim(state, watch, doc! {}).await? {
        return Ok(());
    }
    settle(state, watch, None).await;
    wake_with(state, watch, "connection_finished", text).await;
    Ok(())
}

/// Stable code for an error, never its prose.
fn error_code(error: &AppError) -> &'static str {
    match error {
        AppError::NotFound(_) => "not_found",
        AppError::Conflict(_) => "conflict",
        AppError::ValidationError(_) => "invalid",
        _ => "unavailable",
    }
}

/// Point a connected bot at another agent. Existing chats keep their history
/// with the previous agent; new messages start threads with the new one.
pub async fn link(
    state: &AppState,
    owner: &str,
    channel_id: &str,
    agent: &crate::models::assistant_agent::AssistantAgent,
) -> AppResult<Value> {
    if agent.destroyed_at.is_some() {
        return Err(AppError::Conflict("That agent was destroyed".into()));
    }
    let row = load_channel(state, owner, channel_id).await?;
    if row.agent_id.as_deref() == Some(agent.id.as_str())
        || (row.agent_id.is_none() && agent.is_nyxbot())
    {
        return Ok(json!({"channel_agent_id": row.id, "agent": agent.name, "changed": false}));
    }
    state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .update_one(
            doc! {"_id": &row.id, "user_id": owner},
            doc! {"$set": {"agent_id": &agent.id, "updated_at": bson::DateTime::now()}},
        )
        .await?;
    state
        .db
        .collection::<NyxbotThread>(THREADS)
        .update_many(
            // Chats given their own agent keep it.
            doc! {"channel_id": &row.id, "user_id": owner, "agent_id": bson::Bson::Null},
            doc! {"$set": {"conversation_id": bson::Bson::Null}},
        )
        .await?;
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

pub(crate) async fn list_tool(state: &AppState, owner: &str) -> AppResult<Value> {
    let rows = list(state, owner).await?;
    let mut channel_agents = Vec::with_capacity(rows.len());
    for row in &rows {
        let mut value = serde_json::to_value(ChannelAgentResponse::from(row))
            .map_err(|error| AppError::Internal(error.to_string()))?;
        // While the owner has not verified, say what NyxID saw from the bot
        // (for an org bot, only while they still administer the org).
        if org_access_holds(state, row).await?
            && let Some(hint) = status::verification_hint(state, row).await?
        {
            value["inbound_hint"] = json!(hint);
        }
        channel_agents.push(value);
    }
    Ok(json!({"channel_agents": channel_agents}))
}

async fn audit(state: &AppState, owner: &str, event: &str, data: Value) {
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

// ---------------------------------------------------------------------------
// Human UI endpoints
// ---------------------------------------------------------------------------

pub async fn list_channels(
    State(state): State<AppState>,
    auth: crate::mw::auth::AuthUser,
) -> AppResult<Json<Value>> {
    let owner = auth.user_id.to_string();
    engine::require_enabled(&state.db, &owner).await?;
    Ok(Json(list_tool(&state, &owner).await?))
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectRequest {
    bot_id: String,
    /// The agent to reach; the owner's NyxBot by default.
    agent_id: Option<String>,
}

async fn requested_agent(
    state: &AppState,
    owner: &str,
    agent_id: Option<&str>,
) -> AppResult<crate::models::assistant_agent::AssistantAgent> {
    match agent_id {
        Some(id) => crate::services::assistant_team_service::agent(&state.db, owner, id).await,
        None => crate::services::assistant_team_service::ensure_nyxbot(&state.db, owner).await,
    }
}

pub async fn connect_channel(
    State(state): State<AppState>,
    auth: crate::mw::auth::AuthUser,
    Json(body): Json<ConnectRequest>,
) -> AppResult<Json<Value>> {
    let owner = auth.user_id.to_string();
    engine::require_enabled(&state.db, &owner).await?;
    let agent = requested_agent(&state, &owner, body.agent_id.as_deref()).await?;
    let (row, link) = connect(&state, &owner, None, &body.bot_id, &agent).await?;
    Ok(Json(json!({
        "channel_agent": ChannelAgentResponse::from(&row),
        "link": link_json(&row, &link),
    })))
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LinkRequest {
    /// Move the bot to this agent.
    agent_id: Option<String>,
    /// `owner` or `everyone`: who may talk to the agent in private chats.
    private_chats: Option<String>,
}

pub async fn link_channel(
    State(state): State<AppState>,
    auth: crate::mw::auth::AuthUser,
    Path(id): Path<String>,
    Json(body): Json<LinkRequest>,
) -> AppResult<Json<Value>> {
    let owner = auth.user_id.to_string();
    engine::require_enabled(&state.db, &owner).await?;
    if body.agent_id.is_none() && body.private_chats.is_none() {
        return Err(AppError::ValidationError(
            "Nothing to change: pass agent_id or private_chats".into(),
        ));
    }
    let mut result = json!({"channel_agent_id": &id});
    if let Some(access) = body.private_chats.as_deref() {
        result = chats::set_private_chats(&state, &owner, &id, access).await?;
    }
    if let Some(agent_id) = body.agent_id.as_deref() {
        let agent = requested_agent(&state, &owner, Some(agent_id)).await?;
        result = link(&state, &owner, &id, &agent).await?;
    }
    Ok(Json(result))
}

pub async fn disconnect_channel(
    State(state): State<AppState>,
    auth: crate::mw::auth::AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    let owner = auth.user_id.to_string();
    engine::require_enabled(&state.db, &owner).await?;
    Ok(Json(disconnect(&state, &owner, &id).await?))
}

// ---------------------------------------------------------------------------
// Shared channel turn
// ---------------------------------------------------------------------------

/// What an inbound channel message resolved to.
enum Inbound {
    /// The owner: a turn with the linked agent is running.
    Turn(broadcast::Receiver<Value>),
    /// A reply without a turn (link confirmation, refusal, busy notice).
    Reply(String),
    /// Nothing to say (e.g. a stranger in a group chat).
    Silent,
    /// Retry later: this chat's turn is still running or the pool is full.
    Busy,
}

struct Sender<'a> {
    id: &'a str,
    display_name: Option<&'a str>,
}

const PRIVATE_REFUSAL: &str = "This bot answers only its owner. If this is your bot, ask NyxBot \
    in NyxID for a link and open it here.";

/// Decide what an inbound chat message leads to: the owner verifying their
/// account, a turn (as the owner or a guest), a short reply, or nothing.
/// `addressed`: whether the message mentions or replies to the bot, `None`
/// when the platform cannot tell (then only the owner is answered).
async fn inbound_message(
    state: &AppState,
    row: &NyxbotChannel,
    chat: &NyxbotThread,
    sender: &Sender<'_>,
    text: &str,
    addressed: Option<bool>,
) -> AppResult<Inbound> {
    if let Some(linked) = link_owner(state, row, sender, text).await? {
        return Ok(linked);
    }
    let chat = &chats::note_owner_presence(state, row, chat, sender.id).await?;
    let guest = match chats::admission(row, chat, sender.id, addressed) {
        chats::Admission::Owner => false,
        chats::Admission::Guest => true,
        chats::Admission::Refuse => return Ok(Inbound::Reply(PRIVATE_REFUSAL.into())),
        chats::Admission::Silent => return Ok(Inbound::Silent),
        chats::Admission::Waiting => {
            return Ok(match chats::waiting_hint(state, chat).await? {
                Some(hint) => Inbound::Reply(hint),
                None => Inbound::Silent,
            });
        }
    };
    let addressed = chat.kind.as_deref() == Some("private") || addressed == Some(true);
    start_chat_turn(state, row, chat, sender, text, guest, addressed).await
}

/// Link a sender presenting the owner's one-time code.
async fn link_owner(
    state: &AppState,
    row: &NyxbotChannel,
    sender: &Sender<'_>,
    text: &str,
) -> AppResult<Option<Inbound>> {
    if let (Some(hash), Some(expires_at)) =
        (row.link_code_hash.as_deref(), row.link_code_expires_at)
        && expires_at > Utc::now()
        && text.split_whitespace().any(|word| {
            sha256_hex(word.as_bytes())
                .as_bytes()
                .ct_eq(hash.as_bytes())
                .into()
        })
    {
        state
            .db
            .collection::<NyxbotChannel>(CHANNELS)
            .update_one(
                doc! {"_id": &row.id, "link_code_hash": hash},
                doc! {"$addToSet": {"owner_sender_ids": sender.id},
                "$unset": {"link_code_hash": "", "link_code_expires_at": ""},
                "$set": {"updated_at": bson::DateTime::now()}},
            )
            .await?;
        audit(
            state,
            &row.user_id,
            "nyxbot_channel_owner_linked",
            json!({
                "channel_agent_id": &row.id, "platform": &row.platform,
            }),
        )
        .await;
        let agent_name = match row.agent_id.as_deref() {
            Some(agent_id) => {
                crate::services::assistant_team_service::agent(&state.db, &row.user_id, agent_id)
                    .await
                    .map(|agent| agent.name)
                    .unwrap_or_else(|_| "NyxBot".into())
            }
            None => "NyxBot".into(),
        };
        // The chat that set the bot up hears about it without the user
        // coming back to say so.
        if let Some(source) = row.source_conversation_id.clone() {
            let state = state.clone();
            let owner = row.user_id.clone();
            let text = format!(
                "The user verified their {} account on channel bot {}; it now reaches {} \
                there. No confirmation is needed.",
                identifier(&row.platform),
                identifier(&row.bot_label),
                identifier(&agent_name)
            );
            tokio::spawn(async move {
                super::assistant_team::notify(
                    &state,
                    &owner,
                    &source,
                    vec![crate::services::assistant_team_service::event(
                        "channel_owner_verified",
                        text,
                        None,
                    )],
                )
                .await;
            });
        }
        return Ok(Some(Inbound::Reply(format!(
            "Linked. I'm {agent_name}, your NyxID agent: I can use your NyxID services and \
            account here. Send me anything to get started."
        ))));
    }
    Ok(None)
}

async fn thread_conversation(
    state: &AppState,
    row: &NyxbotChannel,
    partition: &str,
) -> AppResult<(NyxbotThread, String)> {
    let now = bson::DateTime::now();
    let threads = state.db.collection::<NyxbotThread>(THREADS);
    threads
        .update_one(
            doc! {"channel_id": &row.id, "partition": partition},
            doc! {"$setOnInsert": {"_id": Uuid::new_v4().to_string(), "user_id": &row.user_id,
            "created_at": now}, "$set": {"updated_at": now}},
        )
        .upsert(true)
        .await?;
    let reserved = format!("nyxa-{}", Uuid::new_v4().simple());
    let thread = threads
        .find_one_and_update(
            doc! {"channel_id": &row.id, "partition": partition,
            "conversation_id": bson::Bson::Null},
            doc! {"$set": {"conversation_id": &reserved}},
        )
        .return_document(mongodb::options::ReturnDocument::After)
        .await?;
    let thread = match thread {
        Some(thread) => thread,
        None => threads
            .find_one(doc! {"channel_id": &row.id, "partition": partition})
            .await?
            .ok_or_else(|| AppError::Internal("Channel thread unavailable".into()))?,
    };
    let conversation_id = thread
        .conversation_id
        .clone()
        .ok_or_else(|| AppError::Internal("Channel thread unavailable".into()))?;
    Ok((thread, conversation_id))
}

/// Apply the owner's yes/no to a pending card of this chat thread, audited
/// like a card decision. Returns whether it confirmed.
async fn reply_decision(
    state: &AppState,
    row: &NyxbotChannel,
    conversation_id: &str,
    text: &str,
) -> AppResult<Option<bool>> {
    use crate::services::assistant_acknowledgement_service as acks;
    if acks::parse_reply(text).is_none() {
        return Ok(None);
    }
    // The owner's previous message: only cards raised after it are answered
    // by a plain yes/no.
    let since = engine::messages(&state.db, &row.user_id, conversation_id, 100, None)
        .await?
        .into_iter()
        .rev()
        .find(|message| message.role == "user")
        .map(|message| message.created_at);
    let Some(decided) = acks::decide_reply(
        &state.db,
        &row.user_id,
        &[conversation_id.to_owned()],
        text,
        since,
    )
    .await?
    else {
        return Ok(None);
    };
    acks::audit_decision(
        &state.db,
        &crate::services::audit_service::AuditActor {
            user_id: row.user_id.clone(),
            ip_address: None,
            user_agent: None,
            api_key_id: None,
            api_key_name: None,
        },
        &decided,
    )
    .await;
    Ok(Some(decided.status == "allowed"))
}

/// The platform's name as people write it.
fn platform_name(platform: &str) -> String {
    match canonical_platform(platform) {
        "whatsapp" => "WhatsApp".into(),
        "x" => "X".into(),
        other => {
            let mut chars = other.chars();
            chars
                .next()
                .map(|first| first.to_uppercase().chain(chars).collect())
                .unwrap_or_default()
        }
    }
}

/// The agent's own thread (its home in NyxID) that the owner's private chats
/// continue: one context across the app and every chat app. Returns the
/// thread and whether it exists yet (a new one becomes the agent's home).
async fn owner_thread(
    state: &AppState,
    row: &NyxbotChannel,
    agent_id: &str,
) -> AppResult<(String, bool)> {
    let agent =
        crate::services::assistant_team_service::agent(&state.db, &row.user_id, agent_id).await?;
    if let Some(home) = agent.home_conversation_id.as_deref() {
        match engine::get(&state.db, &row.user_id, home).await {
            Ok(conversation)
                if conversation.group_id.is_none() && conversation.channel.is_none() =>
            {
                return Ok((home.to_owned(), true));
            }
            // A deleted home, or one that is not the agent's own thread (a
            // group's or someone else's chat): the next thread takes its place.
            Ok(_) | Err(AppError::NotFound(_)) => {
                state
                    .db
                    .collection::<bson::Document>(crate::models::assistant_agent::COLLECTION_NAME)
                    .update_one(
                        doc! {"_id": &agent.id, "user_id": &row.user_id,
                        "home_conversation_id": home},
                        doc! {"$set": {"home_conversation_id": bson::Bson::Null}},
                    )
                    .await?;
            }
            Err(error) => return Err(error),
        }
    }
    Ok((format!("nyxa-{}", Uuid::new_v4().simple()), false))
}

/// Run the chat's agent on a message from the owner or, as a guest, from
/// someone else the chat lets talk to it. The owner's private chats continue
/// the agent's own thread; groups and other people's chats have their own.
async fn start_chat_turn(
    state: &AppState,
    row: &NyxbotChannel,
    chat: &NyxbotThread,
    sender: &Sender<'_>,
    text: &str,
    guest: bool,
    addressed: bool,
) -> AppResult<Inbound> {
    let private = chat.kind.as_deref() == Some("private");
    // An organization's bot never carries the owner's personal thread.
    let shared = private && !guest && row.bot_owner_id.is_none();
    let agent_id = match chat.agent_id.clone().or_else(|| row.agent_id.clone()) {
        Some(agent_id) => agent_id,
        None => {
            crate::services::assistant_team_service::ensure_nyxbot(&state.db, &row.user_id)
                .await?
                .id
        }
    };
    let origin = ChannelOrigin {
        nyxbot_channel_id: row.id.clone(),
        partition: chat.partition.clone(),
        platform: row.platform.clone(),
    };
    let (conversation_id, exists) = if shared {
        let (id, exists) = owner_thread(state, row, &agent_id).await?;
        // The chat now answers into that thread (asynchronous replies and
        // word confirmations find it there).
        state
            .db
            .collection::<NyxbotThread>(THREADS)
            .update_one(
                doc! {"_id": &chat.id},
                doc! {"$set": {"conversation_id": &id}},
            )
            .await?;
        (id, exists)
    } else {
        let (_, id) = thread_conversation(state, row, &chat.partition).await?;
        let exists = engine::get(&state.db, &row.user_id, &id).await.is_ok();
        (id, exists)
    };
    let question_key = engine::question_key(text);
    // A chat app cannot show NyxID's confirmation cards: the verified owner
    // answers one in words (see `decide_reply` for which card it decides).
    // Nobody else can.
    let mut answered = None;
    if exists && !guest {
        match reply_decision(state, row, &conversation_id, text).await {
            Ok(decided) => answered = decided,
            Err(error) => tracing::debug!(%error, "Chat confirmation not applied"),
        }
    }
    let name = sender
        .display_name
        .map(|name| excerpt(name, 60).replace('"', "'"));
    let who = name
        .as_deref()
        .map(|name| format!("\"{name}\""))
        .unwrap_or_else(|| (if guest { "someone" } else { "the owner" }).into());
    let place = chats::describe(chat);
    let mut note = if guest {
        format!(
            "This message came through the owner's {} channel bot {} from {who} in {place}; \
            they are not the owner.",
            identifier(&row.platform),
            identifier(&row.bot_label),
        )
    } else {
        format!(
            "This message came through the owner's {} channel bot {} from {who} ({place}); \
            NyxID verified this sender as the owner.",
            identifier(&row.platform),
            identifier(&row.bot_label),
        )
    };
    if shared {
        note.push_str(
            " This is your own thread with the owner, which they also reach from the NyxID app \
            and their other chat apps; your reply goes back to this chat.",
        );
    } else if !private {
        note.push_str(
            " Everyone in the chat sees your reply. Messages there start with their sender's \
            name.",
        );
    }
    if let Some(allow) = answered {
        note.push_str(if allow {
            " With this message the owner confirmed the pending action; retry it with its \
            acknowledgement_id."
        } else {
            " With this message the owner declined the pending action; do not retry it."
        });
    }
    let message = if private {
        excerpt(text, engine::MAX_MESSAGE_CHARS - 16)
    } else {
        excerpt(
            &chats::attributed(sender.display_name, text, guest),
            engine::MAX_MESSAGE_CHARS - 16,
        )
    };
    let title = if shared {
        None
    } else if private {
        Some(chats::private_title(row, sender.id, sender.display_name))
    } else {
        // A new group thread is named after the group when the platform can
        // say (bounded; best effort).
        let looked_up = match chat.title.clone() {
            Some(title) => Some(title),
            None if !exists => chats::look_up_title(state, row, chat).await,
            None => None,
        };
        Some(looked_up.unwrap_or_else(|| format!("{} group", platform_name(&row.platform))))
    };
    let start = TurnStart {
        conversation_id: exists.then(|| conversation_id.clone()),
        new_id: (!exists).then(|| conversation_id.clone()),
        text: message,
        model: Some(
            crate::services::assistant_profile_routing::model_for(
                &state.db,
                crate::services::assistant_profile_routing::RouteRole::Channel,
                engine::DEFAULT_MODEL,
            )
            .await,
        ),
        origin: TurnOrigin::Channel,
        // The owner's own thread is not a channel thread: it only remembers
        // which chat to answer asynchronously.
        channel: (!shared).then(|| origin.clone()),
        title,
        note: Some(note),
        agent_id: Some(agent_id),
        report_to: None,
        group_id: None,
        guest,
        question_key: question_key.clone(),
        question: Some(excerpt(text, engine::QUESTION_EXCERPT_CHARS)),
        reply_channel: shared.then(|| origin.clone()),
    };
    match start_server_turn(
        state,
        &row.user_id,
        start,
        Pool::Channel {
            owner: &row.user_id,
        },
    )
    .await
    {
        Ok(Started::Turn { receiver, .. }) => Ok(Inbound::Turn(receiver)),
        Ok(Started::Busy | Started::PoolFull) => {
            // The same question is never worked on twice: a repeat waits for
            // the answer in progress (sent to this chat too) or already queued.
            if exists
                && let Some(key) = question_key.as_deref()
                && let Some(reply) =
                    repeated_question(state, row, &conversation_id, key, &origin).await?
            {
                return Ok(Inbound::Reply(reply));
            }
            // Someone other than the owner is asked to try again (only when
            // they spoke to the bot): their messages never queue up as the
            // owner's work or use the owner's wake-ups.
            if guest {
                return Ok(if addressed {
                    Inbound::Reply(
                        "I'm answering another message right now. Please try again in a moment."
                            .into(),
                    )
                } else {
                    Inbound::Silent
                });
            }
            if !exists {
                return Ok(Inbound::Busy);
            }
            // The chat is busy (or the owner's channel pool is full): queue the
            // message for the agent's next turn instead of bouncing it. Its
            // reply is an asynchronous update delivered back to this chat.
            let note = format!(
                "{who} sent another {} message in {place} while you were working. Answer it \
                next; your reply is delivered to the chat: \"{}\"",
                identifier(&row.platform),
                excerpt(text, 3000).replace('"', "'")
            );
            let mut event = crate::services::assistant_team_service::event("message", note, None);
            event.question_key = question_key;
            event.reply_to = vec![origin.clone()];
            let queued =
                engine::push_events(&state.db, &row.user_id, &conversation_id, vec![event]).await?;
            if queued.is_none() {
                return Ok(Inbound::Busy);
            }
            if shared {
                engine::set_reply_channel(&state.db, &row.user_id, &conversation_id, &origin)
                    .await?;
            }
            super::assistant_team::wake(state, &row.user_id, &conversation_id).await;
            Ok(Inbound::Reply(
                "Got it. I'll answer this right after the message I'm working on.".into(),
            ))
        }
        // A concurrent first message created the chat: it now exists.
        Err(AppError::Conflict(_)) | Err(AppError::DatabaseError(_)) if !exists => {
            Ok(Inbound::Busy)
        }
        Err(error) => Err(error),
    }
}

/// When `key` is the question the thread is answering right now, or one
/// already queued there, the reply to a repeat of it (and the chat is added
/// to the running answer's recipients).
async fn repeated_question(
    state: &AppState,
    row: &NyxbotChannel,
    conversation_id: &str,
    key: &str,
    origin: &ChannelOrigin,
) -> AppResult<Option<String>> {
    let current = engine::get(&state.db, &row.user_id, conversation_id).await?;
    if let Some(turn) = current
        .active_turn
        .as_ref()
        .filter(|turn| turn.question_key.as_deref() == Some(key))
    {
        if turn.asked_from.as_ref() == Some(origin) {
            return Ok(Some(
                "I'm still working on that question and will answer it shortly.".into(),
            ));
        }
        let waiting = engine::also_deliver(
            &state.db,
            &row.user_id,
            conversation_id,
            &turn.turn_id,
            origin,
        )
        .await?;
        return Ok(Some(if waiting {
            "I'm already working on that question; I'll send the answer here too.".into()
        } else {
            "I'm already working on that question; you'll find the answer in NyxID.".into()
        }));
    }
    if current
        .pending_events
        .iter()
        .any(|event| event.question_key.as_deref() == Some(key))
    {
        if !engine::also_reply_to_queued(&state.db, &row.user_id, conversation_id, key, origin)
            .await?
        {
            // A turn has just taken it: wait for that turn's answer instead,
            // unless the turn already answers this chat.
            let current = engine::get(&state.db, &row.user_id, conversation_id).await?;
            let waiting = match current.active_turn.as_ref().filter(|turn| {
                turn.events
                    .iter()
                    .any(|event| event.question_key.as_deref() == Some(key))
            }) {
                Some(turn) => {
                    let answered_here = match turn.origin {
                        TurnOrigin::Channel => turn.asked_from.as_ref(),
                        TurnOrigin::Event => {
                            current.channel.as_ref().or(current.reply_channel.as_ref())
                        }
                        _ => None,
                    };
                    answered_here == Some(origin)
                        || engine::also_deliver(
                            &state.db,
                            &row.user_id,
                            conversation_id,
                            &turn.turn_id,
                            origin,
                        )
                        .await?
                }
                None => false,
            };
            return Ok(Some(if waiting {
                "I'm already working on that question; I'll send the answer here.".into()
            } else {
                "I've just answered that question; you'll find it in NyxID.".into()
            }));
        }
        return Ok(Some(
            "That question is already queued; I'll answer it next.".into(),
        ));
    }
    Ok(None)
}

/// Wait for the turn and return its final reply, or `None` on failure.
async fn final_reply(mut receiver: broadcast::Receiver<Value>) -> Result<String, String> {
    let mut text = String::new();
    loop {
        match receiver.recv().await {
            Ok(event) => match event["event"].as_str() {
                Some("block.completed") => {
                    text = event["block"]["text"]
                        .as_str()
                        .unwrap_or_default()
                        .to_owned();
                }
                Some("turn.completed") => {
                    return if event["status"] == "completed" {
                        Ok(text)
                    } else {
                        Err(event["error"]["code"]
                            .as_str()
                            .unwrap_or("assistant_unavailable")
                            .to_owned())
                    };
                }
                _ => {}
            },
            Err(broadcast::error::RecvError::Lagged(_)) => continue,
            Err(broadcast::error::RecvError::Closed) => return Err("turn_lost".into()),
        }
    }
}

fn bounded_reply(text: &str) -> String {
    let text = text.trim();
    if text.is_empty() {
        "Done.".into()
    } else {
        excerpt(text, MAX_CHANNEL_REPLY_CHARS)
    }
}

// ---------------------------------------------------------------------------
// Gateway provider contract (called by CMAEG with the channel's agent key)
// ---------------------------------------------------------------------------

fn problem(status: StatusCode, code: &str) -> Response {
    (status, Json(json!({"error": {"code": code}, "code": code}))).into_response()
}

/// Authenticate the gateway: the bearer is the channel's dedicated agent key.
#[allow(clippy::result_large_err)]
async fn provider_channel(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<(NyxbotChannel, Zeroizing<String>), Response> {
    let raw = headers
        .get("authorization")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .map(|value| Zeroizing::new(value.trim().to_owned()))
        .ok_or_else(|| problem(StatusCode::UNAUTHORIZED, "not_authenticated"))?;
    let (owner, key, _): (String, ApiKey, Option<String>) =
        key_service::validate_api_key(&state.db, &raw)
            .await
            .map_err(|_| problem(StatusCode::UNAUTHORIZED, "not_authenticated"))?;
    let row = state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .find_one(doc! {"agent_api_key_id": &key.id, "user_id": &owner,
        "status": {"$in": ["pending", "active"]}})
        .await
        .map_err(|_| problem(StatusCode::SERVICE_UNAVAILABLE, "provider_unavailable"))?
        .ok_or_else(|| problem(StatusCode::FORBIDDEN, "forbidden"))?;
    Ok((row, raw))
}

pub async fn agent_card() -> Json<Value> {
    Json(json!({
        "name": "NyxBot",
        "description": "The owner's NyxID assistant (orchestrator with subagents).",
        "capabilities": ["streaming"],
        "authentication": {"schemes": ["bearer-agent-key"]},
    }))
}

pub async fn put_binding(
    State(state): State<AppState>,
    Path(binding_id): Path<String>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    let (row, raw) = match provider_channel(&state, &headers).await {
        Ok(found) => found,
        Err(response) => return response,
    };
    let presented = body["agent_key"].as_str().unwrap_or_default();
    let key_matches: bool = presented.as_bytes().ct_eq(raw.as_bytes()).into();
    let owner_matches = body["profile"]["metadata"]["owner"]["subject"] == json!(row.user_id)
        && body["profile"]["metadata"]["provider"] == json!(PROVIDER_SLUG);
    let valid_id = binding_id.len() <= 128
        && binding_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
    if !key_matches || !owner_matches || !valid_id {
        return problem(StatusCode::FORBIDDEN, "forbidden");
    }
    if row
        .binding_id
        .as_deref()
        .is_some_and(|bound| bound != binding_id)
    {
        return problem(StatusCode::CONFLICT, "binding_conflict");
    }
    if state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .update_one(
            doc! {"_id": &row.id},
            doc! {"$set": {"binding_id": &binding_id, "updated_at": bson::DateTime::now()}},
        )
        .await
        .is_err()
    {
        return problem(StatusCode::SERVICE_UNAVAILABLE, "provider_unavailable");
    }
    let mut refused = vec![json!({"field": "model",
        "reason": "NyxBot runs the owner's NyxAgent profile"})];
    if body["profile"]["permission"]["service_scope"]
        .as_array()
        .is_some_and(|scope| !scope.is_empty())
    {
        refused.push(json!({"field": "permission.service_scope",
            "reason": "NyxBot acts with the owner's full NyxID access"}));
    }
    for field in ["skills", "mcp_servers"] {
        if body["profile"]["context"][field]
            .as_array()
            .is_some_and(|items| !items.is_empty())
        {
            refused.push(json!({"field": format!("context.{field}"),
                "reason": "NyxBot uses the owner's NyxID services"}));
        }
    }
    if !body["profile"]["environment"].is_null() {
        refused.push(json!({"field": "environment", "reason": "not supported"}));
    }
    Json(json!({"binding_id": binding_id, "level": "l1",
        "honoured": ["metadata", "permission.action_mode"], "refused": refused}))
    .into_response()
}

pub async fn delete_binding(
    State(state): State<AppState>,
    Path(binding_id): Path<String>,
    headers: HeaderMap,
) -> Response {
    let (row, _) = match provider_channel(&state, &headers).await {
        Ok(found) => found,
        Err(response) => return response,
    };
    if row.binding_id.as_deref() != Some(binding_id.as_str()) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let _ = state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .update_one(
            doc! {"_id": &row.id},
            doc! {"$unset": {"binding_id": ""}, "$set": {"updated_at": bson::DateTime::now()}},
        )
        .await;
    StatusCode::NO_CONTENT.into_response()
}

fn valid_partition(value: &str) -> bool {
    value
        .strip_prefix("conv_")
        .is_some_and(|tail| tail.len() == 32 && tail.bytes().all(|b| b.is_ascii_hexdigit()))
}

pub async fn put_conversation(
    State(state): State<AppState>,
    Path((binding_id, conversation_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
    let (row, _) = match provider_channel(&state, &headers).await {
        Ok(found) => found,
        Err(response) => return response,
    };
    if row.binding_id.as_deref() != Some(binding_id.as_str()) || !valid_partition(&conversation_id)
    {
        return problem(StatusCode::NOT_FOUND, "binding_not_found");
    }
    let now = bson::DateTime::now();
    match state
        .db
        .collection::<NyxbotThread>(THREADS)
        .update_one(
            doc! {"channel_id": &row.id, "partition": &conversation_id},
            doc! {"$setOnInsert": {"_id": Uuid::new_v4().to_string(), "user_id": &row.user_id,
            "created_at": now}, "$set": {"updated_at": now}},
        )
        .upsert(true)
        .await
    {
        Ok(_) => Json(json!({})).into_response(),
        Err(_) => problem(StatusCode::SERVICE_UNAVAILABLE, "provider_unavailable"),
    }
}

pub async fn delete_conversation(
    State(state): State<AppState>,
    Path((binding_id, conversation_id)): Path<(String, String)>,
    headers: HeaderMap,
) -> Response {
    let (row, _) = match provider_channel(&state, &headers).await {
        Ok(found) => found,
        Err(response) => return response,
    };
    if row.binding_id.as_deref() != Some(binding_id.as_str()) {
        return StatusCode::NOT_FOUND.into_response();
    }
    // The NyxBot chat itself stays in the owner's history.
    let _ = state
        .db
        .collection::<NyxbotThread>(THREADS)
        .delete_one(doc! {"channel_id": &row.id, "partition": &conversation_id})
        .await;
    StatusCode::NO_CONTENT.into_response()
}

pub async fn get_event_context(
    State(state): State<AppState>,
    Path((binding_id, conversation_id, event_id)): Path<(String, String, String)>,
    headers: HeaderMap,
) -> Response {
    let (row, _) = match provider_channel(&state, &headers).await {
        Ok(found) => found,
        Err(response) => return response,
    };
    if row.binding_id.as_deref() != Some(binding_id.as_str()) {
        return problem(StatusCode::NOT_FOUND, "event_not_found");
    }
    let event = state
        .db
        .collection::<NyxbotEvent>(EVENTS)
        .find_one(doc! {"channel_id": &row.id, "partition": &conversation_id,
        "event_id": &event_id})
        .await;
    let Ok(Some(event)) = event else {
        return problem(StatusCode::NOT_FOUND, "event_not_found");
    };
    let Some(ciphertext) = event.event_context_ciphertext.as_ref() else {
        return problem(StatusCode::NOT_FOUND, "event_not_found");
    };
    match state.encryption_keys.decrypt(ciphertext).await {
        Ok(bytes) => Response::builder()
            .status(StatusCode::OK)
            .header("content-type", "application/json")
            .header("cache-control", "no-store")
            .body(Body::from(bytes))
            .unwrap_or_else(|_| problem(StatusCode::SERVICE_UNAVAILABLE, "provider_unavailable")),
        Err(_) => problem(StatusCode::SERVICE_UNAVAILABLE, "provider_unavailable"),
    }
}

fn sse(frames: impl futures::Stream<Item = Value> + Send + 'static) -> Response {
    use futures::StreamExt;
    let stream = frames.map(|frame| {
        Ok::<_, Infallible>(
            axum::response::sse::Event::default()
                .event(frame["type"].as_str().unwrap_or("message"))
                .data(frame.to_string()),
        )
    });
    let mut response = axum::response::Sse::new(stream)
        .keep_alive(axum::response::sse::KeepAlive::new().interval(Duration::from_secs(15)))
        .into_response();
    response.headers_mut().insert(
        "cache-control",
        "no-cache, no-transform".parse().expect("static header"),
    );
    response
        .headers_mut()
        .insert("x-accel-buffering", "no".parse().expect("static header"));
    response
}

fn message_frames(response_id: &str, text: Option<&str>) -> Vec<Value> {
    let mut frames = Vec::new();
    let mut output = Vec::new();
    if let Some(text) = text {
        let item_id = format!("msg_{}", Uuid::new_v4().simple());
        let item = json!({"id": item_id, "type": "message", "role": "assistant",
            "content": [{"type": "output_text", "text": text}]});
        frames.push(
            json!({"type": "response.output_item.done", "output_index": 0,
            "item": item.clone()}),
        );
        output.push(item);
    }
    frames.push(json!({"type": "response.completed",
        "response": {"id": response_id, "status": "completed", "output": output}}));
    frames
}

/// `POST /api/v1/nyxbot/responses`: admit one gateway event as a NyxBot turn.
pub async fn responses(State(state): State<AppState>, headers: HeaderMap, body: Bytes) -> Response {
    let (row, _) = match provider_channel(&state, &headers).await {
        Ok(found) => found,
        Err(response) => return response,
    };
    let Some(idempotency) = headers
        .get("idempotency-key")
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.is_empty() && value.len() <= 256)
    else {
        return problem(StatusCode::BAD_REQUEST, "invalid_request");
    };
    let Ok(request) = serde_json::from_slice::<Value>(&body) else {
        return problem(StatusCode::BAD_REQUEST, "invalid_request");
    };
    let partition = request["conversation"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    if !valid_partition(&partition) || row.status != "active" {
        return problem(StatusCode::NOT_FOUND, "conversation_not_found");
    }
    if state
        .db
        .collection::<NyxbotThread>(THREADS)
        .find_one(doc! {"channel_id": &row.id, "partition": &partition})
        .await
        .ok()
        .flatten()
        .is_none()
    {
        return problem(StatusCode::NOT_FOUND, "conversation_not_found");
    }
    let text: String = request["input"]
        .as_array()
        .into_iter()
        .flatten()
        .flat_map(|item| item["content"].as_array().into_iter().flatten())
        .filter(|part| part["type"] == "input_text")
        .filter_map(|part| part["text"].as_str())
        .collect::<Vec<_>>()
        .join("\n");
    let context = &request["event_context"];
    let activity = &context["activity"];
    let event_id = activity["event_id"]
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| format!("synthetic_{}", sha256_hex(idempotency)));
    let response_id = format!(
        "resp_{}",
        sha256_hex(format!("{}:{idempotency}", row.id))[..32].to_owned()
    );
    // Synthetic management tests carry no event context: answer without a turn.
    if context.is_null() {
        return sse(futures::stream::iter(
            std::iter::once(json!({"type": "response.created",
                "response": {"id": &response_id, "status": "in_progress"}}))
            .chain(message_frames(&response_id, Some("NyxBot is connected."))),
        ));
    }
    if context["conversation_id"] != json!(partition) || context["activity"].is_null() {
        return problem(StatusCode::BAD_REQUEST, "invalid_request");
    }
    // Durable admission: one turn per idempotency key, context kept verbatim.
    let event_key = sha256_hex(format!("{}\0{idempotency}", row.id));
    let ciphertext = match serde_json::to_vec(context) {
        Ok(bytes) => match state.encryption_keys.encrypt(&bytes).await {
            Ok(ciphertext) => ciphertext,
            Err(_) => return problem(StatusCode::SERVICE_UNAVAILABLE, "provider_unavailable"),
        },
        Err(_) => return problem(StatusCode::BAD_REQUEST, "invalid_request"),
    };
    let now = Utc::now();
    let admitted = NyxbotEvent {
        id: event_key.clone(),
        channel_id: row.id.clone(),
        user_id: row.user_id.clone(),
        partition: partition.clone(),
        event_id: event_id.clone(),
        event_context_ciphertext: Some(ciphertext),
        status: "running".into(),
        conversation_id: None,
        turn_id: None,
        created_at: now,
        expires_at: now + ChronoDuration::hours(EVENT_RETENTION_HOURS),
    };
    let events = state.db.collection::<NyxbotEvent>(EVENTS);
    if let Err(error) = events.insert_one(&admitted).await {
        if !is_duplicate(&error) {
            return problem(StatusCode::SERVICE_UNAVAILABLE, "provider_unavailable");
        }
        // A retry of an admitted event never starts a second turn.
        return replay(&state, &row, &event_key, &response_id).await;
    }
    let sender_id = activity["actor"]["id"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    let display = activity["actor"]["display_name"]
        .as_str()
        .map(str::to_owned);
    let human = activity["actor"]["kind"] == "human";
    let inbound = gateway_inbound(
        &state,
        &row,
        &partition,
        activity,
        context["event_ref"].as_str(),
        &Sender {
            id: &sender_id,
            display_name: display.as_deref(),
        },
        &text,
        human,
    )
    .await;
    let finish = |status: &'static str, conversation: Option<String>| {
        let state = state.clone();
        let event_key = event_key.clone();
        async move {
            // Messages no turn answered (e.g. group chatter) keep no content.
            let _ = state
                .db
                .collection::<NyxbotEvent>(EVENTS)
                .update_one(
                    doc! {"_id": &event_key},
                    doc! {"$set": {"status": status, "conversation_id": conversation},
                    "$unset": {"event_context_ciphertext": ""}},
                )
                .await;
        }
    };
    let created = json!({"type": "response.created",
        "response": {"id": &response_id, "status": "in_progress"}});
    match inbound {
        Ok(Inbound::Reply(text)) => {
            finish("refused", None).await;
            sse(futures::stream::iter(
                std::iter::once(created).chain(message_frames(&response_id, Some(&text))),
            ))
        }
        Ok(Inbound::Silent) => {
            finish("refused", None).await;
            sse(futures::stream::iter(
                std::iter::once(created).chain(message_frames(&response_id, None)),
            ))
        }
        Ok(Inbound::Busy) => {
            // Let the gateway retry this same event after the running turn.
            let _ = events.delete_one(doc! {"_id": &event_key}).await;
            let mut response = problem(StatusCode::CONFLICT, "conversation_in_progress");
            response
                .headers_mut()
                .insert("retry-after", "5".parse().expect("static header"));
            response
        }
        Ok(Inbound::Turn(receiver)) => {
            let state = state.clone();
            let stream = async_stream::stream! {
                yield created;
                let outcome = final_reply(receiver).await;
                let (status, conversation) = match &outcome {
                    Ok(_) => ("completed", None),
                    Err(_) => ("failed", None::<String>),
                };
                let _ = state.db.collection::<NyxbotEvent>(EVENTS).update_one(
                    doc! {"_id": &event_key},
                    doc! {"$set": {"status": status, "conversation_id": conversation}},
                ).await;
                match outcome {
                    Ok(text) => {
                        for frame in message_frames(&response_id, Some(&bounded_reply(&text))) {
                            yield frame;
                        }
                    }
                    Err(code) => {
                        yield json!({"type": "response.failed", "response": {
                            "id": &response_id, "status": "failed",
                            "error": {"code": identifier(&code), "message": "NyxBot could not finish this turn."},
                        }});
                    }
                }
            };
            sse(stream)
        }
        Err(_) => {
            let _ = events.delete_one(doc! {"_id": &event_key}).await;
            problem(StatusCode::SERVICE_UNAVAILABLE, "provider_unavailable")
        }
    }
}

/// A gateway message: record its chat (private chats keep the gateway's
/// conversation; a group is one thread whatever the sender), keep the newest
/// event reference there for asynchronous replies, then decide.
#[allow(clippy::too_many_arguments)]
async fn gateway_inbound(
    state: &AppState,
    row: &NyxbotChannel,
    partition: &str,
    activity: &Value,
    event_ref: Option<&str>,
    sender: &Sender<'_>,
    text: &str,
    human: bool,
) -> AppResult<Inbound> {
    let conversation = &activity["conversation"];
    let kind = chats::chat_kind(conversation["kind"].as_str().unwrap_or("private"));
    let chat_id = conversation["id"].as_str().unwrap_or_default();
    let thread_id = conversation["thread_id"].as_str();
    let chat_partition = if kind == "private" {
        partition.to_owned()
    } else {
        chats::group_partition(chat_id, thread_id)
    };
    let chat = chats::record_chat(
        state,
        row,
        &chat_partition,
        &chats::ChatFacts {
            kind,
            chat_id: chat_id.to_owned(),
            thread_id: thread_id.map(str::to_owned),
            owner: row.owner_sender_ids.iter().any(|id| id == sender.id),
            title: (kind == "private")
                .then(|| chats::private_title(row, sender.id, sender.display_name)),
        },
        None,
    )
    .await?;
    chats::spawn_title_lookup(state, row, &chat);
    // Encrypted; opaque, never logged.
    if let Some(event_ref) = event_ref
        && let Ok(sealed) = state.encryption_keys.encrypt(event_ref.as_bytes()).await
    {
        let _ = state
            .db
            .collection::<NyxbotThread>(THREADS)
            .update_one(
                doc! {"_id": &chat.id},
                doc! {"$set": {"event_ref_ciphertext": bson::Binary {
                    subtype: bson::spec::BinarySubtype::Generic, bytes: sealed },
                "event_ref_expires_at": bson::DateTime::from_chrono(
                    Utc::now() + ChronoDuration::minutes(29))}},
            )
            .await;
    }
    if !human || sender.id.is_empty() {
        return Ok(Inbound::Silent);
    }
    // While the gateway admits only mentions and replies in groups, every
    // group message it passes on is addressed to the bot.
    let addressed = Some(
        kind == "private"
            || activity["kind"]["mentions_bot"] == true
            || row.gateway_groups.as_deref() != Some("all")
            || match activity["event_id"].as_str() {
                Some(message_id) => {
                    let bot = channel_bot_service::get_bot(&state.db, &row.channel_bot_id).await?;
                    let reply_to = chats::inbound_reply_to(state, row, message_id).await?;
                    chats::replies_to_bot(state, &bot, chat_id, reply_to.as_deref()).await?
                }
                None => false,
            },
    );
    inbound_message(state, row, &chat, sender, text, addressed).await
}

fn is_duplicate(error: &mongodb::error::Error) -> bool {
    matches!(error.kind.as_ref(), mongodb::error::ErrorKind::Write(
        mongodb::error::WriteFailure::WriteError(write)) if write.code == 11000)
}

async fn replay(
    state: &AppState,
    row: &NyxbotChannel,
    event_key: &str,
    response_id: &str,
) -> Response {
    let event = state
        .db
        .collection::<NyxbotEvent>(EVENTS)
        .find_one(doc! {"_id": event_key, "channel_id": &row.id})
        .await
        .ok()
        .flatten();
    let created = json!({"type": "response.created",
        "response": {"id": response_id, "status": "in_progress"}});
    match event.map(|event| event.status) {
        Some(status) if status == "running" => {
            let mut response = problem(StatusCode::CONFLICT, "conversation_in_progress");
            response
                .headers_mut()
                .insert("retry-after", "5".parse().expect("static header"));
            response
        }
        // The answer was already produced; a lost stream is not re-run.
        _ => sse(futures::stream::iter(
            std::iter::once(created).chain(message_frames(response_id, None)),
        )),
    }
}

/// Deliver an asynchronous NyxBot reply (an event turn, e.g. a subagent
/// report) to the chat that owns the conversation. Gateway channels reply to
/// the newest unexpired event reference; direct channels reply to the newest
/// inbound message as the route key. Best effort: the reply stays in the
/// owner's NyxBot history either way.
pub async fn deliver_update(
    state: &AppState,
    row: &crate::models::assistant_conversation::AssistantConversation,
    text: &str,
) {
    // A channel thread answers its chat; the owner's own thread answers the
    // chat they last wrote from.
    let Some(origin) = row.channel.as_ref().or(row.reply_channel.as_ref()) else {
        return;
    };
    deliver_to(state, row, origin, text).await;
}

/// The chat a conversation may deliver to: its own channel thread's chat,
/// or (for the owner's own thread) one of the owner's verified private chats
/// that now answers into it. Anything else (a relinked chat, a group) gets
/// nothing, whatever an older replica may have recorded.
pub(crate) async fn delivery_target(
    state: &AppState,
    row: &crate::models::assistant_conversation::AssistantConversation,
    origin: &ChannelOrigin,
) -> AppResult<Option<(NyxbotChannel, NyxbotThread)>> {
    let channel = load_channel(state, &row.user_id, &origin.nyxbot_channel_id).await?;
    if channel.status != "active" {
        return Ok(None);
    }
    let Some(thread) = state
        .db
        .collection::<NyxbotThread>(THREADS)
        .find_one(doc! {"channel_id": &channel.id, "partition": &origin.partition})
        .await?
    else {
        return Ok(None);
    };
    // A relinked chat belongs to another agent now; this thread's late
    // replies stay in the app.
    if thread.conversation_id.as_deref() != Some(row.id.as_str()) {
        return Ok(None);
    }
    let own_chat = row.channel.as_ref() == Some(origin);
    let owners_private_chat = thread.kind.as_deref() == Some("private") && thread.owner_chat;
    Ok((own_chat || owners_private_chat).then_some((channel, thread)))
}

/// Deliver `text` from the conversation `row` to one chat. Best effort.
pub async fn deliver_to(
    state: &AppState,
    row: &crate::models::assistant_conversation::AssistantConversation,
    origin: &ChannelOrigin,
    text: &str,
) {
    let result: AppResult<()> = async {
        let Some((channel, thread)) = delivery_target(state, row, origin).await? else {
            return Ok(());
        };
        let reply = bounded_reply(text);
        if channel.transport == "gateway" {
            let (Some(ciphertext), Some(expires_at)) = (
                thread.event_ref_ciphertext.as_ref(),
                thread.event_ref_expires_at,
            ) else {
                return Ok(());
            };
            if expires_at <= Utc::now() {
                return Ok(());
            }
            let event_ref = Zeroizing::new(
                String::from_utf8(state.encryption_keys.decrypt(ciphertext).await?)
                    .map_err(|_| AppError::Internal("Event reference unavailable".into()))?,
            );
            let Some(agent_key) = decrypt_agent_key(state, &channel).await? else {
                return Ok(());
            };
            gateway_call(
                state,
                reqwest::Method::POST,
                &format!("/events/{}/replies", urlencode(&event_ref)),
                &agent_key,
                Some(&json!({"text": reply})),
                None,
            )
            .await?;
        } else if let Some(message_id) = thread.last_message_id.as_deref() {
            direct_reply(state, &channel, message_id, &reply, None).await?;
        }
        Ok(())
    }
    .await;
    if let Err(error) = result {
        tracing::debug!(%error, "NyxBot channel update not delivered");
    }
}

// ---------------------------------------------------------------------------
// Direct relay (platforms the gateway relay does not support yet)
// ---------------------------------------------------------------------------

/// Reply through NyxID's own relay: with the message's single-use reply token
/// when present, otherwise as the route key.
async fn direct_reply(
    state: &AppState,
    channel: &NyxbotChannel,
    message_id: &str,
    text: &str,
    reply_token: Option<&str>,
) -> AppResult<()> {
    let mut headers = HeaderMap::new();
    let auth = match reply_token {
        Some(token) => {
            headers.insert(
                "authorization",
                format!("Bearer {token}")
                    .parse()
                    .map_err(|_| AppError::Internal("Reply token unavailable".into()))?,
            );
            None
        }
        None => {
            // The route key acts as the bot's owner: the org for an org bot,
            // and only while the channel's owner still administers it.
            if !org_access_holds(state, channel).await? {
                release_org_channel(state, channel).await?;
                return Err(AppError::Forbidden("org_access_lost".into()));
            }
            let key_owner = bot_owner(channel);
            let key =
                key_service::get_api_key(&state.db, key_owner, &channel.route_api_key_id).await?;
            let mut auth = super::assistant_team::owner_auth(key_owner)?;
            auth.auth_method = crate::mw::auth::AuthMethod::ApiKey;
            auth.api_key_id = Some(key.id.clone());
            auth.api_key_name = Some(key.name.clone());
            auth.scope = key.scopes.clone();
            auth.allow_all_services = false;
            Some(auth)
        }
    };
    super::channel_relay::async_reply(
        State(state.clone()),
        headers,
        crate::mw::auth::OptionalAuthUser(auth),
        Json(super::channel_relay::AsyncReplyRequest {
            message_id: message_id.to_owned(),
            reply: super::channel_relay::AsyncReplyBody {
                text: Some(text.to_owned()),
                metadata: None,
                attachments: Vec::new(),
            },
        }),
    )
    .await
    .map(|_| ())
}

/// `POST /api/v1/nyxbot/relay/{channel_id}`: NyxID's signed relay callback for
/// a direct NyxBot channel. Verified exactly like any agent verifies it, then
/// answered asynchronously through the reply API.
pub async fn relay_callback(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let unauthorized = || problem(StatusCode::UNAUTHORIZED, "invalid_callback");
    if Uuid::parse_str(&id).is_err() {
        return unauthorized();
    }
    let Some(token) = headers
        .get("x-nyxid-callback-token")
        .and_then(|value| value.to_str().ok())
    else {
        return unauthorized();
    };
    let Ok(claims) =
        crate::crypto::jwt::validate_relay_callback_token(&state.jwt_keys, &state.config, token)
    else {
        return unauthorized();
    };
    let Ok(payload) = serde_json::from_slice::<Value>(&body) else {
        return unauthorized();
    };
    let row = match state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .find_one(doc! {"_id": &id, "transport": "direct", "status": "active"})
        .await
    {
        Ok(Some(row)) => row,
        _ => return unauthorized(),
    };
    if claims.body_sha256 != sha256_hex(&body)
        || claims.api_key_id != row.route_api_key_id
        || payload["message_id"] != json!(claims.message_id)
        || payload["platform"] != json!(claims.platform)
        || payload["correlation_id"] != json!(claims.jti)
    {
        return unauthorized();
    }
    // An org bot reaches a person's agent only while they administer the org.
    match org_access_holds(&state, &row).await {
        Ok(true) => {}
        Ok(false) => {
            if let Err(error) = release_org_channel(&state, &row).await {
                tracing::warn!(%error, "NyxBot org channel release deferred");
            }
            return problem(StatusCode::FORBIDDEN, "org_access_lost");
        }
        Err(_) => return problem(StatusCode::SERVICE_UNAVAILABLE, "provider_unavailable"),
    }
    // Dedup redeliveries of the same inbound message.
    let event_key = sha256_hex(format!("{}\0{}", row.id, claims.message_id));
    let now = Utc::now();
    let kind = chats::chat_kind(
        payload["conversation"]["type"]
            .as_str()
            .unwrap_or("private"),
    );
    let chat_id = payload["conversation"]["platform_id"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    let thread_id = payload["thread_id"].as_str().map(str::to_owned);
    // A private chat is one thread per sender (as before); a group, channel
    // or topic is one thread its members share.
    let partition = if kind == "private" {
        format!(
            "direct_{}",
            &sha256_hex(format!(
                "{}\0{}\0{}",
                chat_id,
                payload["sender"]["platform_id"]
                    .as_str()
                    .unwrap_or_default(),
                thread_id.as_deref().unwrap_or_default()
            ))[..32]
        )
    } else {
        chats::group_partition(&chat_id, thread_id.as_deref())
    };
    let admitted = NyxbotEvent {
        id: event_key,
        channel_id: row.id.clone(),
        user_id: row.user_id.clone(),
        partition: partition.clone(),
        event_id: claims.message_id.clone(),
        event_context_ciphertext: None,
        status: "running".into(),
        conversation_id: None,
        turn_id: None,
        created_at: now,
        expires_at: now + ChronoDuration::hours(EVENT_RETENTION_HOURS),
    };
    match state
        .db
        .collection::<NyxbotEvent>(EVENTS)
        .insert_one(&admitted)
        .await
    {
        Ok(_) => {}
        Err(error) if is_duplicate(&error) => return StatusCode::ACCEPTED.into_response(),
        Err(_) => return problem(StatusCode::SERVICE_UNAVAILABLE, "provider_unavailable"),
    }
    let text = payload["content"]["text"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    let reply_token = payload["reply_token"]
        .as_str()
        .map(|token| Zeroizing::new(token.to_owned()));
    let mut sender_id = payload["sender"]["platform_id"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    let mut display = payload["sender"]["display_name"]
        .as_str()
        .map(str::to_owned);
    let raw = payload["raw_platform_data"].clone();
    let raw_title = ["message", "edited_message", "channel_post"]
        .iter()
        .find_map(|key| raw.get(*key))
        .and_then(|message| message["chat"]["title"].as_str())
        .map(str::to_owned);
    // A channel post comes from the channel itself.
    if sender_id.is_empty() && kind == "channel" {
        sender_id = format!("channel:{chat_id}");
        display = display.or_else(|| raw_title.clone());
    }
    let reply_to = payload["reply_to_platform_message_id"]
        .as_str()
        .map(str::to_owned);
    let message_id = claims.message_id.clone();
    tokio::spawn(async move {
        let result: AppResult<()> = async {
            let chat = chats::record_chat(
                &state,
                &row,
                &partition,
                &chats::ChatFacts {
                    kind,
                    chat_id: chat_id.clone(),
                    thread_id,
                    owner: row.owner_sender_ids.iter().any(|id| id == &sender_id),
                    title: if kind == "private" {
                        Some(chats::private_title(&row, &sender_id, display.as_deref()))
                    } else {
                        raw_title
                    },
                },
                Some(&message_id),
            )
            .await?;
            chats::spawn_title_lookup(&state, &row, &chat);
            if text.trim().is_empty() || sender_id.is_empty() {
                return Ok(());
            }
            let addressed = if kind == "private" {
                Some(true)
            } else {
                let bot = channel_bot_service::get_bot(&state.db, &row.channel_bot_id).await?;
                if chats::replies_to_bot(&state, &bot, &chat_id, reply_to.as_deref()).await? {
                    Some(true)
                } else {
                    chats::raw_addressed(&bot, &raw)
                }
            };
            let sender = Sender {
                id: &sender_id,
                display_name: display.as_deref(),
            };
            let reply =
                match inbound_message(&state, &row, &chat, &sender, &text, addressed).await? {
                    Inbound::Reply(text) => Some(text),
                    Inbound::Silent => None,
                    Inbound::Busy => Some(
                        "I'm still working on the previous message. I'll pick this up next.".into(),
                    ),
                    Inbound::Turn(receiver) => match final_reply(receiver).await {
                        Ok(text) => Some(bounded_reply(&text)),
                        Err(_) => Some("I could not finish that. Please try again.".into()),
                    },
                };
            if let Some(reply) = reply {
                direct_reply(
                    &state,
                    &row,
                    &message_id,
                    &reply,
                    reply_token.as_deref().map(String::as_str),
                )
                .await?;
            }
            Ok(())
        }
        .await;
        if let Err(error) = result {
            tracing::debug!(%error, "NyxBot direct channel message not answered");
        }
    });
    StatusCode::ACCEPTED.into_response()
}

#[path = "nyxbot_chats.rs"]
pub(crate) mod chats;
pub use chats::{list_channel_chats, update_channel_chat};

#[cfg(test)]
#[path = "nyxbot_tests.rs"]
mod tests;

#[path = "nyxbot_status.rs"]
mod status;
pub(crate) use status::{WaitingItem, check_deliveries, waiting};

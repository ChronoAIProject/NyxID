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
    let bot: ChannelBot = channel_bot_service::get_bot_for_user(&state.db, bot_id, owner).await?;
    if !bot.is_active {
        return Err(AppError::ValidationError(
            "That channel bot is not active".into(),
        ));
    }
    // Owners verified on a connection being rebuilt stay verified: the bot
    // and their chat-app account are the same.
    let mut verified_owners: Vec<String> = Vec::new();
    if let Some(existing) = active_for_bot(state, owner, &bot.id).await? {
        let keys_alive = key_service::get_api_key(&state.db, owner, &existing.route_api_key_id)
            .await
            .is_ok_and(|key| key.is_active)
            && match existing.agent_api_key_id.as_deref() {
                Some(agent) => key_service::get_api_key(&state.db, owner, agent)
                    .await
                    .is_ok_and(|key| key.is_active),
                None => true,
            };
        // A channel whose messages stopped arriving is rebuilt from scratch.
        let healthy = existing.delivery_status.as_deref() != Some("failing");
        if existing.status == "active" && keys_alive && healthy {
            link(state, owner, &existing.id, agent).await?;
            let existing = load_channel(state, owner, &existing.id).await?;
            let link = refresh_link_code(state, &existing).await?;
            return Ok((existing, link));
        }
        // A half-created or broken connection (e.g. a key was deleted) is
        // released first, then rebuilt from scratch.
        verified_owners = existing.owner_sender_ids.clone();
        disconnect(state, owner, &existing.id).await?;
    }
    // Never silently replace another agent's default route.
    let routes =
        channel_routing_service::list_conversations(&state.db, owner, Some(&bot.id)).await?;
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
    let gateway = platform == "telegram";
    let now = Utc::now();
    let id = Uuid::new_v4().to_string();
    let label = excerpt(&bot.label, 60);
    let callback = if gateway {
        format!("{}/callbacks/pending", gateway_base(state).await?)
    } else {
        format!(
            "{}/api/v1/nyxbot/relay/{id}",
            state.config.base_url.trim_end_matches('/')
        )
    };
    let route_key = key_service::create_api_key(
        &state.db,
        owner,
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
    let outcome = if gateway {
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
            let row = load_channel(state, owner, &id).await?;
            let link = refresh_link_code(state, &row).await?;
            Ok((row, link))
        }
        Err(code) => {
            // Best-effort rollback of everything this attempt created.
            let _ = key_service::delete_api_key(&state.db, owner, &route_key.id).await;
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

fn gateway_policy(
    state: &AppState,
    row: &NyxbotChannel,
    bot: &ChannelBot,
    route_ids: &[String],
) -> Value {
    let username = bot.platform_bot_username.trim_start_matches('@');
    let mut source = json!({
        "type": "nyxid_relay",
        "issuer": state.config.jwt_issuer,
        "key_id": row.route_api_key_id,
        "route_ids": route_ids,
        "platform": "telegram",
        "admission": {"type": "scoped", "senders": {"type": "open"},
            "chats": {"type": "open"}, "groups": "mention_or_reply_to_bot"},
    });
    let valid_username = (5..=32).contains(&username.len())
        && username
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_');
    if valid_username {
        source["bot_username"] = json!(username);
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
    let mut body = gateway_policy(state, row, bot, &[]);
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
    let mut update = gateway_policy(state, row, bot, std::slice::from_ref(&route.id));
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
                    doc! {"$set": {"gateway_version": version}},
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

async fn connect_direct(
    state: &AppState,
    row: &NyxbotChannel,
    bot: &ChannelBot,
) -> Result<String, &'static str> {
    channel_routing_service::create_conversation(
        &state.db,
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
        match channel_routing_service::delete_conversation(&state.db, route_id, owner).await {
            Ok(()) | Err(AppError::NotFound(_)) => {}
            Err(error) => return Err(error),
        }
    }
    for key in [
        Some(row.route_api_key_id.as_str()),
        row.agent_api_key_id.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        match key_service::delete_api_key(&state.db, owner, key).await {
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
    Ok((
        json!({
            "channel_agent": ChannelAgentResponse::from(&row),
            "agent": agent.name,
            "link": link_json(&row, &link),
            "note": if row.owner_sender_ids.is_empty() {
                "Give the user the link (or code). Until they use it, the bot answers nobody."
            } else {
                "The user's linked account already reaches the bot; the link adds another."
            },
        }),
        false,
    ))
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
            doc! {"channel_id": &row.id, "user_id": owner},
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
    Ok(json!({"channel_agents": rows.iter().map(ChannelAgentResponse::from).collect::<Vec<_>>()}))
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
    agent_id: String,
}

pub async fn link_channel(
    State(state): State<AppState>,
    auth: crate::mw::auth::AuthUser,
    Path(id): Path<String>,
    Json(body): Json<LinkRequest>,
) -> AppResult<Json<Value>> {
    let owner = auth.user_id.to_string();
    engine::require_enabled(&state.db, &owner).await?;
    let agent = requested_agent(&state, &owner, Some(&body.agent_id)).await?;
    Ok(Json(link(&state, &owner, &id, &agent).await?))
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
    private_chat: bool,
}

/// Link a sender presenting the one-time code, or decide whether this sender
/// may reach the owner's NyxBot.
async fn admit_sender(
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
    if row.owner_sender_ids.iter().any(|id| id == sender.id) {
        return Ok(None);
    }
    Ok(Some(if sender.private_chat {
        Inbound::Reply(
            "This NyxBot answers only its owner. If this is your bot, ask NyxBot in NyxID \
            for a link and open it here."
                .into(),
        )
    } else {
        Inbound::Silent
    }))
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

async fn start_owner_turn(
    state: &AppState,
    row: &NyxbotChannel,
    partition: &str,
    sender: &Sender<'_>,
    text: &str,
) -> AppResult<Inbound> {
    let (_, conversation_id) = thread_conversation(state, row, partition).await?;
    let exists = engine::get(&state.db, &row.user_id, &conversation_id)
        .await
        .is_ok();
    // A chat app cannot show NyxID's confirmation cards: the verified owner
    // answers one in words (see `decide_reply` for which card it decides).
    let mut answered = None;
    if exists {
        match reply_decision(state, row, &conversation_id, text).await {
            Ok(decided) => answered = decided,
            Err(error) => tracing::debug!(%error, "Chat confirmation not applied"),
        }
    }
    let chat = if sender.private_chat {
        "a private chat"
    } else {
        "a group chat"
    };
    let mut note = format!(
        "This message came through the owner's {} channel bot {} from {} ({chat}); NyxID \
        verified this sender as the owner.",
        identifier(&row.platform),
        identifier(&row.bot_label),
        sender
            .display_name
            .map(|name| format!("\"{}\"", excerpt(name, 60).replace('"', "'")))
            .unwrap_or_else(|| "the owner".into()),
    );
    if let Some(allow) = answered {
        note.push_str(if allow {
            " With this message the owner confirmed the pending action; retry it with its \
            acknowledgement_id."
        } else {
            " With this message the owner declined the pending action; do not retry it."
        });
    }
    let start = TurnStart {
        conversation_id: exists.then(|| conversation_id.clone()),
        new_id: (!exists).then(|| conversation_id.clone()),
        text: excerpt(text, engine::MAX_MESSAGE_CHARS - 16),
        model: Some(
            crate::services::assistant_profile_routing::model_for(
                &state.db,
                crate::services::assistant_profile_routing::RouteRole::Channel,
                engine::DEFAULT_MODEL,
            )
            .await,
        ),
        origin: TurnOrigin::Channel,
        channel: Some(ChannelOrigin {
            nyxbot_channel_id: row.id.clone(),
            partition: partition.to_owned(),
            platform: row.platform.clone(),
        }),
        title: Some(format!("{} · {}", row.platform, row.bot_label)),
        note: Some(note),
        agent_id: Some(match row.agent_id.clone() {
            Some(agent_id) => agent_id,
            None => {
                crate::services::assistant_team_service::ensure_nyxbot(&state.db, &row.user_id)
                    .await?
                    .id
            }
        }),
        report_to: None,
        group_id: None,
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
        // The chat is busy (or the owner's channel pool is full): queue the
        // message for NyxBot's next turn instead of bouncing it. Its reply is
        // an asynchronous update delivered back to this chat.
        Ok(Started::Busy | Started::PoolFull) if exists => {
            let note = format!(
                "The owner sent another {} message while you were working. Answer it \
                next; your reply is delivered to the chat: \"{}\"",
                identifier(&row.platform),
                excerpt(text, 3000).replace('"', "'")
            );
            let queued = engine::push_events(
                &state.db,
                &row.user_id,
                &conversation_id,
                vec![crate::services::assistant_team_service::event(
                    "message", note, None,
                )],
            )
            .await?;
            if queued.is_none() {
                return Ok(Inbound::Busy);
            }
            super::assistant_team::wake(state, &row.user_id, &conversation_id).await;
            Ok(Inbound::Reply(
                "Got it. I'm finishing your previous request and will answer this right after."
                    .into(),
            ))
        }
        Ok(Started::Busy | Started::PoolFull) => Ok(Inbound::Busy),
        // A concurrent first message created the chat: it now exists.
        Err(AppError::Conflict(_)) | Err(AppError::DatabaseError(_)) if !exists => {
            Ok(Inbound::Busy)
        }
        Err(error) => Err(error),
    }
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
    // Keep the newest event reference for asynchronous replies (encrypted).
    if let Some(event_ref) = context["event_ref"].as_str()
        && let Ok(sealed) = state.encryption_keys.encrypt(event_ref.as_bytes()).await
    {
        let _ = state
            .db
            .collection::<NyxbotThread>(THREADS)
            .update_one(
                doc! {"channel_id": &row.id, "partition": &partition},
                doc! {"$set": {"event_ref_ciphertext": bson::Binary {
                    subtype: bson::spec::BinarySubtype::Generic, bytes: sealed },
                "event_ref_expires_at": bson::DateTime::from_chrono(
                    now + ChronoDuration::minutes(29)),
                "updated_at": bson::DateTime::now()}},
            )
            .await;
    }
    let sender_id = activity["actor"]["id"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    let display = activity["actor"]["display_name"]
        .as_str()
        .map(str::to_owned);
    let private_chat = activity["conversation"]["kind"] == "private";
    let human = activity["actor"]["kind"] == "human";
    let sender = Sender {
        id: &sender_id,
        display_name: display.as_deref(),
        private_chat,
    };
    let inbound = if !human || sender_id.is_empty() {
        Ok(Inbound::Silent)
    } else {
        match admit_sender(&state, &row, &sender, &text).await {
            Ok(Some(decision)) => Ok(decision),
            Ok(None) => start_owner_turn(&state, &row, &partition, &sender, &text).await,
            Err(error) => Err(error),
        }
    };
    let finish = |status: &'static str, conversation: Option<String>| {
        let state = state.clone();
        let event_key = event_key.clone();
        async move {
            let _ = state
                .db
                .collection::<NyxbotEvent>(EVENTS)
                .update_one(
                    doc! {"_id": &event_key},
                    doc! {"$set": {"status": status, "conversation_id": conversation}},
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
    let Some(origin) = row.channel.as_ref() else {
        return;
    };
    let result: AppResult<()> = async {
        let channel = load_channel(state, &row.user_id, &origin.nyxbot_channel_id).await?;
        if channel.status != "active" {
            return Ok(());
        }
        let thread = state
            .db
            .collection::<NyxbotThread>(THREADS)
            .find_one(doc! {"channel_id": &channel.id, "partition": &origin.partition})
            .await?
            .ok_or_else(|| AppError::NotFound("Channel thread not found".into()))?;
        // A relinked chat belongs to another agent now; this thread's late
        // replies stay in the app.
        if thread.conversation_id.as_deref() != Some(row.id.as_str()) {
            return Ok(());
        }
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
            let key =
                key_service::get_api_key(&state.db, &channel.user_id, &channel.route_api_key_id)
                    .await?;
            let mut auth = super::assistant_team::owner_auth(&channel.user_id)?;
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
    // Dedup redeliveries of the same inbound message.
    let event_key = sha256_hex(format!("{}\0{}", row.id, claims.message_id));
    let now = Utc::now();
    let partition = format!(
        "direct_{}",
        &sha256_hex(format!(
            "{}\0{}\0{}",
            payload["conversation"]["platform_id"]
                .as_str()
                .unwrap_or_default(),
            payload["sender"]["platform_id"]
                .as_str()
                .unwrap_or_default(),
            payload["thread_id"].as_str().unwrap_or_default()
        ))[..32]
    );
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
    let sender_id = payload["sender"]["platform_id"]
        .as_str()
        .unwrap_or_default()
        .to_owned();
    let display = payload["sender"]["display_name"]
        .as_str()
        .map(str::to_owned);
    let private_chat = payload["conversation"]["type"] == "private";
    let message_id = claims.message_id.clone();
    tokio::spawn(async move {
        let result: AppResult<()> = async {
            state
                .db
                .collection::<NyxbotThread>(THREADS)
                .update_one(
                    doc! {"channel_id": &row.id, "partition": &partition},
                    doc! {"$setOnInsert": {"_id": Uuid::new_v4().to_string(),
                    "user_id": &row.user_id, "created_at": bson::DateTime::now()},
                    "$set": {"last_message_id": &message_id,
                        "updated_at": bson::DateTime::now()}},
                )
                .upsert(true)
                .await?;
            if text.trim().is_empty() || sender_id.is_empty() {
                return Ok(());
            }
            let sender = Sender {
                id: &sender_id,
                display_name: display.as_deref(),
                private_chat,
            };
            let inbound = match admit_sender(&state, &row, &sender, &text).await? {
                Some(decision) => decision,
                None => start_owner_turn(&state, &row, &partition, &sender, &text).await?,
            };
            let reply = match inbound {
                Inbound::Reply(text) => Some(text),
                Inbound::Silent => None,
                Inbound::Busy => Some(
                    "I'm still working on your previous message. I'll pick this up next.".into(),
                ),
                Inbound::Turn(receiver) => match final_reply(receiver).await {
                    Ok(text) => Some(bounded_reply(&text)),
                    Err(_) => Some("NyxBot could not finish that. Please try again.".into()),
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

#[cfg(test)]
#[path = "nyxbot_tests.rs"]
mod tests;

#[path = "nyxbot_status.rs"]
mod status;
pub(crate) use status::{WaitingItem, check_deliveries, waiting};

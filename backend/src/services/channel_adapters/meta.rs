//! Customer-initiated Meta inboxes. Provider IDs are meaningful only within
//! their platform, application, asset and local bot binding.

use axum::http::HeaderMap;
use bson::doc;
use chrono::{DateTime, Duration, Utc};
use hmac::{Hmac, Mac};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

use crate::services::{
    channel_platform::*,
    channel_registration::{BOT_TOKEN_FIELD, RegistrationDescriptor, RegistrationField},
};
use crate::{
    errors::{AppError, AppResult},
    models::{channel_bot::ChannelBot, channel_message::ChannelMessage},
};

pub const GRAPH_VERSION: &str = "v25.0";
const RESPONSE_LIMIT: usize = 256 * 1024;
const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);
pub const CONTROLS: &str = "channel_meta_controls";

pub fn is_meta(platform: &str) -> bool {
    matches!(platform, "messenger" | "instagram")
}

#[derive(Clone, Copy)]
pub enum Inbox {
    Messenger,
    Instagram,
}

impl Inbox {
    pub fn id(self) -> &'static str {
        match self {
            Self::Messenger => "messenger",
            Self::Instagram => "instagram",
        }
    }
    fn object(self) -> &'static str {
        match self {
            Self::Messenger => "page",
            Self::Instagram => "instagram",
        }
    }
    pub fn host(self) -> &'static str {
        match self {
            Self::Messenger => "https://graph.facebook.com",
            Self::Instagram => "https://graph.instagram.com",
        }
    }
}

pub struct MetaAdapter {
    pub inbox: Inbox,
    pub(crate) graph_base: String,
}
impl MetaAdapter {
    pub fn new(inbox: Inbox) -> Self {
        Self {
            inbox,
            graph_base: {
                #[cfg(test)]
                if let Ok(base) = TEST_GRAPH_BASE.try_with(Clone::clone) {
                    return Self {
                        inbox,
                        graph_base: base,
                    };
                }
                inbox.host().into()
            },
        }
    }
    pub(crate) fn url(&self, path: &str) -> String {
        format!("{}/{GRAPH_VERSION}/{path}", self.graph_base)
    }

    pub(crate) async fn prove_control(
        &self,
        db: &mongodb::Database,
        http: &reqwest::Client,
        bot: &ChannelBot,
        sender: &str,
        token: &str,
    ) -> AppResult<()> {
        let binding = bot
            .meta_binding
            .as_ref()
            .ok_or_else(|| unreachable("binding_unverified"))?;
        if !valid_id(sender) || binding.token_fingerprint != token_fingerprint(token) {
            return Err(unreachable("credential_authority_changed"));
        }
        let generation = binding.generation;
        let chat = conversation(&bot.platform, &bot.id, &bot.platform_bot_id, sender);
        let key = format!("{}:{chat}", bot.id);
        // Read the participant revision before provider I/O. The provider
        // request must never run with a Mongo transaction or bot write lock
        // open behind it.
        let expected_revision = db
            .collection::<bson::Document>(CONTROLS)
            .find_one(doc! {"identity_key": &key})
            .await?
            .map(|row| row.get_i64("revision").unwrap_or(0));
        let owns = match self.inbox {
            Inbox::Messenger => {
                let value = graph_json_read(
                    http.get(self.url(&format!("{}/thread_owner", bot.platform_bot_id)))
                        .bearer_auth(token)
                        .query(&[("recipient", sender)]),
                )
                .await?;
                let app = &value["data"][0]["thread_owner"]["app_id"];
                app.as_str()
                    .map(str::to_owned)
                    .or_else(|| app.as_u64().map(|n| n.to_string()))
                    .as_deref()
                    == Some(binding.app_id.as_str())
            }
            // Instagram Login has no supported participant ownership read.
            // Its initial exclusive grant is recorded, while an observed loss
            // remains false until a new reconnect establishes a new generation.
            Inbox::Instagram => binding.exclusive_routing_verified,
        };
        let args = (
            db.clone(),
            bot.clone(),
            key,
            generation,
            expected_revision,
            owns,
            token_fingerprint(token),
        );
        let recorded = crate::services::transaction_service::run_db_only(
            db,
            &args,
            |tx, (db, bot, key, generation, expected_revision, owns, token_fingerprint)| {
                Box::pin(async move {
                    let live = db
                        .collection::<ChannelBot>(
                            crate::models::channel_bot::COLLECTION_NAME,
                        )
                        .find_one_and_update(
                            doc! {
                                "_id": &bot.id,
                                "user_id": &bot.user_id,
                                "is_active": true,
                                "meta_binding.generation": *generation,
                            },
                            doc! {"$inc": {"meta_control_revision": 1_i64}},
                        )
                        .return_document(mongodb::options::ReturnDocument::After)
                        .session(&mut *tx)
                        .await?
                        .ok_or_else(|| unreachable("credential_authority_changed"))?;
                    let live_binding = live
                        .meta_binding
                        .as_ref()
                        .ok_or_else(|| unreachable("binding_unverified"))?;
                    if live_binding.token_fingerprint != *token_fingerprint {
                        return Err(unreachable("credential_authority_changed"));
                    }
                    crate::services::channel_meta_service::scopes::fence_current(db,tx,&live).await?;
                    let current = db
                        .collection::<bson::Document>(CONTROLS)
                        .find_one(doc! {"identity_key": key})
                        .session(&mut *tx)
                        .await?;
                    if let Some(row) = &current {
                        if matches!(bot.platform.as_str(), "instagram")
                            && control_for_grant(row, live_binding)
                            && row.get_bool("controlled").ok() == Some(false)
                        {
                            // Instagram has no participant ownership read. A
                            // provider-observed loss therefore survives later
                            // DMs and checkbox/Verify attempts until the
                            // owner reconnects with fresh consent.
                            return Ok(false);
                        }
                        let current_revision = row.get_i64("revision").unwrap_or(0);
                        if *expected_revision != Some(current_revision) {
                            if row.get_bool("controlled").ok() == Some(*owns) {
                                return Ok(*owns);
                            }
                            return Err(unreachable("inbox_control_lost"));
                        }
                    } else if expected_revision.is_some() {
                        return Err(unreachable("credential_authority_changed"));
                    }
                    let has_current = current.is_some();
                    let current_generation = current
                        .as_ref()
                        .and_then(|row| row.get_i64("generation").ok())
                        .unwrap_or(*generation);
                    let mut filter = doc! {
                        "identity_key": key,
                        "generation": current_generation,
                    };
                    if has_current && *expected_revision == Some(0) {
                        filter.insert(
                            "$or",
                            bson::bson!([{ "revision": 0_i64 }, { "revision": { "$exists": false } }]),
                        );
                    } else {
                        filter.insert("revision", expected_revision.unwrap_or(0));
                    }
                    let changed = db
                        .collection::<bson::Document>(CONTROLS)
                        .update_one(
                            filter,
                            doc! {
                                "$set": {
                                    "bot_id": &bot.id,
                                    "meta_grant_id": &live_binding.grant_id,
                                    "controlled": *owns,
                                    "generation": *generation,
                                    "revision": expected_revision.unwrap_or(0) + 1,
                                    "at": bson::DateTime::now(),
                                },
                                "$setOnInsert": {"_id": uuid::Uuid::new_v4().to_string()},
                            },
                        )
                        .upsert(true)
                        .session(&mut *tx)
                        .await?;
                    if changed.matched_count != 1 && changed.upserted_id.is_none() {
                        return Err(unreachable("credential_authority_changed"));
                    }
                    Ok(*owns)
                })
            },
        )
        .await?;
        if !recorded {
            return Err(unreachable("inbox_control_lost"));
        }
        require_ingress(db, bot, &chat).await
    }
}

enum GraphSend {
    Accepted(String),
    Refused(AppError),
    Uncertain(AppError),
}

async fn graph_send(request: reqwest::RequestBuilder) -> GraphSend {
    let unknown = || {
        AppError::ChannelPlatformError(
            "Meta send acceptance is unknown; inspect the delivery journal before sending again"
                .into(),
        )
    };
    let mut response = match request.timeout(TIMEOUT).send().await {
        Ok(response) => response,
        Err(_) => return GraphSend::Uncertain(unknown()),
    };
    let status = response.status();
    let mut bytes = Vec::new();
    loop {
        match response.chunk().await {
            Ok(Some(chunk)) if bytes.len().saturating_add(chunk.len()) <= RESPONSE_LIMIT => {
                bytes.extend_from_slice(&chunk)
            }
            Ok(None) => break,
            _ => return GraphSend::Uncertain(unknown()),
        }
    }
    let Ok(value) = serde_json::from_slice::<Value>(&bytes) else {
        return GraphSend::Uncertain(unknown());
    };
    if status.is_success() && value.get("error").is_none() {
        return match value["message_id"]
            .as_str()
            .filter(|id| crate::services::channel_delivery_service::valid_message_id(id))
        {
            Some(id) => GraphSend::Accepted(id.into()),
            None => GraphSend::Uncertain(unknown()),
        };
    }
    let code = value["error"]["code"].as_i64().unwrap_or(0);
    let error = match code {
        190 | 102 => unreachable("reconnect_required"),
        10 | 200 => unreachable("permission_or_window_refused"),
        _ if status.as_u16() == 429 || matches!(code, 4 | 17 | 32 | 613) => {
            AppError::RateLimitedWithRetry { retry_after: 1 }
        }
        _ => AppError::ChannelPlatformError(format!(
            "Meta send refused (HTTP {}, code {code})",
            status.as_u16()
        )),
    };
    if status.is_client_error() && value["error"].is_object() {
        GraphSend::Refused(error)
    } else {
        GraphSend::Uncertain(unknown())
    }
}

pub fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 32 && id.bytes().all(|b| b.is_ascii_digit())
}

/// Meta IDs may be JSON strings or integer numbers. Reject floats and values
/// outside serde_json's exact integer range instead of rounding identifiers.
pub(crate) fn identity_value(value: &Value) -> Option<String> {
    value
        .as_str()
        .filter(|id| valid_id(id))
        .map(str::to_owned)
        .or_else(|| value.as_u64().map(|id| id.to_string()))
}

pub(crate) fn instagram_record(value: &Value) -> AppResult<&Value> {
    let invalid =
        || AppError::ValidationError("Instagram must return one unambiguous account record".into());
    let object = value.as_object().ok_or_else(invalid)?;
    if let Some(data) = object.get("data") {
        let rows = data
            .as_array()
            .filter(|rows| rows.len() == 1)
            .ok_or_else(invalid)?;
        let record = &rows[0];
        let fields = record.as_object().ok_or_else(invalid)?;
        for key in [
            "access_token",
            "permissions",
            "id",
            "user_id",
            "username",
            "account_type",
        ] {
            if let Some(direct) = object.get(key)
                && fields.get(key) != Some(direct)
            {
                return Err(invalid());
            }
        }
        return Ok(record);
    }
    // Some deployed versions return the same single account directly. Accept
    // that form only with its identity or token+permission fields; callers still
    // enforce the exact scopes, professional type and selected inbox identity.
    let subject = identity_value(&value["user_id"]).is_some();
    let token = value["access_token"]
        .as_str()
        .is_some_and(|token| !token.is_empty())
        && value["permissions"].as_str().is_some();
    let identity = value["account_type"].as_str().is_some();
    if subject && (token || identity) {
        Ok(value)
    } else {
        Err(invalid())
    }
}
fn signature_error() -> AppError {
    AppError::ChannelWebhookVerificationFailed("Invalid Meta inbox signature".into())
}
pub fn unreachable(reason: &str) -> AppError {
    AppError::ChannelConversationNotReachable(format!("meta:{reason}"))
}

/// Verification can establish remote availability, never replace consent.
pub(crate) fn require_verifiable(bot: &ChannelBot) -> AppResult<()> {
    if bot
        .meta_binding
        .as_ref()
        .is_some_and(|binding| binding.operator_suspended || binding.subscription == "suspended")
    {
        return Err(unreachable("platform_suspended"));
    }
    require_resumable(bot)
}

pub(crate) fn require_resumable(bot: &ChannelBot) -> AppResult<()> {
    let binding = bot
        .meta_binding
        .as_ref()
        .ok_or_else(|| unreachable("reconnect_required"))?;
    if !bot.is_active
        || !binding.exclusive_routing_verified
        || binding.provider_revoked
        || binding.grant_reconnect_required
        || !matches!(
            binding.subscription.as_str(),
            "active" | "pending" | "suspended"
        )
        || (bot.platform == "instagram" && binding.expires_at.is_some_and(|at| at <= Utc::now()))
    {
        return Err(unreachable("reconnect_required"));
    }
    Ok(())
}

pub fn verify_signature(secret: &str, headers: &HeaderMap, body: &[u8]) -> AppResult<()> {
    let signature = headers
        .get("x-hub-signature-256")
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("sha256="))
        .filter(|s| s.len() == 64)
        .ok_or_else(signature_error)?;
    let bytes = hex::decode(signature).map_err(|_| signature_error())?;
    if secret.is_empty() {
        return Err(signature_error());
    }
    let mut mac =
        Hmac::<Sha256>::new_from_slice(secret.as_bytes()).map_err(|_| signature_error())?;
    mac.update(body);
    mac.verify_slice(&bytes).map_err(|_| signature_error())
}

fn validate_handshake(query: &std::collections::HashMap<String, String>) -> AppResult<()> {
    if query.get("hub.mode").map(String::as_str) != Some("subscribe")
        || query
            .get("hub.verify_token")
            .is_none_or(|s| s.is_empty() || s.len() > 512)
        || query.get("hub.challenge").is_none_or(|s| {
            s.is_empty() || s.len() > 1024 || !s.bytes().all(|b| b.is_ascii_digit())
        })
    {
        return Err(signature_error());
    }
    Ok(())
}

pub(crate) async fn graph_json(request: reqwest::RequestBuilder) -> AppResult<Value> {
    let response = request.timeout(TIMEOUT).send().await.map_err(|_| {
        AppError::ChannelPlatformError("Meta request could not complete; retry verification".into())
    })?;
    let status = response.status();
    let mut response = response;
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| {
        AppError::ChannelPlatformError("Meta response interrupted; retry verification".into())
    })? {
        if bytes.len().saturating_add(chunk.len()) > RESPONSE_LIMIT {
            return Err(AppError::ChannelPlatformError(
                "Meta response exceeded limit; retry verification".into(),
            ));
        }
        bytes.extend_from_slice(&chunk);
    }
    let value: Value = match serde_json::from_slice(&bytes) {
        Ok(value) => value,
        Err(_) if !status.is_success() => Value::Null,
        Err(_) => {
            return Err(AppError::ChannelPlatformError(
                "Invalid Meta response; retry verification".into(),
            ));
        }
    };
    if !status.is_success() || value.get("error").is_some() {
        let code = value["error"]["code"].as_i64().unwrap_or(0);
        return Err(match code {
            190 | 102 => unreachable("reconnect_required"),
            10 | 200 => unreachable("permission_or_window_refused"),
            _ if status.as_u16() == 429 || matches!(code, 4 | 17 | 32 | 613) => {
                AppError::RateLimitedWithRetry { retry_after: 1 }
            }
            _ => AppError::ChannelPlatformError(format!(
                "Meta request refused (HTTP {}, code {code})",
                status.as_u16()
            )),
        });
    }
    Ok(value)
}

/// Idempotent Graph reads may be retried briefly after a transport or 5xx
/// failure. Mutating requests continue to use `graph_json` exactly once.
pub(crate) async fn graph_json_read(request: reqwest::RequestBuilder) -> AppResult<Value> {
    let mut request = Some(request);
    for attempt in 0..3 {
        let current = request.take().ok_or_else(|| {
            AppError::ChannelPlatformError("Meta read retry could not be rebuilt".into())
        })?;
        let retry = current.try_clone();
        match graph_json(current).await {
            Ok(value) => return Ok(value),
            Err(error) if attempt < 2 && is_transient_read_error(&error) => {
                tokio::time::sleep(std::time::Duration::from_millis(75 * (attempt + 1))).await;
                request = retry;
                if request.is_none() {
                    return Err(error);
                }
            }
            Err(error) => return Err(error),
        }
    }
    Err(AppError::ChannelPlatformError(
        "Meta read failed; retry verification".into(),
    ))
}

fn is_transient_read_error(error: &AppError) -> bool {
    matches!(error,
        AppError::ChannelPlatformError(message)
            if message.contains("could not complete")
                || message.contains("interrupted")
                || message.contains("HTTP 5"))
}

pub fn binding(bot: &ChannelBot) -> String {
    let mut hash = Sha256::new();
    // Length-delimit identifiers; operational diagnostics do not change consent.
    for value in [
        bot.platform.as_str(),
        &bot.id,
        &bot.user_id,
        &bot.platform_bot_id,
        bot.app_id.as_deref().unwrap_or_default(),
        bot.connection_id.as_deref().unwrap_or_default(),
        &bot.webhook_secret_hash,
    ] {
        hash.update((value.len() as u64).to_be_bytes());
        hash.update(value.as_bytes());
    }
    hash.update(bot.ownership_version.to_be_bytes());
    hex::encode(hash.finalize())
}

/// A server-generated conversation key, reversible only after exact binding checks.
pub fn conversation(platform: &str, bot_id: &str, asset: &str, sender: &str) -> String {
    format!("{platform}:{bot_id}:{asset}:{sender}")
}

fn parse_event(inbox: Inbox, bot_id: &str, asset: &str, event: &Value) -> Option<InboundMessage> {
    let sender = event["sender"]["id"].as_str().filter(|s| valid_id(s))?;
    if sender == asset || event["recipient"]["id"].as_str() != Some(asset) {
        return None;
    }
    // A text field on an otherwise unsupported envelope does not make it a DM.
    if [
        "delivery",
        "read",
        "postback",
        "reaction",
        "referral",
        "optin",
        "group_id",
        "pass_thread_control",
        "take_thread_control",
        "request_thread_control",
    ]
    .iter()
    .any(|k| event.get(k).is_some())
    {
        return None;
    }
    let message = event.get("message")?;
    if ["is_echo", "is_deleted", "is_unsupported"]
        .iter()
        .any(|k| message[*k].as_bool() == Some(true))
        || ["attachments", "story", "referral", "reaction", "deleted"]
            .iter()
            .any(|k| message.get(k).is_some())
    {
        return None;
    }
    if let Some(quoted) = message.get("reply_to")
        && (quoted.get("story").is_some()
            || quoted["mid"]
                .as_str()
                .is_none_or(|id| !crate::services::channel_delivery_service::valid_message_id(id)))
    {
        return None;
    }
    let mid = message["mid"]
        .as_str()
        .filter(|s| crate::services::channel_delivery_service::valid_message_id(s))?;
    let text = message["text"].as_str().filter(|s| !s.trim().is_empty())?;
    let timestamp = event["timestamp"].as_i64()?;
    let at = DateTime::from_timestamp_millis(timestamp)?;
    if at > Utc::now() || at <= Utc::now() - Duration::hours(24) {
        return None;
    }
    let chat = conversation(inbox.id(), bot_id, asset, sender);
    Some(InboundMessage {
        embedded_attachments: Vec::new(),
        embedded_attachments_omitted: 0,
        platform_message_id: mid.into(),
        conversation_id: chat.clone(),
        conversation_type: "private".into(),
        sender_platform_id: chat,
        sender_display_name: None,
        content_type: "text".into(),
        text: Some(text.into()),
        attachments: Vec::new(),
        reply_to_platform_message_id: message["reply_to"]["mid"].as_str().map(str::to_owned),
        thread_id: None,
        // An explicit allowlist prevents neighboring businesses/customer events,
        // credentials and unsupported event fields entering any agent callback.
        raw_data: json!({"object":inbox.object(),
            "asset_id":asset,
            "sender":{"id":sender},
            "recipient":{"id":asset},
            "timestamp":timestamp,
            "message":{"mid":mid,
                "text":text}}),
    })
}

pub(crate) fn authority(
    bot: &ChannelBot,
    inbound: &InboundMessage,
    ttl: i64,
) -> AppResult<crate::models::channel_message::MetaMessageAuthority> {
    let asset = inbound.raw_data["asset_id"]
        .as_str()
        .ok_or_else(|| unreachable("asset_mismatch"))?;
    let sender = inbound.raw_data["sender"]["id"]
        .as_str()
        .ok_or_else(|| unreachable("sender_mismatch"))?;
    let timestamp = inbound.raw_data["timestamp"]
        .as_i64()
        .and_then(DateTime::from_timestamp_millis)
        .ok_or_else(|| unreachable("event_time_missing"))?;
    if asset != bot.platform_bot_id
        || inbound.conversation_id != conversation(&bot.platform, &bot.id, asset, sender)
    {
        return Err(unreachable("asset_mismatch"));
    }
    let generation = bot
        .meta_binding
        .as_ref()
        .ok_or_else(|| unreachable("binding_unverified"))?
        .generation;
    Ok(crate::models::channel_message::MetaMessageAuthority {
        grant_id: bot
            .meta_binding
            .as_ref()
            .map(|binding| binding.grant_id.clone()),
        generation,
        asset_id: asset.into(),
        sender_id: sender.into(),
        binding: binding(bot),
        event_at: timestamp,
        reply_expires_at: Utc::now() + Duration::seconds(ttl),
    })
}

/// A reconnect closes the old consent range. Operational generation changes
/// within one grant never clear an observed Instagram control loss.
pub(crate) fn control_scope(
    binding: &crate::models::channel_bot::MetaCredentialBinding,
) -> bson::Document {
    doc! {"$or":[{"meta_grant_id":&binding.grant_id},
    {"meta_grant_id":{"$exists":false},"generation":{"$gte":binding.grant_start_generation,"$lte":binding.generation}}]}
}
fn control_for_grant(
    row: &bson::Document,
    binding: &crate::models::channel_bot::MetaCredentialBinding,
) -> bool {
    match row.get("meta_grant_id") {
        Some(bson::Bson::String(grant)) => grant == &binding.grant_id,
        None => row.get_i64("generation").is_ok_and(|generation| {
            generation >= binding.grant_start_generation && generation <= binding.generation
        }),
        _ => false,
    }
}

pub(crate) async fn require_reply(
    db: &mongodb::Database,
    bot: &ChannelBot,
    original: &ChannelMessage,
    chat: &str,
    credentials: &BotCredentials<'_>,
) -> AppResult<()> {
    require_owner_live(db, &bot.user_id).await?;
    let authority = original
        .meta_authority
        .as_ref()
        .ok_or_else(|| unreachable("message_binding_required"))?;
    let now = Utc::now();
    if authority.event_at > now {
        return Err(unreachable("event_time_invalid"));
    }
    if now >= authority.event_at + Duration::hours(24) {
        return Err(unreachable("window_closed"));
    }
    if now >= authority.reply_expires_at {
        return Err(unreachable("reply_token_expired"));
    }
    let live = crate::services::channel_bot_service::get_bot(db, &bot.id).await?;
    let current = live
        .meta_binding
        .as_ref()
        .ok_or_else(|| unreachable("binding_unverified"))?;
    if current.generation != authority.generation
        || bot
            .meta_binding
            .as_ref()
            .is_none_or(|b| b.generation != current.generation)
        || current.app_id != live.app_id.as_deref().unwrap_or_default()
        || current.token_fingerprint != token_fingerprint(credentials.token)
        || current.subscription != "active"
        || current.operator_suspended
        || !current.exclusive_routing_verified
        || (bot.platform == "instagram" && current.expires_at.is_some_and(|expiry| expiry <= now))
    {
        return Err(unreachable("credential_authority_changed"));
    }
    if live.credential_source == "platform" {
        let configured = db
            .collection::<bson::Document>(crate::models::platform_credential::COLLECTION_NAME)
            .find_one(doc! {"provider":&live.platform})
            .await?;
        if configured
            .as_ref()
            .and_then(|r| r.get_document("fields").ok())
            .and_then(|f| f.get_str("app_id").ok())
            != live.app_id.as_deref()
        {
            return Err(unreachable("application_changed"));
        }
    }
    if !live.is_active || live.status != "active" || !live.webhook_registered {
        return Err(unreachable("inbox_suspended"));
    }
    if authority.binding != binding(&live)
        || authority.binding != binding(bot)
        || original.direction != "inbound"
        || original.channel_bot_id.as_deref() != Some(bot.id.as_str())
        || original.user_id != bot.user_id
        || original.platform != bot.platform
        || authority.asset_id != live.platform_bot_id
        || original.platform_conversation_id.as_deref() != Some(chat)
        || chat
            != conversation(
                &bot.platform,
                &bot.id,
                &authority.asset_id,
                &authority.sender_id,
            )
    {
        return Err(unreachable("binding_changed"));
    }
    crate::services::ownership_transfer_service::require_current_bot(db, bot).await?;
    if db
        .collection::<bson::Document>(CONTROLS)
        .find_one(doc! {"identity_key":format!("{}:{chat}", bot.id),
        "$and":[control_scope(current)], "controlled":false})
        .await?
        .is_some()
    {
        return Err(unreachable("inbox_control_lost"));
    }
    if db
        .collection::<bson::Document>(CONTROLS)
        .find_one(doc! {"identity_key":format!("platform:{}",bot.platform), "suspended":true})
        .await?
        .is_some()
    {
        return Err(unreachable("platform_suspended"));
    }
    Ok(())
}

async fn require_owner_live(db: &mongodb::Database, owner: &str) -> AppResult<()> {
    if db
        .collection::<bson::Document>(crate::models::user::COLLECTION_NAME)
        .find_one(
            doc! {"_id": owner, "is_active": true, "platform_personal_deleting": {"$ne": true}},
        )
        .await?
        .is_none()
    {
        return Err(unreachable("owner_authority_changed"));
    }
    Ok(())
}

pub(crate) fn token_fingerprint(token: &str) -> String {
    hex::encode(Sha256::digest(token.as_bytes()))
}

pub(crate) async fn require_ingress(
    db: &mongodb::Database,
    bot: &ChannelBot,
    chat: &str,
) -> AppResult<()> {
    require_owner_live(db, &bot.user_id).await?;
    let live = crate::services::channel_bot_service::get_bot(db, &bot.id).await?;
    if binding(&live) != binding(bot)
        || live.meta_binding.as_ref().map(|b| b.generation)
            != bot.meta_binding.as_ref().map(|b| b.generation)
    {
        return Err(unreachable("credential_authority_changed"));
    }
    crate::services::ownership_transfer_service::require_current_bot(db, bot).await?;
    if live.credential_source == "platform" {
        let configured = db
            .collection::<bson::Document>(crate::models::platform_credential::COLLECTION_NAME)
            .find_one(doc! {"provider":&live.platform})
            .await?;
        if configured
            .as_ref()
            .and_then(|r| r.get_document("fields").ok())
            .and_then(|f| f.get_str("app_id").ok())
            != live.app_id.as_deref()
        {
            return Err(unreachable("application_changed"));
        }
    }

    if !live.is_active
        || live.status != "active"
        || !live.webhook_registered
        || live.meta_binding.as_ref().is_none_or(|b| {
            !b.exclusive_routing_verified
                || b.subscription != "active"
                || b.operator_suspended
                || (bot.platform == "instagram"
                    && b.expires_at.is_some_and(|expiry| expiry <= Utc::now()))
        })
    {
        return Err(unreachable("inbox_suspended"));
    }
    let current = live
        .meta_binding
        .as_ref()
        .ok_or_else(|| unreachable("binding_unverified"))?;
    if db.collection::<bson::Document>(CONTROLS)
        .find_one(doc! {"identity_key":format!("{}:{chat}",bot.id),"controlled":false,"$and":[control_scope(current)]})
        .await?.is_some()
        || db.collection::<bson::Document>(CONTROLS)
            .find_one(doc! {"identity_key":format!("platform:{}",bot.platform),"suspended":true})
            .await?.is_some() {
        return Err(unreachable("inbox_control_lost"));
    }
    Ok(())
}

const APP_ID: RegistrationField = RegistrationField {
    name: "app_id",
    label: "Meta app ID",
    storage: "app_id",
    secret: false,
    required: true,
    patchable: false,
    clearable: false,
    webhook_secret: false,
    platform_fallback: Some("app_id"),
    hint: Some("The dedicated inbox app; changing applications requires reconnecting."),
};
const APP_SECRET: RegistrationField = RegistrationField {
    name: "app_secret",
    label: "Webhook signing secret",
    storage: "app_secret_encrypted",
    secret: true,
    required: true,
    patchable: true,
    clearable: false,
    webhook_secret: true,
    platform_fallback: Some("webhook_signing_secret"),
    hint: Some("The app secret that signs this inbox's webhooks."),
};

fn manual_signer_pin(bot: &ChannelBot) -> AppResult<Option<String>> {
    use super::super::channel_credential_continuity::{APP, field_pin};
    bot.app_secret_encrypted
        .as_ref()
        .map(|_| field_pin(bot, APP))
        .transpose()
}

async fn apply_webhook_state_inner(
    inbox: Inbox,
    db: &mongodb::Database,
    bot: &ChannelBot,
    value: Value,
    platform_signer: Option<&bson::Document>,
    tx: &mut mongodb::ClientSession,
) -> AppResult<()> {
    if let Some(signer) = platform_signer {
        let matched = db
            .collection::<bson::Document>(crate::models::platform_credential::COLLECTION_NAME)
            .update_one(
                signer.clone(),
                doc! {"$inc": {"channel_observation_version": 1_i64}},
            )
            .session(&mut *tx)
            .await?
            .matched_count;
        if matched == 0 {
            return Err(unreachable("binding_changed"));
        }
    }
    let live = db
        .collection::<ChannelBot>(crate::models::channel_bot::COLLECTION_NAME)
        .find_one_and_update(
            doc! {"_id":&bot.id,"user_id":&bot.user_id,"is_active":true},
            doc! {"$inc": {"meta_control_revision": 1_i64}},
        )
        .return_document(mongodb::options::ReturnDocument::After)
        .session(&mut *tx)
        .await?;
    let Some(live) = live else {
        return Err(unreachable("binding_changed"));
    };
    let observed = bot
        .meta_binding
        .as_ref()
        .ok_or_else(|| unreachable("binding_changed"))?;
    let current = live
        .meta_binding
        .as_ref()
        .ok_or_else(|| unreachable("binding_changed"))?;
    // Suspension and resume advance delivery generation without replacing the
    // authority that signed a participant-control observation.
    if live.ownership_version != bot.ownership_version
        || live.platform != bot.platform
        || live.platform_bot_id != bot.platform_bot_id
        || live.app_id != bot.app_id
        || live.credential_source != bot.credential_source
        || manual_signer_pin(&live)? != manual_signer_pin(bot)?
        || current.grant_id != observed.grant_id
        || current.app_id != observed.app_id
        || current.events_since.map(|at| at.timestamp_millis())
            != observed.events_since.map(|at| at.timestamp_millis())
        || current.authorized_at.map(|at| at.timestamp_millis())
            != observed.authorized_at.map(|at| at.timestamp_millis())
    {
        return Err(unreachable("binding_changed"));
    }
    crate::services::channel_meta_service::scopes::fence_current(db, tx, &live).await?;
    for entry in value["entry"].as_array().ok_or_else(signature_error)? {
        if entry["id"].as_str() != Some(bot.platform_bot_id.as_str()) {
            continue;
        }
        // A checkbox or new DM never clears Instagram loss. Messenger
        // gain events bind an exact recipient, app and current grant time;
        // provider ownership is independently re-read before every send.
        let events = entry["messaging"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|e| (e, false))
            .chain(
                entry["standby"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|e| (e, true)),
            );
        for (event, standby) in events {
            if event["recipient"]["id"].as_str() != Some(bot.platform_bot_id.as_str()) {
                continue;
            }
            let control = event.get("pass_thread_control").is_some()
                || event.get("take_thread_control").is_some()
                || standby;
            if !control {
                continue;
            }
            let Some(sender) = event["sender"]["id"].as_str().filter(|s| valid_id(s)) else {
                continue;
            };
            let Some(binding) = live.meta_binding.as_ref() else {
                continue;
            };
            let Some(provider_event_at) = event["timestamp"]
                .as_i64()
                .and_then(DateTime::from_timestamp_millis)
                .filter(|at| {
                    *at <= Utc::now()
                        && binding
                            .events_since
                            .or(binding.authorized_at)
                            .is_some_and(|start| *at >= start)
                })
            else {
                continue;
            };
            let controlled = matches!(inbox, Inbox::Messenger)
                && !standby
                && event["pass_thread_control"].is_object()
                && identity_value(&event["pass_thread_control"]["new_owner_app_id"]).as_deref()
                    == Some(binding.app_id.as_str());
            let chat = conversation(&bot.platform, &bot.id, &bot.platform_bot_id, sender);
            let key = format!("{}:{chat}", bot.id);
            let current = db
                .collection::<bson::Document>(CONTROLS)
                .find_one(doc! {"identity_key": &key})
                .session(&mut *tx)
                .await?;
            if let Some(row) = &current
                && control_for_grant(row, binding)
            {
                let event_at = provider_event_at;
                let previous = row
                    .get_datetime("provider_event_at")
                    .ok()
                    .map(|value| value.to_chrono());
                if previous.is_some_and(|previous| previous > event_at)
                    || (previous == Some(event_at)
                        && row.get_bool("controlled").ok() == Some(false)
                        && controlled)
                {
                    continue;
                }
            }
            let revision = current
                .as_ref()
                .and_then(|row| row.get_i64("revision").ok())
                .unwrap_or(0);
            let mut filter = doc! {
                "identity_key": &key,
                "bot_id": &bot.id,
                    "generation": current
                        .as_ref()
                        .and_then(|row| row.get_i64("generation").ok())
                        .unwrap_or(binding.generation),
            };
            if current.is_some() {
                if revision == 0
                    && current
                        .as_ref()
                        .is_some_and(|row| !row.contains_key("revision"))
                {
                    filter.insert(
                        "$or",
                        bson::bson!([{ "revision": 0_i64 }, { "revision": { "$exists": false } }]),
                    );
                } else {
                    filter.insert("revision", revision);
                }
            } else {
                filter.insert("revision", 0_i64);
            }
            let changed = db
                .collection::<bson::Document>(CONTROLS)
                .update_one(
                    filter,
                    doc! {"$set":{"bot_id":&bot.id,
                    "meta_grant_id":&binding.grant_id,
                    "controlled":controlled,
                    "generation":binding.generation,
                    "revision":revision+1_i64,
                    "at":bson::DateTime::now(),
                    "provider_event_at": bson::DateTime::from_chrono(provider_event_at)},
                    "$setOnInsert":{"_id":uuid::Uuid::new_v4().to_string()}},
                )
                .upsert(true)
                .session(&mut *tx)
                .await?;
            if changed.matched_count != 1 && changed.upserted_id.is_none() {
                return Err(unreachable("inbox_control_lost"));
            }
        }
    }
    Ok(())
}

#[async_trait::async_trait]
impl PlatformAdapter for MetaAdapter {
    async fn authorize_inbound(
        &self,
        db: &mongodb::Database,
        http: &reqwest::Client,
        keys: &crate::crypto::aes::EncryptionKeys,
        bot: &ChannelBot,
        inbound: &InboundMessage,
    ) -> AppResult<()> {
        let event_at = inbound.raw_data["timestamp"]
            .as_i64()
            .and_then(DateTime::from_timestamp_millis)
            .ok_or_else(|| unreachable("event_time_missing"))?;
        if bot
            .meta_binding
            .as_ref()
            .and_then(|b| b.events_since.or(b.authorized_at))
            .is_none_or(|at| event_at < at)
        {
            return Err(unreachable("credential_authority_changed"));
        }
        let sender = inbound.raw_data["sender"]["id"]
            .as_str()
            .ok_or_else(|| unreachable("sender_mismatch"))?;
        let token =
            crate::services::channel_credentials::resolve_bot_token(db, keys, self, bot).await?;
        self.prove_control(db, http, bot, sender, &token).await
    }
    fn platform_id(&self) -> &str {
        self.inbox.id()
    }
    fn display_name(&self) -> &str {
        match self.inbox {
            Inbox::Messenger => "Messenger",
            Inbox::Instagram => "Instagram",
        }
    }
    fn outbound_capabilities(&self) -> OutboundCapabilities {
        OutboundCapabilities {
            initiated_send: false,
            reply_to: false,
            thread: false,
            edit: false,
        }
    }
    fn media_capabilities(&self) -> MediaCapabilities {
        MediaCapabilities::NONE
    }
    fn atomic_inbound_admission(&self) -> bool {
        true
    }
    fn dedup_inbound_by_platform_message_id(&self) -> bool {
        true
    }
    fn serializes_lifecycle(&self) -> bool {
        true
    }
    fn registration(&self) -> RegistrationDescriptor {
        RegistrationDescriptor {
            documentation_url: Some(match self.inbox {
                Inbox::Messenger => {
                    "https://developers.facebook.com/documentation/business-messaging/messenger-platform/get-started"
                }
                Inbox::Instagram => {
                    "https://developers.facebook.com/documentation/instagram-platform/instagram-api-with-instagram-login/business-login"
                }
            }),
            fields: &[
                RegistrationField {
                    label: "Selected inbox access token",
                    patchable: true,
                    ..BOT_TOKEN_FIELD
                },
                APP_ID,
                APP_SECRET,
                RegistrationField {
                    name: "verification_token",
                    label: "Exclusive routing confirmation",
                    storage: "",
                    secret: false,
                    required: true,
                    patchable: true,
                    clearable: false,
                    webhook_secret: false,
                    platform_fallback: None,
                    hint: Some(
                        "Type exclusive after verifying this app exclusively receives inbox conversations. Messenger also checks live thread ownership for each participant.",
                    ),
                },
            ],
            extra_fields: &[],
            token_fields: &["bot_token"],
            create_response_status: "pending_webhook",
            preserve_subscription_on_verify: true,
            webhook_secret_label: Some("Verify Token"),
            unsupported_patch_message: None,
            setup_instructions: &[
                "Use an isolated app-role test inbox for manual setup. Configure the generated callback and Verify Token, then subscribe to messages and messaging handovers.",
                "Verify exclusive conversation routing to this app before enabling delivery. Other inbox apps and human handover are not supported for automatic resumption.",
                "Messenger: configure this app as the Page default and organic/social routing app. NyxID requires own-app proof from thread_owner?recipient=PSID for the exact conversation.",
                "Only customer-initiated private text DMs can receive a bound reply within 24 hours. Enable Everyone explicitly in the assigned specialist's private-chat settings to serve customers.",
            ],
            ..Default::default()
        }
    }
    fn platform_credentials(
        &self,
    ) -> Option<crate::services::channel_managed::PlatformCredentialDescriptor> {
        Some(super::meta_managed::descriptor(self.inbox))
    }
    fn registration_token(
        &self,
        fields: &RegistrationValues<'_>,
    ) -> AppResult<zeroize::Zeroizing<String>> {
        if fields.get("verification_token") != Some("exclusive") {
            return Err(AppError::ValidationError(
                "Verify exclusive inbox routing and provide its confirmation".into(),
            ));
        }
        Ok(zeroize::Zeroizing::new(
            fields.get("bot_token").unwrap_or_default().into(),
        ))
    }
    fn managed_onboarding(
        &self,
    ) -> Option<crate::services::channel_managed::ManagedOnboardingDescriptor> {
        Some(super::meta_managed::onboarding(self.inbox))
    }
    fn platform_webhook(&self) -> bool {
        true
    }
    fn webhook_policy(&self, _: &[u8]) -> WebhookPolicy {
        WebhookPolicy::Immediate(None)
    }
    fn validate_platform_subscription(
        &self,
        query: &std::collections::HashMap<String, String>,
    ) -> AppResult<()> {
        validate_handshake(query)
    }
    fn platform_subscription_handshake(
        &self,
        secrets: &PlatformVerifySecrets,
        query: &std::collections::HashMap<String, String>,
    ) -> AppResult<String> {
        validate_handshake(query)?;
        let token = secrets.get("verify_token").ok_or_else(signature_error)?;
        if !bool::from(token.as_bytes().ct_eq(query["hub.verify_token"].as_bytes())) {
            return Err(signature_error());
        }
        Ok(query["hub.challenge"].clone())
    }
    fn subscription_handshake(
        &self,
        bot: &ChannelBot,
        query: &std::collections::HashMap<String, String>,
    ) -> AppResult<String> {
        validate_handshake(query)?;
        let hash = hex::encode(Sha256::digest(query["hub.verify_token"].as_bytes()));
        if !bool::from(hash.as_bytes().ct_eq(bot.webhook_secret_hash.as_bytes())) {
            return Err(signature_error());
        }
        Ok(query["hub.challenge"].clone())
    }
    async fn platform_webhook_targets(
        &self,
        secrets: &PlatformVerifySecrets,
        headers: &HeaderMap,
        body: &[u8],
    ) -> AppResult<Vec<String>> {
        verify_signature(
            secrets
                .get("webhook_signing_secret")
                .ok_or_else(signature_error)?,
            headers,
            body,
        )?;
        let value: Value = serde_json::from_slice(body).map_err(|_| signature_error())?;
        if value["object"].as_str() != Some(self.inbox.object()) {
            return Err(signature_error());
        }
        let mut targets = std::collections::BTreeSet::new();
        for entry in value["entry"].as_array().ok_or_else(signature_error)? {
            if let Some(id) = entry["id"].as_str().filter(|s| valid_id(s)) {
                targets.insert(id.to_string());
            }
        }
        if targets.len() > 128 {
            return Err(signature_error());
        }
        Ok(targets.into_iter().collect())
    }
    async fn verify_webhook(
        &self,
        _: &ChannelBot,
        secrets: Option<&PlatformVerifySecrets>,
        headers: &HeaderMap,
        body: &[u8],
    ) -> AppResult<()> {
        let secrets = secrets.ok_or_else(signature_error)?;
        verify_signature(
            secrets
                .get("app_secret")
                .or_else(|| secrets.get("webhook_signing_secret"))
                .ok_or_else(signature_error)?,
            headers,
            body,
        )
    }
    async fn prepare_webhook(
        &self,
        bot: &ChannelBot,
        secrets: Option<&PlatformVerifySecrets>,
        headers: &HeaderMap,
        body: &[u8],
    ) -> AppResult<PreparedWebhook> {
        self.verify_webhook(bot, secrets, headers, body).await?;
        if bot.credential_source == "platform"
            && secrets.and_then(|s| s.get("app_id")) != bot.app_id.as_deref()
        {
            return Err(signature_error());
        }
        let mut value: Value = serde_json::from_slice(body).map_err(|_| signature_error())?;
        if value["object"].as_str() != Some(self.inbox.object()) {
            return Err(signature_error());
        }
        let entries = value["entry"].as_array_mut().ok_or_else(signature_error)?;
        entries.retain(|e| e["id"].as_str() == Some(bot.platform_bot_id.as_str()));
        let activate_bot = bot
            .meta_binding
            .as_ref()
            .is_some_and(|binding| binding.subscription == "active")
            && entries.iter().any(|e| {
                e["messaging"].as_array().is_some_and(|events| {
                    events.iter().any(|event| {
                        parse_event(self.inbox, &bot.id, &bot.platform_bot_id, event).is_some()
                    })
                })
            });
        value["_nyxid_bot_id"] = json!(&bot.id);
        Ok(PreparedWebhook {
            body: serde_json::to_vec(&value).map_err(|_| signature_error())?,
            challenge_response: None,
            activate_bot,
        })
    }
    async fn apply_webhook_state(
        &self,
        db: &mongodb::Database,
        bot: &ChannelBot,
        body: &[u8],
        snapshot: &crate::services::channel_verification_service::CredentialSnapshot,
    ) -> AppResult<()> {
        let value: Value = serde_json::from_slice(body).map_err(|_| signature_error())?;
        let has_control = value["entry"]
            .as_array()
            .ok_or_else(signature_error)?
            .iter()
            .any(|entry| {
                entry["standby"]
                    .as_array()
                    .is_some_and(|events| !events.is_empty())
                    || entry["messaging"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .any(|event| {
                            event.get("pass_thread_control").is_some()
                                || event.get("take_thread_control").is_some()
                        })
            });
        if !has_control {
            return Ok(());
        }
        let args = (
            db.clone(),
            bot.clone(),
            value,
            self.inbox,
            snapshot.meta_webhook_signer_filter()?,
        );
        crate::services::transaction_service::run_db_only(
            db,
            &args,
            |tx, (db, bot, value, inbox, signer)| {
                Box::pin(async move {
                    apply_webhook_state_inner(*inbox, db, bot, value.clone(), signer.as_ref(), tx)
                        .await
                })
            },
        )
        .await
    }
    async fn parse_inbound(&self, body: &[u8]) -> AppResult<Vec<InboundMessage>> {
        let value: Value = serde_json::from_slice(body).map_err(|_| signature_error())?;
        if value["object"].as_str() != Some(self.inbox.object()) {
            return Err(signature_error());
        }
        let bot_id = value["_nyxid_bot_id"]
            .as_str()
            .filter(|id| uuid::Uuid::parse_str(id).is_ok())
            .ok_or_else(signature_error)?;
        let mut messages = Vec::new();
        for entry in value["entry"].as_array().ok_or_else(signature_error)? {
            let Some(asset) = entry["id"].as_str().filter(|s| valid_id(s)) else {
                continue;
            };
            for event in entry["messaging"].as_array().into_iter().flatten() {
                if let Some(message) = parse_event(self.inbox, bot_id, asset, event) {
                    messages.push(message);
                }
            }
        }
        Ok(messages)
    }
    async fn send_reply(
        &self,
        _: &reqwest::Client,
        _: &BotCredentials<'_>,
        _: &str,
        _: &OutboundReply,
    ) -> AppResult<Option<String>> {
        Err(unreachable("message_binding_required"))
    }
    async fn send_bound_reply_outcome(
        &self,
        db: &mongodb::Database,
        http: &reqwest::Client,
        bot: &ChannelBot,
        original: &ChannelMessage,
        credentials: &BotCredentials<'_>,
        chat: &str,
        reply: &OutboundReply,
        thread: Option<&crate::services::channel_thread_service::ThreadReplyTarget>,
    ) -> AppResult<SendOutcome> {
        if thread.is_some() {
            return Err(unreachable("text_dm_required"));
        }
        require_reply(db, bot, original, chat, credentials).await?;
        if !reply.attachments.is_empty()
            || reply
                .metadata
                .as_ref()
                .is_some_and(|v| v.as_object().is_none_or(|o| !o.is_empty()))
        {
            return Err(AppError::ChannelMediaUnsupported);
        }
        let text = reply
            .text
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .ok_or_else(|| AppError::ValidationError("Meta inbox reply requires text".into()))?;
        let limit = crate::services::channel_text::TextLimit::for_platform(&bot.platform);
        if limit.measure(text) > limit.budget() {
            return Err(AppError::ValidationError(
                "Use the complete reply planner for multipart text".into(),
            ));
        }
        let authority = original
            .meta_authority
            .as_ref()
            .ok_or_else(|| unreachable("message_binding_required"))?;
        let mut body = json!({"recipient":{"id":authority.sender_id}, "message":{"text":text}});
        if matches!(self.inbox, Inbox::Messenger) {
            body["messaging_type"] = json!("RESPONSE");
        }
        self.prove_control(db, http, bot, &authority.sender_id, credentials.token)
            .await?;
        // The second check follows all request construction and precedes I/O.
        require_reply(db, bot, original, chat, credentials).await?;
        crate::services::channel_platform::mark_provider_attempt().await?;
        let outcome = graph_send(
            http.post(self.url(&format!("{}/messages", authority.asset_id)))
                .bearer_auth(credentials.token)
                .json(&body),
        )
        .await;
        let (id, uncertain, error) = match outcome {
            GraphSend::Accepted(id) => (Some(id), false, None),
            GraphSend::Refused(error) => (None, false, Some(error)),
            GraphSend::Uncertain(error) => (None, true, Some(error)),
        };
        Ok(SendOutcome {
            final_id: id.clone(),
            delivered_kinds: None,
            observation: Some(crate::models::channel_delivery::PlatformSendRecord {
                phone_number_id: authority.asset_id.clone(),
                waba_id: bot.app_id.clone(),
                recipient_id: authority.sender_id.clone(),
                component_ids: id.into_iter().collect(),
                expected_components: 1,
                complete: error.is_none(),
                uncertain,
                failure_code: error.as_ref().map(AppError::error_code),
            }),
            error,
        })
    }
    async fn send_bound_reply(
        &self,
        db: &mongodb::Database,
        http: &reqwest::Client,
        bot: &ChannelBot,
        original: &ChannelMessage,
        credentials: &BotCredentials<'_>,
        chat: &str,
        reply: &OutboundReply,
    ) -> AppResult<Option<String>> {
        self.send_bound_reply_outcome(db, http, bot, original, credentials, chat, reply, None)
            .await?
            .into_result()
    }
    async fn register_webhook(
        &self,
        _: &reqwest::Client,
        _: &str,
        _: &str,
        _: &str,
    ) -> AppResult<()> {
        Ok(())
    }
    async fn verify_bot_token(
        &self,
        http: &reqwest::Client,
        credentials: &BotCredentials<'_>,
    ) -> AppResult<BotIdentity> {
        let fields = match self.inbox {
            Inbox::Messenger => "id,name,category",
            Inbox::Instagram => "user_id,username,account_type",
        };
        let value = graph_json(
            http.get(self.url("me"))
                .bearer_auth(credentials.token)
                .query(&[("fields", fields)]),
        )
        .await?;
        let value = match self.inbox {
            Inbox::Messenger => &value,
            Inbox::Instagram => instagram_record(&value)?,
        };
        let id = match self.inbox {
            Inbox::Messenger => identity_value(&value["id"]),
            Inbox::Instagram => identity_value(&value["user_id"]),
        }
        .ok_or_else(|| {
            AppError::ValidationError("Token did not identify the selected Meta inbox".into())
        })?;
        if matches!(self.inbox, Inbox::Messenger)
            && value["category"]
                .as_str()
                .is_none_or(|category| category.is_empty())
        {
            return Err(AppError::ValidationError(
                "Messenger requires a Page access token".into(),
            ));
        }
        if credentials
            .platform_bot_id
            .is_some_and(|expected| expected != id)
        {
            return Err(unreachable("asset_mismatch"));
        }
        if matches!(self.inbox, Inbox::Instagram)
            && !value["account_type"].as_str().is_some_and(|kind| {
                matches!(
                    kind.to_ascii_uppercase().as_str(),
                    "BUSINESS" | "MEDIA_CREATOR"
                )
            })
        {
            return Err(AppError::ValidationError(
                "Instagram requires a Business or Creator account".into(),
            ));
        }
        let name = value["name"]
            .as_str()
            .or_else(|| value["username"].as_str())
            .unwrap_or(&id)
            .to_string();
        Ok(BotIdentity {
            platform_bot_id: id,
            platform_bot_username: name.clone(),
            display_name: Some(name),
        })
    }
}

#[cfg(test)]
#[path = "meta_tests.rs"]
pub(crate) mod tests;

#[cfg(test)]
tokio::task_local! {static TEST_GRAPH_BASE:String;}
#[cfg(test)]
pub(crate) async fn with_test_graph_base<F: std::future::Future>(
    base: String,
    future: F,
) -> F::Output {
    TEST_GRAPH_BASE.scope(base, future).await
}

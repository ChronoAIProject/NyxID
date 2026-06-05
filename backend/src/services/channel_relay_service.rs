//! Channel relay service: message storage, agent callback delivery, and reply
//! handling.
//!
//! Responsible for persisting inbound/outbound messages, forwarding inbound
//! messages to agent callback URLs with RS256 callback JWTs plus transitional
//! HMAC signatures, and recording callback delivery status.

use chrono::Utc;
use futures::TryStreamExt;
use hmac::{Hmac, Mac};
use mongodb::bson::{Bson, doc};
use serde::Serialize;
use sha2::{Digest, Sha256};

use crate::config::AppConfig;
use crate::errors::{AppError, AppResult};
use crate::models::channel_conversation::COLLECTION_NAME as CONVERSATIONS;
use crate::models::channel_message::{COLLECTION_NAME, ChannelMessage};
use crate::services::channel_platform::InboundMessage;

type HmacSha256 = Hmac<Sha256>;

// ---------------------------------------------------------------------------
// Callback payload types (sent to agent)
// ---------------------------------------------------------------------------

/// Normalized message payload delivered to the agent's callback URL.
#[derive(Debug, Clone, Serialize)]
pub struct CallbackPayload {
    pub message_id: String,
    pub correlation_id: String,
    pub platform: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_token: Option<String>,
    pub agent: CallbackAgent,
    pub conversation: CallbackConversation,
    pub sender: CallbackSender,
    pub content: CallbackContent,
    /// NyxID internal message ID of the message being replied to (if known).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_to_message_id: Option<String>,
    /// Platform-native message ID of the message being replied to.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_to_platform_message_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    pub timestamp: String,
    /// Raw platform-specific webhook payload. Agents that need access to
    /// platform features (Telegram inline keyboards, Discord embeds, Lark
    /// cards, etc.) can read this instead of the normalized fields above.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub raw_platform_data: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CallbackAgent {
    pub api_key_id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CallbackConversation {
    pub id: String,
    pub platform_id: String,
    #[serde(rename = "type")]
    pub conversation_type: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CallbackSender {
    pub platform_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CallbackContent {
    #[serde(rename = "type")]
    pub content_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub attachments: Vec<CallbackAttachment>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CallbackAttachment {
    pub content_type: String,
    pub url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<u64>,
}

// ---------------------------------------------------------------------------
// Message storage
// ---------------------------------------------------------------------------

/// Persist inbound-message metadata (platform -> agent direction).
///
/// Per ADR-013, this record stores routing metadata only — not the message
/// body, attachments, or raw webhook payload. The full message content is
/// held in memory for the duration of the callback forward and then
/// discarded. Downstream agents keep any history they need.
#[allow(clippy::too_many_arguments)]
pub async fn store_inbound_message(
    db: &mongodb::Database,
    channel_bot_id: &str,
    conversation_id: &str,
    user_id: &str,
    platform: &str,
    inbound: &InboundMessage,
    agent_api_key_id: &str,
) -> AppResult<ChannelMessage> {
    let message = ChannelMessage {
        id: uuid::Uuid::new_v4().to_string(),
        channel_bot_id: Some(channel_bot_id.to_string()),
        conversation_id: conversation_id.to_string(),
        platform_conversation_id: Some(inbound.conversation_id.clone()),
        user_id: user_id.to_string(),
        direction: "inbound".to_string(),
        platform: platform.to_string(),
        platform_message_id: Some(inbound.platform_message_id.clone()),
        sender_platform_id: Some(inbound.sender_platform_id.clone()),
        sender_display_name: inbound.sender_display_name.clone(),
        content_type: inbound.content_type.clone(),
        thread_id: inbound.thread_id.clone(),
        agent_api_key_id: Some(agent_api_key_id.to_string()),
        callback_status: Some("pending".to_string()),
        reply_to_message_id: None,
        platform_reply_message_id: None,
        created_at: Utc::now(),
        updated_at: None,
    };

    db.collection::<ChannelMessage>(COLLECTION_NAME)
        .insert_one(&message)
        .await?;

    Ok(message)
}

/// Persist outbound-message metadata (agent -> platform direction).
///
/// Per ADR-013, the reply *text* is not stored here. The caller passes the
/// text straight through to the platform adapter and only routing metadata
/// survives in MongoDB.
#[allow(clippy::too_many_arguments)]
pub async fn store_outbound_message(
    db: &mongodb::Database,
    channel_bot_id: &str,
    conversation_id: &str,
    user_id: &str,
    platform: &str,
    agent_api_key_id: &str,
    reply_to_message_id: Option<&str>,
    platform_message_id: Option<&str>,
) -> AppResult<ChannelMessage> {
    let now = Utc::now();
    let message = ChannelMessage {
        id: uuid::Uuid::new_v4().to_string(),
        channel_bot_id: Some(channel_bot_id.to_string()),
        conversation_id: conversation_id.to_string(),
        platform_conversation_id: None,
        user_id: user_id.to_string(),
        direction: "outbound".to_string(),
        platform: platform.to_string(),
        platform_message_id: platform_message_id.map(String::from),
        sender_platform_id: None,
        sender_display_name: None,
        content_type: "text".to_string(),
        thread_id: None,
        agent_api_key_id: Some(agent_api_key_id.to_string()),
        callback_status: None,
        reply_to_message_id: reply_to_message_id.map(String::from),
        platform_reply_message_id: None,
        created_at: now,
        updated_at: Some(now),
    };

    db.collection::<ChannelMessage>(COLLECTION_NAME)
        .insert_one(&message)
        .await?;

    Ok(message)
}

/// Persist a metadata-only `ChannelMessage` record for an HTTP Event Gateway
/// event (NyxID#221). The record is what the downstream agent's reply looks
/// up through `POST /api/v1/channel-relay/reply` — without it, async replies
/// to device events would 404.
///
/// Per ADR-013 the row stores only routing metadata: no payload, no source
/// envelope body. The caller passes the original client-supplied event id
/// separately as `platform_message_id` so operators can correlate events by
/// their upstream ID without needing the envelope content.
///
/// `inherited_thread_id` carries forward a routing-metadata thread id from
/// the conversation's most recent webhook-driven inbound message. This
/// covers:
///
/// - Discord deferred-interaction follow-up tokens
///   (`interaction:{app}:{token}`) so `async_reply()` can still route
///   replies through Discord's interaction webhook.
/// - Telegram forum-topic ids so replies stay scoped to the originating
///   topic instead of landing in the root chat.
///
/// Pass `None` when there is no thread context to carry forward.
#[allow(clippy::too_many_arguments)]
pub async fn store_device_event_message(
    db: &mongodb::Database,
    channel_bot_id: Option<&str>,
    conversation_id: &str,
    platform_conversation_id: &str,
    user_id: &str,
    client_event_id: &str,
    source: &str,
    event_type: &str,
    agent_api_key_id: &str,
    inherited_thread_id: Option<String>,
) -> AppResult<ChannelMessage> {
    let message = ChannelMessage {
        id: uuid::Uuid::new_v4().to_string(),
        channel_bot_id: channel_bot_id.map(String::from),
        conversation_id: conversation_id.to_string(),
        platform_conversation_id: Some(platform_conversation_id.to_string()),
        user_id: user_id.to_string(),
        direction: "inbound".to_string(),
        platform: "device".to_string(),
        // The client-supplied envelope event_id survives here purely as a
        // correlation handle — it is NOT used as the NyxID message_id.
        platform_message_id: Some(client_event_id.to_string()),
        sender_platform_id: Some(source.to_string()),
        sender_display_name: None,
        content_type: event_type.to_string(),
        thread_id: inherited_thread_id,
        agent_api_key_id: Some(agent_api_key_id.to_string()),
        callback_status: Some("pending".to_string()),
        reply_to_message_id: None,
        platform_reply_message_id: None,
        created_at: Utc::now(),
        updated_at: None,
    };

    db.collection::<ChannelMessage>(COLLECTION_NAME)
        .insert_one(&message)
        .await?;

    Ok(message)
}

// ---------------------------------------------------------------------------
// Callback delivery
// ---------------------------------------------------------------------------

/// Outcome of a single callback delivery attempt.
///
/// `http_status` carries the actual HTTP status returned by the agent when a
/// response was received, or `None` when the request never reached the agent
/// (transport error). Callers record this into audit logs so operators can
/// distinguish between a tolerated legacy 200 reply, a clean 202, and a
/// downstream 500/404.
#[derive(Debug)]
pub struct CallbackDelivery {
    pub http_status: Option<u16>,
    pub result: AppResult<()>,
}

/// Forward an inbound message to the agent's callback URL.
///
/// Emits an RS256 callback JWT in `X-NyxID-Callback-Token` whose
/// `body_sha256` claim covers the exact wire body bytes. It also keeps the
/// legacy `X-NyxID-Signature` HMAC header during the dual-emit transition.
/// **Only HTTP 202 is a success.** HTTP 200 is tolerated for legacy clients
/// but the response body is ignored (sync reply mode was deprecated per
/// ADR-013 and NyxID#221 comment 2 — agent replies must flow through
/// `POST /api/v1/channel-relay/reply`).
///
/// Returns a [`CallbackDelivery`] so callers can observe the actual upstream
/// status code in addition to the success/error result.
#[allow(clippy::too_many_arguments)]
pub async fn forward_to_agent(
    http_client: &reqwest::Client,
    config: &AppConfig,
    keys: &crate::crypto::jwt::JwtKeys,
    callback_url: &str,
    mut payload: CallbackPayload,
    api_key_id: &str,
    api_key_hash: &str,
    user_access_token: Option<&str>,
) -> CallbackDelivery {
    let jti = uuid::Uuid::new_v4().to_string();
    payload.correlation_id = jti.clone();

    let body_bytes = match serde_json::to_vec(&payload) {
        Ok(b) => b,
        Err(e) => {
            return CallbackDelivery {
                http_status: None,
                result: Err(AppError::Internal(format!(
                    "failed to serialize callback payload: {e}"
                ))),
            };
        }
    };

    let body_sha256 = hex::encode(Sha256::digest(&body_bytes));
    let callback_token = match crate::crypto::jwt::generate_relay_callback_token(
        keys,
        config,
        &jti,
        api_key_id,
        &payload.message_id,
        &payload.platform,
        &body_sha256,
    ) {
        Ok(token) => token,
        Err(e) => {
            return CallbackDelivery {
                http_status: None,
                result: Err(e),
            };
        }
    };

    let signature = match compute_hmac_signature(api_key_hash.as_bytes(), &body_bytes) {
        Ok(s) => s,
        Err(e) => {
            return CallbackDelivery {
                http_status: None,
                result: Err(e),
            };
        }
    };
    let timestamp = Utc::now().to_rfc3339();

    let timeout =
        std::time::Duration::from_secs(u64::from(config.channel_relay_callback_timeout_secs));

    let mut request = http_client
        .post(callback_url)
        .header("Content-Type", "application/json")
        .header("X-NyxID-Callback-Token", &callback_token)
        .header("X-NyxID-Signature", &signature)
        .header("X-NyxID-Message-Id", &payload.message_id)
        .header("X-NyxID-Timestamp", &timestamp)
        .header("X-NyxID-Platform", &payload.platform)
        .timeout(timeout);

    // Include the bot owner's access token so the receiving agent can make
    // NyxID API calls (proxy, approvals, etc.) on behalf of the user.
    if let Some(token) = user_access_token {
        request = request.header("X-NyxID-User-Token", token);
    }

    let response = match request.body(body_bytes).send().await {
        Ok(r) => r,
        Err(e) => {
            return CallbackDelivery {
                http_status: None,
                result: Err(AppError::ChannelRelayFailed(format!(
                    "callback request failed: {e}"
                ))),
            };
        }
    };

    let status = response.status();
    let code = status.as_u16();

    match code {
        202 => CallbackDelivery {
            http_status: Some(202),
            result: Ok(()),
        },
        200 => {
            tracing::warn!(
                callback_url = %callback_url,
                message_id = %payload.message_id,
                "Agent returned 200 with body; sync replies are no longer supported. \
                 Body discarded. Use POST /api/v1/channel-relay/reply for async replies."
            );
            CallbackDelivery {
                http_status: Some(200),
                result: Ok(()),
            }
        }
        _ => CallbackDelivery {
            http_status: Some(code),
            result: Err(AppError::ChannelRelayFailed(format!(
                "callback returned HTTP {status}"
            ))),
        },
    }
}

/// Update the callback delivery status on a stored message.
pub async fn update_callback_status(
    db: &mongodb::Database,
    message_id: &str,
    status: &str,
) -> AppResult<()> {
    db.collection::<ChannelMessage>(COLLECTION_NAME)
        .update_one(
            doc! { "_id": message_id },
            doc! { "$set": { "callback_status": status }},
        )
        .await?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Message queries
// ---------------------------------------------------------------------------

/// Get a single message by ID.
pub async fn get_message(db: &mongodb::Database, message_id: &str) -> AppResult<ChannelMessage> {
    db.collection::<ChannelMessage>(COLLECTION_NAME)
        .find_one(doc! { "_id": message_id })
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Message not found: {message_id}")))
}

/// Get a single outbound message by its upstream platform message ID.
pub async fn get_outbound_message_by_platform_id(
    db: &mongodb::Database,
    platform: &str,
    platform_message_id: &str,
) -> AppResult<ChannelMessage> {
    db.collection::<ChannelMessage>(COLLECTION_NAME)
        .find_one(doc! {
            "platform": platform,
            "platform_message_id": platform_message_id,
            "direction": "outbound",
        })
        .await?
        .ok_or_else(|| {
            AppError::NotFound(format!("Outbound message not found: {platform_message_id}"))
        })
}

/// Resolve a single outbound message editable by an assigned agent API key.
pub async fn get_outbound_message_for_api_key(
    db: &mongodb::Database,
    api_key_id: &str,
    platform_message_id: &str,
) -> AppResult<ChannelMessage> {
    let platforms = db
        .collection::<mongodb::bson::Document>(CONVERSATIONS)
        .distinct("platform", doc! { "agent_api_key_id": api_key_id })
        .await?;

    let mut found: Option<ChannelMessage> = None;

    for platform in platforms {
        let Bson::String(platform) = platform else {
            continue;
        };

        match get_outbound_message_by_platform_id(db, &platform, platform_message_id).await {
            Ok(message) => {
                if found.is_some() {
                    return Err(AppError::Conflict(format!(
                        "Multiple outbound messages found for platform message ID: {platform_message_id}"
                    )));
                }
                found = Some(message);
            }
            Err(AppError::NotFound(_)) => {}
            Err(err) => return Err(err),
        }
    }

    if let Some(message) = found {
        return Ok(message);
    }

    let matches: Vec<ChannelMessage> = db
        .collection::<ChannelMessage>(COLLECTION_NAME)
        .find(doc! {
            "direction": "outbound",
            "agent_api_key_id": api_key_id,
            "platform_message_id": platform_message_id,
        })
        .await?
        .try_collect()
        .await?;

    match matches.as_slice() {
        [message] => Ok(message.clone()),
        [] => Err(AppError::NotFound(format!(
            "Outbound message not found: {platform_message_id}"
        ))),
        _ => Err(AppError::Conflict(format!(
            "Multiple outbound messages found for platform message ID: {platform_message_id}"
        ))),
    }
}

/// Update the outbound row's `updated_at` timestamp after a successful edit.
pub async fn update_outbound_message_timestamp(
    db: &mongodb::Database,
    message_id: &str,
    updated_at: chrono::DateTime<Utc>,
) -> AppResult<()> {
    let result = db
        .collection::<ChannelMessage>(COLLECTION_NAME)
        .update_one(
            doc! { "_id": message_id, "direction": "outbound" },
            doc! { "$set": { "updated_at": mongodb::bson::DateTime::from_chrono(updated_at) } },
        )
        .await?;

    if result.matched_count == 0 {
        return Err(AppError::NotFound(format!(
            "Outbound message not found: {message_id}"
        )));
    }

    Ok(())
}

/// List messages for a conversation with pagination (newest first).
///
/// Returns `(messages, total_count)`.
pub async fn list_messages(
    db: &mongodb::Database,
    conversation_id: &str,
    page: u64,
    per_page: u64,
) -> AppResult<(Vec<ChannelMessage>, u64)> {
    let filter = doc! { "conversation_id": conversation_id };

    let total = db
        .collection::<ChannelMessage>(COLLECTION_NAME)
        .count_documents(filter.clone())
        .await?;

    let skip = (page.saturating_sub(1)) * per_page;
    let messages: Vec<ChannelMessage> = db
        .collection::<ChannelMessage>(COLLECTION_NAME)
        .find(filter)
        .sort(doc! { "created_at": -1 })
        .skip(skip)
        .limit(per_page as i64)
        .await?
        .try_collect()
        .await?;

    Ok((messages, total))
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Compute HMAC-SHA256 signature of the given body using the provided key.
fn compute_hmac_signature(key: &[u8], body: &[u8]) -> AppResult<String> {
    let mut mac = HmacSha256::new_from_slice(key)
        .map_err(|e| AppError::Internal(format!("HMAC key error: {e}")))?;
    mac.update(body);
    Ok(hex::encode(mac.finalize().into_bytes()))
}

// ---------------------------------------------------------------------------
// Payload builders
// ---------------------------------------------------------------------------

/// Build a [`CallbackPayload`] from the stored message and routing context.
pub fn build_callback_payload(
    message: &ChannelMessage,
    conversation: &crate::models::channel_conversation::ChannelConversation,
    api_key_id: &str,
    api_key_name: &str,
    inbound: &InboundMessage,
    reply_token: Option<String>,
) -> CallbackPayload {
    let attachments: Vec<CallbackAttachment> = inbound
        .attachments
        .iter()
        .map(|a| CallbackAttachment {
            content_type: a.content_type.clone(),
            url: a.url.clone(),
            filename: a.filename.clone(),
            mime_type: a.mime_type.clone(),
            size_bytes: a.size_bytes,
        })
        .collect();

    CallbackPayload {
        message_id: message.id.clone(),
        correlation_id: String::new(),
        platform: message.platform.clone(),
        reply_token,
        agent: CallbackAgent {
            api_key_id: api_key_id.to_string(),
            name: api_key_name.to_string(),
        },
        conversation: CallbackConversation {
            id: conversation.id.clone(),
            // Use the real platform chat ID from the inbound message, not the
            // route's configured value (which may be "*" for default routes).
            platform_id: inbound.conversation_id.clone(),
            conversation_type: conversation.platform_conversation_type.clone(),
        },
        sender: CallbackSender {
            platform_id: inbound.sender_platform_id.clone(),
            display_name: inbound.sender_display_name.clone(),
        },
        content: CallbackContent {
            content_type: inbound.content_type.clone(),
            text: inbound.text.clone(),
            attachments,
        },
        reply_to_message_id: None, // NyxID ID not available without a DB lookup
        reply_to_platform_message_id: inbound.reply_to_platform_message_id.clone(),
        thread_id: inbound.thread_id.clone(),
        timestamp: message.created_at.to_rfc3339(),
        raw_platform_data: Some(inbound.raw_data.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hmac_signature_is_deterministic() {
        let key = b"test_api_key_hash";
        let body = b"{\"message_id\":\"abc\"}";
        let sig1 = compute_hmac_signature(key, body).unwrap();
        let sig2 = compute_hmac_signature(key, body).unwrap();
        assert_eq!(sig1, sig2);
        // HMAC-SHA256 produces 64-char hex string
        assert_eq!(sig1.len(), 64);
    }

    #[test]
    fn hmac_different_keys_different_signatures() {
        let body = b"same body";
        let sig_a = compute_hmac_signature(b"key_a", body).unwrap();
        let sig_b = compute_hmac_signature(b"key_b", body).unwrap();
        assert_ne!(sig_a, sig_b);
    }

    #[test]
    fn hmac_different_bodies_different_signatures() {
        let key = b"same_key";
        let sig_a = compute_hmac_signature(key, b"body_a").unwrap();
        let sig_b = compute_hmac_signature(key, b"body_b").unwrap();
        assert_ne!(sig_a, sig_b);
    }

    #[tokio::test]
    async fn outbound_message_for_api_key_fallback_returns_single_matching_outbound_message() {
        let Some(db) =
            crate::test_utils::connect_test_database("channel_relay_outbound_api_key_single").await
        else {
            eprintln!("skipping channel_relay service test: no local MongoDB available");
            return;
        };

        let now = Utc::now();
        let agent_api_key_id = uuid::Uuid::new_v4().to_string();
        let platform_message_id = "platform-single-message";
        let conversation = crate::models::channel_conversation::ChannelConversation {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: uuid::Uuid::new_v4().to_string(),
            channel_bot_id: Some(uuid::Uuid::new_v4().to_string()),
            platform: "telegram".to_string(),
            platform_conversation_id: "chat-123".to_string(),
            platform_conversation_type: "private".to_string(),
            platform_sender_id: None,
            agent_api_key_id: agent_api_key_id.clone(),
            default_agent: false,
            is_active: true,
            last_message_at: None,
            created_at: now,
            updated_at: now,
        };
        db.collection::<crate::models::channel_conversation::ChannelConversation>(CONVERSATIONS)
            .insert_one(&conversation)
            .await
            .expect("insert conversation");

        let inserted = store_outbound_message(
            &db,
            conversation
                .channel_bot_id
                .as_deref()
                .expect("test conversation bot id"),
            &conversation.id,
            &conversation.user_id,
            "lark",
            &agent_api_key_id,
            None,
            Some(platform_message_id),
        )
        .await
        .expect("insert outbound message");

        let resolved =
            get_outbound_message_for_api_key(&db, &agent_api_key_id, platform_message_id)
                .await
                .expect("single fallback match should resolve");
        assert_eq!(resolved.id, inserted.id);
        assert_eq!(resolved.platform, "lark");
        assert_eq!(
            resolved.platform_message_id.as_deref(),
            Some(platform_message_id)
        );
        assert_eq!(
            resolved.agent_api_key_id.as_deref(),
            Some(agent_api_key_id.as_str())
        );

        db.drop().await.expect("drop test database");
    }

    #[tokio::test]
    async fn outbound_message_for_api_key_fallback_conflicts_on_duplicate_matches() {
        let Some(db) =
            crate::test_utils::connect_test_database("channel_relay_outbound_api_key").await
        else {
            eprintln!("skipping channel_relay service test: no local MongoDB available");
            return;
        };

        let now = Utc::now();
        let agent_api_key_id = uuid::Uuid::new_v4().to_string();
        let other_agent_api_key_id = uuid::Uuid::new_v4().to_string();
        let platform_message_id = "platform-duplicate-message";
        let conversation = crate::models::channel_conversation::ChannelConversation {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: uuid::Uuid::new_v4().to_string(),
            channel_bot_id: Some(uuid::Uuid::new_v4().to_string()),
            platform: "telegram".to_string(),
            platform_conversation_id: "chat-123".to_string(),
            platform_conversation_type: "private".to_string(),
            platform_sender_id: None,
            agent_api_key_id: agent_api_key_id.clone(),
            default_agent: false,
            is_active: true,
            last_message_at: None,
            created_at: now,
            updated_at: now,
        };
        db.collection::<crate::models::channel_conversation::ChannelConversation>(CONVERSATIONS)
            .insert_one(&conversation)
            .await
            .expect("insert conversation");

        store_outbound_message(
            &db,
            conversation
                .channel_bot_id
                .as_deref()
                .expect("test conversation bot id"),
            &conversation.id,
            &conversation.user_id,
            "lark",
            &agent_api_key_id,
            None,
            Some(platform_message_id),
        )
        .await
        .expect("insert first outbound message");
        store_outbound_message(
            &db,
            conversation
                .channel_bot_id
                .as_deref()
                .expect("test conversation bot id"),
            &conversation.id,
            &conversation.user_id,
            "lark",
            &agent_api_key_id,
            None,
            Some(platform_message_id),
        )
        .await
        .expect("insert duplicate outbound message");

        let err = get_outbound_message_for_api_key(&db, &agent_api_key_id, platform_message_id)
            .await
            .expect_err("duplicate fallback matches should conflict");
        assert!(
            matches!(err, AppError::Conflict(msg) if msg.contains("Multiple outbound messages found"))
        );

        let err =
            get_outbound_message_for_api_key(&db, &other_agent_api_key_id, platform_message_id)
                .await
                .expect_err("fallback lookup should stay scoped to the requesting agent");
        assert!(matches!(err, AppError::NotFound(_)));

        db.drop().await.expect("drop test database");
    }

    #[test]
    fn callback_payload_serializes_to_json() {
        let payload = CallbackPayload {
            message_id: "msg-1".to_string(),
            correlation_id: String::new(),
            platform: "telegram".to_string(),
            reply_token: None,
            agent: CallbackAgent {
                api_key_id: "key-1".to_string(),
                name: "test-agent".to_string(),
            },
            conversation: CallbackConversation {
                id: "conv-1".to_string(),
                platform_id: "12345".to_string(),
                conversation_type: "private".to_string(),
            },
            sender: CallbackSender {
                platform_id: "user-1".to_string(),
                display_name: Some("Alice".to_string()),
            },
            content: CallbackContent {
                content_type: "text".to_string(),
                text: Some("Hello".to_string()),
                attachments: vec![],
            },
            reply_to_message_id: None,
            reply_to_platform_message_id: None,
            thread_id: None,
            timestamp: "2026-01-01T00:00:00Z".to_string(),
            raw_platform_data: None,
        };

        let json = serde_json::to_value(&payload).unwrap();
        assert_eq!(json["message_id"], "msg-1");
        assert_eq!(json["correlation_id"], "");
        assert_eq!(json["agent"]["api_key_id"], "key-1");
        assert_eq!(json["conversation"]["type"], "private");
        assert_eq!(json["content"]["type"], "text");
        // Optional None fields should be absent
        assert!(json.get("reply_token").is_none());
        assert!(json.get("reply_to_message_id").is_none());
        assert!(json.get("thread_id").is_none());
    }

    #[test]
    fn callback_payload_includes_reply_token_when_present() {
        let payload = CallbackPayload {
            message_id: "msg-1".to_string(),
            correlation_id: String::new(),
            platform: "telegram".to_string(),
            reply_token: Some("reply-token".to_string()),
            agent: CallbackAgent {
                api_key_id: "key-1".to_string(),
                name: "test-agent".to_string(),
            },
            conversation: CallbackConversation {
                id: "conv-1".to_string(),
                platform_id: "12345".to_string(),
                conversation_type: "private".to_string(),
            },
            sender: CallbackSender {
                platform_id: "user-1".to_string(),
                display_name: None,
            },
            content: CallbackContent {
                content_type: "text".to_string(),
                text: Some("Hello".to_string()),
                attachments: vec![],
            },
            reply_to_message_id: None,
            reply_to_platform_message_id: None,
            thread_id: None,
            timestamp: "2026-01-01T00:00:00Z".to_string(),
            raw_platform_data: None,
        };

        let json = serde_json::to_value(&payload).unwrap();
        assert_eq!(json["reply_token"], "reply-token");
    }

    #[test]
    fn callback_payload_omits_empty_attachments() {
        let content = CallbackContent {
            content_type: "text".to_string(),
            text: Some("Hi".to_string()),
            attachments: vec![],
        };
        let json = serde_json::to_value(&content).unwrap();
        // Empty attachments should be omitted
        assert!(json.get("attachments").is_none());
    }

    #[test]
    fn callback_payload_includes_nonempty_attachments() {
        let content = CallbackContent {
            content_type: "image".to_string(),
            text: None,
            attachments: vec![CallbackAttachment {
                content_type: "image".to_string(),
                url: "https://example.com/photo.jpg".to_string(),
                filename: Some("photo.jpg".to_string()),
                mime_type: Some("image/jpeg".to_string()),
                size_bytes: Some(1024),
            }],
        };
        let json = serde_json::to_value(&content).unwrap();
        let atts = json["attachments"].as_array().unwrap();
        assert_eq!(atts.len(), 1);
        assert_eq!(atts[0]["content_type"], "image");
    }

    // ─── forward_to_agent HTTP behavior ───
    //
    // Per ADR-013 / NyxID#221: only 202 is a success, 200+body is tolerated
    // but the body is ignored. Non-2xx returns ChannelRelayFailed.

    fn test_payload() -> CallbackPayload {
        CallbackPayload {
            message_id: "msg-test".to_string(),
            correlation_id: String::new(),
            platform: "device".to_string(),
            reply_token: None,
            agent: CallbackAgent {
                api_key_id: "key-1".to_string(),
                name: "test-agent".to_string(),
            },
            conversation: CallbackConversation {
                id: "conv-1".to_string(),
                platform_id: "household-1".to_string(),
                conversation_type: "device".to_string(),
            },
            sender: CallbackSender {
                platform_id: "camera".to_string(),
                display_name: None,
            },
            content: CallbackContent {
                content_type: "person_detected".to_string(),
                text: Some("{}".to_string()),
                attachments: vec![],
            },
            reply_to_message_id: None,
            reply_to_platform_message_id: None,
            thread_id: None,
            timestamp: "2026-04-08T12:00:00Z".to_string(),
            raw_platform_data: None,
        }
    }

    fn test_app_config() -> crate::config::AppConfig {
        // Only fields touched by forward_to_agent matter. The rest are
        // padded with safe defaults to construct a complete AppConfig.
        crate::config::AppConfig {
            port: 0,
            base_url: "http://localhost".to_string(),
            frontend_url: "http://localhost".to_string(),
            cors_allowed_origins: vec![],
            csrf_trusted_origins: vec![],
            database_url: String::new(),
            database_max_connections: 1,
            environment: "test".to_string(),
            jwt_private_key_path: String::new(),
            jwt_public_key_path: String::new(),
            jwt_issuer: "test".to_string(),
            jwt_access_ttl_secs: 900,
            jwt_relay_reply_ttl_secs: 1800,
            jwt_relay_callback_ttl_secs: 300,
            jwt_refresh_ttl_secs: 604800,
            google_client_id: None,
            google_client_secret: None,
            github_client_id: None,
            github_client_secret: None,
            apple_client_id: None,
            apple_team_id: None,
            apple_key_id: None,
            apple_private_key_path: None,
            smtp_host: None,
            smtp_port: None,
            smtp_username: None,
            smtp_password: None,
            smtp_from_address: None,
            encryption_key: None,
            encryption_key_previous: None,
            rate_limit_per_second: 10,
            rate_limit_burst: 30,
            trusted_proxy_ips: vec![],
            mtls_client_cert_header: None,
            cli_pairing_hmac_key: None,
            sa_token_ttl_secs: 3600,
            telemetry_dsn: None,
            telemetry_host: None,
            share_analytics: false,
            cookie_domain: None,
            telegram_bot_token: None,
            telegram_webhook_secret: None,
            telegram_webhook_url: None,
            telegram_bot_username: None,
            approval_expiry_interval_secs: 5,
            oauth_refresh_sweep_interval_secs: 600,
            oauth_refresh_sweep_window_secs: 900,
            fcm_service_account_path: None,
            fcm_project_id: None,
            apns_key_path: None,
            apns_key_id: None,
            apns_team_id: None,
            apns_topic: None,
            apns_sandbox: true,
            key_provider: "local".to_string(),
            aws_kms_key_arn: None,
            aws_kms_key_arn_previous: None,
            gcp_kms_key_name: None,
            gcp_kms_key_name_previous: None,
            node_heartbeat_interval_secs: 30,
            node_heartbeat_timeout_secs: 90,
            node_proxy_timeout_secs: 30,
            node_registration_token_ttl_secs: 3600,
            node_pending_credential_ttl_secs: 86_400,
            node_max_per_user: 10,
            node_max_ws_connections: 100,
            node_max_stream_duration_secs: 300,
            node_hmac_signing_enabled: true,
            proxy_max_body_size: 100 * 1024 * 1024,
            proxy_stream_idle_timeout_secs: 60,
            ssh_max_sessions_per_user: 4,
            ssh_connect_timeout_secs: 10,
            ssh_max_tunnel_duration_secs: 3600,
            ws_passthrough_max_connections: 200,
            channel_relay_callback_timeout_secs: 5,
            channel_relay_max_bots_per_user: 5,
            channel_relay_message_ttl_days: 30,
            channel_relay_edit_rate_limit_per_second: 10,
            channel_relay_edit_rate_limit_burst: 20,
            channel_event_rate_limit_per_second: 100,
            channel_event_rate_limit_burst: 200,
            channel_event_dedup_capacity: 32_768,
            channel_event_dedup_ttl_secs: 300,
            cloud_response_cache_ttl_secs: 0,
            cloud_response_cache_max_entry_bytes: 1024 * 1024,
            cloud_response_cache_max_entries: 256,
            invite_code_required: true,
            email_auth_enabled: false,
            auto_verify_email: false,
        }
    }

    fn test_jwt_keys() -> crate::crypto::jwt::JwtKeys {
        // Reuse the process-wide cached test keypair so this module's
        // 11 callers don't each pay a fresh RSA keygen.
        crate::test_utils::cached_test_jwt_keys()
    }

    #[derive(Clone, Debug)]
    struct CapturedCallbackRequest {
        headers: axum::http::HeaderMap,
        body: Vec<u8>,
    }

    /// Spin up a minimal axum mock server that returns the given status code
    /// and optional body. Returns the callback URL to POST to and captured
    /// callback requests.
    async fn spawn_mock_callback(
        status: u16,
        body: Option<&'static str>,
    ) -> (
        String,
        tokio::task::JoinHandle<()>,
        std::sync::Arc<tokio::sync::Mutex<Vec<CapturedCallbackRequest>>>,
    ) {
        use axum::{Router, body::Bytes, http::HeaderMap, http::StatusCode, routing::post};
        use tokio::net::TcpListener;

        let code = StatusCode::from_u16(status).unwrap();
        let body = body.unwrap_or("");
        let captured = std::sync::Arc::new(tokio::sync::Mutex::new(Vec::new()));
        let captured_for_route = captured.clone();

        let app = Router::new().route(
            "/callback",
            post(move |headers: HeaderMap, request_body: Bytes| {
                let captured = captured_for_route.clone();
                async move {
                    captured.lock().await.push(CapturedCallbackRequest {
                        headers,
                        body: request_body.to_vec(),
                    });
                    (code, body)
                }
            }),
        );
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (format!("http://{addr}/callback"), server, captured)
    }

    async fn captured_request(
        captured: &std::sync::Arc<tokio::sync::Mutex<Vec<CapturedCallbackRequest>>>,
    ) -> CapturedCallbackRequest {
        captured
            .lock()
            .await
            .first()
            .expect("callback request captured")
            .clone()
    }

    fn header_value<'a>(headers: &'a axum::http::HeaderMap, name: &str) -> &'a str {
        headers
            .get(name)
            .unwrap_or_else(|| panic!("{name} header should be present"))
            .to_str()
            .unwrap_or_else(|_| panic!("{name} header should be valid UTF-8"))
    }

    #[tokio::test]
    async fn forward_to_agent_accepts_202() {
        let (url, server, _captured) = spawn_mock_callback(202, None).await;
        let client = reqwest::Client::new();
        let config = test_app_config();
        let keys = test_jwt_keys();
        let delivery = forward_to_agent(
            &client,
            &config,
            &keys,
            &url,
            test_payload(),
            "key-1",
            "test_api_key_hash",
            None,
        )
        .await;
        assert!(delivery.result.is_ok());
        assert_eq!(delivery.http_status, Some(202));
        server.abort();
    }

    #[tokio::test]
    async fn forward_to_agent_emits_dual_signatures() {
        let (url, server, captured) = spawn_mock_callback(202, None).await;
        let client = reqwest::Client::new();
        let config = test_app_config();
        let keys = test_jwt_keys();

        let delivery = forward_to_agent(
            &client,
            &config,
            &keys,
            &url,
            test_payload(),
            "key-1",
            "test_api_key_hash",
            None,
        )
        .await;

        assert!(delivery.result.is_ok());
        let request = captured_request(&captured).await;
        assert!(!header_value(&request.headers, "X-NyxID-Signature").is_empty());
        assert!(!header_value(&request.headers, "X-NyxID-Callback-Token").is_empty());
        server.abort();
    }

    #[tokio::test]
    async fn forward_to_agent_callback_token_validates_via_jwks() {
        let (url, server, captured) = spawn_mock_callback(202, None).await;
        let client = reqwest::Client::new();
        let config = test_app_config();
        let keys = test_jwt_keys();
        let payload = test_payload();
        let expected_message_id = payload.message_id.clone();
        let expected_platform = payload.platform.clone();

        let delivery = forward_to_agent(
            &client,
            &config,
            &keys,
            &url,
            payload,
            "key-1",
            "test_api_key_hash",
            None,
        )
        .await;

        assert!(delivery.result.is_ok());
        let request = captured_request(&captured).await;
        let token = header_value(&request.headers, "X-NyxID-Callback-Token");
        let claims = crate::crypto::jwt::validate_relay_callback_token(&keys, &config, token)
            .expect("callback token should validate");
        assert_eq!(claims.aud, crate::crypto::jwt::RELAY_CALLBACK_AUDIENCE);
        assert_eq!(
            claims.token_type,
            crate::crypto::jwt::RELAY_CALLBACK_TOKEN_TYPE
        );
        assert_eq!(claims.api_key_id, "key-1");
        assert_eq!(claims.message_id, expected_message_id);
        assert_eq!(claims.platform, expected_platform);
        assert_eq!(claims.exp - claims.iat, config.jwt_relay_callback_ttl_secs);
        assert_eq!(claims.iss, config.jwt_issuer);
        server.abort();
    }

    #[tokio::test]
    async fn forward_to_agent_body_sha256_matches_wire_bytes() {
        let (url, server, captured) = spawn_mock_callback(202, None).await;
        let client = reqwest::Client::new();
        let config = test_app_config();
        let keys = test_jwt_keys();
        let api_key_hash = "test_api_key_hash";

        let delivery = forward_to_agent(
            &client,
            &config,
            &keys,
            &url,
            test_payload(),
            "key-1",
            api_key_hash,
            None,
        )
        .await;

        assert!(delivery.result.is_ok());
        let request = captured_request(&captured).await;
        let token = header_value(&request.headers, "X-NyxID-Callback-Token");
        let signature = header_value(&request.headers, "X-NyxID-Signature");
        let claims = crate::crypto::jwt::validate_relay_callback_token(&keys, &config, token)
            .expect("callback token should validate");
        assert_eq!(
            claims.body_sha256,
            hex::encode(Sha256::digest(&request.body))
        );
        assert_eq!(
            signature,
            compute_hmac_signature(api_key_hash.as_bytes(), &request.body).unwrap()
        );
        server.abort();
    }

    #[tokio::test]
    async fn forward_to_agent_correlation_id_equals_jti() {
        let (url, server, captured) = spawn_mock_callback(202, None).await;
        let client = reqwest::Client::new();
        let config = test_app_config();
        let keys = test_jwt_keys();

        let delivery = forward_to_agent(
            &client,
            &config,
            &keys,
            &url,
            test_payload(),
            "key-1",
            "test_api_key_hash",
            None,
        )
        .await;

        assert!(delivery.result.is_ok());
        let request = captured_request(&captured).await;
        let token = header_value(&request.headers, "X-NyxID-Callback-Token");
        let claims = crate::crypto::jwt::validate_relay_callback_token(&keys, &config, token)
            .expect("callback token should validate");
        let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
        assert_eq!(
            body["correlation_id"].as_str().expect("correlation_id"),
            claims.jti
        );
        server.abort();
    }

    #[tokio::test]
    async fn forward_to_agent_body_sha256_rejects_logically_equivalent_different_bytes() {
        let (url, server, captured) = spawn_mock_callback(202, None).await;
        let client = reqwest::Client::new();
        let config = test_app_config();
        let keys = test_jwt_keys();

        let delivery = forward_to_agent(
            &client,
            &config,
            &keys,
            &url,
            test_payload(),
            "key-1",
            "test_api_key_hash",
            None,
        )
        .await;

        assert!(delivery.result.is_ok());
        let request = captured_request(&captured).await;
        let token = header_value(&request.headers, "X-NyxID-Callback-Token");
        let claims = crate::crypto::jwt::validate_relay_callback_token(&keys, &config, token)
            .expect("callback token should validate");
        let parsed_body: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
        let pretty_body = serde_json::to_vec_pretty(&parsed_body).unwrap();
        let pretty_hash = hex::encode(Sha256::digest(&pretty_body));

        assert_eq!(
            claims.body_sha256,
            hex::encode(Sha256::digest(&request.body))
        );
        assert_ne!(claims.body_sha256, pretty_hash);
        server.abort();
    }

    #[tokio::test]
    async fn forward_to_agent_ignores_200_body() {
        // Legacy 200+body replies must be tolerated but the body is dropped.
        let (url, server, _captured) =
            spawn_mock_callback(200, Some(r#"{"reply": {"text": "ignored"}}"#)).await;
        let client = reqwest::Client::new();
        let config = test_app_config();
        let keys = test_jwt_keys();
        let delivery = forward_to_agent(
            &client,
            &config,
            &keys,
            &url,
            test_payload(),
            "key-1",
            "test_api_key_hash",
            None,
        )
        .await;
        assert!(
            delivery.result.is_ok(),
            "200 must be tolerated, got {delivery:?}"
        );
        assert_eq!(delivery.http_status, Some(200));
        server.abort();
    }

    #[tokio::test]
    async fn forward_to_agent_returns_error_on_500() {
        let (url, server, _captured) = spawn_mock_callback(500, None).await;
        let client = reqwest::Client::new();
        let config = test_app_config();
        let keys = test_jwt_keys();
        let delivery = forward_to_agent(
            &client,
            &config,
            &keys,
            &url,
            test_payload(),
            "key-1",
            "test_api_key_hash",
            None,
        )
        .await;
        assert!(matches!(
            delivery.result,
            Err(AppError::ChannelRelayFailed(_))
        ));
        assert_eq!(delivery.http_status, Some(500));
        server.abort();
    }

    #[tokio::test]
    async fn forward_to_agent_returns_error_on_404() {
        let (url, server, _captured) = spawn_mock_callback(404, None).await;
        let client = reqwest::Client::new();
        let config = test_app_config();
        let keys = test_jwt_keys();
        let delivery = forward_to_agent(
            &client,
            &config,
            &keys,
            &url,
            test_payload(),
            "key-1",
            "test_api_key_hash",
            None,
        )
        .await;
        assert!(matches!(
            delivery.result,
            Err(AppError::ChannelRelayFailed(_))
        ));
        assert_eq!(delivery.http_status, Some(404));
        server.abort();
    }

    #[tokio::test]
    async fn forward_to_agent_transport_error_has_no_status() {
        // Unreachable target: port 1 is reserved and should not be listening.
        let client = reqwest::Client::new();
        let config = test_app_config();
        let keys = test_jwt_keys();
        let delivery = forward_to_agent(
            &client,
            &config,
            &keys,
            "http://127.0.0.1:1/callback",
            test_payload(),
            "key-1",
            "test_api_key_hash",
            None,
        )
        .await;
        assert!(matches!(
            delivery.result,
            Err(AppError::ChannelRelayFailed(_))
        ));
        assert_eq!(delivery.http_status, None);
    }
}

//! Channel relay service: message storage, agent callback delivery, and reply
//! handling.
//!
//! Responsible for persisting inbound/outbound messages, forwarding inbound
//! messages to agent callback URLs with HMAC signatures, and parsing
//! synchronous reply payloads.

use chrono::Utc;
use futures::TryStreamExt;
use hmac::{Hmac, Mac};
use mongodb::bson::doc;
use serde::Serialize;
use sha2::Sha256;

use crate::config::AppConfig;
use crate::errors::{AppError, AppResult};
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
        created_at: Utc::now(),
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
/// Signs the request body with HMAC-SHA256 using the API key hash as the
/// signing key. **Only HTTP 202 is a success.** HTTP 200 is tolerated for
/// legacy clients but the response body is ignored (sync reply mode was
/// deprecated per ADR-013 and NyxID#221 comment 2 — agent replies must flow
/// through `POST /api/v1/channel-relay/reply`).
///
/// Returns a [`CallbackDelivery`] so callers can observe the actual upstream
/// status code in addition to the success/error result.
pub async fn forward_to_agent(
    http_client: &reqwest::Client,
    config: &AppConfig,
    callback_url: &str,
    payload: &CallbackPayload,
    api_key_hash: &str,
    user_access_token: Option<&str>,
) -> CallbackDelivery {
    let body_bytes = match serde_json::to_vec(payload) {
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

    #[test]
    fn callback_payload_serializes_to_json() {
        let payload = CallbackPayload {
            message_id: "msg-1".to_string(),
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
            channel_event_rate_limit_per_second: 100,
            channel_event_rate_limit_burst: 200,
            channel_event_dedup_capacity: 32_768,
            channel_event_dedup_ttl_secs: 300,
            invite_code_required: true,
            email_auth_enabled: false,
            auto_verify_email: false,
        }
    }

    /// Spin up a minimal axum mock server that returns the given status code
    /// and optional body. Returns the callback URL to POST to.
    async fn spawn_mock_callback(
        status: u16,
        body: Option<&'static str>,
    ) -> (String, tokio::task::JoinHandle<()>) {
        use axum::{Router, http::StatusCode, routing::post};
        use tokio::net::TcpListener;

        let code = StatusCode::from_u16(status).unwrap();
        let body = body.unwrap_or("");

        let app = Router::new().route("/callback", post(move || async move { (code, body) }));
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (format!("http://{addr}/callback"), server)
    }

    #[tokio::test]
    async fn forward_to_agent_accepts_202() {
        let (url, server) = spawn_mock_callback(202, None).await;
        let client = reqwest::Client::new();
        let config = test_app_config();
        let delivery = forward_to_agent(
            &client,
            &config,
            &url,
            &test_payload(),
            "test_api_key_hash",
            None,
        )
        .await;
        assert!(delivery.result.is_ok());
        assert_eq!(delivery.http_status, Some(202));
        server.abort();
    }

    #[tokio::test]
    async fn forward_to_agent_ignores_200_body() {
        // Legacy 200+body replies must be tolerated but the body is dropped.
        let (url, server) =
            spawn_mock_callback(200, Some(r#"{"reply": {"text": "ignored"}}"#)).await;
        let client = reqwest::Client::new();
        let config = test_app_config();
        let delivery = forward_to_agent(
            &client,
            &config,
            &url,
            &test_payload(),
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
        let (url, server) = spawn_mock_callback(500, None).await;
        let client = reqwest::Client::new();
        let config = test_app_config();
        let delivery = forward_to_agent(
            &client,
            &config,
            &url,
            &test_payload(),
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
        let (url, server) = spawn_mock_callback(404, None).await;
        let client = reqwest::Client::new();
        let config = test_app_config();
        let delivery = forward_to_agent(
            &client,
            &config,
            &url,
            &test_payload(),
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
        let delivery = forward_to_agent(
            &client,
            &config,
            "http://127.0.0.1:1/callback",
            &test_payload(),
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

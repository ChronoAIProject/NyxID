//! Discord platform adapter for the Channel Bot Relay system.
//!
//! Implements [`PlatformAdapter`] to normalize Discord Interactions (slash
//! commands, message components) and Gateway message events into the
//! platform-agnostic [`InboundMessage`] format. Replies are sent via the
//! Discord REST API (`POST /channels/{id}/messages`).
//!
//! Webhook verification uses Ed25519 signature validation per Discord's
//! Interactions endpoint requirements.

use ed25519_dalek::{Signature, VerifyingKey};

use crate::errors::{AppError, AppResult};
use crate::models::channel_bot::ChannelBot;
use crate::services::channel_platform::{
    BotIdentity, InboundMessage, OutboundReply, PlatformAdapter,
};

const DISCORD_API_BASE: &str = "https://discord.com/api/v10";

/// Discord Interaction types.
const INTERACTION_PING: u64 = 1;
const INTERACTION_APPLICATION_COMMAND: u64 = 2;
const INTERACTION_MESSAGE_COMPONENT: u64 = 4;

/// Discord channel types.
const CHANNEL_DM: u64 = 1;
const CHANNEL_GROUP_DM: u64 = 3;

/// Discord platform adapter.
///
/// Stateless -- all state lives in the [`ChannelBot`] document and the Discord
/// API itself.
pub struct DiscordAdapter;

impl Default for DiscordAdapter {
    fn default() -> Self {
        Self
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Map Discord channel type integer to our normalized conversation type.
fn map_conversation_type(channel_type: Option<u64>) -> &'static str {
    match channel_type {
        Some(CHANNEL_DM) => "private",
        Some(CHANNEL_GROUP_DM) => "group",
        // Guild text (0), announcement (5), forum (15), etc. are all "group"
        Some(0 | 5 | 10 | 11 | 12 | 15) => "group",
        // Default to group for unknown guild channel types
        _ => "group",
    }
}

/// Parse sender information from a Discord interaction or message payload.
fn extract_sender(payload: &serde_json::Value) -> (String, Option<String>) {
    // Interaction: member.user.id or user.id
    if let Some(member) = payload.get("member")
        && let Some(user) = member.get("user")
    {
        let id = user
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let name = user
            .get("username")
            .and_then(|v| v.as_str())
            .map(String::from);
        return (id, name);
    }

    // DM interaction: user.id
    if let Some(user) = payload.get("user") {
        let id = user
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let name = user
            .get("username")
            .and_then(|v| v.as_str())
            .map(String::from);
        return (id, name);
    }

    // Gateway message: author.id
    if let Some(author) = payload.get("author") {
        let id = author
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let name = author
            .get("username")
            .and_then(|v| v.as_str())
            .map(String::from);
        return (id, name);
    }

    (String::new(), None)
}

/// Parse an APPLICATION_COMMAND or MESSAGE_COMPONENT interaction into an
/// [`InboundMessage`].
fn parse_interaction(payload: &serde_json::Value) -> Option<InboundMessage> {
    let interaction_id = payload.get("id")?.as_str()?;
    let channel_id = payload.get("channel_id").and_then(|v| v.as_str())?;
    let channel_type = payload
        .get("channel")
        .and_then(|c| c.get("type"))
        .and_then(|v| v.as_u64());

    let (sender_id, sender_name) = extract_sender(payload);

    // Extract text content from interaction data
    let text = payload.get("data").and_then(|d| {
        // Slash command: concatenate options as text representation
        if let Some(name) = d.get("name").and_then(|v| v.as_str()) {
            let opts = d
                .get("options")
                .and_then(|o| o.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|opt| {
                            let k = opt.get("name")?.as_str()?;
                            let v = opt.get("value")?;
                            let v_str = match v.as_str() {
                                Some(s) => s.to_string(),
                                None => v.to_string(),
                            };
                            Some(format!("{k}={v_str}"))
                        })
                        .collect::<Vec<_>>()
                        .join(" ")
                })
                .unwrap_or_default();
            if opts.is_empty() {
                return Some(format!("/{name}"));
            }
            return Some(format!("/{name} {opts}"));
        }
        // Button / select menu: custom_id
        d.get("custom_id")
            .and_then(|v| v.as_str())
            .map(String::from)
    });

    // Preserve the interaction token for follow-up replies (deferred interactions).
    // Stored as "interaction:{application_id}:{token}" in thread_id.
    let interaction_token = payload.get("token").and_then(|v| v.as_str());
    let application_id = payload.get("application_id").and_then(|v| v.as_str());
    let thread_id = match (application_id, interaction_token) {
        (Some(app_id), Some(token)) => Some(format!("interaction:{app_id}:{token}")),
        _ => None,
    };

    Some(InboundMessage {
        platform_message_id: interaction_id.to_string(),
        conversation_id: channel_id.to_string(),
        conversation_type: map_conversation_type(channel_type).to_string(),
        sender_platform_id: sender_id,
        sender_display_name: sender_name,
        content_type: "text".to_string(),
        text,
        attachments: Vec::new(),
        reply_to_platform_message_id: None,
        thread_id,
        raw_data: payload.clone(),
    })
}

/// Parse a Gateway-style message object into an [`InboundMessage`].
fn parse_gateway_message(payload: &serde_json::Value) -> Option<InboundMessage> {
    let msg = payload.get("d").unwrap_or(payload);
    let message_id = msg.get("id")?.as_str()?;
    let channel_id = msg.get("channel_id")?.as_str()?;

    let (sender_id, sender_name) = extract_sender(msg);

    let text = msg
        .get("content")
        .and_then(|v| v.as_str())
        .map(String::from);

    let reply_to = msg
        .get("message_reference")
        .and_then(|r| r.get("message_id"))
        .and_then(|v| v.as_str())
        .map(String::from);

    let thread_id = msg
        .get("thread")
        .and_then(|t| t.get("id"))
        .and_then(|v| v.as_str())
        .map(String::from);

    Some(InboundMessage {
        platform_message_id: message_id.to_string(),
        conversation_id: channel_id.to_string(),
        conversation_type: "group".to_string(),
        sender_platform_id: sender_id,
        sender_display_name: sender_name,
        content_type: if text.is_some() { "text" } else { "unknown" }.to_string(),
        text,
        attachments: Vec::new(),
        reply_to_platform_message_id: reply_to,
        thread_id,
        raw_data: payload.clone(),
    })
}

// ---------------------------------------------------------------------------
// PlatformAdapter implementation
// ---------------------------------------------------------------------------

#[async_trait::async_trait]
impl PlatformAdapter for DiscordAdapter {
    fn platform_id(&self) -> &str {
        "discord"
    }

    async fn verify_webhook(
        &self,
        bot: &ChannelBot,
        _secrets: Option<&crate::services::channel_platform::PlatformVerifySecrets>,
        headers: &axum::http::HeaderMap,
        body: &[u8],
    ) -> AppResult<()> {
        let public_key_hex = bot.public_key.as_deref().ok_or_else(|| {
            AppError::ChannelWebhookVerificationFailed(
                "Discord bot missing public_key configuration".to_string(),
            )
        })?;

        let signature_hex = headers
            .get("x-signature-ed25519")
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| {
                AppError::ChannelWebhookVerificationFailed(
                    "missing X-Signature-Ed25519 header".to_string(),
                )
            })?;

        let timestamp = headers
            .get("x-signature-timestamp")
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| {
                AppError::ChannelWebhookVerificationFailed(
                    "missing X-Signature-Timestamp header".to_string(),
                )
            })?;

        // Decode the public key from hex (32 bytes)
        let pk_bytes = hex::decode(public_key_hex).map_err(|_| {
            AppError::ChannelWebhookVerificationFailed(
                "invalid public key hex encoding".to_string(),
            )
        })?;

        let verifying_key =
            VerifyingKey::from_bytes(pk_bytes.as_slice().try_into().map_err(|_| {
                AppError::ChannelWebhookVerificationFailed(
                    "invalid public key length (expected 32 bytes)".to_string(),
                )
            })?)
            .map_err(|_| {
                AppError::ChannelWebhookVerificationFailed("invalid Ed25519 public key".to_string())
            })?;

        // Decode the signature from hex (64 bytes)
        let sig_bytes = hex::decode(signature_hex).map_err(|_| {
            AppError::ChannelWebhookVerificationFailed("invalid signature hex encoding".to_string())
        })?;

        let signature = Signature::from_bytes(sig_bytes.as_slice().try_into().map_err(|_| {
            AppError::ChannelWebhookVerificationFailed(
                "invalid signature length (expected 64 bytes)".to_string(),
            )
        })?);

        // Verify: Ed25519(public_key, timestamp + body, signature)
        let mut message = Vec::with_capacity(timestamp.len() + body.len());
        message.extend_from_slice(timestamp.as_bytes());
        message.extend_from_slice(body);

        use ed25519_dalek::Verifier;
        verifying_key.verify(&message, &signature).map_err(|_| {
            AppError::ChannelWebhookVerificationFailed(
                "Ed25519 signature verification failed".to_string(),
            )
        })?;

        Ok(())
    }

    async fn parse_inbound(&self, body: &[u8]) -> AppResult<Vec<InboundMessage>> {
        let payload: serde_json::Value = serde_json::from_slice(body)
            .map_err(|e| AppError::BadRequest(format!("invalid Discord webhook JSON: {e}")))?;

        let interaction_type = payload.get("type").and_then(|v| v.as_u64());

        match interaction_type {
            // PING -- handled by handle_challenge, return empty
            Some(INTERACTION_PING) => Ok(Vec::new()),
            // APPLICATION_COMMAND or MESSAGE_COMPONENT
            Some(INTERACTION_APPLICATION_COMMAND) | Some(INTERACTION_MESSAGE_COMPONENT) => {
                match parse_interaction(&payload) {
                    Some(msg) => Ok(vec![msg]),
                    None => Ok(Vec::new()),
                }
            }
            // No type field -- might be a Gateway-style message
            None => match parse_gateway_message(&payload) {
                Some(msg) => Ok(vec![msg]),
                None => Ok(Vec::new()),
            },
            // Unhandled interaction types
            _ => Ok(Vec::new()),
        }
    }

    async fn send_reply(
        &self,
        http: &reqwest::Client,
        bot_token: &str,
        conversation_id: &str,
        reply: &OutboundReply,
    ) -> AppResult<Option<String>> {
        let text = reply.text.as_deref().unwrap_or("");

        let body = serde_json::json!({ "content": text });

        // Check if this is a deferred interaction follow-up. The interaction
        // token is passed via metadata as "interaction_thread_id" =
        // "interaction:{application_id}:{token}".
        let interaction_info = reply
            .metadata
            .as_ref()
            .and_then(|m| m.get("interaction_thread_id"))
            .and_then(|v| v.as_str())
            .and_then(|s| {
                let parts: Vec<&str> = s.splitn(3, ':').collect();
                if parts.len() == 3 && parts[0] == "interaction" {
                    Some((parts[1].to_string(), parts[2].to_string()))
                } else {
                    None
                }
            });

        let url = if let Some((app_id, token)) = &interaction_info {
            // Interaction follow-up endpoint (for deferred responses)
            format!("{DISCORD_API_BASE}/webhooks/{app_id}/{token}")
        } else {
            // Regular channel message
            format!("{DISCORD_API_BASE}/channels/{conversation_id}/messages")
        };
        let resp: serde_json::Value = http
            .post(&url)
            .header("Authorization", format!("Bot {bot_token}"))
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                AppError::ChannelPlatformError(format!(
                    "Discord create message request failed: {e}"
                ))
            })?
            .json()
            .await
            .map_err(|e| {
                AppError::ChannelPlatformError(format!(
                    "Discord create message response parse failed: {e}"
                ))
            })?;

        // Discord returns the message object on success with an `id` field.
        // On error it returns `{ "code": ..., "message": "..." }`.
        if let Some(error_msg) = resp.get("message").filter(|_| resp.get("code").is_some()) {
            let desc = error_msg.as_str().unwrap_or("unknown error");
            return Err(AppError::ChannelPlatformError(format!(
                "Discord create message failed: {desc}"
            )));
        }

        let message_id = resp.get("id").and_then(|v| v.as_str()).map(String::from);

        Ok(message_id)
    }

    async fn register_webhook(
        &self,
        _http: &reqwest::Client,
        _bot_token: &str,
        _webhook_url: &str,
        _secret: &str,
    ) -> AppResult<()> {
        // Discord Interactions endpoint URL is configured in the Discord
        // Developer Portal, not via API. This is a no-op.
        Ok(())
    }

    async fn verify_bot_token(
        &self,
        http: &reqwest::Client,
        bot_token: &str,
    ) -> AppResult<BotIdentity> {
        let url = format!("{DISCORD_API_BASE}/users/@me");
        let resp: serde_json::Value = http
            .get(&url)
            .header("Authorization", format!("Bot {bot_token}"))
            .send()
            .await
            .map_err(|e| {
                AppError::ChannelPlatformError(format!("Discord users/@me request failed: {e}"))
            })?
            .json()
            .await
            .map_err(|e| {
                AppError::ChannelPlatformError(format!(
                    "Discord users/@me response parse failed: {e}"
                ))
            })?;

        let bot_id = resp.get("id").and_then(|v| v.as_str()).ok_or_else(|| {
            AppError::ChannelPlatformError("Discord users/@me response missing id".to_string())
        })?;

        let username = resp
            .get("username")
            .and_then(|v| v.as_str())
            .unwrap_or_default();

        Ok(BotIdentity {
            platform_bot_id: bot_id.to_string(),
            platform_bot_username: username.to_string(),
        })
    }

    fn handle_challenge(&self, body: &[u8]) -> Option<serde_json::Value> {
        let payload: serde_json::Value = serde_json::from_slice(body).ok()?;
        let interaction_type = payload.get("type")?.as_u64()?;

        if interaction_type == INTERACTION_PING {
            // Respond with PONG (type: 1)
            Some(serde_json::json!({ "type": 1 }))
        } else {
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- platform_id ---------------------------------------------------------

    #[test]
    fn platform_id_is_discord() {
        let adapter = DiscordAdapter;
        assert_eq!(adapter.platform_id(), "discord");
    }

    // -- handle_challenge ----------------------------------------------------

    #[test]
    fn handle_challenge_ping_returns_pong() {
        let adapter = DiscordAdapter;
        let body = serde_json::json!({ "type": 1 }).to_string();
        let result = adapter.handle_challenge(body.as_bytes());
        assert!(result.is_some());
        let pong = result.unwrap();
        assert_eq!(pong["type"], 1);
    }

    #[test]
    fn handle_challenge_non_ping_returns_none() {
        let adapter = DiscordAdapter;
        let body = serde_json::json!({ "type": 2, "data": {} }).to_string();
        assert!(adapter.handle_challenge(body.as_bytes()).is_none());
    }

    #[test]
    fn handle_challenge_invalid_json_returns_none() {
        let adapter = DiscordAdapter;
        assert!(adapter.handle_challenge(b"not json").is_none());
    }

    // -- parse_inbound -------------------------------------------------------

    #[tokio::test]
    async fn parse_ping_returns_empty() {
        let adapter = DiscordAdapter;
        let body = serde_json::json!({ "type": 1 });
        let raw = serde_json::to_vec(&body).unwrap();
        let msgs = adapter.parse_inbound(&raw).await.unwrap();
        assert!(msgs.is_empty());
    }

    #[tokio::test]
    async fn parse_application_command() {
        let adapter = DiscordAdapter;
        let body = serde_json::json!({
            "type": 2,
            "id": "interaction_123",
            "channel_id": "ch_456",
            "channel": { "type": 0 },
            "member": {
                "user": {
                    "id": "user_789",
                    "username": "TestUser"
                }
            },
            "data": {
                "name": "ask",
                "options": [
                    { "name": "question", "value": "hello?" }
                ]
            }
        });
        let raw = serde_json::to_vec(&body).unwrap();
        let msgs = adapter.parse_inbound(&raw).await.unwrap();

        assert_eq!(msgs.len(), 1);
        let m = &msgs[0];
        assert_eq!(m.platform_message_id, "interaction_123");
        assert_eq!(m.conversation_id, "ch_456");
        assert_eq!(m.conversation_type, "group");
        assert_eq!(m.sender_platform_id, "user_789");
        assert_eq!(m.sender_display_name.as_deref(), Some("TestUser"));
        assert_eq!(m.content_type, "text");
        assert_eq!(m.text.as_deref(), Some("/ask question=hello?"));
    }

    #[tokio::test]
    async fn parse_message_component() {
        let adapter = DiscordAdapter;
        let body = serde_json::json!({
            "type": 4,
            "id": "comp_111",
            "channel_id": "ch_222",
            "channel": { "type": 1 },
            "user": {
                "id": "dm_user",
                "username": "DMUser"
            },
            "data": {
                "custom_id": "approve_action"
            }
        });
        let raw = serde_json::to_vec(&body).unwrap();
        let msgs = adapter.parse_inbound(&raw).await.unwrap();

        assert_eq!(msgs.len(), 1);
        let m = &msgs[0];
        assert_eq!(m.conversation_type, "private");
        assert_eq!(m.sender_platform_id, "dm_user");
        assert_eq!(m.text.as_deref(), Some("approve_action"));
    }

    #[tokio::test]
    async fn parse_gateway_message() {
        let adapter = DiscordAdapter;
        let body = serde_json::json!({
            "id": "msg_555",
            "channel_id": "ch_666",
            "author": {
                "id": "author_777",
                "username": "GatewayUser"
            },
            "content": "Hello from gateway",
            "message_reference": {
                "message_id": "msg_444"
            }
        });
        let raw = serde_json::to_vec(&body).unwrap();
        let msgs = adapter.parse_inbound(&raw).await.unwrap();

        assert_eq!(msgs.len(), 1);
        let m = &msgs[0];
        assert_eq!(m.platform_message_id, "msg_555");
        assert_eq!(m.conversation_id, "ch_666");
        assert_eq!(m.sender_platform_id, "author_777");
        assert_eq!(m.text.as_deref(), Some("Hello from gateway"));
        assert_eq!(m.reply_to_platform_message_id.as_deref(), Some("msg_444"));
    }

    #[tokio::test]
    async fn parse_unhandled_interaction_returns_empty() {
        let adapter = DiscordAdapter;
        // Type 5 = MODAL_SUBMIT -- not handled
        let body = serde_json::json!({ "type": 5, "data": {} });
        let raw = serde_json::to_vec(&body).unwrap();
        let msgs = adapter.parse_inbound(&raw).await.unwrap();
        assert!(msgs.is_empty());
    }

    #[tokio::test]
    async fn parse_invalid_json_returns_error() {
        let adapter = DiscordAdapter;
        let result = adapter.parse_inbound(b"not json").await;
        assert!(result.is_err());
    }

    // -- conversation_type mapping -------------------------------------------

    #[test]
    fn conversation_type_mapping() {
        assert_eq!(map_conversation_type(Some(CHANNEL_DM)), "private");
        assert_eq!(map_conversation_type(Some(CHANNEL_GROUP_DM)), "group");
        assert_eq!(map_conversation_type(Some(0)), "group"); // GUILD_TEXT
        assert_eq!(map_conversation_type(Some(5)), "group"); // GUILD_ANNOUNCEMENT
        assert_eq!(map_conversation_type(Some(15)), "group"); // GUILD_FORUM
        assert_eq!(map_conversation_type(None), "group");
        assert_eq!(map_conversation_type(Some(99)), "group"); // unknown
    }

    // -- verify_webhook (signature verification) -----------------------------

    #[tokio::test]
    async fn verify_webhook_valid_signature() {
        use ed25519_dalek::{Signer, SigningKey};

        let adapter = DiscordAdapter;

        // Generate a test key pair
        let signing_key = SigningKey::from_bytes(&[42u8; 32]);
        let verifying_key = signing_key.verifying_key();
        let public_key_hex = hex::encode(verifying_key.to_bytes());

        let timestamp = "1700000000";
        let body_content = b"{\"type\":2}";

        // Build the message to sign
        let mut message = Vec::new();
        message.extend_from_slice(timestamp.as_bytes());
        message.extend_from_slice(body_content);

        let signature = signing_key.sign(&message);
        let signature_hex = hex::encode(signature.to_bytes());

        let bot = make_test_bot(Some(&public_key_hex));
        let mut headers = axum::http::HeaderMap::new();
        headers.insert("x-signature-ed25519", signature_hex.parse().unwrap());
        headers.insert("x-signature-timestamp", timestamp.parse().unwrap());

        let result = adapter
            .verify_webhook(&bot, None, &headers, body_content)
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn verify_webhook_invalid_signature() {
        let adapter = DiscordAdapter;

        // Use a known public key but wrong signature
        let bot = make_test_bot(Some(&hex::encode([1u8; 32])));
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            "x-signature-ed25519",
            hex::encode([0u8; 64]).parse().unwrap(),
        );
        headers.insert("x-signature-timestamp", "12345".parse().unwrap());

        let result = adapter.verify_webhook(&bot, None, &headers, b"{}").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn verify_webhook_missing_public_key() {
        let adapter = DiscordAdapter;
        let bot = make_test_bot(None);
        let headers = axum::http::HeaderMap::new();

        let result = adapter.verify_webhook(&bot, None, &headers, b"{}").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn verify_webhook_missing_headers() {
        let adapter = DiscordAdapter;
        let bot = make_test_bot(Some(&hex::encode([1u8; 32])));
        let headers = axum::http::HeaderMap::new();

        let result = adapter.verify_webhook(&bot, None, &headers, b"{}").await;
        assert!(result.is_err());
    }

    // -- test helper ---------------------------------------------------------

    fn make_test_bot(public_key: Option<&str>) -> ChannelBot {
        ChannelBot {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: uuid::Uuid::new_v4().to_string(),
            platform: "discord".to_string(),
            label: "Test Discord Bot".to_string(),
            bot_token_encrypted: vec![0; 16],
            platform_bot_id: "bot_123".to_string(),
            platform_bot_username: "testbot".to_string(),
            webhook_registered: true,
            webhook_secret_hash: "unused_for_discord".to_string(),
            app_id: None,
            app_secret_encrypted: None,
            lark_verification_token_encrypted: None,
            lark_encrypt_key_encrypted: None,
            public_key: public_key.map(String::from),
            status: "active".to_string(),
            is_active: true,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    // -- slash command without options ---------------------------------------

    #[tokio::test]
    async fn parse_slash_command_no_options() {
        let adapter = DiscordAdapter;
        let body = serde_json::json!({
            "type": 2,
            "id": "int_no_opts",
            "channel_id": "ch_1",
            "channel": { "type": 0 },
            "member": {
                "user": { "id": "u1", "username": "User1" }
            },
            "data": { "name": "help" }
        });
        let raw = serde_json::to_vec(&body).unwrap();
        let msgs = adapter.parse_inbound(&raw).await.unwrap();

        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].text.as_deref(), Some("/help"));
    }
}

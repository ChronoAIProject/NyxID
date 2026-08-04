//! OpenClaw platform adapter for the Channel Bot Relay system.
//!
//! This is an intentionally thin adapter that exists so the `channel_bots` data
//! model can represent OpenClaw connections alongside native chat platform bots
//! (Telegram, Discord, Lark, Feishu).
//!
//! OpenClaw webhooks use per-mapping HMAC-SHA256 verification that is already
//! handled by `openclaw_channel_service`. The adapter methods are therefore
//! mostly no-ops -- the real message processing stays in the legacy handler at
//! `handlers/openclaw_channel.rs` until the full migration is complete.

use crate::errors::AppResult;
use crate::models::channel_bot::ChannelBot;
use crate::services::channel_platform::{
    BotIdentity, InboundMessage, OutboundReply, PlatformAdapter,
};
use crate::services::openclaw_channel_service::OpenClawChannelMessage;

/// OpenClaw platform adapter.
///
/// Stateless -- verification and routing are handled by the legacy
/// `openclaw_channel_service` path. This adapter provides forward
/// compatibility so OpenClaw bots can be registered in `channel_bots`.
pub struct OpenClawAdapter;

impl Default for OpenClawAdapter {
    fn default() -> Self {
        Self
    }
}

#[async_trait::async_trait]
impl PlatformAdapter for OpenClawAdapter {
    fn platform_id(&self) -> &str {
        "openclaw"
    }

    /// Webhook verification is handled by the legacy OpenClaw handler which
    /// performs per-mapping HMAC-SHA256 verification via
    /// `openclaw_channel_service::verify_webhook_for_mapping`. This is a no-op
    /// in the adapter because the existing handler already validates signatures
    /// before the relay path would be reached.
    async fn verify_webhook(
        &self,
        _bot: &ChannelBot,
        _secrets: Option<&crate::services::channel_platform::PlatformVerifySecrets>,
        _headers: &axum::http::HeaderMap,
        _body: &[u8],
    ) -> AppResult<()> {
        Ok(())
    }

    /// Parse an OpenClaw channel webhook payload into the normalized
    /// [`InboundMessage`] format.
    async fn parse_inbound(&self, body: &[u8]) -> AppResult<Vec<InboundMessage>> {
        let msg: OpenClawChannelMessage = serde_json::from_slice(body).map_err(|e| {
            crate::errors::AppError::BadRequest(format!(
                "invalid OpenClaw channel message JSON: {e}"
            ))
        })?;

        // Only process inbound (user -> agent) messages
        if msg.direction != "inbound" {
            return Ok(Vec::new());
        }

        let message_id = msg
            .session_key
            .clone()
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

        let inbound = InboundMessage {
            platform_message_id: message_id,
            conversation_id: msg.channel_user_id.clone(),
            conversation_type: "private".to_string(),
            sender_platform_id: msg.channel_user_id,
            sender_display_name: None,
            content_type: "text".to_string(),
            text: Some(msg.message),
            attachments: Vec::new(),
            reply_to_platform_message_id: None,
            thread_id: msg.agent_id,
            raw_data: serde_json::json!({
                "channel": msg.channel,
                "direction": msg.direction,
                "metadata": msg.metadata,
            }),
        };

        Ok(vec![inbound])
    }

    /// OpenClaw channel replies go through the existing webhook response or a
    /// separate callback mechanism. For now, this is a no-op -- the relay
    /// system will forward to the agent's callback URL which handles the reply
    /// externally.
    async fn send_reply(
        &self,
        _http: &reqwest::Client,
        _bot_token: &str,
        _conversation_id: &str,
        _reply: &OutboundReply,
    ) -> AppResult<Option<String>> {
        Ok(None)
    }

    /// OpenClaw webhooks are configured at the per-mapping level, not via a
    /// platform API. No registration step is needed.
    async fn register_webhook(
        &self,
        _http: &reqwest::Client,
        _bot_token: &str,
        _webhook_url: &str,
        _secret: &str,
    ) -> AppResult<()> {
        Ok(())
    }

    /// OpenClaw does not have a bot identity verification API. Return a
    /// placeholder identity derived from the platform context.
    async fn verify_bot_token(
        &self,
        _http: &reqwest::Client,
        _bot_token: &str,
    ) -> AppResult<BotIdentity> {
        Ok(BotIdentity {
            platform_bot_id: "openclaw".to_string(),
            platform_bot_username: "openclaw".to_string(),
        })
    }

    /// OpenClaw does not use a webhook challenge mechanism.
    fn handle_challenge(&self, _body: &[u8]) -> Option<serde_json::Value> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platform_id_is_openclaw() {
        let adapter = OpenClawAdapter;
        assert_eq!(adapter.platform_id(), "openclaw");
    }

    #[test]
    fn handle_challenge_returns_none() {
        let adapter = OpenClawAdapter;
        assert!(adapter.handle_challenge(b"{}").is_none());
        assert!(adapter.handle_challenge(b"").is_none());
    }

    #[tokio::test]
    async fn verify_webhook_always_succeeds() {
        let adapter = OpenClawAdapter;
        let bot = make_test_bot();
        let headers = axum::http::HeaderMap::new();
        let result = adapter.verify_webhook(&bot, None, &headers, b"").await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn parse_inbound_text_message() {
        let adapter = OpenClawAdapter;
        let body = serde_json::json!({
            "channel": "whatsapp",
            "channel_user_id": "user123",
            "agent_id": "agent_abc",
            "session_key": "sess_001",
            "message": "Hello from WhatsApp",
            "direction": "inbound",
            "metadata": { "phone": "+1234567890" }
        });
        let raw = serde_json::to_vec(&body).unwrap();
        let msgs = adapter.parse_inbound(&raw).await.unwrap();

        assert_eq!(msgs.len(), 1);
        let m = &msgs[0];
        assert_eq!(m.platform_message_id, "sess_001");
        assert_eq!(m.conversation_id, "user123");
        assert_eq!(m.conversation_type, "private");
        assert_eq!(m.sender_platform_id, "user123");
        assert_eq!(m.content_type, "text");
        assert_eq!(m.text.as_deref(), Some("Hello from WhatsApp"));
        assert_eq!(m.thread_id.as_deref(), Some("agent_abc"));
        assert!(m.attachments.is_empty());
    }

    #[tokio::test]
    async fn parse_inbound_outbound_direction_ignored() {
        let adapter = OpenClawAdapter;
        let body = serde_json::json!({
            "channel": "telegram",
            "channel_user_id": "user456",
            "message": "Reply from agent",
            "direction": "outbound"
        });
        let raw = serde_json::to_vec(&body).unwrap();
        let msgs = adapter.parse_inbound(&raw).await.unwrap();
        assert!(msgs.is_empty());
    }

    #[tokio::test]
    async fn parse_inbound_default_direction_is_inbound() {
        let adapter = OpenClawAdapter;
        let body = serde_json::json!({
            "channel": "discord",
            "channel_user_id": "user789",
            "message": "No direction field"
        });
        let raw = serde_json::to_vec(&body).unwrap();
        let msgs = adapter.parse_inbound(&raw).await.unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0].text.as_deref(), Some("No direction field"));
    }

    #[tokio::test]
    async fn parse_inbound_invalid_json() {
        let adapter = OpenClawAdapter;
        let result = adapter.parse_inbound(b"not json").await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn parse_inbound_generates_message_id_without_session_key() {
        let adapter = OpenClawAdapter;
        let body = serde_json::json!({
            "channel": "whatsapp",
            "channel_user_id": "user_no_session",
            "message": "No session key",
            "direction": "inbound"
        });
        let raw = serde_json::to_vec(&body).unwrap();
        let msgs = adapter.parse_inbound(&raw).await.unwrap();

        assert_eq!(msgs.len(), 1);
        // Should be a valid UUID v4 string (36 chars with hyphens)
        assert_eq!(msgs[0].platform_message_id.len(), 36);
    }

    #[tokio::test]
    async fn send_reply_returns_none() {
        let adapter = OpenClawAdapter;
        let http = reqwest::Client::new();
        let reply = OutboundReply {
            text: Some("test".to_string()),
            reply_to_platform_message_id: None,
            metadata: None,
        };
        let result = adapter.send_reply(&http, "", "conv_id", &reply).await;
        assert!(result.is_ok());
        assert!(result.unwrap().is_none());
    }

    #[tokio::test]
    async fn register_webhook_is_noop() {
        let adapter = OpenClawAdapter;
        let http = reqwest::Client::new();
        let result = adapter
            .register_webhook(&http, "", "https://example.com/hook", "secret")
            .await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn verify_bot_token_returns_placeholder() {
        let adapter = OpenClawAdapter;
        let http = reqwest::Client::new();
        let identity = adapter.verify_bot_token(&http, "any_token").await.unwrap();
        assert_eq!(identity.platform_bot_id, "openclaw");
        assert_eq!(identity.platform_bot_username, "openclaw");
    }

    fn make_test_bot() -> ChannelBot {
        ChannelBot {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: uuid::Uuid::new_v4().to_string(),
            platform: "openclaw".to_string(),
            label: "OpenClaw Bot".to_string(),
            bot_token_encrypted: vec![0; 16],
            platform_bot_id: "openclaw".to_string(),
            platform_bot_username: "openclaw".to_string(),
            webhook_registered: false,
            webhook_secret_hash: "placeholder".to_string(),
            app_id: None,
            app_secret_encrypted: None,
            lark_verification_token_encrypted: None,
            lark_encrypt_key_encrypted: None,
            public_key: None,
            status: "active".to_string(),
            is_active: true,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }
}

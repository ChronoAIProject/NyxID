use serde::{Deserialize, Serialize};

use crate::errors::AppResult;

/// Verified bot identity returned by the platform after token validation.
#[derive(Debug, Clone)]
pub struct BotIdentity {
    pub platform_bot_id: String,
    pub platform_bot_username: String,
}

/// A normalized inbound message parsed from any platform's webhook payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InboundMessage {
    pub platform_message_id: String,
    /// Platform-native conversation/chat identifier
    pub conversation_id: String,
    /// Conversation type: "private", "group", "channel"
    pub conversation_type: String,
    pub sender_platform_id: String,
    pub sender_display_name: Option<String>,
    /// Content category: "text", "image", "file", "audio", "video", "unknown"
    pub content_type: String,
    pub text: Option<String>,
    pub attachments: Vec<InboundAttachment>,
    /// Platform message ID that this message is a reply to (if threaded)
    pub reply_to_platform_message_id: Option<String>,
    /// Thread or topic identifier (platform-specific)
    pub thread_id: Option<String>,
    /// Raw webhook payload for auditing and debugging
    pub raw_data: serde_json::Value,
}

/// A file or media attachment on an inbound message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InboundAttachment {
    /// Content category: "image", "file", "audio", "video"
    pub content_type: String,
    /// Download URL (may require bot token to fetch)
    pub url: String,
    pub filename: Option<String>,
    pub mime_type: Option<String>,
    pub size_bytes: Option<u64>,
}

/// A reply to send back to the chat platform.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutboundReply {
    pub text: Option<String>,
    /// Platform message ID to reply to (for threading)
    pub reply_to_platform_message_id: Option<String>,
    /// Platform-specific metadata (e.g. parse mode, keyboard markup)
    pub metadata: Option<serde_json::Value>,
}

/// A previously-sent outbound message edit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutboundEdit {
    pub text: Option<String>,
    /// Platform-specific metadata for edit operations (e.g. Lark cards).
    pub metadata: Option<serde_json::Value>,
}

/// Decrypted, platform-specific webhook verification material prepared by the
/// handler. Secrets never live on persisted model structs.
#[derive(Clone, Default)]
pub struct PlatformVerifySecrets {
    pub slack_signing_secret: Option<String>,
    pub lark_verification_token: Option<String>,
    pub lark_encrypt_key: Option<String>,
}

impl std::fmt::Debug for PlatformVerifySecrets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlatformVerifySecrets")
            .field(
                "slack_signing_secret",
                &self.slack_signing_secret.as_ref().map(|_| "[REDACTED]"),
            )
            .field(
                "lark_verification_token",
                &self.lark_verification_token.as_ref().map(|_| "[REDACTED]"),
            )
            .field(
                "lark_encrypt_key",
                &self.lark_encrypt_key.as_ref().map(|_| "[REDACTED]"),
            )
            .finish()
    }
}

/// Webhook payload after platform-specific verification and preprocessing.
#[derive(Debug, Clone)]
pub struct PreparedWebhook {
    pub body: Vec<u8>,
    pub challenge_response: Option<serde_json::Value>,
}

/// Trait that each chat platform (Telegram, Discord, Lark, Feishu) implements
/// to normalize webhook verification, message parsing, and reply sending.
#[async_trait::async_trait]
pub trait PlatformAdapter: Send + Sync {
    /// Platform identifier (e.g. "telegram", "discord", "lark", "feishu").
    fn platform_id(&self) -> &str;

    /// Verify and preprocess the webhook payload before parsing. Adapters may
    /// return a platform challenge response or a transformed body (for example,
    /// an already-decrypted Lark event payload).
    async fn prepare_webhook(
        &self,
        bot: &crate::models::channel_bot::ChannelBot,
        secrets: Option<&PlatformVerifySecrets>,
        headers: &axum::http::HeaderMap,
        body: &[u8],
    ) -> AppResult<PreparedWebhook> {
        self.verify_webhook(bot, secrets, headers, body).await?;
        Ok(PreparedWebhook {
            body: body.to_vec(),
            challenge_response: None,
        })
    }

    /// Verify the incoming webhook signature or secret headers.
    async fn verify_webhook(
        &self,
        bot: &crate::models::channel_bot::ChannelBot,
        secrets: Option<&PlatformVerifySecrets>,
        headers: &axum::http::HeaderMap,
        body: &[u8],
    ) -> AppResult<()>;

    /// Parse the raw webhook body into zero or more normalized inbound messages.
    async fn parse_inbound(&self, body: &[u8]) -> AppResult<Vec<InboundMessage>>;

    /// Send a reply back to the platform conversation.
    /// Returns the platform-assigned message ID of the sent reply, if available.
    async fn send_reply(
        &self,
        http: &reqwest::Client,
        bot_token: &str,
        conversation_id: &str,
        reply: &OutboundReply,
    ) -> AppResult<Option<String>>;

    /// Edit a previously-sent platform message.
    async fn edit_reply(
        &self,
        _http: &reqwest::Client,
        _bot_token: &str,
        _platform_message_id: &str,
        _edit: &OutboundEdit,
    ) -> AppResult<()> {
        Err(crate::errors::AppError::ChannelPlatformEditUnsupported)
    }

    /// Register a webhook URL with the platform API.
    async fn register_webhook(
        &self,
        http: &reqwest::Client,
        bot_token: &str,
        webhook_url: &str,
        secret: &str,
    ) -> AppResult<()>;

    /// Validate the bot token and retrieve the bot's identity from the platform.
    async fn verify_bot_token(
        &self,
        http: &reqwest::Client,
        bot_token: &str,
    ) -> AppResult<BotIdentity>;

    /// Handle a platform-specific verification challenge (e.g. Discord PING,
    /// Lark url_verification). Returns `Some(response)` if this is a challenge
    /// request that should be answered immediately, `None` if it is a regular
    /// message webhook.
    fn handle_challenge(&self, _body: &[u8]) -> Option<serde_json::Value> {
        None
    }
}

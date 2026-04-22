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

/// Plaintext secrets that the handler has already decrypted from the
/// [`ChannelBot`](crate::models::channel_bot::ChannelBot) record and is
/// passing to the adapter for webhook verification.
///
/// Kept separate from `ChannelBot` so verification material never has to be
/// written back into the model struct in memory, and so each adapter only
/// sees the fields it actually needs.
#[derive(Default, Debug)]
pub struct WebhookSecrets {
    /// Slack: plaintext app signing secret used as the HMAC-SHA256 key for
    /// `X-Slack-Signature` verification.
    pub slack_signing_secret: Option<String>,
    /// Lark/Feishu: plaintext Event Subscription Verification Token. Lark
    /// places this value in every inbound body so the server can confirm
    /// the request came from Lark.
    pub lark_verification_token: Option<String>,
    /// Lark/Feishu: plaintext Event Subscription Encrypt Key. When set, Lark
    /// AES-256-CBC-encrypts the request body and signs it via
    /// `X-Lark-Signature`.
    pub lark_encrypt_key: Option<String>,
}

/// Trait that each chat platform (Telegram, Discord, Lark, Feishu) implements
/// to normalize webhook verification, message parsing, and reply sending.
#[async_trait::async_trait]
pub trait PlatformAdapter: Send + Sync {
    /// Platform identifier (e.g. "telegram", "discord", "lark", "feishu").
    fn platform_id(&self) -> &str;

    /// Verify the incoming webhook signature or secret headers.
    ///
    /// Returns `Some(plaintext_body)` if the adapter decrypted the body on
    /// the way through (e.g. Lark with Encrypt Key enabled) and the handler
    /// should feed that plaintext to [`parse_inbound`] instead of the raw
    /// body. Returns `None` when the input body should be used as-is.
    async fn verify_webhook(
        &self,
        bot: &crate::models::channel_bot::ChannelBot,
        secrets: &WebhookSecrets,
        headers: &axum::http::HeaderMap,
        body: &[u8],
    ) -> AppResult<Option<Vec<u8>>>;

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

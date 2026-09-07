use serde::{Deserialize, Serialize};

pub use super::channel_registration::{BotCredentials, RegistrationDescriptor, RegistrationValues};
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
    /// Platform-native message ID needed by providers such as Lark/Feishu to
    /// dereference opaque resource keys.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub platform_message_id: Option<String>,
    /// Provider file handle, when the platform exposes one directly.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_key: Option<String>,
    /// Provider image handle, when the platform exposes one directly.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub image_key: Option<String>,
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
    values: std::collections::BTreeMap<String, zeroize::Zeroizing<String>>,
}

impl PlatformVerifySecrets {
    pub fn insert(&mut self, name: &str, value: String) {
        self.values
            .insert(name.to_string(), zeroize::Zeroizing::new(value));
    }

    pub fn get(&self, name: &str) -> Option<&str> {
        self.values.get(name).map(|value| value.as_str())
    }
}

impl<const N: usize> From<[(&str, &str); N]> for PlatformVerifySecrets {
    fn from(values: [(&str, &str); N]) -> Self {
        let mut secrets = Self::default();
        for (name, value) in values {
            secrets.insert(name, value.to_string());
        }
        secrets
    }
}

impl std::fmt::Debug for PlatformVerifySecrets {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.values.is_empty() {
            return f
                .debug_struct("PlatformVerifySecrets")
                .field("values", &Option::<()>::None)
                .finish();
        }
        f.debug_map()
            .entries(self.values.keys().map(|key| (key, "[REDACTED]")))
            .finish()
    }
}

/// Webhook payload after platform-specific verification and preprocessing.
#[derive(Debug, Clone)]
pub struct PreparedWebhook {
    pub body: Vec<u8>,
    pub challenge_response: Option<serde_json::Value>,
}

/// Challenge-only responses perform no relay work. Immediate acknowledgments
/// dispatch processing in the background; inline adapters can verify challenges.
pub enum WebhookPolicy {
    Inline,
    Immediate(Option<serde_json::Value>),
    Challenge(serde_json::Value),
}

pub fn insert_reply_context(metadata: &mut Option<serde_json::Value>, key: &str, value: &str) {
    if let Some(object) = metadata
        .get_or_insert_with(|| serde_json::json!({}))
        .as_object_mut()
    {
        object
            .entry(key)
            .or_insert_with(|| serde_json::json!(value));
    }
}

/// Trait that each chat platform (Telegram, Discord, Lark, Feishu) implements
/// to normalize webhook verification, message parsing, and reply sending.
#[async_trait::async_trait]
pub trait PlatformAdapter: Send + Sync {
    /// Platform identifier (e.g. "telegram", "discord", "lark", "feishu").
    fn platform_id(&self) -> &str;

    fn registration(&self) -> RegistrationDescriptor {
        RegistrationDescriptor::default()
    }

    fn registration_token(
        &self,
        fields: &RegistrationValues<'_>,
    ) -> AppResult<zeroize::Zeroizing<String>> {
        Ok(zeroize::Zeroizing::new(
            fields.get("bot_token").unwrap_or_default().to_string(),
        ))
    }

    fn updated_token(
        &self,
        _current: &str,
        fields: &RegistrationValues<'_>,
    ) -> AppResult<Option<zeroize::Zeroizing<String>>> {
        Ok(fields
            .get("bot_token")
            .map(|token| zeroize::Zeroizing::new(token.to_string())))
    }

    fn updated_token_error(&self, error: crate::errors::AppError) -> crate::errors::AppError {
        error
    }

    fn validate_stored_verification(
        &self,
        _bot: &crate::models::channel_bot::ChannelBot,
    ) -> AppResult<()> {
        Ok(())
    }

    async fn build_verify_secrets(
        &self,
        keys: &crate::crypto::aes::EncryptionKeys,
        bot: &crate::models::channel_bot::ChannelBot,
    ) -> AppResult<PlatformVerifySecrets> {
        self.registration().verification_secrets(keys, bot).await
    }

    fn webhook_policy(&self, _body: &[u8]) -> WebhookPolicy {
        WebhookPolicy::Inline
    }

    /// Opt in only when repeated message IDs identify retries, not edits.
    fn dedup_inbound_by_platform_message_id(&self) -> bool {
        false
    }

    fn subscription_handshake(
        &self,
        _bot: &crate::models::channel_bot::ChannelBot,
        _query: &std::collections::HashMap<String, String>,
    ) -> AppResult<String> {
        Err(crate::errors::AppError::Forbidden(
            "Subscription handshake is not supported".to_string(),
        ))
    }

    fn reply_context(
        &self,
        thread_id: Option<&str>,
        created_at: chrono::DateTime<chrono::Utc>,
        metadata: &mut Option<serde_json::Value>,
    ) {
        super::channel_adapters::discord::apply_interaction_context(
            thread_id, created_at, metadata,
        );
    }

    fn supports_reply_metadata(&self, _metadata: &serde_json::Value) -> bool {
        false
    }

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
        credentials: &BotCredentials<'_>,
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
        credentials: &BotCredentials<'_>,
    ) -> AppResult<BotIdentity>;

    /// Handle a platform-specific verification challenge (e.g. Discord PING,
    /// Lark url_verification). Returns `Some(response)` if this is a challenge
    /// request that should be answered immediately, `None` if it is a regular
    /// message webhook.
    fn handle_challenge(&self, _body: &[u8]) -> Option<serde_json::Value> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn platform_verify_secrets_default_all_none() {
        let secrets = PlatformVerifySecrets::default();
        assert!(secrets.get("app_secret").is_none());
        assert!(secrets.get("verification_token").is_none());
        assert!(secrets.get("encrypt_key").is_none());
    }

    #[test]
    fn platform_verify_secrets_debug_redacts_slack_signing_secret() {
        let secrets = PlatformVerifySecrets::from([("app_secret", "super-secret-slack-value")]);
        let debug_output = format!("{:?}", secrets);
        assert!(
            !debug_output.contains("super-secret-slack-value"),
            "debug output must not contain the raw slack secret"
        );
        assert!(
            debug_output.contains("[REDACTED]"),
            "debug output must show [REDACTED]"
        );
    }

    #[test]
    fn platform_verify_secrets_debug_redacts_lark_verification_token() {
        let secrets = PlatformVerifySecrets::from([("verification_token", "lark-token-value")]);
        let debug_output = format!("{:?}", secrets);
        assert!(!debug_output.contains("lark-token-value"));
        assert!(debug_output.contains("[REDACTED]"));
    }

    #[test]
    fn platform_verify_secrets_debug_redacts_lark_encrypt_key() {
        let secrets = PlatformVerifySecrets::from([("encrypt_key", "encrypt-key-raw")]);
        let debug_output = format!("{:?}", secrets);
        assert!(!debug_output.contains("encrypt-key-raw"));
        assert!(debug_output.contains("[REDACTED]"));
    }

    #[test]
    fn platform_verify_secrets_debug_redacts_all_fields() {
        let secrets = PlatformVerifySecrets::from([
            ("app_secret", "s1"),
            ("verification_token", "s2"),
            ("encrypt_key", "s3"),
        ]);
        let debug_output = format!("{:?}", secrets);
        assert!(!debug_output.contains("s1"));
        assert!(!debug_output.contains("s2"));
        assert!(!debug_output.contains("s3"));
        assert_eq!(
            debug_output.matches("[REDACTED]").count(),
            3,
            "all three secrets should be redacted"
        );
    }

    #[test]
    fn platform_verify_secrets_debug_none_fields_show_none() {
        let secrets = PlatformVerifySecrets::default();
        let debug_output = format!("{:?}", secrets);
        assert!(
            debug_output.contains("None"),
            "None fields should display as None, got: {debug_output}"
        );
        assert!(
            !debug_output.contains("[REDACTED]"),
            "no [REDACTED] when all fields are None"
        );
    }

    #[test]
    fn outbound_reply_serde_roundtrip() {
        let reply = OutboundReply {
            text: Some("hello".to_string()),
            reply_to_platform_message_id: Some("msg-123".to_string()),
            metadata: Some(serde_json::json!({"parse_mode": "markdown"})),
        };
        let json = serde_json::to_string(&reply).expect("serialize");
        let restored: OutboundReply = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(restored.text.as_deref(), Some("hello"));
        assert_eq!(
            restored.reply_to_platform_message_id.as_deref(),
            Some("msg-123")
        );
        assert!(restored.metadata.is_some());
    }

    #[test]
    fn outbound_reply_serde_roundtrip_with_none_fields() {
        let reply = OutboundReply {
            text: None,
            reply_to_platform_message_id: None,
            metadata: None,
        };
        let json = serde_json::to_string(&reply).expect("serialize");
        let restored: OutboundReply = serde_json::from_str(&json).expect("deserialize");
        assert!(restored.text.is_none());
        assert!(restored.reply_to_platform_message_id.is_none());
        assert!(restored.metadata.is_none());
    }

    #[test]
    fn inbound_message_serde_roundtrip() {
        let msg = InboundMessage {
            platform_message_id: "pm-1".to_string(),
            conversation_id: "conv-1".to_string(),
            conversation_type: "private".to_string(),
            sender_platform_id: "user-42".to_string(),
            sender_display_name: Some("Alice".to_string()),
            content_type: "text".to_string(),
            text: Some("hi there".to_string()),
            attachments: vec![],
            reply_to_platform_message_id: None,
            thread_id: None,
            raw_data: serde_json::json!({}),
        };
        let json = serde_json::to_string(&msg).expect("serialize");
        let restored: InboundMessage = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(restored.platform_message_id, "pm-1");
        assert_eq!(restored.conversation_id, "conv-1");
        assert_eq!(restored.text.as_deref(), Some("hi there"));
    }

    #[test]
    fn inbound_attachment_serde_roundtrip() {
        let att = InboundAttachment {
            content_type: "image".to_string(),
            url: "https://cdn.example.com/img.png".to_string(),
            platform_message_id: None,
            file_key: None,
            image_key: None,
            filename: Some("img.png".to_string()),
            mime_type: Some("image/png".to_string()),
            size_bytes: Some(12345),
        };
        let json = serde_json::to_string(&att).expect("serialize");
        let restored: InboundAttachment = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(restored.content_type, "image");
        assert_eq!(restored.url, "https://cdn.example.com/img.png");
        assert_eq!(restored.filename.as_deref(), Some("img.png"));
        assert_eq!(restored.size_bytes, Some(12345));
    }

    #[test]
    fn inbound_message_with_attachments_roundtrip() {
        let msg = InboundMessage {
            platform_message_id: "pm-2".to_string(),
            conversation_id: "conv-2".to_string(),
            conversation_type: "group".to_string(),
            sender_platform_id: "user-99".to_string(),
            sender_display_name: None,
            content_type: "file".to_string(),
            text: None,
            attachments: vec![InboundAttachment {
                content_type: "file".to_string(),
                url: "https://files.example.com/doc.pdf".to_string(),
                platform_message_id: None,
                file_key: None,
                image_key: None,
                filename: Some("doc.pdf".to_string()),
                mime_type: Some("application/pdf".to_string()),
                size_bytes: Some(999_999),
            }],
            reply_to_platform_message_id: Some("pm-1".to_string()),
            thread_id: Some("thread-abc".to_string()),
            raw_data: serde_json::json!({"event_type": "message"}),
        };
        let json = serde_json::to_string(&msg).expect("serialize");
        let restored: InboundMessage = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(restored.attachments.len(), 1);
        assert_eq!(restored.attachments[0].filename.as_deref(), Some("doc.pdf"));
        assert_eq!(
            restored.reply_to_platform_message_id.as_deref(),
            Some("pm-1")
        );
        assert_eq!(restored.thread_id.as_deref(), Some("thread-abc"));
    }

    #[test]
    fn outbound_edit_serde_roundtrip() {
        let edit = OutboundEdit {
            text: Some("updated text".to_string()),
            metadata: Some(serde_json::json!({"card_id": "c1"})),
        };
        let json = serde_json::to_string(&edit).expect("serialize");
        let restored: OutboundEdit = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(restored.text.as_deref(), Some("updated text"));
        assert!(restored.metadata.is_some());
    }
}

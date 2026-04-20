use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "channel_bots";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChannelBot {
    #[serde(rename = "_id")]
    pub id: String,
    pub user_id: String,
    /// Platform identifier: "telegram", "discord", "lark", "feishu", "slack"
    pub platform: String,
    pub label: String,
    /// Encrypted bot token (AES-256 envelope encryption).
    /// For Slack this is the `xoxb-` bot user token.
    #[serde(with = "crate::models::bson_bytes::required")]
    pub bot_token_encrypted: Vec<u8>,
    /// Platform-assigned bot identifier
    pub platform_bot_id: String,
    /// Platform-assigned bot username or display handle
    pub platform_bot_username: String,
    /// Whether a webhook has been successfully registered with the platform
    pub webhook_registered: bool,
    /// SHA-256 hash of the webhook verification secret (Telegram); unused for
    /// platforms that derive the verifier from `app_secret_encrypted` instead
    /// (Lark/Feishu/Slack).
    pub webhook_secret_hash: String,
    /// Lark/Feishu only: application ID
    #[serde(default)]
    pub app_id: Option<String>,
    /// Encrypted app/signing secret (AES-256 envelope encryption).
    /// Lark/Feishu use it for HMAC + tenant-token exchange; Slack stores its
    /// app signing secret here for Events API signature verification.
    #[serde(default, with = "crate::models::bson_bytes::optional")]
    pub app_secret_encrypted: Option<Vec<u8>>,
    /// Discord only: application public key for Ed25519 signature verification
    #[serde(default)]
    pub public_key: Option<String>,
    /// Bot status: "pending", "active", "failed", "invalid"
    pub status: String,
    pub is_active: bool,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collection_name() {
        assert_eq!(COLLECTION_NAME, "channel_bots");
    }

    fn make_channel_bot() -> ChannelBot {
        ChannelBot {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: uuid::Uuid::new_v4().to_string(),
            platform: "telegram".to_string(),
            label: "My Bot".to_string(),
            bot_token_encrypted: vec![1, 2, 3, 4],
            platform_bot_id: "123456789".to_string(),
            platform_bot_username: "mybot".to_string(),
            webhook_registered: false,
            webhook_secret_hash: "abc123".to_string(),
            app_id: None,
            app_secret_encrypted: None,
            public_key: None,
            status: "pending".to_string(),
            is_active: true,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn bson_roundtrip() {
        let bot = make_channel_bot();
        let doc = bson::to_document(&bot).expect("serialize");
        let restored: ChannelBot = bson::from_document(doc).expect("deserialize");
        assert_eq!(bot.id, restored.id);
        assert_eq!(bot.platform, restored.platform);
        assert_eq!(bot.label, restored.label);
        assert_eq!(bot.platform_bot_id, restored.platform_bot_id);
        assert_eq!(bot.status, restored.status);
    }

    #[test]
    fn bson_roundtrip_with_optional_fields() {
        let mut bot = make_channel_bot();
        bot.platform = "lark".to_string();
        bot.app_id = Some("cli_abc123".to_string());
        bot.app_secret_encrypted = Some(vec![10, 20, 30]);
        bot.public_key = Some("ed25519pubkey".to_string());
        let doc = bson::to_document(&bot).expect("serialize");
        let restored: ChannelBot = bson::from_document(doc).expect("deserialize");
        assert_eq!(restored.app_id.as_deref(), Some("cli_abc123"));
        assert_eq!(restored.app_secret_encrypted, Some(vec![10, 20, 30]));
        assert_eq!(restored.public_key.as_deref(), Some("ed25519pubkey"));
    }

    #[test]
    fn bson_all_fields_serialized() {
        let bot = make_channel_bot();
        let doc = bson::to_document(&bot).expect("serialize");
        assert!(doc.contains_key("_id"));
        assert!(doc.contains_key("user_id"));
        assert!(doc.contains_key("platform"));
        assert!(doc.contains_key("label"));
        assert!(doc.contains_key("bot_token_encrypted"));
        assert!(doc.contains_key("platform_bot_id"));
        assert!(doc.contains_key("platform_bot_username"));
        assert!(doc.contains_key("webhook_registered"));
        assert!(doc.contains_key("webhook_secret_hash"));
        assert!(doc.contains_key("status"));
        assert!(doc.contains_key("is_active"));
        assert!(doc.contains_key("created_at"));
        assert!(doc.contains_key("updated_at"));
    }

    #[test]
    fn bson_backward_compat_missing_optional_fields() {
        let bot = make_channel_bot();
        let mut doc = bson::to_document(&bot).expect("serialize");
        doc.remove("app_id");
        doc.remove("app_secret_encrypted");
        doc.remove("public_key");
        let restored: ChannelBot = bson::from_document(doc).expect("deserialize");
        assert_eq!(restored.app_id, None);
        assert_eq!(restored.app_secret_encrypted, None);
        assert_eq!(restored.public_key, None);
    }
}

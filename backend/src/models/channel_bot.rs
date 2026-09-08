use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "channel_bots";

#[derive(Clone, Serialize, Deserialize)]
pub struct ChannelBot {
    #[serde(rename = "_id")]
    pub id: String,
    pub user_id: String,
    /// Platform identifier: "telegram", "discord", "lark", "feishu", "slack", "whatsapp"
    pub platform: String,
    pub label: String,
    #[serde(default = "default_credential_source")]
    pub credential_source: String,
    #[serde(default, with = "crate::models::bson_bytes::optional")]
    pub registration_pin_encrypted: Option<Vec<u8>>,
    /// Managed-only copy of the generated verify token for repeatable setup.
    #[serde(default, with = "crate::models::bson_bytes::optional")]
    pub webhook_secret_encrypted: Option<Vec<u8>>,
    #[serde(default)]
    pub managed_setup: Option<ManagedBotSetup>,
    /// Encrypted bot token (AES-256 envelope encryption).
    /// For Slack this is the `xoxb-` bot user token.
    #[serde(with = "crate::models::bson_bytes::required")]
    pub bot_token_encrypted: Vec<u8>,
    /// Platform-assigned bot identifier; WhatsApp's Phone Number ID.
    pub platform_bot_id: String,
    /// Platform-assigned bot username or display handle
    pub platform_bot_username: String,
    /// Whether a webhook has been successfully registered with the platform
    pub webhook_registered: bool,
    /// SHA-256 hash of the generated webhook secret (Telegram secret header,
    /// WhatsApp GET subscription Verify Token). Never contains a raw secret.
    pub webhook_secret_hash: String,
    /// Platform account/application identifier: Lark/Feishu App ID or optional
    /// WhatsApp Business Account ID (WABA). Not WhatsApp's Phone Number ID.
    #[serde(default)]
    pub app_id: Option<String>,
    /// Encrypted app/signing secret (AES-256 envelope encryption).
    /// Lark/Feishu use it for tenant-token exchange; Slack and WhatsApp store
    /// their app secrets here for inbound HMAC verification.
    #[serde(default, with = "crate::models::bson_bytes::optional")]
    pub app_secret_encrypted: Option<Vec<u8>>,
    /// Lark/Feishu only: encrypted verification token from the Event
    /// Subscriptions console.
    #[serde(default, with = "crate::models::bson_bytes::optional")]
    pub lark_verification_token_encrypted: Option<Vec<u8>>,
    /// Lark/Feishu only: encrypted optional Encrypt Key from the Event
    /// Subscriptions console.
    #[serde(default, with = "crate::models::bson_bytes::optional")]
    pub lark_encrypt_key_encrypted: Option<Vec<u8>>,
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

fn default_credential_source() -> String {
    "user".to_string()
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct ManagedBotSetup {
    pub subscription: String,
    pub webhook_override: String,
    pub registration: String,
    #[serde(default)]
    pub business_id: Option<String>,
    #[serde(default)]
    pub coexistence: bool,
    #[serde(default)]
    pub coexistence_sync: std::collections::BTreeMap<String, String>,
}

impl std::fmt::Debug for ChannelBot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChannelBot")
            .field("id", &self.id)
            .field("platform", &self.platform)
            .field("credentials", &"[REDACTED]")
            .finish_non_exhaustive()
    }
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
            credential_source: "user".to_string(),
            registration_pin_encrypted: None,
            webhook_secret_encrypted: None,
            managed_setup: None,
            bot_token_encrypted: vec![1, 2, 3, 4],
            platform_bot_id: "123456789".to_string(),
            platform_bot_username: "mybot".to_string(),
            webhook_registered: false,
            webhook_secret_hash: "abc123".to_string(),
            app_id: None,
            app_secret_encrypted: None,
            lark_verification_token_encrypted: None,
            lark_encrypt_key_encrypted: None,
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
        bot.lark_verification_token_encrypted = Some(vec![40, 50, 60]);
        bot.lark_encrypt_key_encrypted = Some(vec![70, 80, 90]);
        bot.public_key = Some("ed25519pubkey".to_string());
        let doc = bson::to_document(&bot).expect("serialize");
        let restored: ChannelBot = bson::from_document(doc).expect("deserialize");
        assert_eq!(restored.app_id.as_deref(), Some("cli_abc123"));
        assert_eq!(restored.app_secret_encrypted, Some(vec![10, 20, 30]));
        assert_eq!(
            restored.lark_verification_token_encrypted,
            Some(vec![40, 50, 60])
        );
        assert_eq!(restored.lark_encrypt_key_encrypted, Some(vec![70, 80, 90]));
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
        doc.remove("lark_verification_token_encrypted");
        doc.remove("lark_encrypt_key_encrypted");
        doc.remove("public_key");
        let restored: ChannelBot = bson::from_document(doc).expect("deserialize");
        assert_eq!(restored.app_id, None);
        assert_eq!(restored.app_secret_encrypted, None);
        assert_eq!(restored.lark_verification_token_encrypted, None);
        assert_eq!(restored.lark_encrypt_key_encrypted, None);
        assert_eq!(restored.public_key, None);
    }

    #[test]
    fn whatsapp_storage_roundtrip_uses_existing_optional_columns() {
        let mut bot = make_channel_bot();
        bot.platform = "whatsapp".to_string();
        bot.platform_bot_id = "123456".to_string();
        bot.app_id = Some("654321".to_string());
        bot.app_secret_encrypted = Some(vec![7, 8, 9]);
        let document = bson::to_document(&bot).unwrap();
        assert_eq!(document.get_str("platform_bot_id").unwrap(), "123456");
        assert_eq!(document.get_str("app_id").unwrap(), "654321");
        assert_eq!(
            document.get_binary_generic("app_secret_encrypted").unwrap(),
            &[7, 8, 9]
        );
        let restored: ChannelBot = bson::from_document(document).unwrap();
        assert_eq!(restored.app_secret_encrypted, bot.app_secret_encrypted);
        assert_eq!(restored.platform_bot_id, bot.platform_bot_id);
    }

    #[test]
    fn managed_fields_are_optional_and_pin_is_bson_binary() {
        let mut bot = make_channel_bot();
        let mut old = bson::to_document(&bot).unwrap();
        for field in [
            "credential_source",
            "registration_pin_encrypted",
            "webhook_secret_encrypted",
            "managed_setup",
        ] {
            old.remove(field);
        }
        let legacy: ChannelBot = bson::from_document(old).unwrap();
        assert_eq!(legacy.credential_source, "user");
        assert!(legacy.registration_pin_encrypted.is_none());
        assert!(legacy.webhook_secret_encrypted.is_none());
        assert!(legacy.managed_setup.is_none());
        bot.credential_source = "platform".to_string();
        bot.registration_pin_encrypted = Some(vec![9, 8, 7]);
        bot.webhook_secret_encrypted = Some(vec![6, 5, 4]);
        bot.managed_setup = Some(ManagedBotSetup {
            registration: "registered".to_string(),
            ..Default::default()
        });
        let stored = bson::to_document(&bot).unwrap();
        assert_eq!(
            stored
                .get_binary_generic("registration_pin_encrypted")
                .unwrap(),
            &[9, 8, 7]
        );
        let restored: ChannelBot = bson::from_document(stored).unwrap();
        assert_eq!(restored.webhook_secret_encrypted, Some(vec![6, 5, 4]));
        assert_eq!(restored.managed_setup.unwrap().registration, "registered");
        assert!(!format!("{bot:?}").contains("9, 8, 7"));
    }
}

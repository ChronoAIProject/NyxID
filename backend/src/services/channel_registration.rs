//! Adapter-owned registration fields mapped onto the existing bot document.

use std::collections::BTreeMap;

use zeroize::Zeroizing;

use crate::crypto::aes::EncryptionKeys;
use crate::errors::{AppError, AppResult};
use crate::models::channel_bot::ChannelBot;

#[derive(Clone, Copy, Debug)]
pub struct RegistrationField {
    pub name: &'static str,
    pub label: &'static str,
    pub storage: &'static str,
    pub secret: bool,
    pub required: bool,
    pub patchable: bool,
    pub clearable: bool,
    pub webhook_secret: bool,
    pub platform_fallback: Option<&'static str>,
}

pub const BOT_TOKEN_FIELD: RegistrationField = RegistrationField {
    name: "bot_token",
    label: "Bot token",
    storage: "bot_token_encrypted",
    secret: true,
    required: true,
    patchable: false,
    clearable: false,
    webhook_secret: false,
    platform_fallback: None,
};

// These fields were accepted and stored by every pre-existing platform's
// create endpoint, even when irrelevant to that platform's verification.
const LEGACY_OPTIONAL_FIELDS: &[RegistrationField] = &[
    RegistrationField {
        name: "app_id",
        label: "App ID",
        storage: "app_id",
        secret: false,
        required: false,
        patchable: false,
        clearable: false,
        webhook_secret: false,
        platform_fallback: None,
    },
    RegistrationField {
        name: "app_secret",
        label: "App Secret",
        storage: "app_secret_encrypted",
        secret: true,
        required: false,
        patchable: false,
        clearable: false,
        webhook_secret: false,
        platform_fallback: None,
    },
    RegistrationField {
        name: "public_key",
        label: "Public Key",
        storage: "public_key",
        secret: false,
        required: false,
        patchable: false,
        clearable: false,
        webhook_secret: false,
        platform_fallback: None,
    },
    RegistrationField {
        name: "verification_token",
        label: "Verification Token",
        storage: "lark_verification_token_encrypted",
        secret: true,
        required: false,
        patchable: false,
        clearable: false,
        webhook_secret: false,
        platform_fallback: None,
    },
    RegistrationField {
        name: "encrypt_key",
        label: "Encrypt Key",
        storage: "lark_encrypt_key_encrypted",
        secret: true,
        required: false,
        patchable: false,
        clearable: true,
        webhook_secret: false,
        platform_fallback: None,
    },
];

#[derive(Clone, Copy, Debug)]
pub struct RegistrationDescriptor {
    pub fields: &'static [RegistrationField],
    pub token_fields: &'static [&'static str],
    pub extra_fields: &'static [RegistrationField],
    pub required_suffix: &'static str,
    pub unsupported_patch_message: Option<&'static str>,
    pub create_response_status: &'static str,
    pub preserve_subscription_on_verify: bool,
    pub empty_ack_is_text: bool,
    pub enabled: bool,
    pub automatic_webhook: bool,
    pub webhook_ingestion: bool,
    pub managed_only: bool,
    pub managed_only_message: &'static str,
    pub webhook_secret_label: Option<&'static str>,
    pub setup_instructions: &'static [&'static str],
}

impl Default for RegistrationDescriptor {
    fn default() -> Self {
        Self {
            fields: &[BOT_TOKEN_FIELD],
            token_fields: &["bot_token"],
            extra_fields: LEGACY_OPTIONAL_FIELDS,
            required_suffix: "",
            unsupported_patch_message: Some(
                "Only label updates are supported for this bot platform",
            ),
            create_response_status: "active",
            preserve_subscription_on_verify: false,
            empty_ack_is_text: true,
            enabled: true,
            automatic_webhook: false,
            webhook_ingestion: true,
            managed_only: false,
            managed_only_message: "This platform requires managed onboarding",
            webhook_secret_label: None,
            setup_instructions: &[],
        }
    }
}

/// Borrowed, normalized inputs. An empty PATCH value means clear only for
/// fields whose descriptor explicitly permits clearing.
#[derive(Default)]
pub struct RegistrationValues<'a>(pub BTreeMap<&'static str, &'a str>);

impl std::fmt::Debug for RegistrationValues<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_map()
            .entries(self.0.keys().map(|key| (key, "[REDACTED]")))
            .finish()
    }
}

impl<'a> RegistrationValues<'a> {
    pub fn get(&self, name: &str) -> Option<&'a str> {
        self.0.get(name).copied()
    }
}

impl RegistrationDescriptor {
    pub fn all_fields(&self) -> impl Iterator<Item = &RegistrationField> {
        self.fields.iter().chain(
            self.extra_fields
                .iter()
                .filter(|extra| !self.fields.iter().any(|field| field.name == extra.name)),
        )
    }
    pub fn identity<'a>(&self, fields: &RegistrationValues<'a>) -> Option<&'a str> {
        self.fields
            .iter()
            .find(|field| field.storage == "platform_bot_id")
            .and_then(|field| fields.get(field.name))
    }

    pub fn validate(&self, values: &RegistrationValues<'_>, patch: bool) -> AppResult<()> {
        if !self.enabled {
            return Err(AppError::ValidationError(
                "This platform uses a separate integration path".to_string(),
            ));
        }
        for name in values.0.keys() {
            let field = self.all_fields().find(|field| field.name == *name);
            if field.is_none_or(|field| patch && !field.patchable) {
                if patch && let Some(message) = self.unsupported_patch_message {
                    return Err(AppError::ValidationError(message.to_string()));
                }
                return Err(AppError::ValidationError(format!(
                    "{name} is not supported for this platform{}",
                    if patch { " update" } else { "" }
                )));
            }
        }
        for field in self.fields {
            match values.get(field.name) {
                None if !patch && field.required => {
                    return Err(AppError::ValidationError(format!(
                        "{} is required{}",
                        field.label,
                        if field.name == "bot_token" {
                            ""
                        } else {
                            self.required_suffix
                        }
                    )));
                }
                Some(value) if value.trim().is_empty() && !(patch && field.clearable) => {
                    return Err(AppError::ValidationError(format!(
                        "{} cannot be blank",
                        field.label
                    )));
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Only nonsecret adapter fields are exposed through management responses.
    pub fn configuration(&self, bot: &ChannelBot) -> AppResult<BTreeMap<String, String>> {
        let document = bson::to_document(bot)
            .map_err(|_| AppError::Internal("Unable to serialize channel bot".to_string()))?;
        Ok(self
            .fields
            .iter()
            .filter(|field| !field.secret)
            .filter_map(|field| {
                document
                    .get_str(field.storage)
                    .ok()
                    .map(|value| (field.name.to_string(), value.to_string()))
            })
            .collect())
    }

    pub async fn verification_secrets(
        &self,
        keys: &EncryptionKeys,
        bot: &ChannelBot,
    ) -> AppResult<super::channel_platform::PlatformVerifySecrets> {
        let document = bson::to_document(bot)
            .map_err(|_| AppError::Internal("Unable to serialize channel bot".to_string()))?;
        let mut secrets = super::channel_platform::PlatformVerifySecrets::default();
        for field in self.fields.iter().filter(|field| field.webhook_secret) {
            if let Ok(encrypted) = document.get_binary_generic(field.storage) {
                let bytes = Zeroizing::new(keys.decrypt(encrypted).await?);
                let value = std::str::from_utf8(&bytes).map_err(|_| {
                    AppError::Internal("invalid webhook secret encoding".to_string())
                })?;
                secrets.insert(field.name, value.to_string());
            }
        }
        Ok(secrets)
    }
}

/// Outbound authentication and identity are separate, never a new composite token.
pub struct BotCredentials<'a> {
    pub token: &'a str,
    pub platform_bot_id: Option<&'a str>,
    pub platform_secrets: Option<&'a super::channel_platform::PlatformVerifySecrets>,
}

impl std::fmt::Debug for BotCredentials<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BotCredentials")
            .field("token", &"[REDACTED]")
            .field("platform_bot_id", &self.platform_bot_id)
            .finish()
    }
}

impl<'a> From<&'a str> for BotCredentials<'a> {
    fn from(token: &'a str) -> Self {
        Self {
            token,
            platform_bot_id: None,
            platform_secrets: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::channel_adapters::resolve_adapter;
    use crate::services::channel_platform::WebhookPolicy;
    use crate::services::provider_token_exchange_service::TokenExchangeCache;
    use std::sync::Arc;

    #[test]
    fn channel_retry_dedup_is_opt_in_for_whatsapp_only() {
        let cache = Arc::new(TokenExchangeCache::new());
        for platform in [
            "telegram", "discord", "lark", "feishu", "slack", "openclaw", "whatsapp",
        ] {
            assert_eq!(
                resolve_adapter(platform, &cache)
                    .unwrap()
                    .dedup_inbound_by_platform_message_id(),
                platform == "whatsapp"
            );
        }
    }

    #[test]
    fn channel_registration_preserves_exact_legacy_validation_messages() {
        let cache = Arc::new(TokenExchangeCache::new());
        for (platform, field, expected) in [
            (
                "lark",
                "verification_token",
                "Verification Token is required for Lark/Feishu",
            ),
            ("lark", "app_id", "App ID is required for Lark/Feishu"),
            (
                "lark",
                "app_secret",
                "App Secret is required for Lark/Feishu",
            ),
            (
                "feishu",
                "verification_token",
                "Verification Token is required for Lark/Feishu",
            ),
            ("feishu", "app_id", "App ID is required for Lark/Feishu"),
            (
                "feishu",
                "app_secret",
                "App Secret is required for Lark/Feishu",
            ),
            (
                "discord",
                "public_key",
                "Public Key is required for Discord",
            ),
            (
                "slack",
                "app_secret",
                "Signing Secret is required for Slack",
            ),
            ("telegram", "bot_token", "Bot token is required"),
        ] {
            let descriptor = resolve_adapter(platform, &cache).unwrap().registration();
            let mut fields = RegistrationValues(
                [
                    ("bot_token", "token"),
                    ("app_id", "id"),
                    ("app_secret", "secret"),
                    ("verification_token", "verification"),
                    ("public_key", "public"),
                ]
                .into(),
            );
            fields.0.remove(field);
            let AppError::ValidationError(message) =
                descriptor.validate(&fields, false).unwrap_err()
            else {
                panic!("expected validation error")
            };
            assert_eq!(message, expected);
        }
        for (platform, field, expected) in [
            (
                "telegram",
                "app_secret",
                "Only label updates are supported for this bot platform",
            ),
            (
                "slack",
                "verification_token",
                "verification_token, encrypt_key, and app_id are only supported for Lark/Feishu bots",
            ),
            (
                "slack",
                "encrypt_key",
                "verification_token, encrypt_key, and app_id are only supported for Lark/Feishu bots",
            ),
            (
                "slack",
                "app_id",
                "verification_token, encrypt_key, and app_id are only supported for Lark/Feishu bots",
            ),
        ] {
            let descriptor = resolve_adapter(platform, &cache).unwrap().registration();
            let AppError::ValidationError(message) = descriptor
                .validate(&RegistrationValues([(field, "value")].into()), true)
                .unwrap_err()
            else {
                panic!("expected validation error")
            };
            assert_eq!(message, expected);
        }
    }

    #[test]
    fn channel_registration_requirements_and_patch_policies_are_adapter_owned() {
        let cache = Arc::new(TokenExchangeCache::new());
        for (platform, required) in [
            ("telegram", vec!["bot_token"]),
            ("discord", vec!["bot_token", "public_key"]),
            (
                "lark",
                vec!["bot_token", "app_id", "app_secret", "verification_token"],
            ),
            (
                "feishu",
                vec!["bot_token", "app_id", "app_secret", "verification_token"],
            ),
            ("slack", vec!["bot_token", "app_secret"]),
            (
                "whatsapp",
                vec!["bot_token", "phone_number_id", "app_secret"],
            ),
        ] {
            let adapter = resolve_adapter(platform, &cache).unwrap();
            let descriptor = adapter.registration();
            let values =
                RegistrationValues(required.iter().map(|field| (*field, "12345")).collect());
            descriptor.validate(&values, false).unwrap();
            for field in &required {
                let mut missing = RegistrationValues(values.0.clone());
                missing.0.remove(field);
                assert!(
                    descriptor.validate(&missing, false).is_err(),
                    "{platform}: {field}"
                );
                missing.0.insert(field, " ");
                assert!(descriptor.validate(&missing, false).is_err());
            }
            for field in descriptor.fields {
                let patch = RegistrationValues([(field.name, "value")].into());
                assert_eq!(descriptor.validate(&patch, true).is_ok(), field.patchable);
                let clear = RegistrationValues([(field.name, "")].into());
                assert_eq!(
                    descriptor.validate(&clear, true).is_ok(),
                    field.patchable && field.clearable
                );
            }
            descriptor
                .validate(&RegistrationValues::default(), true)
                .unwrap();
        }
        assert!(
            !resolve_adapter("openclaw", &cache)
                .unwrap()
                .registration()
                .enabled
        );
    }

    #[test]
    fn channel_webhook_policies_preserve_existing_challenges_and_acknowledgments() {
        let cache = Arc::new(TokenExchangeCache::new());
        assert!(matches!(
            resolve_adapter("telegram", &cache)
                .unwrap()
                .webhook_policy(b"{}"),
            WebhookPolicy::Inline
        ));
        for platform in ["lark", "feishu"] {
            assert!(matches!(
                resolve_adapter(platform, &cache)
                    .unwrap()
                    .webhook_policy(br#"{"type":"url_verification","challenge":"x"}"#),
                WebhookPolicy::Inline
            ));
        }
        assert!(matches!(
            resolve_adapter("slack", &cache)
                .unwrap()
                .webhook_policy(b"{}"),
            WebhookPolicy::Immediate(None)
        ));
        assert!(matches!(
            resolve_adapter("slack", &cache)
                .unwrap()
                .webhook_policy(br#"{"type":"url_verification","challenge":"x"}"#),
            WebhookPolicy::Challenge(_)
        ));
        let discord = resolve_adapter("discord", &cache).unwrap();
        assert!(matches!(
            discord.webhook_policy(br#"{"type":1}"#),
            WebhookPolicy::Challenge(_)
        ));
        assert!(matches!(
            discord.webhook_policy(br#"{"type":2}"#),
            WebhookPolicy::Immediate(Some(_))
        ));
        assert!(matches!(
            discord.webhook_policy(br#"{"type":4}"#),
            WebhookPolicy::Immediate(Some(_))
        ));
    }

    #[test]
    fn channel_registration_preserves_legacy_optional_inputs_and_lifecycle() {
        let cache = Arc::new(TokenExchangeCache::new());
        for platform in ["telegram", "discord", "lark", "feishu", "slack"] {
            let descriptor = resolve_adapter(platform, &cache).unwrap().registration();
            assert_eq!(descriptor.create_response_status, "active");
            assert!(!descriptor.preserve_subscription_on_verify);
            assert_eq!(descriptor.automatic_webhook, platform == "telegram");
            let fields = RegistrationValues(
                [
                    ("bot_token", "token"),
                    ("app_id", "app"),
                    ("app_secret", "secret"),
                    ("verification_token", "verification"),
                    ("encrypt_key", "encrypt"),
                    ("public_key", "public"),
                ]
                .into(),
            );
            descriptor.validate(&fields, false).unwrap();
            for name in fields.0.keys() {
                assert!(descriptor.all_fields().any(|field| field.name == *name));
            }
        }
    }

    #[test]
    fn channel_legacy_interaction_context_takes_precedence_on_every_existing_platform() {
        let cache = Arc::new(TokenExchangeCache::new());
        for platform in ["telegram", "discord", "lark", "feishu", "slack", "openclaw"] {
            let adapter = resolve_adapter(platform, &cache).unwrap();
            let mut metadata = None;
            adapter.reply_context(
                Some("interaction:app:token"),
                chrono::Utc::now(),
                &mut metadata,
            );
            assert_eq!(
                metadata.unwrap(),
                serde_json::json!({"interaction_thread_id": "interaction:app:token"})
            );
            let mut expired = None;
            adapter.reply_context(
                Some("interaction:app:token"),
                chrono::Utc::now() - chrono::Duration::minutes(15),
                &mut expired,
            );
            assert!(expired.is_none());
        }
    }

    #[test]
    fn channel_reply_context_keeps_thread_roots_overrides_and_discord_expiry() {
        let cache = Arc::new(TokenExchangeCache::new());
        for (platform, key, thread) in [
            ("telegram", "message_thread_id", "123"),
            ("slack", "thread_ts", "123.456"),
            ("discord", "interaction_thread_id", "interaction:app:token"),
        ] {
            let adapter = resolve_adapter(platform, &cache).unwrap();
            let mut metadata = None;
            adapter.reply_context(Some(thread), chrono::Utc::now(), &mut metadata);
            assert_eq!(metadata.as_ref().unwrap()[key], thread);
            let mut existing = Some(serde_json::json!({key: "explicit"}));
            adapter.reply_context(Some(thread), chrono::Utc::now(), &mut existing);
            assert_eq!(existing.unwrap()[key], "explicit");
        }
        let mut expired = None;
        resolve_adapter("discord", &cache).unwrap().reply_context(
            Some("interaction:app:token"),
            chrono::Utc::now() - chrono::Duration::minutes(15),
            &mut expired,
        );
        assert!(expired.is_none());
    }

    #[tokio::test]
    async fn channel_secrets_decrypt_only_descriptor_fields_and_configuration_never_exposes_secrets()
     {
        let keys = EncryptionKeys::with_provider(Arc::new(
            crate::crypto::local_key_provider::LocalKeyProvider::new([0x11; 32], None),
        ));
        let encrypted = keys.encrypt(b"meta-secret").await.unwrap();
        let bot: ChannelBot = bson::from_document(bson::doc! {
            "_id": "bot", "user_id": "user", "platform": "whatsapp", "label": "Support",
            "platform_bot_id": "123456", "platform_bot_username": "+123456", "app_id": "654321",
            "bot_token_encrypted": bson::Binary { subtype: bson::spec::BinarySubtype::Generic, bytes: vec![1] },
            "app_secret_encrypted": bson::Binary { subtype: bson::spec::BinarySubtype::Generic, bytes: encrypted },
            "lark_encrypt_key_encrypted": bson::Binary { subtype: bson::spec::BinarySubtype::Generic, bytes: vec![0] },
            "webhook_secret_hash": "hash", "webhook_registered": false, "status": "pending_webhook", "is_active": true,
            "created_at": bson::DateTime::now(), "updated_at": bson::DateTime::now(),
        }).unwrap();
        let cache = Arc::new(TokenExchangeCache::new());
        let adapter = resolve_adapter("whatsapp", &cache).unwrap();
        let secrets = adapter.build_verify_secrets(&keys, &bot).await.unwrap();
        assert_eq!(secrets.get("app_secret"), Some("meta-secret"));
        assert!(secrets.get("encrypt_key").is_none());
        assert!(!format!("{secrets:?}").contains("meta-secret"));
        let configuration = adapter.registration().configuration(&bot).unwrap();
        assert_eq!(
            configuration,
            [
                ("phone_number_id".to_string(), "123456".to_string()),
                ("waba_id".to_string(), "654321".to_string())
            ]
            .into()
        );
    }
}

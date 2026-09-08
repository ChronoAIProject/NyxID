//! Adapter contracts for platform-owned credentials and hosted onboarding.

use std::collections::BTreeMap;

use serde::Serialize;
use zeroize::Zeroizing;

use super::channel_platform::{BotIdentity, PlatformAdapter, PlatformVerifySecrets};
use crate::errors::{AppError, AppResult};

#[derive(Clone, Copy, Debug, Serialize)]
pub struct PlatformCredentialField {
    pub name: &'static str,
    pub label: &'static str,
    pub secret: bool,
    pub help: &'static str,
    pub required: bool,
    pub numeric: bool,
}

#[derive(Clone, Copy, Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum PlatformCredentialBacking {
    Stored,
    #[serde(rename = "provider_oauth")]
    ProviderOAuth {
        provider_slug: &'static str,
    },
}

#[derive(Clone, Copy, Debug, Serialize)]
pub struct PlatformCredentialDescriptor {
    pub provider: &'static str,
    pub label: &'static str,
    pub backing: PlatformCredentialBacking,
    pub fields: &'static [PlatformCredentialField],
    pub setup_checklist: &'static [&'static str],
    /// Saving this field creates the platform handshake token if absent.
    #[serde(skip)]
    pub webhook_secret_field: Option<&'static str>,
}

#[derive(Clone, Copy, Debug)]
pub struct ManagedOnboardingDescriptor {
    pub flow: &'static str,
    pub provider: &'static str,
    pub bootstrap_fields: &'static [&'static str],
    pub completion_fields: &'static [&'static str],
    pub graph_version: &'static str,
    pub signup_version: &'static str,
    /// SDK extras for the selected contract; the configuration selects stable v4.
    pub signup_extras: fn(&str) -> serde_json::Value,
    pub feature_types: &'static [&'static str],
}

#[derive(Default)]
pub struct ManagedOnboardingInput(pub BTreeMap<String, Zeroizing<String>>);

#[derive(Clone, Default)]
pub struct ManagedProgress(pub Option<tokio::sync::mpsc::UnboundedSender<serde_json::Value>>);

impl ManagedProgress {
    pub fn stage(&self, stage: &'static str) {
        if let Some(sender) = &self.0 {
            let _ = sender.send(serde_json::json!({ "stage": stage }));
        }
    }
}

impl std::fmt::Debug for ManagedOnboardingInput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ManagedOnboardingInput([REDACTED])")
    }
}

impl ManagedOnboardingInput {
    pub fn get(&self, name: &str) -> AppResult<&str> {
        self.0
            .get(name)
            .map(|value| value.as_str())
            .ok_or_else(|| AppError::ValidationError(format!("Missing onboarding field: {name}")))
    }
}

pub struct ManagedOnboardingResult {
    pub token: Zeroizing<String>,
    pub identity: BotIdentity,
    /// Adapter-selected ordinary registration fields, without platform secrets.
    pub fields: BTreeMap<&'static str, String>,
    pub registration_pin: Zeroizing<String>,
    pub setup: crate::models::channel_bot::ManagedBotSetup,
}

impl std::fmt::Debug for ManagedOnboardingResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ManagedOnboardingResult([REDACTED])")
    }
}

/// Resolve on demand so rotations take effect on every replica immediately.
pub async fn build_verify_secrets(
    db: &mongodb::Database,
    keys: &crate::crypto::aes::EncryptionKeys,
    adapter: &dyn PlatformAdapter,
    bot: &crate::models::channel_bot::ChannelBot,
) -> AppResult<PlatformVerifySecrets> {
    let mut secrets = adapter.build_verify_secrets(keys, bot).await?;
    if bot.credential_source == "platform" {
        let descriptor = adapter.platform_credentials().ok_or_else(unavailable)?;
        let credentials =
            super::platform_credential_service::load_decrypted(db, keys, descriptor.provider)
                .await?;
        for field in adapter.registration().fields {
            if secrets.get(field.name).is_none()
                && let Some(fallback) = field.platform_fallback
                && let Some(value) = credentials.get(fallback)
            {
                secrets.insert(field.name, value.to_string());
            }
        }
    }
    Ok(secrets)
}

pub fn unavailable() -> AppError {
    AppError::ValidationError("Managed onboarding is not configured for this platform".to_string())
}

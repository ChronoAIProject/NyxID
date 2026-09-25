//! Metadata-only catalog available to every ordinary authenticated caller.
use crate::services::{
    channel_platform::{ChannelCapabilities, Ingestion},
    channel_platform_catalog_service::{self, PlatformCatalogEntry},
    channel_registration::RegistrationField,
};
use crate::{AppState, errors::AppResult, mw::auth::AuthUser};
use axum::{Json, extract::State};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct PlatformListResponse {
    pub platforms: Vec<PlatformItem>,
}
#[derive(Debug, Serialize)]
pub struct PlatformItem {
    pub activities: &'static [crate::services::channel_activity_service::ActivityDescriptor],
    pub platform: String,
    pub display_name: String,
    pub enabled: bool,
    pub managed_only: bool,
    pub managed_only_message: &'static str,
    pub ingestion: Ingestion,
    pub registration: RegistrationItem,
    pub managed_onboarding: Option<ManagedOnboardingItem>,
    pub platform_credentials: Option<PlatformCredentialsItem>,
    pub capabilities: ChannelCapabilities,
    pub webhook_path: Option<String>,
}
#[derive(Debug, Serialize)]
pub struct RegistrationItem {
    pub documentation_url: Option<&'static str>,
    pub fields: Vec<RegistrationFieldItem>,
    pub token_fields: &'static [&'static str],
    pub extra_fields: Vec<RegistrationFieldItem>,
    pub required_suffix: &'static str,
    pub automatic_webhook: bool,
    pub webhook_ingestion: bool,
    pub webhook_secret_label: Option<&'static str>,
    pub create_response_status: &'static str,
    pub setup_instructions: &'static [&'static str],
}
#[derive(Debug, Serialize)]
pub struct RegistrationFieldItem {
    pub name: &'static str,
    pub label: &'static str,
    pub hint: Option<&'static str>,
    pub secret: bool,
    pub required: bool,
    pub patchable: bool,
    pub clearable: bool,
    pub storage: &'static str,
    pub webhook_secret: bool,
    pub platform_fallback: Option<&'static str>,
}
impl From<&RegistrationField> for RegistrationFieldItem {
    fn from(f: &RegistrationField) -> Self {
        Self {
            name: f.name,
            label: f.label,
            hint: f.hint,
            secret: f.secret,
            required: f.required,
            patchable: f.patchable,
            clearable: f.clearable,
            storage: f.storage,
            webhook_secret: f.webhook_secret,
            platform_fallback: f.platform_fallback,
        }
    }
}
#[derive(Debug, Serialize)]
pub struct ManagedOnboardingItem {
    pub flow: &'static str,
    pub provider: &'static str,
    pub bootstrap_fields: &'static [&'static str],
    pub completion_fields: &'static [&'static str],
}
#[derive(Debug, Serialize)]
pub struct PlatformCredentialsItem {
    pub provider: &'static str,
    pub configured: bool,
}

impl From<PlatformCatalogEntry> for PlatformItem {
    fn from(entry: PlatformCatalogEntry) -> Self {
        let r = entry.registration;
        Self {
            activities: entry.activities,
            webhook_path: (r.enabled && r.webhook_ingestion)
                .then(|| format!("/api/v1/webhooks/channel/{}/{{bot_id}}", entry.platform)),
            platform: entry.platform,
            display_name: entry.display_name,
            enabled: r.enabled,
            managed_only: r.managed_only,
            managed_only_message: r.managed_only_message,
            ingestion: entry.ingestion,
            registration: RegistrationItem {
                documentation_url: r.documentation_url,
                fields: r.fields.iter().map(Into::into).collect(),
                token_fields: r.token_fields,
                extra_fields: r.extra_fields.iter().map(Into::into).collect(),
                required_suffix: r.required_suffix,
                automatic_webhook: r.automatic_webhook,
                webhook_ingestion: r.webhook_ingestion,
                webhook_secret_label: r.webhook_secret_label,
                create_response_status: r.create_response_status,
                setup_instructions: r.setup_instructions,
            },
            managed_onboarding: entry.managed_onboarding.map(|m| ManagedOnboardingItem {
                flow: m.flow,
                provider: m.provider,
                bootstrap_fields: m.bootstrap_fields,
                completion_fields: m.completion_fields,
            }),
            platform_credentials: entry.platform_credentials.map(|(p, configured)| {
                PlatformCredentialsItem {
                    provider: p.provider,
                    configured,
                }
            }),
            capabilities: entry.capabilities,
        }
    }
}

pub async fn list_platforms(
    State(state): State<AppState>,
    _auth: AuthUser,
) -> AppResult<Json<PlatformListResponse>> {
    Ok(Json(PlatformListResponse {
        platforms: channel_platform_catalog_service::list(&state.db, &state.token_exchange_cache)
            .await?
            .into_iter()
            .map(Into::into)
            .collect(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::{
        channel_adapters::registered_adapters, provider_token_exchange_service::TokenExchangeCache,
    };
    use std::sync::Arc;

    #[tokio::test]
    async fn channel_platform_catalog_exact_registry_and_descriptor_fields() {
        let Some(db) = crate::test_utils::connect_test_database("channel_platform_catalog").await
        else {
            eprintln!("no local MongoDB available");
            return;
        };
        let cache = Arc::new(TokenExchangeCache::new());
        let entries: Vec<PlatformItem> = channel_platform_catalog_service::list(&db, &cache)
            .await
            .unwrap()
            .into_iter()
            .map(Into::into)
            .collect();
        let adapters = registered_adapters(&cache);
        assert_eq!(entries.len(), adapters.len());
        let ids: std::collections::HashSet<_> = entries.iter().map(|e| &e.platform).collect();
        assert_eq!(ids.len(), 10);
        for adapter in adapters {
            let entry = entries
                .iter()
                .find(|e| e.platform == adapter.platform_id())
                .unwrap();
            let registration = adapter.registration();
            for (actual, expected) in [
                (&entry.registration.fields, registration.fields),
                (&entry.registration.extra_fields, registration.extra_fields),
            ] {
                assert_eq!(actual.len(), expected.len());
                for (actual, expected) in actual.iter().zip(expected) {
                    assert_eq!(
                        serde_json::to_value(actual).unwrap(),
                        serde_json::to_value(RegistrationFieldItem::from(expected)).unwrap()
                    );
                }
            }
            assert_eq!(entry.registration.token_fields, registration.token_fields);
            assert_eq!(entry.enabled, registration.enabled);
            assert_eq!(entry.capabilities.media, adapter.media_capabilities());
        }
        let telegram =
            serde_json::to_value(entries.iter().find(|e| e.platform == "telegram").unwrap())
                .unwrap();
        let aurinko =
            serde_json::to_value(entries.iter().find(|e| e.platform == "aurinko").unwrap())
                .unwrap();
        assert_eq!(aurinko["display_name"], "Aurinko Email");
        assert_eq!(aurinko["ingestion"], serde_json::json!({"mode":"webhook"}));
        assert!(aurinko["managed_onboarding"].is_null());
        assert_eq!(
            aurinko["capabilities"],
            serde_json::json!({"initiated_send":false,"reply_to":true,"thread":false,"edit":false,
                "media":{"inbound":[],"outbound":[]}})
        );
        assert_eq!(
            aurinko["registration"]["fields"].as_array().unwrap().len(),
            2
        );
        for field in aurinko["registration"]["fields"].as_array().unwrap() {
            assert_eq!(field["secret"], true);
            assert_eq!(field["required"], true);
            assert_eq!(field["patchable"], true);
            assert!(field["hint"].as_str().is_some_and(|hint| !hint.is_empty()));
        }
        assert_eq!(
            telegram["registration"]["fields"],
            serde_json::json!([{
                "name":"bot_token","label":"Bot token","secret":true,"required":true,"patchable":false,"clearable":false,
                "storage":"bot_token_encrypted","webhook_secret":false,"platform_fallback":null,
                "hint":"Connect an existing Telegram bot using its BotFather token."
            }])
        );
        assert_eq!(
            telegram["capabilities"],
            serde_json::json!({"initiated_send":true,"reply_to":true,"thread":true,"edit":true,
            "media":{"inbound":["image","file","audio","video"],"outbound":["image","file","audio","video"]}})
        );
        assert_eq!(
            telegram["webhook_path"],
            "/api/v1/webhooks/channel/telegram/{bot_id}"
        );
        let whatsapp =
            serde_json::to_value(entries.iter().find(|e| e.platform == "whatsapp").unwrap())
                .unwrap();
        let phone_number = whatsapp["registration"]["fields"]
            .as_array()
            .unwrap()
            .iter()
            .find(|field| field["name"] == "phone_number_id")
            .unwrap();
        assert_eq!(
            phone_number["hint"],
            "Meta phone number identifier, not the display phone number or App ID."
        );
        assert_eq!(
            whatsapp["managed_onboarding"]["flow"],
            "meta_embedded_signup"
        );
        assert_eq!(
            whatsapp["platform_credentials"],
            serde_json::json!({"provider":"meta","configured":false})
        );
        assert_eq!(whatsapp["ingestion"], serde_json::json!({"mode":"webhook"}));
        assert_eq!(
            whatsapp["registration"]["webhook_secret_label"],
            "Verify Token"
        );
        assert_eq!(
            telegram.as_object().unwrap().keys().collect::<Vec<_>>(),
            whatsapp.as_object().unwrap().keys().collect::<Vec<_>>()
        );
        assert!(
            !entries
                .iter()
                .find(|e| e.platform == "openclaw")
                .unwrap()
                .enabled
        );
        db.drop().await.unwrap();
    }
}

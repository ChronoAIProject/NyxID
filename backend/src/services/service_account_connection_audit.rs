//! Administration-only evidence. Credential and state material never enter this projection.
use crate::{
    errors::AppResult,
    models::security_effect_intent::SecurityEffectIntent,
    services::audit_service::{
        AuditActor,
        required::{self, TextField},
    },
};

#[derive(Clone, Copy)]
pub(crate) enum Event {
    ApiKeyConnected,
    DeviceConnected,
    ProviderDisconnected,
    OAuthInitiated,
    DeviceInitiated,
    ServiceConnected { has_credential: bool },
    CredentialUpdated,
    ServiceDisconnected,
}

pub(crate) fn intent(
    actor: &AuditActor,
    sa: &str,
    target: &str,
    event: Event,
) -> AppResult<SecurityEffectIntent> {
    let (name, field) = match event {
        Event::ApiKeyConnected | Event::DeviceConnected => {
            ("admin.sa.provider_connected", "provider_id")
        }
        Event::ProviderDisconnected => ("admin.sa.provider_disconnected", "provider_id"),
        Event::OAuthInitiated => ("admin.sa.oauth_initiated", "provider_id"),
        Event::DeviceInitiated => ("admin.sa.device_code_initiated", "provider_id"),
        Event::ServiceConnected { .. } => ("admin.sa.service_connected", "service_id"),
        Event::CredentialUpdated => ("admin.sa.service_credential_updated", "service_id"),
        Event::ServiceDisconnected => ("admin.sa.service_disconnected", "service_id"),
    };
    let mut metadata = serde_json::json!({"target_sa_id":required::text(TextField::Identifier, sa),field:required::text(TextField::Identifier, target)});
    match event {
        Event::ApiKeyConnected => metadata["token_type"] = "api_key".into(),
        Event::DeviceConnected => metadata["token_type"] = "device_code".into(),
        Event::ServiceConnected { has_credential } => {
            metadata["has_credential"] = has_credential.into()
        }
        _ => {}
    }
    required::intent_preserving_actor(actor, name, Some(metadata))
}

use std::collections::BTreeMap;

use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode, header},
};
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use super::admin_helpers::require_admin;
use crate::services::channel_managed::{PlatformCredentialDescriptor, PlatformCredentialField};
use crate::services::{audit_service, platform_credential_service as service};
use crate::{AppState, errors::AppResult, mw::auth::AuthUser};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UpdatePlatformCredentialsRequest {
    #[serde(default)]
    pub fields: BTreeMap<String, Option<Zeroizing<String>>>,
    #[serde(default)]
    pub regenerate_verify_token: bool,
}

impl std::fmt::Debug for UpdatePlatformCredentialsRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("UpdatePlatformCredentialsRequest([REDACTED])")
    }
}

#[derive(Debug, Serialize)]
pub struct CredentialFieldResponse {
    #[serde(flatten)]
    pub descriptor: PlatformCredentialField,
    pub configured: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
}

#[derive(Serialize)]
pub struct PlatformCredentialsResponse {
    pub backing: crate::services::channel_managed::PlatformCredentialBacking,
    pub provider: &'static str,
    pub label: &'static str,
    pub platform: String,
    pub available: bool,
    pub fields: Vec<CredentialFieldResponse>,
    pub setup_checklist: &'static [&'static str],
    pub callback_url: Option<String>,
    pub webhook_verify_token: Option<String>,
    pub updated_at: Option<String>,
}

impl std::fmt::Debug for PlatformCredentialsResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlatformCredentialsResponse")
            .field("provider", &self.provider)
            .field("webhook_verify_token", &"[REDACTED]")
            .finish_non_exhaustive()
    }
}

async fn response(
    state: &AppState,
    platform: String,
    descriptor: PlatformCredentialDescriptor,
) -> AppResult<PlatformCredentialsResponse> {
    let row = service::load(&state.db, descriptor.provider).await?;
    let fields = descriptor
        .fields
        .iter()
        .map(|field| CredentialFieldResponse {
            descriptor: *field,
            configured: row.as_ref().is_some_and(|row| {
                if field.secret {
                    row.secrets.contains_key(field.name)
                } else {
                    row.fields.contains_key(field.name)
                }
            }),
            value: if field.secret {
                None
            } else {
                row.as_ref()
                    .and_then(|row| row.fields.get(field.name).cloned())
            },
        })
        .collect();
    // This generated handshake token is intentionally readable only by admins.
    // App secrets are never returned or included in the field projection.
    let credentials =
        service::load_decrypted(&state.db, &state.encryption_keys, descriptor.provider).await?;
    Ok(PlatformCredentialsResponse {
        backing: descriptor.backing,
        provider: descriptor.provider,
        label: descriptor.label,
        available: service::configured(row.as_ref(), &descriptor),
        callback_url: if matches!(
            descriptor.backing,
            crate::services::channel_managed::PlatformCredentialBacking::ProviderOAuth { .. }
        ) {
            Some(format!(
                "{}/api/v1/providers/callback",
                state.config.base_url
            ))
        } else {
            descriptor.webhook_secret_field.map(|_| {
                format!(
                    "{}/api/v1/webhooks/channel/{platform}/platform",
                    state.config.base_url
                )
            })
        },
        webhook_verify_token: credentials
            .get(service::VERIFY_TOKEN_FIELD)
            .map(String::from),
        platform,
        fields,
        setup_checklist: descriptor.setup_checklist,
        updated_at: row.map(|row| row.updated_at.to_rfc3339()),
    })
}

fn no_store() -> HeaderMap {
    HeaderMap::from_iter([(
        header::CACHE_CONTROL,
        "no-store".parse().expect("static header"),
    )])
}

pub async fn list(
    State(state): State<AppState>,
    auth: AuthUser,
) -> AppResult<(HeaderMap, Json<Vec<PlatformCredentialsResponse>>)> {
    require_admin(&state, &auth).await?;
    let mut responses = Vec::new();
    for (platform, descriptor) in service::descriptors(&state.token_exchange_cache) {
        responses.push(response(&state, platform, descriptor).await?);
    }
    Ok((no_store(), Json(responses)))
}

pub async fn update(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(provider): Path<String>,
    Json(body): Json<UpdatePlatformCredentialsRequest>,
) -> AppResult<(HeaderMap, Json<PlatformCredentialsResponse>)> {
    require_admin(&state, &auth).await?;
    let (platform, descriptor) = service::descriptor(&state.token_exchange_cache, &provider)?;
    service::update(
        &state.db,
        &state.encryption_keys,
        &descriptor,
        &auth.user_id.to_string(),
        &body.fields,
        body.regenerate_verify_token,
    )
    .await?;
    audit_service::log_for_user(
        state.db.clone(),
        &auth,
        "admin_platform_credentials_updated",
        Some(
            serde_json::json!({ "provider": provider, "fields": body.fields.keys().collect::<Vec<_>>(), "verify_token_regenerated": body.regenerate_verify_token }),
        ),
    );
    Ok((
        no_store(),
        Json(response(&state, platform, descriptor).await?),
    ))
}

pub async fn delete(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(provider): Path<String>,
) -> AppResult<StatusCode> {
    require_admin(&state, &auth).await?;
    service::descriptor(&state.token_exchange_cache, &provider)?;
    service::delete(&state.db, &provider).await?;
    audit_service::log_for_user(
        state.db.clone(),
        &auth,
        "admin_platform_credentials_updated",
        Some(serde_json::json!({ "provider": provider, "deleted": true })),
    );
    Ok(StatusCode::NO_CONTENT)
}

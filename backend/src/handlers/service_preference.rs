use axum::{Json, extract::State};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use utoipa::ToSchema;

use crate::{
    AppState,
    errors::AppResult,
    mw::auth::{AuthMethod, AuthUser},
    services::{
        audit_service::{self, AuditActor},
        service_preference_service as preferences,
    },
};

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ServicePreferenceRequest {
    pub ordered: Vec<String>,
    pub expected_version: i64,
}

#[derive(Serialize, ToSchema)]
pub struct ServicePreferenceResponse {
    pub ordered: Vec<String>,
    pub version: i64,
    pub updated_at: Option<String>,
}

fn response(
    row: Option<&crate::models::service_preference::ServicePreference>,
    visible: &HashSet<String>,
) -> ServicePreferenceResponse {
    ServicePreferenceResponse {
        ordered: row.map_or_else(Vec::new, |row| {
            preferences::resolve_visible(&row.ordered, visible)
        }),
        version: row.map_or(0, |row| row.version),
        updated_at: row.map(|row| row.updated_at.to_rfc3339()),
    }
}

async fn visible(state: &AppState, auth: &AuthUser) -> AppResult<HashSet<String>> {
    Ok(preferences::visible_inventory(
        &state.db,
        &state.encryption_keys,
        &auth.user_id.to_string(),
        auth.api_key_service_scope(),
        auth.auth_method == AuthMethod::ApiKey,
    )
    .await?
    .into_iter()
    .map(|view| view.id)
    .collect())
}

#[utoipa::path(get, path = "/api/v1/service-preferences", tag = "AI Services", responses((status = 200, body = ServicePreferenceResponse), (status = 401, body = crate::errors::ErrorResponse), (status = 403, body = crate::errors::ErrorResponse)))]
pub async fn get(
    State(state): State<AppState>,
    auth: AuthUser,
) -> AppResult<Json<ServicePreferenceResponse>> {
    let visible = visible(&state, &auth).await?;
    let row = preferences::get(&state.db, &auth.user_id.to_string()).await?;
    Ok(Json(response(row.as_ref(), &visible)))
}

#[utoipa::path(put, path = "/api/v1/service-preferences", tag = "AI Services", request_body = ServicePreferenceRequest, responses((status = 200, body = ServicePreferenceResponse), (status = 400, body = crate::errors::ErrorResponse), (status = 403, body = crate::errors::ErrorResponse), (status = 409, body = crate::errors::ErrorResponse), (status = 413, description = "Request exceeds 16 KiB")))]
pub async fn put(
    State(state): State<AppState>,
    auth: AuthUser,
    body: Result<Json<ServicePreferenceRequest>, axum::extract::rejection::JsonRejection>,
) -> AppResult<Json<ServicePreferenceResponse>> {
    super::login_client_context::require_first_party_human(&auth)?;
    let Json(body) = body.map_err(|error| {
        if error.status() == axum::http::StatusCode::PAYLOAD_TOO_LARGE {
            crate::errors::AppError::RequestBodyTooLarge {
                max_bytes: preferences::MAX_REQUEST_BYTES,
                context: "service preferences".into(),
            }
        } else {
            crate::errors::AppError::ValidationError("invalid service preference request".into())
        }
    })?;
    let visible = visible(&state, &auth).await?;
    let replacement = preferences::replace(
        &state.db,
        &auth.user_id.to_string(),
        &body.ordered,
        body.expected_version,
        &visible,
    )
    .await?;
    if replacement.changed {
        let row = replacement
            .preference
            .as_ref()
            .expect("changed preference exists");
        let _ = audit_service::log_actor_event(
            state.db.clone(),
            &AuditActor::from_auth_user(&auth),
            "service_preference_updated",
            Some(serde_json::json!({ "count": row.ordered.len(), "version": row.version })),
        )
        .await;
    }
    Ok(Json(response(replacement.preference.as_ref(), &visible)))
}

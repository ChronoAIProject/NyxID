use axum::{
    Json,
    extract::{Path, State},
};
use serde::{Deserialize, Serialize};
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
pub struct ServicePreferenceGroup {
    pub group: String,
    pub ordered: Vec<String>,
}

#[derive(Serialize, ToSchema)]
pub struct ServicePreferenceResponse {
    pub groups: Vec<ServicePreferenceGroup>,
    pub version: i64,
    pub updated_at: Option<String>,
}

async fn response(
    state: &AppState,
    auth: &AuthUser,
    row: Option<&crate::models::service_preference::ServicePreference>,
) -> AppResult<ServicePreferenceResponse> {
    let ordered = row.map_or(&[][..], |row| row.ordered.as_slice());
    let visible = preferences::visible_ordered_members(
        &state.db,
        &auth.user_id.to_string(),
        ordered,
        auth.api_key_service_scope(),
        auth.auth_method == AuthMethod::ApiKey,
    )
    .await?;
    Ok(ServicePreferenceResponse {
        groups: preferences::grouped_visible(ordered, &visible)
            .into_iter()
            .map(|(group, ordered)| ServicePreferenceGroup { group, ordered })
            .collect(),
        version: row.map_or(0, |row| row.version),
        updated_at: row.map(|row| row.updated_at.to_rfc3339()),
    })
}

#[utoipa::path(get, path = "/api/v1/service-preferences", tag = "AI Services", responses((status = 200, body = ServicePreferenceResponse), (status = 401, body = crate::errors::ErrorResponse), (status = 403, body = crate::errors::ErrorResponse)))]
pub async fn get(
    State(state): State<AppState>,
    auth: AuthUser,
) -> AppResult<Json<ServicePreferenceResponse>> {
    let row = preferences::get(&state.db, &auth.user_id.to_string()).await?;
    Ok(Json(response(&state, &auth, row.as_ref()).await?))
}

#[utoipa::path(put, path = "/api/v1/service-preferences/groups/{group}", tag = "AI Services", params(("group" = String, Path, description = "Immutable catalog:<UUID> group")), request_body = ServicePreferenceRequest, responses((status = 200, body = ServicePreferenceResponse), (status = 400, body = crate::errors::ErrorResponse), (status = 403, body = crate::errors::ErrorResponse), (status = 409, body = crate::errors::ErrorResponse), (status = 413, description = "Request exceeds 16 KiB")))]
pub async fn put_group(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(group): Path<String>,
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
    let replacement = preferences::replace_group(
        &state.db,
        &auth.user_id.to_string(),
        &group,
        &body.ordered,
        body.expected_version,
    )
    .await?;
    if replacement.changed {
        let row = replacement
            .preference
            .as_ref()
            .expect("changed preference exists");
        let _ = audit_service::log_actor_event(state.db.clone(), &AuditActor::from_auth_user(&auth), "service_preference_updated", Some(serde_json::json!({ "group": group, "count": body.ordered.len(), "version": row.version }))).await;
    }
    Ok(Json(
        response(&state, &auth, replacement.preference.as_ref()).await?,
    ))
}

#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ReleaseHiddenPreferenceRequest {
    pub expected_version: i64,
}

#[utoipa::path(delete, path = "/api/v1/service-preferences/hidden", tag = "AI Services", request_body = ReleaseHiddenPreferenceRequest, responses((status = 200, body = ServicePreferenceResponse), (status = 400, body = crate::errors::ErrorResponse), (status = 403, body = crate::errors::ErrorResponse), (status = 409, body = crate::errors::ErrorResponse), (status = 413, description = "Request exceeds 16 KiB")))]
pub async fn release_hidden(
    State(state): State<AppState>,
    auth: AuthUser,
    body: Result<Json<ReleaseHiddenPreferenceRequest>, axum::extract::rejection::JsonRejection>,
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
    let released =
        preferences::release_hidden(&state.db, &auth.user_id.to_string(), body.expected_version)
            .await?;
    if released.replacement.changed {
        let row = released
            .replacement
            .preference
            .as_ref()
            .expect("changed preference exists");
        let _ = audit_service::log_actor_event(state.db.clone(), &AuditActor::from_auth_user(&auth), "service_preference_hidden_released", Some(serde_json::json!({ "released_hidden": released.released_hidden, "version": row.version }))).await;
    }
    Ok(Json(
        response(&state, &auth, released.replacement.preference.as_ref()).await?,
    ))
}

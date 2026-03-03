use axum::{
    Json,
    extract::{Path, State},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::mw::auth::AuthUser;
use crate::services::key_service;

// --- Request / Response types ---

#[derive(Debug, Deserialize)]
pub struct CreateApiKeyRequest {
    pub name: String,
    pub scopes: Option<String>,
    pub expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize)]
pub struct CreateApiKeyResponse {
    pub id: String,
    pub name: String,
    pub key_prefix: String,
    /// The full API key. Shown only once at creation time.
    pub full_key: String,
    pub scopes: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct ApiKeyListItem {
    pub id: String,
    pub name: String,
    pub key_prefix: String,
    pub scopes: String,
    pub last_used_at: Option<DateTime<Utc>>,
    pub expires_at: Option<DateTime<Utc>>,
    pub is_active: bool,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Serialize)]
pub struct ApiKeyListResponse {
    pub keys: Vec<ApiKeyListItem>,
}

#[derive(Debug, Serialize)]
pub struct DeleteApiKeyResponse {
    pub message: String,
}

// --- Handlers ---

/// GET /api/v1/api-keys
///
/// List all API keys for the authenticated user.
pub async fn list_keys(
    State(state): State<AppState>,
    auth_user: AuthUser,
) -> AppResult<Json<ApiKeyListResponse>> {
    let user_id_str = auth_user.user_id.to_string();
    let keys = key_service::list_api_keys(&state.db, &user_id_str).await?;

    let items: Vec<ApiKeyListItem> = keys
        .into_iter()
        .map(|k| ApiKeyListItem {
            id: k.id.to_string(),
            name: k.name,
            key_prefix: k.key_prefix,
            scopes: k.scopes,
            last_used_at: k.last_used_at,
            expires_at: k.expires_at,
            is_active: k.is_active,
            created_at: k.created_at,
        })
        .collect();

    Ok(Json(ApiKeyListResponse { keys: items }))
}

/// POST /api/v1/api-keys
///
/// Create a new API key. The full key is returned only in this response.
pub async fn create_key(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<CreateApiKeyRequest>,
) -> AppResult<Json<CreateApiKeyResponse>> {
    if body.name.is_empty() {
        return Err(AppError::ValidationError(
            "API key name is required".to_string(),
        ));
    }

    let scopes = body.scopes.as_deref().unwrap_or("read");

    let user_id_str = auth_user.user_id.to_string();
    let created =
        key_service::create_api_key(&state.db, &user_id_str, &body.name, scopes, body.expires_at)
            .await?;

    Ok(Json(CreateApiKeyResponse {
        id: created.id.to_string(),
        name: created.name,
        key_prefix: created.key_prefix,
        full_key: created.full_key,
        scopes: created.scopes,
        created_at: created.created_at,
    }))
}

/// DELETE /api/v1/api-keys/:key_id
///
/// Deactivate an API key.
pub async fn delete_key(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(key_id): Path<String>,
) -> AppResult<Json<DeleteApiKeyResponse>> {
    let user_id_str = auth_user.user_id.to_string();
    key_service::delete_api_key(&state.db, &user_id_str, &key_id).await?;

    Ok(Json(DeleteApiKeyResponse {
        message: "API key deleted".to_string(),
    }))
}

/// POST /api/v1/api-keys/:key_id/rotate
///
/// Rotate an API key: deactivate the old one and create a new one.
pub async fn rotate_key(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(key_id): Path<String>,
) -> AppResult<Json<CreateApiKeyResponse>> {
    let user_id_str = auth_user.user_id.to_string();
    let created = key_service::rotate_api_key(&state.db, &user_id_str, &key_id).await?;

    Ok(Json(CreateApiKeyResponse {
        id: created.id.to_string(),
        name: created.name,
        key_prefix: created.key_prefix,
        full_key: created.full_key,
        scopes: created.scopes,
        created_at: created.created_at,
    }))
}

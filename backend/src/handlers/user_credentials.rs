use axum::{
    Json,
    extract::{Path, State},
};
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::mw::auth::AuthUser;
use crate::services::{audit_service, provider_service, user_credentials_service};

// --- Request / Response types ---

#[derive(Deserialize)]
pub struct SetUserCredentialsRequest {
    pub client_id: String,
    pub client_secret: Option<String>,
    pub label: Option<String>,
}

impl std::fmt::Debug for SetUserCredentialsRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SetUserCredentialsRequest")
            .field("client_id", &"[REDACTED]")
            .field("client_secret", &"[REDACTED]")
            .field("label", &self.label)
            .finish()
    }
}

#[derive(Debug, Serialize)]
pub struct UserCredentialsResponse {
    pub provider_config_id: String,
    pub has_credentials: bool,
    pub label: Option<String>,
    pub created_at: Option<String>,
    pub updated_at: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct DeleteCredentialsResponse {
    pub message: String,
}

// --- Handlers ---

/// GET /api/v1/providers/{provider_id}/credentials
pub async fn get_my_credentials(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(provider_id): Path<String>,
) -> AppResult<Json<UserCredentialsResponse>> {
    let user_id_str = auth_user.user_id.to_string();

    let metadata = user_credentials_service::get_user_credentials_metadata(
        &state.db,
        &user_id_str,
        &provider_id,
    )
    .await?;

    match metadata {
        Some(m) => Ok(Json(UserCredentialsResponse {
            provider_config_id: m.provider_config_id,
            has_credentials: true,
            label: m.label,
            created_at: Some(m.created_at.to_rfc3339()),
            updated_at: Some(m.updated_at.to_rfc3339()),
        })),
        None => Ok(Json(UserCredentialsResponse {
            provider_config_id: provider_id,
            has_credentials: false,
            label: None,
            created_at: None,
            updated_at: None,
        })),
    }
}

/// PUT /api/v1/providers/{provider_id}/credentials
pub async fn set_my_credentials(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(provider_id): Path<String>,
    Json(body): Json<SetUserCredentialsRequest>,
) -> AppResult<Json<UserCredentialsResponse>> {
    let user_id_str = auth_user.user_id.to_string();

    // Validate provider exists and is active
    let provider = provider_service::get_provider(&state.db, &provider_id).await?;
    if !provider.is_active {
        return Err(AppError::BadRequest("Provider is not active".to_string()));
    }

    // Validate credential_mode allows user credentials
    if !user_credentials_service::supports_user_credentials(&provider) {
        return Err(AppError::BadRequest(
            "This provider does not accept user-provided credentials".to_string(),
        ));
    }

    // Validate inputs
    if body.client_id.is_empty() || body.client_id.len() > 500 {
        return Err(AppError::ValidationError(
            "client_id must be between 1 and 500 characters".to_string(),
        ));
    }
    if body
        .client_secret
        .as_ref()
        .is_some_and(|value| value.len() > 2000)
    {
        return Err(AppError::ValidationError(
            "client_secret must be at most 2000 characters".to_string(),
        ));
    }
    let client_secret = body
        .client_secret
        .as_deref()
        .filter(|value| !value.is_empty());

    let cred = user_credentials_service::upsert_user_credentials(
        &state.db,
        &state.encryption_keys,
        &user_id_str,
        &provider_id,
        &body.client_id,
        client_secret,
        body.label.as_deref(),
    )
    .await?;

    audit_service::log_async(
        state.db.clone(),
        Some(user_id_str),
        "user_credentials_set".to_string(),
        Some(serde_json::json!({
            "provider_id": &provider_id,
        })),
        None,
        None,
    );

    Ok(Json(UserCredentialsResponse {
        provider_config_id: cred.provider_config_id,
        has_credentials: true,
        label: cred.label,
        created_at: Some(cred.created_at.to_rfc3339()),
        updated_at: Some(cred.updated_at.to_rfc3339()),
    }))
}

/// DELETE /api/v1/providers/{provider_id}/credentials
pub async fn delete_my_credentials(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(provider_id): Path<String>,
) -> AppResult<Json<DeleteCredentialsResponse>> {
    let user_id_str = auth_user.user_id.to_string();

    user_credentials_service::delete_user_credentials(&state.db, &user_id_str, &provider_id)
        .await?;

    audit_service::log_async(
        state.db.clone(),
        Some(user_id_str),
        "user_credentials_deleted".to_string(),
        Some(serde_json::json!({
            "provider_id": &provider_id,
        })),
        None,
        None,
    );

    Ok(Json(DeleteCredentialsResponse {
        message: "Credentials deleted successfully".to_string(),
    }))
}

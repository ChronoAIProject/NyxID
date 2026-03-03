use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
};
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::crypto::aes;
use crate::errors::{AppError, AppResult};
use crate::handlers::admin_helpers::{extract_ip, extract_user_agent, require_admin};
use crate::mw::auth::AuthUser;
use crate::services::{audit_service, service_account_service, user_token_service};

// --- Request types ---

#[derive(Deserialize)]
pub struct AdminConnectApiKeyRequest {
    pub api_key: String,
    pub label: Option<String>,
}

impl std::fmt::Debug for AdminConnectApiKeyRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AdminConnectApiKeyRequest")
            .field("api_key", &"[REDACTED]")
            .field("label", &self.label)
            .finish()
    }
}

// --- Response types ---

#[derive(Debug, Serialize)]
pub struct AdminSaProviderTokenResponse {
    pub provider_id: String,
    pub provider_name: String,
    pub provider_slug: String,
    pub provider_type: String,
    pub status: String,
    pub label: Option<String>,
    pub expires_at: Option<String>,
    pub last_used_at: Option<String>,
    pub connected_at: String,
}

#[derive(Debug, Serialize)]
pub struct AdminSaProviderListResponse {
    pub tokens: Vec<AdminSaProviderTokenResponse>,
}

#[derive(Debug, Serialize)]
pub struct AdminSaProviderActionResponse {
    pub status: String,
    pub message: String,
}

#[derive(Debug, Serialize)]
pub struct AdminSaOAuthInitiateResponse {
    pub authorization_url: String,
}

#[derive(Debug, Serialize)]
pub struct AdminSaDeviceCodeInitiateResponse {
    pub user_code: String,
    pub verification_uri: String,
    pub state: String,
    pub expires_in: i64,
    pub interval: i32,
}

#[derive(Debug, Deserialize)]
pub struct AdminSaDeviceCodePollRequest {
    pub state: String,
}

#[derive(Debug, Serialize)]
pub struct AdminSaDeviceCodePollResponse {
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interval: Option<i32>,
}

// --- Handlers ---

/// GET /api/v1/admin/service-accounts/{sa_id}/providers
pub async fn list_sa_providers(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(sa_id): Path<String>,
) -> AppResult<Json<AdminSaProviderListResponse>> {
    require_admin(&state, &auth_user).await?;

    // Verify SA exists
    let _sa = service_account_service::get_service_account(&state.db, &sa_id).await?;

    let summaries = user_token_service::list_user_tokens(&state.db, &sa_id).await?;

    let tokens: Vec<AdminSaProviderTokenResponse> = summaries
        .into_iter()
        .map(|s| AdminSaProviderTokenResponse {
            provider_id: s.provider_config_id,
            provider_name: s.provider_name,
            provider_slug: s.provider_slug,
            provider_type: s.token_type,
            status: s.status,
            label: s.label,
            expires_at: s.expires_at,
            last_used_at: s.last_used_at,
            connected_at: s.connected_at,
        })
        .collect();

    Ok(Json(AdminSaProviderListResponse { tokens }))
}

/// POST /api/v1/admin/service-accounts/{sa_id}/providers/{provider_id}/connect/api-key
pub async fn connect_api_key_for_sa(
    State(state): State<AppState>,
    auth_user: AuthUser,
    headers: HeaderMap,
    Path((sa_id, provider_id)): Path<(String, String)>,
    Json(body): Json<AdminConnectApiKeyRequest>,
) -> AppResult<Json<AdminSaProviderActionResponse>> {
    require_admin(&state, &auth_user).await?;

    // Verify SA exists and is active
    let sa = service_account_service::get_service_account(&state.db, &sa_id).await?;
    if !sa.is_active {
        return Err(AppError::BadRequest(
            "Cannot connect providers to an inactive service account".to_string(),
        ));
    }

    if body.api_key.is_empty() {
        return Err(AppError::ValidationError(
            "API key must not be empty".to_string(),
        ));
    }
    if body.api_key.len() > 4096 {
        return Err(AppError::ValidationError(
            "API key exceeds maximum length".to_string(),
        ));
    }

    let encryption_key = aes::parse_hex_key(&state.config.encryption_key)?;

    // Reuse existing service -- pass sa.id as user_id
    user_token_service::store_api_key(
        &state.db,
        &encryption_key,
        &sa_id,
        &provider_id,
        &body.api_key,
        body.label.as_deref(),
    )
    .await?;

    audit_service::log_async(
        state.db.clone(),
        Some(auth_user.user_id.to_string()),
        "admin.sa.provider_connected".to_string(),
        Some(serde_json::json!({
            "target_sa_id": &sa_id,
            "provider_id": &provider_id,
            "token_type": "api_key",
        })),
        extract_ip(&headers),
        extract_user_agent(&headers),
    );

    Ok(Json(AdminSaProviderActionResponse {
        status: "connected".to_string(),
        message: "API key stored for service account".to_string(),
    }))
}

/// DELETE /api/v1/admin/service-accounts/{sa_id}/providers/{provider_id}/disconnect
pub async fn disconnect_sa_provider(
    State(state): State<AppState>,
    auth_user: AuthUser,
    headers: HeaderMap,
    Path((sa_id, provider_id)): Path<(String, String)>,
) -> AppResult<Json<AdminSaProviderActionResponse>> {
    require_admin(&state, &auth_user).await?;

    // Verify SA exists
    let _sa = service_account_service::get_service_account(&state.db, &sa_id).await?;

    user_token_service::disconnect_provider(&state.db, &sa_id, &provider_id).await?;

    audit_service::log_async(
        state.db.clone(),
        Some(auth_user.user_id.to_string()),
        "admin.sa.provider_disconnected".to_string(),
        Some(serde_json::json!({
            "target_sa_id": &sa_id,
            "provider_id": &provider_id,
        })),
        extract_ip(&headers),
        extract_user_agent(&headers),
    );

    Ok(Json(AdminSaProviderActionResponse {
        status: "disconnected".to_string(),
        message: "Provider disconnected from service account".to_string(),
    }))
}

/// GET /api/v1/admin/service-accounts/{sa_id}/providers/{provider_id}/connect/oauth
///
/// Admin initiates an OAuth redirect flow on behalf of a service account.
/// Returns the authorization URL for the admin to redirect to.
pub async fn initiate_oauth_for_sa(
    State(state): State<AppState>,
    auth_user: AuthUser,
    headers: HeaderMap,
    Path((sa_id, provider_id)): Path<(String, String)>,
) -> AppResult<Json<AdminSaOAuthInitiateResponse>> {
    require_admin(&state, &auth_user).await?;

    let sa = service_account_service::get_service_account(&state.db, &sa_id).await?;
    if !sa.is_active {
        return Err(AppError::BadRequest(
            "Cannot connect providers to an inactive service account".to_string(),
        ));
    }

    let admin_id = auth_user.user_id.to_string();
    let encryption_key = aes::parse_hex_key(&state.config.encryption_key)?;
    let redirect_path = format!("/admin/service-accounts/{}", &sa_id);

    let auth_url = user_token_service::initiate_oauth_connect(
        &state.db,
        &encryption_key,
        &state.config.base_url,
        &admin_id,
        &provider_id,
        Some(&sa_id),
        Some(&redirect_path),
    )
    .await?;

    audit_service::log_async(
        state.db.clone(),
        Some(admin_id),
        "admin.sa.oauth_initiated".to_string(),
        Some(serde_json::json!({
            "target_sa_id": &sa_id,
            "provider_id": &provider_id,
        })),
        extract_ip(&headers),
        extract_user_agent(&headers),
    );

    Ok(Json(AdminSaOAuthInitiateResponse {
        authorization_url: auth_url,
    }))
}

/// POST /api/v1/admin/service-accounts/{sa_id}/providers/{provider_id}/connect/device-code/initiate
///
/// Admin initiates a device code flow on behalf of a service account.
/// Returns user_code and verification_uri for the admin to authenticate.
pub async fn initiate_device_code_for_sa(
    State(state): State<AppState>,
    auth_user: AuthUser,
    headers: HeaderMap,
    Path((sa_id, provider_id)): Path<(String, String)>,
) -> AppResult<Json<AdminSaDeviceCodeInitiateResponse>> {
    require_admin(&state, &auth_user).await?;

    let sa = service_account_service::get_service_account(&state.db, &sa_id).await?;
    if !sa.is_active {
        return Err(AppError::BadRequest(
            "Cannot connect providers to an inactive service account".to_string(),
        ));
    }

    let admin_id = auth_user.user_id.to_string();
    let encryption_key = aes::parse_hex_key(&state.config.encryption_key)?;

    let result = user_token_service::request_device_code(
        &state.db,
        &encryption_key,
        &admin_id,
        &provider_id,
        Some(&sa_id),
    )
    .await?;

    audit_service::log_async(
        state.db.clone(),
        Some(admin_id),
        "admin.sa.device_code_initiated".to_string(),
        Some(serde_json::json!({
            "target_sa_id": &sa_id,
            "provider_id": &provider_id,
        })),
        extract_ip(&headers),
        extract_user_agent(&headers),
    );

    Ok(Json(AdminSaDeviceCodeInitiateResponse {
        user_code: result.user_code,
        verification_uri: result.verification_uri,
        state: result.state,
        expires_in: result.expires_in,
        interval: result.interval,
    }))
}

/// POST /api/v1/admin/service-accounts/{sa_id}/providers/{provider_id}/connect/device-code/poll
///
/// Admin polls device code status for a service account flow.
/// Returns status: "pending", "slow_down", "expired", "denied", or "complete".
pub async fn poll_device_code_for_sa(
    State(state): State<AppState>,
    auth_user: AuthUser,
    headers: HeaderMap,
    Path((sa_id, provider_id)): Path<(String, String)>,
    Json(body): Json<AdminSaDeviceCodePollRequest>,
) -> AppResult<Json<AdminSaDeviceCodePollResponse>> {
    require_admin(&state, &auth_user).await?;

    // Verify SA exists (no active check needed for polling)
    let _sa = service_account_service::get_service_account(&state.db, &sa_id).await?;

    let admin_id = auth_user.user_id.to_string();
    let encryption_key = aes::parse_hex_key(&state.config.encryption_key)?;

    let result = user_token_service::poll_device_code(
        &state.db,
        &encryption_key,
        &admin_id,
        &provider_id,
        &body.state,
    )
    .await?;

    if result.status == "complete" {
        audit_service::log_async(
            state.db.clone(),
            Some(admin_id),
            "admin.sa.provider_connected".to_string(),
            Some(serde_json::json!({
                "target_sa_id": &sa_id,
                "provider_id": &provider_id,
                "token_type": "device_code",
            })),
            extract_ip(&headers),
            extract_user_agent(&headers),
        );
    }

    Ok(Json(AdminSaDeviceCodePollResponse {
        status: result.status,
        interval: result.interval,
    }))
}

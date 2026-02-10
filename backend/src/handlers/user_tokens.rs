use axum::{
    extract::{Path, Query, State},
    Json,
};
use serde::{Deserialize, Serialize};

use crate::crypto::aes;
use crate::errors::{AppError, AppResult};
use crate::mw::auth::AuthUser;
use crate::services::{audit_service, user_token_service};
use crate::AppState;

// TODO(SEC-9): Apply stricter per-endpoint rate limiting to OAuth callback and
// initiate endpoints (e.g. 10 requests/minute per user) instead of relying
// solely on the global rate limiter.

// --- Request / Response types ---

#[derive(Deserialize)]
pub struct ConnectApiKeyRequest {
    pub api_key: String,
    pub label: Option<String>,
}

impl std::fmt::Debug for ConnectApiKeyRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConnectApiKeyRequest")
            .field("api_key", &"[REDACTED]")
            .field("label", &self.label)
            .finish()
    }
}

#[derive(Debug, Serialize)]
pub struct UserTokenResponse {
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
pub struct UserTokenListResponse {
    pub tokens: Vec<UserTokenResponse>,
}

#[derive(Debug, Serialize)]
pub struct OAuthInitiateResponse {
    pub authorization_url: String,
}

#[derive(Debug, Serialize)]
pub struct ConnectResponse {
    pub status: String,
    pub message: String,
}

#[derive(Debug, Deserialize)]
pub struct OAuthCallbackQuery {
    pub code: String,
    pub state: String,
}

#[derive(Debug, Deserialize)]
pub struct GenericOAuthCallbackQuery {
    pub code: Option<String>,
    pub state: Option<String>,
    pub error: Option<String>,
    pub error_description: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct DeviceCodeInitiateResponse {
    pub user_code: String,
    pub verification_uri: String,
    pub state: String,
    pub expires_in: i64,
    pub interval: i32,
}

#[derive(Debug, Deserialize)]
pub struct DeviceCodePollRequest {
    pub state: String,
}

#[derive(Debug, Serialize)]
pub struct DeviceCodePollResponse {
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub interval: Option<i32>,
}

// --- Handlers ---

/// GET /api/v1/providers/my-tokens
pub async fn list_my_tokens(
    State(state): State<AppState>,
    auth_user: AuthUser,
) -> AppResult<Json<UserTokenListResponse>> {
    let user_id_str = auth_user.user_id.to_string();

    let summaries = user_token_service::list_user_tokens(&state.db, &user_id_str).await?;

    let tokens: Vec<UserTokenResponse> = summaries
        .into_iter()
        .map(|s| UserTokenResponse {
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

    Ok(Json(UserTokenListResponse { tokens }))
}

/// POST /api/v1/providers/{provider_id}/connect/api-key
pub async fn connect_api_key(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(provider_id): Path<String>,
    Json(body): Json<ConnectApiKeyRequest>,
) -> AppResult<Json<ConnectResponse>> {
    let user_id_str = auth_user.user_id.to_string();
    let encryption_key = aes::parse_hex_key(&state.config.encryption_key)?;

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

    user_token_service::store_api_key(
        &state.db,
        &encryption_key,
        &user_id_str,
        &provider_id,
        &body.api_key,
        body.label.as_deref(),
    )
    .await?;

    audit_service::log_async(
        state.db.clone(),
        Some(user_id_str),
        "provider_token_connected".to_string(),
        Some(serde_json::json!({
            "provider_id": &provider_id,
            "token_type": "api_key",
        })),
        None,
        None,
    );

    Ok(Json(ConnectResponse {
        status: "connected".to_string(),
        message: "API key stored successfully".to_string(),
    }))
}

/// GET /api/v1/providers/{provider_id}/connect/oauth
pub async fn initiate_oauth_connect(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(provider_id): Path<String>,
) -> AppResult<Json<OAuthInitiateResponse>> {
    let user_id_str = auth_user.user_id.to_string();
    let encryption_key = aes::parse_hex_key(&state.config.encryption_key)?;

    let auth_url = user_token_service::initiate_oauth_connect(
        &state.db,
        &encryption_key,
        &state.config.base_url,
        &user_id_str,
        &provider_id,
    )
    .await?;

    audit_service::log_async(
        state.db.clone(),
        Some(user_id_str),
        "provider_oauth_initiated".to_string(),
        Some(serde_json::json!({ "provider_id": &provider_id })),
        None,
        None,
    );

    Ok(Json(OAuthInitiateResponse {
        authorization_url: auth_url,
    }))
}

/// GET /api/v1/providers/{provider_id}/callback (legacy per-provider route)
pub async fn oauth_callback(
    State(state): State<AppState>,
    Path(provider_id): Path<String>,
    Query(query): Query<OAuthCallbackQuery>,
) -> AppResult<Json<ConnectResponse>> {
    let encryption_key = aes::parse_hex_key(&state.config.encryption_key)?;

    let token = user_token_service::handle_oauth_callback(
        &state.db,
        &encryption_key,
        &state.config.base_url,
        &provider_id,
        &query.code,
        &query.state,
    )
    .await?;

    audit_service::log_async(
        state.db.clone(),
        Some(token.user_id.clone()),
        "provider_token_connected".to_string(),
        Some(serde_json::json!({
            "provider_id": &provider_id,
            "token_type": "oauth2",
        })),
        None,
        None,
    );

    Ok(Json(ConnectResponse {
        status: "connected".to_string(),
        message: "OAuth connection established successfully".to_string(),
    }))
}

/// GET /api/v1/providers/callback?code=...&state=...
///
/// Generic OAuth callback that resolves the provider from the state parameter.
/// Requires session auth (AuthUser) and redirects to the frontend with status params.
pub async fn generic_oauth_callback(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Query(query): Query<GenericOAuthCallbackQuery>,
) -> axum::response::Redirect {
    let frontend_url = state.config.frontend_url.trim_end_matches('/');

    // Handle OAuth provider errors
    if let Some(ref error) = query.error {
        let msg = query
            .error_description
            .as_deref()
            .unwrap_or(error.as_str());
        audit_service::log_async(
            state.db.clone(),
            Some(auth_user.user_id.to_string()),
            "provider_oauth_callback_failed".to_string(),
            Some(serde_json::json!({
                "error": error,
                "error_description": &query.error_description,
            })),
            None,
            None,
        );
        return redirect_callback(frontend_url, "error", Some(msg));
    }

    let code = match query.code.as_deref() {
        Some(c) if !c.is_empty() => c,
        _ => {
            return redirect_callback(
                frontend_url,
                "error",
                Some("Missing authorization code"),
            );
        }
    };
    let state_param = match query.state.as_deref() {
        Some(s) if !s.is_empty() => s,
        _ => {
            return redirect_callback(
                frontend_url,
                "error",
                Some("Missing state parameter"),
            );
        }
    };

    let encryption_key = match aes::parse_hex_key(&state.config.encryption_key) {
        Ok(k) => k,
        Err(_) => {
            return redirect_callback(frontend_url, "error", Some("Internal server error"));
        }
    };

    // Peek at the OAuth state to find the provider_id and verify user ownership
    let oauth_state = match user_token_service::peek_oauth_state(&state.db, state_param).await {
        Ok(s) => s,
        Err(e) => {
            audit_service::log_async(
                state.db.clone(),
                Some(auth_user.user_id.to_string()),
                "provider_oauth_callback_failed".to_string(),
                Some(serde_json::json!({ "error": e.to_string() })),
                None,
                None,
            );
            return redirect_callback(frontend_url, "error", Some("Invalid or expired OAuth state"));
        }
    };

    // Verify the session user matches the state's user_id
    let user_id_str = auth_user.user_id.to_string();
    if oauth_state.user_id != user_id_str {
        audit_service::log_async(
            state.db.clone(),
            Some(user_id_str.clone()),
            "provider_oauth_callback_failed".to_string(),
            Some(serde_json::json!({ "error": "user_id mismatch" })),
            None,
            None,
        );
        return redirect_callback(frontend_url, "error", Some("Session mismatch"));
    }

    let provider_id = &oauth_state.provider_config_id;

    match user_token_service::handle_oauth_callback(
        &state.db,
        &encryption_key,
        &state.config.base_url,
        provider_id,
        code,
        state_param,
    )
    .await
    {
        Ok(token) => {
            audit_service::log_async(
                state.db.clone(),
                Some(token.user_id.clone()),
                "provider_token_connected".to_string(),
                Some(serde_json::json!({
                    "provider_id": provider_id,
                    "token_type": "oauth2",
                })),
                None,
                None,
            );
            redirect_callback(frontend_url, "success", None)
        }
        Err(e) => {
            audit_service::log_async(
                state.db.clone(),
                Some(user_id_str),
                "provider_oauth_callback_failed".to_string(),
                Some(serde_json::json!({
                    "provider_id": provider_id,
                    "error": e.to_string(),
                })),
                None,
                None,
            );
            redirect_callback(frontend_url, "error", Some(&e.to_string()))
        }
    }
}

/// Build a redirect URL to the frontend callback page with status params.
fn redirect_callback(
    frontend_url: &str,
    status: &str,
    message: Option<&str>,
) -> axum::response::Redirect {
    let mut url = url::Url::parse(&format!("{frontend_url}/providers/callback"))
        .expect("frontend_url should be a valid URL");
    url.query_pairs_mut().append_pair("status", status);
    if let Some(msg) = message {
        url.query_pairs_mut().append_pair("message", msg);
    }
    axum::response::Redirect::to(url.as_str())
}

/// DELETE /api/v1/providers/{provider_id}/disconnect
pub async fn disconnect_provider(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(provider_id): Path<String>,
) -> AppResult<Json<ConnectResponse>> {
    let user_id_str = auth_user.user_id.to_string();

    user_token_service::disconnect_provider(&state.db, &user_id_str, &provider_id).await?;

    audit_service::log_async(
        state.db.clone(),
        Some(user_id_str),
        "provider_token_disconnected".to_string(),
        Some(serde_json::json!({ "provider_id": &provider_id })),
        None,
        None,
    );

    Ok(Json(ConnectResponse {
        status: "disconnected".to_string(),
        message: "Provider disconnected and credentials removed".to_string(),
    }))
}

/// POST /api/v1/providers/{provider_id}/refresh
pub async fn manual_refresh(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(provider_id): Path<String>,
) -> AppResult<Json<ConnectResponse>> {
    let user_id_str = auth_user.user_id.to_string();
    let encryption_key = aes::parse_hex_key(&state.config.encryption_key)?;

    // Attempt to get active token (which triggers lazy refresh for expired OAuth tokens)
    user_token_service::get_active_token(
        &state.db,
        &encryption_key,
        &user_id_str,
        &provider_id,
    )
    .await?;

    audit_service::log_async(
        state.db.clone(),
        Some(user_id_str),
        "provider_token_refreshed".to_string(),
        Some(serde_json::json!({ "provider_id": &provider_id })),
        None,
        None,
    );

    Ok(Json(ConnectResponse {
        status: "refreshed".to_string(),
        message: "Token refreshed successfully".to_string(),
    }))
}

/// POST /api/v1/providers/{provider_id}/connect/device-code/initiate
///
/// RFC 8628 Step 1: Request a device code from the provider.
/// Returns user_code & verification_uri for the user to authenticate in their browser.
pub async fn request_device_code(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(provider_id): Path<String>,
) -> AppResult<Json<DeviceCodeInitiateResponse>> {
    let user_id_str = auth_user.user_id.to_string();
    let encryption_key = aes::parse_hex_key(&state.config.encryption_key)?;

    let result = user_token_service::request_device_code(
        &state.db,
        &encryption_key,
        &user_id_str,
        &provider_id,
    )
    .await?;

    audit_service::log_async(
        state.db.clone(),
        Some(user_id_str),
        "provider_device_code_initiated".to_string(),
        Some(serde_json::json!({ "provider_id": &provider_id })),
        None,
        None,
    );

    Ok(Json(DeviceCodeInitiateResponse {
        user_code: result.user_code,
        verification_uri: result.verification_uri,
        state: result.state,
        expires_in: result.expires_in,
        interval: result.interval,
    }))
}

/// POST /api/v1/providers/{provider_id}/connect/device-code/poll
///
/// RFC 8628 Step 3: Poll for token completion after user authenticates.
/// Returns status: "pending", "slow_down", "expired", "denied", or "complete".
pub async fn poll_device_code(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(provider_id): Path<String>,
    Json(body): Json<DeviceCodePollRequest>,
) -> AppResult<Json<DeviceCodePollResponse>> {
    let user_id_str = auth_user.user_id.to_string();
    let encryption_key = aes::parse_hex_key(&state.config.encryption_key)?;

    let result = user_token_service::poll_device_code(
        &state.db,
        &encryption_key,
        &user_id_str,
        &provider_id,
        &body.state,
    )
    .await?;

    if result.status == "complete" {
        audit_service::log_async(
            state.db.clone(),
            Some(user_id_str),
            "provider_token_connected".to_string(),
            Some(serde_json::json!({
                "provider_id": &provider_id,
                "token_type": "device_code",
            })),
            None,
            None,
        );
    }

    Ok(Json(DeviceCodePollResponse {
        status: result.status,
        interval: result.interval,
    }))
}

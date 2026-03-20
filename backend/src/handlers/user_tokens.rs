use std::collections::HashMap;

use axum::{
    Form, Json,
    extract::{Path, Query, State},
};
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::crypto::telegram::TelegramLoginData;
use crate::errors::{AppError, AppResult};
use crate::mw::auth::{AuthUser, OptionalAuthUser};
use crate::services::{audit_service, user_token_service};

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
    pub metadata: Option<serde_json::Value>,
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

#[derive(Debug, Serialize)]
pub struct TelegramWidgetConnectResponse {
    pub bot_username: String,
    pub callback_url: String,
    pub redirect_url: String,
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
            provider_type: s.provider_type,
            status: s.status,
            label: s.label,
            expires_at: s.expires_at,
            last_used_at: s.last_used_at,
            connected_at: s.connected_at,
            metadata: s.metadata,
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
        &state.encryption_keys,
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

/// GET /api/v1/providers/{provider_id}/connect/telegram
pub async fn get_telegram_widget_config(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(provider_id): Path<String>,
) -> AppResult<Json<TelegramWidgetConnectResponse>> {
    let user_id_str = auth_user.user_id.to_string();

    let config = user_token_service::get_telegram_widget_connect_config(
        &state.db,
        &state.encryption_keys,
        &state.config.frontend_url,
        &state.config.base_url,
        &user_id_str,
        &provider_id,
    )
    .await?;

    audit_service::log_async(
        state.db.clone(),
        Some(user_id_str),
        "provider_telegram_widget_requested".to_string(),
        Some(serde_json::json!({ "provider_id": &provider_id })),
        None,
        None,
    );

    Ok(Json(TelegramWidgetConnectResponse {
        bot_username: config.bot_username,
        callback_url: config.callback_url,
        redirect_url: config.redirect_url,
    }))
}

/// POST /api/v1/providers/{provider_id}/connect/telegram/callback
pub async fn telegram_widget_callback(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(provider_id): Path<String>,
    Json(body): Json<TelegramLoginData>,
) -> AppResult<Json<ConnectResponse>> {
    let user_id_str = auth_user.user_id.to_string();

    let token = user_token_service::handle_telegram_widget_callback(
        &state.db,
        &state.encryption_keys,
        &user_id_str,
        &provider_id,
        &body,
    )
    .await?;

    audit_service::log_async(
        state.db.clone(),
        Some(token.user_id.clone()),
        "provider_token_connected".to_string(),
        Some(serde_json::json!({
            "provider_id": &provider_id,
            "token_type": "telegram_identity",
        })),
        None,
        None,
    );

    Ok(Json(ConnectResponse {
        status: "connected".to_string(),
        message: "Telegram identity connected successfully".to_string(),
    }))
}

/// GET /api/v1/providers/{provider_id}/connect/oauth
pub async fn initiate_oauth_connect(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(provider_id): Path<String>,
) -> AppResult<Json<OAuthInitiateResponse>> {
    let user_id_str = auth_user.user_id.to_string();

    let auth_url = user_token_service::initiate_oauth_connect(
        &state.db,
        &state.encryption_keys,
        &state.config.base_url,
        &user_id_str,
        &provider_id,
        None,
        None,
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
    let token = user_token_service::handle_oauth_callback(
        &state.db,
        &state.encryption_keys,
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
/// If the browser still has an active session cookie, the callback verifies it
/// matches the initiating user. Otherwise it relies on the one-time OAuth state.
pub async fn generic_oauth_callback(
    State(state): State<AppState>,
    opt_auth_user: OptionalAuthUser,
    Query(query): Query<GenericOAuthCallbackQuery>,
) -> axum::response::Redirect {
    generic_oauth_callback_impl(state, opt_auth_user.0, query).await
}

async fn generic_oauth_callback_impl(
    state: AppState,
    auth_user: Option<AuthUser>,
    query: GenericOAuthCallbackQuery,
) -> axum::response::Redirect {
    let frontend_url = state.config.frontend_url.trim_end_matches('/');

    // Handle OAuth provider errors
    if let Some(ref error) = query.error {
        let msg = query.error_description.as_deref().unwrap_or(error.as_str());
        audit_service::log_async(
            state.db.clone(),
            auth_user.as_ref().map(|u| u.user_id.to_string()),
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
            return redirect_callback(frontend_url, "error", Some("Missing authorization code"));
        }
    };
    let state_param = match query.state.as_deref() {
        Some(s) if !s.is_empty() => s,
        _ => {
            return redirect_callback(frontend_url, "error", Some("Missing state parameter"));
        }
    };

    // Peek at the OAuth state to find the provider_id and verify user ownership
    let oauth_state = match user_token_service::peek_oauth_state(&state.db, state_param).await {
        Ok(s) => s,
        Err(e) => {
            audit_service::log_async(
                state.db.clone(),
                auth_user.as_ref().map(|u| u.user_id.to_string()),
                "provider_oauth_callback_failed".to_string(),
                Some(serde_json::json!({ "error": e.to_string() })),
                None,
                None,
            );
            return redirect_callback(
                frontend_url,
                "error",
                Some("Invalid or expired OAuth state"),
            );
        }
    };

    if let Err(e) = ensure_callback_user_matches_state(auth_user.as_ref(), &oauth_state.user_id) {
        audit_service::log_async(
            state.db.clone(),
            auth_user.as_ref().map(|u| u.user_id.to_string()),
            "provider_oauth_callback_failed".to_string(),
            Some(serde_json::json!({ "error": e.to_string() })),
            None,
            None,
        );
        return redirect_callback(frontend_url, "error", Some("Session mismatch"));
    }

    let provider_id = &oauth_state.provider_config_id;
    let redirect_path = oauth_state.redirect_path.clone();

    match user_token_service::handle_oauth_callback(
        &state.db,
        &state.encryption_keys,
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
                    "on_behalf_of": &oauth_state.target_user_id,
                })),
                None,
                None,
            );
            if let Some(ref path) = redirect_path {
                redirect_to_path(frontend_url, path, "success", None)
            } else {
                redirect_callback(frontend_url, "success", None)
            }
        }
        Err(e) => {
            audit_service::log_async(
                state.db.clone(),
                Some(oauth_state.user_id.clone()),
                "provider_oauth_callback_failed".to_string(),
                Some(serde_json::json!({
                    "provider_id": provider_id,
                    "error": e.to_string(),
                    "on_behalf_of": &oauth_state.target_user_id,
                })),
                None,
                None,
            );
            // Sanitize error for user-facing redirect -- never leak internal details
            let user_msg = safe_error_message(&e);
            if let Some(ref path) = redirect_path {
                redirect_to_path(frontend_url, path, "error", Some(&user_msg))
            } else {
                redirect_callback(frontend_url, "error", Some(&user_msg))
            }
        }
    }
}

/// POST /api/v1/providers/callback
///
/// Handles OAuth callbacks from providers that use response_mode=form_post (e.g., Apple).
/// Reads code and state from the form body instead of query params. Session
/// cookies are optional here because cross-site POST callbacks may omit Lax cookies.
pub async fn generic_oauth_callback_post(
    State(state): State<AppState>,
    opt_auth_user: OptionalAuthUser,
    Form(params): Form<HashMap<String, String>>,
) -> axum::response::Redirect {
    let query = GenericOAuthCallbackQuery {
        code: params.get("code").cloned(),
        state: params.get("state").cloned(),
        error: params.get("error").cloned(),
        error_description: params.get("error_description").cloned(),
    };
    generic_oauth_callback_impl(state, opt_auth_user.0, query).await
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

/// Build a redirect URL to a custom frontend path with provider_status params.
/// Used for admin-on-behalf flows that redirect back to the SA detail page.
fn redirect_to_path(
    frontend_url: &str,
    path: &str,
    status: &str,
    message: Option<&str>,
) -> axum::response::Redirect {
    let mut url = url::Url::parse(&format!("{frontend_url}{path}"))
        .expect("frontend_url + path should be a valid URL");
    url.query_pairs_mut().append_pair("provider_status", status);
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

    user_token_service::disconnect_provider(
        &state.db,
        &state.encryption_keys,
        &user_id_str,
        &provider_id,
    )
    .await?;

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

    // Attempt to get active token (which triggers lazy refresh for expired OAuth tokens)
    user_token_service::get_active_token(
        &state.db,
        &state.encryption_keys,
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

    let result = user_token_service::request_device_code(
        &state.db,
        &state.encryption_keys,
        &user_id_str,
        &provider_id,
        None,
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

    let result = user_token_service::poll_device_code(
        &state.db,
        &state.encryption_keys,
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

/// Return a user-safe error message for redirects, matching the sanitization
/// applied by `AppError::into_response` for JSON errors. Internal/database
/// errors are replaced with a generic message so implementation details never
/// leak through URL query parameters.
fn safe_error_message(e: &AppError) -> String {
    match e {
        AppError::Internal(_) | AppError::DatabaseError(_) => {
            "An internal error occurred".to_string()
        }
        other => other.to_string(),
    }
}

fn ensure_callback_user_matches_state(
    auth_user: Option<&AuthUser>,
    oauth_state_user_id: &str,
) -> AppResult<()> {
    if let Some(auth_user) = auth_user
        && auth_user.user_id.to_string() != oauth_state_user_id
    {
        return Err(AppError::BadRequest("Session mismatch".to_string()));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mw::auth::AuthMethod;
    use uuid::Uuid;

    fn test_auth_user() -> AuthUser {
        AuthUser {
            user_id: Uuid::new_v4(),
            session_id: None,
            scope: String::new(),
            acting_client_id: None,
            approval_owner_user_id: None,
            auth_method: AuthMethod::Session,
        }
    }

    #[test]
    fn callback_state_allows_missing_session_cookie() {
        assert!(ensure_callback_user_matches_state(None, "user-123").is_ok());
    }

    #[test]
    fn callback_state_accepts_matching_session_user() {
        let auth_user = test_auth_user();
        assert!(
            ensure_callback_user_matches_state(Some(&auth_user), &auth_user.user_id.to_string())
                .is_ok()
        );
    }

    #[test]
    fn callback_state_rejects_mismatched_session_user() {
        let auth_user = test_auth_user();
        let err = ensure_callback_user_matches_state(Some(&auth_user), "other-user")
            .expect_err("mismatched session should fail");

        assert!(matches!(err, AppError::BadRequest(message) if message == "Session mismatch"));
    }
}

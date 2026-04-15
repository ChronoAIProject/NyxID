use std::collections::HashMap;

use axum::{
    Form, Json,
    extract::{Path, Query, State},
};
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::mw::auth::{AuthUser, OptionalAuthUser};
use crate::services::url_validation::validate_base_url;
use crate::services::{
    audit_service, credential_push_service, org_service, provider_service, user_api_key_service,
    user_token_service,
};

// TODO(SEC-9): Apply stricter per-endpoint rate limiting to OAuth callback and
// initiate endpoints (e.g. 10 requests/minute per user) instead of relying
// solely on the global rate limiter.

// --- Request / Response types ---

#[derive(Deserialize)]
pub struct ConnectApiKeyRequest {
    pub api_key: String,
    pub label: Option<String>,
    /// Per-user gateway URL for self-hosted providers (e.g., OpenClaw).
    pub gateway_url: Option<String>,
}

impl std::fmt::Debug for ConnectApiKeyRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConnectApiKeyRequest")
            .field("api_key", &"[REDACTED]")
            .field("label", &self.label)
            .field("gateway_url", &self.gateway_url)
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
    pub gateway_url: Option<String>,
    pub expires_at: Option<String>,
    pub last_used_at: Option<String>,
    pub connected_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<HashMap<String, String>>,
}

#[derive(Debug, Serialize)]
pub struct UserTokenListResponse {
    pub tokens: Vec<UserTokenResponse>,
}

#[derive(Debug, Serialize)]
pub struct OAuthInitiateResponse {
    pub authorization_url: String,
}

#[derive(Debug, Deserialize, Default)]
pub struct OAuthInitiateQuery {
    pub redirect_path: Option<String>,
    /// Optional comma- or space-separated list of additional OAuth scopes
    /// to append to the provider's `default_scopes` when building the
    /// authorization URL. The upstream provider decides whether to grant them.
    pub scope: Option<String>,
    /// When set, initiate the OAuth flow on behalf of an org. The resulting
    /// `UserProviderToken` is stored under the org's user_id so that every
    /// member of the org can proxy through the resulting credential. The
    /// caller must be an admin of the org.
    pub target_org_id: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct DeviceCodeInitiateQuery {
    /// Optional comma- or space-separated list of additional OAuth scopes
    /// to append to the provider's `default_scopes` when requesting the
    /// device code.
    pub scope: Option<String>,
    /// When set, initiate the device-code flow on behalf of an org. The
    /// resulting token is stored under the org's user_id. The caller must
    /// be an admin of the org. See [`OAuthInitiateQuery::target_org_id`].
    pub target_org_id: Option<String>,
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
            provider_type: s.provider_type,
            status: s.status,
            label: s.label,
            gateway_url: s.gateway_url,
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

    // Validate gateway_url if provided: must be a valid URL, SSRF-safe
    if let Some(ref url) = body.gateway_url {
        if url.is_empty() {
            return Err(AppError::ValidationError(
                "gateway_url must not be empty when provided".to_string(),
            ));
        }
        if url.len() > 2048 {
            return Err(AppError::ValidationError(
                "gateway_url exceeds maximum length".to_string(),
            ));
        }
        validate_base_url(url)?;
    }

    // If provider requires a gateway URL, enforce it
    let provider = provider_service::get_provider(&state.db, &provider_id).await?;
    if provider.requires_gateway_url && body.gateway_url.is_none() {
        return Err(AppError::ValidationError(
            "This provider requires a gateway URL (your self-hosted instance URL)".to_string(),
        ));
    }

    user_token_service::store_api_key(
        &state.db,
        &state.encryption_keys,
        &user_id_str,
        &provider_id,
        &body.api_key,
        body.label.as_deref(),
        body.gateway_url.as_deref(),
    )
    .await?;
    sync_provider_credentials_to_unified_keys(&state, &user_id_str, &provider_id, true).await?;

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
        None,
        None,
    );

    Ok(Json(ConnectResponse {
        status: "connected".to_string(),
        message: "API key stored successfully".to_string(),
    }))
}

/// Verify that the caller is an admin of the given org and return the org's
/// user_id so OAuth state plumbing can store the token on its behalf.
///
/// Returns `Ok(None)` when `target_org_id` is not set. Errors out with
/// `OrgRoleInsufficient` if the caller is not an admin of the target org.
async fn resolve_oauth_target_org(
    state: &AppState,
    actor: &str,
    target_org_id: Option<&str>,
) -> AppResult<Option<String>> {
    let Some(target) = target_org_id else {
        return Ok(None);
    };
    let access = org_service::resolve_owner_access(&state.db, actor, target).await?;
    if !access.can_write() {
        return Err(AppError::OrgRoleInsufficient(
            "you must be an admin of the target org to initiate OAuth on its behalf".to_string(),
        ));
    }
    Ok(Some(target.to_string()))
}

/// GET /api/v1/providers/{provider_id}/connect/oauth
pub async fn initiate_oauth_connect(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(provider_id): Path<String>,
    Query(query): Query<OAuthInitiateQuery>,
) -> AppResult<Json<OAuthInitiateResponse>> {
    let user_id_str = auth_user.user_id.to_string();
    if let Some(ref redirect_path) = query.redirect_path {
        validate_redirect_path(redirect_path)?;
    }
    let additional_scopes = user_token_service::parse_additional_scopes(query.scope.as_deref())?;

    // Optional org-targeted flow. When set, the admin is initiating OAuth
    // on behalf of the org -- the resulting token lives under the org's
    // user_id and is visible to every org member via the proxy fallback.
    let target_org_user_id =
        resolve_oauth_target_org(&state, &user_id_str, query.target_org_id.as_deref()).await?;

    let auth_url = user_token_service::initiate_oauth_connect(
        &state.db,
        &state.encryption_keys,
        &state.config.base_url,
        &user_id_str,
        &provider_id,
        target_org_user_id.as_deref(),
        query.redirect_path.as_deref(),
        &additional_scopes,
    )
    .await?;

    audit_service::log_async(
        state.db.clone(),
        Some(user_id_str),
        "provider_oauth_initiated".to_string(),
        Some(serde_json::json!({
            "provider_id": &provider_id,
            "additional_scope_count": additional_scopes.len(),
        })),
        None,
        None,
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
                None,
                None,
            );

            if let Err(error) =
                sync_provider_credentials_to_unified_keys(&state, &token.user_id, provider_id, true)
                    .await
            {
                audit_service::log_async(
                    state.db.clone(),
                    Some(token.user_id.clone()),
                    "provider_oauth_callback_failed".to_string(),
                    Some(serde_json::json!({
                        "provider_id": provider_id,
                        "error": error.to_string(),
                        "reason": "failed_to_sync_unified_keys",
                    })),
                    None,
                    None,
                    None,
                    None,
                );
                let user_msg = safe_error_message(&error);
                if let Some(ref path) = redirect_path {
                    return redirect_to_path(frontend_url, path, "error", Some(&user_msg));
                }
                return redirect_callback(frontend_url, "error", Some(&user_msg));
            }

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
    sync_provider_credentials_to_unified_keys(&state, &user_id_str, &provider_id, false).await?;

    audit_service::log_async(
        state.db.clone(),
        Some(user_id_str),
        "provider_token_disconnected".to_string(),
        Some(serde_json::json!({ "provider_id": &provider_id })),
        None,
        None,
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
    sync_provider_credentials_to_unified_keys(&state, &user_id_str, &provider_id, true).await?;

    audit_service::log_async(
        state.db.clone(),
        Some(user_id_str),
        "provider_token_refreshed".to_string(),
        Some(serde_json::json!({ "provider_id": &provider_id })),
        None,
        None,
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
    Query(query): Query<DeviceCodeInitiateQuery>,
) -> AppResult<Json<DeviceCodeInitiateResponse>> {
    let user_id_str = auth_user.user_id.to_string();
    let additional_scopes = user_token_service::parse_additional_scopes(query.scope.as_deref())?;

    // See `initiate_oauth_connect` for the org-targeting contract.
    let target_org_user_id =
        resolve_oauth_target_org(&state, &user_id_str, query.target_org_id.as_deref()).await?;

    let result = user_token_service::request_device_code(
        &state.db,
        &state.encryption_keys,
        &user_id_str,
        &provider_id,
        target_org_user_id.as_deref(),
        &additional_scopes,
    )
    .await?;

    audit_service::log_async(
        state.db.clone(),
        Some(user_id_str),
        "provider_device_code_initiated".to_string(),
        Some(serde_json::json!({
            "provider_id": &provider_id,
            "additional_scope_count": additional_scopes.len(),
        })),
        None,
        None,
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
        sync_provider_credentials_to_unified_keys(&state, &user_id_str, &provider_id, true).await?;
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
            None,
            None,
        );
    }

    Ok(Json(DeviceCodePollResponse {
        status: result.status,
        interval: result.interval,
    }))
}

// --- Telegram Login Widget types & handlers ---

#[derive(Debug, Serialize)]
pub struct TelegramConnectConfigResponse {
    pub bot_username: String,
}

/// GET /api/v1/providers/{provider_id}/connect/telegram
///
/// Returns the Telegram bot username needed to render the Login Widget on the
/// frontend.
pub async fn get_telegram_connect_config(
    State(state): State<AppState>,
    _auth_user: AuthUser,
    Path(provider_id): Path<String>,
) -> AppResult<Json<TelegramConnectConfigResponse>> {
    let bot_username =
        user_token_service::get_telegram_connect_bot_username(&state.db, &provider_id).await?;

    Ok(Json(TelegramConnectConfigResponse { bot_username }))
}

/// POST /api/v1/providers/{provider_id}/connect/telegram/callback
///
/// Receives Telegram Login Widget data, verifies the HMAC-SHA256 signature,
/// and stores the verified identity as a `telegram_identity` token.
pub async fn telegram_callback(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(provider_id): Path<String>,
    Json(body): Json<crate::crypto::telegram::TelegramLoginData>,
) -> AppResult<Json<ConnectResponse>> {
    let user_id_str = auth_user.user_id.to_string();

    user_token_service::connect_telegram_widget(
        &state.db,
        &state.encryption_keys,
        &user_id_str,
        &provider_id,
        &body,
    )
    .await?;

    audit_service::log_async(
        state.db.clone(),
        Some(user_id_str),
        "provider_token_connected".to_string(),
        Some(serde_json::json!({
            "provider_id": &provider_id,
            "token_type": "telegram_identity",
            "telegram_user_id": body.id,
        })),
        None,
        None,
        None,
        None,
    );

    Ok(Json(ConnectResponse {
        status: "connected".to_string(),
        message: "Telegram identity verified and stored".to_string(),
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

async fn sync_provider_credentials_to_unified_keys(
    state: &AppState,
    user_id: &str,
    provider_id: &str,
    push_to_nodes: bool,
) -> AppResult<()> {
    user_api_key_service::sync_provider_token_to_api_keys(&state.db, user_id, provider_id).await?;

    if push_to_nodes {
        let db = state.db.clone();
        let enc = state.encryption_keys.clone();
        let ws = state.node_ws_manager.clone();
        let uid = user_id.to_string();
        let pid = provider_id.to_string();
        tokio::spawn(async move {
            credential_push_service::push_oauth_credential_to_nodes(&db, &enc, &ws, &uid, &pid)
                .await;
        });
    }

    Ok(())
}

fn validate_redirect_path(path: &str) -> AppResult<()> {
    if path.is_empty() || !path.starts_with('/') || path.starts_with("//") {
        return Err(AppError::ValidationError(
            "redirect_path must be a frontend path beginning with '/'".to_string(),
        ));
    }
    if path.contains('\r') || path.contains('\n') {
        return Err(AppError::ValidationError(
            "redirect_path must not contain control characters".to_string(),
        ));
    }

    Ok(())
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
            allow_all_services: true,
            allow_all_nodes: true,
            allowed_service_ids: vec![],
            allowed_node_ids: vec![],
            api_key_id: None,
            api_key_name: None,
            rate_limit_per_second: None,
            rate_limit_burst: None,
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

    #[test]
    fn user_token_response_serializes_telegram_metadata() {
        let mut metadata = HashMap::new();
        metadata.insert("telegram_user_id".to_string(), "12345".to_string());
        metadata.insert("first_name".to_string(), "Nyx".to_string());
        metadata.insert("username".to_string(), "nyx_user".to_string());
        metadata.insert(
            "photo_url".to_string(),
            "https://t.me/i/userpic/photo.jpg".to_string(),
        );

        let response = UserTokenResponse {
            provider_id: "provider-1".to_string(),
            provider_name: "Telegram".to_string(),
            provider_slug: "telegram".to_string(),
            provider_type: "telegram_widget".to_string(),
            status: "active".to_string(),
            label: None,
            gateway_url: None,
            expires_at: None,
            last_used_at: None,
            connected_at: "2026-01-01T00:00:00Z".to_string(),
            metadata: Some(metadata),
        };

        let json = serde_json::to_value(&response).expect("serialization");

        assert_eq!(json["provider_type"], "telegram_widget");
        assert_eq!(json["metadata"]["telegram_user_id"], "12345");
        assert_eq!(json["metadata"]["username"], "nyx_user");
        assert_eq!(
            json["metadata"]["photo_url"],
            "https://t.me/i/userpic/photo.jpg"
        );
        assert_eq!(json["metadata"]["first_name"], "Nyx");
    }

    #[test]
    fn user_token_response_omits_metadata_when_none() {
        let response = UserTokenResponse {
            provider_id: "provider-1".to_string(),
            provider_name: "GitHub".to_string(),
            provider_slug: "github".to_string(),
            provider_type: "oauth2".to_string(),
            status: "active".to_string(),
            label: None,
            gateway_url: None,
            expires_at: None,
            last_used_at: None,
            connected_at: "2026-01-01T00:00:00Z".to_string(),
            metadata: None,
        };

        let json = serde_json::to_value(&response).expect("serialization");

        assert!(json.get("metadata").is_none());
    }
}

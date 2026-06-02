use std::collections::HashMap;

use axum::{
    Form, Json,
    extract::{Path, Query, State},
    http::HeaderMap,
};
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::mw::auth::{AuthUser, OptionalAuthUser};
use crate::services::url_validation::validate_base_url;
use crate::services::{
    admin_user_service, audit_service, credential_push_service, org_service, provider_service,
    user_api_key_service, user_service_service, user_token_service,
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
    /// Multi-connection: the placeholder this OAuth flow is authorizing,
    /// identified by the `POST /keys` response `id` (a `UserService` id).
    /// When set, the handler resolves it to the linked `UserApiKey`'s
    /// `connection_id` and threads that into the `OAuthState` so the
    /// callback writes the token directly onto that key (instead of the
    /// legacy `user_provider_tokens` path). Set by the wizard for every
    /// multi-connection add; absent for legacy provider-connect flows.
    pub key_id: Option<String>,
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
    /// Multi-connection: the placeholder this device-code flow is
    /// authorizing (a `UserService` id). See [`OAuthInitiateQuery::key_id`].
    pub key_id: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct ProviderTokenTargetQuery {
    /// When set, operate on an org-owned provider token. The caller must be an
    /// admin of the org.
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
    Query(query): Query<ProviderTokenTargetQuery>,
) -> AppResult<Json<UserTokenListResponse>> {
    let user_id_str = auth_user.user_id.to_string();
    let target_org_user_id =
        resolve_oauth_target_org(&state, &user_id_str, query.target_org_id.as_deref()).await?;
    let effective_user_id = target_org_user_id.as_deref().unwrap_or(&user_id_str);

    let summaries = user_token_service::list_user_tokens(&state.db, effective_user_id).await?;

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

    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "provider_token_connected",
        Some(serde_json::json!({
            "provider_id": &provider_id,
            "token_type": "api_key",
        })),
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
            "you must be an admin of the target org to manage provider tokens on its behalf"
                .to_string(),
        ));
    }
    Ok(Some(target.to_string()))
}

/// Multi-connection: resolve the `connection_id` for the placeholder the
/// wizard is authorizing, so the OAuth / device-code callback can scope
/// its token write to that one `UserApiKey` (`write_oauth_tokens_to_key`)
/// instead of the legacy `user_provider_tokens` path.
///
/// The wizard's `key_id` query param carries the `POST /keys` response
/// `id`, which is the **`UserService`** id (the `UserApiKey` id is the
/// separate `api_key_id` field on that response). So this resolves
/// `key_id` as a `UserService` id first — looking up the service, then
/// reading `connection_id` off its linked `UserApiKey`. As a defensive
/// fallback — and to honor the original param contract — if no
/// `UserService` matches it also tries `key_id` as a `UserApiKey` id
/// directly, so any caller passing a `UserApiKey` id still works.
///
/// `owner_id` must be the effective owner of the placeholder — the org
/// user_id for org-scoped adds, otherwise the caller. Both lookups are
/// owner-scoped. Returns `None` (legacy path) when `key_id` is absent,
/// nothing resolves under `owner_id`, or the resolved key carries no
/// `connection_id`. A `None` result is always safe: the callback simply
/// takes the legacy write path.
async fn resolve_connection_id_for_key(
    state: &AppState,
    owner_id: &str,
    key_id: Option<&str>,
) -> Option<String> {
    let key_id = key_id?;
    // Primary path: `key_id` is a `UserService` id (what both wizard
    // frontends send). Resolve service -> its `api_key_id` -> that
    // `UserApiKey`'s `connection_id`.
    if let Ok(service) = user_service_service::get_user_service(&state.db, owner_id, key_id).await
        && let Some(api_key_id) = service.api_key_id.as_deref()
        && let Ok(key) = user_api_key_service::get_api_key(&state.db, owner_id, api_key_id).await
    {
        return key.connection_id;
    }
    // Fallback: `key_id` may already be a `UserApiKey` id (the original
    // param contract). Preserved so no existing caller regresses.
    user_api_key_service::get_api_key(&state.db, owner_id, key_id)
        .await
        .ok()
        .and_then(|key| key.connection_id)
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

    // Multi-connection: if the wizard passed the placeholder `key_id`,
    // thread its `connection_id` so the callback writes the token onto
    // that key. `None` (legacy provider-connect flows) keeps the
    // `user_provider_tokens` write path.
    let effective_owner = target_org_user_id.as_deref().unwrap_or(&user_id_str);
    let connection_id =
        resolve_connection_id_for_key(&state, effective_owner, query.key_id.as_deref()).await;

    let auth_url = user_token_service::initiate_oauth_connect(
        &state.db,
        &state.encryption_keys,
        &state.config.base_url,
        &user_id_str,
        &provider_id,
        target_org_user_id.as_deref(),
        query.redirect_path.as_deref(),
        &additional_scopes,
        connection_id.as_deref(),
    )
    .await?;

    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "provider_oauth_initiated",
        Some(serde_json::json!({
            "provider_id": &provider_id,
            "additional_scope_count": additional_scopes.len(),
        })),
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
    headers: HeaderMap,
) -> AppResult<Json<ConnectResponse>> {
    let outcome = user_token_service::handle_oauth_callback(
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
        Some(outcome.user_id.clone()),
        "provider_token_connected".to_string(),
        Some(serde_json::json!({
            "provider_id": &provider_id,
            "token_type": "oauth2",
            // Surfaces which write path executed: a UUID for multi-
            // connection, null for the legacy single-tenant path.
            "connection_id": &outcome.connection_id,
        })),
        crate::handlers::admin_helpers::extract_ip(&headers),
        crate::handlers::admin_helpers::extract_user_agent(&headers),
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
        let msg = safe_provider_error_message(error, query.error_description.as_deref());

        let mut failed_placeholders = 0_u64;
        let mut state_lookup_error: Option<String> = None;
        if let Some(state_param) = query.state.as_deref().filter(|s| !s.is_empty()) {
            match user_token_service::peek_oauth_state(&state.db, state_param).await {
                Ok(oauth_state) => {
                    let owner_id = oauth_state
                        .target_user_id
                        .as_deref()
                        .unwrap_or(&oauth_state.user_id);
                    match user_api_key_service::fail_oauth_placeholders(
                        &state.db,
                        owner_id,
                        &oauth_state.provider_config_id,
                        oauth_state.connection_id.as_deref(),
                        &msg,
                    )
                    .await
                    {
                        Ok(count) => failed_placeholders = count,
                        Err(e) => state_lookup_error = Some(e.to_string()),
                    }
                }
                Err(e) => state_lookup_error = Some(e.to_string()),
            }
        }

        audit_service::log_async(
            state.db.clone(),
            auth_user.as_ref().map(|u| u.user_id.to_string()),
            "provider_oauth_callback_failed".to_string(),
            Some(serde_json::json!({
                "error": error,
                "error_description": &query.error_description,
                "failed_placeholders": failed_placeholders,
                "state_lookup_error": state_lookup_error,
            })),
            auth_user.as_ref().and_then(|u| u.ip_address.clone()),
            auth_user.as_ref().and_then(|u| u.user_agent.clone()),
            auth_user.as_ref().and_then(|u| u.api_key_id.clone()),
            auth_user.as_ref().and_then(|u| u.api_key_name.clone()),
        );
        return redirect_callback(frontend_url, "error", Some(&msg));
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
                auth_user.as_ref().and_then(|u| u.ip_address.clone()),
                auth_user.as_ref().and_then(|u| u.user_agent.clone()),
                auth_user.as_ref().and_then(|u| u.api_key_id.clone()),
                auth_user.as_ref().and_then(|u| u.api_key_name.clone()),
            );
            return redirect_callback(
                frontend_url,
                "error",
                Some("Invalid or expired OAuth state"),
            );
        }
    };

    let provider_id = &oauth_state.provider_config_id;

    if let Err(_e) = ensure_callback_user_matches_state(auth_user.as_ref(), &oauth_state.user_id) {
        let browser_session_user_id = auth_user.as_ref().map(|u| u.user_id.to_string());
        let browser_email = match browser_session_user_id.as_deref() {
            Some(user_id) => admin_user_service::get_user_email(&state.db, user_id)
                .await
                .ok()
                .flatten(),
            None => None,
        };
        let initiator_email = admin_user_service::get_user_email(&state.db, &oauth_state.user_id)
            .await
            .ok()
            .flatten();
        let message = match (initiator_email.as_deref(), browser_email.as_deref()) {
            (Some(cli_email), Some(browser_email)) => format!(
                "This OAuth flow was started by {}, but this browser is signed in as {}. Sign out of NyxID in this browser (or switch to the CLI account) and retry.",
                mask_email(cli_email),
                mask_email(browser_email)
            ),
            _ => "This OAuth flow was started by a different NyxID account. Sign out of NyxID in this browser (or switch to the CLI account) and retry.".to_string(),
        };
        let owner_id = oauth_state
            .target_user_id
            .as_deref()
            .unwrap_or(&oauth_state.user_id);
        let failed_placeholders = match user_api_key_service::fail_oauth_placeholders(
            &state.db,
            owner_id,
            provider_id,
            oauth_state.connection_id.as_deref(),
            &message,
        )
        .await
        {
            Ok(count) => Some(count),
            Err(error) => {
                tracing::warn!(
                    user_id = %owner_id,
                    provider_id = %provider_id,
                    error = %error,
                    "failed to mark OAuth placeholders as failed after session mismatch"
                );
                None
            }
        };
        audit_service::log_async(
            state.db.clone(),
            Some(oauth_state.user_id.clone()),
            "provider_oauth_callback_failed".to_string(),
            Some(serde_json::json!({
                "provider_id": provider_id,
                // Use a fixed audit error so future detailed mismatch errors cannot leak identifiers.
                "error": "session mismatch",
                "reason": "session_mismatch",
                "browser_session_user_id": browser_session_user_id,
                "on_behalf_of": &oauth_state.target_user_id,
                "failed_placeholders": failed_placeholders,
            })),
            auth_user.as_ref().and_then(|u| u.ip_address.clone()),
            auth_user.as_ref().and_then(|u| u.user_agent.clone()),
            auth_user.as_ref().and_then(|u| u.api_key_id.clone()),
            auth_user.as_ref().and_then(|u| u.api_key_name.clone()),
        );
        return redirect_callback(frontend_url, "error", Some(&message));
    }

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
        Ok(outcome) => {
            audit_service::log_async(
                state.db.clone(),
                Some(outcome.user_id.clone()),
                "provider_token_connected".to_string(),
                Some(serde_json::json!({
                    "provider_id": provider_id,
                    "token_type": "oauth2",
                    "on_behalf_of": &oauth_state.target_user_id,
                    "connection_id": &outcome.connection_id,
                })),
                auth_user.as_ref().and_then(|u| u.ip_address.clone()),
                auth_user.as_ref().and_then(|u| u.user_agent.clone()),
                auth_user.as_ref().and_then(|u| u.api_key_id.clone()),
                auth_user.as_ref().and_then(|u| u.api_key_name.clone()),
            );

            // Multi-connection writes already populated the UserApiKey
            // directly inside `handle_oauth_callback`. The legacy fan-out
            // sync only matters when the callback wrote to
            // `user_provider_tokens` (connection_id == None).
            if outcome.connection_id.is_some() {
                // Skip legacy sync but still wake the wizard if redirect was set.
                // (No further work needed; UserApiKey already active.)
                tracing::debug!(
                    user_id = %outcome.user_id,
                    provider_id = %provider_id,
                    connection_id = ?outcome.connection_id,
                    "Multi-connection callback complete; skipping legacy sync"
                );
            } else if let Err(error) = sync_provider_credentials_to_unified_keys(
                &state,
                &outcome.user_id,
                provider_id,
                true,
            )
            .await
            {
                let user_msg = safe_error_message(&error);
                let failed_placeholders = match user_api_key_service::fail_oauth_placeholders(
                    &state.db,
                    &outcome.user_id,
                    provider_id,
                    oauth_state.connection_id.as_deref(),
                    &user_msg,
                )
                .await
                {
                    Ok(count) => Some(count),
                    Err(e) => {
                        tracing::warn!(
                            user_id = %outcome.user_id,
                            provider_id = %provider_id,
                            error = %e,
                            "failed to mark OAuth placeholders as failed after sync error"
                        );
                        None
                    }
                };
                audit_service::log_async(
                    state.db.clone(),
                    Some(outcome.user_id.clone()),
                    "provider_oauth_callback_failed".to_string(),
                    Some(serde_json::json!({
                        "provider_id": provider_id,
                        "error": error.to_string(),
                        "reason": "failed_to_sync_unified_keys",
                        "failed_placeholders": failed_placeholders,
                    })),
                    auth_user.as_ref().and_then(|u| u.ip_address.clone()),
                    auth_user.as_ref().and_then(|u| u.user_agent.clone()),
                    auth_user.as_ref().and_then(|u| u.api_key_id.clone()),
                    auth_user.as_ref().and_then(|u| u.api_key_name.clone()),
                );
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
            let owner_id = oauth_state
                .target_user_id
                .as_deref()
                .unwrap_or(&oauth_state.user_id);
            let user_msg = safe_error_message(&e);
            let failed_placeholders = match user_api_key_service::fail_oauth_placeholders(
                &state.db,
                owner_id,
                provider_id,
                oauth_state.connection_id.as_deref(),
                &user_msg,
            )
            .await
            {
                Ok(count) => Some(count),
                Err(error) => {
                    tracing::warn!(
                        user_id = %owner_id,
                        provider_id = %provider_id,
                        error = %error,
                        "failed to mark OAuth placeholders as failed"
                    );
                    None
                }
            };
            audit_service::log_async(
                state.db.clone(),
                Some(oauth_state.user_id.clone()),
                "provider_oauth_callback_failed".to_string(),
                Some(serde_json::json!({
                    "provider_id": provider_id,
                    "error": e.to_string(),
                    "on_behalf_of": &oauth_state.target_user_id,
                    "failed_placeholders": failed_placeholders,
                })),
                auth_user.as_ref().and_then(|u| u.ip_address.clone()),
                auth_user.as_ref().and_then(|u| u.user_agent.clone()),
                auth_user.as_ref().and_then(|u| u.api_key_id.clone()),
                auth_user.as_ref().and_then(|u| u.api_key_name.clone()),
            );
            // Sanitize error for user-facing redirect -- never leak internal details
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
///
/// Audit primary `user_id` is the affected token owner (`effective_user_id`);
/// org-targeted calls add `on_behalf_of` when the caller differs, matching OAuth callback events.
pub async fn disconnect_provider(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(provider_id): Path<String>,
    Query(query): Query<ProviderTokenTargetQuery>,
) -> AppResult<Json<ConnectResponse>> {
    let user_id_str = auth_user.user_id.to_string();
    let target_org_user_id =
        resolve_oauth_target_org(&state, &user_id_str, query.target_org_id.as_deref()).await?;
    let effective_user_id = target_org_user_id.as_deref().unwrap_or(&user_id_str);

    user_token_service::disconnect_provider(
        &state.db,
        &state.encryption_keys,
        effective_user_id,
        &provider_id,
    )
    .await?;
    sync_provider_credentials_to_unified_keys(&state, effective_user_id, &provider_id, false)
        .await?;

    let mut event_data = serde_json::json!({ "provider_id": &provider_id });
    if effective_user_id != user_id_str {
        event_data["on_behalf_of"] = serde_json::Value::String(effective_user_id.to_string());
    }

    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "provider_token_disconnected",
        Some(event_data),
    );

    Ok(Json(ConnectResponse {
        status: "disconnected".to_string(),
        message: "Provider disconnected and credentials removed".to_string(),
    }))
}

/// POST /api/v1/providers/{provider_id}/refresh
///
/// Audit primary `user_id` is the affected token owner (`effective_user_id`);
/// org-targeted calls add `on_behalf_of` when the caller differs, matching OAuth callback events.
pub async fn manual_refresh(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(provider_id): Path<String>,
    Query(query): Query<ProviderTokenTargetQuery>,
) -> AppResult<Json<ConnectResponse>> {
    let user_id_str = auth_user.user_id.to_string();
    let target_org_user_id =
        resolve_oauth_target_org(&state, &user_id_str, query.target_org_id.as_deref()).await?;
    let effective_user_id = target_org_user_id.as_deref().unwrap_or(&user_id_str);

    // Attempt to get active token (which triggers lazy refresh for expired OAuth tokens)
    user_token_service::get_active_token(
        &state.db,
        &state.encryption_keys,
        effective_user_id,
        &provider_id,
    )
    .await?;
    sync_provider_credentials_to_unified_keys(&state, effective_user_id, &provider_id, true)
        .await?;

    let mut event_data = serde_json::json!({ "provider_id": &provider_id });
    if effective_user_id != user_id_str {
        event_data["on_behalf_of"] = serde_json::Value::String(effective_user_id.to_string());
    }

    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "provider_token_refreshed",
        Some(event_data),
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

    // Multi-connection: thread the placeholder's `connection_id` (if the
    // wizard passed `key_id`) so the device-code completion writes the
    // token onto that key. `None` keeps the legacy `user_provider_tokens`
    // path. See `initiate_oauth_connect` for the full rationale.
    let effective_owner = target_org_user_id.as_deref().unwrap_or(&user_id_str);
    let connection_id =
        resolve_connection_id_for_key(&state, effective_owner, query.key_id.as_deref()).await;

    let result = user_token_service::request_device_code(
        &state.db,
        &state.encryption_keys,
        &user_id_str,
        &provider_id,
        target_org_user_id.as_deref(),
        &additional_scopes,
        connection_id.as_deref(),
    )
    .await?;

    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "provider_device_code_initiated",
        Some(serde_json::json!({
            "provider_id": &provider_id,
            "additional_scope_count": additional_scopes.len(),
        })),
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
///
/// Audit primary `user_id` is the affected token owner (`effective_user_id`);
/// org-targeted completions add `on_behalf_of` when the caller differs, matching OAuth callback events.
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
        let effective_user_id = result.effective_user_id.as_deref().unwrap_or(&user_id_str);
        sync_provider_credentials_to_unified_keys(&state, effective_user_id, &provider_id, true)
            .await?;
        let mut event_data = serde_json::json!({
            "provider_id": &provider_id,
            "token_type": "device_code",
        });
        if effective_user_id != user_id_str {
            event_data["on_behalf_of"] = serde_json::Value::String(effective_user_id.to_string());
        }
        audit_service::log_for_user(
            state.db.clone(),
            &auth_user,
            "provider_token_connected",
            Some(event_data),
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

    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "provider_token_connected",
        Some(serde_json::json!({
            "provider_id": &provider_id,
            "token_type": "telegram_identity",
            "telegram_user_id": body.id,
        })),
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

fn mask_email(email: &str) -> String {
    let Some((local, domain)) = email.split_once('@') else {
        return "***".to_string();
    };
    let Some(first) = local.chars().next() else {
        return "***".to_string();
    };
    if domain.is_empty() {
        return "***".to_string();
    }

    format!("{first}***@{domain}")
}

fn safe_provider_error_message(error: &str, error_description: Option<&str>) -> String {
    let message = error_description.unwrap_or(error);
    safe_error_message(&AppError::BadRequest(message.to_string()))
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
    use crate::models::audit_log::{AuditLog, COLLECTION_NAME as AUDIT_LOG};
    use crate::models::oauth_state::{COLLECTION_NAME as OAUTH_STATES, OAuthState};
    use crate::models::org_membership::COLLECTION_NAME as ORG_MEMBERSHIPS;
    use crate::models::provider_config::{COLLECTION_NAME as PROVIDER_CONFIGS, ProviderConfig};
    use crate::models::user::{COLLECTION_NAME as USERS, UserType};
    use crate::models::user_api_key::{COLLECTION_NAME as USER_API_KEYS, UserApiKey};
    use crate::models::user_provider_token::{
        COLLECTION_NAME as USER_PROVIDER_TOKENS, UserProviderToken,
    };
    use crate::mw::auth::AuthMethod;
    use crate::test_utils::{connect_test_database, test_app_state, test_membership, test_user};
    use axum::http::header::LOCATION;
    use axum::response::IntoResponse;
    use chrono::{Duration, Utc};
    use mongodb::bson::doc;
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
            ip_address: None,
            user_agent: None,
        }
    }

    fn test_provider_config(provider_id: &str) -> ProviderConfig {
        let now = Utc::now();
        ProviderConfig {
            id: provider_id.to_string(),
            slug: "github".to_string(),
            name: "GitHub".to_string(),
            description: None,
            provider_type: "oauth2".to_string(),
            authorization_url: Some("https://github.com/login/oauth/authorize".to_string()),
            token_url: Some("https://github.com/login/oauth/access_token".to_string()),
            revocation_url: None,
            default_scopes: Some(vec!["read:user".to_string()]),
            client_id_encrypted: None,
            client_secret_encrypted: Some(vec![1, 2, 3]),
            supports_pkce: false,
            device_code_url: None,
            device_token_url: None,
            device_verification_url: None,
            hosted_callback_url: None,
            api_key_instructions: None,
            api_key_url: None,
            icon_url: None,
            documentation_url: None,
            is_active: true,
            credential_mode: "admin".to_string(),
            token_endpoint_auth_method: "client_secret_post".to_string(),
            extra_auth_params: None,
            device_code_format: "rfc8628".to_string(),
            client_id_param_name: None,
            requires_gateway_url: false,
            created_by: "system".to_string(),
            created_at: now,
            updated_at: now,
        }
    }

    fn test_provider_token(token_id: &str, user_id: &str, provider_id: &str) -> UserProviderToken {
        let now = Utc::now();
        UserProviderToken {
            id: token_id.to_string(),
            user_id: user_id.to_string(),
            provider_config_id: provider_id.to_string(),
            connection_id: None,
            credential_user_id: None,
            token_type: "oauth2".to_string(),
            access_token_encrypted: Some(vec![1, 2, 3]),
            refresh_token_encrypted: Some(vec![4, 5, 6]),
            token_scopes: Some("read:user".to_string()),
            expires_at: None,
            api_key_encrypted: None,
            status: "active".to_string(),
            last_refreshed_at: None,
            last_used_at: None,
            error_message: None,
            label: None,
            metadata: None,
            gateway_url: None,
            created_at: now,
            updated_at: now,
        }
    }

    fn test_oauth_state(state_id: &str, user_id: &str, provider_id: &str) -> OAuthState {
        let now = Utc::now();
        OAuthState {
            id: state_id.to_string(),
            user_id: user_id.to_string(),
            provider_config_id: provider_id.to_string(),
            code_verifier: None,
            device_code_encrypted: None,
            user_code_encrypted: None,
            poll_interval: None,
            target_user_id: None,
            credential_user_id: None,
            redirect_path: None,
            connection_id: None,
            consumed: false,
            expires_at: now + Duration::minutes(10),
            created_at: now,
        }
    }

    fn test_pending_oauth_api_key(key_id: &str, user_id: &str, provider_id: &str) -> UserApiKey {
        let now = Utc::now();
        UserApiKey {
            id: key_id.to_string(),
            user_id: user_id.to_string(),
            label: "GitHub OAuth".to_string(),
            credential_type: "oauth2".to_string(),
            credential_encrypted: None,
            access_token_encrypted: None,
            refresh_token_encrypted: None,
            token_scopes: None,
            expires_at: None,
            provider_config_id: Some(provider_id.to_string()),
            connection_id: None,
            user_oauth_client_id_encrypted: None,
            user_oauth_client_secret_encrypted: None,
            status: "pending_auth".to_string(),
            last_used_at: None,
            error_message: None,
            source: Some("user_created".to_string()),
            source_id: None,
            created_at: now,
            updated_at: now,
        }
    }

    async fn get_api_key(db: &mongodb::Database, key_id: &str) -> UserApiKey {
        db.collection::<UserApiKey>(USER_API_KEYS)
            .find_one(mongodb::bson::doc! { "_id": key_id })
            .await
            .unwrap()
            .unwrap()
    }

    fn redirect_location(redirect: axum::response::Redirect) -> String {
        let response = redirect.into_response();
        assert!(response.status().is_redirection());
        response
            .headers()
            .get(LOCATION)
            .expect("redirect location")
            .to_str()
            .expect("valid redirect location")
            .to_string()
    }

    fn redirect_query_param(location: &str, key: &str) -> Option<String> {
        url::Url::parse(location)
            .expect("valid redirect URL")
            .query_pairs()
            .find_map(|(name, value)| (name == key).then(|| value.into_owned()))
    }

    async fn wait_for_session_mismatch_audit(
        db: &mongodb::Database,
        browser_user_id: &str,
    ) -> Option<AuditLog> {
        for _ in 0..20 {
            let found = db
                .collection::<AuditLog>(AUDIT_LOG)
                .find_one(doc! {
                    "event_type": "provider_oauth_callback_failed",
                    "event_data.reason": "session_mismatch",
                    "event_data.browser_session_user_id": browser_user_id,
                })
                .await
                .expect("query audit log");
            if found.is_some() {
                return found;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        None
    }

    async fn spawn_oauth_token_server() -> (String, tokio::task::JoinHandle<()>) {
        let app = axum::Router::new().route(
            "/token",
            axum::routing::post(|| async {
                axum::Json(serde_json::json!({
                    "access_token": "test-access-token",
                    "refresh_token": "test-refresh-token",
                    "expires_in": 3600,
                    "scope": "read:user",
                }))
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let handle = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });
        (format!("http://{addr}/token"), handle)
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
    fn mask_email_masks_local_part() {
        assert_eq!(mask_email("alice@example.com"), "a***@example.com");
        assert_eq!(mask_email("not-an-email"), "***");
        assert_eq!(mask_email(""), "***");
        assert_eq!(mask_email("a@example.com"), "a***@example.com");
        assert_eq!(mask_email("a@b@c.com"), "a***@b@c.com");
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

    #[tokio::test]
    async fn list_my_tokens_accepts_target_org_id_for_admins() {
        let Some(db) = connect_test_database("provider_tokens_org_list").await else {
            eprintln!(
                "skipping provider token handler integration test: no local MongoDB available"
            );
            return;
        };
        let state = test_app_state(db.clone());
        let admin_id = Uuid::new_v4().to_string();
        let org_user_id = Uuid::new_v4().to_string();
        let provider_id = Uuid::new_v4().to_string();
        let token_id = Uuid::new_v4().to_string();

        db.collection(USERS)
            .insert_one(test_user(&org_user_id, UserType::Org))
            .await
            .unwrap();
        db.collection(ORG_MEMBERSHIPS)
            .insert_one(test_membership(
                &org_user_id,
                &admin_id,
                crate::models::org_membership::OrgRole::Admin,
                None,
            ))
            .await
            .unwrap();
        db.collection(PROVIDER_CONFIGS)
            .insert_one(test_provider_config(&provider_id))
            .await
            .unwrap();
        db.collection(USER_PROVIDER_TOKENS)
            .insert_one(test_provider_token(&token_id, &org_user_id, &provider_id))
            .await
            .unwrap();

        let Json(response) = list_my_tokens(
            State(state),
            crate::test_utils::test_auth_user(&admin_id),
            Query(ProviderTokenTargetQuery {
                target_org_id: Some(org_user_id),
            }),
        )
        .await
        .unwrap();

        assert_eq!(response.tokens.len(), 1);
        assert_eq!(response.tokens[0].provider_id, provider_id);
        assert_eq!(response.tokens[0].provider_name, "GitHub");
    }

    #[tokio::test]
    async fn generic_oauth_callback_denial_marks_placeholder_failed() {
        let Some(db) =
            connect_test_database("oauth_callback_denial_marks_placeholder_failed").await
        else {
            eprintln!(
                "skipping provider token handler integration test: no local MongoDB available"
            );
            return;
        };
        let state = test_app_state(db.clone());
        let user_id = Uuid::new_v4().to_string();
        let provider_id = Uuid::new_v4().to_string();
        let state_id = Uuid::new_v4().to_string();
        let key_id = Uuid::new_v4().to_string();

        db.collection::<OAuthState>(OAUTH_STATES)
            .insert_one(test_oauth_state(&state_id, &user_id, &provider_id))
            .await
            .unwrap();
        db.collection::<UserApiKey>(USER_API_KEYS)
            .insert_one(test_pending_oauth_api_key(&key_id, &user_id, &provider_id))
            .await
            .unwrap();

        let redirect = generic_oauth_callback_impl(
            state,
            None,
            GenericOAuthCallbackQuery {
                code: None,
                state: Some(state_id),
                error: Some("access_denied".to_string()),
                error_description: None,
            },
        )
        .await;

        let location = redirect_location(redirect);
        assert!(location.contains("/providers/callback"));
        assert_eq!(
            redirect_query_param(&location, "status").as_deref(),
            Some("error")
        );
        assert_eq!(
            redirect_query_param(&location, "message").as_deref(),
            Some(safe_provider_error_message("access_denied", None).as_str())
        );
        let api_key = get_api_key(&db, &key_id).await;
        assert_eq!(api_key.status, "failed");
        assert_eq!(
            api_key.error_message.as_deref(),
            Some(safe_provider_error_message("access_denied", None).as_str())
        );
    }

    #[tokio::test]
    async fn generic_oauth_callback_session_mismatch_marks_placeholder_failed() {
        let Some(db) =
            connect_test_database("oauth_callback_session_mismatch_marks_placeholder_failed").await
        else {
            eprintln!(
                "skipping provider token handler integration test: no local MongoDB available"
            );
            return;
        };
        let state = test_app_state(db.clone());
        let cli_user_id = Uuid::new_v4().to_string();
        let browser_user_id = Uuid::new_v4().to_string();
        let provider_id = Uuid::new_v4().to_string();
        let state_id = Uuid::new_v4().to_string();
        let key_id = Uuid::new_v4().to_string();

        let mut cli_user = test_user(&cli_user_id, UserType::Person);
        cli_user.email = "alice@example.com".to_string();
        let mut browser_user = test_user(&browser_user_id, UserType::Person);
        browser_user.email = "bob@example.com".to_string();

        db.collection(USERS).insert_one(cli_user).await.unwrap();
        db.collection(USERS).insert_one(browser_user).await.unwrap();
        db.collection::<OAuthState>(OAUTH_STATES)
            .insert_one(test_oauth_state(&state_id, &cli_user_id, &provider_id))
            .await
            .unwrap();
        db.collection::<UserApiKey>(USER_API_KEYS)
            .insert_one(test_pending_oauth_api_key(
                &key_id,
                &cli_user_id,
                &provider_id,
            ))
            .await
            .unwrap();

        let mut auth_user = test_auth_user();
        auth_user.user_id = Uuid::parse_str(&browser_user_id).unwrap();

        let redirect = generic_oauth_callback_impl(
            state,
            Some(auth_user),
            GenericOAuthCallbackQuery {
                code: Some("oauth-code".to_string()),
                state: Some(state_id),
                error: None,
                error_description: None,
            },
        )
        .await;

        let location = redirect_location(redirect);
        assert_eq!(
            redirect_query_param(&location, "status").as_deref(),
            Some("error")
        );
        let message = redirect_query_param(&location, "message").expect("message query param");
        assert!(message.starts_with("This OAuth flow was started by"));
        assert!(message.contains("a***@example.com"));
        assert!(message.contains("b***@example.com"));

        let api_key = get_api_key(&db, &key_id).await;
        assert_eq!(api_key.status, "failed");
        assert!(
            api_key
                .error_message
                .as_deref()
                .is_some_and(|msg| !msg.is_empty())
        );

        let audit = wait_for_session_mismatch_audit(&db, &browser_user_id)
            .await
            .expect("session mismatch audit log");
        assert_eq!(audit.user_id.as_deref(), Some(cli_user_id.as_str()));
        assert_eq!(audit.event_type, "provider_oauth_callback_failed");
        let event_data = audit.event_data.expect("audit event data");
        assert_eq!(
            event_data.get("reason").and_then(|v| v.as_str()),
            Some("session_mismatch")
        );
        assert_eq!(
            event_data.get("error").and_then(|v| v.as_str()),
            Some("session mismatch")
        );
        assert_eq!(
            event_data
                .get("browser_session_user_id")
                .and_then(|v| v.as_str()),
            Some(browser_user_id.as_str())
        );
    }

    #[tokio::test]
    async fn generic_oauth_callback_session_mismatch_soft_miss_uses_fallback_message() {
        let Some(db) = connect_test_database("oauth_callback_session_mismatch_soft_miss").await
        else {
            eprintln!(
                "skipping provider token handler integration test: no local MongoDB available"
            );
            return;
        };
        let state = test_app_state(db.clone());
        let cli_user_id = Uuid::new_v4().to_string();
        let browser_user_id = Uuid::new_v4().to_string();
        let provider_id = Uuid::new_v4().to_string();
        let state_id = Uuid::new_v4().to_string();
        let key_id = Uuid::new_v4().to_string();

        let mut cli_user = test_user(&cli_user_id, UserType::Person);
        cli_user.email = "alice@example.com".to_string();

        db.collection(USERS).insert_one(cli_user).await.unwrap();
        db.collection::<OAuthState>(OAUTH_STATES)
            .insert_one(test_oauth_state(&state_id, &cli_user_id, &provider_id))
            .await
            .unwrap();
        db.collection::<UserApiKey>(USER_API_KEYS)
            .insert_one(test_pending_oauth_api_key(
                &key_id,
                &cli_user_id,
                &provider_id,
            ))
            .await
            .unwrap();

        let mut auth_user = test_auth_user();
        auth_user.user_id = Uuid::parse_str(&browser_user_id).unwrap();

        let redirect = generic_oauth_callback_impl(
            state,
            Some(auth_user),
            GenericOAuthCallbackQuery {
                code: Some("oauth-code".to_string()),
                state: Some(state_id),
                error: None,
                error_description: None,
            },
        )
        .await;

        let location = redirect_location(redirect);
        assert_eq!(
            redirect_query_param(&location, "status").as_deref(),
            Some("error")
        );
        assert_eq!(
            redirect_query_param(&location, "message").as_deref(),
            Some(
                "This OAuth flow was started by a different NyxID account. Sign out of NyxID in this browser (or switch to the CLI account) and retry."
            )
        );

        let api_key = get_api_key(&db, &key_id).await;
        assert_eq!(api_key.status, "failed");
        assert!(
            api_key
                .error_message
                .as_deref()
                .is_some_and(|msg| !msg.is_empty())
        );
    }

    #[tokio::test]
    async fn generic_oauth_callback_sync_failure_marks_placeholder_failed() {
        let Some(db) =
            connect_test_database("oauth_callback_sync_failure_marks_placeholder_failed").await
        else {
            eprintln!(
                "skipping provider token handler integration test: no local MongoDB available"
            );
            return;
        };
        let state = test_app_state(db.clone());
        let user_id = Uuid::new_v4().to_string();
        let provider_id = Uuid::new_v4().to_string();
        let state_id = Uuid::new_v4().to_string();
        let key_id = Uuid::new_v4().to_string();
        let (token_url, token_server) = spawn_oauth_token_server().await;

        let mut provider = test_provider_config(&provider_id);
        provider.token_url = Some(token_url);
        provider.client_id_encrypted = Some(
            state
                .encryption_keys
                .encrypt(b"test-client-id")
                .await
                .unwrap(),
        );
        provider.client_secret_encrypted = Some(
            state
                .encryption_keys
                .encrypt(b"test-client-secret")
                .await
                .unwrap(),
        );

        db.collection::<ProviderConfig>(PROVIDER_CONFIGS)
            .insert_one(provider)
            .await
            .unwrap();
        db.collection::<OAuthState>(OAUTH_STATES)
            .insert_one(test_oauth_state(&state_id, &user_id, &provider_id))
            .await
            .unwrap();
        db.collection::<UserApiKey>(USER_API_KEYS)
            .insert_one(test_pending_oauth_api_key(&key_id, &user_id, &provider_id))
            .await
            .unwrap();
        // Simulate a production MongoDB write rejection during sync:
        // token exchange succeeds, then the UserApiKey active-state update fails.
        db.run_command(mongodb::bson::doc! {
            "collMod": USER_API_KEYS,
            "validator": { "status": { "$ne": "active" } },
            "validationLevel": "strict",
            "validationAction": "error",
        })
        .await
        .unwrap();

        let redirect = generic_oauth_callback_impl(
            state,
            None,
            GenericOAuthCallbackQuery {
                code: Some("oauth-code".to_string()),
                state: Some(state_id),
                error: None,
                error_description: None,
            },
        )
        .await;
        token_server.abort();

        let location = redirect_location(redirect);
        assert_eq!(
            redirect_query_param(&location, "status").as_deref(),
            Some("error")
        );
        assert_eq!(
            redirect_query_param(&location, "message").as_deref(),
            Some("An internal error occurred")
        );
        let api_key = get_api_key(&db, &key_id).await;
        assert_eq!(api_key.status, "failed");
        assert_eq!(
            api_key.error_message.as_deref(),
            Some("An internal error occurred")
        );
    }

    #[test]
    fn validate_redirect_path_rejects_empty() {
        assert!(validate_redirect_path("").is_err());
    }

    #[test]
    fn validate_redirect_path_rejects_double_slash() {
        assert!(validate_redirect_path("//evil.com").is_err());
    }

    #[test]
    fn validate_redirect_path_rejects_control_chars() {
        assert!(validate_redirect_path("/path\r\ninjection").is_err());
    }

    #[test]
    fn validate_redirect_path_accepts_valid_path() {
        assert!(validate_redirect_path("/keys/detail?tab=settings").is_ok());
    }

    #[tokio::test]
    async fn list_my_tokens_returns_empty_when_no_tokens() {
        let Some(db) = connect_test_database("user_tokens_ext_list_empty").await else {
            eprintln!(
                "skipping provider token handler integration test: no local MongoDB available"
            );
            return;
        };
        let state = test_app_state(db);
        let actor_id = Uuid::new_v4().to_string();

        let Json(response) = list_my_tokens(
            State(state),
            crate::test_utils::test_auth_user(&actor_id),
            Query(ProviderTokenTargetQuery {
                target_org_id: None,
            }),
        )
        .await
        .unwrap();

        assert!(response.tokens.is_empty());
    }

    #[tokio::test]
    async fn connect_api_key_rejects_empty_key() {
        let Some(db) = connect_test_database("user_tokens_ext_connect_empty").await else {
            eprintln!(
                "skipping provider token handler integration test: no local MongoDB available"
            );
            return;
        };
        let state = test_app_state(db);
        let provider_id = Uuid::new_v4().to_string();

        let err = connect_api_key(
            State(state),
            crate::test_utils::test_auth_user(&Uuid::new_v4().to_string()),
            Path(provider_id),
            Json(ConnectApiKeyRequest {
                api_key: String::new(),
                label: None,
                gateway_url: None,
            }),
        )
        .await
        .expect_err("should reject empty key");

        assert!(matches!(err, AppError::ValidationError(_)));
    }

    #[tokio::test]
    async fn connect_api_key_rejects_oversized_key() {
        let Some(db) = connect_test_database("user_tokens_ext_connect_long").await else {
            eprintln!(
                "skipping provider token handler integration test: no local MongoDB available"
            );
            return;
        };
        let state = test_app_state(db);
        let provider_id = Uuid::new_v4().to_string();

        let err = connect_api_key(
            State(state),
            crate::test_utils::test_auth_user(&Uuid::new_v4().to_string()),
            Path(provider_id),
            Json(ConnectApiKeyRequest {
                api_key: "x".repeat(4097),
                label: None,
                gateway_url: None,
            }),
        )
        .await
        .expect_err("should reject oversized key");

        assert!(matches!(err, AppError::ValidationError(_)));
    }

    #[tokio::test]
    async fn connect_api_key_rejects_empty_gateway_url() {
        let Some(db) = connect_test_database("user_tokens_ext_connect_empty_gw").await else {
            eprintln!(
                "skipping provider token handler integration test: no local MongoDB available"
            );
            return;
        };
        let state = test_app_state(db);
        let provider_id = Uuid::new_v4().to_string();

        let err = connect_api_key(
            State(state),
            crate::test_utils::test_auth_user(&Uuid::new_v4().to_string()),
            Path(provider_id),
            Json(ConnectApiKeyRequest {
                api_key: "valid-key".to_string(),
                label: None,
                gateway_url: Some(String::new()),
            }),
        )
        .await
        .expect_err("should reject empty gateway_url");

        assert!(matches!(err, AppError::ValidationError(_)));
    }

    #[tokio::test]
    async fn disconnect_provider_rejects_org_member() {
        let Some(db) = connect_test_database("user_tokens_ext_disconnect_member").await else {
            eprintln!(
                "skipping provider token handler integration test: no local MongoDB available"
            );
            return;
        };
        let state = test_app_state(db.clone());
        let member_id = Uuid::new_v4().to_string();
        let org_id = Uuid::new_v4().to_string();

        db.collection(USERS)
            .insert_one(test_user(&org_id, UserType::Org))
            .await
            .unwrap();
        db.collection(ORG_MEMBERSHIPS)
            .insert_one(test_membership(
                &org_id,
                &member_id,
                crate::models::org_membership::OrgRole::Member,
                None,
            ))
            .await
            .unwrap();

        let err = disconnect_provider(
            State(state),
            crate::test_utils::test_auth_user(&member_id),
            Path(Uuid::new_v4().to_string()),
            Query(ProviderTokenTargetQuery {
                target_org_id: Some(org_id),
            }),
        )
        .await
        .expect_err("member should not disconnect org tokens");

        assert!(matches!(err, AppError::OrgRoleInsufficient(_)));
    }

    #[test]
    fn safe_error_message_hides_internal_details() {
        let internal = safe_error_message(&AppError::Internal("db crash".to_string()));
        assert_eq!(internal, "An internal error occurred");
    }

    #[test]
    fn safe_error_message_passes_through_user_errors() {
        let msg = safe_error_message(&AppError::BadRequest("bad input".to_string()));
        assert!(msg.contains("bad input"));
    }

    #[tokio::test]
    async fn generic_oauth_callback_denial_without_state_redirects_only() {
        let Some(db) = connect_test_database("oauth_callback_denial_without_state").await else {
            eprintln!(
                "skipping provider token handler integration test: no local MongoDB available"
            );
            return;
        };
        let state = test_app_state(db.clone());
        let user_id = Uuid::new_v4().to_string();
        let provider_id = Uuid::new_v4().to_string();
        let key_id = Uuid::new_v4().to_string();

        db.collection::<UserApiKey>(USER_API_KEYS)
            .insert_one(test_pending_oauth_api_key(&key_id, &user_id, &provider_id))
            .await
            .unwrap();

        let redirect = generic_oauth_callback_impl(
            state,
            None,
            GenericOAuthCallbackQuery {
                code: None,
                state: None,
                error: Some("access_denied".to_string()),
                error_description: None,
            },
        )
        .await;

        let location = redirect_location(redirect);
        assert_eq!(
            redirect_query_param(&location, "status").as_deref(),
            Some("error")
        );
        assert_eq!(
            redirect_query_param(&location, "message").as_deref(),
            Some(safe_provider_error_message("access_denied", None).as_str())
        );
        assert_eq!(get_api_key(&db, &key_id).await.status, "pending_auth");
    }

    #[tokio::test]
    async fn generic_oauth_callback_denial_with_invalid_state_redirects_only() {
        let Some(db) = connect_test_database("oauth_callback_denial_invalid_state").await else {
            eprintln!(
                "skipping provider token handler integration test: no local MongoDB available"
            );
            return;
        };
        let state = test_app_state(db.clone());
        let user_id = Uuid::new_v4().to_string();
        let provider_id = Uuid::new_v4().to_string();
        let key_id = Uuid::new_v4().to_string();

        db.collection::<UserApiKey>(USER_API_KEYS)
            .insert_one(test_pending_oauth_api_key(&key_id, &user_id, &provider_id))
            .await
            .unwrap();

        let redirect = generic_oauth_callback_impl(
            state,
            None,
            GenericOAuthCallbackQuery {
                code: None,
                state: Some("bogus-state".to_string()),
                error: Some("access_denied".to_string()),
                error_description: None,
            },
        )
        .await;

        let location = redirect_location(redirect);
        assert_eq!(
            redirect_query_param(&location, "status").as_deref(),
            Some("error")
        );
        assert_eq!(
            redirect_query_param(&location, "message").as_deref(),
            Some(safe_provider_error_message("access_denied", None).as_str())
        );
        assert_eq!(get_api_key(&db, &key_id).await.status, "pending_auth");
    }
}

use std::net::SocketAddr;

use axum::{
    extract::{ConnectInfo, Path, Query, State},
    http::{HeaderMap, StatusCode, header},
};
use serde::Deserialize;

use crate::AppState;
use crate::crypto::token::{constant_time_eq, generate_random_token, hash_token};
use crate::errors::{AppError, AppResult};
use crate::handlers::auth::{
    build_cookie, build_cookie_with_same_site, clear_cookie, clear_cookie_with_same_site,
    extract_ip, extract_user_agent,
};
use crate::mw::auth::{ACCESS_TOKEN_COOKIE_NAME, SESSION_COOKIE_NAME};
use crate::services::{audit_service, social_auth_service, token_service};
use social_auth_service::SocialProfile;

const SOCIAL_STATE_COOKIE: &str = "nyx_social_state";
const SOCIAL_CLIENT_COOKIE: &str = "nyx_social_client";
const SOCIAL_REDIRECT_COOKIE: &str = "nyx_social_redirect";
const SOCIAL_CLIENT_MOBILE: &str = "mobile";
const SOCIAL_NONCE_COOKIE: &str = "nyx_social_nonce";
const SOCIAL_STATE_MAX_AGE: i64 = 600; // 10 minutes
const COOKIE_SAMESITE_LAX: &str = "Lax";
const COOKIE_SAMESITE_NONE: &str = "None";

#[derive(Debug, Deserialize)]
pub struct AuthorizeQuery {
    pub client: Option<String>,
    pub redirect_uri: Option<String>,
}

/// GET /api/v1/auth/social/{provider}
///
/// Initiates the OAuth flow by generating a CSRF state token,
/// setting a state cookie, and redirecting to the provider's authorization URL.
pub async fn authorize(
    State(state): State<AppState>,
    Path(provider_name): Path<String>,
    Query(query): Query<AuthorizeQuery>,
) -> AppResult<(StatusCode, HeaderMap, ())> {
    let provider = social_auth_service::SocialProvider::parse(&provider_name).ok_or_else(|| {
        AppError::SocialAuthFailed(format!("Unsupported provider: {provider_name}"))
    })?;

    let base_secure = state.config.use_secure_cookies();
    let domain = state.config.cookie_domain();

    // Apple's form_post callback is cross-origin, so cookies need SameSite=None.
    // SameSite=None requires the Secure flag, but browsers treat localhost as a
    // secure context even over plain HTTP, so we force Secure=true for Apple.
    let (secure, same_site) = if provider == social_auth_service::SocialProvider::Apple {
        (true, COOKIE_SAMESITE_NONE)
    } else {
        (base_secure, COOKIE_SAMESITE_LAX)
    };

    let is_mobile_client = query.client.as_deref() == Some(SOCIAL_CLIENT_MOBILE);

    let csrf_token = generate_random_token();
    let state_hash = hash_token(&csrf_token);
    let nonce_token =
        (provider == social_auth_service::SocialProvider::Apple).then(generate_random_token);

    // For mobile clients: encode redirect_uri into the OAuth state param so
    // the callback can recover it without relying on cookies (which
    // ASWebAuthenticationSession and Apple form_post may not preserve).
    // Format: "{csrf}.m.{base64url(redirect_uri)}"
    let state_token = if is_mobile_client {
        let redirect_uri = query
            .redirect_uri
            .as_deref()
            .ok_or_else(|| AppError::ValidationError("redirect_uri is required".to_string()))?;
        if !is_supported_mobile_redirect_uri(redirect_uri) {
            return Err(AppError::ValidationError(
                "redirect_uri is not allowed for mobile auth".to_string(),
            ));
        }
        encode_mobile_state(&csrf_token, redirect_uri)
    } else {
        csrf_token.clone()
    };

    let authorization_url = social_auth_service::build_authorization_url(
        provider,
        &state_token,
        nonce_token.as_deref(),
        &state.config,
    )?;

    let mut headers = HeaderMap::new();
    headers.insert(
        header::SET_COOKIE,
        build_cookie_with_same_site(
            SOCIAL_STATE_COOKIE,
            &state_hash,
            SOCIAL_STATE_MAX_AGE,
            "/api/v1/auth/social",
            secure,
            domain,
            same_site,
        )
        .parse()
        .map_err(|_| AppError::Internal("Cookie error".to_string()))?,
    );

    // Set nonce cookie for Apple (used to verify id_token)
    if let Some(nonce) = nonce_token {
        let nonce_hash = hash_token(&nonce);
        headers.append(
            header::SET_COOKIE,
            build_cookie_with_same_site(
                SOCIAL_NONCE_COOKIE,
                &nonce_hash,
                SOCIAL_STATE_MAX_AGE,
                "/api/v1/auth/social",
                secure,
                domain,
                same_site,
            )
            .parse()
            .map_err(|_| AppError::Internal("Cookie error".to_string()))?,
        );
    }

    // Mobile client cookies for redirect after callback
    if is_mobile_client {
        let redirect_uri = query
            .redirect_uri
            .as_deref()
            .ok_or_else(|| AppError::ValidationError("redirect_uri is required".to_string()))?;
        if !is_supported_mobile_redirect_uri(redirect_uri) {
            return Err(AppError::ValidationError(
                "redirect_uri is not allowed for mobile auth".to_string(),
            ));
        }

        headers.append(
            header::SET_COOKIE,
            build_cookie(
                SOCIAL_CLIENT_COOKIE,
                SOCIAL_CLIENT_MOBILE,
                SOCIAL_STATE_MAX_AGE,
                "/api/v1/auth/social",
                secure,
                domain,
            )
            .parse()
            .map_err(|_| AppError::Internal("Cookie error".to_string()))?,
        );
        headers.append(
            header::SET_COOKIE,
            build_cookie(
                SOCIAL_REDIRECT_COOKIE,
                &urlencoding::encode(redirect_uri),
                SOCIAL_STATE_MAX_AGE,
                "/api/v1/auth/social",
                secure,
                domain,
            )
            .parse()
            .map_err(|_| AppError::Internal("Cookie error".to_string()))?,
        );
    } else {
        if let Ok(cookie) =
            clear_cookie(SOCIAL_CLIENT_COOKIE, "/api/v1/auth/social", secure, domain).parse()
        {
            headers.append(header::SET_COOKIE, cookie);
        }
        if let Ok(cookie) =
            clear_cookie(SOCIAL_REDIRECT_COOKIE, "/api/v1/auth/social", secure, domain).parse()
        {
            headers.append(header::SET_COOKIE, cookie);
        }
    }

    headers.insert(
        header::LOCATION,
        authorization_url
            .parse()
            .map_err(|_| AppError::Internal("Redirect URL error".to_string()))?,
    );

    Ok((StatusCode::FOUND, headers, ()))
}

#[derive(Debug, Deserialize)]
pub struct CallbackQuery {
    pub code: Option<String>,
    pub state: Option<String>,
    pub error: Option<String>,
    pub error_description: Option<String>,
}

/// GET /api/v1/auth/social/{provider}/callback
///
/// Handles the OAuth callback: validates state, exchanges code for token,
/// fetches the user profile, creates/finds the user, issues session tokens,
/// and redirects to the frontend.
pub async fn callback(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Path(provider_name): Path<String>,
    Query(params): Query<CallbackQuery>,
    headers: HeaderMap,
) -> Result<(StatusCode, HeaderMap, ()), (StatusCode, HeaderMap, ())> {
    let secure = state.config.use_secure_cookies();
    let domain = state.config.cookie_domain();
    let frontend_url = &state.config.frontend_url;

    // Resolve redirect target from state param first (mobile info may be
    // encoded there since ASWebAuthenticationSession drops cookies), then
    // fall back to cookie-based detection for web flows.
    let raw_state = params.state.as_deref().unwrap_or("");
    let redirect_target = match decode_mobile_state(raw_state) {
        Some((_csrf, ref redirect_uri)) if is_supported_mobile_redirect_uri(redirect_uri) => {
            SocialRedirectTarget::Mobile {
                redirect_uri: redirect_uri.clone(),
            }
        }
        _ => resolve_redirect_target(frontend_url, &headers),
    };

    // Parse provider
    let provider = match social_auth_service::SocialProvider::parse(&provider_name) {
        Some(p) => p,
        None => {
            return Err(redirect_with_error(
                &redirect_target,
                "social_auth_unsupported",
                secure,
                domain,
            ));
        }
    };

    // Check for provider error response
    if params.error.is_some() {
        tracing::warn!(
            error = ?params.error,
            desc = ?params.error_description,
            "Provider returned error"
        );
        return Err(redirect_with_error(
            &redirect_target,
            "social_auth_denied",
            secure,
            domain,
        ));
    }

    // Extract code and state
    let code = match params.code {
        Some(ref c) if !c.is_empty() => c.as_str(),
        _ => {
            return Err(redirect_with_error(
                &redirect_target,
                "social_auth_invalid",
                secure,
                domain,
            ));
        }
    };
    let state_param = match params.state {
        Some(ref s) if !s.is_empty() => s.as_str(),
        _ => {
            return Err(redirect_with_error(
                &redirect_target,
                "social_auth_invalid",
                secure,
                domain,
            ));
        }
    };

    // Validate CSRF state.
    // Mobile flows (ASWebAuthenticationSession / Apple form_post) do not
    // preserve cookies, so the state cookie may be absent. When the state
    // param carries a valid mobile-encoded compound token the CSRF
    // protection is provided by the unguessable csrf portion embedded in
    // the state itself (OAuth 2.0 RFC 6749 §10.12). For web flows we
    // verify against the cookie hash as before.
    let csrf_portion = extract_csrf_from_state(state_param);
    let cookie_hash = extract_cookie_value(&headers, SOCIAL_STATE_COOKIE);
    let is_mobile_state = decode_mobile_state(state_param).is_some();

    let csrf_ok = match cookie_hash.as_deref() {
        Some(hash) => state_matches_cookie_hash(csrf_portion, Some(hash)),
        None => is_mobile_state,
    };

    if !csrf_ok {
        return Err(redirect_with_error(
            &redirect_target,
            "social_auth_csrf",
            secure,
            domain,
        ));
    }

    // Exchange code for access token
    let access_token =
        social_auth_service::exchange_code(provider, code, &state.config, &state.http_client)
            .await
            .map_err(|e| {
                tracing::warn!(error = %e, "Social auth code exchange failed");
                redirect_with_error(&redirect_target, "social_auth_exchange", secure, domain)
            })?;

    // Fetch user profile
    let profile =
        social_auth_service::fetch_user_profile(provider, &access_token, &state.http_client)
            .await
            .map_err(|e| {
                tracing::warn!(error = %e, "Social auth profile fetch failed");
                redirect_with_error(&redirect_target, "social_auth_profile", secure, domain)
            })?;

    // Find or create user
    let user = social_auth_service::find_or_create_user(&state.db, &profile)
        .await
        .map_err(|e| {
            tracing::warn!(error = %e, "Social auth find_or_create_user failed");
            let error_key = match &e {
                AppError::SocialAuthConflict => "social_auth_conflict",
                AppError::SocialAuthNoEmail => "social_auth_no_email",
                AppError::SocialAuthDeactivated => "social_auth_deactivated",
                _ => "social_auth_exchange",
            };
            redirect_with_error(&redirect_target, error_key, secure, domain)
        })?;

    // Issue session and tokens
    let ip = extract_ip(&headers, Some(peer));
    let ua = extract_user_agent(&headers);

    let tokens = token_service::create_session_and_issue_tokens(
        &state.db,
        &state.config,
        &state.jwt_keys,
        &user.id,
        ip.as_deref(),
        ua.as_deref(),
    )
    .await
    .map_err(|e| {
        tracing::error!(error = %e, "Social auth session creation failed");
        redirect_with_error(&redirect_target, "social_auth_exchange", secure, domain)
    })?;

    // Audit log
    audit_service::log_async(
        state.db.clone(),
        Some(user.id.clone()),
        "social_login".to_string(),
        Some(serde_json::json!({
            "provider": provider.as_str(),
            "session_id": tokens.session_id,
        })),
        ip,
        ua,
    );

    // Build response with auth cookies and redirect
    build_auth_redirect(
        &state.config,
        &tokens,
        &redirect_target,
        provider.as_str(),
        &user.id,
        secure,
        domain,
    )
}

// --- Apple POST callback (response_mode=form_post) ---

#[derive(Debug, Deserialize)]
pub struct AppleCallbackForm {
    pub code: Option<String>,
    pub state: Option<String>,
    pub error: Option<String>,
    /// JSON string with user name info, only present on FIRST authorization.
    pub user: Option<String>,
    #[allow(dead_code)]
    pub id_token: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AppleUser {
    name: Option<AppleUserName>,
    #[allow(dead_code)]
    email: Option<String>,
}

#[derive(Debug, Deserialize)]
struct AppleUserName {
    #[serde(rename = "firstName")]
    first_name: Option<String>,
    #[serde(rename = "lastName")]
    last_name: Option<String>,
}

/// POST /api/v1/auth/social/apple/callback
///
/// Handles Apple's form_post callback. Apple POSTs the authorization code,
/// state, and optionally user info (name/email, only on first auth) as
/// application/x-www-form-urlencoded form data.
///
/// Mobile client info is encoded in the state param (not cookies) because
/// Apple's cross-site POST does not carry SameSite=Lax cookies.
pub async fn apple_callback(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    axum::Form(form): axum::Form<AppleCallbackForm>,
) -> Result<(StatusCode, HeaderMap, ()), (StatusCode, HeaderMap, ())> {
    let secure = state.config.use_secure_cookies();
    let domain = state.config.cookie_domain();
    let frontend_url = &state.config.frontend_url;

    // Resolve redirect target from state param (mobile info encoded in state)
    // before full validation so error redirects also go to the right place.
    let raw_state = form.state.as_deref().unwrap_or("");
    let redirect_target = match decode_mobile_state(raw_state) {
        Some((_csrf, redirect_uri)) if is_supported_mobile_redirect_uri(&redirect_uri) => {
            SocialRedirectTarget::Mobile { redirect_uri }
        }
        _ => SocialRedirectTarget::Web {
            frontend_url: frontend_url.to_string(),
        },
    };

    if form.error.is_some() {
        return Err(redirect_with_error(
            &redirect_target,
            "social_auth_denied",
            secure,
            domain,
        ));
    }

    let code = match form.code {
        Some(ref c) if !c.is_empty() => c.as_str(),
        _ => {
            return Err(redirect_with_error(
                &redirect_target,
                "social_auth_invalid",
                secure,
                domain,
            ));
        }
    };
    let state_param = match form.state {
        Some(ref s) if !s.is_empty() => s.as_str(),
        _ => {
            return Err(redirect_with_error(
                &redirect_target,
                "social_auth_invalid",
                secure,
                domain,
            ));
        }
    };

    // CSRF validation: same logic as GET callback — cookie when available,
    // compound mobile state as implicit proof when cookies are absent.
    let csrf_portion = extract_csrf_from_state(state_param);
    let cookie_hash = extract_cookie_value(&headers, SOCIAL_STATE_COOKIE);
    let is_mobile_state = decode_mobile_state(state_param).is_some();

    let csrf_ok = match cookie_hash.as_deref() {
        Some(hash) => state_matches_cookie_hash(csrf_portion, Some(hash)),
        None => is_mobile_state,
    };

    if !csrf_ok {
        return Err(redirect_with_error(
            &redirect_target,
            "social_auth_csrf",
            secure,
            domain,
        ));
    }

    // Exchange code for id_token (Apple returns id_token, not access_token)
    let id_token = social_auth_service::exchange_code(
        social_auth_service::SocialProvider::Apple,
        code,
        &state.config,
        &state.http_client,
    )
    .await
    .map_err(|e| {
        tracing::warn!(error = %e, "Apple auth code exchange failed");
        redirect_with_error(&redirect_target, "social_auth_exchange", secure, domain)
    })?;

    // Verify id_token via Apple JWKS
    let apple_client_id =
        state.config.apple_client_id.as_deref().ok_or_else(|| {
            redirect_with_error(&redirect_target, "social_auth_exchange", secure, domain)
        })?;

    let claims = state
        .jwks_cache
        .verify_apple_id_token(&id_token, apple_client_id)
        .await
        .map_err(|e| {
            tracing::warn!(error = %e, "Apple ID token verification failed");
            redirect_with_error(&redirect_target, "social_auth_exchange", secure, domain)
        })?;

    // Validate nonce against the cookie set at authorization time.
    // Mobile flows lose cookies, but the nonce is already verified as part
    // of the Apple id_token signature (JWKS check above), so we only
    // enforce the cookie check when the cookie is actually present.
    let nonce_cookie = extract_cookie_value(&headers, SOCIAL_NONCE_COOKIE);
    if nonce_cookie.is_some()
        && !nonce_matches_cookie_hash(claims.nonce.as_deref(), nonce_cookie.as_deref())
    {
        return Err(redirect_with_error(
            &redirect_target,
            "social_auth_csrf",
            secure,
            domain,
        ));
    }

    // Build profile from verified claims
    let mut profile = social_auth_service::profile_from_apple_id_token(&claims).map_err(|e| {
        tracing::warn!(error = %e, "Apple profile extraction failed");
        redirect_with_error(&redirect_target, "social_auth_no_email", secure, domain)
    })?;

    // Extract user name from the POST body (only present on first authorization)
    if let Some(ref user_json) = form.user {
        if let Ok(apple_user) = serde_json::from_str::<AppleUser>(user_json) {
            if let Some(name) = apple_user.name {
                let display_name = match (name.first_name, name.last_name) {
                    (Some(f), Some(l)) => Some(format!("{f} {l}")),
                    (Some(f), None) => Some(f),
                    (None, Some(l)) => Some(l),
                    (None, None) => None,
                };
                profile = SocialProfile {
                    display_name,
                    ..profile
                };
            }
        }
    }

    // Find or create user (same as Google/GitHub flow)
    let user = social_auth_service::find_or_create_user(&state.db, &profile)
        .await
        .map_err(|e| {
            let error_key = match &e {
                AppError::SocialAuthConflict => "social_auth_conflict",
                AppError::SocialAuthNoEmail => "social_auth_no_email",
                AppError::SocialAuthDeactivated => "social_auth_deactivated",
                _ => "social_auth_exchange",
            };
            redirect_with_error(&redirect_target, error_key, secure, domain)
        })?;

    // Issue session and tokens
    let ip = extract_ip(&headers, Some(peer));
    let ua = extract_user_agent(&headers);

    let tokens = token_service::create_session_and_issue_tokens(
        &state.db,
        &state.config,
        &state.jwt_keys,
        &user.id,
        ip.as_deref(),
        ua.as_deref(),
    )
    .await
    .map_err(|e| {
        tracing::error!(error = %e, "Apple auth session creation failed");
        redirect_with_error(&redirect_target, "social_auth_exchange", secure, domain)
    })?;

    // Audit log
    audit_service::log_async(
        state.db.clone(),
        Some(user.id.clone()),
        "social_login".to_string(),
        Some(serde_json::json!({
            "provider": "apple",
            "session_id": tokens.session_id,
        })),
        ip,
        ua,
    );

    // Build response with auth cookies and redirect
    build_auth_redirect(
        &state.config,
        &tokens,
        &redirect_target,
        "apple",
        &user.id,
        secure,
        domain,
    )
}

/// Build the auth cookie headers and redirect after social login.
///
/// Shared between the GET callback (Google/GitHub) and the POST callback (Apple).
fn build_auth_redirect(
    config: &crate::config::AppConfig,
    tokens: &token_service::IssuedTokens,
    target: &SocialRedirectTarget,
    provider: &str,
    user_id: &str,
    secure: bool,
    domain: Option<&str>,
) -> Result<(StatusCode, HeaderMap, ()), (StatusCode, HeaderMap, ())> {
    let mut headers = HeaderMap::new();

    // Session cookie (30 days)
    headers.insert(
        header::SET_COOKIE,
        build_cookie(
            SESSION_COOKIE_NAME,
            &tokens.session_token,
            30 * 24 * 3600,
            "/",
            secure,
            domain,
        )
        .parse()
        .map_err(|_| redirect_with_error(target, "social_auth_exchange", secure, domain))?,
    );

    // Access token cookie
    headers.append(
        header::SET_COOKIE,
        build_cookie(
            ACCESS_TOKEN_COOKIE_NAME,
            &tokens.access_token,
            tokens.access_expires_in,
            "/",
            secure,
            domain,
        )
        .parse()
        .map_err(|_| redirect_with_error(target, "social_auth_exchange", secure, domain))?,
    );

    // Refresh token cookie
    headers.append(
        header::SET_COOKIE,
        build_cookie(
            "nyx_refresh_token",
            &tokens.refresh_token,
            config.jwt_refresh_ttl_secs,
            "/api/v1/auth/refresh",
            secure,
            domain,
        )
        .parse()
        .map_err(|_| redirect_with_error(target, "social_auth_exchange", secure, domain))?,
    );

    // Clear social flow cookies (state + nonce, both SameSite variants)
    for cookie in social_clear_cookie_values(secure, domain) {
        headers.append(
            header::SET_COOKIE,
            cookie.parse().map_err(|_| {
                redirect_with_error(target, "social_auth_exchange", secure, domain)
            })?,
        );
    }

    // Clear mobile client cookies
    if let Ok(cookie) =
        clear_cookie(SOCIAL_CLIENT_COOKIE, "/api/v1/auth/social", secure, domain).parse()
    {
        headers.append(header::SET_COOKIE, cookie);
    }
    if let Ok(cookie) =
        clear_cookie(SOCIAL_REDIRECT_COOKIE, "/api/v1/auth/social", secure, domain).parse()
    {
        headers.append(header::SET_COOKIE, cookie);
    }

    let redirect_url = build_success_redirect_url(
        target,
        provider,
        user_id,
        &tokens.access_token,
        tokens.access_expires_in,
        &tokens.refresh_token,
    );
    headers.insert(
        header::LOCATION,
        redirect_url.parse().map_err(|_| {
            redirect_with_error(target, "social_auth_exchange", secure, domain)
        })?,
    );

    Ok((StatusCode::FOUND, headers, ()))
}

#[derive(Debug, Clone)]
enum SocialRedirectTarget {
    Web { frontend_url: String },
    Mobile { redirect_uri: String },
}

fn resolve_redirect_target(frontend_url: &str, headers: &HeaderMap) -> SocialRedirectTarget {
    let client_cookie = extract_cookie_value(headers, SOCIAL_CLIENT_COOKIE);
    let redirect_cookie = extract_cookie_value(headers, SOCIAL_REDIRECT_COOKIE);
    let mobile_redirect = redirect_cookie
        .and_then(|encoded| urlencoding::decode(&encoded).ok().map(|v| v.to_string()))
        .filter(|uri| is_supported_mobile_redirect_uri(uri));

    if client_cookie.as_deref() == Some(SOCIAL_CLIENT_MOBILE)
        && let Some(redirect_uri) = mobile_redirect
    {
        return SocialRedirectTarget::Mobile { redirect_uri };
    }

    SocialRedirectTarget::Web {
        frontend_url: frontend_url.to_string(),
    }
}

fn is_supported_mobile_redirect_uri(uri: &str) -> bool {
    uri.starts_with("nyxid://") || uri.starts_with("exp://")
}

fn build_success_redirect_url(
    target: &SocialRedirectTarget,
    provider: &str,
    user_id: &str,
    access_token: &str,
    expires_in: i64,
    refresh_token: &str,
) -> String {
    match target {
        SocialRedirectTarget::Web { frontend_url } => {
            frontend_url.trim_end_matches('/').to_string() + "/"
        }
        SocialRedirectTarget::Mobile { redirect_uri } => {
            let joiner = if redirect_uri.contains('?') { "&" } else { "?" };
            format!(
                "{}{}status=success&provider={}&user_id={}&access_token={}&refresh_token={}&expires_in={}",
                redirect_uri,
                joiner,
                urlencoding::encode(provider),
                urlencoding::encode(user_id),
                urlencoding::encode(access_token),
                urlencoding::encode(refresh_token),
                expires_in
            )
        }
    }
}

/// Build an error redirect response that clears social flow cookies.
fn redirect_with_error(
    target: &SocialRedirectTarget,
    error: &str,
    secure: bool,
    domain: Option<&str>,
) -> (StatusCode, HeaderMap, ()) {
    let mut headers = HeaderMap::new();
    let url = match target {
        SocialRedirectTarget::Web { frontend_url } => {
            let base = frontend_url.trim_end_matches('/');
            format!("{}/login?error={}", base, error)
        }
        SocialRedirectTarget::Mobile { redirect_uri } => {
            let joiner = if redirect_uri.contains('?') { "&" } else { "?" };
            format!(
                "{}{}status=error&error={}",
                redirect_uri,
                joiner,
                urlencoding::encode(error)
            )
        }
    };
    if let Ok(location) = url.parse() {
        headers.insert(header::LOCATION, location);
    }
    for cookie in social_clear_cookie_values(secure, domain) {
        if let Ok(parsed) = cookie.parse() {
            headers.append(header::SET_COOKIE, parsed);
        }
    }
    if let Ok(cookie) =
        clear_cookie(SOCIAL_CLIENT_COOKIE, "/api/v1/auth/social", secure, domain).parse()
    {
        headers.append(header::SET_COOKIE, cookie);
    }
    if let Ok(cookie) =
        clear_cookie(SOCIAL_REDIRECT_COOKIE, "/api/v1/auth/social", secure, domain).parse()
    {
        headers.append(header::SET_COOKIE, cookie);
    }
    (StatusCode::FOUND, headers, ())
}

// ─── Compound state helpers ─────────────────────────────────────────
// Apple form_post loses cookies, so we embed mobile redirect info in
// the OAuth state: "{csrf}.m.{base64url(redirect_uri)}".
// Web flows use a plain csrf token with no separator.

const STATE_MOBILE_SEPARATOR: &str = ".m.";

fn encode_mobile_state(csrf_token: &str, redirect_uri: &str) -> String {
    use base64::engine::{Engine, general_purpose::URL_SAFE_NO_PAD};
    let encoded_uri = URL_SAFE_NO_PAD.encode(redirect_uri.as_bytes());
    format!("{csrf_token}{STATE_MOBILE_SEPARATOR}{encoded_uri}")
}

fn decode_mobile_state(state: &str) -> Option<(String, String)> {
    use base64::engine::{Engine, general_purpose::URL_SAFE_NO_PAD};
    let (csrf, b64) = state.split_once(STATE_MOBILE_SEPARATOR)?;
    let bytes = URL_SAFE_NO_PAD.decode(b64).ok()?;
    let redirect_uri = String::from_utf8(bytes).ok()?;
    Some((csrf.to_string(), redirect_uri))
}

fn extract_csrf_from_state(state: &str) -> &str {
    state
        .split_once(STATE_MOBILE_SEPARATOR)
        .map_or(state, |(csrf, _)| csrf)
}

fn state_matches_cookie_hash(state_param: &str, cookie_hash: Option<&str>) -> bool {
    let computed_hash = hash_token(state_param);
    match cookie_hash {
        Some(hash) => constant_time_eq(hash.as_bytes(), computed_hash.as_bytes()),
        None => false,
    }
}

fn nonce_matches_cookie_hash(nonce_claim: Option<&str>, cookie_hash: Option<&str>) -> bool {
    let nonce = match nonce_claim {
        Some(n) if !n.is_empty() => n,
        _ => return false,
    };
    let hash = match cookie_hash {
        Some(h) if !h.is_empty() => h,
        _ => return false,
    };
    let computed_hash = hash_token(nonce);
    constant_time_eq(hash.as_bytes(), computed_hash.as_bytes())
}

fn social_clear_cookie_values(secure: bool, domain: Option<&str>) -> [String; 4] {
    [
        clear_cookie_with_same_site(
            SOCIAL_STATE_COOKIE,
            "/api/v1/auth/social",
            secure,
            domain,
            COOKIE_SAMESITE_LAX,
        ),
        clear_cookie_with_same_site(
            SOCIAL_STATE_COOKIE,
            "/api/v1/auth/social",
            secure,
            domain,
            COOKIE_SAMESITE_NONE,
        ),
        clear_cookie_with_same_site(
            SOCIAL_NONCE_COOKIE,
            "/api/v1/auth/social",
            secure,
            domain,
            COOKIE_SAMESITE_LAX,
        ),
        clear_cookie_with_same_site(
            SOCIAL_NONCE_COOKIE,
            "/api/v1/auth/social",
            secure,
            domain,
            COOKIE_SAMESITE_NONE,
        ),
    ]
}

/// Extract a cookie value by name from the request headers.
///
/// Reads only the first `Cookie` header. Per RFC 6265 section 5.4, the user
/// agent SHOULD send all cookies in a single header. Multiple `Cookie` headers
/// are non-standard and not handled here; this is an accepted limitation.
fn extract_cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get("cookie")
        .and_then(|v| v.to_str().ok())
        .and_then(|cookie_header| {
            cookie_header.split(';').find_map(|pair| {
                let pair = pair.trim();
                let (key, value) = pair.split_once('=')?;
                if key.trim() == name {
                    Some(value.trim().to_string())
                } else {
                    None
                }
            })
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_matches_cookie_hash_true_when_value_matches() {
        let state = "state-token";
        let state_hash = hash_token(state);
        assert!(state_matches_cookie_hash(state, Some(&state_hash)));
    }

    #[test]
    fn state_matches_cookie_hash_false_when_missing_or_mismatch() {
        let state = "state-token";
        let wrong_hash = hash_token("different-state");
        assert!(!state_matches_cookie_hash(state, None));
        assert!(!state_matches_cookie_hash(state, Some(&wrong_hash)));
    }

    #[test]
    fn nonce_matches_cookie_hash_true_when_value_matches() {
        let nonce = "nonce-token";
        let nonce_hash = hash_token(nonce);
        assert!(nonce_matches_cookie_hash(Some(nonce), Some(&nonce_hash)));
    }

    #[test]
    fn nonce_matches_cookie_hash_false_when_missing_or_mismatch() {
        let nonce_hash = hash_token("nonce-token");
        let wrong_hash = hash_token("other-nonce");
        assert!(!nonce_matches_cookie_hash(None, Some(&nonce_hash)));
        assert!(!nonce_matches_cookie_hash(Some("nonce-token"), None));
        assert!(!nonce_matches_cookie_hash(
            Some("nonce-token"),
            Some(&wrong_hash)
        ));
    }

    #[test]
    fn social_clear_cookie_values_include_state_and_nonce_variants() {
        let cookies = social_clear_cookie_values(true, Some(".example.com"));
        assert_eq!(cookies.len(), 4);
        assert!(cookies.iter().any(|c| c.contains("nyx_social_state=")));
        assert!(cookies.iter().any(|c| c.contains("nyx_social_nonce=")));
        assert!(cookies.iter().any(|c| c.contains("SameSite=Lax")));
        assert!(cookies.iter().any(|c| c.contains("SameSite=None")));
    }

    #[test]
    fn encode_decode_mobile_state_roundtrip() {
        let csrf = "abc123def456";
        let redirect = "nyxid://auth/social/callback";
        let encoded = encode_mobile_state(csrf, redirect);
        assert!(encoded.contains(STATE_MOBILE_SEPARATOR));

        let (decoded_csrf, decoded_uri) = decode_mobile_state(&encoded).unwrap();
        assert_eq!(decoded_csrf, csrf);
        assert_eq!(decoded_uri, redirect);
    }

    #[test]
    fn decode_mobile_state_returns_none_for_plain_csrf() {
        assert!(decode_mobile_state("plain_csrf_token_hex").is_none());
    }

    #[test]
    fn extract_csrf_from_compound_state() {
        let csrf = "abc123";
        let compound = encode_mobile_state(csrf, "nyxid://callback");
        assert_eq!(extract_csrf_from_state(&compound), csrf);
    }

    #[test]
    fn extract_csrf_from_plain_state() {
        assert_eq!(extract_csrf_from_state("plain_token"), "plain_token");
    }

    #[test]
    fn compound_state_csrf_matches_cookie_hash() {
        let csrf = generate_random_token();
        let hash = hash_token(&csrf);
        let compound = encode_mobile_state(&csrf, "nyxid://auth/social/callback");
        let csrf_portion = extract_csrf_from_state(&compound);
        assert!(state_matches_cookie_hash(csrf_portion, Some(&hash)));
    }
}

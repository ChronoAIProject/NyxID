use std::net::SocketAddr;

use axum::{
    extract::{ConnectInfo, Path, Query, State},
    http::{header, HeaderMap, StatusCode},
    Form,
};
use serde::Deserialize;

use crate::crypto::token::{constant_time_eq, generate_random_token, hash_token};
use crate::errors::{AppError, AppResult};
use crate::handlers::auth::{build_cookie, clear_cookie, extract_ip, extract_user_agent};
use crate::mw::auth::{ACCESS_TOKEN_COOKIE_NAME, SESSION_COOKIE_NAME};
use crate::services::{audit_service, social_auth_service, token_service};
use crate::AppState;

const SOCIAL_STATE_COOKIE: &str = "nyx_social_state";
const SOCIAL_PLATFORM_COOKIE: &str = "nyx_social_platform";
const SOCIAL_STATE_MAX_AGE: i64 = 600; // 10 minutes
const NATIVE_APP_SCHEME: &str = "nyxid";

#[derive(Debug, Deserialize)]
pub struct AuthorizeQuery {
    pub platform: Option<String>,
}

/// GET /api/v1/auth/social/{provider}
pub async fn authorize(
    State(state): State<AppState>,
    Path(provider_name): Path<String>,
    Query(query): Query<AuthorizeQuery>,
) -> AppResult<(StatusCode, HeaderMap, ())> {
    let provider = social_auth_service::SocialProvider::parse(&provider_name).ok_or_else(|| {
        AppError::SocialAuthFailed(format!("Unsupported provider: {provider_name}"))
    })?;

    let state_token = generate_random_token();
    let state_hash = hash_token(&state_token);

    let authorization_url =
        social_auth_service::build_authorization_url(provider, &state_token, &state.config)?;

    let secure = state.config.use_secure_cookies();
    let domain = state.config.cookie_domain();
    let is_native = query.platform.as_deref() == Some("native");

    let mut headers = HeaderMap::new();
    headers.insert(
        header::SET_COOKIE,
        build_cookie(
            SOCIAL_STATE_COOKIE,
            &state_hash,
            SOCIAL_STATE_MAX_AGE,
            "/api/v1/auth/social",
            secure,
            domain,
        )
        .parse()
        .map_err(|_| AppError::Internal("Cookie error".to_string()))?,
    );

    // Tag the flow so the callback knows to redirect via custom URL scheme
    if is_native {
        headers.append(
            header::SET_COOKIE,
            build_cookie(
                SOCIAL_PLATFORM_COOKIE,
                "native",
                SOCIAL_STATE_MAX_AGE,
                "/api/v1/auth/social",
                secure,
                domain,
            )
            .parse()
            .map_err(|_| AppError::Internal("Cookie error".to_string()))?,
        );
    }

    headers.insert(
        header::LOCATION,
        authorization_url
            .parse()
            .map_err(|_| AppError::Internal("Redirect URL error".to_string()))?,
    );

    Ok((StatusCode::FOUND, headers, ()))
}

// ===================================================================
// Callback parameter structs
// ===================================================================

#[derive(Debug, Deserialize)]
pub struct CallbackQuery {
    pub code: Option<String>,
    pub state: Option<String>,
    pub error: Option<String>,
    pub error_description: Option<String>,
}

/// Apple uses response_mode=form_post, so callback params arrive as POST form.
#[derive(Debug, Deserialize)]
pub struct AppleCallbackForm {
    pub code: Option<String>,
    pub state: Option<String>,
    pub id_token: Option<String>,
    pub user: Option<String>,
    pub error: Option<String>,
}

/// Unified callback params extracted from GET query or POST form.
struct CallbackParams {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
    /// Apple-only: user JSON (name) sent only on first authorization
    apple_user: Option<String>,
}

// ===================================================================
// GET callback (Google, GitHub)
// ===================================================================

pub async fn callback_get(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Path(provider_name): Path<String>,
    Query(params): Query<CallbackQuery>,
    headers: HeaderMap,
) -> Result<(StatusCode, HeaderMap, ()), (StatusCode, HeaderMap, ())> {
    handle_callback(
        state,
        peer,
        provider_name,
        CallbackParams {
            code: params.code,
            state: params.state,
            error: params.error,
            apple_user: None,
        },
        headers,
    )
    .await
}

// ===================================================================
// POST callback (Apple form_post)
// ===================================================================

pub async fn callback_post(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Path(provider_name): Path<String>,
    headers: HeaderMap,
    Form(form): Form<AppleCallbackForm>,
) -> Result<(StatusCode, HeaderMap, ()), (StatusCode, HeaderMap, ())> {
    handle_callback(
        state,
        peer,
        provider_name,
        CallbackParams {
            code: form.code,
            state: form.state,
            error: form.error,
            apple_user: form.user,
        },
        headers,
    )
    .await
}

// ===================================================================
// Unified callback handler
// ===================================================================

async fn handle_callback(
    state: AppState,
    peer: SocketAddr,
    provider_name: String,
    params: CallbackParams,
    headers: HeaderMap,
) -> Result<(StatusCode, HeaderMap, ()), (StatusCode, HeaderMap, ())> {
    let secure = state.config.use_secure_cookies();
    let domain = state.config.cookie_domain();
    let frontend_url = &state.config.frontend_url;
    let is_native = extract_cookie_value(&headers, SOCIAL_PLATFORM_COOKIE)
        .as_deref() == Some("native");

    let provider = match social_auth_service::SocialProvider::parse(&provider_name) {
        Some(p) => p,
        None => return Err(redirect_with_error(frontend_url, "social_auth_unsupported", secure, domain, is_native)),
    };

    if params.error.is_some() {
        tracing::warn!(error = ?params.error, "Provider returned error");
        return Err(redirect_with_error(frontend_url, "social_auth_denied", secure, domain, is_native));
    }

    let code = match params.code {
        Some(ref c) if !c.is_empty() => c.as_str(),
        _ => return Err(redirect_with_error(frontend_url, "social_auth_invalid", secure, domain, is_native)),
    };
    let state_param = match params.state {
        Some(ref s) if !s.is_empty() => s.as_str(),
        _ => return Err(redirect_with_error(frontend_url, "social_auth_invalid", secure, domain, is_native)),
    };

    // CSRF state validation
    let computed_hash = hash_token(state_param);
    let cookie_hash = extract_cookie_value(&headers, SOCIAL_STATE_COOKIE);
    match cookie_hash {
        Some(ref h) if constant_time_eq(h.as_bytes(), computed_hash.as_bytes()) => {}
        _ => return Err(redirect_with_error(frontend_url, "social_auth_csrf", secure, domain, is_native)),
    }

    let token = social_auth_service::exchange_code(
        provider, code, &state.config, &state.http_client,
    )
    .await
    .map_err(|e| {
        tracing::warn!(error = %e, "Social auth code exchange failed");
        redirect_with_error(frontend_url, "social_auth_exchange", secure, domain, is_native)
    })?;

    let profile = social_auth_service::fetch_user_profile(
        provider,
        &token,
        &state.http_client,
        params.apple_user.as_deref(),
        &state.config,
    )
    .await
    .map_err(|e| {
        tracing::warn!(error = %e, "Social auth profile fetch failed");
        redirect_with_error(frontend_url, "social_auth_profile", secure, domain, is_native)
    })?;

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
            redirect_with_error(frontend_url, error_key, secure, domain, is_native)
        })?;

    let ip = extract_ip(&headers, Some(peer));
    let ua = extract_user_agent(&headers);

    let tokens = token_service::create_session_and_issue_tokens(
        &state.db, &state.config, &state.jwt_keys, &user.id,
        ip.as_deref(), ua.as_deref(),
    )
    .await
    .map_err(|e| {
        tracing::error!(error = %e, "Social auth session creation failed");
        redirect_with_error(frontend_url, "social_auth_exchange", secure, domain, is_native)
    })?;

    audit_service::log_async(
        state.db.clone(),
        Some(user.id.clone()),
        "social_login".to_string(),
        Some(serde_json::json!({
            "provider": provider.as_str(),
            "session_id": tokens.session_id,
        })),
        ip, ua,
    );

    // ── Native app: redirect via custom URL scheme with tokens ──
    if is_native {
        let mut resp = HeaderMap::new();
        let redirect = format!(
            "{}://auth/callback?session_token={}&access_token={}&refresh_token={}",
            NATIVE_APP_SCHEME,
            urlencoding::encode(&tokens.session_token),
            urlencoding::encode(&tokens.access_token),
            urlencoding::encode(&tokens.refresh_token),
        );
        if let Ok(loc) = redirect.parse() {
            resp.insert(header::LOCATION, loc);
        }
        if let Ok(c) = clear_cookie(SOCIAL_STATE_COOKIE, "/api/v1/auth/social", secure, domain).parse() {
            resp.append(header::SET_COOKIE, c);
        }
        if let Ok(c) = clear_cookie(SOCIAL_PLATFORM_COOKIE, "/api/v1/auth/social", secure, domain).parse() {
            resp.append(header::SET_COOKIE, c);
        }
        return Ok((StatusCode::FOUND, resp, ()));
    }

    // ── Web: set cookies and redirect to frontend ──
    let mut resp = HeaderMap::new();
    let err = |_| redirect_with_error(frontend_url, "social_auth_exchange", secure, domain, false);

    resp.insert(header::SET_COOKIE,
        build_cookie(SESSION_COOKIE_NAME, &tokens.session_token, 30 * 24 * 3600, "/", secure, domain)
            .parse().map_err(err)?);
    resp.append(header::SET_COOKIE,
        build_cookie(ACCESS_TOKEN_COOKIE_NAME, &tokens.access_token, tokens.access_expires_in, "/", secure, domain)
            .parse().map_err(err)?);
    resp.append(header::SET_COOKIE,
        build_cookie("nyx_refresh_token", &tokens.refresh_token, state.config.jwt_refresh_ttl_secs, "/api/v1/auth/refresh", secure, domain)
            .parse().map_err(err)?);
    resp.append(header::SET_COOKIE,
        clear_cookie(SOCIAL_STATE_COOKIE, "/api/v1/auth/social", secure, domain)
            .parse().map_err(err)?);

    let redirect_url = state.config.frontend_url.trim_end_matches('/').to_string() + "/";
    resp.insert(header::LOCATION, redirect_url.parse().map_err(err)?);

    Ok((StatusCode::FOUND, resp, ()))
}

/// Build an error redirect response that clears state cookies.
fn redirect_with_error(
    frontend_url: &str,
    error: &str,
    secure: bool,
    domain: Option<&str>,
    is_native: bool,
) -> (StatusCode, HeaderMap, ()) {
    let mut headers = HeaderMap::new();

    let url = if is_native {
        format!("{}://auth/callback?error={}", NATIVE_APP_SCHEME, error)
    } else {
        let base = frontend_url.trim_end_matches('/');
        format!("{}/login?error={}", base, error)
    };

    if let Ok(location) = url.parse() {
        headers.insert(header::LOCATION, location);
    }
    if let Ok(cookie) = clear_cookie(SOCIAL_STATE_COOKIE, "/api/v1/auth/social", secure, domain).parse() {
        headers.append(header::SET_COOKIE, cookie);
    }
    if let Ok(cookie) = clear_cookie(SOCIAL_PLATFORM_COOKIE, "/api/v1/auth/social", secure, domain).parse() {
        headers.append(header::SET_COOKIE, cookie);
    }
    (StatusCode::FOUND, headers, ())
}

/// Extract a cookie value by name from the request headers.
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

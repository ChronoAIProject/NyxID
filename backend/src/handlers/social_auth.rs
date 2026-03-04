use std::net::SocketAddr;

use axum::{
    extract::{ConnectInfo, Path, Query, State},
    http::{HeaderMap, StatusCode, header},
};
use serde::Deserialize;

use crate::AppState;
use crate::crypto::token::{constant_time_eq, generate_random_token, hash_token};
use crate::errors::{AppError, AppResult};
use crate::handlers::auth::{build_cookie, clear_cookie, extract_ip, extract_user_agent};
use crate::mw::auth::{ACCESS_TOKEN_COOKIE_NAME, SESSION_COOKIE_NAME};
use crate::services::{audit_service, social_auth_service, token_service};

const SOCIAL_STATE_COOKIE: &str = "nyx_social_state";
const SOCIAL_CLIENT_COOKIE: &str = "nyx_social_client";
const SOCIAL_REDIRECT_COOKIE: &str = "nyx_social_redirect";
const SOCIAL_CLIENT_MOBILE: &str = "mobile";
const SOCIAL_STATE_MAX_AGE: i64 = 600; // 10 minutes

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

    let state_token = generate_random_token();
    let state_hash = hash_token(&state_token);

    let authorization_url =
        social_auth_service::build_authorization_url(provider, &state_token, &state.config)?;

    let secure = state.config.use_secure_cookies();
    let domain = state.config.cookie_domain();
    let is_mobile_client = query.client.as_deref() == Some(SOCIAL_CLIENT_MOBILE);

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
    let redirect_target = resolve_redirect_target(frontend_url, &headers);

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

    // Validate CSRF state (constant-time comparison to prevent timing attacks)
    let computed_hash = hash_token(state_param);
    let cookie_hash = extract_cookie_value(&headers, SOCIAL_STATE_COOKIE);
    match cookie_hash {
        Some(ref h) if constant_time_eq(h.as_bytes(), computed_hash.as_bytes()) => {}
        _ => {
            return Err(redirect_with_error(
                &redirect_target,
                "social_auth_csrf",
                secure,
                domain,
            ));
        }
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

    // Build response with auth cookies
    let mut response_headers = HeaderMap::new();

    // Session cookie (30 days)
    response_headers.insert(
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
        .map_err(|_| redirect_with_error(&redirect_target, "social_auth_exchange", secure, domain))?,
    );

    // Access token cookie
    response_headers.append(
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
        .map_err(|_| redirect_with_error(&redirect_target, "social_auth_exchange", secure, domain))?,
    );

    // Refresh token cookie
    response_headers.append(
        header::SET_COOKIE,
        build_cookie(
            "nyx_refresh_token",
            &tokens.refresh_token,
            state.config.jwt_refresh_ttl_secs,
            "/api/v1/auth/refresh",
            secure,
            domain,
        )
        .parse()
        .map_err(|_| redirect_with_error(&redirect_target, "social_auth_exchange", secure, domain))?,
    );

    // Clear state cookie
    response_headers.append(
        header::SET_COOKIE,
        clear_cookie(SOCIAL_STATE_COOKIE, "/api/v1/auth/social", secure, domain)
            .parse()
            .map_err(|_| {
                redirect_with_error(&redirect_target, "social_auth_exchange", secure, domain)
            })?,
    );
    response_headers.append(
        header::SET_COOKIE,
        clear_cookie(SOCIAL_CLIENT_COOKIE, "/api/v1/auth/social", secure, domain)
            .parse()
            .map_err(|_| {
                redirect_with_error(&redirect_target, "social_auth_exchange", secure, domain)
            })?,
    );
    response_headers.append(
        header::SET_COOKIE,
        clear_cookie(SOCIAL_REDIRECT_COOKIE, "/api/v1/auth/social", secure, domain)
            .parse()
            .map_err(|_| {
                redirect_with_error(&redirect_target, "social_auth_exchange", secure, domain)
            })?,
    );

    let redirect_url = build_success_redirect_url(
        &redirect_target,
        provider.as_str(),
        &user.id,
        &tokens.access_token,
        tokens.access_expires_in,
        &tokens.refresh_token,
    );
    response_headers.insert(
        header::LOCATION,
        redirect_url.parse().map_err(|_| {
            redirect_with_error(&redirect_target, "social_auth_exchange", secure, domain)
        })?,
    );

    Ok((StatusCode::FOUND, response_headers, ()))
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

/// Build an error redirect response that clears the state cookie.
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
    if let Ok(cookie) =
        clear_cookie(SOCIAL_STATE_COOKIE, "/api/v1/auth/social", secure, domain).parse()
    {
        headers.append(header::SET_COOKIE, cookie);
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

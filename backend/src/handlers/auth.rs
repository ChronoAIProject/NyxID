use std::net::SocketAddr;

use axum::{
    Json,
    body::Bytes,
    extract::{ConnectInfo, State},
    http::{HeaderMap, header},
};
use serde::{Deserialize, Serialize};
use validator::{Validate, ValidationError};

use mongodb::bson::doc;

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::models::user::{COLLECTION_NAME as USERS, User};
use crate::mw::auth::{ACCESS_TOKEN_COOKIE_NAME, AuthUser, SESSION_COOKIE_NAME};
use crate::services::{
    audit_service, auth_service, feature_flag_service, invite_code_service, role_service,
    token_service,
};
use crate::telemetry::{TelemetryContext, TelemetryEvent, emit_event, hash_short_id};

// --- Request / Response types ---

#[derive(Debug, Deserialize, Validate)]
pub struct RegisterRequest {
    #[validate(email(message = "Invalid email address"))]
    pub email: String,
    #[validate(length(
        min = 8,
        max = 128,
        message = "Password must be between 8 and 128 characters"
    ))]
    pub password: String,
    /// Required when the invitation-code feature flag is enabled.
    #[serde(default)]
    pub invite_code: Option<String>,
    pub display_name: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct RegisterResponse {
    pub user_id: String,
    pub message: String,
}

#[derive(Debug, Deserialize, Validate)]
pub struct LoginRequest {
    #[validate(email(message = "Invalid email address"))]
    pub email: String,
    #[validate(length(max = 128, message = "Password too long"))]
    pub password: String,
    pub mfa_code: Option<String>,
    pub client: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct LoginResponse {
    pub user_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub access_token: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_in: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct RefreshResponse {
    pub access_token: String,
    pub expires_in: i64,
    pub refresh_token: String,
}

#[derive(Debug, Serialize)]
pub struct LogoutResponse {
    pub message: String,
}

pub(crate) const REFRESH_TOKEN_COOKIE_NAME: &str = "nyx_refresh_token";

const WEB_CLIENT_KIND: &str = "web";
const MOBILE_CLIENT_KIND: &str = "mobile";
const TOKEN_CLIENT_KIND: &str = "token";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum AuthClientMode {
    BrowserSession,
    TokenClient,
}

// --- Helper functions ---

pub(crate) use super::admin_helpers::extract_ip;

/// Extract the User-Agent header.
pub(crate) fn extract_user_agent(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(String::from)
}

/// Lowercase domain portion of an email (everything after the last `@`).
/// Returns `None` when the email is empty or has no `@`. Public so the
/// signup telemetry path can populate `user.signed_up.email_domain`
/// without leaking the full address through the egress scrubber.
pub(crate) fn extract_email_domain(email: &str) -> Option<String> {
    email
        .rsplit_once('@')
        .map(|(_, domain)| domain.trim().to_lowercase())
        .filter(|d| !d.is_empty())
}

/// Bare host portion of the HTTP `Referer` header. Strips scheme, path,
/// query, and any port suffix so that referer URLs with PII in path or
/// query never reach telemetry. Returns `None` when the header is
/// missing, malformed, or carries no host.
pub(crate) fn extract_referrer_domain(headers: &HeaderMap) -> Option<String> {
    let raw = headers.get(header::REFERER).and_then(|v| v.to_str().ok())?;
    // Tolerate scheme-relative (`//host/...`) and bare-host references.
    let after_scheme = raw
        .find("://")
        .map(|i| &raw[i + 3..])
        .unwrap_or(raw.strip_prefix("//").unwrap_or(raw));
    let host = after_scheme
        .split(['/', '?', '#'])
        .next()
        .unwrap_or("")
        .split('@')
        .next_back()
        .unwrap_or("");
    let host = host.split(':').next().unwrap_or("").trim().to_lowercase();
    if host.is_empty() { None } else { Some(host) }
}

fn resolve_cli_session_user_agent(
    client_ua: Option<String>,
    headers: &HeaderMap,
) -> Option<String> {
    client_ua
        .map(|ua| ua.trim().to_string())
        .filter(|ua| !ua.is_empty())
        .or_else(|| extract_user_agent(headers))
}

fn validate_cli_user_agent(client_ua: &str) -> Result<(), ValidationError> {
    if client_ua.chars().any(|ch| ch.is_ascii_control()) {
        return Err(ValidationError::new("cli_user_agent_control_chars"));
    }

    Ok(())
}

fn parse_cli_token_request_body(body: Bytes) -> AppResult<Option<CliTokenRequest>> {
    if body.is_empty() {
        return Ok(None);
    }

    let request: CliTokenRequest = serde_json::from_slice(&body)
        .map_err(|_| AppError::BadRequest("Invalid CLI token request body".to_string()))?;
    request
        .validate()
        .map_err(|e| AppError::ValidationError(e.to_string()))?;

    Ok(Some(request))
}

/// Build a Set-Cookie header value for an HttpOnly cookie with explicit SameSite policy.
/// The Secure flag is set based on the deployment environment.
/// When `domain` is provided, includes `Domain=<value>` for cross-subdomain sharing.
pub(crate) fn build_cookie_with_same_site(
    name: &str,
    value: &str,
    max_age_secs: i64,
    path: &str,
    secure: bool,
    domain: Option<&str>,
    same_site: &str,
) -> String {
    let secure_flag = if secure { "; Secure" } else { "" };
    let domain_attr = domain.map(|d| format!("; Domain={d}")).unwrap_or_default();
    format!(
        "{}={}; HttpOnly; SameSite={}; Path={}; Max-Age={}{}{}",
        name, value, same_site, path, max_age_secs, secure_flag, domain_attr
    )
}

/// Build a Set-Cookie header value for an HttpOnly, SameSite=Lax cookie.
/// The Secure flag is set based on the deployment environment.
/// When `domain` is provided, includes `Domain=<value>` for cross-subdomain sharing.
pub(crate) fn build_cookie(
    name: &str,
    value: &str,
    max_age_secs: i64,
    path: &str,
    secure: bool,
    domain: Option<&str>,
) -> String {
    build_cookie_with_same_site(name, value, max_age_secs, path, secure, domain, "Lax")
}

/// Build a cookie-clearing header value with explicit SameSite policy.
/// When `domain` is provided, includes `Domain=<value>` so the browser clears
/// the correct cross-subdomain cookie.
pub(crate) fn clear_cookie_with_same_site(
    name: &str,
    path: &str,
    secure: bool,
    domain: Option<&str>,
    same_site: &str,
) -> String {
    let secure_flag = if secure { "; Secure" } else { "" };
    let domain_attr = domain.map(|d| format!("; Domain={d}")).unwrap_or_default();
    format!(
        "{}=; HttpOnly; SameSite={}; Path={}; Max-Age=0{}{}",
        name, same_site, path, secure_flag, domain_attr
    )
}

/// Build a SameSite=Lax cookie-clearing header value.
pub(crate) fn clear_cookie(name: &str, path: &str, secure: bool, domain: Option<&str>) -> String {
    clear_cookie_with_same_site(name, path, secure, domain, "Lax")
}

pub(crate) fn append_set_cookie(
    response_headers: &mut HeaderMap,
    cookie_value: String,
) -> AppResult<()> {
    response_headers.append(
        header::SET_COOKIE,
        cookie_value
            .parse()
            .map_err(|_| AppError::Internal("Failed to build cookie header".to_string()))?,
    );
    Ok(())
}

pub(crate) fn clear_legacy_auth_cookies(
    response_headers: &mut HeaderMap,
    secure: bool,
    domain: Option<&str>,
) -> AppResult<()> {
    append_set_cookie(
        response_headers,
        clear_cookie(ACCESS_TOKEN_COOKIE_NAME, "/", secure, domain),
    )?;
    append_set_cookie(
        response_headers,
        clear_cookie(
            REFRESH_TOKEN_COOKIE_NAME,
            "/api/v1/auth/refresh",
            secure,
            domain,
        ),
    )?;
    Ok(())
}

pub(crate) fn apply_browser_session_cookies(
    response_headers: &mut HeaderMap,
    session_token: &str,
    secure: bool,
    domain: Option<&str>,
) -> AppResult<()> {
    append_set_cookie(
        response_headers,
        build_cookie(
            SESSION_COOKIE_NAME,
            session_token,
            token_service::SESSION_TTL_SECS,
            "/",
            secure,
            domain,
        ),
    )?;
    clear_legacy_auth_cookies(response_headers, secure, domain)
}

fn looks_like_browser_request(headers: &HeaderMap) -> bool {
    headers.contains_key(header::ORIGIN)
        || headers.contains_key(header::REFERER)
        || headers.contains_key("sec-fetch-site")
        || headers.contains_key("sec-fetch-mode")
        || headers.contains_key("sec-fetch-dest")
}

pub(crate) fn resolve_auth_client_mode(
    headers: &HeaderMap,
    explicit_client: Option<&str>,
) -> AuthClientMode {
    let normalized_client = explicit_client
        .map(str::trim)
        .filter(|client| !client.is_empty());

    match normalized_client {
        Some(client) if client.eq_ignore_ascii_case(WEB_CLIENT_KIND) => {
            AuthClientMode::BrowserSession
        }
        Some(client)
            if client.eq_ignore_ascii_case(MOBILE_CLIENT_KIND)
                || client.eq_ignore_ascii_case(TOKEN_CLIENT_KIND) =>
        {
            AuthClientMode::TokenClient
        }
        _ if looks_like_browser_request(headers) => AuthClientMode::BrowserSession,
        _ => AuthClientMode::TokenClient,
    }
}

// --- Handlers ---

/// POST /api/v1/auth/register
///
/// Process registration and return sign-in guidance. Email verification status
/// follows AUTO_VERIFY_EMAIL; password login currently accepts active accounts.
pub async fn register(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<RegisterRequest>,
) -> AppResult<Json<RegisterResponse>> {
    if !state.config.email_auth_enabled {
        return Err(AppError::EmailSignupDisabled);
    }

    body.validate()
        .map_err(|e| AppError::ValidationError(e.to_string()))?;

    // When the invite-code gate is enabled, an invite code is mandatory and
    // we reserve one slot up front. When it is disabled (public launch),
    // any invite code the client sent is ignored and registration proceeds
    // without reserving anything.
    let invite_code_id = if feature_flag_service::invitation_code_required(&state.db).await? {
        let raw_code = body
            .invite_code
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .ok_or_else(|| AppError::ValidationError("Invite code is required".to_string()))?;
        match invite_code_service::reserve_invite_code(&state.db, raw_code, &body.email).await {
            Ok(id) => Some(id),
            // Enumeration-safe: treat an already-redeemed code as if the
            // email address simply doesn't exist yet. Return the same
            // fake-success response used for duplicate-email registrations so
            // callers cannot distinguish "code used" from "email unknown".
            // Nothing was reserved, so there is nothing to release. We mirror
            // the duplicate-email path's audit side-effect (and add a warn) so
            // the two paths are indistinguishable AND the reuse attempt is
            // still recorded for audit/monitoring.
            Err(AppError::InviteCodeAlreadyRedeemed) => {
                tracing::warn!("Registration attempt with an already-redeemed invite code");
                let message = "Registration processed. You can now sign in.".to_string();
                let fake_user_id = uuid::Uuid::new_v4().to_string();
                audit_service::log_async(
                    state.db.clone(),
                    Some(fake_user_id.clone()),
                    "register".to_string(),
                    Some(serde_json::json!({ "email": body.email })),
                    extract_ip(&headers, Some(peer), &state.config.trusted_proxy_ips),
                    extract_user_agent(&headers),
                    None,
                    None,
                );
                return Ok(Json(RegisterResponse {
                    user_id: fake_user_id,
                    message,
                }));
            }
            Err(e) => return Err(e),
        }
    } else {
        None
    };

    let register_result = auth_service::register_user(
        &state.db,
        &body.email,
        &body.password,
        body.display_name.as_deref(),
        invite_code_id.as_deref(),
        state.config.auto_verify_email,
    )
    .await;

    let result = match register_result {
        Ok(r) if r.actually_created => {
            if let Some(ref code_id) = invite_code_id {
                invite_code_service::record_usage(&state.db, code_id, &r.user_id, &body.email)
                    .await;
            }
            r
        }
        // The two arms below silently no-op the invite-code accounting:
        // `Ok(_)` is the email-enumeration-protection fake-success branch
        // (no real user was created); `Err(_)` is a downstream failure.
        // In both cases we release the reservation and do NOT emit
        // `invite.code_redeemed` — only an actually-redeemed code counts.
        Ok(r) => {
            // Email already existed: the service returned a fake-success to
            // prevent enumeration. The invite code slot (if any) was never
            // actually used, so release it before returning the fake
            // result to the caller.
            if let Some(ref code_id) = invite_code_id {
                invite_code_service::release_reservation(&state.db, code_id).await;
            }
            r
        }
        Err(e) => {
            // Registration failed for another reason (hash, DB write, etc).
            // Release any reservation before surfacing the error.
            if let Some(ref code_id) = invite_code_id {
                invite_code_service::release_reservation(&state.db, code_id).await;
            }
            return Err(e);
        }
    };

    audit_service::log_async(
        state.db.clone(),
        Some(result.user_id.clone()),
        "register".to_string(),
        Some(serde_json::json!({ "email": body.email })),
        extract_ip(&headers, Some(peer), &state.config.trusted_proxy_ips),
        extract_user_agent(&headers),
        None,
        None,
    );
    if result.actually_created {
        ensure_billing_wallet_after_user_create(&state, &result.user_id).await;
    }

    let message = "Registration processed. You can now sign in.".to_string();

    // Telemetry: user.signed_up. Pre-auth path, so surface comes from
    // the `X-NyxID-Client` header rather than from `AuthUser`.
    let tele = TelemetryContext::from_headers(
        headers.get("x-nyxid-client").and_then(|v| v.to_str().ok()),
        headers
            .get("x-nyxid-client-version")
            .and_then(|v| v.to_str().ok()),
    );

    // Skip telemetry on the fake-success enumeration-protection branch:
    // `result.actually_created` is false there and no real user exists.
    // Emitting would inflate signup counts with phantom users.
    if result.actually_created {
        let invite_code_id_hash = invite_code_id.as_deref().map(hash_short_id);
        let source = if invite_code_id.is_some() {
            "invite_code".to_string()
        } else {
            "direct".to_string()
        };
        emit_event(
            state.telemetry.as_deref(),
            &result.user_id,
            None,
            &tele,
            TelemetryEvent::UserSignedUp {
                method: "email".to_string(),
                source,
                email_domain: extract_email_domain(&body.email),
                invite_code_id: invite_code_id_hash,
                referrer_domain: extract_referrer_domain(&headers),
                via_org: None,
                invite_code_used: invite_code_id.is_some(),
            },
        );

        // Emit `invite.code_redeemed` after the user is actually created so
        // the funnel (`invite.code_generated` → `invite.code_redeemed`)
        // counts only successful conversions. Metadata is best-effort:
        // a missing fetch result drops the event rather than fabricating
        // placeholder ids.
        if let Some(ref code_id) = invite_code_id
            && let Some(meta) = invite_code_service::fetch_telemetry_meta(&state.db, code_id).await
        {
            let days = (chrono::Utc::now() - meta.created_at).num_days().max(0) as u64;
            emit_event(
                state.telemetry.as_deref(),
                &result.user_id,
                None,
                &tele,
                TelemetryEvent::InviteCodeRedeemed {
                    code_id: hash_short_id(code_id),
                    created_by_user_id: hash_short_id(&meta.created_by),
                    days_to_redemption: days,
                },
            );
        }
    }

    Ok(Json(RegisterResponse {
        user_id: result.user_id,
        message,
    }))
}

/// POST /api/v1/auth/login
///
/// Authenticate with email and password. If MFA is enabled, returns a
/// 403 with mfa_required unless a valid mfa_code is provided.
/// On success, sets HttpOnly cookies and returns the access token.
pub async fn login(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<LoginRequest>,
) -> AppResult<(HeaderMap, Json<LoginResponse>)> {
    body.validate()
        .map_err(|e| AppError::ValidationError(e.to_string()))?;

    let user = auth_service::authenticate_user(&state.db, &body.email, &body.password).await?;

    // Retain the verified factor so issuance can fence it in its transaction.
    let mut mfa_factor = None;
    if user.mfa_enabled {
        match &body.mfa_code {
            Some(code) => {
                // Build the telemetry context eagerly so both failure
                // branches (length abuse + wrong code) can emit the same
                // `mfa.challenge_failed` event before returning the error.
                let tele_mfa = TelemetryContext::from_headers(
                    headers.get("x-nyxid-client").and_then(|v| v.to_str().ok()),
                    headers
                        .get("x-nyxid-client-version")
                        .and_then(|v| v.to_str().ok()),
                );

                // Validate MFA code length to prevent abuse
                if code.len() > 10 {
                    emit_event(
                        state.telemetry.as_deref(),
                        &user.id.to_string(),
                        None,
                        &tele_mfa,
                        TelemetryEvent::MfaChallengeFailed {
                            factor_type: "totp".to_string(),
                            reason: "wrong_code".to_string(),
                        },
                    );
                    return Err(AppError::AuthenticationFailed(
                        "Invalid MFA code".to_string(),
                    ));
                }

                let verified = crate::services::mfa_service::verified_factor(
                    &state.db,
                    &state.encryption_keys,
                    &user.id,
                    code,
                )
                .await?;

                if verified.is_none() {
                    emit_event(
                        state.telemetry.as_deref(),
                        &user.id.to_string(),
                        None,
                        &tele_mfa,
                        TelemetryEvent::MfaChallengeFailed {
                            factor_type: "totp".to_string(),
                            reason: "wrong_code".to_string(),
                        },
                    );
                    return Err(AppError::AuthenticationFailed(
                        "Invalid MFA code".to_string(),
                    ));
                }

                mfa_factor = verified;

                // Inline-MFA success: pair with the failure emit above so
                // the MFA funnel sees both outcomes from the
                // single-request `/auth/login` path. The standalone
                // `/auth/mfa/verify` path emits its own success event.
                emit_event(
                    state.telemetry.as_deref(),
                    &user.id.to_string(),
                    None,
                    &tele_mfa,
                    TelemetryEvent::MfaChallengeSucceeded {
                        factor_type: "totp".to_string(),
                    },
                );
            }
            None => {
                // Store a temporary MFA session bound to the user.
                // The temp_token is hashed and stored in the database
                // so the MFA step can be tied to a prior password verification.
                let temp_token = crate::crypto::token::generate_random_token();
                let temp_token_hash = crate::crypto::token::hash_token(&temp_token);

                // Store the MFA session as a short-lived session record
                token_service::create_mfa_pending_session(
                    &state.db,
                    &state.audit_runtime,
                    &user.id,
                    &temp_token_hash,
                )
                .await?;

                return Err(AppError::MfaRequired {
                    session_token: temp_token,
                });
            }
        }
    }

    let ip = extract_ip(&headers, Some(peer), &state.config.trusted_proxy_ips);
    let ua = extract_user_agent(&headers);

    // Telemetry context is derived here from the X-NyxID-Client headers
    // so both match arms can emit `auth.logged_in` AFTER their
    // state-changing session/token creation succeeds. Emitting
    // before-the-match risked reporting "logged in" on a request that
    // later failed inside `create_session*`.
    let tele_login = TelemetryContext::from_headers(
        headers.get("x-nyxid-client").and_then(|v| v.to_str().ok()),
        headers
            .get("x-nyxid-client-version")
            .and_then(|v| v.to_str().ok()),
    );
    let client_mode = resolve_auth_client_mode(&headers, body.client.as_deref());
    let secure = state.config.use_secure_cookies();
    let domain = state.config.cookie_domain();
    let mut response_headers = HeaderMap::new();

    match client_mode {
        AuthClientMode::BrowserSession => {
            let session = if let Some(factor) = &mfa_factor {
                let prepared = crate::services::mfa_service::MfaSession::browser(
                    token_service::prepare_browser_session(&user.id, ip.as_deref(), ua.as_deref())?,
                );
                crate::services::mfa_service::complete_login(
                    &state.db,
                    &state.audit_runtime,
                    None,
                    factor,
                    &prepared,
                )
                .await?;
                let crate::services::mfa_service::MfaSession::Browser(value) = prepared else {
                    unreachable!()
                };
                value.issued
            } else {
                token_service::create_session(
                    &state.db,
                    &state.audit_runtime,
                    &user.id,
                    ip.as_deref(),
                    ua.as_deref(),
                )
                .await?
            };

            apply_browser_session_cookies(
                &mut response_headers,
                &session.session_token,
                secure,
                domain,
            )?;

            emit_event(
                state.telemetry.as_deref(),
                &user.id.to_string(),
                None,
                &tele_login,
                TelemetryEvent::AuthLoggedIn {
                    method: "password".to_string(),
                    mfa_required: body.mfa_code.is_some(),
                },
            );

            Ok((
                response_headers,
                Json(LoginResponse {
                    user_id: user.id.to_string(),
                    access_token: None,
                    expires_in: None,
                    refresh_token: None,
                }),
            ))
        }
        AuthClientMode::TokenClient => {
            let tokens = if let Some(factor) = &mfa_factor {
                let prepared = crate::services::mfa_service::MfaSession::tokens(
                    token_service::prepare_session_tokens(
                        &state.db,
                        &state.config,
                        &state.jwt_keys,
                        &user.id,
                        ip.as_deref(),
                        ua.as_deref(),
                    )
                    .await?,
                );
                crate::services::mfa_service::complete_login(
                    &state.db,
                    &state.audit_runtime,
                    None,
                    factor,
                    &prepared,
                )
                .await?;
                let crate::services::mfa_service::MfaSession::Tokens(value) = prepared else {
                    unreachable!()
                };
                value.tokens
            } else {
                token_service::create_session_and_issue_tokens(
                    &state.db,
                    &state.audit_runtime,
                    &state.config,
                    &state.jwt_keys,
                    &user.id,
                    ip.as_deref(),
                    ua.as_deref(),
                )
                .await?
            };

            emit_event(
                state.telemetry.as_deref(),
                &user.id.to_string(),
                None,
                &tele_login,
                TelemetryEvent::AuthLoggedIn {
                    method: "password".to_string(),
                    mfa_required: body.mfa_code.is_some(),
                },
            );

            Ok((
                response_headers,
                Json(LoginResponse {
                    user_id: user.id.to_string(),
                    access_token: Some(tokens.access_token),
                    expires_in: Some(tokens.access_expires_in),
                    refresh_token: Some(tokens.refresh_token),
                }),
            ))
        }
    }
}

/// POST /api/v1/auth/logout
///
/// Revoke the current session and clear all auth cookies.
pub async fn logout(
    State(state): State<AppState>,
    ConnectInfo(_peer): ConnectInfo<SocketAddr>,
    auth_user: AuthUser,
    tele: TelemetryContext,
    _headers: HeaderMap,
) -> AppResult<(HeaderMap, Json<LogoutResponse>)> {
    if let Some(session_id) = auth_user.session_id {
        token_service::revoke_session(
            &state.db,
            &state.audit_runtime,
            &session_id.to_string(),
            token_service::audit_principal(&auth_user)?,
            Some(&state.mcp_sessions),
        )
        .await?;
    }

    emit_event(
        state.telemetry.as_deref(),
        &auth_user.user_id.to_string(),
        auth_user.api_key_id.as_deref(),
        &tele,
        TelemetryEvent::AuthLoggedOut,
    );

    let secure = state.config.use_secure_cookies();
    let domain = state.config.cookie_domain();

    let mut response_headers = HeaderMap::new();
    response_headers.insert(
        header::SET_COOKIE,
        clear_cookie(SESSION_COOKIE_NAME, "/", secure, domain)
            .parse()
            .map_err(|_| AppError::Internal("Failed to build cookie header".to_string()))?,
    );
    response_headers.append(
        header::SET_COOKIE,
        clear_cookie(ACCESS_TOKEN_COOKIE_NAME, "/", secure, domain)
            .parse()
            .map_err(|_| AppError::Internal("Failed to build cookie header".to_string()))?,
    );
    response_headers.append(
        header::SET_COOKIE,
        clear_cookie(
            REFRESH_TOKEN_COOKIE_NAME,
            "/api/v1/auth/refresh",
            secure,
            domain,
        )
        .parse()
        .map_err(|_| AppError::Internal("Failed to build cookie header".to_string()))?,
    );

    Ok((
        response_headers,
        Json(LogoutResponse {
            message: "Logged out successfully".to_string(),
        }),
    ))
}

/// Optional JSON body for /auth/refresh — mobile clients send the refresh
/// token in the body since HttpOnly cookies are unreliable outside browsers.
#[derive(Debug, Deserialize, Default)]
pub struct RefreshRequest {
    pub refresh_token: Option<String>,
}

/// POST /api/v1/auth/refresh
///
/// Exchange a refresh token for a new access token for token-based clients.
/// Browser sessions do not use this endpoint.
pub async fn refresh(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Option<Json<RefreshRequest>>,
) -> AppResult<(HeaderMap, Json<RefreshResponse>)> {
    let refresh_token = body
        .and_then(|payload| payload.0.refresh_token)
        .filter(|token| !token.trim().is_empty())
        .ok_or_else(|| AppError::Unauthorized("No refresh token provided".to_string()))?;

    let tokens = token_service::refresh_tokens(
        &state.db,
        &state.audit_runtime,
        &state.config,
        &state.jwt_keys,
        &refresh_token,
        Some(&state.mcp_sessions),
        None,
    )
    .await?;

    // Telemetry: auth.token_refreshed. Pre-auth path (no AuthUser),
    // so we decode the newly-issued access token to get the user id.
    if let Ok(claims) =
        crate::crypto::jwt::verify_token(&state.jwt_keys, &state.config, &tokens.access_token)
    {
        let tele = TelemetryContext::from_headers(
            headers.get("x-nyxid-client").and_then(|v| v.to_str().ok()),
            headers
                .get("x-nyxid-client-version")
                .and_then(|v| v.to_str().ok()),
        );
        emit_event(
            state.telemetry.as_deref(),
            &claims.sub,
            None,
            &tele,
            TelemetryEvent::AuthTokenRefreshed,
        );
    }

    Ok((
        HeaderMap::new(),
        Json(RefreshResponse {
            access_token: tokens.access_token,
            expires_in: tokens.access_expires_in,
            refresh_token: tokens.refresh_token,
        }),
    ))
}

// --- Verify Email ---

#[derive(Debug, Deserialize)]
pub struct VerifyEmailRequest {
    pub token: String,
}

#[derive(Debug, Serialize)]
pub struct VerifyEmailResponse {
    pub message: String,
}

/// POST /api/v1/auth/verify-email
///
/// Verify a user's email address using the token sent during registration.
pub async fn verify_email(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<VerifyEmailRequest>,
) -> AppResult<Json<VerifyEmailResponse>> {
    let user_id = auth_service::verify_email(&state.db, &body.token).await?;

    // Telemetry: user.email_verified. Pre-auth path (token-gated), so
    // surface comes from the `X-NyxID-Client` header.
    let tele = TelemetryContext::from_headers(
        headers.get("x-nyxid-client").and_then(|v| v.to_str().ok()),
        headers
            .get("x-nyxid-client-version")
            .and_then(|v| v.to_str().ok()),
    );
    emit_event(
        state.telemetry.as_deref(),
        &user_id,
        None,
        &tele,
        TelemetryEvent::UserEmailVerified,
    );

    Ok(Json(VerifyEmailResponse {
        message: "Email verified successfully".to_string(),
    }))
}

// --- Forgot Password ---

#[derive(Debug, Deserialize, Validate)]
pub struct ForgotPasswordRequest {
    #[validate(email(message = "Invalid email address"))]
    pub email: String,
}

#[derive(Debug, Serialize)]
pub struct ForgotPasswordResponse {
    pub message: String,
}

/// POST /api/v1/auth/forgot-password
///
/// Initiate a password reset flow. Always returns success to prevent
/// email enumeration.
pub async fn forgot_password(
    State(state): State<AppState>,
    Json(body): Json<ForgotPasswordRequest>,
) -> AppResult<Json<ForgotPasswordResponse>> {
    body.validate()
        .map_err(|e| AppError::ValidationError(e.to_string()))?;

    // Always return success to prevent email enumeration
    let _token = auth_service::initiate_password_reset(&state.db, &body.email).await?;

    // In production, send the reset token via email.
    // In development, the token is logged for testing.
    #[cfg(debug_assertions)]
    if let Some(ref token) = _token {
        tracing::debug!(token = %token, "Password reset token generated (dev only)");
    }

    // TODO(telemetry): blocked — see TELEMETRY.md §6.5 (auth.password_reset_requested).
    // `initiate_password_reset` returns `Option<String>` (the bare reset token)
    // and intentionally does not reveal whether the email matched a user,
    // so we have no `user_id` to use as distinct_id without a service
    // refactor. Emitting here would require changing the service to return
    // the user_id alongside the token, which is explicitly out of scope for
    // this sweep per the Part-2 hard scope rule.

    Ok(Json(ForgotPasswordResponse {
        message: "If that email exists, a password reset link has been sent.".to_string(),
    }))
}

// --- Reset Password ---

#[derive(Debug, Deserialize, Validate)]
pub struct ResetPasswordRequest {
    pub token: String,
    #[validate(length(
        min = 8,
        max = 128,
        message = "Password must be between 8 and 128 characters"
    ))]
    pub new_password: String,
}

#[derive(Debug, Serialize)]
pub struct ResetPasswordResponse {
    pub message: String,
}

/// POST /api/v1/auth/reset-password
///
/// Complete a password reset using the token and a new password.
pub async fn reset_password(
    State(state): State<AppState>,
    Json(body): Json<ResetPasswordRequest>,
) -> AppResult<Json<ResetPasswordResponse>> {
    body.validate()
        .map_err(|e| AppError::ValidationError(e.to_string()))?;

    auth_service::reset_password(&state.db, &body.token, &body.new_password).await?;

    // TODO(telemetry): blocked — see TELEMETRY.md §6.5 (auth.password_reset_completed).
    // `auth_service::reset_password` returns `AppResult<()>` and the handler
    // has no `AuthUser` (pre-auth path), so we have no `user_id` to use as
    // distinct_id. Emitting here would require changing the service to
    // return the user_id, which is explicitly out of scope for this sweep.

    Ok(Json(ResetPasswordResponse {
        message: "Password has been reset successfully".to_string(),
    }))
}

// --- Bootstrap Setup ---

#[derive(Debug, Deserialize, Validate)]
pub struct SetupRequest {
    #[validate(email(message = "Invalid email address"))]
    pub email: String,
    #[validate(length(
        min = 8,
        max = 128,
        message = "Password must be between 8 and 128 characters"
    ))]
    pub password: String,
    pub display_name: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct SetupResponse {
    pub user_id: String,
    pub message: String,
}

/// POST /api/v1/auth/setup
///
/// One-time bootstrap endpoint to create the initial admin user.
/// Only works when the users collection is empty. After the first admin
/// is created, this endpoint returns 403 Forbidden.
pub async fn setup(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<SetupRequest>,
) -> AppResult<Json<SetupResponse>> {
    body.validate()
        .map_err(|e| AppError::ValidationError(e.to_string()))?;

    // Guard: only allow setup when no users exist
    let user_count = state
        .db
        .collection::<User>(USERS)
        .count_documents(doc! {})
        .await?;

    if user_count > 0 {
        return Err(AppError::Forbidden(
            "Setup has already been completed. Use the CLI --promote-admin flag to promote existing users.".to_string(),
        ));
    }

    // Create the user via the normal registration flow
    let result = auth_service::register_user(
        &state.db,
        &body.email,
        &body.password,
        body.display_name.as_deref(),
        None,
        true, // Admin setup always auto-verifies
    )
    .await?;

    // Promote to admin and mark email as verified. The RBAC membership is
    // authoritative; the legacy flag is mirrored during the migration window.
    let platform_role_ids = role_service::get_platform_role_ids(&state.db).await?;
    let mut pipeline = role_service::set_platform_role_update(
        crate::models::user::PlatformRole::Admin,
        &platform_role_ids,
        mongodb::bson::DateTime::from_chrono(chrono::Utc::now()),
    );
    pipeline.push(doc! { "$set": { "email_verified": true } });
    state
        .db
        .collection::<User>(USERS)
        .update_one(doc! { "_id": &result.user_id }, pipeline)
        .await?;

    audit_service::log_async(
        state.db.clone(),
        Some(result.user_id.clone()),
        "admin_setup".to_string(),
        Some(serde_json::json!({
            "email": body.email,
            "method": "bootstrap"
        })),
        extract_ip(&headers, Some(peer), &state.config.trusted_proxy_ips),
        extract_user_agent(&headers),
        None,
        None,
    );
    ensure_billing_wallet_after_user_create(&state, &result.user_id).await;

    tracing::info!(user_id = %result.user_id, email = %body.email, "Initial admin created via bootstrap");

    Ok(Json(SetupResponse {
        user_id: result.user_id,
        message: "Admin account created successfully.".to_string(),
    }))
}

async fn ensure_billing_wallet_after_user_create(state: &AppState, user_id: &str) {
    if !state.billing.billing_enabled() {
        return;
    }
    if !state.billing.lago_configured() {
        tracing::warn!(
            user_id,
            "Billing is enabled but Lago is not configured; new-user wallet provisioning skipped"
        );
        return;
    }

    if let Err(error) = state.billing.ensure_wallet(user_id).await {
        tracing::warn!(
            user_id,
            error = %error,
            "New-user billing wallet provisioning failed"
        );
    }
}

/// POST /api/v1/auth/cli-token
///
/// Issue an access token and refresh token for the CLI. Requires a first-party
/// human session or first-party access token. The `/cli-auth` frontend page
/// uses its session cookie after browser login.
///
/// The `client_ua` field in the request body carries the CLI's own User-Agent
/// string through the browser round-trip so the session is recorded as a CLI
/// session rather than a browser session.
pub async fn cli_token(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    auth_user: AuthUser,
    headers: HeaderMap,
    body: Bytes,
) -> AppResult<Json<CliTokenResponse>> {
    super::login_client_context::require_first_party_human(&auth_user)?;
    let client_ua = parse_cli_token_request_body(body)?.and_then(|body| body.client_ua);

    let user_id_str = auth_user.user_id.to_string();
    let user_agent = resolve_cli_session_user_agent(client_ua, &headers);

    let prepared = token_service::prepare_session_tokens(
        &state.db,
        &state.config,
        &state.jwt_keys,
        &user_id_str,
        extract_ip(&headers, Some(peer), &state.config.trusted_proxy_ips).as_deref(),
        user_agent.as_deref(),
    )
    .await?
    .with_principal(token_service::audit_principal(&auth_user)?);
    let tokens =
        token_service::issue_prepared_session(&state.db, &state.audit_runtime, prepared).await?;

    Ok(Json(CliTokenResponse {
        access_token: tokens.access_token,
        refresh_token: tokens.refresh_token,
    }))
}

#[derive(Debug, Deserialize, Validate)]
pub struct CliTokenRequest {
    /// The CLI's User-Agent string, passed through the browser round-trip.
    #[validate(length(max = 512, message = "CLI user-agent too long"))]
    #[validate(custom(function = "validate_cli_user_agent"))]
    pub client_ua: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct CliTokenResponse {
    pub access_token: String,
    pub refresh_token: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Bytes;

    #[tokio::test]
    async fn w8_id_cli_session_refuses_authenticated_key() {
        use crate::{
            models::{
                audit_outbox::{PENDING_COLLECTION_NAME, PendingAuditEvent},
                user::{COLLECTION_NAME as USERS, User, UserType},
            },
            test_utils::*,
        };
        use axum::{
            Router,
            body::Body,
            http::{Request, StatusCode},
            routing::post,
        };
        use tower::ServiceExt;
        use uuid::Uuid;
        let db = connect_transaction_test_database("w8_id_cli_key_session").await;
        let state = test_app_state(db.clone());
        let owner = Uuid::new_v4().to_string();
        let key = Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_one(test_user(&owner, UserType::Person))
            .await
            .unwrap();
        let (prefix, secret, hash) = crate::crypto::token::generate_api_key();
        db.collection::<mongodb::bson::Document>(crate::models::api_key::COLLECTION_NAME)
            .insert_one(doc! {
                "_id": &key, "user_id": &owner, "name": "w8-id-private-key-name",
                "key_prefix": prefix, "key_hash": hash, "scopes": "read write",
                "is_active": true, "created_at": mongodb::bson::DateTime::now(),
            })
            .await
            .unwrap();
        let app = Router::new()
            .route("/api/v1/auth/cli-token", post(cli_token))
            .layer(axum::Extension(ConnectInfo(
                "127.0.0.1:23151".parse::<SocketAddr>().unwrap(),
            )))
            .with_state(state);
        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/auth/cli-token")
                    .header("x-api-key", &secret)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
        assert_eq!(
            db.collection::<PendingAuditEvent>(PENDING_COLLECTION_NAME)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            db.collection::<mongodb::bson::Document>(crate::models::session::COLLECTION_NAME)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
    }

    #[test]
    fn email_domain_extracts_lowercased_host_portion() {
        assert_eq!(
            extract_email_domain("Alice@Example.COM"),
            Some("example.com".into())
        );
        assert_eq!(extract_email_domain(""), None);
        assert_eq!(extract_email_domain("no-at-sign"), None);
        // Empty domain portion (`alice@`) must not produce an empty
        // string in telemetry — egress would still send `""` as a value.
        assert_eq!(extract_email_domain("alice@"), None);
    }

    #[test]
    fn referrer_domain_strips_scheme_path_and_port() {
        let mut headers = HeaderMap::new();
        headers.insert(
            header::REFERER,
            "https://example.com:443/path?leak=secret".parse().unwrap(),
        );
        assert_eq!(
            extract_referrer_domain(&headers),
            Some("example.com".into())
        );

        // Bare hostname (rare but allowed by RFC).
        let mut h2 = HeaderMap::new();
        h2.insert(header::REFERER, "twitter.com".parse().unwrap());
        assert_eq!(extract_referrer_domain(&h2), Some("twitter.com".into()));

        // Missing header → None (signup did not arrive from web).
        let empty = HeaderMap::new();
        assert_eq!(extract_referrer_domain(&empty), None);
    }

    #[test]
    fn explicit_mobile_client_uses_token_mode() {
        let headers = HeaderMap::new();
        assert_eq!(
            resolve_auth_client_mode(&headers, Some("mobile")),
            AuthClientMode::TokenClient
        );
    }

    #[test]
    fn browser_headers_default_to_session_mode() {
        let mut headers = HeaderMap::new();
        headers.insert(header::ORIGIN, "https://app.example.com".parse().unwrap());

        assert_eq!(
            resolve_auth_client_mode(&headers, None),
            AuthClientMode::BrowserSession
        );
    }

    #[test]
    fn non_browser_requests_default_to_token_mode() {
        let headers = HeaderMap::new();
        assert_eq!(
            resolve_auth_client_mode(&headers, None),
            AuthClientMode::TokenClient
        );
    }

    #[test]
    fn explicit_token_client_overrides_browser_headers() {
        let mut headers = HeaderMap::new();
        headers.insert(header::ORIGIN, "https://app.example.com".parse().unwrap());

        assert_eq!(
            resolve_auth_client_mode(&headers, Some("token")),
            AuthClientMode::TokenClient
        );
    }

    #[test]
    fn cli_session_prefers_client_user_agent_over_browser_header() {
        let mut headers = HeaderMap::new();
        headers.insert(header::USER_AGENT, "Mozilla/5.0".parse().unwrap());

        assert_eq!(
            resolve_cli_session_user_agent(Some("nyxid-cli/0.1.0".to_string()), &headers),
            Some("nyxid-cli/0.1.0".to_string())
        );
    }

    #[test]
    fn cli_session_falls_back_to_browser_header_when_client_user_agent_is_blank() {
        let mut headers = HeaderMap::new();
        headers.insert(header::USER_AGENT, "Mozilla/5.0".parse().unwrap());

        assert_eq!(
            resolve_cli_session_user_agent(Some("   ".to_string()), &headers),
            Some("Mozilla/5.0".to_string())
        );
    }

    #[test]
    fn cli_session_falls_back_to_browser_header_when_client_user_agent_is_missing() {
        let mut headers = HeaderMap::new();
        headers.insert(header::USER_AGENT, "Mozilla/5.0".parse().unwrap());

        assert_eq!(
            resolve_cli_session_user_agent(None, &headers),
            Some("Mozilla/5.0".to_string())
        );
    }

    #[test]
    fn cli_token_request_rejects_overlong_client_user_agent() {
        let body = CliTokenRequest {
            client_ua: Some("x".repeat(513)),
        };

        assert!(body.validate().is_err());
    }

    #[test]
    fn cli_token_request_rejects_control_characters() {
        let body = CliTokenRequest {
            client_ua: Some("nyxid-cli/0.1.0\nspoof".to_string()),
        };

        assert!(body.validate().is_err());
    }

    #[test]
    fn parse_cli_token_request_body_accepts_empty_body() {
        assert!(
            parse_cli_token_request_body(Bytes::new())
                .expect("empty body should be accepted")
                .is_none()
        );
    }

    #[test]
    fn parse_cli_token_request_body_rejects_invalid_json() {
        let error = parse_cli_token_request_body(Bytes::from_static(b"{"))
            .expect_err("invalid JSON should be rejected");

        assert!(matches!(error, AppError::BadRequest(_)));
    }

    #[test]
    fn extract_ip_from_x_forwarded_for() {
        let mut h = HeaderMap::new();
        h.insert("x-forwarded-for", "1.2.3.4, 5.6.7.8".parse().unwrap());
        assert_eq!(
            extract_ip(
                &h,
                Some("10.10.10.1:443".parse().unwrap()),
                &["10.10.10.1".parse().unwrap(), "5.6.7.8".parse().unwrap()]
            ),
            Some("1.2.3.4".to_string())
        );
    }

    #[test]
    fn extract_ip_from_x_real_ip() {
        let mut h = HeaderMap::new();
        h.insert("x-real-ip", "10.0.0.1".parse().unwrap());
        assert_eq!(
            extract_ip(
                &h,
                Some("10.10.10.1:443".parse().unwrap()),
                &["10.10.10.1".parse().unwrap(), "5.6.7.8".parse().unwrap()]
            ),
            Some("10.0.0.1".to_string())
        );
    }

    #[test]
    fn extract_ip_falls_back_to_peer() {
        let peer = "192.168.1.1:8080".parse().ok();
        assert_eq!(
            extract_ip(&HeaderMap::new(), peer, &[]),
            Some("192.168.1.1".to_string())
        );
    }

    #[test]
    fn extract_ip_returns_none_when_nothing_available() {
        assert_eq!(extract_ip(&HeaderMap::new(), None, &[]), None);
    }

    #[test]
    fn build_cookie_includes_all_attributes() {
        let c = build_cookie("name", "val", 3600, "/", true, Some(".example.com"));
        assert!(c.contains("name=val"));
        assert!(c.contains("HttpOnly"));
        assert!(c.contains("SameSite=Lax"));
        assert!(c.contains("Max-Age=3600"));
        assert!(c.contains("; Secure"));
        assert!(c.contains("Domain=.example.com"));
    }

    #[test]
    fn build_cookie_no_secure_for_http() {
        let c = build_cookie("n", "v", 60, "/", false, None);
        assert!(!c.contains("; Secure"));
        assert!(!c.contains("Domain="));
    }

    #[test]
    fn clear_cookie_sets_max_age_zero() {
        let c = clear_cookie("tok", "/", true, None);
        assert!(c.contains("Max-Age=0"));
        assert!(c.contains("tok=;"));
    }

    #[test]
    fn resolve_auth_client_mode_web_explicit() {
        assert_eq!(
            resolve_auth_client_mode(&HeaderMap::new(), Some("web")),
            AuthClientMode::BrowserSession
        );
        assert_eq!(
            resolve_auth_client_mode(&HeaderMap::new(), Some("WEB")),
            AuthClientMode::BrowserSession
        );
    }

    #[test]
    fn resolve_auth_client_mode_empty_and_whitespace_ignored() {
        assert_eq!(
            resolve_auth_client_mode(&HeaderMap::new(), Some("")),
            AuthClientMode::TokenClient
        );
        assert_eq!(
            resolve_auth_client_mode(&HeaderMap::new(), Some("  ")),
            AuthClientMode::TokenClient
        );
    }

    #[test]
    fn parse_cli_token_request_body_valid_json() {
        let body = Bytes::from(r#"{"client_ua":"nyxid/1.0"}"#);
        let result = parse_cli_token_request_body(body).unwrap().unwrap();
        assert_eq!(result.client_ua, Some("nyxid/1.0".to_string()));
    }

    #[test]
    fn build_cookie_with_same_site_none() {
        let c = build_cookie_with_same_site("tok", "abc", 900, "/api", true, None, "None");
        assert!(c.contains("SameSite=None"));
        assert!(c.contains("; Secure"));
        assert!(c.contains("Path=/api"));
    }

    #[test]
    fn clear_cookie_with_same_site_includes_domain() {
        let c = clear_cookie_with_same_site("tok", "/", true, Some(".example.com"), "None");
        assert!(c.contains("Max-Age=0"));
        assert!(c.contains("SameSite=None"));
        assert!(c.contains("Domain=.example.com"));
    }

    #[test]
    fn extract_ip_x_forwarded_for_takes_priority_over_real_ip() {
        let mut h = HeaderMap::new();
        h.insert("x-forwarded-for", "1.1.1.1".parse().unwrap());
        h.insert("x-real-ip", "2.2.2.2".parse().unwrap());
        assert_eq!(
            extract_ip(
                &h,
                Some("10.10.10.1:443".parse().unwrap()),
                &["10.10.10.1".parse().unwrap(), "5.6.7.8".parse().unwrap()]
            ),
            Some("1.1.1.1".to_string())
        );
    }

    #[test]
    fn extract_user_agent_returns_value() {
        let mut h = HeaderMap::new();
        h.insert(header::USER_AGENT, "nyxid-cli/0.5.0".parse().unwrap());
        assert_eq!(extract_user_agent(&h), Some("nyxid-cli/0.5.0".to_string()));
    }

    #[test]
    fn extract_user_agent_missing_returns_none() {
        assert!(extract_user_agent(&HeaderMap::new()).is_none());
    }

    #[test]
    fn looks_like_browser_request_with_sec_fetch() {
        let mut h = HeaderMap::new();
        h.insert("sec-fetch-site", "same-origin".parse().unwrap());
        assert!(looks_like_browser_request(&h));
    }

    #[test]
    fn looks_like_browser_request_empty_headers() {
        assert!(!looks_like_browser_request(&HeaderMap::new()));
    }

    #[test]
    fn validate_cli_user_agent_rejects_control_chars() {
        assert!(validate_cli_user_agent("ok\x00bad").is_err());
        assert!(validate_cli_user_agent("ok\ttab").is_err());
    }

    #[test]
    fn validate_cli_user_agent_accepts_normal_string() {
        assert!(validate_cli_user_agent("nyxid-cli/0.5.0 (macOS)").is_ok());
    }

    #[test]
    fn referrer_domain_scheme_relative() {
        let mut h = HeaderMap::new();
        h.insert(header::REFERER, "//cdn.example.com/path".parse().unwrap());
        assert_eq!(extract_referrer_domain(&h), Some("cdn.example.com".into()));
    }

    #[test]
    fn referrer_domain_with_userinfo() {
        let mut h = HeaderMap::new();
        h.insert(
            header::REFERER,
            "https://user@host.com/path".parse().unwrap(),
        );
        assert_eq!(extract_referrer_domain(&h), Some("host.com".into()));
    }

    #[test]
    fn email_domain_with_plus_addressing() {
        assert_eq!(
            extract_email_domain("user+tag@gmail.com"),
            Some("gmail.com".into())
        );
    }

    #[test]
    fn email_domain_multiple_at_signs() {
        assert_eq!(
            extract_email_domain("tricky@sign@domain.com"),
            Some("domain.com".into())
        );
    }
}

#[cfg(test)]
mod w8_id_first_party_tests {
    use super::*;
    use crate::{
        crypto::jwt,
        handlers::{cli_pairings, devices, mfa, users},
        models::{
            audit_outbox::{AuditPrincipal, PENDING_COLLECTION_NAME, PendingAuditEvent},
            user::{COLLECTION_NAME as USERS, User, UserType},
        },
        mw::auth::AuthMethod,
        test_utils::{
            connect_transaction_test_database, test_app_state, test_auth_user, test_user,
        },
    };
    use axum::{
        Router,
        body::{Body, to_bytes},
        http::{Request, StatusCode},
        routing::{delete, get, post, put},
    };
    use mongodb::bson::{Document, doc};
    use serde_json::{Value, json};
    use tower::ServiceExt;
    use uuid::Uuid;

    async fn fixture(name: &str) -> (AppState, String) {
        let db = connect_transaction_test_database(name).await;
        role_service::seed_system_roles(&db).await.unwrap();
        let owner = Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_one(test_user(&owner, UserType::Person))
            .await
            .unwrap();
        (test_app_state(db), owner)
    }

    fn bare_routes() -> Router<AppState> {
        Router::new()
            .route("/api/v1/auth/cli-token", post(cli_token))
            .route("/api/v1/auth/mfa/setup", post(mfa::setup))
            .route("/api/v1/auth/mfa/confirm", post(mfa::confirm))
            .route("/api/v1/auth/mfa/disable", post(mfa::disable))
            .route(
                "/api/v1/devices/code/approve",
                post(devices::approve_device_code),
            )
            .route("/api/v1/devices/onboard", post(devices::onboard_device))
            .route(
                "/api/v1/devices/onboard/{id}",
                delete(devices::revoke_onboard_device),
            )
            .route("/api/v1/cli-pairings", post(cli_pairings::create_pairing))
            .route(
                "/api/v1/cli-pairings/claim",
                post(cli_pairings::claim_pairing),
            )
            .route(
                "/api/v1/cli-pairings/{id}/poll",
                get(cli_pairings::poll_pairing),
            )
            .route(
                "/api/v1/cli-pairings/{id}/reserve-action",
                post(cli_pairings::reserve_action),
            )
            .route(
                "/api/v1/cli-pairings/{id}/rewind-action",
                post(cli_pairings::rewind_action),
            )
            .route(
                "/api/v1/cli-pairings/{id}/complete",
                post(cli_pairings::complete_pairing),
            )
            .route(
                "/api/v1/cli-pairings/{id}/cancel",
                post(cli_pairings::cancel_pairing),
            )
            .route(
                "/api/v1/users/me",
                get(users::get_me)
                    .put(users::update_me)
                    .delete(users::delete_me),
            )
            .route(
                "/api/v1/users/me/preferences/services",
                put(users::save_services_view),
            )
            .route(
                "/api/v1/users/me/preferences/service-views",
                put(users::save_service_views),
            )
            .route(
                "/api/v1/users/me/onboarding/complete",
                post(users::complete_onboarding),
            )
    }

    fn serve(router: Router<AppState>, state: AppState) -> Router {
        router
            .layer(axum::Extension(ConnectInfo(
                "127.0.0.1:23151".parse::<SocketAddr>().unwrap(),
            )))
            .with_state(state)
    }

    async fn request(
        app: &Router,
        method: &str,
        path: &str,
        credential: (&str, &str),
        body: Value,
    ) -> (StatusCode, Value) {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method(method)
                    .uri(path)
                    .header(credential.0, credential.1)
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let bytes = to_bytes(response.into_body(), 1_000_000).await.unwrap();
        let body = if bytes.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap()
        };
        (status, body)
    }

    async fn assert_no_authority_or_audit(db: &mongodb::Database) {
        for collection in [
            crate::models::session::COLLECTION_NAME,
            crate::models::refresh_token::COLLECTION_NAME,
            crate::models::mfa_factor::COLLECTION_NAME,
            crate::models::cli_pairing::COLLECTION_NAME,
            crate::models::device_onboard_credential::COLLECTION_NAME,
            crate::models::api_key::COLLECTION_NAME,
            crate::models::node::COLLECTION_NAME,
            crate::models::audit_log::COLLECTION_NAME,
            PENDING_COLLECTION_NAME,
        ] {
            assert_eq!(
                db.collection::<Document>(collection)
                    .count_documents(doc! {})
                    .await
                    .unwrap(),
                0,
                "unexpected mutation in {collection}"
            );
        }
    }

    #[tokio::test]
    async fn w8_id_oauth_client_tokens_are_refused_at_every_first_party_handler() {
        let (state, owner) = fixture("w8_id_oauth_first_party_handlers").await;
        let token = jwt::generate_oauth_access_token(
            &state.jwt_keys,
            &state.config,
            &Uuid::parse_str(&owner).unwrap(),
            "openid profile email proxy",
            None,
            None,
            None,
            None,
            None,
            &Uuid::new_v4().to_string(),
        )
        .unwrap();
        let bearer = format!("Bearer {token}");
        let (_, production) = crate::routes::build_router_with_state(state.clone());
        let full = serve(production, state.clone());
        let bare = serve(bare_routes(), state.clone());
        let id = Uuid::new_v4().to_string();
        // Valid request shapes reach each handler's credential decision. Exercise
        // both direct mounts and production middleware: app tokens used to pass
        // those layers even though API keys were already refused there.
        let cases = [
            ("POST", "/api/v1/auth/cli-token".to_string(), json!({})),
            ("POST", "/api/v1/auth/mfa/setup".to_string(), json!({})),
            (
                "POST",
                "/api/v1/auth/mfa/confirm".to_string(),
                json!({"code":"123456"}),
            ),
            (
                "POST",
                "/api/v1/auth/mfa/disable".to_string(),
                json!({"password":"unused"}),
            ),
            (
                "POST",
                "/api/v1/devices/code/approve".to_string(),
                json!({"user_code":"ABCD-EFGH"}),
            ),
            (
                "POST",
                "/api/v1/devices/onboard".to_string(),
                json!({"label":"unused"}),
            ),
            ("DELETE", format!("/api/v1/devices/onboard/{id}"), json!({})),
            (
                "POST",
                "/api/v1/cli-pairings".to_string(),
                json!({"kind":"api-key-create","prefill":{}}),
            ),
            (
                "POST",
                "/api/v1/cli-pairings/claim".to_string(),
                json!({"code":"ABCD-EFGH"}),
            ),
            ("GET", format!("/api/v1/cli-pairings/{id}/poll"), json!({})),
            (
                "POST",
                format!("/api/v1/cli-pairings/{id}/reserve-action"),
                json!({}),
            ),
            (
                "POST",
                format!("/api/v1/cli-pairings/{id}/rewind-action"),
                json!({}),
            ),
            (
                "POST",
                format!("/api/v1/cli-pairings/{id}/complete"),
                json!({"ack":{}}),
            ),
            (
                "POST",
                format!("/api/v1/cli-pairings/{id}/cancel"),
                json!({}),
            ),
            (
                "PUT",
                "/api/v1/users/me".to_string(),
                json!({"display_name":"must not change"}),
            ),
            ("DELETE", "/api/v1/users/me".to_string(), json!({})),
            (
                "PUT",
                "/api/v1/users/me/preferences/services".to_string(),
                json!({"organization_ids":[],"service_group_ids":[],"search":"must not save","source":"all","state":"all","service_type":"all","show_auto_connected":true}),
            ),
            (
                "PUT",
                "/api/v1/users/me/preferences/service-views".to_string(),
                json!({"views":[],"default_id":null}),
            ),
            (
                "POST",
                "/api/v1/users/me/onboarding/complete".to_string(),
                json!({"key":"ai_services"}),
            ),
        ];
        let before = state
            .db
            .collection::<Document>(USERS)
            .find_one(doc! {"_id":&owner})
            .await
            .unwrap()
            .unwrap();
        for (mount, app) in [("bare", bare), ("production", full)] {
            let (status, control) = request(
                &app,
                "GET",
                "/api/v1/users/me",
                ("authorization", &bearer),
                json!({}),
            )
            .await;
            assert_eq!(
                status,
                StatusCode::OK,
                "the same verified app token can still read its profile through {mount}: {control}"
            );
            for (method, path, body) in &cases {
                let (status, response) =
                    request(&app, method, path, ("authorization", &bearer), body.clone()).await;
                assert_eq!(
                    status,
                    StatusCode::FORBIDDEN,
                    "{mount} {method} {path}: {response}"
                );
                assert_eq!(response["error"], "forbidden");
                assert_eq!(response["error_code"], 1002);
                assert_eq!(
                    response["details"]["reason"], "credential_type_unsupported",
                    "{mount} {method} {path}: {response}"
                );
                assert_eq!(response["details"]["credential_type"], "oauth_client");
                assert_no_authority_or_audit(&state.db).await;
            }
        }
        assert_eq!(
            state
                .db
                .collection::<Document>(USERS)
                .find_one(doc! {"_id":&owner})
                .await
                .unwrap()
                .unwrap(),
            before
        );
    }

    #[tokio::test]
    async fn w8_id_cli_token_accepts_cookie_and_first_party_access_token() {
        let (state, owner) = fixture("w8_id_cli_first_party_success").await;
        let browser =
            token_service::create_session(&state.db, &state.audit_runtime, &owner, None, None)
                .await
                .unwrap();
        let tokens = token_service::create_session_and_issue_tokens(
            &state.db,
            &state.audit_runtime,
            &state.config,
            &state.jwt_keys,
            &owner,
            None,
            None,
        )
        .await
        .unwrap();
        let claims =
            jwt::verify_token(&state.jwt_keys, &state.config, &tokens.access_token).unwrap();
        assert_eq!(claims.sid.as_deref(), Some(tokens.session_id.as_str()));
        assert!(claims.client_id.is_none());
        let cookie = format!("nyx_session={}", browser.session_token);
        let bearer = format!("Bearer {}", tokens.access_token);
        let app = serve(bare_routes(), state.clone());
        for credential in [
            ("cookie", cookie.as_str()),
            ("authorization", bearer.as_str()),
        ] {
            let (status, body) = request(
                &app,
                "POST",
                "/api/v1/auth/cli-token",
                credential,
                json!({"client_ua":"nyxid-cli/w8-id"}),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "first-party exchange must succeed");
            let access = jwt::verify_token(
                &state.jwt_keys,
                &state.config,
                body["access_token"].as_str().unwrap(),
            )
            .unwrap();
            assert!(access.client_id.is_none());
            let event = state.db.collection::<PendingAuditEvent>(PENDING_COLLECTION_NAME).find_one(doc! {"event.action":"session_issued","event.target_id":access.sid.as_deref().unwrap()}).await.unwrap().unwrap();
            assert_eq!(
                event.event.principal,
                AuditPrincipal::Person { id: owner.clone() }
            );
        }
    }

    #[tokio::test]
    async fn w8_id_cli_token_refuses_verified_nonhuman_credentials_before_parsing() {
        let (state, owner) = fixture("w8_id_cli_credential_guard").await;
        for (method, oauth, actor, key, child, unrestricted) in [
            (AuthMethod::AccessToken, true, false, false, false, true),
            (AuthMethod::Delegated, false, true, false, false, true),
            (AuthMethod::Relay, false, false, true, false, true),
            (AuthMethod::ServiceAccount, false, false, false, false, true),
            (AuthMethod::ApiKey, false, false, true, false, true),
            (AuthMethod::ApiKey, false, false, true, false, false),
            (AuthMethod::ApiKey, false, false, true, true, false),
            // Verified identity fields must also prevent a falsely broad method.
            (AuthMethod::AccessToken, false, true, false, false, true),
            (AuthMethod::AccessToken, false, false, true, false, true),
        ] {
            let mut user = test_auth_user(&owner);
            user.auth_method = method.clone();
            user.oauth_client_id = oauth.then(|| Uuid::new_v4().to_string());
            user.acting_client_id = actor.then(|| Uuid::new_v4().to_string());
            user.api_key_id = key.then(|| Uuid::new_v4().to_string());
            user.api_key_credential_id = child.then(|| Uuid::new_v4().to_string());
            user.allow_all_services = unrestricted;
            let response = cli_token(
                State(state.clone()),
                ConnectInfo("127.0.0.1:23151".parse().unwrap()),
                user,
                HeaderMap::new(),
                Bytes::from_static(b"invalid-json"),
            )
            .await;
            assert!(
                matches!(response, Err(AppError::ForbiddenWithGuidance { .. })),
                "{method:?} must fail before body parsing"
            );
        }
        assert_no_authority_or_audit(&state.db).await;
    }

    #[tokio::test]
    async fn w8_id_cli_token_router_layers_refuse_key_headers() {
        let (state, _) = fixture("w8_id_cli_key_layers").await;
        let (_, production) = crate::routes::build_router_with_state(state.clone());
        let app = serve(production, state.clone());
        // Deliberately unregistered keys: the route layer must decide from the
        // credential shape before the extractor can report invalid credentials.
        for credential in [
            ("x-api-key", "nyx_unregistered"),
            ("authorization", "Bearer nyxid_ag_unregistered_child"),
        ] {
            let (status, body) = request(
                &app,
                "POST",
                "/api/v1/auth/cli-token",
                credential,
                json!({}),
            )
            .await;
            assert_eq!(
                status,
                StatusCode::FORBIDDEN,
                "router layer must reject keys: {body}"
            );
            assert_eq!(body["error"], "forbidden");
            assert_eq!(body["error_code"], 1002);
            assert_eq!(
                body["details"]["reason"], "credential_type_unsupported",
                "{body}"
            );
            assert_eq!(body["details"]["credential_type"], "api_key");
            assert_eq!(
                body["message"], "Forbidden: API keys cannot access this endpoint",
                "the shape-based reject_api_key_tokens layer must refuse both identities"
            );
            assert_no_authority_or_audit(&state.db).await;
        }
        assert_no_authority_or_audit(&state.db).await;
    }

    #[tokio::test]
    async fn w8_id_connection_audit_retains_authenticated_key() {
        use crate::models::{
            audit_log::{AuditLog, COLLECTION_NAME as AUDITS},
            downstream_service::{
                COLLECTION_NAME as SERVICES, DownstreamService, test_helpers::dummy_service,
            },
        };
        let (state, owner) = fixture("w8_id_key_connection_audit").await;
        crate::test_utils::init_test_audit_chain_hmac_key();
        let key = Uuid::new_v4().to_string();
        let (prefix, secret, hash) = crate::crypto::token::generate_api_key();
        state
            .db
            .collection::<Document>(crate::models::api_key::COLLECTION_NAME)
            .insert_one(doc! {
                "_id": &key, "user_id": &owner, "name": "w8-id-key",
                "key_prefix": prefix, "key_hash": hash, "scopes": "read write",
                "is_active": true, "created_at": mongodb::bson::DateTime::now(),
            })
            .await
            .unwrap();
        let mut service = dummy_service();
        service.id = Uuid::new_v4().to_string();
        service.service_category = "internal".into();
        service.service_type = "http".into();
        service.requires_user_credential = false;
        state
            .db
            .collection::<DownstreamService>(SERVICES)
            .insert_one(&service)
            .await
            .unwrap();
        let (_, production) = crate::routes::build_router_with_state(state.clone());
        let app = serve(production, state.clone());
        let (status, body) = request(
            &app,
            "POST",
            &format!("/api/v1/connections/{}", service.id),
            ("x-api-key", &secret),
            json!({}),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::OK,
            "keys may connect an internal service: {body}"
        );
        // This endpoint retains its legacy audit until the connection batch.
        // Preserve key attribution on a production route that admits API keys.
        let event = tokio::time::timeout(std::time::Duration::from_secs(5), async {
            loop {
                if let Some(event) = state
                    .db
                    .collection::<AuditLog>(AUDITS)
                    .find_one(doc! {"event_type":"connection_created"})
                    .await
                    .unwrap()
                {
                    break event;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("connection audit must finish");
        assert_eq!(event.api_key_id.as_deref(), Some(key.as_str()));
        assert_eq!(event.user_id.as_deref(), Some(owner.as_str()));
        assert!(!serde_json::to_string(&event).unwrap().contains(&secret));
    }
}

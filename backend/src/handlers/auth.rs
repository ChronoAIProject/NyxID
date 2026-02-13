use std::net::SocketAddr;

use axum::{
    extract::{ConnectInfo, State},
    http::{header, HeaderMap},
    Json,
};
use serde::{Deserialize, Serialize};
use validator::Validate;

use mongodb::bson::doc;

use crate::errors::{AppError, AppResult};
use crate::models::user::{User, COLLECTION_NAME as USERS};
use crate::mw::auth::{AuthUser, ACCESS_TOKEN_COOKIE_NAME, SESSION_COOKIE_NAME};
use crate::services::{audit_service, auth_service, token_service};
use crate::AppState;

// --- Request / Response types ---

#[derive(Debug, Deserialize, Validate)]
pub struct RegisterRequest {
    #[validate(email(message = "Invalid email address"))]
    pub email: String,
    #[validate(length(min = 8, max = 128, message = "Password must be between 8 and 128 characters"))]
    pub password: String,
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
}

#[derive(Debug, Serialize)]
pub struct LoginResponse {
    pub user_id: String,
    pub access_token: String,
    pub expires_in: i64,
}

#[derive(Debug, Serialize)]
pub struct RefreshResponse {
    pub access_token: String,
    pub expires_in: i64,
}

#[derive(Debug, Serialize)]
pub struct LogoutResponse {
    pub message: String,
}

// --- Helper functions ---

/// Extract the client IP from proxy headers, falling back to the TCP peer address.
///
/// Checks (in order): X-Forwarded-For, X-Real-IP, then the peer socket address.
pub(crate) fn extract_ip(headers: &HeaderMap, peer_addr: Option<SocketAddr>) -> Option<String> {
    // 1. X-Forwarded-For (first IP in the chain)
    if let Some(forwarded) = headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.split(',').next().unwrap_or("").trim().to_string())
        .filter(|s| !s.is_empty())
    {
        return Some(forwarded);
    }

    // 2. X-Real-IP
    if let Some(real_ip) = headers
        .get("x-real-ip")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
    {
        return Some(real_ip);
    }

    // 3. TCP peer address
    peer_addr.map(|addr| addr.ip().to_string())
}

/// Extract the User-Agent header.
pub(crate) fn extract_user_agent(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::USER_AGENT)
        .and_then(|v| v.to_str().ok())
        .map(String::from)
}

/// Build a Set-Cookie header value for an HttpOnly, SameSite=Lax cookie.
/// The Secure flag is set based on the deployment environment.
/// When `domain` is provided, includes `Domain=<value>` for cross-subdomain sharing.
pub(crate) fn build_cookie(name: &str, value: &str, max_age_secs: i64, path: &str, secure: bool, domain: Option<&str>) -> String {
    let secure_flag = if secure { "; Secure" } else { "" };
    let domain_attr = domain.map(|d| format!("; Domain={d}")).unwrap_or_default();
    format!(
        "{}={}; HttpOnly; SameSite=Lax; Path={}; Max-Age={}{}{}",
        name, value, path, max_age_secs, secure_flag, domain_attr
    )
}

/// Build a cookie-clearing header value.
/// When `domain` is provided, includes `Domain=<value>` so the browser clears
/// the correct cross-subdomain cookie.
pub(crate) fn clear_cookie(name: &str, path: &str, secure: bool, domain: Option<&str>) -> String {
    let secure_flag = if secure { "; Secure" } else { "" };
    let domain_attr = domain.map(|d| format!("; Domain={d}")).unwrap_or_default();
    format!(
        "{}=; HttpOnly; SameSite=Lax; Path={}; Max-Age=0{}{}",
        name, path, secure_flag, domain_attr
    )
}

// --- Handlers ---

/// POST /api/v1/auth/register
///
/// Create a new user account. Returns the user ID and sends an email
/// verification link (when SMTP is configured).
pub async fn register(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<RegisterRequest>,
) -> AppResult<Json<RegisterResponse>> {
    body.validate()
        .map_err(|e| AppError::ValidationError(e.to_string()))?;

    let result = auth_service::register_user(
        &state.db,
        &body.email,
        &body.password,
        body.display_name.as_deref(),
    )
    .await?;

    audit_service::log_async(
        state.db.clone(),
        Some(result.user_id.clone()),
        "register".to_string(),
        Some(serde_json::json!({ "email": body.email })),
        extract_ip(&headers, Some(peer)),
        extract_user_agent(&headers),
    );

    #[cfg(debug_assertions)]
    tracing::debug!(
        token = %result.email_verification_token,
        "Email verification token (dev only)"
    );

    Ok(Json(RegisterResponse {
        user_id: result.user_id,
        message: "Registration successful. Please verify your email.".to_string(),
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

    // Check MFA requirement
    if user.mfa_enabled {
        match &body.mfa_code {
            Some(code) => {
                // Validate MFA code length to prevent abuse
                if code.len() > 10 {
                    return Err(AppError::AuthenticationFailed(
                        "Invalid MFA code".to_string(),
                    ));
                }

                let encryption_key = crate::crypto::aes::parse_hex_key(&state.config.encryption_key)?;
                let valid = crate::services::mfa_service::verify_totp(
                    &state.db,
                    &encryption_key,
                    &user.id,
                    code,
                )
                .await?;

                if !valid {
                    return Err(AppError::AuthenticationFailed(
                        "Invalid MFA code".to_string(),
                    ));
                }
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
    .await?;

    audit_service::log_async(
        state.db.clone(),
        Some(user.id.clone()),
        "login".to_string(),
        Some(serde_json::json!({ "session_id": tokens.session_id })),
        ip,
        ua,
    );

    let secure = state.config.use_secure_cookies();
    let domain = state.config.cookie_domain();

    let mut response_headers = HeaderMap::new();
    response_headers.insert(
        header::SET_COOKIE,
        build_cookie(
            SESSION_COOKIE_NAME,
            &tokens.session_token,
            30 * 24 * 3600, // 30 days
            "/",
            secure,
            domain,
        )
        .parse()
        .map_err(|_| AppError::Internal("Failed to build cookie header".to_string()))?,
    );
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
        .map_err(|_| AppError::Internal("Failed to build cookie header".to_string()))?,
    );
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
        .map_err(|_| AppError::Internal("Failed to build cookie header".to_string()))?,
    );

    Ok((
        response_headers,
        Json(LoginResponse {
            user_id: user.id.to_string(),
            access_token: tokens.access_token,
            expires_in: tokens.access_expires_in,
        }),
    ))
}

/// POST /api/v1/auth/logout
///
/// Revoke the current session and clear all auth cookies.
pub async fn logout(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    auth_user: AuthUser,
    headers: HeaderMap,
) -> AppResult<(HeaderMap, Json<LogoutResponse>)> {
    if let Some(session_id) = auth_user.session_id {
        token_service::revoke_session(&state.db, &session_id.to_string(), Some(&state.mcp_sessions)).await?;
    }

    audit_service::log_async(
        state.db.clone(),
        Some(auth_user.user_id.to_string()),
        "logout".to_string(),
        None,
        extract_ip(&headers, Some(peer)),
        extract_user_agent(&headers),
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
        clear_cookie("nyx_refresh_token", "/api/v1/auth/refresh", secure, domain)
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

/// POST /api/v1/auth/refresh
///
/// Exchange a refresh token for a new access token.
/// Implements token rotation for security.
pub async fn refresh(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> AppResult<(HeaderMap, Json<RefreshResponse>)> {
    // Extract refresh token from cookie
    let cookie_header = headers
        .get("cookie")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");

    let refresh_token = cookie_header
        .split(';')
        .find_map(|pair| {
            let pair = pair.trim();
            let (key, value) = pair.split_once('=')?;
            if key.trim() == "nyx_refresh_token" {
                Some(value.trim())
            } else {
                None
            }
        })
        .ok_or_else(|| AppError::Unauthorized("No refresh token provided".to_string()))?;

    let tokens = token_service::refresh_tokens(
        &state.db,
        &state.config,
        &state.jwt_keys,
        refresh_token,
        Some(&state.mcp_sessions),
    )
    .await?;

    let secure = state.config.use_secure_cookies();
    let domain = state.config.cookie_domain();

    let mut response_headers = HeaderMap::new();
    response_headers.insert(
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
        .map_err(|_| AppError::Internal("Failed to build cookie header".to_string()))?,
    );
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
        .map_err(|_| AppError::Internal("Failed to build cookie header".to_string()))?,
    );

    Ok((
        response_headers,
        Json(RefreshResponse {
            access_token: tokens.access_token,
            expires_in: tokens.access_expires_in,
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
    Json(body): Json<VerifyEmailRequest>,
) -> AppResult<Json<VerifyEmailResponse>> {
    auth_service::verify_email(&state.db, &body.token).await?;

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

    Ok(Json(ForgotPasswordResponse {
        message: "If that email exists, a password reset link has been sent.".to_string(),
    }))
}

// --- Reset Password ---

#[derive(Debug, Deserialize, Validate)]
pub struct ResetPasswordRequest {
    pub token: String,
    #[validate(length(min = 8, max = 128, message = "Password must be between 8 and 128 characters"))]
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

    Ok(Json(ResetPasswordResponse {
        message: "Password has been reset successfully".to_string(),
    }))
}

// --- Bootstrap Setup ---

#[derive(Debug, Deserialize, Validate)]
pub struct SetupRequest {
    #[validate(email(message = "Invalid email address"))]
    pub email: String,
    #[validate(length(min = 8, max = 128, message = "Password must be between 8 and 128 characters"))]
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
    )
    .await?;

    // Promote to admin and mark email as verified
    let now = chrono::Utc::now();
    state
        .db
        .collection::<User>(USERS)
        .update_one(
            doc! { "_id": &result.user_id },
            doc! { "$set": {
                "is_admin": true,
                "email_verified": true,
                "updated_at": mongodb::bson::DateTime::from_chrono(now),
            }},
        )
        .await?;

    audit_service::log_async(
        state.db.clone(),
        Some(result.user_id.clone()),
        "admin_setup".to_string(),
        Some(serde_json::json!({
            "email": body.email,
            "method": "bootstrap"
        })),
        extract_ip(&headers, Some(peer)),
        extract_user_agent(&headers),
    );

    tracing::info!(user_id = %result.user_id, email = %body.email, "Initial admin created via bootstrap");

    Ok(Json(SetupResponse {
        user_id: result.user_id,
        message: "Admin account created successfully.".to_string(),
    }))
}

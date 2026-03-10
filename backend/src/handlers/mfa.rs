use std::net::SocketAddr;

use axum::{
    Json,
    extract::{ConnectInfo, State},
    http::HeaderMap,
};
use chrono::Utc;
use mongodb::bson::{self, doc};
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::crypto::aes;
use crate::errors::{AppError, AppResult};
use crate::handlers::auth::{
    AuthClientMode, LoginResponse, apply_browser_session_cookies, extract_ip, extract_user_agent,
    resolve_auth_client_mode,
};
use crate::models::mfa_factor::{COLLECTION_NAME as MFA_FACTORS, MfaFactor};
use crate::models::session::{COLLECTION_NAME as SESSIONS, Session};
use crate::models::user::{COLLECTION_NAME as USERS, User};
use crate::mw::auth::AuthUser;
use crate::services::{audit_service, mfa_service, token_service};

// --- Request / Response types ---

#[derive(Debug, Serialize)]
pub struct MfaSetupResponse {
    pub factor_id: String,
    pub secret: String,
    pub qr_code_url: String,
}

#[derive(Debug, Deserialize)]
pub struct MfaConfirmRequest {
    pub code: String,
}

#[derive(Debug, Serialize)]
pub struct MfaConfirmResponse {
    pub message: String,
    pub recovery_codes: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct MfaLoginVerifyRequest {
    pub code: String,
    pub mfa_token: String,
    pub client: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct MfaDisableRequest {
    pub password: String,
}

#[derive(Debug, Serialize)]
pub struct MfaDisableResponse {
    pub message: String,
}

// --- Handlers ---

/// POST /api/v1/auth/mfa/setup
///
/// Begin TOTP enrollment. Returns the secret and QR code URL.
pub async fn setup(
    State(state): State<AppState>,
    auth_user: AuthUser,
) -> AppResult<Json<MfaSetupResponse>> {
    let encryption_key = aes::parse_hex_key(&state.config.encryption_key)?;
    let user_id_str = auth_user.user_id.to_string();

    let user = state
        .db
        .collection::<User>(USERS)
        .find_one(doc! { "_id": &user_id_str })
        .await?
        .ok_or_else(|| AppError::NotFound("User not found".to_string()))?;

    let result =
        mfa_service::setup_totp(&state.db, &encryption_key, &user_id_str, &user.email).await?;

    Ok(Json(MfaSetupResponse {
        factor_id: result.factor_id,
        secret: result.secret,
        qr_code_url: result.qr_code_url,
    }))
}

/// POST /api/v1/auth/mfa/confirm
///
/// Complete TOTP enrollment by verifying a code. Returns recovery codes.
/// Finds the user's pending (unverified) factor automatically.
pub async fn confirm(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<MfaConfirmRequest>,
) -> AppResult<Json<MfaConfirmResponse>> {
    let encryption_key = aes::parse_hex_key(&state.config.encryption_key)?;
    let user_id_str = auth_user.user_id.to_string();

    // Find the pending (unverified, active) TOTP factor for this user
    let factor = state
        .db
        .collection::<MfaFactor>(MFA_FACTORS)
        .find_one(doc! {
            "user_id": &user_id_str,
            "factor_type": "totp",
            "is_verified": false,
            "is_active": true,
        })
        .await?
        .ok_or_else(|| {
            AppError::NotFound("No pending MFA setup found. Start setup first.".to_string())
        })?;

    let recovery_codes = mfa_service::verify_totp_setup(
        &state.db,
        &encryption_key,
        &factor.id,
        &user_id_str,
        &body.code,
    )
    .await?;

    // Enable MFA on user account
    let now = Utc::now();
    state
        .db
        .collection::<User>(USERS)
        .update_one(
            doc! { "_id": &user_id_str },
            doc! { "$set": {
                "mfa_enabled": true,
                "updated_at": bson::DateTime::from_chrono(now),
            }},
        )
        .await?;

    Ok(Json(MfaConfirmResponse {
        message: "MFA enabled successfully. Save your recovery codes.".to_string(),
        recovery_codes,
    }))
}

/// POST /api/v1/auth/mfa/verify
///
/// Verify a TOTP code during login. Validates the MFA session token,
/// verifies the TOTP code, then issues real session tokens.
pub async fn verify(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<MfaLoginVerifyRequest>,
) -> AppResult<(HeaderMap, Json<LoginResponse>)> {
    // Validate the MFA pending session via its token hash
    let token_hash = crate::crypto::token::hash_token(&body.mfa_token);

    let pending_session = state
        .db
        .collection::<Session>(SESSIONS)
        .find_one(doc! {
            "token_hash": &token_hash,
            "user_agent": "mfa_pending",
            "revoked": false,
        })
        .await?
        .ok_or_else(|| AppError::Unauthorized("Invalid or expired MFA session".to_string()))?;

    if pending_session.expires_at < Utc::now() {
        return Err(AppError::TokenExpired);
    }

    let user_id = &pending_session.user_id;

    // Verify TOTP code
    let encryption_key = aes::parse_hex_key(&state.config.encryption_key)?;
    let valid = mfa_service::verify_totp(&state.db, &encryption_key, user_id, &body.code).await?;

    if !valid {
        return Err(AppError::AuthenticationFailed(
            "Invalid MFA code".to_string(),
        ));
    }

    // Revoke the pending MFA session
    state
        .db
        .collection::<Session>(SESSIONS)
        .update_one(
            doc! { "_id": &pending_session.id },
            doc! { "$set": { "revoked": true } },
        )
        .await?;

    // Issue real session tokens
    let ip = extract_ip(&headers, Some(peer));
    let ua = extract_user_agent(&headers);
    let client_mode = resolve_auth_client_mode(&headers, body.client.as_deref());
    let secure = state.config.use_secure_cookies();
    let domain = state.config.cookie_domain();
    let mut response_headers = HeaderMap::new();

    match client_mode {
        AuthClientMode::BrowserSession => {
            let session =
                token_service::create_session(&state.db, user_id, ip.as_deref(), ua.as_deref())
                    .await?;

            audit_service::log_async(
                state.db.clone(),
                Some(user_id.to_string()),
                "login_mfa".to_string(),
                Some(serde_json::json!({ "session_id": session.session_id })),
                ip,
                ua,
            );

            apply_browser_session_cookies(
                &mut response_headers,
                &session.session_token,
                secure,
                domain,
            )?;

            Ok((
                response_headers,
                Json(LoginResponse {
                    user_id: user_id.to_string(),
                    access_token: None,
                    expires_in: None,
                    refresh_token: None,
                }),
            ))
        }
        AuthClientMode::TokenClient => {
            let tokens = token_service::create_session_and_issue_tokens(
                &state.db,
                &state.config,
                &state.jwt_keys,
                user_id,
                ip.as_deref(),
                ua.as_deref(),
            )
            .await?;

            audit_service::log_async(
                state.db.clone(),
                Some(user_id.to_string()),
                "login_mfa".to_string(),
                Some(serde_json::json!({ "session_id": tokens.session_id })),
                ip,
                ua,
            );

            Ok((
                response_headers,
                Json(LoginResponse {
                    user_id: user_id.to_string(),
                    access_token: Some(tokens.access_token),
                    expires_in: Some(tokens.access_expires_in),
                    refresh_token: Some(tokens.refresh_token),
                }),
            ))
        }
    }
}

/// POST /api/v1/auth/mfa/disable
///
/// Disable MFA on the account. Requires password confirmation.
pub async fn disable(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<MfaDisableRequest>,
) -> AppResult<Json<MfaDisableResponse>> {
    let user_id_str = auth_user.user_id.to_string();

    // Verify password
    let user = state
        .db
        .collection::<User>(USERS)
        .find_one(doc! { "_id": &user_id_str })
        .await?
        .ok_or_else(|| AppError::NotFound("User not found".to_string()))?;

    let password_hash = user
        .password_hash
        .as_ref()
        .ok_or_else(|| AppError::BadRequest("No password set on account".to_string()))?;

    let password_valid = crate::crypto::password::verify_password(&body.password, password_hash)?;
    if !password_valid {
        return Err(AppError::AuthenticationFailed(
            "Invalid password".to_string(),
        ));
    }

    // Deactivate all MFA factors
    let now = Utc::now();
    state
        .db
        .collection::<MfaFactor>(MFA_FACTORS)
        .update_many(
            doc! { "user_id": &user_id_str, "is_active": true },
            doc! { "$set": {
                "is_active": false,
                "updated_at": bson::DateTime::from_chrono(now),
            }},
        )
        .await?;

    // Disable MFA on user
    state
        .db
        .collection::<User>(USERS)
        .update_one(
            doc! { "_id": &user_id_str },
            doc! { "$set": {
                "mfa_enabled": false,
                "updated_at": bson::DateTime::from_chrono(now),
            }},
        )
        .await?;

    Ok(Json(MfaDisableResponse {
        message: "MFA has been disabled.".to_string(),
    }))
}

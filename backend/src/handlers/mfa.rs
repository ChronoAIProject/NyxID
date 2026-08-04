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
use crate::telemetry::{TelemetryContext, TelemetryEvent, emit_event};

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
    tele: TelemetryContext,
) -> AppResult<Json<MfaSetupResponse>> {
    let user_id_str = auth_user.user_id.to_string();

    let user = state
        .db
        .collection::<User>(USERS)
        .find_one(doc! { "_id": &user_id_str })
        .await?
        .ok_or_else(|| AppError::NotFound("User not found".to_string()))?;

    let result =
        mfa_service::setup_totp(&state.db, &state.encryption_keys, &user_id_str, &user.email)
            .await?;

    emit_event(
        state.telemetry.as_deref(),
        &user_id_str,
        auth_user.api_key_id.as_deref(),
        &tele,
        TelemetryEvent::MfaEnrollmentStarted {
            factor_type: "totp".to_string(),
        },
    );

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
    tele: TelemetryContext,
    Json(body): Json<MfaConfirmRequest>,
) -> AppResult<Json<MfaConfirmResponse>> {
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
        &state.encryption_keys,
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

    emit_event(
        state.telemetry.as_deref(),
        &user_id_str,
        auth_user.api_key_id.as_deref(),
        &tele,
        TelemetryEvent::MfaEnrollmentCompleted {
            factor_type: "totp".to_string(),
        },
    );

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

    // Pre-auth path: build telemetry context from headers so both the
    // success and failure branches can attribute the event to the user
    // held by the MFA pending session.
    let tele_mfa = TelemetryContext::from_headers(
        headers.get("x-nyxid-client").and_then(|v| v.to_str().ok()),
        headers
            .get("x-nyxid-client-version")
            .and_then(|v| v.to_str().ok()),
    );

    // Verify TOTP code
    let valid =
        mfa_service::verify_totp(&state.db, &state.encryption_keys, user_id, &body.code).await?;

    if !valid {
        emit_event(
            state.telemetry.as_deref(),
            user_id,
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

    emit_event(
        state.telemetry.as_deref(),
        user_id,
        None,
        &tele_mfa,
        TelemetryEvent::MfaChallengeSucceeded {
            factor_type: "totp".to_string(),
        },
    );

    // AuthLoggedIn is emitted AFTER session/token creation succeeds
    // (see both arms of `match client_mode` below). Emitting here would
    // record a false-positive login if the DB/JWT/cookie step that
    // follows fails, matching the bug Part 1 already fixed on the
    // non-MFA password path.

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
                None,
                None,
            );

            apply_browser_session_cookies(
                &mut response_headers,
                &session.session_token,
                secure,
                domain,
            )?;

            // Session is now materialized; record the login. `method`
            // matches Part 1's non-MFA password-login path so a single
            // password-login funnel covers MFA and non-MFA users.
            emit_event(
                state.telemetry.as_deref(),
                user_id,
                None,
                &tele_mfa,
                TelemetryEvent::AuthLoggedIn {
                    method: "password".to_string(),
                    mfa_required: true,
                },
            );

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
                None,
                None,
            );

            // Tokens are materialized; record the login under the same
            // `method="password"` label as non-MFA logins so the
            // password-login funnel is consistent for both MFA and
            // non-MFA users.
            emit_event(
                state.telemetry.as_deref(),
                user_id,
                None,
                &tele_mfa,
                TelemetryEvent::AuthLoggedIn {
                    method: "password".to_string(),
                    mfa_required: true,
                },
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::user::{COLLECTION_NAME as USERS, User, UserType};
    use crate::test_utils::{connect_test_database, test_app_state, test_auth_user, test_user};
    use axum::extract::State;

    fn tele() -> TelemetryContext {
        TelemetryContext::default()
    }

    #[tokio::test]
    async fn setup_mfa_returns_secret_and_qr() {
        let Some(db) = connect_test_database("h_mfa_setup").await else {
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_one(test_user(&user_id, UserType::Person))
            .await
            .unwrap();
        let state = test_app_state(db);
        let auth = test_auth_user(&user_id);

        let Json(resp) = setup(State(state), auth, tele()).await.unwrap();

        assert!(!resp.factor_id.is_empty());
        assert!(!resp.secret.is_empty());
        assert!(resp.qr_code_url.starts_with("otpauth://totp/"));
    }

    #[tokio::test]
    async fn setup_mfa_user_not_found() {
        let Some(db) = connect_test_database("h_mfa_setup_no_user").await else {
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();
        let state = test_app_state(db);
        let auth = test_auth_user(&user_id);

        let err = setup(State(state), auth, tele()).await;
        assert!(err.is_err());
    }

    #[tokio::test]
    async fn confirm_mfa_no_pending_factor() {
        let Some(db) = connect_test_database("h_mfa_confirm_no_factor").await else {
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_one(test_user(&user_id, UserType::Person))
            .await
            .unwrap();
        let state = test_app_state(db);
        let auth = test_auth_user(&user_id);

        let err = confirm(
            State(state),
            auth,
            tele(),
            Json(MfaConfirmRequest {
                code: "123456".to_string(),
            }),
        )
        .await;

        assert!(err.is_err());
    }

    #[tokio::test]
    async fn confirm_mfa_wrong_code() {
        let Some(db) = connect_test_database("h_mfa_confirm_bad_code").await else {
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_one(test_user(&user_id, UserType::Person))
            .await
            .unwrap();
        let state = test_app_state(db);
        let auth = test_auth_user(&user_id);

        let Json(_) = setup(State(state.clone()), auth.clone(), tele())
            .await
            .unwrap();

        let err = confirm(
            State(state),
            auth,
            tele(),
            Json(MfaConfirmRequest {
                code: "000000".to_string(),
            }),
        )
        .await;

        assert!(err.is_err());
    }

    #[tokio::test]
    async fn disable_mfa_wrong_password() {
        let Some(db) = connect_test_database("h_mfa_disable_bad_pw").await else {
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();
        let pw_hash = crate::crypto::password::hash_password("correct_password").unwrap();
        let mut user = test_user(&user_id, UserType::Person);
        user.password_hash = Some(pw_hash);
        db.collection::<User>(USERS).insert_one(user).await.unwrap();
        let state = test_app_state(db);
        let auth = test_auth_user(&user_id);

        let err = disable(
            State(state),
            auth,
            Json(MfaDisableRequest {
                password: "wrong_password".to_string(),
            }),
        )
        .await;

        assert!(err.is_err());
    }

    #[tokio::test]
    async fn disable_mfa_no_password_set() {
        let Some(db) = connect_test_database("h_mfa_disable_no_pw").await else {
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_one(test_user(&user_id, UserType::Person))
            .await
            .unwrap();
        let state = test_app_state(db);
        let auth = test_auth_user(&user_id);

        let err = disable(
            State(state),
            auth,
            Json(MfaDisableRequest {
                password: "any".to_string(),
            }),
        )
        .await;

        assert!(err.is_err());
    }
}

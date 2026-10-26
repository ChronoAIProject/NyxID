use std::net::SocketAddr;

use axum::{
    Json,
    extract::{ConnectInfo, State},
    http::HeaderMap,
};
use mongodb::bson::doc;
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::handlers::auth::{
    AuthClientMode, LoginResponse, apply_browser_session_cookies, build_cookie_with_same_site,
    clear_cookie_with_same_site, extract_ip, extract_user_agent, resolve_auth_client_mode,
};
use crate::models::mfa_factor::{COLLECTION_NAME as MFA_FACTORS, MfaFactor};
use crate::models::user::{COLLECTION_NAME as USERS, User};
use crate::mw::auth::AuthUser;
use crate::services::{audit_service, mfa_service, token_service};
use crate::telemetry::{TelemetryContext, TelemetryEvent, emit_event};

// --- Request / Response types ---

#[cfg(test)]
mod enforcement_tests;

pub(crate) const MFA_CHALLENGE_COOKIE: &str = "nyx_mfa_challenge";
const MFA_CHALLENGE_COOKIE_PATH: &str = "/api/v1/auth/mfa";

pub(crate) fn mfa_challenge_cookie(value: &str, secure: bool, domain: Option<&str>) -> String {
    build_cookie_with_same_site(
        MFA_CHALLENGE_COOKIE,
        value,
        300,
        MFA_CHALLENGE_COOKIE_PATH,
        secure,
        domain,
        "Lax",
    )
}

pub(crate) fn clear_mfa_challenge_cookie(secure: bool, domain: Option<&str>) -> String {
    clear_cookie_with_same_site(
        MFA_CHALLENGE_COOKIE,
        MFA_CHALLENGE_COOKIE_PATH,
        secure,
        domain,
        "Lax",
    )
}

fn challenge_cookie(headers: &HeaderMap) -> Option<String> {
    super::social_auth::extract_cookie_value(headers, MFA_CHALLENGE_COOKIE)
}

#[derive(Serialize)]
pub struct MfaSetupResponse {
    pub factor_id: String,
    pub secret: Zeroizing<String>,
    pub qr_code_url: Zeroizing<String>,
}

#[derive(Deserialize)]
pub struct MfaConfirmRequest {
    pub code: Zeroizing<String>,
}

#[derive(Serialize)]
pub struct MfaConfirmResponse {
    pub message: String,
    pub recovery_codes: Zeroizing<Vec<String>>,
}

#[derive(Deserialize)]
pub struct MfaLoginVerifyRequest {
    pub code: Zeroizing<String>,
    pub mfa_token: Option<Zeroizing<String>>,
    pub client: Option<String>,
}

#[derive(Deserialize)]
pub struct MfaDisableRequest {
    pub code: Zeroizing<String>,
    #[serde(default)]
    pub password: Option<Zeroizing<String>>,
}

#[derive(Debug, Serialize)]
pub struct MfaDisableResponse {
    pub message: String,
}

// --- Handlers ---

/// POST /api/v1/auth/mfa/step-up. The requesting device keeps its session.
pub async fn step_up(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<MfaConfirmRequest>,
) -> AppResult<Json<serde_json::Value>> {
    super::login_client_context::require_first_party_human(&auth_user)?;
    let session_id = auth_user.session_id.ok_or_else(|| {
        AppError::Forbidden("An identified first-party session is required".into())
    })?;
    let user_id = auth_user.user_id.to_string();
    let factor = mfa_service::step_up(
        &state.db,
        &state.encryption_keys,
        &user_id,
        &session_id.to_string(),
        &body.code,
        &audit_service::AuditActor::from_auth_user(&auth_user),
    )
    .await?;
    mfa_service::notify_recovery_use(&state, &user_id, factor);
    Ok(Json(serde_json::json!({"message": "Session verified"})))
}

/// POST /api/v1/auth/mfa/setup
///
/// Begin TOTP enrollment. Returns the secret and QR code URL.
pub async fn setup(
    State(state): State<AppState>,
    auth_user: AuthUser,
    tele: TelemetryContext,
) -> AppResult<Json<MfaSetupResponse>> {
    super::login_client_context::require_first_party_human(&auth_user)?;
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
    )
    .await;

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
    super::login_client_context::require_first_party_human(&auth_user)?;
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

    let current_session_id = auth_user.session_id.map(|id| id.to_string());
    if current_session_id.is_none() {
        return Err(AppError::Conflict(
            "MFA confirmation requires an identified session; sign in again".into(),
        ));
    }
    let recovery_codes = mfa_service::verify_totp_setup(
        &state.db,
        &state.encryption_keys,
        &factor.id,
        &user_id_str,
        &body.code,
        current_session_id.as_deref(),
        Some(&crate::services::audit_service::AuditActor::from_auth_user(
            &auth_user,
        )),
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
    )
    .await;

    Ok(Json(MfaConfirmResponse {
        message: "MFA enabled. Other devices were signed out. Save your recovery codes."
            .to_string(),
        recovery_codes,
    }))
}

/// POST /api/v1/auth/mfa/verify
///
/// Complete login with a TOTP or single-use recovery code bound to a
/// first-factor challenge, then issue an authenticated session.
pub async fn verify(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<MfaLoginVerifyRequest>,
) -> AppResult<(HeaderMap, Json<LoginResponse>)> {
    let cookie_token = challenge_cookie(&headers);
    let mfa_token = body
        .mfa_token
        .as_deref()
        .map(String::as_str)
        .or(cookie_token.as_deref())
        .ok_or_else(|| AppError::AuthenticationFailed("MFA challenge is missing".into()))?;
    // Validate the MFA pending session via its token hash
    let token_hash = crate::crypto::token::hash_token(mfa_token);

    let challenge = mfa_service::reserve_pending_challenge(&state.db, &token_hash).await?;
    let user_id = &challenge.session.user_id;
    let challenge_method = challenge
        .session
        .mfa_method
        .clone()
        .unwrap_or_else(|| "password".to_string());

    // Challenge-phase product analytics is always denied, including failures.
    let tele_mfa = TelemetryContext::pre_auth(&headers);

    // Verify TOTP code
    let factor = mfa_service::begin_login_completion(
        &state.db,
        &state.encryption_keys,
        user_id,
        &body.code,
        Some(&challenge),
    )
    .await?;

    if factor.is_none() {
        emit_event(
            state.telemetry.as_deref(),
            user_id,
            None,
            &tele_mfa,
            TelemetryEvent::MfaChallengeFailed {
                factor_type: "totp".to_string(),
                reason: "wrong_code".to_string(),
            },
        )
        .await;
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
    )
    .await;

    // AuthLoggedIn is emitted AFTER session/token creation succeeds
    // (see both arms of `match client_mode` below). Emitting here would
    // record a false-positive login if the DB/JWT/cookie step that
    // follows fails, matching the bug Part 1 already fixed on the
    // non-MFA password path.

    let mut completion = factor.expect("verified factor");

    // Issue real session tokens
    let ip = extract_ip(&headers, Some(peer));
    let ua = extract_user_agent(&headers);
    let client_mode = resolve_auth_client_mode(&headers, body.client.as_deref());
    let secure = state.config.use_secure_cookies();
    let domain = state.config.cookie_domain();
    let mut response_headers = HeaderMap::new();

    match client_mode {
        AuthClientMode::BrowserSession => {
            let session_result = token_service::create_session_with_transaction_and_assurance(
                &state.db,
                user_id,
                ip.as_deref(),
                ua.as_deref(),
                Some(&mut completion.transaction),
                true,
            )
            .await;
            let session = match session_result {
                Ok(session) => session,
                Err(error) => {
                    completion.abort().await?;
                    return Err(error);
                }
            };
            if let Err(error) = apply_browser_session_cookies(
                &mut response_headers,
                &session.session_token,
                secure,
                domain,
            ) {
                completion.abort().await?;
                return Err(error);
            }
            response_headers.append(
                axum::http::header::SET_COOKIE,
                clear_mfa_challenge_cookie(secure, domain)
                    .parse()
                    .map_err(|_| AppError::Internal("Cookie error".into()))?,
            );
            let factor = completion.commit().await?;
            mfa_service::notify_recovery_use(&state, user_id, factor);

            audit_service::log_async(
                state.db.clone(),
                Some(user_id.to_string()),
                "login_mfa".to_string(),
                Some(
                    serde_json::json!({ "session_id": session.session_id, "method": challenge_method, "mfa_required": true }),
                ),
                ip,
                ua,
                None,
                None,
            );

            // Session is now materialized; record the login. `method`
            // matches Part 1's non-MFA password-login path so a single
            // password-login funnel covers MFA and non-MFA users.
            emit_event(
                state.telemetry.as_deref(),
                user_id,
                None,
                &tele_mfa,
                TelemetryEvent::AuthLoggedIn {
                    method: challenge_method.clone(),
                    mfa_required: true,
                },
            )
            .await;

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
            let prepared_result = token_service::prepare_session_tokens_with_assurance(
                &state.db,
                &state.config,
                &state.jwt_keys,
                user_id,
                ip.as_deref(),
                ua.as_deref(),
                true,
            )
            .await;
            let prepared = match prepared_result {
                Ok(prepared) => prepared,
                Err(error) => {
                    completion.abort().await?;
                    return Err(error);
                }
            };

            if let Err(error) = token_service::insert_prepared_session(
                &state.db,
                &prepared,
                &mut completion.transaction,
            )
            .await
            {
                completion.abort().await?;
                return Err(error);
            }
            let factor = completion.commit().await?;
            let tokens = prepared.tokens;
            mfa_service::notify_recovery_use(&state, user_id, factor);
            response_headers.append(
                axum::http::header::SET_COOKIE,
                clear_mfa_challenge_cookie(secure, domain)
                    .parse()
                    .map_err(|_| AppError::Internal("Cookie error".into()))?,
            );
            audit_service::log_async(
                state.db.clone(),
                Some(user_id.to_string()),
                "login_mfa".to_string(),
                Some(
                    serde_json::json!({ "session_id": tokens.session_id, "method": challenge_method, "mfa_required": true }),
                ),
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
                    method: challenge_method,
                    mfa_required: true,
                },
            )
            .await;

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

/// The HTTP boundary attaches cookie cleanup to terminal challenge errors.
/// A wrong code retains a live challenge, including its existing attempt budget.
pub async fn verify_http(
    State(state): State<AppState>,
    peer: ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    Json(body): Json<MfaLoginVerifyRequest>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    let cookie_mode = body.mfa_token.is_none();
    let challenge_hash = body
        .mfa_token
        .as_deref()
        .map(String::as_str)
        .map(crate::crypto::token::hash_token)
        .or_else(|| {
            challenge_cookie(&headers).map(|token| crate::crypto::token::hash_token(&token))
        });
    let result = verify(State(state.clone()), peer, headers, Json(body)).await;
    let clear = if result.is_err() {
        if let Some(hash) = challenge_hash {
            !mfa_service::pending_challenge_is_live(&state.db, &hash)
                .await
                .unwrap_or(true)
        } else {
            // Browsers remove the cookie at its five-minute expiry. Treat
            // absence in cookie mode as terminal so the UI can restart login.
            cookie_mode
        }
    } else {
        false
    };
    let mut response = match result {
        Err(error) if clear => {
            let mut body = error.response_body();
            body.details = Some(serde_json::json!({"reason": "mfa_challenge_expired"}));
            (axum::http::StatusCode::UNAUTHORIZED, Json(body)).into_response()
        }
        other => other.into_response(),
    };
    if clear
        && cookie_mode
        && let Ok(value) = clear_mfa_challenge_cookie(
            state.config.use_secure_cookies(),
            state.config.cookie_domain(),
        )
        .parse()
    {
        response
            .headers_mut()
            .append(axum::http::header::SET_COOKIE, value);
    }
    response
}

/// POST /api/v1/auth/mfa/disable
///
/// Disable MFA with a current factor and, when configured, the password.
pub async fn disable(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<MfaDisableRequest>,
) -> AppResult<Json<MfaDisableResponse>> {
    super::login_client_context::require_first_party_human(&auth_user)?;
    let user_id_str = auth_user.user_id.to_string();
    mfa_service::disable_totp_with_code(
        &state.db,
        &state.encryption_keys,
        &user_id_str,
        &body.code,
        body.password.as_deref().map(String::as_str),
        Some(&crate::services::audit_service::AuditActor::from_auth_user(
            &auth_user,
        )),
    )
    .await?;
    mfa_service::notify_mfa_disabled(&state, &user_id_str);

    Ok(Json(MfaDisableResponse {
        message: "MFA has been disabled.".to_string(),
    }))
}

/// Abandon a browser MFA challenge. The raw token is read only from the
/// HttpOnly cookie and is never returned, logged, or placed in a URL.
pub async fn cancel(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> AppResult<(HeaderMap, Json<serde_json::Value>)> {
    if let Some(token) = challenge_cookie(&headers) {
        let token_hash = crate::crypto::token::hash_token(&token);
        mfa_service::cancel_pending_challenge(&state.db, &token_hash).await?;
    }
    let mut response = HeaderMap::new();
    response.append(
        axum::http::header::SET_COOKIE,
        clear_mfa_challenge_cookie(
            state.config.use_secure_cookies(),
            state.config.cookie_domain(),
        )
        .parse()
        .map_err(|_| AppError::Internal("Cookie error".into()))?,
    );
    Ok((response, Json(serde_json::json!({"cancelled": true}))))
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
    async fn recovery_redemption_over_http_has_exactly_one_winner_across_app_states() {
        use axum::{
            Router,
            body::{Body, to_bytes},
            http::{Request, StatusCode},
            routing::post,
        };
        use tower::ServiceExt;
        let db = crate::test_utils::connect_transaction_test_database("mfa_http_single_use").await;
        let independent =
            mongodb::Client::with_uri_str(std::env::var("NYXID_TEST_DATABASE_URL").unwrap())
                .await
                .unwrap()
                .database(db.name());
        let id = uuid::Uuid::new_v4().to_string();
        let mut user = test_user(&id, UserType::Person);
        user.mfa_enabled = true;
        db.collection::<User>(USERS).insert_one(user).await.unwrap();
        let a = test_app_state(db.clone());
        let b = test_app_state(independent);
        // Same shape as issued recovery codes; never a fixed credential.
        let recovery = crate::crypto::token::generate_random_token()[..32].to_string();
        let factor_id = uuid::Uuid::new_v4().to_string();
        let secret_encrypted = crate::services::record_field_service::encrypt(
            &a.encryption_keys,
            MFA_FACTORS,
            &factor_id,
            &id,
            "secret_encrypted",
            b"JBSWY3DPEHPK3PXP",
        )
        .await
        .unwrap();
        db.collection::<MfaFactor>(MFA_FACTORS)
            .insert_one(MfaFactor {
                id: factor_id,
                user_id: id.clone(),
                factor_type: "totp".into(),
                secret_encrypted: Some(secret_encrypted),
                secret_context_version: a.encryption_keys.contextual_record_writes().then_some(3),
                recovery_codes: Some(serde_json::json!([crate::crypto::password::hash_password(
                    &recovery
                )
                .unwrap()])),
                last_totp_counter: None,
                is_verified: true,
                is_active: true,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            })
            .await
            .unwrap();
        // Independent, valid first-factor challenges must still contend on the
        // one exact recovery hash. Neither challenge grants account authority.
        let tokens = ["first-challenge", "second-challenge"];
        for token in tokens {
            token_service::create_mfa_pending_session_with_method(
                &db,
                &id,
                &crate::crypto::token::hash_token(token),
                Some("password"),
            )
            .await
            .unwrap();
        }
        let request = |token: &str| {
            Request::builder()
                .method("POST")
                .uri("/verify")
                .header("content-type", "application/json")
                .extension(ConnectInfo(
                    "127.0.0.1:43210".parse::<SocketAddr>().unwrap(),
                ))
                .body(Body::from(
                    serde_json::json!({"mfa_token":token,"code":recovery.as_str(),"client":"cli"})
                        .to_string(),
                ))
                .unwrap()
        };
        let (first, second) = tokio::join!(
            Router::new()
                .route("/verify", post(verify))
                .with_state(a)
                .oneshot(request(tokens[0])),
            Router::new()
                .route("/verify", post(verify))
                .with_state(b)
                .oneshot(request(tokens[1])),
        );
        let responses = [first.unwrap(), second.unwrap()];
        assert_eq!(
            responses
                .iter()
                .filter(|response| response.status() == StatusCode::OK)
                .count(),
            1
        );
        for response in responses {
            let status = response.status();
            let body: serde_json::Value =
                serde_json::from_slice(&to_bytes(response.into_body(), 65536).await.unwrap())
                    .unwrap();
            if status == StatusCode::OK {
                assert!(body["access_token"].is_string());
                assert_eq!(body["user_id"], id);
            } else {
                assert!(matches!(
                    status,
                    StatusCode::UNAUTHORIZED | StatusCode::CONFLICT
                ));
                assert!(body.get("access_token").is_none());
            }
        }
        let factor = db
            .collection::<MfaFactor>(MFA_FACTORS)
            .find_one(doc! {"user_id":&id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(factor.recovery_codes, Some(serde_json::json!([])));
        assert_eq!(
            db.collection::<bson::Document>(crate::models::session::COLLECTION_NAME)
                .count_documents(doc! {"user_id":&id,"revoked":false})
                .await
                .unwrap(),
            1
        );
        let audits: Vec<bson::Document> = {
            use futures::TryStreamExt;
            db.collection::<bson::Document>("audit_log")
                .find(doc! {"user_id":&id,"event_type":"mfa_recovery_used"})
                .await
                .unwrap()
                .try_collect()
                .await
                .unwrap()
        };
        assert_eq!(audits.len(), 1);
        assert!(!format!("{audits:?}").contains(recovery.as_str()));
        db.drop().await.unwrap();
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
                code: "123456".to_string().into(),
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
                code: "000000".to_string().into(),
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
                code: "000000".to_string().into(),
                password: Some("wrong_password".to_string().into()),
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
                code: "000000".to_string().into(),
                password: Some("any".to_string().into()),
            }),
        )
        .await;

        assert!(err.is_err());
    }
    #[tokio::test]
    async fn account_factor_budget_spans_inline_and_new_challenges_across_http_replicas() {
        use crate::crypto::password;
        use axum::{Router, routing::post};
        use std::time::Duration;
        crate::test_utils::ensure_rate_window_headroom(
            Duration::from_secs(300),
            Duration::from_secs(30),
        )
        .await;
        let db = crate::test_utils::connect_transaction_test_database("mfa_account_budget").await;
        let independent =
            mongodb::Client::with_uri_str(std::env::var("NYXID_TEST_DATABASE_URL").unwrap())
                .await
                .unwrap()
                .database(db.name());
        let id = uuid::Uuid::new_v4().to_string();
        let mut actor = test_user(&id, UserType::Person);
        actor.mfa_enabled = true;
        // Runtime test credential; never a fixed password.
        let budget_password = crate::crypto::token::generate_random_token();
        actor.password_hash = Some(password::hash_password(&budget_password).unwrap());
        let email = actor.email.clone();
        db.collection::<User>(USERS)
            .insert_one(actor)
            .await
            .unwrap();
        let state = test_app_state(db.clone());
        let recovery = "00112233445566778899001122334455";
        let hash = password::hash_password(recovery).unwrap();
        db.collection::<MfaFactor>(MFA_FACTORS)
            .insert_one(MfaFactor {
                id: uuid::Uuid::new_v4().to_string(),
                user_id: id.clone(),
                factor_type: "totp".into(),
                secret_encrypted: Some(
                    state
                        .encryption_keys
                        .encrypt(b"JBSWY3DPEHPK3PXP")
                        .await
                        .unwrap(),
                ),
                secret_context_version: None,
                recovery_codes: Some(serde_json::json!([hash])),
                last_totp_counter: None,
                is_verified: true,
                is_active: true,
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            })
            .await
            .unwrap();
        let mut bases = Vec::new();
        let mut servers = Vec::new();
        for database in [db.clone(), independent] {
            let app = Router::new()
                .route("/api/v1/auth/login", post(crate::handlers::auth::login))
                .route("/api/v1/auth/mfa/verify", post(verify))
                .with_state(test_app_state(database));
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            bases.push(format!("http://{}", listener.local_addr().unwrap()));
            servers.push(tokio::spawn(async move {
                axum::serve(
                    listener,
                    app.into_make_service_with_connect_info::<SocketAddr>(),
                )
                .await
                .unwrap();
            }));
        }
        let client = reqwest::Client::new();
        for attempt in 0..10 {
            let base = &bases[attempt % 2];
            // Eight-character nonmatching recovery avoids a probabilistic TOTP match.
            let mut body = serde_json::json!({"email":email,"password":budget_password.as_str(),"client":"cli"});
            if attempt % 2 == 0 {
                body["mfa_code"] = serde_json::json!("wrongKEY");
            }
            let login = client
                .post(format!("{base}/api/v1/auth/login"))
                .json(&body)
                .send()
                .await
                .unwrap();
            if attempt % 2 == 0 {
                assert_eq!(login.status(), reqwest::StatusCode::UNAUTHORIZED);
            } else {
                assert_eq!(login.status(), reqwest::StatusCode::FORBIDDEN);
                let challenge: serde_json::Value = login.json().await.unwrap();
                let response = client.post(format!("{base}/api/v1/auth/mfa/verify"))
                    .json(&serde_json::json!({"mfa_token":challenge["session_token"],"code":"wrongKEY","client":"cli"}))
                    .send().await.unwrap();
                assert_eq!(response.status(), reqwest::StatusCode::UNAUTHORIZED);
            }
        }
        for base in &bases {
            let response = client.post(format!("{base}/api/v1/auth/login"))
                .json(&serde_json::json!({"email":email,"password":budget_password.as_str(),"mfa_code":recovery,"client":"cli"}))
                .send().await.unwrap();
            assert_eq!(response.status(), reqwest::StatusCode::TOO_MANY_REQUESTS);
            let challenge = client
                .post(format!("{base}/api/v1/auth/login"))
                .json(
                    &serde_json::json!({"email":email,"password":budget_password.as_str(),"client":"cli"}),
                )
                .send()
                .await
                .unwrap()
                .json::<serde_json::Value>()
                .await
                .unwrap();
            let response = client.post(format!("{base}/api/v1/auth/mfa/verify"))
                .json(&serde_json::json!({"mfa_token":challenge["session_token"],"code":recovery,"client":"cli"}))
                .send().await.unwrap();
            assert_eq!(response.status(), reqwest::StatusCode::TOO_MANY_REQUESTS);
        }
        assert_eq!(
            db.collection::<bson::Document>("sessions")
                .count_documents(doc! {"user_id":&id})
                .await
                .unwrap(),
            0
        );
        let factor = db
            .collection::<MfaFactor>(MFA_FACTORS)
            .find_one(doc! {"user_id":&id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(factor.recovery_codes, Some(serde_json::json!([hash])));
        assert_eq!(
            db.collection::<bson::Document>(crate::models::session::MFA_CHALLENGES)
                .count_documents(doc! {"user_id":&id,"revoked":true})
                .await
                .unwrap(),
            0
        );
        for server in servers {
            server.abort();
        }
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn first_factor_challenge_has_no_cookie_or_management_authority_over_http() {
        use crate::crypto::{password, token::hash_token};
        use crate::models::session::{MFA_CHALLENGES, Session, SessionPurpose};
        use axum::{
            Router,
            routing::{get, post},
        };
        let db = connect_test_database("mfa_http_boundary")
            .await
            .expect("isolated MongoDB required");
        crate::services::role_service::seed_system_roles(&db)
            .await
            .unwrap();
        let id = uuid::Uuid::new_v4().to_string();
        let password_text = "correct-horse-battery-staple";
        let mut actor = test_user(&id, UserType::Person);
        actor.mfa_enabled = true;
        actor.password_hash = Some(password::hash_password(password_text).unwrap());
        let email = actor.email.clone();
        db.collection::<User>(USERS)
            .insert_one(actor)
            .await
            .unwrap();
        let mut state = test_app_state(db.clone());
        // A local HTTP CONNECT proxy holds the actual configured Telegram
        // transport before TLS. No real vendor request or credential is sent.
        let notice_proxy = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let proxy_url = format!("http://{}", notice_proxy.local_addr().unwrap());
        let notice_started = std::sync::Arc::new(tokio::sync::Notify::new());
        let started = notice_started.clone();
        let notice_server = tokio::spawn(async move {
            use tokio::io::AsyncReadExt;
            let (mut socket, _) = notice_proxy.accept().await.unwrap();
            let mut request = [0u8; 1024];
            let size = socket.read(&mut request).await.unwrap();
            assert!(
                std::str::from_utf8(&request[..size])
                    .unwrap()
                    .starts_with("CONNECT api.telegram.org:443 ")
            );
            started.notify_one();
            std::future::pending::<()>().await;
        });
        state.http_client = reqwest::Client::builder()
            .proxy(reqwest::Proxy::all(proxy_url).unwrap())
            .build()
            .unwrap();
        state.config.telegram_bot_token = Some("fixture-not-real-token".into());
        db.collection::<bson::Document>(crate::models::notification_channel::COLLECTION_NAME).insert_one(doc! {
            "_id":uuid::Uuid::new_v4().to_string(),"user_id":&id,"telegram_enabled":true,"telegram_chat_id":1_i64,
            "push_enabled":false,"created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()
        }).await.unwrap();
        let recovery = "0123456789abcdef0123456789abcdef";
        let row = MfaFactor {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: id.clone(),
            factor_type: "totp".into(),
            secret_encrypted: Some(
                state
                    .encryption_keys
                    .encrypt(b"JBSWY3DPEHPK3PXP")
                    .await
                    .unwrap(),
            ),
            secret_context_version: None,
            recovery_codes: Some(serde_json::json!([
                password::hash_password(recovery).unwrap()
            ])),
            last_totp_counter: None,
            is_verified: true,
            is_active: true,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        db.collection::<MfaFactor>(MFA_FACTORS)
            .insert_one(row)
            .await
            .unwrap();
        let held = std::sync::Arc::new(tokio::sync::Notify::new());
        let release = std::sync::Arc::new(tokio::sync::Notify::new());
        let held_handler = held.clone();
        let release_handler = release.clone();
        let app = Router::new()
            .route(
                "/api/v1/auth/mfa/verify-held",
                post(
                    move |state: State<AppState>,
                          peer: ConnectInfo<SocketAddr>,
                          headers: HeaderMap,
                          body: Json<MfaLoginVerifyRequest>| {
                        let held = held_handler.clone();
                        let release = release_handler.clone();
                        async move {
                            let result = verify(state, peer, headers, body).await;
                            held.notify_one();
                            release.notified().await;
                            result
                        }
                    },
                ),
            )
            .route("/api/v1/auth/login", post(crate::handlers::auth::login))
            .route("/api/v1/auth/mfa/verify", post(verify))
            .route("/api/v1/users/me", get(crate::handlers::users::get_me))
            .route(
                "/api/v1/users/me/preferences/analytics",
                get(crate::handlers::users::get_analytics_consent),
            )
            .with_state(state.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                app.into_make_service_with_connect_info::<SocketAddr>(),
            )
            .await
            .unwrap();
        });
        let client = reqwest::Client::new();
        let response = client
            .post(format!("{base}/api/v1/auth/login"))
            .json(&serde_json::json!({"email":email,"password":password_text,"client":"web"}))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::FORBIDDEN);
        let body: serde_json::Value = response.json().await.unwrap();
        assert_eq!(body["error_code"], 2002);
        let token = body["session_token"]
            .as_str()
            .expect("first-factor challenge");
        assert_eq!(
            db.collection::<Session>("sessions")
                .count_documents(doc! {"token_hash":hash_token(token)})
                .await
                .unwrap(),
            0
        );
        let challenge = db
            .collection::<Session>(MFA_CHALLENGES)
            .find_one(doc! {"token_hash":hash_token(token)})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(challenge.purpose, Some(SessionPurpose::MfaPending));
        // An old sentinel row must also remain powerless on new readers.
        let mut legacy = challenge.clone();
        legacy.id = uuid::Uuid::new_v4().to_string();
        legacy.purpose = None;
        legacy.user_agent = Some("mfa_pending".into());
        legacy.token_hash = hash_token("legacy-pending-token");
        let legacy_id = legacy.id.clone();
        db.collection::<Session>("sessions")
            .insert_one(legacy)
            .await
            .unwrap();
        for pending in [token, "legacy-pending-token"] {
            for path in ["/api/v1/users/me", "/api/v1/users/me/preferences/analytics"] {
                let status = client
                    .get(format!("{base}{path}"))
                    .header("cookie", format!("nyx_session={pending}"))
                    .send()
                    .await
                    .unwrap()
                    .status();
                assert!(
                    status == reqwest::StatusCode::UNAUTHORIZED
                        || status == reqwest::StatusCode::FORBIDDEN,
                    "pending challenge gained authority: {status}"
                );
            }
        }
        // A valid first-party JWT cannot elevate a pending sid, and a refresh
        // record misbound to that sid cannot create a completed login either.
        let issued = crate::services::token_service::create_session_and_issue_tokens(
            &db,
            &state.config,
            &state.jwt_keys,
            &id,
            None,
            None,
        )
        .await
        .unwrap();
        for pending_sid in [&challenge.id, &legacy_id] {
            let mut claims = crate::crypto::jwt::verify_token(
                &state.jwt_keys,
                &state.config,
                &issued.access_token,
            )
            .unwrap();
            claims.sid = Some(pending_sid.clone());
            let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256);
            header.kid = Some(state.jwt_keys.kid.clone());
            let jwt = jsonwebtoken::encode(&header, &claims, &state.jwt_keys.encoding).unwrap();
            assert_eq!(
                client
                    .get(format!("{base}/api/v1/users/me"))
                    .bearer_auth(jwt)
                    .send()
                    .await
                    .unwrap()
                    .status(),
                reqwest::StatusCode::UNAUTHORIZED
            );
            db.collection::<bson::Document>(crate::models::refresh_token::COLLECTION_NAME)
                .update_many(
                    doc! {"user_id":&id},
                    doc! {"$set":{"session_id":pending_sid}},
                )
                .await
                .unwrap();
            assert!(matches!(
                crate::services::token_service::refresh_tokens(
                    &db,
                    &state.config,
                    &state.jwt_keys,
                    &issued.refresh_token,
                    None,
                    None
                )
                .await,
                Err(AppError::Unauthorized(_))
            ));
        }
        // Force the actual handler's session insert to fail after factor and
        // challenge writes. Neither proof may be spent by this failed login.
        db.collection::<Session>("sessions")
            .create_index(
                mongodb::IndexModel::builder()
                    .keys(doc! {"user_agent":1})
                    .options(
                        mongodb::options::IndexOptions::builder()
                            .name("mfa_test_session_failure".to_string())
                            .unique(true)
                            .build(),
                    )
                    .build(),
            )
            .await
            .unwrap();
        let failed = client
            .post(format!("{base}/api/v1/auth/mfa/verify"))
            .json(&serde_json::json!({"mfa_token":token,"code":recovery,"client":"web"}))
            .send()
            .await
            .unwrap();
        assert_eq!(failed.status(), reqwest::StatusCode::INTERNAL_SERVER_ERROR);
        let untouched = db
            .collection::<Session>(MFA_CHALLENGES)
            .find_one(doc! {"_id":&challenge.id})
            .await
            .unwrap()
            .unwrap();
        assert!(
            !untouched.revoked,
            "failed materialization consumed challenge"
        );
        let untouched = db
            .collection::<MfaFactor>(MFA_FACTORS)
            .find_one(doc! {"user_id":&id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            untouched.recovery_codes.unwrap().as_array().unwrap().len(),
            1,
            "failed materialization spent recovery proof"
        );
        db.collection::<Session>("sessions")
            .drop_index("mfa_test_session_failure")
            .await
            .unwrap();
        let complete = tokio::time::timeout(
            std::time::Duration::from_secs(3),
            client
                .post(format!("{base}/api/v1/auth/mfa/verify"))
                .json(&serde_json::json!({"mfa_token":token,"code":recovery,"client":"web"}))
                .send(),
        )
        .await
        .expect("stalled notice must not withhold completed recovery login")
        .unwrap();
        tokio::time::timeout(std::time::Duration::from_secs(2), notice_started.notified())
            .await
            .unwrap();
        assert_eq!(complete.status(), reqwest::StatusCode::OK);
        let cookie = complete
            .headers()
            .get_all("set-cookie")
            .iter()
            .filter_map(|value| value.to_str().ok())
            .find(|value| value.starts_with("nyx_session="))
            .expect("completed session cookie")
            .split(';')
            .next()
            .unwrap()
            .to_owned();
        for path in ["/api/v1/users/me", "/api/v1/users/me/preferences/analytics"] {
            let response = client
                .get(format!("{base}{path}"))
                .header("cookie", &cookie)
                .send()
                .await
                .unwrap();
            let status = response.status();
            let body: serde_json::Value = response.json().await.unwrap();
            assert_eq!(
                status,
                reqwest::StatusCode::OK,
                "completed login GET {path} failed: status={status}, error_code={:?}",
                body.get("error_code")
            );
        }
        let replay = client
            .post(format!("{base}/api/v1/auth/mfa/verify"))
            .json(&serde_json::json!({"mfa_token":token,"code":recovery,"client":"web"}))
            .send()
            .await
            .unwrap();
        assert_eq!(replay.status(), reqwest::StatusCode::UNAUTHORIZED);
        // The existing inline password-login surface also accepts new-format
        // recovery codes; it must not retain the old ten-character limit.
        let inline = "fedcba9876543210fedcba9876543210";
        db.collection::<bson::Document>(MFA_FACTORS)
            .update_one(
                doc! {"user_id":&id},
                doc! {"$push":{"recovery_codes":password::hash_password(inline).unwrap()}},
            )
            .await
            .unwrap();
        let inline_login = client
            .post(format!("{base}/api/v1/auth/login"))
            .json(&serde_json::json!({
            "email":email,"password":password_text,"mfa_code":inline,"client":"web"}))
            .send()
            .await
            .unwrap();
        assert_eq!(inline_login.status(), reqwest::StatusCode::OK);
        // Hold a real completed HTTP login before returning its response. A
        // completed revoke must invalidate that session even if the response
        // carrying its cookie reaches the client later.
        let held_code = "aabbccddaabbccddaabbccddaabbccdd";
        db.collection::<bson::Document>(MFA_FACTORS)
            .update_one(
                doc! {"user_id":&id},
                doc! {"$push":{"recovery_codes":password::hash_password(held_code).unwrap()}},
            )
            .await
            .unwrap();
        let raw = crate::crypto::token::generate_random_token();
        token_service::create_mfa_pending_session_with_method(
            &db,
            &id,
            &hash_token(&raw),
            Some("password"),
        )
        .await
        .unwrap();
        let pending = tokio::spawn({
            let client = client.clone();
            let base = base.clone();
            async move {
                client
                    .post(format!("{base}/api/v1/auth/mfa/verify-held"))
                    .json(&serde_json::json!({"mfa_token":raw,"code":held_code,"client":"web"}))
                    .send()
                    .await
                    .unwrap()
            }
        });
        tokio::time::timeout(std::time::Duration::from_secs(10), held.notified())
            .await
            .unwrap();
        crate::services::admin_user_service::revoke_all_user_sessions(&db, &id)
            .await
            .unwrap();
        release.notify_one();
        let response = pending.await.unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        let late_cookie = response
            .headers()
            .get_all("set-cookie")
            .iter()
            .filter_map(|v| v.to_str().ok())
            .find(|v| v.starts_with("nyx_session="))
            .unwrap()
            .split(';')
            .next()
            .unwrap();
        assert_eq!(
            client
                .get(format!("{base}/api/v1/users/me"))
                .header("cookie", late_cookie)
                .send()
                .await
                .unwrap()
                .status(),
            reqwest::StatusCode::UNAUTHORIZED
        );
        // Already issued first-factor proofs cannot finish after revoke/reset,
        // nor after disable followed by a successor enrollment.
        for invalidation in ["revoke", "reset", "disable_reenroll"] {
            let mut submission_code = "11223344112233441122334411223344".to_owned();
            db.collection::<bson::Document>(MFA_FACTORS).update_one(doc! {"user_id":&id,"is_active":true},
                doc! {"$push":{"recovery_codes":password::hash_password(&submission_code).unwrap()}}).await.unwrap();
            let raw = crate::crypto::token::generate_random_token();
            token_service::create_mfa_pending_session_with_method(
                &db,
                &id,
                &hash_token(&raw),
                Some("password"),
            )
            .await
            .unwrap();
            match invalidation {
                "revoke" => {
                    crate::services::admin_user_service::revoke_all_user_sessions(&db, &id)
                        .await
                        .unwrap();
                }
                "reset" => {
                    let token = crate::services::auth_service::initiate_password_reset(&db, &email)
                        .await
                        .unwrap()
                        .unwrap();
                    crate::services::auth_service::reset_password(
                        &db,
                        &token,
                        "replacement-password",
                    )
                    .await
                    .unwrap();
                }
                _ => {
                    let proof =
                        mfa_service::prepare_totp_disable(&db, &id, Some("replacement-password"))
                            .await
                            .unwrap();
                    mfa_service::disable_totp(
                        &db,
                        &state.encryption_keys,
                        proof,
                        &submission_code,
                        None,
                    )
                    .await
                    .unwrap();
                    let setup = mfa_service::setup_totp(&db, &state.encryption_keys, &id, &email)
                        .await
                        .unwrap();
                    let totp = totp_rs::TOTP::new(
                        totp_rs::Algorithm::SHA1,
                        6,
                        1,
                        30,
                        totp_rs::Secret::Encoded(setup.secret.to_string())
                            .to_bytes()
                            .unwrap(),
                        Some("NyxID".into()),
                        email.clone(),
                    )
                    .unwrap();
                    let codes = mfa_service::verify_totp_setup(
                        &db,
                        &state.encryption_keys,
                        &setup.factor_id,
                        &id,
                        &totp.generate_current().unwrap(),
                        None,
                        None,
                    )
                    .await
                    .unwrap();
                    assert_eq!(codes.len(), 10);
                    submission_code = codes[0].clone();
                }
            }
            let denied = client
                .post(format!("{base}/api/v1/auth/mfa/verify"))
                .json(&serde_json::json!({"mfa_token":raw,"code":submission_code,"client":"web"}))
                .send()
                .await
                .unwrap();
            assert_eq!(
                denied.status(),
                reqwest::StatusCode::UNAUTHORIZED,
                "{invalidation} left pending authority"
            );
        }
        notice_server.abort();
        server.abort();
    }
    #[tokio::test]
    async fn invalid_browser_cookie_does_not_spend_recovery_on_any_completion_surface() {
        use crate::{
            crypto::{password, token},
            services::{auth_device_service as device, login_approval_service as approval},
        };
        use axum::{Router, routing::post};
        for surface in ["standalone", "inline", "approval"] {
            let db = connect_test_database("mfa_cookie_preparation")
                .await
                .expect("isolated MongoDB required");
            let id = uuid::Uuid::new_v4().to_string();
            let password_text = "cookie-preparation-password";
            let recovery = "0123456789abcdef0123456789abcdef";
            let mut actor = test_user(&id, UserType::Person);
            actor.mfa_enabled = true;
            actor.password_hash = Some(password::hash_password(password_text).unwrap());
            let email = actor.email.clone();
            db.collection::<User>(USERS)
                .insert_one(&actor)
                .await
                .unwrap();
            let state = test_app_state(db.clone());
            let mut invalid = state.clone();
            invalid.config.cookie_domain = Some("example.test\ninvalid".into());
            db.collection::<MfaFactor>(MFA_FACTORS)
                .insert_one(MfaFactor {
                    id: uuid::Uuid::new_v4().to_string(),
                    user_id: id.clone(),
                    factor_type: "totp".into(),
                    secret_encrypted: Some(
                        state
                            .encryption_keys
                            .encrypt(b"JBSWY3DPEHPK3PXP")
                            .await
                            .unwrap(),
                    ),
                    secret_context_version: None,
                    recovery_codes: Some(serde_json::json!([
                        password::hash_password(recovery).unwrap()
                    ])),
                    last_totp_counter: None,
                    is_verified: true,
                    is_active: true,
                    created_at: chrono::Utc::now(),
                    updated_at: chrono::Utc::now(),
                })
                .await
                .unwrap();
            let pending = token::generate_random_token();
            token_service::create_mfa_pending_session_for_password(
                &db,
                &actor,
                &token::hash_token(&pending),
            )
            .await
            .unwrap();
            let request = device::initiate_v2(
                &db,
                state.auth_device_hmac_key.as_slice(),
                Default::default(),
                true,
            )
            .await
            .unwrap();
            let (row, secret) = approval::begin(
                &db,
                state.auth_device_hmac_key.as_slice(),
                crate::models::login_approval::LoginFlow::Device,
                &request.user_code,
                true,
            )
            .await
            .unwrap();
            let row = approval::identify_password(&db, &row, &actor)
                .await
                .unwrap();
            let approval_cookie = format!(
                "nyx_approval_{}={}",
                row.id.replace('-', ""),
                secret.as_str()
            );
            let routes = Router::new()
                .route("/auth/login", post(crate::handlers::auth::login))
                .route("/auth/mfa/verify", post(verify))
                .route(
                    "/auth/approval/{id}/mfa",
                    post(crate::handlers::login_approval::mfa),
                );
            let app = Router::new()
                .nest("/bad", routes.clone().with_state(invalid))
                .nest("/good", routes.with_state(state.clone()));
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let base = format!("http://{}", listener.local_addr().unwrap());
            let server = tokio::spawn(async move {
                axum::serve(
                    listener,
                    app.into_make_service_with_connect_info::<SocketAddr>(),
                )
                .await
                .unwrap()
            });
            let (path, body) = match surface {
                "standalone" => (
                    "auth/mfa/verify".to_owned(),
                    serde_json::json!({"mfa_token":pending,"code":recovery,"client":"web"}),
                ),
                "inline" => (
                    "auth/login".to_owned(),
                    serde_json::json!({"email":email,"password":password_text,"mfa_code":recovery,"client":"web"}),
                ),
                _ => (
                    format!("auth/approval/{}/mfa", row.id),
                    serde_json::json!({"code":recovery}),
                ),
            };
            let client = reqwest::Client::new();
            let failed = client
                .post(format!("{base}/bad/{path}"))
                .header("Origin", &state.config.frontend_url)
                .header("Cookie", &approval_cookie)
                .json(&body)
                .send()
                .await
                .unwrap();
            assert_eq!(
                failed.status(),
                reqwest::StatusCode::INTERNAL_SERVER_ERROR,
                "{surface} must reach cookie preparation failure"
            );
            assert_eq!(
                db.collection::<bson::Document>("sessions")
                    .count_documents(doc! {"user_id": &id})
                    .await
                    .unwrap(),
                0,
                "{surface} persisted a session on failed response preparation"
            );
            let factor = db
                .collection::<MfaFactor>(MFA_FACTORS)
                .find_one(doc! {"user_id": &id})
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                factor.recovery_codes.unwrap().as_array().unwrap().len(),
                1,
                "{surface} spent recovery proof"
            );
            assert!(
                !db.collection::<crate::models::login_approval::LoginApproval>(
                    crate::models::login_approval::COLLECTION_NAME
                )
                .find_one(doc! {"_id": &row.id})
                .await
                .unwrap()
                .unwrap()
                .verified
            );
            let completed = client
                .post(format!("{base}/good/{path}"))
                .header("Origin", &state.config.frontend_url)
                .header("Cookie", &approval_cookie)
                .json(&body)
                .send()
                .await
                .unwrap();
            assert_eq!(
                completed.status(),
                reqwest::StatusCode::OK,
                "{surface} immediate retry failed"
            );
            assert!(
                completed
                    .headers()
                    .get_all("set-cookie")
                    .iter()
                    .any(|value| value.to_str().unwrap().starts_with("nyx_session="))
            );
            let factor = db
                .collection::<MfaFactor>(MFA_FACTORS)
                .find_one(doc! {"user_id": &id})
                .await
                .unwrap()
                .unwrap();
            assert_eq!(factor.recovery_codes.unwrap().as_array().unwrap().len(), 0);
            server.abort();
            db.drop().await.unwrap();
        }
    }

    #[tokio::test]
    async fn password_proof_held_across_reset_cannot_install_login_or_approval() {
        use crate::{
            crypto::password,
            models::user::UserType,
            services::{
                auth_device_service as device, auth_service, login_approval_service as approval,
            },
        };
        use axum::{Router, routing::post};
        for surface in ["challenge", "inline", "without_mfa", "approval"] {
            let db = connect_test_database("mfa_stale_password")
                .await
                .expect("isolated MongoDB required");
            let id = uuid::Uuid::new_v4().to_string();
            let old = "old-verified-password";
            let code = "00112233001122330011223300112233";
            let mut actor = test_user(&id, UserType::Person);
            actor.password_hash = Some(password::hash_password(old).unwrap());
            actor.mfa_enabled = surface != "without_mfa";
            let email = actor.email.clone();
            db.collection::<User>(USERS)
                .insert_one(actor)
                .await
                .unwrap();
            let state = test_app_state(db.clone());
            db.collection::<MfaFactor>(MFA_FACTORS)
                .insert_one(MfaFactor {
                    id: uuid::Uuid::new_v4().to_string(),
                    user_id: id.clone(),
                    factor_type: "totp".into(),
                    secret_encrypted: Some(
                        state
                            .encryption_keys
                            .encrypt(b"JBSWY3DPEHPK3PXP")
                            .await
                            .unwrap(),
                    ),
                    secret_context_version: None,
                    recovery_codes: Some(serde_json::json!([
                        password::hash_password(code).unwrap()
                    ])),
                    last_totp_counter: None,
                    is_verified: true,
                    is_active: true,
                    created_at: chrono::Utc::now(),
                    updated_at: chrono::Utc::now(),
                })
                .await
                .unwrap();
            let request = device::initiate_v2(
                &db,
                state.auth_device_hmac_key.as_slice(),
                Default::default(),
                true,
            )
            .await
            .unwrap();
            let (row, _) = approval::begin(
                &db,
                state.auth_device_hmac_key.as_slice(),
                crate::models::login_approval::LoginFlow::Device,
                &request.user_code,
                true,
            )
            .await
            .unwrap();
            let verified = std::sync::Arc::new(tokio::sync::Notify::new());
            let release = std::sync::Arc::new(tokio::sync::Notify::new());
            let handler_verified = verified.clone();
            let handler_release = release.clone();
            let is_approval = surface == "approval";
            let app=Router::new().route("/held-password",post(move |State(state):State<AppState>,ConnectInfo(peer):ConnectInfo<SocketAddr>,headers:HeaderMap,Json(body):Json<crate::handlers::auth::LoginRequest>| {
                let verified=handler_verified.clone();let release=handler_release.clone();let row=row.clone();
                async move {
                    use axum::response::IntoResponse;
                    let actor=auth_service::authenticate_user(&state.db,&body.email,&body.password).await?;
                    verified.notify_one();release.notified().await;
                    if is_approval {
                        Ok::<_,AppError>(crate::handlers::login_approval::finish_password_identity(&state,&row,&actor,&headers,peer).await?.into_response())
                    } else {
                        Ok(crate::handlers::auth::finish_password_login(state,peer,headers,body,actor).await?.into_response())
                    }
                }
            })).with_state(state);
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let base = format!("http://{}", listener.local_addr().unwrap());
            let server = tokio::spawn(async move {
                axum::serve(
                    listener,
                    app.into_make_service_with_connect_info::<SocketAddr>(),
                )
                .await
                .unwrap();
            });
            let pending = tokio::spawn({
                let email = email.clone();
                async move {
                    reqwest::Client::new().post(format!("{base}/held-password")).json(&serde_json::json!({
                    "email":email,"password":old,"client":"web","mfa_code":if surface=="inline"{Some(code)}else{None}
                })).send().await.unwrap()
                }
            });
            tokio::time::timeout(std::time::Duration::from_secs(5), verified.notified())
                .await
                .unwrap();
            let reset = auth_service::initiate_password_reset(&db, &email)
                .await
                .unwrap()
                .unwrap();
            auth_service::reset_password(&db, &reset, "successor-password")
                .await
                .unwrap();
            release.notify_one();
            assert_eq!(
                pending.await.unwrap().status(),
                reqwest::StatusCode::UNAUTHORIZED,
                "stale {surface} password proof gained authority"
            );
            assert_eq!(
                db.collection::<bson::Document>(crate::models::session::MFA_CHALLENGES)
                    .count_documents(doc! {"user_id":&id})
                    .await
                    .unwrap(),
                0
            );
            assert_eq!(
                db.collection::<bson::Document>("sessions")
                    .count_documents(doc! {"user_id":&id,"revoked":false})
                    .await
                    .unwrap(),
                0
            );
            let factor = db
                .collection::<MfaFactor>(MFA_FACTORS)
                .find_one(doc! {"user_id":&id})
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                factor.recovery_codes.unwrap().as_array().unwrap().len(),
                1,
                "stale inline proof consumed recovery code"
            );
            assert_eq!(
                db.collection::<bson::Document>(crate::models::login_approval::COLLECTION_NAME)
                    .count_documents(doc! {"user_id":&id,"verified":true,"closed":false})
                    .await
                    .unwrap(),
                0
            );
            server.abort();
        }
    }
}

crate::errors::redacted_debug!(
    MfaSetupResponse,
    MfaConfirmRequest,
    MfaConfirmResponse,
    MfaLoginVerifyRequest,
    MfaDisableRequest
);

#[cfg(test)]
#[test]
fn mfa_dto_debug_never_discloses_factor_challenge_or_recovery_material() {
    let secret = "distinctive-mfa-plaintext-b821";
    let setup = MfaSetupResponse {
        factor_id: "factor".into(),
        secret: Zeroizing::new(secret.into()),
        qr_code_url: Zeroizing::new(format!("otpauth://{secret}")),
    };
    let confirm = MfaConfirmResponse {
        message: "ok".into(),
        recovery_codes: Zeroizing::new(vec![secret.into()]),
    };
    let request = MfaLoginVerifyRequest {
        code: Zeroizing::new(secret.into()),
        mfa_token: Some(Zeroizing::new(secret.into())),
        client: None,
    };
    let disable = MfaDisableRequest {
        code: Zeroizing::new(secret.into()),
        password: Some(Zeroizing::new(secret.into())),
    };
    for value in [&setup as &dyn std::fmt::Debug, &confirm, &request, &disable] {
        assert!(!format!("{value:?}").contains(secret));
    }
    assert_eq!(serde_json::to_value(setup).unwrap()["secret"], secret);
    assert_eq!(
        serde_json::to_value(confirm).unwrap()["recovery_codes"][0],
        secret
    );
}

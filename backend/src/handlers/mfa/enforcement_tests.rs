use super::*;
use crate::crypto::token::hash_token;
use crate::models::{
    session::{COLLECTION_NAME as SESSIONS, MFA_CHALLENGES, Session},
    user::UserType,
};
use crate::test_utils::{
    connect_transaction_test_database, test_app_state, test_auth_user, test_user,
};
use axum::{
    Router,
    http::{StatusCode, header},
    routing::{get, post},
};
use serde_json::{Value, json};

async fn fixture(name: &str) -> (AppState, String) {
    let db = connect_transaction_test_database(name).await;
    let mut state = test_app_state(db.clone());
    state.mcp_sessions = std::sync::Arc::new(crate::models::mcp_session::McpSessionStore::with_db(
        db.clone(),
    ));
    let id = uuid::Uuid::new_v4().to_string();
    db.collection::<User>(USERS)
        .insert_one(test_user(&id, UserType::Person))
        .await
        .unwrap();
    (state, id)
}

async fn enroll(state: &AppState, id: &str) -> Vec<String> {
    let setup = mfa_service::setup_totp(&state.db, &state.encryption_keys, id, "test@example.com")
        .await
        .unwrap();
    let totp = totp_rs::TOTP::from_url(setup.qr_code_url).unwrap();
    mfa_service::verify_totp_setup(
        &state.db,
        &state.encryption_keys,
        &setup.factor_id,
        id,
        &totp.generate_current().unwrap(),
        None,
        None,
    )
    .await
    .unwrap()
    .to_vec()
}

async fn serve(state: AppState) -> (String, tokio::task::JoinHandle<()>) {
    let router = Router::new()
        .route("/login", post(crate::handlers::auth::login))
        .route("/cli-token", post(crate::handlers::auth::cli_token))
        .route("/login-code", post(crate::handlers::login_code::mint))
        .route("/authorize", get(crate::handlers::oauth::authorize))
        .route(
            "/device-approve",
            post(crate::handlers::auth_device::approve_auth_device),
        )
        .route(
            "/agent-approve",
            post(crate::handlers::auth_agent_key::approve),
        )
        .route("/verify", post(verify_http))
        .route("/cancel", post(cancel))
        .route(
            "/identity",
            get(|auth: AuthUser| async move { auth.user_id.to_string() }),
        )
        .with_state(state);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            router.into_make_service_with_connect_info::<SocketAddr>(),
        )
        .await
        .unwrap();
    });
    (base, server)
}

fn cleared(response: &reqwest::Response) -> bool {
    response
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .any(|v| {
            let value = v.to_str().unwrap();
            value.starts_with("nyx_mfa_challenge=") && value.contains("Max-Age=0")
        })
}

#[tokio::test]
async fn mfa_management_rejects_non_first_party_humans_on_every_operation() {
    use crate::mw::auth::AuthMethod;
    let (state, id) = fixture("mfa_management_guard").await;
    for (method, credential_type) in [
        (AuthMethod::ApiKey, "api_key"),
        (AuthMethod::ServiceAccount, "service_account"),
        (AuthMethod::Delegated, "delegated"),
        (AuthMethod::Relay, "relay"),
        (AuthMethod::AccessToken, "oauth_client"),
    ] {
        let mut auth = test_auth_user(&id);
        auth.auth_method = method.clone();
        if method == AuthMethod::AccessToken {
            auth.oauth_client_id = Some("third-party".into());
        }
        let errors = [
            setup(
                State(state.clone()),
                auth.clone(),
                TelemetryContext::pre_auth(&HeaderMap::new()),
            )
            .await
            .expect_err("setup must reject unsupported credentials"),
            confirm(
                State(state.clone()),
                auth.clone(),
                TelemetryContext::pre_auth(&HeaderMap::new()),
                Json(MfaConfirmRequest {
                    code: "123456".to_owned().into(),
                }),
            )
            .await
            .expect_err("confirmation must reject unsupported credentials"),
            disable(
                State(state.clone()),
                auth,
                Json(MfaDisableRequest {
                    code: "123456".to_owned().into(),
                    password: None,
                }),
            )
            .await
            .expect_err("disable must reject unsupported credentials"),
        ];
        for error in errors {
            let body = error.response_body();
            assert_eq!(
                axum::response::IntoResponse::into_response(error).status(),
                StatusCode::FORBIDDEN
            );
            assert_eq!(body.error_code, 1002);
            assert_eq!(
                body.details,
                Some(json!({
                    "reason": "credential_type_unsupported",
                    "credential_type": credential_type,
                    "accepted": ["user_session"],
                    "hint": crate::errors::access_denial::HUMAN_HINT,
                }))
            );
        }
    }
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn mfa_social_enrollment_keeps_and_assures_current_session_and_revokes_others() {
    use crate::models::refresh_token::{COLLECTION_NAME as REFRESH, RefreshToken};
    let (mut state, id) = fixture("mfa_enrollment_cascade").await;
    state.config.email_auth_enabled = false;
    let current = token_service::create_session_and_issue_tokens(
        &state.db,
        &state.config,
        &state.jwt_keys,
        &id,
        None,
        None,
    )
    .await
    .unwrap();
    let other = token_service::create_session_and_issue_tokens(
        &state.db,
        &state.config,
        &state.jwt_keys,
        &id,
        None,
        None,
    )
    .await
    .unwrap();
    // A legacy row without session_id must be revoked alongside sid-bearing rows.
    let mut legacy = state
        .db
        .collection::<bson::Document>(REFRESH)
        .find_one(doc! {"session_id": &other.session_id})
        .await
        .unwrap()
        .unwrap();
    legacy.insert("_id", uuid::Uuid::new_v4().to_string());
    legacy.insert("jti", uuid::Uuid::new_v4().to_string());
    legacy.remove("session_id");
    state
        .db
        .collection::<bson::Document>(REFRESH)
        .insert_one(legacy)
        .await
        .unwrap();
    let mcp = state
        .mcp_sessions
        .create_with_parent_session(&id, true, true, Some(&other.session_id))
        .await
        .unwrap()
        .unwrap();
    let mut auth = test_auth_user(&id);
    auth.session_id = Some(uuid::Uuid::parse_str(&current.session_id).unwrap());
    auth.ip_address = Some("192.0.2.50".into());
    auth.user_agent = Some("mfa-test-agent".into());
    let pending = setup(
        State(state.clone()),
        auth.clone(),
        TelemetryContext::pre_auth(&HeaderMap::new()),
    )
    .await
    .unwrap()
    .0;
    let totp = totp_rs::TOTP::from_url(pending.qr_code_url).unwrap();
    let sidless = confirm(
        State(state.clone()),
        test_auth_user(&id),
        TelemetryContext::pre_auth(&HeaderMap::new()),
        Json(MfaConfirmRequest {
            code: totp.generate_current().unwrap().into(),
        }),
    )
    .await;
    assert!(matches!(sidless, Err(AppError::Conflict(_))));
    let result = confirm(
        State(state.clone()),
        auth,
        TelemetryContext::pre_auth(&HeaderMap::new()),
        Json(MfaConfirmRequest {
            code: totp.generate_current().unwrap().into(),
        }),
    )
    .await
    .unwrap()
    .0;
    assert!(result.message.to_lowercase().contains("other"));
    let retained = state
        .db
        .collection::<Session>(SESSIONS)
        .find_one(doc! {"_id": &current.session_id})
        .await
        .unwrap()
        .unwrap();
    assert!(!retained.revoked && retained.mfa_assured);
    assert_eq!(
        state
            .db
            .collection::<Session>(SESSIONS)
            .count_documents(doc! {"_id": &other.session_id, "revoked": true})
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        state
            .db
            .collection::<RefreshToken>(REFRESH)
            .count_documents(doc! {"user_id": &id, "revoked": false})
            .await
            .unwrap(),
        1
    );
    assert!(
        state
            .mcp_sessions
            .get_for_auth(&mcp)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        state
            .db
            .collection::<bson::Document>(crate::models::audit_log::COLLECTION_NAME)
            .count_documents(doc! {"user_id": &id, "event_type": "mfa_enabled"})
            .await
            .unwrap(),
        1
    );
    let audit = state
        .db
        .collection::<bson::Document>(crate::models::audit_log::COLLECTION_NAME)
        .find_one(doc! {"user_id": &id, "event_type": "mfa_enabled"})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(audit.get_str("ip_address").unwrap(), "192.0.2.50");
    assert_eq!(audit.get_str("user_agent").unwrap(), "mfa-test-agent");
    assert_eq!(
        token_service::refresh_tokens(
            &state.db,
            &state.config,
            &state.jwt_keys,
            &current.refresh_token,
            None,
            None
        )
        .await
        .unwrap()
        .session_id,
        current.session_id
    );
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn mfa_cookie_challenges_keep_wrong_codes_and_clear_every_terminal_state() {
    let (state, id) = fixture("mfa_cookie_lifecycle").await;
    let codes = enroll(&state, &id).await;
    let (base, server) = serve(state.clone()).await;
    let client = reqwest::Client::new();
    let missing_cookie = client
        .post(format!("{base}/verify"))
        .json(&json!({"code": &codes[0], "client":"web"}))
        .send()
        .await
        .unwrap();
    assert_eq!(missing_cookie.status(), StatusCode::UNAUTHORIZED);
    assert!(cleared(&missing_cookie));
    let missing_cookie: Value = missing_cookie.json().await.unwrap();
    assert_eq!(missing_cookie["details"]["reason"], "mfa_challenge_expired");
    let token = token_service::start_social_mfa_challenge(&state.db, &id, "apple")
        .await
        .unwrap();
    let cookie = format!("nyx_mfa_challenge={}", token.as_str());
    let wrong = client
        .post(format!("{base}/verify"))
        .header("Cookie", &cookie)
        .json(&json!({"code":"wrong", "client":"web"}))
        .send()
        .await
        .unwrap();
    assert!(!cleared(&wrong));
    assert!(!wrong.status().is_success());
    assert_eq!(
        state
            .db
            .collection::<Session>(SESSIONS)
            .count_documents(doc! {"user_id": &id})
            .await
            .unwrap(),
        0
    );
    let success = client
        .post(format!("{base}/verify"))
        .header("Cookie", &cookie)
        .json(&json!({"code": &codes[0], "client":"web"}))
        .send()
        .await
        .unwrap();
    assert_eq!(success.status(), StatusCode::OK);
    assert!(cleared(&success));
    assert!(
        success
            .headers()
            .get_all(header::SET_COOKIE)
            .iter()
            .any(|v| v.to_str().unwrap().starts_with("nyx_session="))
    );
    assert_eq!(
        state
            .db
            .collection::<Session>(SESSIONS)
            .count_documents(doc! {"user_id": &id, "mfa_assured": true})
            .await
            .unwrap(),
        1
    );
    for terminal in ["consumed", "unknown", "expired", "cancelled", "exhausted"] {
        let challenge = if terminal == "consumed" {
            token.to_string()
        } else if terminal == "unknown" {
            "unknown-challenge".into()
        } else {
            token_service::start_social_mfa_challenge(&state.db, &id, "google")
                .await
                .unwrap()
                .to_string()
        };
        let cookie = format!("nyx_mfa_challenge={challenge}");
        if terminal == "expired" {
            state
                .db
                .collection::<bson::Document>(MFA_CHALLENGES)
                .update_one(
                    doc! {"token_hash": hash_token(&challenge)},
                    doc! {"$set": {"expires_at": bson::DateTime::from_millis(0)}},
                )
                .await
                .unwrap();
        } else if terminal == "exhausted" {
            state
                .db
                .collection::<bson::Document>(MFA_CHALLENGES)
                .update_one(
                    doc! {"token_hash": hash_token(&challenge)},
                    doc! {"$set": {"mfa_attempts": 5}},
                )
                .await
                .unwrap();
        } else if terminal == "cancelled" {
            let cancelled = client
                .post(format!("{base}/cancel"))
                .header("Cookie", &cookie)
                .send()
                .await
                .unwrap();
            assert!(cleared(&cancelled));
        }
        let rejected = client
            .post(format!("{base}/verify"))
            .header("Cookie", cookie)
            .json(&json!({"code": &codes[1], "client":"web"}))
            .send()
            .await
            .unwrap();
        assert_eq!(rejected.status(), StatusCode::UNAUTHORIZED, "{terminal}");
        assert!(cleared(&rejected), "{terminal}");
        let body: Value = rejected.json().await.unwrap();
        assert_eq!(body["details"]["reason"], "mfa_challenge_expired");
    }
    // None of the terminal attempts may consume the unused recovery code.
    assert!(
        mfa_service::verify_login_factor(&state.db, &state.encryption_keys, &id, &codes[1], None)
            .await
            .unwrap()
            .is_some()
    );
    server.abort();
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn mfa_unassured_legacy_sessions_fail_cookie_bearer_refresh_and_derived_authority() {
    let (state, id) = fixture("mfa_legacy_authority").await;
    let tokens = token_service::create_session_and_issue_tokens(
        &state.db,
        &state.config,
        &state.jwt_keys,
        &id,
        None,
        None,
    )
    .await
    .unwrap();
    state.db.collection::<bson::Document>(SESSIONS).update_one(doc! {"_id": &tokens.session_id}, doc! {"$set": {"token_hash": hash_token("legacy-cookie")}, "$unset": {"mfa_assured": ""}}).await.unwrap();
    let (base, server) = serve(state.clone()).await;
    let client = reqwest::Client::new();
    for enabled in [false, true] {
        state
            .db
            .collection::<User>(USERS)
            .update_one(doc! {"_id": &id}, doc! {"$set": {"mfa_enabled": enabled}})
            .await
            .unwrap();
        let expected = if enabled {
            StatusCode::UNAUTHORIZED
        } else {
            StatusCode::OK
        };
        assert_eq!(
            client
                .get(format!("{base}/identity"))
                .header("Cookie", "nyx_session=legacy-cookie")
                .send()
                .await
                .unwrap()
                .status(),
            expected
        );
        assert_eq!(
            client
                .get(format!("{base}/identity"))
                .bearer_auth(&tokens.access_token)
                .send()
                .await
                .unwrap()
                .status(),
            expected
        );
        assert_eq!(
            crate::services::session_authority::mfa_assurance(
                &state.db,
                &id,
                Some(&tokens.session_id)
            )
            .await
            .is_ok(),
            !enabled
        );
        if enabled {
            let sidless = crate::crypto::jwt::generate_access_token(
                &state.jwt_keys,
                &state.config,
                &uuid::Uuid::parse_str(&id).unwrap(),
                "openid profile email proxy",
                None,
                None,
                None,
                None,
                None,
            )
            .unwrap();
            assert_eq!(
                client
                    .get(format!("{base}/identity"))
                    .bearer_auth(sidless)
                    .send()
                    .await
                    .unwrap()
                    .status(),
                StatusCode::UNAUTHORIZED
            );
            state
                .db
                .collection::<bson::Document>(crate::models::refresh_token::COLLECTION_NAME)
                .update_one(
                    doc! {"session_id": &tokens.session_id},
                    doc! {"$unset": {"session_id": ""}},
                )
                .await
                .unwrap();
            assert!(
                token_service::refresh_tokens(
                    &state.db,
                    &state.config,
                    &state.jwt_keys,
                    &tokens.refresh_token,
                    None,
                    None
                )
                .await
                .is_err()
            );
            state
                .db
                .collection::<bson::Document>(crate::models::refresh_token::COLLECTION_NAME)
                .update_one(
                    doc! {"user_id": &id},
                    doc! {"$set": {"session_id": &tokens.session_id}},
                )
                .await
                .unwrap();
            for path in ["cli-token", "login-code", "device-approve", "agent-approve"] {
                assert_eq!(
                    client
                        .post(format!("{base}/{path}"))
                        .bearer_auth(&tokens.access_token)
                        .json(&json!({"auth_kind":"account_session", "user_code":"ABCDEFGH"}))
                        .send()
                        .await
                        .unwrap()
                        .status(),
                    StatusCode::UNAUTHORIZED,
                    "{path}"
                );
            }
            // OAuth's optional extractor must treat the rejected legacy
            // session as signed out before resolving a pushed authorization.
            for cookie in [false, true] {
                let request = client
                    .get(format!("{base}/authorize"))
                    .query(&[
                        ("client_id", "test-client"),
                        ("request_uri", "urn:test:request"),
                    ])
                    .header(header::ACCEPT, "application/json");
                let request = if cookie {
                    request.header(header::COOKIE, "nyx_session=legacy-cookie")
                } else {
                    request.bearer_auth(&tokens.access_token)
                };
                let response = request.send().await.unwrap();
                assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
                assert!(!response.headers().contains_key(header::SET_COOKIE));
            }
            assert!(
                token_service::refresh_tokens(
                    &state.db,
                    &state.config,
                    &state.jwt_keys,
                    &tokens.refresh_token,
                    None,
                    None
                )
                .await
                .is_err()
            );
        }
    }
    for _ in 0..5 {
        assert_eq!(
            client
                .get(format!("{base}/identity"))
                .header("Cookie", "nyx_session=legacy-cookie")
                .send()
                .await
                .unwrap()
                .status(),
            StatusCode::UNAUTHORIZED
        );
    }
    assert!(
        state
            .db
            .collection::<Session>(SESSIONS)
            .find_one(doc! {"_id": &tokens.session_id})
            .await
            .unwrap()
            .unwrap()
            .revoked
    );
    assert_eq!(
        state
            .db
            .collection::<bson::Document>(crate::models::audit_log::COLLECTION_NAME)
            .count_documents(doc! {"user_id": &id, "event_type": "mfa_unassured_session_rejected"})
            .await
            .unwrap(),
        1
    );
    // Independent fixture of an assured session (the production rejection
    // never restores a revoked row).
    state
        .db
        .collection::<bson::Document>(crate::models::refresh_token::COLLECTION_NAME)
        .update_many(doc! {"user_id": &id}, doc! {"$set": {"revoked": false}})
        .await
        .unwrap();
    state
        .db
        .collection::<Session>(SESSIONS)
        .update_one(
            doc! {"_id": &tokens.session_id},
            doc! {"$set": {"mfa_assured": true, "revoked": false}},
        )
        .await
        .unwrap();
    assert_eq!(
        client
            .get(format!("{base}/identity"))
            .bearer_auth(&tokens.access_token)
            .send()
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    let derived = token_service::create_derived_session_tokens(
        &state.db,
        &state.config,
        &state.jwt_keys,
        &id,
        None,
        None,
        true,
    )
    .await
    .unwrap();
    assert!(
        crate::services::session_authority::mfa_assurance(
            &state.db,
            &id,
            Some(&derived.session_id)
        )
        .await
        .unwrap()
    );
    let rotated = token_service::refresh_tokens(
        &state.db,
        &state.config,
        &state.jwt_keys,
        &tokens.refresh_token,
        None,
        None,
    )
    .await
    .unwrap();
    assert_eq!(rotated.session_id, tokens.session_id);
    server.abort();
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn mfa_disable_requires_current_factor_and_password_only_when_present() {
    for password_account in [false, true] {
        let (state, id) = fixture("mfa_disable_accounts").await;
        let codes = enroll(&state, &id).await;
        let password = crate::crypto::token::generate_random_token();
        if password_account {
            state.db.collection::<User>(USERS).update_one(doc! {"_id": &id}, doc! {"$set": {"password_hash": crate::crypto::password::hash_password(&password).unwrap()}}).await.unwrap();
            assert!(
                mfa_service::disable_totp_with_code(
                    &state.db,
                    &state.encryption_keys,
                    &id,
                    &codes[0],
                    None,
                    None
                )
                .await
                .is_err()
            );
        }
        let proof = mfa_service::prepare_totp_disable(
            &state.db,
            &id,
            password_account.then_some(password.as_str()),
        )
        .await
        .unwrap();
        let before = state
            .db
            .collection::<bson::Document>(USERS)
            .find_one(doc! {"_id": &id})
            .await
            .unwrap()
            .unwrap();
        for provider in ["google", "github", "apple"] {
            token_service::start_social_mfa_challenge(&state.db, &id, provider)
                .await
                .unwrap();
        }
        let after = state
            .db
            .collection::<bson::Document>(USERS)
            .find_one(doc! {"_id": &id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            before.get("mfa_state_version"),
            after.get("mfa_state_version")
        );
        let factor = mfa_service::disable_totp(
            &state.db,
            &state.encryption_keys,
            proof,
            &codes[0],
            Some(&crate::services::audit_service::AuditActor {
                user_id: id.clone(),
                ip_address: Some("192.0.2.51".into()),
                user_agent: Some("disable-test-agent".into()),
                api_key_id: None,
                api_key_name: None,
            }),
        )
        .await
        .unwrap();
        assert_eq!(factor, mfa_service::LoginFactor::Recovery);
        assert!(
            !state
                .db
                .collection::<User>(USERS)
                .find_one(doc! {"_id": &id})
                .await
                .unwrap()
                .unwrap()
                .mfa_enabled
        );
        let factor = state
            .db
            .collection::<MfaFactor>(MFA_FACTORS)
            .find_one(doc! {"user_id": &id})
            .await
            .unwrap()
            .unwrap();
        assert!(!factor.is_active);
        assert_eq!(
            factor.recovery_codes.unwrap().as_array().unwrap().len(),
            codes.len() - 1
        );
        let audit = state
            .db
            .collection::<bson::Document>(crate::models::audit_log::COLLECTION_NAME)
            .find_one(doc! {"user_id": &id, "event_type": "mfa_disabled"})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(audit.get_str("ip_address").unwrap(), "192.0.2.51");
        assert_eq!(audit.get_str("user_agent").unwrap(), "disable-test-agent");
        let metadata = serde_json::to_string(&audit).unwrap();
        assert!(metadata.contains("recovery"));
        assert!(!metadata.contains(&codes[0]));
        state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn mfa_social_challenge_checks_live_account_and_models_method_without_session() {
    let (state, id) = fixture("mfa_social_fence").await;
    assert!(
        token_service::start_social_mfa_challenge(&state.db, &id, "github")
            .await
            .is_err()
    );
    state
        .db
        .collection::<User>(USERS)
        .update_one(doc! {"_id": &id}, doc! {"$set": {"mfa_enabled": true}})
        .await
        .unwrap();
    for provider in ["google", "github", "apple"] {
        let raw = token_service::start_social_mfa_challenge(&state.db, &id, provider)
            .await
            .unwrap();
        let reserved = mfa_service::reserve_pending_challenge(&state.db, &hash_token(&raw))
            .await
            .unwrap();
        assert_eq!(reserved.session.mfa_method.as_deref(), Some(provider));
        assert!(!reserved.session.mfa_assured);
    }
    state
        .db
        .collection::<User>(USERS)
        .update_one(doc! {"_id": &id}, doc! {"$set": {"is_active": false}})
        .await
        .unwrap();
    assert!(
        token_service::start_social_mfa_challenge(&state.db, &id, "apple")
            .await
            .is_err()
    );
    assert_eq!(
        state
            .db
            .collection::<Session>(SESSIONS)
            .count_documents(doc! {"user_id": &id})
            .await
            .unwrap(),
        0
    );
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn mfa_password_login_and_mobile_token_completion_keep_existing_contracts() {
    let (mut state, id) = fixture("mfa_password_regression").await;
    state.config.email_auth_enabled = true;
    let password = crate::crypto::token::generate_random_token();
    state.db.collection::<User>(USERS).update_one(doc! {"_id": &id}, doc! {"$set": {"password_hash": crate::crypto::password::hash_password(&password).unwrap()}}).await.unwrap();
    let email = state
        .db
        .collection::<User>(USERS)
        .find_one(doc! {"_id": &id})
        .await
        .unwrap()
        .unwrap()
        .email;
    let codes = enroll(&state, &id).await;
    let (base, server) = serve(state.clone()).await;
    let client = reqwest::Client::new();
    let first = client
        .post(format!("{base}/login"))
        .json(&json!({"email": email, "password": password, "client":"web"}))
        .send()
        .await
        .unwrap();
    assert_eq!(first.status(), StatusCode::FORBIDDEN);
    assert!(!first.headers().contains_key(header::SET_COOKIE));
    let challenge: Value = first.json().await.unwrap();
    assert_eq!(challenge["error_code"], 2002);
    let verified = client
        .post(format!("{base}/verify"))
        .json(
            &json!({"code": &codes[0], "mfa_token": challenge["session_token"], "client":"mobile"}),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(verified.status(), StatusCode::OK);
    let tokens: Value = verified.json().await.unwrap();
    assert!(tokens["access_token"].is_string() && tokens["refresh_token"].is_string());
    let inline = client
        .post(format!("{base}/login"))
        .json(
            &json!({"email": email, "password": password, "mfa_code": &codes[1], "client":"token"}),
        )
        .send()
        .await
        .unwrap();
    assert_eq!(inline.status(), StatusCode::OK);
    assert_eq!(
        state
            .db
            .collection::<Session>(SESSIONS)
            .count_documents(doc! {"user_id": &id, "mfa_assured": true})
            .await
            .unwrap(),
        2
    );
    server.abort();
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn mfa_device_browser_and_login_code_deliveries_inherit_approval_assurance() {
    use crate::services::{auth_device_service as device, login_code_service as code};
    let (state, id) = fixture("mfa_derived_delivery").await;
    state
        .db
        .collection::<User>(USERS)
        .update_one(doc! {"_id": &id}, doc! {"$set": {"mfa_enabled": true}})
        .await
        .unwrap();
    for browser in [false, true] {
        let request = device::initiate_v2(
            &state.db,
            state.auth_device_hmac_key.as_slice(),
            Default::default(),
            true,
        )
        .await
        .unwrap();
        device::approve(
            &state.db,
            &state.config,
            &state.jwt_keys,
            &state.encryption_keys,
            state.auth_device_hmac_key.as_slice(),
            device::ApproveInput {
                user_id: id.clone(),
                user_code: request.user_code,
                approver_ip: None,
                approver_user_agent: None,
            },
            true,
        )
        .await
        .unwrap();
        let delivery = if browser {
            device::poll_for_browser(
                &state.db,
                &state.encryption_keys,
                state.auth_device_hmac_key.as_slice(),
                &request.device_code,
                &hash_token("browser-cookie"),
                "127.0.0.1",
                None,
            )
            .await
            .unwrap()
        } else {
            device::poll_and_prepare(
                &state.db,
                &state.encryption_keys,
                state.auth_device_hmac_key.as_slice(),
                &request.device_code,
            )
            .await
            .unwrap()
        };
        let device::PollClaim::Account(delivery) = delivery else {
            panic!("account delivery expected")
        };
        assert!(
            crate::services::session_authority::mfa_assurance(
                &state.db,
                &id,
                Some(&delivery.session_id)
            )
            .await
            .unwrap()
        );
    }
    for assured in [false, true] {
        let minted = code::mint(
            &state.db,
            state.auth_device_hmac_key.as_slice(),
            &id,
            None,
            None,
            assured,
        )
        .await
        .unwrap();
        let result = code::redeem(
            &state.db,
            &state.config,
            &state.jwt_keys,
            &state.encryption_keys,
            state.auth_device_hmac_key.as_slice(),
            &minted.code,
            crate::models::login_client_context::LoginClientContext {
                client_ip: Some("127.0.0.1".into()),
                ..Default::default()
            },
            None,
        )
        .await;
        if assured {
            let code::Delivery::Account(tokens) = result.unwrap() else {
                panic!("account delivery expected")
            };
            assert!(
                crate::services::session_authority::mfa_assurance(
                    &state.db,
                    &id,
                    Some(&tokens.session_id)
                )
                .await
                .unwrap()
            );
        } else {
            assert!(matches!(result, Err(AppError::Unauthorized(_))));
        }
    }
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn mfa_rejection_revokes_and_audits_once_for_each_authority_entry_point() {
    for source in ["cookie", "bearer", "refresh", "mcp"] {
        let (state, id) = fixture("mfa_rejection_once").await;
        let tokens = token_service::create_session_and_issue_tokens(
            &state.db,
            &state.config,
            &state.jwt_keys,
            &id,
            None,
            None,
        )
        .await
        .unwrap();
        state
            .db
            .collection::<bson::Document>(SESSIONS)
            .update_one(
                doc! {"_id": &tokens.session_id},
                doc! {"$set": {"token_hash": hash_token("stale-cookie")}},
            )
            .await
            .unwrap();
        let mcp = state
            .mcp_sessions
            .create_with_parent_session(&id, true, true, Some(&tokens.session_id))
            .await
            .unwrap()
            .unwrap();
        state
            .db
            .collection::<User>(USERS)
            .update_one(doc! {"_id": &id}, doc! {"$set": {"mfa_enabled": true}})
            .await
            .unwrap();
        let (base, server) = serve(state.clone()).await;
        let client = reqwest::Client::new();
        for _ in 0..5 {
            match source {
                "cookie" | "bearer" => {
                    let req = client.get(format!("{base}/identity"));
                    let req = if source == "cookie" {
                        req.header("Cookie", "nyx_session=stale-cookie")
                    } else {
                        req.bearer_auth(&tokens.access_token)
                    };
                    assert_eq!(req.send().await.unwrap().status(), StatusCode::UNAUTHORIZED);
                }
                "refresh" => {
                    assert!(
                        token_service::refresh_tokens(
                            &state.db,
                            &state.config,
                            &state.jwt_keys,
                            &tokens.refresh_token,
                            None,
                            None
                        )
                        .await
                        .is_err()
                    );
                }
                _ => {
                    assert!(
                        crate::services::session_authority::mfa_assurance(
                            &state.db,
                            &id,
                            Some(&tokens.session_id)
                        )
                        .await
                        .is_err()
                    );
                }
            }
        }
        assert!(
            state
                .db
                .collection::<Session>(SESSIONS)
                .find_one(doc! {"_id": &tokens.session_id})
                .await
                .unwrap()
                .unwrap()
                .revoked
        );
        assert_eq!(
            state
                .db
                .collection::<bson::Document>(crate::models::refresh_token::COLLECTION_NAME)
                .count_documents(doc! {"user_id": &id, "revoked": false})
                .await
                .unwrap(),
            0
        );
        assert!(
            state
                .mcp_sessions
                .get_for_auth(&mcp)
                .await
                .unwrap()
                .is_none()
        );
        assert_eq!(
            state
                .db
                .collection::<bson::Document>(crate::models::audit_log::COLLECTION_NAME)
                .count_documents(
                    doc! {"user_id": &id, "event_type": "mfa_unassured_session_rejected"}
                )
                .await
                .unwrap(),
            1,
            "{source}"
        );
        server.abort();
        state.db.drop().await.unwrap();
    }
}

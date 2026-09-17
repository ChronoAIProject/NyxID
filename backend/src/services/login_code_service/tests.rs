use super::*;
use crate::AppState;
use crate::models::{
    api_key::{ApiKey, COLLECTION_NAME as API_KEYS},
    api_key_credential::{ApiKeyCredential, COLLECTION_NAME as CHILDREN},
    session::{COLLECTION_NAME as SESSIONS, Session},
    user::{COLLECTION_NAME as USERS, User, UserType},
};
use crate::test_utils::{connect_transaction_test_database, test_app_state, test_user};

const KEY: &[u8] = b"login-code-fixture-hmac-key";

async fn fixture(name: &str) -> (AppState, String) {
    let db = connect_transaction_test_database(name).await;
    crate::db::ensure_indexes(&db).await.unwrap();
    let user = Uuid::new_v4().to_string();
    db.collection::<User>(USERS)
        .insert_one(test_user(&user, UserType::Person))
        .await
        .unwrap();
    (test_app_state(db), user)
}

fn selection() -> Selection {
    serde_json::from_value(
        serde_json::json!({"kind": "new", "name": "Fixture restricted", "scopes": "proxy"}),
    )
    .unwrap()
}

fn context(ip: &str) -> LoginClientContext {
    LoginClientContext {
        client_label: Some("fixture terminal".into()),
        client_ip: Some(ip.into()),
        client_user_agent: Some("nyxid-cli/test".into()),
        ..Default::default()
    }
}

async fn redeem_code(state: &AppState, code: &str, ip: &str) -> AppResult<Delivery> {
    redeem(
        &state.db,
        &state.config,
        &state.jwt_keys,
        &state.encryption_keys,
        KEY,
        code,
        context(ip),
        Some("fixture"),
    )
    .await
}

#[tokio::test]
async fn concurrent_redemption_commits_exactly_one_grant_of_either_kind() {
    let (state, actor) = fixture("login_code_atomic").await;
    for restricted in [false, true] {
        let code = mint(&state.db, KEY, &actor, restricted.then(selection), None)
            .await
            .unwrap();
        assert_eq!(
            state
                .db
                .collection::<LoginCode>(COLLECTION_NAME)
                .find_one(doc! {"_id": &code.id})
                .await
                .unwrap()
                .unwrap()
                .status,
            Status::Pending
        );
        let (a, b, c) = tokio::join!(
            redeem_code(&state, &code.code, "192.0.2.1"),
            redeem_code(&state, &code.code, "192.0.2.2"),
            redeem_code(&state, &code.code, "192.0.2.3")
        );
        let results = [a, b, c];
        assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
        for error in results.iter().filter_map(|r| r.as_ref().err()) {
            assert!(matches!(error, AppError::LoginCodeRedeemed));
        }
        let row = status(&state.db, &actor, &code.id).await.unwrap();
        assert_eq!(row.status, Status::Redeemed);
        assert_eq!(
            row.context.unwrap().client_label.as_deref(),
            Some("fixture terminal")
        );
        assert!(row.redeemed_at.is_some());
        assert_eq!(row.credential_id.is_some(), restricted);
        assert_eq!(row.session_id.is_some(), !restricted);
    }
    assert_eq!(
        state
            .db
            .collection::<Session>(SESSIONS)
            .count_documents(doc! {"revoked": false})
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        state
            .db
            .collection::<ApiKeyCredential>(CHILDREN)
            .count_documents(doc! {"is_active": true})
            .await
            .unwrap(),
        1
    );
    let parent = state
        .db
        .collection::<ApiKey>(API_KEYS)
        .find_one(doc! {})
        .await
        .unwrap()
        .unwrap();
    assert!(!parent.allow_all_services && !parent.allow_all_nodes);
    assert_eq!(parent.scopes, "proxy");
}

#[tokio::test]
async fn expiry_cancellation_and_invalid_selection_issue_nothing() {
    let (state, actor) = fixture("login_code_no_grant").await;
    for restricted in [false, true] {
        let code = mint(&state.db, KEY, &actor, restricted.then(selection), None)
            .await
            .unwrap();
        cancel(&state.db, &actor, &code.id).await.unwrap();
        assert!(matches!(
            redeem_code(&state, &code.code, "192.0.2.1").await,
            Err(AppError::LoginCodeCancelled)
        ));
        let code = mint(&state.db, KEY, &actor, restricted.then(selection), None)
            .await
            .unwrap();
        state.db.collection::<LoginCode>(COLLECTION_NAME).update_one(doc! {"_id": &code.id},
            doc! {"$set": {"expires_at": bson::DateTime::from_chrono(Utc::now() - Duration::seconds(1))}}).await.unwrap();
        assert!(matches!(
            redeem_code(&state, &code.code, "192.0.2.1").await,
            Err(AppError::LoginCodeExpired)
        ));
    }
    let invalid = serde_json::from_value(
        serde_json::json!({"kind":"new", "name":"bad", "scopes":"root:everything"}),
    )
    .unwrap();
    assert!(
        mint(&state.db, KEY, &actor, Some(invalid), None)
            .await
            .is_err()
    );
    for collection in [SESSIONS, CHILDREN, API_KEYS] {
        assert_eq!(
            state
                .db
                .collection::<bson::Document>(collection)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
    }
}

#[tokio::test]
async fn failed_issuance_rolls_back_claim_and_all_rows_then_can_retry() {
    let (state, actor) = fixture("login_code_rollback").await;
    for restricted in [false, true] {
        let code = mint(&state.db, KEY, &actor, restricted.then(selection), None)
            .await
            .unwrap();
        let collection = if restricted { CHILDREN } else { SESSIONS };
        state
            .db
            .run_command(
                doc! {"collMod": collection, "validator": {"reject_fixture": true},
                "validationLevel": "strict", "validationAction": "error"},
            )
            .await
            .unwrap();
        assert!(redeem_code(&state, &code.code, "192.0.2.1").await.is_err());
        assert_eq!(
            status(&state.db, &actor, &code.id).await.unwrap().status,
            Status::Pending
        );
        assert_eq!(
            state
                .db
                .collection::<ApiKey>(API_KEYS)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
        state
            .db
            .run_command(doc! {"collMod": collection, "validator": {}})
            .await
            .unwrap();
        assert!(redeem_code(&state, &code.code, "192.0.2.1").await.is_ok());
    }
}

#[tokio::test]
async fn source_and_owner_budgets_throttle_distributed_attempts() {
    let (state, actor) = fixture("login_code_limits").await;
    for _ in 0..10 {
        assert!(matches!(
            redeem_code(&state, "ABCD-EFGH", "192.0.2.1").await,
            Err(AppError::LoginCodeInvalid)
        ));
    }
    assert!(matches!(
        redeem_code(&state, "ABCD-EFGH", "192.0.2.1").await,
        Err(AppError::LoginCodeRateLimited)
    ));
    let code = mint(&state.db, KEY, &actor, None, None).await.unwrap();
    cancel(&state.db, &actor, &code.id).await.unwrap();
    for i in 1..=10 {
        assert!(matches!(
            redeem_code(&state, &code.code, &format!("192.0.2.{}", i + 1)).await,
            Err(AppError::LoginCodeCancelled)
        ));
    }
    assert!(matches!(
        redeem_code(&state, &code.code, "192.0.2.20").await,
        Err(AppError::LoginCodeRateLimited)
    ));
    for _ in 0..4 {
        mint(&state.db, KEY, &actor, None, None).await.unwrap();
    }
    assert!(matches!(
        mint(&state.db, KEY, &actor, None, None).await,
        Err(AppError::LoginCodeRateLimited)
    ));
}

async fn protected_status(state: &AppState, token: &str) -> axum::http::StatusCode {
    use tower::ServiceExt;
    let app = axum::Router::new()
        .route(
            "/protected",
            axum::routing::get(|_: crate::mw::auth::AuthUser| async { "allowed" }),
        )
        .with_state(state.clone());
    app.oneshot(
        axum::http::Request::builder()
            .uri("/protected")
            .header("authorization", format!("Bearer {token}"))
            .body(axum::body::Body::empty())
            .unwrap(),
    )
    .await
    .unwrap()
    .status()
}

#[tokio::test]
async fn issuer_revocation_blocks_initial_and_refreshed_access_at_authentication() {
    let (state, actor) = fixture("login_code_revoke_access").await;
    let code = mint(&state.db, KEY, &actor, None, None).await.unwrap();
    let Delivery::Account(tokens) = redeem_code(&state, &code.code, "192.0.2.1").await.unwrap()
    else {
        panic!("expected account");
    };
    assert_eq!(
        protected_status(&state, &tokens.access_token).await,
        axum::http::StatusCode::OK
    );
    let refreshed = token_service::refresh_tokens(
        &state.db,
        &state.config,
        &state.jwt_keys,
        &tokens.refresh_token,
        None,
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        protected_status(&state, &refreshed.access_token).await,
        axum::http::StatusCode::OK
    );
    revoke(&state.db, &actor, &code.id, Some(&state.mcp_sessions))
        .await
        .unwrap();
    for token in [&tokens.access_token, &refreshed.access_token] {
        assert_eq!(
            protected_status(&state, token).await,
            axum::http::StatusCode::UNAUTHORIZED
        );
    }
}

#[tokio::test]
async fn refresh_refuses_expired_deleted_or_revoked_bound_session_before_rotation() {
    let (state, actor) = fixture("login_bound_refresh").await;
    for action in ["expired", "deleted", "revoked"] {
        let code = mint(&state.db, KEY, &actor, None, None).await.unwrap();
        let Delivery::Account(tokens) = redeem_code(&state, &code.code, "192.0.2.1").await.unwrap()
        else {
            panic!("expected account");
        };
        let sessions = state.db.collection::<Session>(SESSIONS);
        if action == "deleted" {
            sessions
                .delete_one(doc! {"_id": &tokens.session_id})
                .await
                .unwrap();
        } else {
            let update = if action == "expired" {
                doc! {"expires_at": bson::DateTime::from_chrono(Utc::now() - Duration::seconds(1))}
            } else {
                doc! {"revoked": true}
            };
            sessions
                .update_one(doc! {"_id": &tokens.session_id}, doc! {"$set": update})
                .await
                .unwrap();
        }
        assert!(matches!(
            token_service::refresh_tokens(
                &state.db,
                &state.config,
                &state.jwt_keys,
                &tokens.refresh_token,
                None,
                None
            )
            .await,
            Err(AppError::Unauthorized(_))
        ));
        assert_eq!(
            state
                .db
                .collection::<bson::Document>(crate::models::refresh_token::COLLECTION_NAME)
                .count_documents(doc! {"session_id": &tokens.session_id})
                .await
                .unwrap(),
            1
        );
    }
}

#[tokio::test]
async fn restricted_browser_handoff_and_direct_delivery_cannot_both_win() {
    let (state, actor) = fixture("login_code_browser_race").await;
    let request = device::initiate_v2(&state.db, KEY, context("192.0.2.1"))
        .await
        .unwrap();
    device::approve_with_agent_key(
        &state.db,
        &state.encryption_keys,
        KEY,
        device::ApproveInput {
            user_id: actor.clone(),
            user_code: request.user_code,
            approver_ip: None,
            approver_user_agent: None,
        },
        selection(),
        None,
    )
    .await
    .unwrap();
    let handoff = browser_handoff(&state.db, KEY, &request.device_code)
        .await
        .unwrap();
    let (code, direct) = tokio::join!(
        redeem_code(&state, &handoff.code, "192.0.2.2"),
        device::poll_agent_key(&state.db, &state.encryption_keys, KEY, &request.device_code)
    );
    assert_ne!(code.is_ok(), direct.is_ok());
    assert_eq!(
        state
            .db
            .collection::<Session>(SESSIONS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        state
            .db
            .collection::<ApiKeyCredential>(CHILDREN)
            .count_documents(doc! {"is_active": true})
            .await
            .unwrap(),
        1
    );
    assert!(
        browser_handoff(&state.db, KEY, &request.device_code)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn third_party_oauth_cannot_mint_or_approve_human_login_grants() {
    use axum::{
        Extension, Router,
        body::Body,
        extract::ConnectInfo,
        http::{Request, StatusCode},
        routing::post,
    };
    use tower::ServiceExt;
    let (state, actor) = fixture("login_code_oauth_guard").await;
    let token = crate::crypto::jwt::generate_oauth_access_token(
        &state.jwt_keys,
        &state.config,
        &Uuid::parse_str(&actor).unwrap(),
        "openid profile",
        None,
        None,
        None,
        None,
        None,
        &Uuid::new_v4().to_string(),
    )
    .unwrap();
    let app = Router::new()
        .route("/code", post(crate::handlers::login_code::mint))
        .route("/code/options", post(crate::handlers::login_code::options))
        .route(
            "/device/approve",
            post(crate::handlers::auth_device::approve_auth_device),
        )
        .route(
            "/device/approve-agent-key",
            post(crate::handlers::auth_device::approve_auth_device_agent_key),
        )
        .route(
            "/device/options",
            post(crate::handlers::auth_device::auth_device_options),
        )
        .route(
            "/device/deny",
            post(crate::handlers::auth_device::deny_auth_device),
        )
        .route(
            "/agent/approve",
            post(crate::handlers::auth_agent_key::approve),
        )
        .route(
            "/agent/options",
            post(crate::handlers::auth_agent_key::options),
        )
        .route("/agent/deny", post(crate::handlers::auth_agent_key::deny))
        .layer(Extension(ConnectInfo(
            "192.0.2.1:9000".parse::<std::net::SocketAddr>().unwrap(),
        )))
        .with_state(state.clone());
    for path in [
        "/code",
        "/code/options",
        "/device/approve",
        "/device/approve-agent-key",
        "/device/options",
        "/device/deny",
        "/agent/approve",
        "/agent/options",
        "/agent/deny",
    ] {
        let body = serde_json::json!({"auth_kind":"account_session", "user_code":"ABCD-EFGH", "selection":{"kind":"existing", "api_key_id":Uuid::new_v4().to_string()}});
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(path)
                    .header("authorization", format!("Bearer {token}"))
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN, "{path}");
    }
    assert_eq!(
        state
            .db
            .collection::<LoginCode>(COLLECTION_NAME)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
}

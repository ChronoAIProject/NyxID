use super::auth::{ChangePasswordRequest, ResetPasswordRequest};
use crate::{
    AppState,
    crypto::{jwt, password},
    models::user::{COLLECTION_NAME as USERS, User, UserType},
    services::{auth_service, mfa_service, token_service},
    test_utils::{connect_test_database, test_app_state, test_user},
};
use axum::{
    body::{Body, to_bytes},
    extract::ConnectInfo,
    http::{HeaderMap, Request, StatusCode},
};
use mongodb::bson::doc;
use serde_json::{Value, json};
use std::net::SocketAddr;
use tower::ServiceExt;
use uuid::Uuid;

async fn state(prefix: &str) -> AppState {
    crate::test_utils::init_test_audit_chain_hmac_key();
    let db = connect_test_database(prefix).await.expect("test MongoDB");
    crate::services::role_service::seed_system_roles(&db)
        .await
        .unwrap();
    test_app_state(db)
}
async fn actor(state: &AppState, credential: Option<&str>, social: bool) -> (User, String) {
    let id = Uuid::new_v4();
    let mut user = test_user(&id.to_string(), UserType::Person);
    user.password_hash = credential.map(|value| password::hash_password(value).unwrap());
    user.social_provider = social.then(|| "google".into());
    state
        .db
        .collection::<User>(USERS)
        .insert_one(&user)
        .await
        .unwrap();
    let token = jwt::generate_access_token(
        &state.jwt_keys,
        &state.config,
        &id,
        "",
        None,
        None,
        None,
        None,
        None,
    )
    .unwrap();
    (user, token)
}
async fn request(
    state: &AppState,
    method: &str,
    path: &str,
    authorization: Option<&str>,
    cookie: Option<&str>,
    forwarded: Option<&str>,
    body: Value,
) -> (StatusCode, Value) {
    let (status, _, body) =
        request_with_headers(state, method, path, authorization, cookie, forwarded, body).await;
    (status, body)
}
async fn request_with_headers(
    state: &AppState,
    method: &str,
    path: &str,
    authorization: Option<&str>,
    cookie: Option<&str>,
    forwarded: Option<&str>,
    body: Value,
) -> (StatusCode, HeaderMap, Value) {
    let (_, router) = crate::routes::build_router_with_state(state.clone());
    let mut req = Request::builder()
        .method(method)
        .uri(format!("/api/v1{path}"))
        .header("Content-Type", "application/json");
    if let Some(token) = authorization {
        req = req.header("Authorization", format!("Bearer {token}"));
    }
    if let Some(cookie) = cookie {
        req = req.header("Cookie", cookie);
    }
    if let Some(forwarded) = forwarded {
        req = req.header("X-Forwarded-For", forwarded);
    }
    let mut req = req
        .body(Body::from(serde_json::to_vec(&body).unwrap()))
        .unwrap();
    req.extensions_mut().insert(ConnectInfo(
        "127.0.0.1:41000".parse::<SocketAddr>().unwrap(),
    ));
    let response = router.with_state(state.clone()).oneshot(req).await.unwrap();
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
    (status, headers, serde_json::from_slice(&bytes).unwrap())
}
async fn change(state: &AppState, token: &str, current: &str, new: &str) -> (StatusCode, Value) {
    request(
        state,
        "POST",
        "/auth/password/change",
        Some(token),
        None,
        None,
        json!({"current_password": current, "new_password": new}),
    )
    .await
}

#[tokio::test]
async fn password_change_mounted_session_social_reset_invalidation_and_audit() {
    let state = state("password_change_mounted").await;
    let (user, token) = actor(&state, Some("old"), true).await;
    let reset = auth_service::initiate_password_reset(&state.db, &user.email)
        .await
        .unwrap()
        .unwrap();
    let browser = token_service::create_session(&state.db, &user.id, None, None)
        .await
        .unwrap();
    let other = token_service::create_session_and_issue_tokens(
        &state.db,
        &state.config,
        &state.jwt_keys,
        &user.id,
        None,
        None,
    )
    .await
    .unwrap();
    let cookie = format!("nyx_session={}", browser.session_token);
    let before = state
        .db
        .collection::<User>(USERS)
        .find_one(doc! {"_id": &user.id})
        .await
        .unwrap()
        .unwrap()
        .password_hash;
    let (status, body) = request(
        &state,
        "POST",
        "/auth/password/change",
        None,
        Some(&cookie),
        None,
        json!({"current_password":"old", "new_password":"abcdefé"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(body.get("password_hash").is_none());
    let stored = state
        .db
        .collection::<User>(USERS)
        .find_one(doc! {"_id": &user.id})
        .await
        .unwrap()
        .unwrap();
    assert_ne!(stored.password_hash, before);
    assert!(stored.password_reset_token.is_none());
    assert!(stored.password_reset_expires_at.is_none());
    assert!(
        auth_service::reset_password(&state.db, &reset, "resetpassword")
            .await
            .is_err()
    );
    assert!(
        auth_service::authenticate_user(&state.db, &user.email, "abcdefé")
            .await
            .is_ok()
    );
    assert!(
        auth_service::authenticate_user(&state.db, &user.email, "old")
            .await
            .is_err()
    );
    // A same-as-current replacement succeeds, salts anew, and invalidates resets too.
    let reset = auth_service::initiate_password_reset(&state.db, &user.email)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        change(&state, &token, "abcdefé", "abcdefé").await.0,
        StatusCode::OK
    );
    assert!(
        auth_service::reset_password(&state.db, &reset, "resetpassword")
            .await
            .is_err()
    );
    for (credential, session_cookie) in [
        (Some(token.as_str()), None),
        (Some(other.access_token.as_str()), None),
        (None, Some(cookie.as_str())),
    ] {
        let (status, body) = request(
            &state,
            "GET",
            "/users/me",
            credential,
            session_cookie,
            None,
            Value::Null,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["has_password"], true);
        assert_eq!(body["social_provider"], "google");
        assert!(body.get("password_hash").is_none());
    }
    token_service::refresh_tokens(
        &state.db,
        &state.config,
        &state.jwt_keys,
        &other.refresh_token,
        None,
        None,
    )
    .await
    .expect("other session refresh remains valid");
    let mut audit = None;
    for _ in 0..100 {
        audit = state
            .db
            .collection::<crate::models::audit_log::AuditLog>(
                crate::models::audit_log::COLLECTION_NAME,
            )
            .find_one(doc! {"event_type":"password_changed", "user_id": &user.id})
            .await
            .unwrap();
        if audit.is_some() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    let audit = audit.expect("password change audit");
    assert_eq!(
        audit.event_data.unwrap(),
        json!({"sessions_retained": true})
    );
    assert_eq!(audit.ip_address.as_deref(), Some("127.0.0.1"));
}

#[tokio::test]
async fn password_change_profile_presence_and_input_byte_boundaries() {
    let state = state("password_change_validation").await;
    for social in [false, true] {
        for credential in [None, Some("oldpassword")] {
            let (_, token) = actor(&state, credential, social).await;
            let (status, body) = request(
                &state,
                "GET",
                "/users/me",
                Some(&token),
                None,
                None,
                Value::Null,
            )
            .await;
            assert_eq!(status, StatusCode::OK);
            assert_eq!(body["has_password"], credential.is_some());
            assert!(body.get("password_hash").is_none());
            if credential.is_none() {
                assert_eq!(
                    change(&state, &token, "anything", "newpassword").await.0,
                    StatusCode::BAD_REQUEST
                );
            }
        }
    }
    for (new, accepted) in [
        ("a".repeat(7), false),
        ("abcdefé".into(), true),
        (format!("{}a", "é".repeat(63)), true),
        ("é".repeat(64), true),
        (format!("{}abc", "🔐".repeat(31)), true),
        (format!("{}abcde", "🔐".repeat(31)), false),
        (format!("{}a", "é".repeat(64)), false),
    ] {
        let (user, token) = actor(&state, Some("oldpassword"), false).await;
        let (status, body) = change(&state, &token, "oldpassword", &new).await;
        assert_eq!(
            status,
            if accepted {
                StatusCode::OK
            } else {
                StatusCode::BAD_REQUEST
            },
            "{body}"
        );
        if accepted {
            assert!(
                auth_service::authenticate_user(&state.db, &user.email, &new)
                    .await
                    .is_ok()
            );
        } else {
            assert!(body["message"].as_str().unwrap().contains("UTF-8 bytes"));
        }
    }
    let (_, token) = actor(&state, Some("oldpassword"), false).await;
    for current in ["".to_string(), "é".repeat(64) + "a", "wrong".into()] {
        let (status, _) = change(&state, &token, &current, "newpassword").await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }
}

#[tokio::test]
async fn password_change_rejects_non_human_and_untrusted_credentials() {
    let state = state("password_change_authority").await;
    let (user, token) = actor(&state, Some("oldpassword"), false).await;
    let body = json!({"current_password":"oldpassword", "new_password":"newpassword"});
    assert_eq!(
        request(
            &state,
            "POST",
            "/auth/password/change",
            None,
            None,
            None,
            body.clone()
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    let oauth = jwt::generate_oauth_access_token(
        &state.jwt_keys,
        &state.config,
        &Uuid::parse_str(&user.id).unwrap(),
        "account:read",
        None,
        None,
        None,
        None,
        None,
        "third-party",
    )
    .unwrap();
    assert_eq!(
        change(&state, &oauth, "oldpassword", "newpassword").await.0,
        StatusCode::FORBIDDEN
    );
    let delegated = jwt::generate_delegated_access_token(
        &state.jwt_keys,
        &state.config,
        &Uuid::parse_str(&user.id).unwrap(),
        "account:read",
        "password-change-client",
        3600,
        None,
    )
    .unwrap();
    let (sa, secret) = crate::services::service_account_service::create_service_account(
        &state.db,
        "Password change forbidden",
        None,
        "account:read",
        &[],
        None,
        &user.id,
    )
    .await
    .unwrap();
    let service = crate::services::service_account_service::authenticate_client_credentials(
        &state.db,
        &state.config,
        &state.jwt_keys,
        &sa.client_id,
        &secret,
        None,
    )
    .await
    .unwrap()
    .access_token;
    for rejected in [&delegated, &service] {
        let (status, body) = change(&state, rejected, "oldpassword", "newpassword").await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
        assert_eq!(body["error_code"], 1002);
    }
    let key = crate::services::key_service::create_api_key(
        &state.db,
        &user.id,
        "password-change-security-test",
        "read write proxy",
        None,
        None,
        None,
        None,
        Some(false),
        None,
        Some(true),
        None,
        None,
        Some("codex"),
        None,
    )
    .await
    .unwrap();
    let relay = jwt::generate_relay_access_token(
        &state.jwt_keys,
        &state.config,
        &Uuid::parse_str(&user.id).unwrap(),
        &key.scopes,
        None,
        &jwt::RelayAgentScope {
            api_key_id: key.id.clone(),
            api_key_name: key.name.clone(),
            allowed_service_ids: key.allowed_service_ids.clone(),
            allowed_node_ids: key.allowed_node_ids.clone(),
            allow_all_services: key.allow_all_services,
            allow_all_nodes: key.allow_all_nodes,
        },
    )
    .unwrap();
    let (status, body) = change(&state, &relay, "oldpassword", "newpassword").await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["error_code"], 1002);
    assert_eq!(
        change(&state, &key.full_key, "oldpassword", "newpassword")
            .await
            .0,
        StatusCode::FORBIDDEN
    );
    state
        .db
        .collection::<User>(USERS)
        .update_one(doc! {"_id": &user.id}, doc! {"$set": {"user_type":"org"}})
        .await
        .unwrap();
    let (status, body) = change(&state, &token, "oldpassword", "newpassword").await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert_eq!(body["error_code"], 1403);
    state
        .db
        .collection::<User>(USERS)
        .update_one(
            doc! {"_id": &user.id},
            doc! {"$set": {"user_type":"person", "is_active":false}},
        )
        .await
        .unwrap();
    let (status, body) = change(&state, &token, "oldpassword", "newpassword").await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    assert_eq!(body["error_code"], 1001);
}

#[tokio::test]
async fn password_change_shared_user_throttle_counts_success_and_precedes_hash_verification() {
    crate::test_utils::ensure_rate_window_headroom(
        std::time::Duration::from_secs(60),
        std::time::Duration::from_secs(20),
    )
    .await;
    let mut state = state("password_change_user_limit").await;
    state.config.trusted_proxy_ips = vec!["127.0.0.1".parse().unwrap()];
    let (_, token) = actor(&state, Some("oldpassword"), false).await;
    let other_replica = test_app_state(state.db.clone());
    for attempt in 0..5 {
        let replica = if attempt % 2 == 0 {
            &state
        } else {
            &other_replica
        };
        let current = if attempt == 0 { "oldpassword" } else { "wrong" };
        assert_eq!(
            request(
                replica,
                "POST",
                "/auth/password/change",
                Some(&token),
                None,
                Some(&format!("192.0.2.{}", attempt + 1)),
                json!({"current_password":current, "new_password":"oldpassword"})
            )
            .await
            .0,
            if attempt == 0 {
                StatusCode::OK
            } else {
                StatusCode::BAD_REQUEST
            }
        );
    }
    state
        .db
        .collection::<User>(USERS)
        .update_many(doc! {}, doc! {"$set": {"password_hash":"malformed"}})
        .await
        .unwrap();
    let (status, body) = change(&other_replica, &token, "oldpassword", "newpassword").await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(body["error_code"], 1005);
}

#[tokio::test]
async fn password_change_shared_ip_throttle_ignores_untrusted_forwarding() {
    crate::test_utils::ensure_rate_window_headroom(
        std::time::Duration::from_secs(60),
        std::time::Duration::from_secs(20),
    )
    .await;
    let state = state("password_change_ip_limit").await;
    let mut tokens = vec![];
    for _ in 0..5 {
        tokens.push(actor(&state, None, false).await.1);
    }
    let other_replica = test_app_state(state.db.clone());
    for attempt in 0..20 {
        let replica = if attempt % 2 == 0 {
            &state
        } else {
            &other_replica
        };
        let (status, _) = request(
            replica,
            "POST",
            "/auth/password/change",
            Some(&tokens[attempt / 4]),
            None,
            Some(&format!("192.0.2.{}", attempt + 1)),
            json!({"current_password":"anything", "new_password":"newpassword"}),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }
    state
        .db
        .collection::<User>(USERS)
        .update_many(doc! {}, doc! {"$set": {"password_hash":"malformed"}})
        .await
        .unwrap();
    let (status, body) = change(&other_replica, &tokens[0], "anything", "newpassword").await;
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(body["error_code"], 1005);
}

#[test]
fn password_requests_debug_is_redacted() {
    let change: ChangePasswordRequest = serde_json::from_value(
        json!({"current_password":"secret-old", "new_password":"secret-new"}),
    )
    .unwrap();
    let reset: ResetPasswordRequest =
        serde_json::from_value(json!({"token":"secret-token", "new_password":"secret-new"}))
            .unwrap();
    assert_eq!(format!("{change:?}"), "ChangePasswordRequest { .. }");
    assert_eq!(format!("{reset:?}"), "ResetPasswordRequest { .. }");
}

#[tokio::test]
async fn password_change_pending_mfa_is_not_a_first_party_human_session() {
    let state = state("password_change_pending_mfa").await;
    let (user, _) = actor(&state, Some("oldpassword"), false).await;
    let setup = mfa_service::setup_totp(&state.db, &state.encryption_keys, &user.id, &user.email)
        .await
        .unwrap();
    let totp = totp_rs::TOTP::from_url(setup.qr_code_url).unwrap();
    mfa_service::verify_totp_setup(
        &state.db,
        &state.encryption_keys,
        &setup.factor_id,
        &user.id,
        &totp.generate_current().unwrap(),
    )
    .await
    .unwrap();
    state
        .db
        .collection::<User>(USERS)
        .update_one(doc! {"_id": &user.id}, doc! {"$set": {"mfa_enabled":true}})
        .await
        .unwrap();
    let (status, body) = request(
        &state,
        "POST",
        "/auth/login",
        None,
        None,
        None,
        json!({"email":user.email, "password":"oldpassword", "client":"web"}),
    )
    .await;
    assert_eq!(body["error_code"], 2002, "{status}: {body}");
    assert_eq!(status, StatusCode::FORBIDDEN);
    let pending = body["session_token"].as_str().unwrap();
    let (status, qr) = request(
        &state,
        "POST",
        "/auth/device/request",
        None,
        None,
        None,
        json!({}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{qr}");
    let cookie = format!("nyx_session={pending}");
    for (method, path, body) in [
        ("GET", "/users/me", Value::Null),
        (
            "POST",
            "/auth/password/change",
            json!({"current_password":"oldpassword", "new_password":"newpassword"}),
        ),
        (
            "POST",
            "/auth/device/approve",
            json!({"user_code":qr["user_code"]}),
        ),
    ] {
        let (status, body) = request(&state, method, path, None, Some(&cookie), None, body).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{path}: {body}");
        assert_eq!(body["error_code"], 1001);
    }
    let (status, headers, body) = request_with_headers(
        &state,
        "POST",
        "/auth/mfa/verify",
        None,
        None,
        None,
        json!({"mfa_token":pending, "code":totp.generate_current().unwrap(), "client":"web"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["user_id"], user.id);
    let cookie = headers
        .get_all("set-cookie")
        .iter()
        .filter_map(|value| value.to_str().ok())
        .find(|value| value.starts_with("nyx_session="))
        .expect("MFA verification issues browser session cookie")
        .split(';')
        .next()
        .unwrap();
    let (status, profile) = request(
        &state,
        "GET",
        "/users/me",
        None,
        Some(cookie),
        None,
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{profile}");
    assert_eq!(profile["id"], user.id);
    assert_eq!(profile["mfa_enabled"], true);
    let (status, approval) = request(
        &state,
        "POST",
        "/auth/device/approve",
        None,
        Some(cookie),
        None,
        json!({"user_code":qr["user_code"]}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{approval}");
    assert_eq!(approval["ok"], true);
    // Completed pending proofs stay unusable even after legitimate MFA succeeds.
    let (status, body) = request(
        &state,
        "GET",
        "/users/me",
        None,
        Some(&format!("nyx_session={pending}")),
        None,
        Value::Null,
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
    assert_eq!(body["error_code"], 1001);
    assert!(
        auth_service::authenticate_user(&state.db, &user.email, "oldpassword")
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn password_change_concurrent_mounted_requests_have_one_winner() {
    let state = state("password_change_mounted_race").await;
    let (user, token) = actor(&state, Some("oldpassword"), false).await;
    let (first, second) = tokio::join!(
        change(&state, &token, "oldpassword", "firstnewpassword"),
        change(&state, &token, "oldpassword", "secondnewpassword"),
    );
    assert_eq!(
        [first.0, second.0]
            .iter()
            .filter(|status| **status == StatusCode::OK)
            .count(),
        1,
        "{first:?} {second:?}"
    );
    let (winner, loser, rejected) = if first.0 == StatusCode::OK {
        ("firstnewpassword", "secondnewpassword", &second)
    } else {
        ("secondnewpassword", "firstnewpassword", &first)
    };
    match rejected.0 {
        StatusCode::BAD_REQUEST => {
            assert_eq!(rejected.1["error_code"], 1008);
            assert_eq!(
                rejected.1["message"],
                "Validation error: Current password is incorrect"
            );
        }
        StatusCode::CONFLICT => assert_eq!(rejected.1["error_code"], 1004),
        _ => panic!("unexpected concurrent loser: {rejected:?}"),
    }
    let stored = state
        .db
        .collection::<User>(USERS)
        .find_one(doc! {"_id": &user.id})
        .await
        .unwrap()
        .unwrap();
    let hash = stored.password_hash.as_deref().unwrap();
    assert!(password::verify_password(winner, hash).unwrap());
    assert!(!password::verify_password(loser, hash).unwrap());
    assert!(!password::verify_password("oldpassword", hash).unwrap());
}

#[tokio::test]
async fn password_change_reset_route_uses_server_byte_policy() {
    let state = state("password_change_reset_policy").await;
    let (user, _) = actor(&state, Some("oldpassword"), false).await;
    let token = auth_service::initiate_password_reset(&state.db, &user.email)
        .await
        .unwrap()
        .unwrap();
    for new in ["a".repeat(7), "é".repeat(64) + "a"] {
        let (status, body) = request(
            &state,
            "POST",
            "/auth/reset-password",
            None,
            None,
            None,
            json!({"token":token, "new_password":new}),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(body["message"].as_str().unwrap().contains("UTF-8 bytes"));
    }
    let (status, body) = request(
        &state,
        "POST",
        "/auth/reset-password",
        None,
        None,
        None,
        json!({"token":token, "new_password":"abcdefé"}),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        auth_service::authenticate_user(&state.db, &user.email, "abcdefé")
            .await
            .is_ok()
    );
    assert_eq!(
        request(
            &state,
            "POST",
            "/auth/reset-password",
            None,
            None,
            None,
            json!({"token":token, "new_password":"abcdefé"})
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    let token = auth_service::initiate_password_reset(&state.db, &user.email)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        request(
            &state,
            "POST",
            "/auth/reset-password",
            None,
            None,
            None,
            json!({"token":token, "new_password":"abcdefé"})
        )
        .await
        .0,
        StatusCode::OK
    );
}

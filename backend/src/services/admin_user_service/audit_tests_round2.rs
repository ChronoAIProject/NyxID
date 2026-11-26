use super::*;
use crate::services::{audit_service::AuditActor, auth_service, security_effect_service};
use std::future::IntoFuture;

async fn target_disappears(action: Action) {
    let mut f = fixture().await;
    let original = f.state.db.clone();
    let target = f.target.clone();
    let deleted = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let observed = deleted.clone();
    let mut options =
        mongodb::options::ClientOptions::parse(std::env::var("NYXID_TEST_DATABASE_URL").unwrap())
            .await
            .unwrap();
    options.command_event_handler = Some(EventHandler::callback(move |event| {
        let CommandEvent::Started(event) = event else {
            return;
        };
        if event.command_name == "update"
            && event.command.get_str("update") == Ok("users")
            && !observed.swap(true, std::sync::atomic::Ordering::SeqCst)
        {
            // The synchronous callback blocks dispatch until the deletion is
            // acknowledged. AsyncCallback is spawned by the driver, not awaited.
            tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current().block_on(async {
                    let result = tokio::time::timeout(
                        std::time::Duration::from_secs(5),
                        original
                            .collection::<Document>("users")
                            .delete_one(doc! {"_id": &target})
                            .into_future(),
                    )
                    .await
                    .unwrap()
                    .unwrap();
                    assert_eq!(result.deleted_count, 1);
                });
            });
        }
    }));
    f.state.db = mongodb::Client::with_options(options)
        .unwrap()
        .database(f.state.db.name());
    let response = request(&f, action).await;
    assert!(deleted.load(std::sync::atomic::Ordering::SeqCst));
    let expected = match action {
        Action::Update | Action::Role => 500,
        _ => 404,
    };
    assert_eq!(
        response.status().as_u16(),
        expected,
        "{action:?}: vanished target must fail"
    );
    let body: serde_json::Value = serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), 4096)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(
        body["error_code"],
        if expected == 500 { 1006 } else { 1003 }
    );
    assert!(
        snapshot(&f.state.db, INTENTS).await.is_empty(),
        "{action:?}: no action, no intent"
    );
    security_effect_service::sweep(&f.state.db, &f.state.encryption_keys)
        .await
        .unwrap();
    assert!(snapshot(&f.state.db, "audit_log").await.is_empty());
    assert!(
        !snapshot(&f.state.db, "sessions").await[0]
            .get_bool("revoked")
            .unwrap()
    );
    f.state.db.drop().await.unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn vanished_profile_target_has_no_audit() {
    target_disappears(Action::Update).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn vanished_role_target_has_no_audit() {
    target_disappears(Action::Role).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn vanished_email_target_has_no_audit() {
    target_disappears(Action::Email).await;
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn vanished_reset_target_has_no_audit() {
    target_disappears(Action::Password).await;
}

#[tokio::test]
async fn forced_reset_fixture_uses_production_audit_boundary() {
    let f = fixture().await;
    super::super::force_password_reset(&f.state.db, &f.target)
        .await
        .unwrap();
    assert_eq!(
        snapshot(&f.state.db, INTENTS).await.len(),
        1,
        "fixture must exercise the production atomic reset"
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn ordinary_and_admin_reset_tokens_complete_with_unchanged_ordinary_writes() {
    for admin_reset in [false, true] {
        let f = fixture().await;
        let user = f
            .state
            .db
            .collection::<User>("users")
            .find_one(doc! {"_id": &f.target})
            .await
            .unwrap()
            .unwrap();
        let token = if admin_reset {
            super::super::force_password_reset_as(
                &f.state.db,
                &f.target,
                &AuditActor::from_auth_user(&audit_auth(&f.actor)),
            )
            .await
            .unwrap()
            .unwrap()
        } else {
            let token = REQUEST
                .scope(
                    (),
                    auth_service::initiate_password_reset(&f.state.db, &user.email.to_uppercase()),
                )
                .await
                .unwrap()
                .unwrap();
            let commands = f.monitor.lock().unwrap().commands.clone();
            assert_eq!(
                commands,
                vec![
                    ("find".into(), "users".into()),
                    ("update".into(), "users".into())
                ]
            );
            println!("ORDINARY_RESET_COMMANDS {commands:?}");
            token
        };
        let mut actual = f
            .state
            .db
            .collection::<Document>("users")
            .find_one(doc! {"_id": &f.target})
            .await
            .unwrap()
            .unwrap();
        let mut expected = bson::to_document(&user).unwrap();
        let stored_hash = actual.get_str("password_reset_token").unwrap().to_owned();
        assert_eq!(stored_hash, crate::crypto::token::hash_token(&token));
        let expires = actual
            .get_datetime("password_reset_expires_at")
            .unwrap()
            .timestamp_millis();
        let updated = actual
            .get_datetime("updated_at")
            .unwrap()
            .timestamp_millis();
        assert!((3_599_000..=3_600_000).contains(&(expires - updated)));
        if !admin_reset {
            // Compare the entire stored row, normalizing only the fresh token
            // and two clock values also generated by the base implementation.
            for (field, value) in [
                (
                    "password_reset_token",
                    bson::Bson::String("<generated-hash>".into()),
                ),
                (
                    "password_reset_expires_at",
                    bson::Bson::DateTime(bson::DateTime::from_millis(3_600_000)),
                ),
                (
                    "updated_at",
                    bson::Bson::DateTime(bson::DateTime::from_millis(0)),
                ),
            ] {
                actual.insert(field, value.clone());
                expected.insert(field, value);
            }
            assert_eq!(
                bson::to_vec(&actual).unwrap(),
                bson::to_vec(&expected).unwrap(),
                "base reset row bytes changed"
            );
            println!("ORDINARY_RESET_BASE_ROW_BYTES_MATCH");
        }
        auth_service::reset_password(&f.state.db, &token, "replacement-password")
            .await
            .unwrap();
        let stored = f
            .state
            .db
            .collection::<User>("users")
            .find_one(doc! {"_id": &f.target})
            .await
            .unwrap()
            .unwrap();
        assert!(
            crate::crypto::password::verify_password(
                "replacement-password",
                stored.password_hash.as_deref().unwrap()
            )
            .unwrap()
        );
        assert!(stored.password_reset_token.is_none());
        assert!(stored.password_reset_expires_at.is_none());
        f.state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn long_user_agent_cannot_block_admin_security_action() {
    let f = fixture().await;
    let mut auth = audit_auth(&f.actor);
    auth.user_agent = Some("U".repeat(64 * 1024));
    let response = admin::verify_user_email(
        State(f.state.clone()),
        auth,
        HeaderMap::new(),
        Path(f.target.clone()),
    )
    .await;
    assert!(
        response.is_ok(),
        "untrusted User-Agent must not block verification"
    );
    security_effect_service::sweep(&f.state.db, &f.state.encryption_keys)
        .await
        .unwrap();
    let rows = snapshot(&f.state.db, "audit_log").await;
    let ua = rows[0].get_str("user_agent").unwrap();
    assert_eq!(ua.len(), 1024);
    assert!(ua.ends_with("...[truncated]"));
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn maximum_profile_fields_and_long_user_agent_succeed() {
    let f = fixture().await;
    let mut auth = audit_auth(&f.actor);
    auth.user_agent = Some("U".repeat(64 * 1024));
    auth.api_key_id = Some(uuid::Uuid::new_v4().to_string());
    auth.api_key_name = Some("k".repeat(200));
    auth.ip_address = Some("ffff:ffff:ffff:ffff:ffff:ffff:255.255.255.255".into());
    let email = format!("{}@{}.test", "e".repeat(64), "d".repeat(184));
    assert_eq!(email.len(), 254);
    let avatar = format!("https://example.test/{}", "a".repeat(2027));
    assert_eq!(avatar.len(), 2048);
    let response = admin::update_user(
        State(f.state.clone()),
        auth,
        HeaderMap::new(),
        Path(f.target.clone()),
        Json(admin::UpdateUserRequest {
            display_name: Some("n".repeat(200)),
            email: Some(email),
            avatar_url: Some(avatar),
        }),
    )
    .await;
    assert!(response.is_ok(), "valid maximum profile metadata must fit");
    let rows = snapshot(&f.state.db, INTENTS).await;
    let bytes = bson::to_vec(rows[0].get_document("event").unwrap())
        .unwrap()
        .len();
    println!("MAX_PROFILE_AUDIT_BYTES {bytes}");
    assert!(bytes < 16 * 1024);
    f.state.db.drop().await.unwrap();
}

async fn relayed_admin_event(f: &Fixture, event_type: &str) -> Document {
    security_effect_service::sweep(&f.state.db, &f.state.encryption_keys)
        .await
        .unwrap();
    f.state
        .db
        .collection::<Document>("audit_log")
        .find_one(doc! {"event_type": event_type})
        .await
        .unwrap()
        .unwrap()
}

#[tokio::test]
async fn oversized_email_keeps_account_value_and_bounds_both_audit_values() {
    let f = fixture().await;
    let email = format!("{}@example.test", "a".repeat(17 * 1024));
    let Json(response) = admin::update_user(
        State(f.state.clone()),
        audit_auth(&f.actor),
        HeaderMap::new(),
        Path(f.target.clone()),
        Json(admin::UpdateUserRequest {
            display_name: None,
            email: Some(email.clone()),
            avatar_url: None,
        }),
    )
    .await
    .expect("accepted email must not be rejected by the audit bound");
    assert_eq!(response.email, email);
    let row = relayed_admin_event(&f, "admin.user.updated").await;
    let metadata = row.get_document("event_data").unwrap();
    for value in [
        metadata.get_str("target_email").unwrap(),
        metadata
            .get_document("changes")
            .unwrap()
            .get_str("email")
            .unwrap(),
    ] {
        assert_eq!(value.len(), 512);
        assert_eq!(value, format!("{}...[truncated]", "a".repeat(498)));
    }
    let user = f
        .state
        .db
        .collection::<User>("users")
        .find_one(doc! {"_id": &f.target})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(user.email, email);
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn oversized_forwarded_ip_header_cannot_block_admin_action() {
    use axum::{Router, body::Body, http::Request, routing::post};
    use tower::ServiceExt;
    let f = fixture().await;
    let raw_session = crate::crypto::token::generate_random_token();
    let now = chrono::Utc::now();
    let session = crate::models::session::Session {
        id: uuid::Uuid::new_v4().to_string(),
        user_id: f.actor.clone(),
        purpose: Some(crate::models::session::SessionPurpose::Authenticated),
        mfa_method: None,
        mfa_assured: Some(false),
        mfa_exemption: None,
        token_hash: crate::crypto::token::hash_token(&raw_session),
        ip_address: None,
        user_agent: None,
        expires_at: now + chrono::Duration::hours(1),
        revoked: false,
        created_at: now,
        last_active_at: now,
    };
    f.state
        .db
        .collection::<crate::models::session::Session>("sessions")
        .insert_one(session)
        .await
        .unwrap();
    let router = Router::new()
        .route("/users/{id}/verify-email", post(admin::verify_user_email))
        .with_state(f.state.clone());
    let response = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("/users/{}/verify-email", f.target))
                .header(
                    "cookie",
                    format!("{}={raw_session}", crate::mw::auth::SESSION_COOKIE_NAME),
                )
                .header("x-forwarded-for", "p".repeat(64 * 1024))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(
        response.status().is_success(),
        "oversized forwarded IP must not fail: {}",
        response.status()
    );
    let row = relayed_admin_event(&f, "admin.user.email_verified").await;
    assert_eq!(
        row.get_str("ip_address").unwrap(),
        format!("{}...[truncated]", "p".repeat(242))
    );
    assert!(
        f.state
            .db
            .collection::<User>("users")
            .find_one(doc! {"_id": &f.target})
            .await
            .unwrap()
            .unwrap()
            .email_verified
    );
    f.state.db.drop().await.unwrap();
}

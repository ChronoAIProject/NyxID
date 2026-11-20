use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use axum::{
    Json,
    extract::{Path, State},
    http::HeaderMap,
    response::IntoResponse,
};
use bson::{Document, doc};
use futures::{FutureExt, TryStreamExt};
use mongodb::event::{EventHandler, command::CommandEvent};

use crate::handlers::admin;
use crate::models::user::{User, UserType};
use crate::test_utils::{test_app_state, test_auth_user, test_user};

const INTENTS: &str = "security_effect_intents";
tokio::task_local! { static REQUEST: (); }

#[derive(Clone, Copy, Debug)]
enum Action {
    Create,
    Update,
    Role,
    Status,
    Password,
    Email,
    Sessions,
}

impl Action {
    const ALL: [Self; 7] = [
        Self::Create,
        Self::Update,
        Self::Role,
        Self::Status,
        Self::Password,
        Self::Email,
        Self::Sessions,
    ];

    fn event(self) -> &'static str {
        match self {
            Self::Create => "admin.user.created",
            Self::Update => "admin.user.updated",
            Self::Role => "admin.user.role_changed",
            Self::Status => "admin.user.status_changed",
            Self::Password => "admin.user.password_reset",
            Self::Email => "admin.user.email_verified",
            Self::Sessions => "admin.user.sessions_revoked",
        }
    }
}

#[derive(Debug)]
struct ProcessStopped;

#[derive(Default)]
struct Monitor {
    stop: bool,
    transaction: Option<i64>,
    completions: HashSet<i32>,
    commands: Vec<(String, String)>,
}

struct Fixture {
    state: crate::AppState,
    actor: String,
    target: String,
    monitor: Arc<Mutex<Monitor>>,
}

async fn fixture() -> Fixture {
    let monitor = Arc::new(Mutex::new(Monitor::default()));
    let seen = monitor.clone();
    let db = crate::test_utils::connect_transaction_test_database_with_command_handler(
        "admin_audit",
        EventHandler::callback(move |event| {
            if REQUEST.try_with(|_| ()).is_err() {
                return;
            }
            let mut seen = seen.lock().unwrap();
            match event {
                CommandEvent::Started(e) => {
                    let collection = e.command.get_str(&e.command_name).unwrap_or("").to_owned();
                    seen.commands
                        .push((e.command_name.clone(), collection.clone()));
                    if !seen.stop {
                        return;
                    }
                    if matches!(
                        e.command_name.as_str(),
                        "insert" | "update" | "findAndModify"
                    ) && collection == "users"
                    {
                        if e.command.get_bool("autocommit") == Ok(false) {
                            seen.transaction = e.command.get_i64("txnNumber").ok();
                        } else {
                            seen.completions.insert(e.request_id);
                        }
                    } else if e.command_name == "commitTransaction"
                        && seen.transaction.is_some()
                        && e.command.get_i64("txnNumber").ok() == seen.transaction
                    {
                        seen.completions.insert(e.request_id);
                    }
                }
                CommandEvent::Succeeded(e)
                    if seen.stop && seen.completions.remove(&e.request_id) =>
                {
                    seen.stop = false;
                    drop(seen);
                    std::panic::resume_unwind(Box::new(ProcessStopped));
                }
                _ => {}
            }
        }),
    )
    .await;
    crate::services::role_service::seed_system_roles(&db)
        .await
        .unwrap();
    let roles = crate::services::role_service::get_platform_role_ids(&db)
        .await
        .unwrap();
    let actor = uuid::Uuid::new_v4().to_string();
    let target = uuid::Uuid::new_v4().to_string();
    let mut admin = test_user(&actor, UserType::Person);
    admin.role_ids = vec![roles.admin];
    let mut user = test_user(&target, UserType::Person);
    user.email_verified = false;
    user.password_hash = Some("fixture-hash-never-exported".into());
    db.collection::<User>("users")
        .insert_many([admin, user])
        .await
        .unwrap();
    db.collection::<Document>("sessions")
        .insert_one(
            doc! {"_id":"session", "user_id":&target, "revoked":false, "purpose":"authenticated"},
        )
        .await
        .unwrap();
    crate::services::security_effect_service::ensure_indexes(&db)
        .await
        .unwrap();
    Fixture {
        state: test_app_state(db),
        actor,
        target,
        monitor,
    }
}

fn audit_auth(actor: &str) -> crate::mw::auth::AuthUser {
    let mut auth = test_auth_user(actor);
    auth.ip_address = Some("192.0.2.10".into());
    // Longer than the foundation's per-string shortening: retain the existing
    // actor context while bounding the entire occurrence instead.
    auth.user_agent = Some(format!("NyxID-Test/1.0 {}", "x".repeat(300)));
    auth.api_key_id = Some("fixture-key-id".into());
    auth.api_key_name = Some("Administrator test key".into());
    auth
}

async fn request(f: &Fixture, action: Action) -> axum::response::Response {
    let state = State(f.state.clone());
    let auth = audit_auth(&f.actor);
    let headers = HeaderMap::new();
    let target = Path(f.target.clone());
    match action {
        Action::Create => admin::create_user(
            state,
            auth,
            headers,
            Json(admin::CreateUserRequest {
                email: format!("{}@example.test", uuid::Uuid::new_v4()),
                password: crate::crypto::token::generate_random_token(),
                display_name: Some("Created user".into()),
                role: "user".into(),
            }),
        )
        .await
        .into_response(),
        Action::Update => admin::update_user(
            state,
            auth,
            headers,
            target,
            Json(admin::UpdateUserRequest {
                display_name: Some("Changed profile".into()),
                email: None,
                avatar_url: Some("https://example.test/avatar?opaque=must-not-enter-intent".into()),
            }),
        )
        .await
        .into_response(),
        Action::Role => admin::set_user_role(
            state,
            auth,
            headers,
            target,
            Json(admin::SetRoleRequest {
                role: Some("operator".into()),
                is_admin: Some(true),
            }),
        )
        .await
        .into_response(),
        Action::Status => admin::set_user_status(
            state,
            auth,
            crate::telemetry::TelemetryContext::default(),
            headers,
            target,
            Json(admin::SetStatusRequest { is_active: true }),
        )
        .await
        .into_response(),
        Action::Password => admin::force_password_reset(state, auth, headers, target)
            .await
            .into_response(),
        Action::Email => admin::verify_user_email(state, auth, headers, target)
            .await
            .into_response(),
        Action::Sessions => admin::revoke_user_sessions(state, auth, headers, target)
            .await
            .into_response(),
    }
}

async fn crash_after_commit(action: Action) {
    let f = fixture().await;
    f.monitor.lock().unwrap().stop = true;
    let request = REQUEST.scope((), request(&f, action));
    let stopped = std::panic::AssertUnwindSafe(Box::pin(request))
        .catch_unwind()
        .await;
    assert!(stopped.unwrap_err().is::<ProcessStopped>());
    let db = &f.state.db;
    let user = db
        .collection::<Document>("users")
        .find_one(doc! {"_id": &f.target})
        .await
        .unwrap()
        .unwrap();
    match action {
        Action::Create => assert_eq!(
            db.collection::<Document>("users")
                .count_documents(doc! {"display_name":"Created user"})
                .await
                .unwrap(),
            1
        ),
        Action::Update => assert_eq!(user.get_str("display_name").unwrap(), "Changed profile"),
        Action::Role => assert!(user.get_bool("is_operator").unwrap()),
        Action::Status => assert_eq!(
            user.get_i64("platform_personal_effect_revision").unwrap(),
            1
        ),
        Action::Password => assert!(user.get_str("password_reset_token").is_ok()),
        Action::Email => assert!(user.get_bool("email_verified").unwrap()),
        Action::Sessions => assert_eq!(
            db.collection::<Document>("sessions")
                .count_documents(doc! {"user_id":&f.target, "revoked":false})
                .await
                .unwrap(),
            0
        ),
    }
    tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    assert_eq!(
        db.collection::<Document>(crate::models::audit_log::COLLECTION_NAME)
            .count_documents(doc! {"event_type":action.event()})
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        db.collection::<Document>(INTENTS)
            .count_documents(doc! {"event.event_type":action.event()})
            .await
            .unwrap(),
        1,
        "committed {:?} must retain audit evidence after process death",
        action
    );
    crate::services::security_effect_service::sweep(db, &f.state.encryption_keys)
        .await
        .unwrap();
    crate::services::security_effect_service::sweep(db, &f.state.encryption_keys)
        .await
        .unwrap();
    assert_eq!(
        db.collection::<Document>(crate::models::audit_log::COLLECTION_NAME)
            .count_documents(doc! {"event_type":action.event()})
            .await
            .unwrap(),
        1
    );
    db.drop().await.unwrap();
}

macro_rules! crash_test {
    ($name:ident, $action:ident) => {
        #[tokio::test]
        async fn $name() {
            crash_after_commit(Action::$action).await;
        }
    };
}
crash_test!(admin_create_audit_survives_process_death, Create);
crash_test!(admin_update_audit_survives_process_death, Update);
crash_test!(admin_role_audit_survives_process_death, Role);
crash_test!(admin_status_audit_survives_process_death, Status);
crash_test!(admin_password_audit_survives_process_death, Password);
crash_test!(admin_email_audit_survives_process_death, Email);
crash_test!(admin_sessions_audit_survives_process_death, Sessions);

#[tokio::test]
async fn admin_audit_request_command_counts() {
    for action in Action::ALL {
        let f = fixture().await;
        let started = std::time::Instant::now();
        let response = REQUEST.scope((), request(&f, action)).await;
        assert!(
            response.status().is_success(),
            "{action:?}: {}",
            response.status()
        );
        let commands = f.monitor.lock().unwrap().commands.clone();
        assert_eq!(
            commands.len(),
            match action {
                // One live-person lookup and one read for each schedule timing.
                Action::Create => 15,
                Action::Role => 12,
                Action::Sessions => 13,
                Action::Update => 10,
                Action::Status => 13,
                Action::Password => 15,
                Action::Email => 7,
            },
            "{action:?}: {commands:?}"
        );
        println!(
            "ADMIN_COMMANDS {action:?} total={} elapsed_us={} commands={commands:?}",
            commands.len(),
            started.elapsed().as_micros()
        );
        f.state.db.drop().await.unwrap();
    }
}

async fn json(response: axum::response::Response) -> serde_json::Value {
    serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), 65_536)
            .await
            .unwrap(),
    )
    .unwrap()
}

async fn snapshot(db: &mongodb::Database, collection: &str) -> Vec<Document> {
    db.collection::<Document>(collection)
        .find(doc! {})
        .sort(doc! {"_id":1})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap()
}

#[tokio::test]
async fn admin_rejected_intent_rolls_back_every_source() {
    for action in Action::ALL {
        let f = fixture().await;
        let db = &f.state.db;
        let users = snapshot(db, "users").await;
        let sessions = snapshot(db, "sessions").await;
        db.run_command(doc! {"collMod":INTENTS, "validator":{"forbidden":{"$exists":true}}})
            .await
            .unwrap();
        let response = request(&f, action).await;
        assert_eq!(
            response.status(),
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            "{action:?}"
        );
        let body = json(response).await;
        assert_eq!(body["error_code"], 1007, "{action:?}: {body}");
        assert_eq!(
            snapshot(db, "users").await,
            users,
            "{action:?}: users changed after rejected intent"
        );
        assert_eq!(
            snapshot(db, "sessions").await,
            sessions,
            "{action:?}: sessions changed after rejected intent"
        );
        assert_eq!(
            db.collection::<Document>(INTENTS)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
        db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn admin_intents_preserve_metadata_and_exclude_credentials() {
    for action in Action::ALL {
        let f = fixture().await;
        let response = request(&f, action).await;
        assert!(response.status().is_success());
        if matches!(action, Action::Update) {
            // Redaction does not alter the mutation or its response.
            let body = json(response).await;
            assert_eq!(body["display_name"], "Changed profile");
            assert_eq!(
                body["avatar_url"],
                "https://example.test/avatar?opaque=must-not-enter-intent"
            );
        }
        let rows = snapshot(&f.state.db, INTENTS).await;
        assert_eq!(rows.len(), 1);
        let row = &rows[0];
        assert!(bson::to_vec(row).unwrap().len() < 16 * 1024);
        let event = row.get_document("event").unwrap();
        assert_eq!(event.get_str("event_type").unwrap(), action.event());
        let metadata = event.get_document("event_data").unwrap();
        let mut keys: Vec<_> = metadata.keys().map(String::as_str).collect();
        keys.sort_unstable();
        let mut expected = match action {
            Action::Create => vec!["target_user_id", "target_email", "role"],
            Action::Update => vec!["target_user_id", "target_email", "changes"],
            Action::Role => vec!["target_user_id", "role"],
            Action::Status => vec!["target_user_id", "is_active"],
            Action::Password | Action::Email => vec!["target_user_id"],
            Action::Sessions => vec!["target_user_id", "revoked_count"],
        };
        expected.sort_unstable();
        assert_eq!(keys, expected, "{action:?}");
        let persisted = serde_json::to_string(row).unwrap();
        for prohibited in [
            "fixture-hash-never-exported",
            "must-not-enter-intent",
            "password_reset_token",
            "password_hash",
        ] {
            assert!(!persisted.contains(prohibited), "{action:?}: {prohibited}");
        }
        f.state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn admin_retry_responses_and_noop_audit_preserve_existing_contracts() {
    let f = fixture().await;
    for _ in 0..2 {
        let response = request(&f, Action::Role).await;
        assert!(response.status().is_success());
        let body = json(response).await;
        assert_eq!(body["role"], "operator");
        assert_eq!(body["is_admin"], false);
        assert_eq!(body["is_operator"], true);
    }
    assert!(request(&f, Action::Email).await.status().is_success());
    let repeat = request(&f, Action::Email).await;
    assert_eq!(repeat.status(), axum::http::StatusCode::BAD_REQUEST);
    assert_eq!(json(repeat).await["error_code"], 1000);
    for expected in [1, 0] {
        let response = request(&f, Action::Sessions).await;
        assert!(response.status().is_success());
        assert_eq!(json(response).await["revoked_count"], expected);
    }
    f.monitor.lock().unwrap().commands.clear();
    let noop = REQUEST
        .scope(
            (),
            admin::update_user(
                State(f.state.clone()),
                test_auth_user(&f.actor),
                HeaderMap::new(),
                Path(f.target.clone()),
                Json(admin::UpdateUserRequest {
                    display_name: None,
                    email: None,
                    avatar_url: None,
                }),
            ),
        )
        .await
        .unwrap();
    assert_eq!(noop.0.id, f.target);
    let commands = f.monitor.lock().unwrap().commands.clone();
    assert_eq!(commands.len(), 7, "no-op: {commands:?}");
    assert!(
        !commands
            .iter()
            .any(|(name, _)| matches!(name.as_str(), "update" | "commitTransaction"))
    );
    assert_eq!(
        commands
            .iter()
            .filter(|(name, collection)| name == "insert" && collection == INTENTS)
            .count(),
        1
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn admin_shared_session_revocation_adds_no_commands_or_audit() {
    let f = fixture().await;
    assert_eq!(
        REQUEST
            .scope((), super::revoke_all_user_sessions(&f.state.db, &f.target))
            .await
            .unwrap(),
        1
    );
    let commands = f.monitor.lock().unwrap().commands.clone();
    assert_eq!(commands.len(), 8, "shared revocation: {commands:?}");
    assert_eq!(
        f.state
            .db
            .collection::<Document>(INTENTS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    f.state.db.drop().await.unwrap();
}

// These constructors reproduce the detached handler metadata on the foundation
// base. Compare real chained rows, not just the intent's selected keys.
async fn audit_content_matches_legacy(action: Action) {
    use crate::models::audit_log::{AuditLog, COLLECTION_NAME};
    use crate::services::{audit_service, security_effect_service};
    let f = fixture().await;
    let response = request(&f, action).await;
    assert!(response.status().is_success());
    let body = json(response).await;
    let legacy_metadata = match action {
        Action::Create => serde_json::json!({
            "target_user_id": body["id"], "target_email": body["email"], "role": body["role"],
        }),
        Action::Update => serde_json::json!({
            "target_user_id": f.target, "target_email": body["email"],
            "changes": {"display_name": "Changed profile", "email": null,
                "avatar_url": "https://example.test/avatar?opaque=must-not-enter-intent"},
        }),
        Action::Role => serde_json::json!({"target_user_id": f.target, "role": body["role"]}),
        Action::Status => serde_json::json!({"target_user_id": f.target, "is_active": true}),
        Action::Password | Action::Email => serde_json::json!({"target_user_id": f.target}),
        Action::Sessions => {
            serde_json::json!({"target_user_id": f.target, "revoked_count": body["revoked_count"]})
        }
    };
    let written = audit_service::notify_on_audit_write_for_user(action.event(), &f.actor);
    audit_service::log_for_user(
        f.state.db.clone(),
        &audit_auth(&f.actor),
        action.event(),
        Some(legacy_metadata),
    );
    let old_id = tokio::time::timeout(std::time::Duration::from_secs(5), written)
        .await
        .unwrap()
        .unwrap();
    let collection = f.state.db.collection::<AuditLog>(COLLECTION_NAME);
    let old = collection
        .find_one(doc! {"_id": &old_id})
        .await
        .unwrap()
        .unwrap();
    let mut expected = old.event_data.unwrap();
    if matches!(action, Action::Update) {
        // Sole difference for this request: a URL query can contain credentials.
        expected["changes"]["avatar_url"] = "https://example.test/avatar".into();
    }
    security_effect_service::sweep(&f.state.db, &f.state.encryption_keys)
        .await
        .unwrap();
    let new = collection
        .find_one(doc! {"event_type": action.event(), "_id": {"$ne":old_id}})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        new.event_data,
        Some(expected),
        "{action:?}: legacy audit facts changed"
    );
    assert_eq!(new.user_id, old.user_id);
    assert_eq!(new.api_key_id, old.api_key_id);
    assert_eq!(new.api_key_name, old.api_key_name);
    assert_eq!(new.ip_address, old.ip_address);
    assert_eq!(new.user_agent, old.user_agent);
    f.state.db.drop().await.unwrap();
}
macro_rules! content_test {
    ($name:ident, $action:ident) => {
        #[tokio::test]
        async fn $name() {
            audit_content_matches_legacy(Action::$action).await;
        }
    };
}
content_test!(admin_create_audit_content_matches_legacy, Create);
content_test!(admin_update_audit_content_matches_legacy, Update);
content_test!(admin_role_audit_content_matches_legacy, Role);
content_test!(admin_status_audit_content_matches_legacy, Status);
content_test!(admin_password_audit_content_matches_legacy, Password);
content_test!(admin_email_audit_content_matches_legacy, Email);
content_test!(admin_sessions_audit_content_matches_legacy, Sessions);

#[tokio::test]
async fn admin_profile_audit_preserves_values_with_only_url_credential_redactions() {
    for (avatar, expected) in [
        (None, None),
        (Some(""), Some("")),
        (
            Some("  https://EXAMPLE.test:443/a%20b  "),
            Some("  https://EXAMPLE.test:443/a%20b  "),
        ),
        (
            Some("https://example.test/a?token=secret#fragment"),
            Some("https://example.test/a"),
        ),
        (
            Some("https://example.test/a#access_token=secret"),
            Some("https://example.test/a"),
        ),
        (
            Some("https://user:secret@example.test/a"),
            Some("https://example.test/a"),
        ),
        (
            Some("https://bad host/a?token=secret"),
            Some("https://bad host/a"),
        ),
    ] {
        let f = fixture().await;
        let response = admin::update_user(
            State(f.state.clone()),
            test_auth_user(&f.actor),
            HeaderMap::new(),
            Path(f.target.clone()),
            Json(admin::UpdateUserRequest {
                display_name: Some("  Submitted Name  ".into()),
                email: Some("Updated@Example.test".into()),
                avatar_url: avatar.map(str::to_owned),
            }),
        )
        .await
        .unwrap();
        assert_eq!(response.0.display_name.as_deref(), Some("Submitted Name"));
        assert_eq!(response.0.email, "updated@example.test");
        let rows = snapshot(&f.state.db, INTENTS).await;
        let metadata = rows[0]
            .get_document("event")
            .unwrap()
            .get_document("event_data")
            .unwrap();
        assert_eq!(
            bson::from_document::<serde_json::Value>(metadata.clone()).unwrap(),
            serde_json::json!({
                "target_user_id": f.target, "target_email": "updated@example.test",
                "changes": {"display_name": "  Submitted Name  ", "email": "Updated@Example.test", "avatar_url": expected},
            })
        );
        f.state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn admin_profile_padded_name_succeeds_with_marked_audit_value() {
    let f = fixture().await;
    let submitted = format!("{}Name", " ".repeat(17 * 1024));
    let Json(response) = admin::update_user(
        State(f.state.clone()),
        test_auth_user(&f.actor),
        HeaderMap::new(),
        Path(f.target.clone()),
        Json(admin::UpdateUserRequest {
            display_name: Some(submitted.clone()),
            email: None,
            avatar_url: None,
        }),
    )
    .await
    .expect("accepted padded display name must not fail the audit bound");
    assert_eq!(response.display_name.as_deref(), Some("Name"));
    let user = f
        .state
        .db
        .collection::<User>("users")
        .find_one(doc! {"_id": &f.target})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(user.display_name.as_deref(), Some("Name"));
    crate::services::security_effect_service::sweep(&f.state.db, &f.state.encryption_keys)
        .await
        .unwrap();
    let row = f
        .state
        .db
        .collection::<Document>("audit_log")
        .find_one(doc! {"event_type":"admin.user.updated"})
        .await
        .unwrap()
        .unwrap();
    let stored = row
        .get_document("event_data")
        .unwrap()
        .get_document("changes")
        .unwrap()
        .get_str("display_name")
        .unwrap();
    assert_eq!(stored, format!("{}...[truncated]", " ".repeat(498)));
    f.state.db.drop().await.unwrap();
}

#[path = "audit_tests_round2.rs"]
mod review_round2;

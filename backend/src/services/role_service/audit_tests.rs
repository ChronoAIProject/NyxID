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

use crate::handlers::admin_roles as handlers;
use crate::models::{
    role::Role,
    user::{User, UserType},
};
use crate::services::{audit_service, security_effect_service};
use crate::test_utils::{test_app_state, test_auth_user, test_user};

const INTENTS: &str = "security_effect_intents";
tokio::task_local! { static REQUEST: (); }

#[derive(Clone, Copy, Debug)]
enum Action {
    Create,
    Update,
    Delete,
    Assign,
    Revoke,
    Bulk,
}
impl Action {
    const ALL: [Self; 6] = [
        Self::Create,
        Self::Update,
        Self::Delete,
        Self::Assign,
        Self::Revoke,
        Self::Bulk,
    ];
    fn event(self) -> &'static str {
        match self {
            Self::Create => "admin.role.created",
            Self::Update => "admin.role.updated",
            Self::Delete => "admin.role.deleted",
            Self::Assign => "admin.role.assigned",
            Self::Revoke => "admin.role.revoked",
            Self::Bulk => "admin.role.bulk_assigned",
        }
    }
    fn source_command(self) -> (&'static str, &'static str) {
        match self {
            Self::Create => ("insert", "roles"),
            Self::Update => ("update", "roles"),
            Self::Delete => ("delete", "roles"),
            _ => ("update", "users"),
        }
    }
}
#[derive(Debug)]
struct ProcessStopped;
#[derive(Default)]
struct Monitor {
    stop: Option<(&'static str, &'static str)>,
    transaction: Option<i64>,
    completions: HashSet<i32>,
    commands: Vec<(String, String)>,
}
struct Fixture {
    state: crate::AppState,
    actor: String,
    target: String,
    role: String,
    monitor: Arc<Mutex<Monitor>>,
}

async fn fixture(action: Action) -> Fixture {
    let monitor = Arc::new(Mutex::new(Monitor::default()));
    let seen = monitor.clone();
    let db = crate::test_utils::connect_transaction_test_database_with_command_handler(
        "role_audit",
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
                    let Some(source) = seen.stop else {
                        return;
                    };
                    if (e.command_name.as_str(), collection.as_str()) == source {
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
                    if seen.stop.is_some() && seen.completions.remove(&e.request_id) =>
                {
                    seen.stop = None;
                    drop(seen);
                    std::panic::resume_unwind(Box::new(ProcessStopped));
                }
                _ => {}
            }
        }),
    )
    .await;
    super::seed_system_roles(&db).await.unwrap();
    let roles = super::get_platform_role_ids(&db).await.unwrap();
    let actor = uuid::Uuid::new_v4().to_string();
    let target = uuid::Uuid::new_v4().to_string();
    let role = uuid::Uuid::new_v4().to_string();
    let mut admin = test_user(&actor, UserType::Person);
    admin.role_ids = vec![roles.admin];
    let mut user = test_user(&target, UserType::Person);
    if matches!(action, Action::Delete | Action::Revoke) {
        user.role_ids.push(role.clone());
    }
    db.collection::<User>("users")
        .insert_many([admin, user])
        .await
        .unwrap();
    let now = chrono::Utc::now();
    db.collection::<Role>("roles")
        .insert_one(Role {
            id: role.clone(),
            name: "Original".into(),
            slug: "audit-role".into(),
            description: None,
            permissions: vec!["read".into()],
            is_default: false,
            is_system: false,
            client_id: None,
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();
    db.collection::<Document>("groups")
        .insert_one(doc! {"_id":"group", "role_ids":[&role]})
        .await
        .unwrap();
    security_effect_service::ensure_indexes(&db).await.unwrap();
    Fixture {
        state: test_app_state(db),
        actor,
        target,
        role,
        monitor,
    }
}
fn audit_auth(actor: &str) -> crate::mw::auth::AuthUser {
    let mut auth = test_auth_user(actor);
    auth.ip_address = Some("192.0.2.10".into());
    auth.user_agent = Some(format!("NyxID-Test/1.0 {}", "x".repeat(300)));
    auth.api_key_id = Some("fixture-key-id".into());
    auth.api_key_name = Some("Administrator test key".into());
    auth
}
async fn request(f: &Fixture, action: Action) -> axum::response::Response {
    let state = State(f.state.clone());
    let auth = audit_auth(&f.actor);
    let headers = HeaderMap::new();
    match action {
        Action::Create => handlers::create_role(
            state,
            auth,
            headers,
            Json(handlers::CreateRoleRequest {
                name: "Created role".into(),
                slug: "created-role".into(),
                description: Some("Description".into()),
                permissions: vec!["read".into()],
                is_default: Some(false),
                client_id: None,
            }),
        )
        .await
        .into_response(),
        Action::Update => handlers::update_role(
            state,
            auth,
            headers,
            Path(f.role.clone()),
            Json(handlers::UpdateRoleRequest {
                name: Some("Updated role".into()),
                slug: Some("updated-role".into()),
                description: None,
                permissions: Some(vec!["write".into()]),
                is_default: Some(true),
            }),
        )
        .await
        .into_response(),
        Action::Delete => handlers::delete_role(state, auth, headers, Path(f.role.clone()))
            .await
            .into_response(),
        Action::Assign => handlers::assign_role(
            state,
            auth,
            headers,
            Path((f.target.clone(), f.role.clone())),
        )
        .await
        .into_response(),
        Action::Revoke => handlers::revoke_role(
            state,
            auth,
            headers,
            Path((f.target.clone(), f.role.clone())),
        )
        .await
        .into_response(),
        Action::Bulk => handlers::bulk_assign_role(
            state,
            auth,
            headers,
            Path(f.role.clone()),
            Json(handlers::BulkAssignRequest {
                all: false,
                user_ids: vec![f.target.clone()],
            }),
        )
        .await
        .into_response(),
    }
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
async fn json(response: axum::response::Response) -> serde_json::Value {
    serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), 65_536)
            .await
            .unwrap(),
    )
    .unwrap()
}
async fn crash_after_commit(action: Action) {
    let f = fixture(action).await;
    f.monitor.lock().unwrap().stop = Some(action.source_command());
    let stopped = std::panic::AssertUnwindSafe(Box::pin(REQUEST.scope((), request(&f, action))))
        .catch_unwind()
        .await;
    assert!(stopped.unwrap_err().is::<ProcessStopped>());
    let db = &f.state.db;
    match action {
        Action::Create => assert_eq!(
            db.collection::<Document>("roles")
                .count_documents(doc! {"slug":"created-role"})
                .await
                .unwrap(),
            1
        ),
        Action::Update => assert_eq!(
            db.collection::<Document>("roles")
                .find_one(doc! {"_id":&f.role})
                .await
                .unwrap()
                .unwrap()
                .get_str("slug")
                .unwrap(),
            "updated-role"
        ),
        Action::Delete => {
            assert_eq!(
                db.collection::<Document>("roles")
                    .count_documents(doc! {"_id":&f.role})
                    .await
                    .unwrap(),
                0
            );
            for collection in ["users", "groups"] {
                assert_eq!(
                    db.collection::<Document>(collection)
                        .count_documents(doc! {"role_ids":&f.role})
                        .await
                        .unwrap(),
                    0
                );
            }
        }
        Action::Assign | Action::Bulk | Action::Revoke => assert_eq!(
            db.collection::<Document>("users")
                .count_documents(doc! {"_id":&f.target,"role_ids":&f.role})
                .await
                .unwrap(),
            if matches!(action, Action::Revoke) {
                0
            } else {
                1
            }
        ),
    }
    tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    assert_eq!(
        db.collection::<Document>("audit_log")
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
        "committed {action:?} lost its audit evidence"
    );
    security_effect_service::sweep(db, &f.state.encryption_keys)
        .await
        .unwrap();
    security_effect_service::sweep(db, &f.state.encryption_keys)
        .await
        .unwrap();
    assert_eq!(
        db.collection::<Document>("audit_log")
            .count_documents(doc! {"event_type":action.event()})
            .await
            .unwrap(),
        1
    );
    db.drop().await.unwrap();
}
macro_rules! crash_test {
    ($name:ident,$action:ident) => {
        #[tokio::test]
        async fn $name() {
            crash_after_commit(Action::$action).await;
        }
    };
}
crash_test!(role_create_audit_survives_process_death, Create);
crash_test!(role_update_audit_survives_process_death, Update);
crash_test!(role_delete_audit_survives_process_death, Delete);
crash_test!(role_assign_audit_survives_process_death, Assign);
crash_test!(role_revoke_audit_survives_process_death, Revoke);
crash_test!(role_bulk_audit_survives_process_death, Bulk);
#[tokio::test]
async fn role_audit_request_command_counts() {
    for action in Action::ALL {
        let f = fixture(action).await;
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
                Action::Create => 7,
                Action::Update | Action::Delete => 9,
                Action::Assign | Action::Bulk => 8,
                Action::Revoke => 6,
            },
            "{action:?}: {commands:?}"
        );
        println!(
            "ROLE_COMMANDS {action:?} total={} commands={commands:?}",
            commands.len()
        );
        f.state.db.drop().await.unwrap();
    }
}

async fn audit_content_matches_legacy(action: Action) {
    use crate::models::audit_log::{AuditLog, COLLECTION_NAME};
    let f = fixture(action).await;
    let response = request(&f, action).await;
    assert!(response.status().is_success());
    let body = json(response).await;
    // Verbatim fields from each former handler-side log_for_user constructor.
    let metadata = match action {
        Action::Create => serde_json::json!({"role_id":body["id"],"role_slug":body["slug"]}),
        Action::Update => serde_json::json!({"role_id":f.role,"role_slug":body["slug"]}),
        Action::Delete => serde_json::json!({"role_id":f.role}),
        Action::Assign | Action::Revoke => {
            serde_json::json!({"target_user_id":f.target,"role_id":f.role})
        }
        Action::Bulk => serde_json::json!({"role_id":f.role,"all":false,"user_ids_count":1,
            "assigned_count":body["assigned_count"],"already_assigned_count":body["already_assigned_count"]}),
    };
    let written = audit_service::notify_on_audit_write_for_user(action.event(), &f.actor);
    audit_service::log_for_user(
        f.state.db.clone(),
        &audit_auth(&f.actor),
        action.event(),
        Some(metadata),
    );
    let old_id = tokio::time::timeout(std::time::Duration::from_secs(5), written)
        .await
        .unwrap()
        .unwrap();
    let collection = f.state.db.collection::<AuditLog>(COLLECTION_NAME);
    let old = collection
        .find_one(doc! {"_id":&old_id})
        .await
        .unwrap()
        .unwrap();
    security_effect_service::sweep(&f.state.db, &f.state.encryption_keys)
        .await
        .unwrap();
    let new = collection
        .find_one(doc! {"event_type":action.event(),"_id":{"$ne":old_id}})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        new.event_data, old.event_data,
        "{action:?}: no intentional metadata differences"
    );
    assert_eq!(new.user_id, old.user_id);
    assert_eq!(new.api_key_id, old.api_key_id);
    assert_eq!(new.api_key_name, old.api_key_name);
    assert_eq!(new.ip_address, old.ip_address);
    assert_eq!(new.user_agent, old.user_agent);
    f.state.db.drop().await.unwrap();
}
macro_rules! content_test {
    ($name:ident,$action:ident) => {
        #[tokio::test]
        async fn $name() {
            audit_content_matches_legacy(Action::$action).await;
        }
    };
}
content_test!(role_create_audit_content_matches_legacy, Create);
content_test!(role_update_audit_content_matches_legacy, Update);
content_test!(role_delete_audit_content_matches_legacy, Delete);
content_test!(role_assign_audit_content_matches_legacy, Assign);
content_test!(role_revoke_audit_content_matches_legacy, Revoke);
content_test!(role_bulk_audit_content_matches_legacy, Bulk);

#[tokio::test]
async fn role_rejected_intent_rolls_back_every_source() {
    for action in Action::ALL {
        let f = fixture(action).await;
        let db = &f.state.db;
        let mut before = Vec::new();
        for collection in ["roles", "users", "groups"] {
            before.push(snapshot(db, collection).await);
        }
        db.run_command(doc! {"collMod":INTENTS,"validator":{"forbidden":{"$exists":true}}})
            .await
            .unwrap();
        let response = request(&f, action).await;
        assert_eq!(
            response.status(),
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            "{action:?}"
        );
        assert_eq!(json(response).await["error_code"], 1007);
        for (index, collection) in ["roles", "users", "groups"].into_iter().enumerate() {
            assert_eq!(
                snapshot(db, collection).await,
                before[index],
                "{action:?}: {collection}"
            );
        }
        assert!(snapshot(db, INTENTS).await.is_empty());
        db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn role_retry_errors_and_noops_preserve_existing_contracts() {
    let f = fixture(Action::Assign).await;
    assert!(request(&f, Action::Assign).await.status().is_success());
    let repeated = request(&f, Action::Assign).await;
    assert_eq!(repeated.status(), axum::http::StatusCode::CONFLICT);
    assert_eq!(snapshot(&f.state.db, INTENTS).await.len(), 1);
    for _ in 0..2 {
        assert!(request(&f, Action::Revoke).await.status().is_success());
    }
    // Revoking an unknown role from an unknown user is still a successful noop.
    let noop = handlers::revoke_role(
        State(f.state.clone()),
        test_auth_user(&f.actor),
        HeaderMap::new(),
        Path((
            uuid::Uuid::new_v4().to_string(),
            uuid::Uuid::new_v4().to_string(),
        )),
    )
    .await;
    assert!(noop.is_ok());
    for (assigned, already) in [(1, 0), (0, 1)] {
        let response = request(&f, Action::Bulk).await;
        assert!(response.status().is_success());
        let body = json(response).await;
        assert_eq!(body["assigned_count"], assigned);
        assert_eq!(body["already_assigned_count"], already);
    }
    assert!(request(&f, Action::Delete).await.status().is_success());
    let repeated = request(&f, Action::Delete).await;
    assert_eq!(repeated.status(), axum::http::StatusCode::NOT_FOUND);
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn role_bulk_all_audit_preserves_actual_counts_and_request_shape() {
    let f = fixture(Action::Assign).await;
    for (assigned, already) in [(2, 0), (0, 2)] {
        let result = handlers::bulk_assign_role(
            State(f.state.clone()),
            test_auth_user(&f.actor),
            HeaderMap::new(),
            Path(f.role.clone()),
            Json(handlers::BulkAssignRequest {
                all: true,
                user_ids: vec![],
            }),
        )
        .await
        .unwrap();
        assert_eq!(result.0.assigned_count, assigned);
        assert_eq!(result.0.already_assigned_count, already);
        if assigned == 0 {
            assert_eq!(
                snapshot(&f.state.db, INTENTS).await.len(),
                1,
                "no audit for zero bulk changes"
            );
            continue;
        }
        let row = f.state.db.collection::<Document>(INTENTS).find_one(doc!{"event.event_type":"admin.role.bulk_assigned","event.event_data.assigned_count":assigned as i64}).await.unwrap().unwrap();
        assert_eq!(
            bson::from_document::<serde_json::Value>(
                row.get_document("event")
                    .unwrap()
                    .get_document("event_data")
                    .unwrap()
                    .clone()
            )
            .unwrap(),
            serde_json::json!({
                "role_id":f.role,"all":true,"user_ids_count":0,"assigned_count":assigned,"already_assigned_count":already,
            })
        );
    }
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn role_reads_add_no_commands_or_intents() {
    let f = fixture(Action::Assign).await;
    let result = REQUEST
        .scope(
            (),
            handlers::get_role(
                State(f.state.clone()),
                test_auth_user(&f.actor),
                Path(f.role.clone()),
            ),
        )
        .await
        .unwrap();
    assert_eq!(result.0.id, f.role);
    let commands = f.monitor.lock().unwrap().commands.clone();
    assert_eq!(commands.len(), 4, "role read: {commands:?}");
    assert!(commands.iter().all(|(name, _)| name == "find"));
    assert!(snapshot(&f.state.db, INTENTS).await.is_empty());
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn role_delete_secondary_write_failure_still_errors_without_partial_removal() {
    let f = fixture(Action::Delete).await;
    let users = snapshot(&f.state.db, "users").await;
    let roles = snapshot(&f.state.db, "roles").await;
    let groups = snapshot(&f.state.db, "groups").await;
    f.state
        .db
        .run_command(doc! {"collMod":"groups","validator":{"role_ids":&f.role}})
        .await
        .unwrap();
    let response = request(&f, Action::Delete).await;
    assert_eq!(
        response.status(),
        axum::http::StatusCode::INTERNAL_SERVER_ERROR
    );
    assert_eq!(json(response).await["error_code"], 1007);
    assert_eq!(snapshot(&f.state.db, "users").await, users);
    assert_eq!(snapshot(&f.state.db, "roles").await, roles);
    assert_eq!(snapshot(&f.state.db, "groups").await, groups);
    assert!(snapshot(&f.state.db, INTENTS).await.is_empty());
    f.state.db.drop().await.unwrap();
}

#[path = "audit_tests_safeguards.rs"]
mod safeguards;

#[path = "audit_tests_scale.rs"]
mod scale;

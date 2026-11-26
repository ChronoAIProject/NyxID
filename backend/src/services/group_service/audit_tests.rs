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

use crate::handlers::admin_groups as handlers;
use crate::models::{
    group::Group,
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
    Add,
    Remove,
}
impl Action {
    const ALL: [Self; 5] = [
        Self::Create,
        Self::Update,
        Self::Delete,
        Self::Add,
        Self::Remove,
    ];
    fn event(self) -> &'static str {
        match self {
            Self::Create => "admin.group.created",
            Self::Update => "admin.group.updated",
            Self::Delete => "admin.group.deleted",
            Self::Add => "admin.group.member_added",
            Self::Remove => "admin.group.member_removed",
        }
    }
    fn source_command(self) -> (&'static str, &'static str) {
        match self {
            Self::Create => ("insert", "groups"),
            Self::Update => ("update", "groups"),
            Self::Delete => ("delete", "groups"),
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
    group: String,
    monitor: Arc<Mutex<Monitor>>,
}

async fn fixture(action: Action) -> Fixture {
    let monitor = Arc::new(Mutex::new(Monitor::default()));
    let seen = monitor.clone();
    let db = crate::test_utils::connect_transaction_test_database_with_command_handler(
        "group_audit",
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
    crate::services::role_service::seed_system_roles(&db)
        .await
        .unwrap();
    let groups = crate::services::role_service::get_platform_role_ids(&db)
        .await
        .unwrap();
    let actor = uuid::Uuid::new_v4().to_string();
    let target = uuid::Uuid::new_v4().to_string();
    let group = uuid::Uuid::new_v4().to_string();
    let mut admin = test_user(&actor, UserType::Person);
    admin.role_ids = vec![groups.admin];
    let mut user = test_user(&target, UserType::Person);
    if matches!(action, Action::Delete | Action::Remove) {
        user.group_ids.push(group.clone());
    }
    db.collection::<User>("users")
        .insert_many([admin, user])
        .await
        .unwrap();
    let now = chrono::Utc::now();
    db.collection::<Group>("groups")
        .insert_one(Group {
            id: group.clone(),
            name: "Original".into(),
            slug: "audit-group".into(),
            description: None,
            role_ids: vec![],
            parent_group_id: None,
            created_at: now,
            updated_at: now,
        })
        .await
        .unwrap();
    security_effect_service::ensure_indexes(&db).await.unwrap();
    Fixture {
        state: test_app_state(db),
        actor,
        target,
        group,
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
        Action::Create => handlers::create_group(
            state,
            auth,
            headers,
            Json(handlers::CreateGroupRequest {
                name: "Created group".into(),
                slug: "created-group".into(),
                description: Some("Description".into()),
                role_ids: vec![],
                parent_group_id: None,
            }),
        )
        .await
        .into_response(),
        Action::Update => handlers::update_group(
            state,
            auth,
            headers,
            Path(f.group.clone()),
            Json(handlers::UpdateGroupRequest {
                name: Some("Updated group".into()),
                slug: Some("updated-group".into()),
                description: None,
                role_ids: None,
                parent_group_id: None,
            }),
        )
        .await
        .into_response(),
        Action::Delete => handlers::delete_group(state, auth, headers, Path(f.group.clone()))
            .await
            .into_response(),
        Action::Add => handlers::add_member(
            state,
            auth,
            headers,
            Path((f.group.clone(), f.target.clone())),
        )
        .await
        .into_response(),
        Action::Remove => handlers::remove_member(
            state,
            auth,
            headers,
            Path((f.group.clone(), f.target.clone())),
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
            db.collection::<Document>("groups")
                .count_documents(doc! {"slug":"created-group"})
                .await
                .unwrap(),
            1
        ),
        Action::Update => assert_eq!(
            db.collection::<Document>("groups")
                .find_one(doc! {"_id":&f.group})
                .await
                .unwrap()
                .unwrap()
                .get_str("slug")
                .unwrap(),
            "updated-group"
        ),
        Action::Delete => {
            assert_eq!(
                db.collection::<Document>("groups")
                    .count_documents(doc! {"_id":&f.group})
                    .await
                    .unwrap(),
                0
            );
            assert_eq!(
                db.collection::<Document>("users")
                    .count_documents(doc! {"group_ids":&f.group})
                    .await
                    .unwrap(),
                0
            );
        }
        Action::Add | Action::Remove => assert_eq!(
            db.collection::<Document>("users")
                .count_documents(doc! {"_id":&f.target,"group_ids":&f.group})
                .await
                .unwrap(),
            if matches!(action, Action::Remove) {
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
crash_test!(group_create_audit_survives_process_death, Create);
crash_test!(group_update_audit_survives_process_death, Update);
crash_test!(group_delete_audit_survives_process_death, Delete);
crash_test!(group_add_audit_survives_process_death, Add);
crash_test!(group_remove_audit_survives_process_death, Remove);
#[tokio::test]
async fn group_audit_request_command_counts() {
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
                Action::Create => 8,
                Action::Update => 10,
                Action::Delete => 9,
                Action::Add => 11,
                Action::Remove => 6,
            },
            "{action:?}: {commands:?}"
        );
        // The join checks live group membership and both schedule timings.
        assert_eq!(
            commands
                .iter()
                .filter(
                    |(command, collection)| command == "find" && collection == "credit_schedules"
                )
                .count(),
            usize::from(matches!(action, Action::Add))
        );
        println!(
            "GROUP_COMMANDS {action:?} total={} commands={commands:?}",
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
        Action::Create => serde_json::json!({"group_id":body["id"],"group_slug":body["slug"]}),
        Action::Update => serde_json::json!({"group_id":f.group,"group_slug":body["slug"]}),
        Action::Delete => serde_json::json!({"group_id":f.group}),
        Action::Add | Action::Remove => {
            serde_json::json!({"target_user_id":f.target,"group_id":f.group})
        }
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
content_test!(group_create_audit_content_matches_legacy, Create);
content_test!(group_update_audit_content_matches_legacy, Update);
content_test!(group_delete_audit_content_matches_legacy, Delete);
content_test!(group_add_audit_content_matches_legacy, Add);
content_test!(group_remove_audit_content_matches_legacy, Remove);

#[tokio::test]
async fn group_rejected_intent_rolls_back_every_source() {
    for action in Action::ALL {
        let f = fixture(action).await;
        let db = &f.state.db;
        let mut before = Vec::new();
        for collection in ["groups", "users"] {
            before.push(snapshot(db, collection).await);
        }
        db.run_command(doc! {"collMod":INTENTS,"validator":{"forbidden":{"$exists":true}}})
            .await
            .unwrap();
        let response = REQUEST.scope((), request(&f, action)).await;
        assert_eq!(
            response.status(),
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            "{action:?}"
        );
        assert_eq!(json(response).await["error_code"], 1007);
        let commands = f.monitor.lock().unwrap().commands.clone();
        let intent = commands
            .iter()
            .position(|(cmd, col)| cmd == "insert" && col == INTENTS)
            .unwrap();
        let source = action.source_command();
        let position = commands
            .iter()
            .position(|(cmd, col)| cmd == source.0 && col == source.1)
            .unwrap();
        assert!(
            position < intent,
            "source must succeed before rejected intent"
        );
        if matches!(action, Action::Delete) {
            assert!(
                commands[..position]
                    .iter()
                    .any(|(cmd, col)| cmd == "update" && col == "users")
            );
        }

        for (index, collection) in ["groups", "users"].into_iter().enumerate() {
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

#[path = "audit_tests_safeguards.rs"]
mod safeguards;

#[path = "audit_tests_scale.rs"]
mod scale;

#[tokio::test]
async fn group_read_commands_are_unchanged() {
    let f = fixture(Action::Add).await;
    let _ = REQUEST
        .scope(
            (),
            handlers::get_group(
                State(f.state.clone()),
                audit_auth(&f.actor),
                Path(f.group.clone()),
            ),
        )
        .await
        .unwrap();
    let commands = f.monitor.lock().unwrap().commands.clone();
    assert_eq!(commands.len(), 5, "{commands:?}");
    assert!(
        commands
            .iter()
            .all(|(name, _)| name == "find" || name == "aggregate")
    );
    assert!(snapshot(&f.state.db, INTENTS).await.is_empty());
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn group_membership_retries_keep_existing_responses() {
    let f = fixture(Action::Add).await;
    assert!(request(&f, Action::Add).await.status().is_success());
    let repeated = request(&f, Action::Add).await;
    assert_eq!(repeated.status().as_u16(), 409);
    assert_eq!(json(repeated).await["error_code"], 4004);
    assert_eq!(snapshot(&f.state.db, INTENTS).await.len(), 1);
    for _ in 0..2 {
        assert!(request(&f, Action::Remove).await.status().is_success());
    }
    assert_eq!(snapshot(&f.state.db, INTENTS).await.len(), 2);
    assert!(request(&f, Action::Delete).await.status().is_success());
    let repeated = request(&f, Action::Delete).await;
    assert_eq!(repeated.status().as_u16(), 404);
    assert_eq!(json(repeated).await["error_code"], 4001);
    f.state.db.drop().await.unwrap();
}

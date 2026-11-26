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

use crate::handlers::invite_codes as handlers;
use crate::models::{
    invite_code::InviteCode,
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
}
impl Action {
    const ALL: [Self; 3] = [Self::Create, Self::Update, Self::Delete];
    fn event(self) -> &'static str {
        match self {
            Self::Create => "admin_invite_code_create",
            Self::Update => "admin_invite_code_update",
            Self::Delete => "admin_invite_code_deactivate",
        }
    }
    fn source_command(self) -> (&'static str, &'static str) {
        match self {
            Self::Create => ("insert", "invite_codes"),
            Self::Update => ("findAndModify", "invite_codes"),
            Self::Delete => ("update", "invite_codes"),
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
    invite: String,
    monitor: Arc<Mutex<Monitor>>,
}

async fn fixture(_action: Action) -> Fixture {
    let monitor = Arc::new(Mutex::new(Monitor::default()));
    let seen = monitor.clone();
    let db = crate::test_utils::connect_transaction_test_database_with_command_handler(
        "invite_audit",
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
    let invites = crate::services::role_service::get_platform_role_ids(&db)
        .await
        .unwrap();
    let actor = uuid::Uuid::new_v4().to_string();
    let target = uuid::Uuid::new_v4().to_string();
    let invite = uuid::Uuid::new_v4().to_string();
    let mut admin = test_user(&actor, UserType::Person);
    admin.role_ids = vec![invites.admin];
    let user = test_user(&target, UserType::Person);
    db.collection::<User>("users")
        .insert_many([admin, user])
        .await
        .unwrap();
    let now = chrono::Utc::now();
    db.collection::<InviteCode>("invite_codes")
        .insert_one(InviteCode {
            id: invite.clone(),
            code: "NYX-FIXTURE".into(),
            max_uses: 10,
            used_count: 0,
            created_by: actor.clone(),
            note: Some("Old note".into()),
            is_active: true,
            created_at: now,
            updated_at: now,
            usages: vec![],
            reservations: vec![],
        })
        .await
        .unwrap();
    security_effect_service::ensure_indexes(&db).await.unwrap();
    Fixture {
        state: test_app_state(db),
        actor,
        invite,
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
        Action::Create => handlers::create_invite_code(
            state,
            axum::extract::ConnectInfo("127.0.0.1:12345".parse::<std::net::SocketAddr>().unwrap()),
            auth,
            crate::telemetry::TelemetryContext::default(),
            headers,
            Json(handlers::CreateInviteCodeRequest {
                max_uses: Some(5),
                note: Some("New note".into()),
            }),
        )
        .await
        .into_response(),
        Action::Update => handlers::update_invite_code(
            state,
            axum::extract::ConnectInfo("127.0.0.1:12345".parse::<std::net::SocketAddr>().unwrap()),
            auth,
            headers,
            Path(f.invite.clone()),
            Json(handlers::UpdateInviteCodeRequest {
                note: Some("Updated note".into()),
            }),
        )
        .await
        .into_response(),
        Action::Delete => handlers::deactivate_invite_code(
            state,
            axum::extract::ConnectInfo("127.0.0.1:12345".parse::<std::net::SocketAddr>().unwrap()),
            auth,
            headers,
            Path(f.invite.clone()),
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
    let rows = snapshot(db, "invite_codes").await;
    match action {
        Action::Create => assert_eq!(rows.len(), 2),
        Action::Update => assert!(rows.iter().any(|r| r.get_str("note") == Ok("Updated note"))),
        Action::Delete => assert!(rows.iter().any(|r| r.get_bool("is_active") == Ok(false))),
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
crash_test!(invite_create_audit_survives_process_death, Create);
crash_test!(invite_update_audit_survives_process_death, Update);
crash_test!(invite_delete_audit_survives_process_death, Delete);
#[tokio::test]
async fn invite_audit_request_command_counts() {
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
            if matches!(action, Action::Update) {
                7
            } else {
                6
            },
            "{action:?}: {commands:?}"
        );
        println!(
            "INVITE_COMMANDS {action:?} total={} commands={commands:?}",
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
    let mut metadata = match action {
        Action::Create => {
            serde_json::json!({"invite_code_id":body["id"],"code":body["code"],"max_uses":body["max_uses"]})
        }
        Action::Update => serde_json::json!({"invite_code_id":f.invite,"fields_changed":["note"]}),
        Action::Delete => serde_json::json!({"invite_code_id":f.invite}),
    };
    let written = audit_service::notify_on_audit_write_for_user(action.event(), &f.actor);
    audit_service::log_for_user(
        f.state.db.clone(),
        &audit_auth(&f.actor),
        action.event(),
        Some(metadata.clone()),
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
    assert_eq!(old.event_data, Some(metadata.clone()));
    if matches!(action, Action::Create) {
        metadata["code"] = "[REDACTED]".into();
    }
    assert_eq!(
        new.event_data,
        Some(metadata),
        "{action:?}: only the redeemable invite credential is redacted"
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
content_test!(invite_create_audit_content_matches_legacy, Create);
content_test!(invite_update_audit_content_matches_legacy, Update);
content_test!(invite_delete_audit_content_matches_legacy, Delete);

#[tokio::test]
async fn invite_rejected_intent_rolls_back_every_source() {
    for action in Action::ALL {
        let f = fixture(action).await;
        let db = &f.state.db;
        let mut before = Vec::new();
        for collection in ["invite_codes", "users"] {
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

        for (index, collection) in ["invite_codes", "users"].into_iter().enumerate() {
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
async fn missing_invite_writes_preserve_not_found_without_intents() {
    for action in [Action::Update, Action::Delete] {
        let f = fixture(action).await;
        f.state
            .db
            .collection::<Document>("invite_codes")
            .delete_one(doc! {"_id":&f.invite})
            .await
            .unwrap();
        let response = request(&f, action).await;
        assert_eq!(response.status().as_u16(), 404);
        assert_eq!(json(response).await["error_code"], 1003);
        assert!(snapshot(&f.state.db, INTENTS).await.is_empty());
        f.state.db.drop().await.unwrap();
    }
}
#[tokio::test]
async fn invite_capacity_path_command_counts_are_unchanged() {
    let f = fixture(Action::Update).await;
    let reservation = REQUEST
        .scope(
            (),
            super::reserve_invite_code(&f.state.db, "NYX-FIXTURE", "fixture@example.com"),
        )
        .await
        .unwrap();
    let commands = f.monitor.lock().unwrap().commands.clone();
    println!(
        "INVITE_RESERVE_COMMANDS total={} commands={commands:?}",
        commands.len()
    );
    assert_eq!(commands.len(), 1);
    f.monitor.lock().unwrap().commands.clear();
    REQUEST
        .scope((), super::release_reservation(&f.state.db, &reservation))
        .await;
    assert_eq!(f.monitor.lock().unwrap().commands.len(), 1);
    f.state.db.drop().await.unwrap();
}
#[tokio::test]
async fn maximum_invite_occurrences_fit_backstop() {
    use super::audit;
    use crate::{
        models::security_effect_intent::SecurityEffect,
        services::audit_service::{
            AuditActor,
            required::{TextField, text},
        },
    };
    let raw = "x".repeat(64 * 1024);
    let max = |field| text(field, &raw);
    let actor = AuditActor {
        user_id: max(TextField::Identifier),
        ip_address: Some(max(TextField::ForwardedIp)),
        user_agent: Some(max(TextField::UserAgent)),
        api_key_id: Some(max(TextField::Identifier)),
        api_key_name: Some(max(TextField::KeyName)),
    };
    for action in Action::ALL {
        let turn = uuid::Uuid::new_v4().to_string();
        let intent =
            crate::turn_trace::scope(crate::turn_trace::TurnContext::for_turn(&turn), async {
                audit::intent(
                    &actor,
                    action.event(),
                    &raw,
                    matches!(action, Action::Create).then_some(1000),
                )
            })
            .await
            .unwrap();
        let SecurityEffect::Audit { event } = intent.effect else {
            panic!("expected audit")
        };
        let bytes = bson::to_vec(&event).unwrap().len();
        assert!(bytes < 16 * 1024);
        let metadata = event.event_data.as_ref().unwrap();
        assert_eq!(metadata["invite_code_id"].as_str().unwrap().len(), 256);
        assert!(
            metadata["invite_code_id"]
                .as_str()
                .unwrap()
                .ends_with("...[truncated]")
        );
        println!(
            "INVITE_AUDIT_MAX {} bytes={bytes} margin={}",
            action.event(),
            16 * 1024 - bytes
        );
    }
}

#[tokio::test]
async fn invite_note_response_failure_keeps_committed_source_and_intent() {
    let f = fixture(Action::Update).await;
    f.state
        .db
        .collection::<Document>("users")
        .insert_one(doc! {"_id":"malformed-creator","email":null,"display_name":null})
        .await
        .unwrap();
    f.state
        .db
        .collection::<Document>("invite_codes")
        .update_one(
            doc! {"_id":&f.invite},
            doc! {"$set":{"created_by":"malformed-creator"}},
        )
        .await
        .unwrap();
    let response = request(&f, Action::Update).await;
    assert_eq!(response.status().as_u16(), 500);
    assert_eq!(json(response).await["error_code"], 1007);
    assert_eq!(
        f.state
            .db
            .collection::<Document>("invite_codes")
            .find_one(doc! {"_id":&f.invite})
            .await
            .unwrap()
            .unwrap()
            .get_str("note"),
        Ok("Updated note")
    );
    assert_eq!(snapshot(&f.state.db, INTENTS).await.len(), 1);
    f.state.db.drop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn invite_generation_collision_retries_without_orphan_intent() {
    use std::future::IntoFuture;
    use std::sync::atomic::{AtomicBool, Ordering};
    let mut f = fixture(Action::Create).await;
    let original = f.state.db.clone();
    original
        .collection::<Document>("invite_codes")
        .create_index(
            mongodb::IndexModel::builder()
                .keys(doc! {"code":1})
                .options(
                    mongodb::options::IndexOptions::builder()
                        .unique(true)
                        .build(),
                )
                .build(),
        )
        .await
        .unwrap();
    let fired = Arc::new(AtomicBool::new(false));
    let observed = fired.clone();
    let mut options =
        mongodb::options::ClientOptions::parse(std::env::var("NYXID_TEST_DATABASE_URL").unwrap())
            .await
            .unwrap();
    options.command_event_handler = Some(EventHandler::callback(move |event| {
        let CommandEvent::Started(event) = event else {
            return;
        };
        if event.command_name == "insert"
            && event.command.get_str("insert") == Ok("invite_codes")
            && !observed.swap(true, Ordering::SeqCst)
        {
            let mut row = event.command.get_array("documents").unwrap()[0]
                .as_document()
                .unwrap()
                .clone();
            row.insert("_id", "collision-winner");
            tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current().block_on(async {
                    tokio::time::timeout(
                        std::time::Duration::from_secs(5),
                        original
                            .collection::<Document>("invite_codes")
                            .insert_one(row)
                            .into_future(),
                    )
                    .await
                    .unwrap()
                    .unwrap();
                })
            });
        }
    }));
    f.state.db = mongodb::Client::with_options(options)
        .unwrap()
        .database(f.state.db.name());
    let response = request(&f, Action::Create).await;
    assert!(response.status().is_success());
    let body = json(response).await;
    assert!(fired.load(Ordering::SeqCst));
    let competitor = f
        .state
        .db
        .collection::<Document>("invite_codes")
        .find_one(doc! {"_id":"collision-winner"})
        .await
        .unwrap()
        .unwrap();
    assert_ne!(
        body["code"].as_str().unwrap(),
        competitor.get_str("code").unwrap()
    );
    assert_eq!(snapshot(&f.state.db, "invite_codes").await.len(), 3);
    let intents = snapshot(&f.state.db, INTENTS).await;
    assert_eq!(intents.len(), 1);
    assert_eq!(
        intents[0]
            .get_document("event")
            .unwrap()
            .get_document("event_data")
            .unwrap()
            .get_str("invite_code_id")
            .unwrap(),
        body["id"].as_str().unwrap()
    );
    f.state.db.drop().await.unwrap();
}

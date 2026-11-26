use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use axum::{
    extract::{Path, State},
    response::IntoResponse,
};
use bson::{Document, doc};
use futures::{FutureExt, TryStreamExt};
use mongodb::event::{EventHandler, command::CommandEvent};

use crate::handlers::orgs;
use crate::models::{
    org_membership::{OrgMembership, OrgRole},
    user::{User, UserType},
};
use crate::services::{audit_service, security_effect_service};
use crate::test_utils::{test_app_state, test_auth_user, test_membership, test_user};

const INTENTS: &str = "security_effect_intents";
const REQUESTED: &str = "org_deletion_requested";
const COMPLETED: &str = "org_deleted";
tokio::task_local! { static REQUEST: (); }

#[derive(Clone, Copy)]
enum Boundary {
    Start,
    Cleanup,
    Complete,
}
#[derive(Debug)]
struct ProcessStopped;
#[derive(Default)]
struct Monitor {
    units: retry::Units,
    stop: Option<Boundary>,
    transaction: Option<i64>,
    completions: HashSet<i32>,
    commands: Vec<(String, String)>,
}
struct Fixture {
    state: crate::AppState,
    actor: String,
    org: String,
    monitor: Arc<Mutex<Monitor>>,
}
async fn fixture() -> Fixture {
    let monitor = Arc::new(Mutex::new(Monitor::default()));
    let seen = monitor.clone();
    let db = crate::test_utils::connect_transaction_test_database_with_command_handler(
        "org_delete_audit",
        EventHandler::callback(move |event| {
            if REQUEST.try_with(|_| ()).is_err() {
                return;
            }
            let mut seen = seen.lock().unwrap();
            if seen.units.observe(&event) {
                drop(seen);
                std::panic::resume_unwind(Box::new(ProcessStopped));
            }
            match event {
                CommandEvent::Started(e) => {
                    let collection = e.command.get_str(&e.command_name).unwrap_or("").to_owned();
                    seen.commands
                        .push((e.command_name.clone(), collection.clone()));
                    let Some(boundary) = seen.stop else {
                        return;
                    };
                    let selected = match boundary {
                        Boundary::Start => {
                            (e.command_name == "findAndModify" && collection == "users")
                                || (e.command_name == "delete" && collection == "approval_requests")
                        }
                        Boundary::Cleanup => {
                            e.command_name == "delete" && collection == "user_service_connections"
                        }
                        Boundary::Complete => e.command_name == "delete" && collection == "users",
                    };
                    if selected {
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
    let actor = uuid::Uuid::new_v4().to_string();
    let org = uuid::Uuid::new_v4().to_string();
    db.collection::<User>("users")
        .insert_many([
            test_user(&actor, UserType::Person),
            test_user(&org, UserType::Org),
        ])
        .await
        .unwrap();
    db.collection::<OrgMembership>("org_memberships")
        .insert_one(test_membership(&org, &actor, OrgRole::Admin, None))
        .await
        .unwrap();
    db.collection::<Document>("approval_requests")
        .insert_one(
            doc! {"_id":uuid::Uuid::new_v4().to_string(),"user_id":&org,"status":"approved"},
        )
        .await
        .unwrap();
    db.collection::<Document>("user_service_connections")
        .insert_one(doc! {"_id":uuid::Uuid::new_v4().to_string(),"user_id":&org,"is_active":false})
        .await
        .unwrap();
    security_effect_service::ensure_indexes(&db).await.unwrap();
    Fixture {
        state: test_app_state(db),
        actor,
        org,
        monitor,
    }
}
fn auth(f: &Fixture) -> crate::mw::auth::AuthUser {
    let mut auth = test_auth_user(&f.actor);
    auth.ip_address = Some("192.0.2.10".into());
    auth.user_agent = Some("NyxID-Deletion-Test/1.0".into());
    auth.api_key_id = Some("fixture-key-id".into());
    auth.api_key_name = Some("Administrator test key".into());
    auth
}
async fn request(f: &Fixture) -> axum::response::Response {
    orgs::delete_org(State(f.state.clone()), auth(f), Path(f.org.clone()))
        .await
        .into_response()
}
async fn rows(f: &Fixture, collection: &str) -> Vec<Document> {
    f.state
        .db
        .collection::<Document>(collection)
        .find(doc! {})
        .sort(doc! {"_id":1})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap()
}
async fn count(f: &Fixture, collection: &str, event: &str) -> u64 {
    let field = if collection == INTENTS {
        "event.event_type"
    } else {
        "event_type"
    };
    f.state
        .db
        .collection::<Document>(collection)
        .count_documents(doc! {field:event})
        .await
        .unwrap()
}
async fn relay_twice(f: &Fixture) {
    for _ in 0..2 {
        security_effect_service::sweep(&f.state.db, &f.state.encryption_keys)
            .await
            .unwrap();
    }
}
async fn stop(f: &Fixture, boundary: Boundary) {
    f.monitor.lock().unwrap().stop = Some(boundary);
    let result = std::panic::AssertUnwindSafe(Box::pin(REQUEST.scope((), request(f))))
        .catch_unwind()
        .await;
    assert!(result.unwrap_err().is::<ProcessStopped>());
}
async fn crash(boundary: Boundary) {
    let f = fixture().await;
    stop(&f, boundary).await;
    assert_eq!(
        count(&f, INTENTS, REQUESTED).await,
        1,
        "accepted deletion lost requested evidence"
    );
    let completed = matches!(boundary, Boundary::Complete);
    assert_eq!(count(&f, INTENTS, COMPLETED).await, u64::from(completed));
    let org = f
        .state
        .db
        .collection::<Document>("users")
        .find_one(doc! {"_id":&f.org})
        .await
        .unwrap();
    if completed {
        assert!(org.is_none());
    } else {
        assert!(org.unwrap().get_document("deletion_audit").is_ok());
    }
    match boundary {
        Boundary::Start => assert_eq!(rows(&f, "approval_requests").await.len(), 1),
        Boundary::Cleanup | Boundary::Complete => {
            assert!(rows(&f, "approval_requests").await.is_empty());
            assert!(rows(&f, "user_service_connections").await.is_empty());
        }
    }
    assert_eq!(count(&f, "audit_log", REQUESTED).await, 0);
    assert_eq!(count(&f, "audit_log", COMPLETED).await, 0);
    relay_twice(&f).await;
    assert_eq!(count(&f, "audit_log", REQUESTED).await, 1);
    assert_eq!(
        count(&f, "audit_log", COMPLETED).await,
        u64::from(completed)
    );
    f.state.db.drop().await.unwrap();
}
#[tokio::test]
async fn org_deletion_requested_survives_process_death() {
    crash(Boundary::Start).await;
}
#[tokio::test]
async fn org_deletion_completed_survives_process_death() {
    crash(Boundary::Complete).await;
}
#[tokio::test]
async fn org_deletion_mid_cleanup_retains_requested_only() {
    crash(Boundary::Cleanup).await;
}
#[tokio::test]
async fn org_deletion_request_command_counts() {
    let f = fixture().await;
    let response = REQUEST.scope((), request(&f)).await;
    assert_eq!(response.status(), axum::http::StatusCode::NO_CONTENT);
    let commands = f.monitor.lock().unwrap().commands.clone();
    println!(
        "ORG_DELETE_COMMANDS total={} commands={commands:?}",
        commands.len()
    );
    assert_eq!(
        commands.len(),
        76,
        "includes discovery of bots with guest data before deletion (#3004)"
    );
    f.state.db.drop().await.unwrap();
}

async fn reject_event(f: &Fixture, event: &str) {
    f.state.db.run_command(doc! {"collMod": INTENTS, "validator": {"event.event_type": {"$ne": event}}, "validationLevel": "strict"}).await.unwrap();
}
async fn response_json(response: axum::response::Response) -> serde_json::Value {
    serde_json::from_slice(
        &axum::body::to_bytes(response.into_body(), 65_536)
            .await
            .unwrap(),
    )
    .unwrap()
}
fn actor(f: &Fixture) -> audit_service::AuditActor {
    audit_service::AuditActor::from_auth_user(&auth(f))
}

#[tokio::test]
async fn org_deletion_rejected_requested_intent_rolls_back_marker_and_starts_no_cleanup() {
    let f = fixture().await;
    let users = rows(&f, "users").await;
    let memberships = rows(&f, "org_memberships").await;
    reject_event(&f, REQUESTED).await;
    let response = request(&f).await;
    assert_eq!(
        response.status(),
        axum::http::StatusCode::INTERNAL_SERVER_ERROR
    );
    assert_eq!(response_json(response).await["error_code"], 1007);
    assert_eq!(rows(&f, "users").await, users);
    assert_eq!(rows(&f, "org_memberships").await, memberships);
    assert_eq!(rows(&f, "approval_requests").await.len(), 1);
    assert_eq!(rows(&f, "user_service_connections").await.len(), 1);
    assert!(rows(&f, INTENTS).await.is_empty());
    f.state.db.drop().await.unwrap();
}
#[tokio::test]
async fn org_deletion_rejected_completion_intent_rolls_back_final_delete_only() {
    let f = fixture().await;
    reject_event(&f, COMPLETED).await;
    let response = request(&f).await;
    assert_eq!(
        response.status(),
        axum::http::StatusCode::INTERNAL_SERVER_ERROR
    );
    assert_eq!(response_json(response).await["error_code"], 1007);
    let org = f
        .state
        .db
        .collection::<Document>("users")
        .find_one(doc! {"_id":&f.org})
        .await
        .unwrap()
        .unwrap();
    assert!(org.get_document("deletion_audit").is_ok());
    assert!(rows(&f, "approval_requests").await.is_empty());
    assert!(rows(&f, "user_service_connections").await.is_empty());
    assert_eq!(rows(&f, "org_memberships").await.len(), 1);
    assert_eq!(count(&f, INTENTS, REQUESTED).await, 1);
    assert_eq!(count(&f, INTENTS, COMPLETED).await, 0);
    relay_twice(&f).await;
    assert_eq!(count(&f, "audit_log", REQUESTED).await, 1);
    assert_eq!(count(&f, "audit_log", COMPLETED).await, 0);
    f.state.db.drop().await.unwrap();
}
#[tokio::test]
async fn org_deletion_retry_reuses_relayed_marker_without_second_requested_event() {
    let f = fixture().await;
    stop(&f, Boundary::Cleanup).await;
    let marked = f
        .state
        .db
        .collection::<Document>("users")
        .find_one(doc! {"_id":&f.org})
        .await
        .unwrap()
        .unwrap();
    let marker = marked.get_document("deletion_audit").unwrap().clone();
    relay_twice(&f).await;
    assert!(rows(&f, INTENTS).await.is_empty());
    // Refuse any new requested occurrence: completion must still succeed.
    reject_event(&f, REQUESTED).await;
    f.monitor.lock().unwrap().commands.clear();
    let response = REQUEST.scope((), request(&f)).await;
    assert_eq!(response.status(), axum::http::StatusCode::NO_CONTENT);
    let commands = f.monitor.lock().unwrap().commands.clone();
    println!(
        "ORG_DELETE_RETRY_COMMANDS total={} commands={commands:?}",
        commands.len()
    );
    assert_eq!(
        commands.len(),
        75,
        "retry repeats guest-data discovery and omits the requested intent insert"
    );
    relay_twice(&f).await;
    assert_eq!(count(&f, "audit_log", REQUESTED).await, 1);
    assert_eq!(count(&f, "audit_log", COMPLETED).await, 1);
    let requested = f
        .state
        .db
        .collection::<Document>("audit_log")
        .find_one(doc! {"event_type":REQUESTED})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(requested.get_str("_id"), marker.get_str("event_id"));
    f.state.db.drop().await.unwrap();
}
#[tokio::test]
async fn org_deletion_competing_starts_and_completions_each_commit_one_occurrence() {
    let f = fixture().await;
    let actor = actor(&f);
    let (left, right) = tokio::join!(
        super::deletion::start(&f.state.db, &f.org, &actor),
        super::deletion::start(&f.state.db, &f.org, &actor)
    );
    let marker = left.unwrap();
    assert_eq!(right.unwrap(), marker);
    assert_eq!(count(&f, INTENTS, REQUESTED).await, 1);
    let (left, right) = tokio::join!(
        super::deletion::complete(&f.state.db, &f.org, &marker, &actor),
        super::deletion::complete(&f.state.db, &f.org, &marker, &actor)
    );
    assert_eq!(usize::from(left.is_ok()) + usize::from(right.is_ok()), 1);
    let error = left.err().or(right.err()).unwrap();
    assert!(matches!(error, crate::errors::AppError::OrgNotFound(_)));
    assert_eq!(count(&f, INTENTS, COMPLETED).await, 1);
    // Recreate the exact pending intents after their first delivery to model a
    // lost receipt deletion. The foundation must recognize the same audit UUIDs.
    let intents = rows(&f, INTENTS).await;
    relay_twice(&f).await;
    f.state
        .db
        .collection::<Document>(INTENTS)
        .insert_many(intents)
        .await
        .unwrap();
    relay_twice(&f).await;
    assert_eq!(count(&f, "audit_log", REQUESTED).await, 1);
    assert_eq!(count(&f, "audit_log", COMPLETED).await, 1);
    let events = rows(&f, "audit_log").await;
    assert_eq!(events.len(), 2);
    let mut seqs = events
        .iter()
        .map(|e| e.get_i64("seq").unwrap())
        .collect::<Vec<_>>();
    seqs.sort_unstable();
    assert_eq!(seqs, vec![1, 2]);
    f.state.db.drop().await.unwrap();
}
#[tokio::test]
async fn org_deletion_zero_match_and_replaced_marker_never_commit_completion() {
    let f = fixture().await;
    let actor = actor(&f);
    let missing = uuid::Uuid::new_v4().to_string();
    assert!(matches!(
        super::deletion::start(&f.state.db, &missing, &actor).await,
        Err(crate::errors::AppError::OrgNotFound(_))
    ));
    let marker = super::deletion::start(&f.state.db, &f.org, &actor)
        .await
        .unwrap();
    let successor = uuid::Uuid::new_v4().to_string();
    f.state
        .db
        .collection::<Document>("users")
        .update_one(
            doc! {"_id":&f.org},
            doc! {"$set":{"deletion_audit.event_id":&successor}},
        )
        .await
        .unwrap();
    let before = rows(&f, "users").await;
    assert!(matches!(
        super::deletion::complete(&f.state.db, &f.org, &marker, &actor).await,
        Err(crate::errors::AppError::OrgNotFound(_))
    ));
    assert_eq!(rows(&f, "users").await, before);
    assert_eq!(count(&f, INTENTS, COMPLETED).await, 0);
    f.state
        .db
        .collection::<Document>("users")
        .delete_one(doc! {"_id":&f.org})
        .await
        .unwrap();
    assert!(matches!(
        super::deletion::complete(&f.state.db, &f.org, &successor, &actor).await,
        Err(crate::errors::AppError::OrgNotFound(_))
    ));
    assert_eq!(count(&f, INTENTS, COMPLETED).await, 0);
    f.state.db.drop().await.unwrap();
}
#[tokio::test]
async fn org_deletion_marker_uses_server_time_and_never_grants_retry_authority() {
    let f = fixture().await;
    let before = f.state.db.run_command(doc! {"hello":1}).await.unwrap();
    let marker = super::deletion::start(&f.state.db, &f.org, &actor(&f))
        .await
        .unwrap();
    let after = f.state.db.run_command(doc! {"hello":1}).await.unwrap();
    let row = f
        .state
        .db
        .collection::<Document>("users")
        .find_one(doc! {"_id":&f.org})
        .await
        .unwrap()
        .unwrap();
    let evidence = row.get_document("deletion_audit").unwrap();
    assert_eq!(evidence.get_str("event_id"), Ok(marker.as_str()));
    let at = evidence.get_datetime("requested_at").unwrap();
    assert!(
        at >= before.get_datetime("localTime").unwrap()
            && at <= after.get_datetime("localTime").unwrap()
    );
    f.state
        .db
        .collection::<Document>("org_memberships")
        .delete_many(doc! {})
        .await
        .unwrap();
    let response = request(&f).await;
    assert_eq!(response.status(), axum::http::StatusCode::FORBIDDEN);
    assert_eq!(rows(&f, "approval_requests").await.len(), 1);
    assert_eq!(count(&f, INTENTS, REQUESTED).await, 1);
    assert_eq!(count(&f, INTENTS, COMPLETED).await, 0);
    f.state.db.drop().await.unwrap();
}
#[tokio::test]
async fn org_deleted_audit_content_matches_legacy() {
    use crate::models::audit_log::AuditLog;
    let f = fixture().await;
    assert_eq!(
        request(&f).await.status(),
        axum::http::StatusCode::NO_CONTENT
    );
    let written = audit_service::notify_on_audit_write_for_user(COMPLETED, &f.actor);
    audit_service::log_for_user(
        f.state.db.clone(),
        &auth(&f),
        COMPLETED,
        Some(serde_json::json!({"org_user_id":f.org})),
    );
    let old_id = tokio::time::timeout(std::time::Duration::from_secs(5), written)
        .await
        .unwrap()
        .unwrap();
    relay_twice(&f).await;
    let collection = f.state.db.collection::<AuditLog>("audit_log");
    let old = collection
        .find_one(doc! {"_id":&old_id})
        .await
        .unwrap()
        .unwrap();
    let new = collection
        .find_one(doc! {"event_type":COMPLETED,"_id":{"$ne":old_id}})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(new.event_type, old.event_type);
    assert_eq!(new.event_data, old.event_data);
    assert_eq!(new.user_id, old.user_id);
    assert_eq!(new.ip_address, old.ip_address);
    assert_eq!(new.user_agent, old.user_agent);
    assert_eq!(new.api_key_id, old.api_key_id);
    assert_eq!(new.api_key_name, old.api_key_name);
    f.state.db.drop().await.unwrap();
}
#[tokio::test]
async fn org_deletion_requested_has_only_approved_facts() {
    let f = fixture().await;
    stop(&f, Boundary::Start).await;
    relay_twice(&f).await;
    let row = f
        .state
        .db
        .collection::<crate::models::audit_log::AuditLog>("audit_log")
        .find_one(doc! {"event_type":REQUESTED})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        row.event_data,
        Some(serde_json::json!({"org_user_id":f.org}))
    );
    let auth = auth(&f);
    assert_eq!(row.user_id, Some(f.actor.clone()));
    assert_eq!(row.ip_address, auth.ip_address);
    assert_eq!(row.user_agent, auth.user_agent);
    assert_eq!(row.api_key_id, auth.api_key_id);
    assert_eq!(row.api_key_name, auth.api_key_name);
    f.state.db.drop().await.unwrap();
}
#[tokio::test]
async fn org_deletion_occurrence_caps_and_maxima() {
    use crate::{
        models::security_effect_intent::SecurityEffect,
        services::audit_service::{
            AuditActor,
            required::{self, TextField},
        },
    };
    let raw = "x".repeat(17000);
    let actor = AuditActor {
        user_id: raw.clone(),
        ip_address: Some(raw.clone()),
        user_agent: Some(raw.clone()),
        api_key_id: Some(raw.clone()),
        api_key_name: Some(raw.clone()),
    };
    for event_type in [REQUESTED, COMPLETED] {
        let turn = uuid::Uuid::new_v4().to_string();
        let intent =
            crate::turn_trace::scope(crate::turn_trace::TurnContext::for_turn(&turn), async {
                super::deletion::occurrence(&actor, event_type, &raw)
            })
            .await
            .unwrap();
        let SecurityEffect::Audit { event } = intent.effect else {
            panic!("audit")
        };
        let bytes = bson::to_vec(&event).unwrap().len();
        println!(
            "ORG_DELETION_MAX {event_type} bytes={bytes} margin={}",
            16384 - bytes
        );
        assert!(bytes < 16384);
        assert_eq!(
            event.user_id,
            Some(required::text(TextField::Identifier, &raw))
        );
        assert_eq!(
            event.ip_address,
            Some(required::text(TextField::ForwardedIp, &raw))
        );
        assert_eq!(
            event.user_agent,
            Some(required::text(TextField::UserAgent, &raw))
        );
        assert_eq!(
            event.api_key_id,
            Some(required::text(TextField::Identifier, &raw))
        );
        assert_eq!(
            event.api_key_name,
            Some(required::text(TextField::KeyName, &raw))
        );
        assert_eq!(
            event.event_data.as_ref().unwrap()["org_user_id"],
            required::text(TextField::Identifier, &raw)
        );
    }
}
#[tokio::test]
async fn org_deletion_oversized_request_context_succeeds_and_relays_capped_values() {
    let f = fixture().await;
    let mut auth = auth(&f);
    let raw = "界".repeat(17000);
    auth.ip_address = Some(raw.clone());
    auth.user_agent = Some(raw.clone());
    auth.api_key_id = Some(raw.clone());
    auth.api_key_name = Some(raw.clone());
    let response = orgs::delete_org(State(f.state.clone()), auth, Path(f.org.clone()))
        .await
        .into_response();
    assert_eq!(response.status(), axum::http::StatusCode::NO_CONTENT);
    relay_twice(&f).await;
    for event in rows(&f, "audit_log").await {
        for (key, cap) in [
            ("ip_address", 256),
            ("user_agent", 1024),
            ("api_key_id", 256),
            ("api_key_name", 256),
        ] {
            let value = event.get_str(key).unwrap();
            assert!(value.len() <= cap && cap - value.len() < 4);
            assert!(value.ends_with("...[truncated]"));
        }
    }
    f.state.db.drop().await.unwrap();
}

mod retry;

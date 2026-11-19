use super::{RevokeAssistantGrantRequest, revoke_approval_grant};
use crate::{
    AppState,
    handlers::approvals,
    mw::auth::AuthUser,
    services::{approval_service, audit_service, operation_descriptor, security_effect_service},
    test_utils::{
        connect_transaction_test_database_with_command_handler, test_app_state, test_auth_user,
    },
};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use bson::{Document, doc};
use futures::{FutureExt, TryStreamExt};
use mongodb::event::{EventHandler, command::CommandEvent};
use serde_json::json;
use std::{
    panic::AssertUnwindSafe,
    sync::{Arc, Mutex},
};
use uuid::Uuid;

const GRANTS: &str = crate::models::approval_grant::COLLECTION_NAME;
const INTENTS: &str = crate::models::security_effect_intent::COLLECTION_NAME;
const AUDIT: &str = crate::models::audit_log::COLLECTION_NAME;
tokio::task_local! { static REQUEST: (); }

#[derive(Default)]
struct Monitor {
    commands: Vec<(String, String)>,
    transaction_commands: Vec<(String, String, Document, i64)>,
    stop_after_source: bool,
    stop_on_ack: Option<i32>,
    before_insert: Option<InterleavedInsert>,
}
struct InterleavedInsert {
    command: &'static str,
    collection: &'static str,
    insert: Box<dyn FnOnce() + Send>,
}
#[derive(Debug)]
struct ProcessStopped;

struct Fixture {
    state: AppState,
    owner: String,
    grant: String,
    request_id: String,
    service: String,
    monitor: Arc<Mutex<Monitor>>,
}

impl Fixture {
    async fn new() -> Self {
        Self::with_command_observer(|_| {}).await
    }

    async fn with_command_observer(
        observe: impl Fn(&CommandEvent) + Send + Sync + 'static,
    ) -> Self {
        let monitor = Arc::new(Mutex::new(Monitor::default()));
        let seen = monitor.clone();
        let db = connect_transaction_test_database_with_command_handler(
            "approval_grant_audit",
            EventHandler::callback(move |event| {
                if REQUEST.try_with(|_| ()).is_err() {
                    return;
                }
                observe(&event);
                let mut m = seen.lock().unwrap();
                match event {
                    CommandEvent::Started(e) => {
                        let coll = e.command.get_str(&e.command_name).unwrap_or("").to_owned();
                        if m.stop_after_source
                            && (((e.command_name == "update" && coll == GRANTS)
                                || (e.command_name == "findAndModify"
                                    && matches!(
                                        coll.as_str(),
                                        "approval_requests" | "service_approval_configs"
                                    ))
                                || (e.command_name == "delete"
                                    && coll == "service_approval_configs"))
                                && !e.command.contains_key("autocommit")
                                || e.command_name == "commitTransaction")
                        {
                            m.stop_on_ack = Some(e.request_id);
                        }
                        if e.command_name == "commitTransaction" {
                            assert_eq!(
                                e.command.get_document("writeConcern").unwrap().get_str("w"),
                                Ok("majority")
                            );
                        }
                        if e.command.get_bool("autocommit") == Ok(false) {
                            m.transaction_commands.push((
                                e.command_name.clone(),
                                coll.clone(),
                                e.command.get_document("lsid").unwrap().clone(),
                                e.command.get_i64("txnNumber").unwrap(),
                            ));
                        }
                        let before_write = if m.before_insert.as_ref().is_some_and(|hook| {
                            hook.command == e.command_name && hook.collection == coll
                        }) {
                            m.before_insert.take()
                        } else {
                            None
                        };
                        m.commands.push((e.command_name, coll));
                        drop(m);
                        if let Some(insert_winner) = before_write {
                            (insert_winner.insert)();
                        }
                    }
                    CommandEvent::Succeeded(e) if m.stop_on_ack == Some(e.request_id) => {
                        m.stop_on_ack = None;
                        drop(m);
                        std::panic::resume_unwind(Box::new(ProcessStopped));
                    }
                    _ => {}
                }
            }),
        )
        .await;
        let owner = Uuid::new_v4().to_string();
        let grant = Uuid::new_v4().to_string();
        let service = Uuid::new_v4().to_string();
        db.collection::<Document>(GRANTS).insert_one(doc! {
            "_id":&grant,"user_id":&owner,"service_id":&service,"service_name":"Fixture",
            "requester_type":"api_key","requester_id":&owner,"approval_request_id":Uuid::new_v4().to_string(),
            "granted_at":bson::DateTime::now(),"expires_at":bson::DateTime::from_chrono(chrono::Utc::now()+chrono::Duration::days(1)), "revoked":false
        }).await.unwrap();
        security_effect_service::ensure_indexes(&db).await.unwrap();
        Self {
            state: test_app_state(db),
            owner,
            grant,
            request_id: Uuid::new_v4().to_string(),
            service,
            monitor,
        }
    }

    fn auth(&self) -> AuthUser {
        let mut auth = test_auth_user(&self.owner);
        auth.ip_address = Some("192.0.2.14".into());
        auth.user_agent = Some("Fixture/1.0".into());
        auth.api_key_name = Some("Fixture caller".into());
        auth.api_key_id = Some(Uuid::new_v4().to_string());
        auth
    }

    async fn revoke(&self, assistant: bool, auth: AuthUser) -> axum::response::Response {
        if assistant {
            revoke_approval_grant(
                State(self.state.clone()),
                auth,
                Default::default(),
                Json(RevokeAssistantGrantRequest {
                    action_request_id: Uuid::new_v4().to_string(),
                    grant_id: self.grant.clone(),
                }),
            )
            .await
            .into_response()
        } else {
            approvals::revoke_grant(
                State(self.state.clone()),
                auth,
                Default::default(),
                Path(self.grant.clone()),
                Query(approvals::RevokeGrantQuery { org_id: None }),
            )
            .await
            .into_response()
        }
    }

    async fn rows(&self, collection: &str) -> Vec<Document> {
        self.state
            .db
            .collection::<Document>(collection)
            .find(doc! {})
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap()
    }
}

async fn verify_revoke(assistant: bool) {
    let f = Fixture::new().await;
    let auth = f.auth();
    let context = crate::turn_trace::TurnContext::for_turn(&Uuid::new_v4().to_string());
    f.monitor.lock().unwrap().stop_after_source = true;
    let stopped = AssertUnwindSafe(REQUEST.scope(
        (),
        crate::turn_trace::scope(context, Box::pin(f.revoke(assistant, auth.clone()))),
    ))
    .catch_unwind()
    .await;
    assert!(stopped.unwrap_err().is::<ProcessStopped>());
    assert_eq!(f.rows(GRANTS).await[0].get_bool("revoked"), Ok(true));
    assert!(f.rows(AUDIT).await.is_empty());
    let intents = f.rows(INTENTS).await;
    println!(
        "REVOKE_DEATH assistant={assistant} revoked=true durable_intents={}",
        intents.len()
    );
    assert_eq!(
        intents.len(),
        1,
        "committed revoke requires durable evidence"
    );

    let db = &f.state.db;
    security_effect_service::sweep(db, &f.state.encryption_keys)
        .await
        .unwrap();
    db.collection::<Document>(INTENTS)
        .insert_one(&intents[0])
        .await
        .unwrap();
    security_effect_service::sweep(db, &f.state.encryption_keys)
        .await
        .unwrap();
    assert!(f.rows(INTENTS).await.is_empty());
    let mut rows = f.rows(AUDIT).await;
    assert_eq!(rows.len(), 1);
    let mut actual = rows.pop().unwrap();
    assert_eq!(actual.get_str("_id"), intents[0].get_str("_id"));
    assert_eq!(actual.get_i64("seq"), Ok(1));
    assert!(actual.get_str("entry_hash").is_ok());

    let event = "approval_grant_revoked";
    let mut expected = json!({"grant_id":f.grant,"owner_user_id":f.owner});
    if assistant {
        expected["assistant_action"] = true.into();
    }
    let written = audit_service::notify_on_audit_write_for_user(event, &f.owner);
    crate::turn_trace::scope(context, async {
        audit_service::log_for_user(db.clone(), &auth, event, Some(expected));
    })
    .await;
    let mut old = db
        .collection::<Document>(AUDIT)
        .find_one(doc! {"_id":written.await.unwrap()})
        .await
        .unwrap()
        .unwrap();
    for key in ["_id", "created_at", "seq", "prev_hash", "entry_hash"] {
        actual.remove(key);
        old.remove(key);
    }
    assert_eq!(actual, old);
    db.drop().await.unwrap();
}

#[tokio::test]
async fn web_revoke_survives_death_and_replay_with_legacy_content() {
    verify_revoke(false).await;
}
#[tokio::test]
async fn assistant_revoke_survives_death_and_replay_with_legacy_content() {
    verify_revoke(true).await;
}

#[tokio::test]
async fn revoke_request_commands() {
    for assistant in [false, true] {
        let f = Fixture::new().await;
        assert_eq!(
            REQUEST
                .scope((), f.revoke(assistant, f.auth()))
                .await
                .status(),
            StatusCode::OK
        );
        let commands = f.monitor.lock().unwrap().commands.clone();
        assert_eq!(
            commands.len(),
            if assistant { 7 } else { 6 },
            "{commands:?}"
        );
        println!(
            "REVOKE_REQUEST assistant={assistant} count={} commands={commands:?}",
            commands.len()
        );
        f.state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn proxy_approval_lookup_commands() {
    let f = Fixture::new().await;
    let db = &f.state.db;
    db.collection::<Document>("service_approval_configs").insert_one(doc! {
        "_id":Uuid::new_v4().to_string(),"user_id":&f.owner,"service_id":&f.service,"service_name":"Fixture",
        "approval_mode":"grant","approval_required":true, "created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()
    }).await.unwrap();
    let op = operation_descriptor::build_http_descriptor("POST", "/items", None);
    let result = REQUEST
        .scope(
            (),
            approval_service::evaluate_and_check(
                db,
                &f.owner,
                &f.owner,
                &f.service,
                &op,
                Some("api_key"),
                &f.owner,
                false,
                false,
            ),
        )
        .await
        .unwrap();
    assert!(matches!(
        result,
        approval_service::ApprovalOutcome::Allowed { required: true }
    ));
    let commands = f.monitor.lock().unwrap().commands.clone();
    println!(
        "PROXY_APPROVAL count={} commands={commands:?}",
        commands.len()
    );
    assert_eq!(
        commands,
        [
            ("find".into(), "users".into()),
            ("find".into(), "service_approval_configs".into()),
            ("find".into(), GRANTS.into())
        ]
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn rejected_intent_keeps_the_grant_active() {
    for assistant in [false, true] {
        let f = Fixture::new().await;
        let before = f.rows(GRANTS).await;
        f.state
            .db
            .run_command(doc! {"collMod":INTENTS,"validator":{"reject_fixture":{"$exists":true}}})
            .await
            .unwrap();
        let response = f.revoke(assistant, f.auth()).await;
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let error: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(error["error_code"], 1007);
        assert_eq!(error["message"], "An internal error occurred");
        assert_eq!(f.rows(GRANTS).await, before);
        assert!(f.rows(INTENTS).await.is_empty());
        assert!(f.rows(AUDIT).await.is_empty());
        if assistant {
            let receipts = f.rows("assistant_action_receipts").await;
            assert_eq!(receipts.len(), 1);
            assert_eq!(receipts[0].get_str("status"), Ok("pending"));
        }
        f.state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn repeated_web_revoke_remains_a_successful_audited_match() {
    let f = Fixture::new().await;
    for count in 1..=2 {
        assert_eq!(f.revoke(false, f.auth()).await.status(), StatusCode::OK);
        assert_eq!(f.rows(INTENTS).await.len(), count);
    }
    assert_eq!(
        f.revoke(false, test_auth_user(&Uuid::new_v4().to_string()))
            .await
            .status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(
        f.revoke(true, f.auth()).await.status(),
        StatusCode::NOT_FOUND
    );
    assert_eq!(f.rows(INTENTS).await.len(), 2);
    f.state.db.drop().await.unwrap();
}

#[path = "approval_change_audit_tests.rs"]
mod changes;

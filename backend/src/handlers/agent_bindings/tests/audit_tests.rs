use super::*;

use axum::{http::StatusCode, response::IntoResponse};
use bson::Document;
use futures::FutureExt;
use mongodb::event::{EventHandler, command::CommandEvent};
use std::sync::{Arc, Mutex};

const INTENTS: &str = "security_effect_intents";
const BINDINGS: &str = "agent_service_bindings";
tokio::task_local! { static REQUEST: (); }

#[derive(Default)]
struct Monitor {
    stop_after_commit: bool,
    commands: Vec<(String, String)>,
}
#[derive(Debug)]
struct ProcessStopped;

struct Fixture {
    state: AppState,
    ids: Fixtures,
    binding: Option<AgentServiceBinding>,
    monitor: Arc<Mutex<Monitor>>,
}

async fn fixture(deleting: bool) -> Fixture {
    let monitor = Arc::new(Mutex::new(Monitor::default()));
    let seen = monitor.clone();
    let db = crate::test_utils::connect_transaction_test_database_with_command_handler(
        "binding_audit",
        EventHandler::callback(move |event| {
            if REQUEST.try_with(|_| ()).is_err() {
                return;
            }
            let mut m = seen.lock().unwrap();
            match event {
                CommandEvent::Started(event) => {
                    let collection = event
                        .command
                        .get_str(&event.command_name)
                        .unwrap_or("")
                        .to_owned();
                    m.commands.push((event.command_name, collection));
                }
                CommandEvent::Succeeded(event)
                    if event.command_name == "commitTransaction" && m.stop_after_commit =>
                {
                    m.stop_after_commit = false;
                    drop(m);
                    std::panic::resume_unwind(Box::new(ProcessStopped));
                }
                _ => {}
            }
        }),
    )
    .await;
    let ids = seed_fixtures(&db).await;
    db.collection::<Document>(API_KEYS)
        .update_one(
            doc! {"_id":&ids.api_key_id},
            doc! {"$set":{"allow_all_services":false}},
        )
        .await
        .unwrap();
    let binding = if deleting {
        Some(
            agent_binding_service::create_binding(
                &db,
                &ids.user_id,
                &ids.api_key_id,
                &ids.service_id,
                &ids.credential_id,
            )
            .await
            .unwrap(),
        )
    } else {
        None
    };
    crate::services::security_effect_service::ensure_indexes(&db)
        .await
        .unwrap();
    Fixture {
        state: test_app_state(db),
        ids,
        binding,
        monitor,
    }
}

async fn request(f: &Fixture) -> axum::response::Response {
    request_as(f, test_auth_user(&f.ids.user_id)).await
}

async fn request_as(f: &Fixture, auth: AuthUser) -> axum::response::Response {
    if let Some(binding) = &f.binding {
        delete_binding(
            State(f.state.clone()),
            auth,
            tele(),
            Path((f.ids.api_key_id.clone(), binding.id.clone())),
        )
        .await
        .into_response()
    } else {
        create_binding(
            State(f.state.clone()),
            auth,
            tele(),
            Path(f.ids.api_key_id.clone()),
            Json(CreateBindingRequest {
                user_service_id: f.ids.service_id.clone(),
                user_api_key_id: f.ids.credential_id.clone(),
            }),
        )
        .await
        .into_response()
    }
}

async fn rows(db: &mongodb::Database, collection: &str) -> Vec<Document> {
    db.collection::<Document>(collection)
        .find(doc! {})
        .sort(doc! {"_id":1})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap()
}

async fn death(deleting: bool) {
    let f = fixture(deleting).await;
    f.monitor.lock().unwrap().stop_after_commit = true;
    let result = std::panic::AssertUnwindSafe(Box::pin(REQUEST.scope((), request(&f))))
        .catch_unwind()
        .await;
    assert!(result.unwrap_err().is::<ProcessStopped>());
    let bindings = rows(&f.state.db, BINDINGS).await;
    assert_eq!(bindings.len(), usize::from(!deleting));
    let key = rows(&f.state.db, API_KEYS).await.remove(0);
    assert_eq!(
        key.get_array("allowed_service_ids").unwrap().len(),
        usize::from(!deleting)
    );
    assert!(rows(&f.state.db, "audit_log").await.is_empty());
    let intents = rows(&f.state.db, INTENTS).await;
    assert_eq!(
        intents.len(),
        1,
        "binding mutation committed without durable evidence"
    );
    crate::services::security_effect_service::sweep(&f.state.db, &f.state.encryption_keys)
        .await
        .unwrap();
    f.state
        .db
        .collection::<Document>(INTENTS)
        .insert_one(&intents[0])
        .await
        .unwrap();
    crate::services::security_effect_service::sweep(&f.state.db, &f.state.encryption_keys)
        .await
        .unwrap();
    let audits = rows(&f.state.db, "audit_log").await;
    assert_eq!(audits.len(), 1);
    assert_eq!(
        audits[0].get_str("event_type"),
        Ok(if deleting {
            "agent_binding_deleted"
        } else {
            "agent_binding_created"
        })
    );
    assert_eq!(audits[0].get_i64("seq"), Ok(1));
    assert!(audits[0].get_str("entry_hash").is_ok());
    assert!(rows(&f.state.db, INTENTS).await.is_empty());
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn binding_create_survives_process_death_and_replay() {
    death(false).await;
}
#[tokio::test]
async fn binding_delete_survives_process_death_and_replay() {
    death(true).await;
}

#[tokio::test]
async fn rejected_intent_rolls_back_binding_and_parent_authority() {
    for deleting in [false, true] {
        let f = fixture(deleting).await;
        let key = rows(&f.state.db, API_KEYS).await;
        let binding = rows(&f.state.db, BINDINGS).await;
        let credential = rows(&f.state.db, USER_API_KEYS).await;
        f.state
            .db
            .run_command(doc! {"collMod":INTENTS,"validator":{"reject_fixture":{"$exists":true}}})
            .await
            .unwrap();
        let response = request(&f).await;
        assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
        let body = axum::body::to_bytes(response.into_body(), usize::MAX)
            .await
            .unwrap();
        let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(body["error_code"], 1007);
        assert_eq!(body["error"], "database_error");
        assert_eq!(body["message"], "An internal error occurred");
        assert_eq!(rows(&f.state.db, API_KEYS).await, key);
        assert_eq!(rows(&f.state.db, BINDINGS).await, binding);
        assert_eq!(rows(&f.state.db, USER_API_KEYS).await, credential);
        assert!(rows(&f.state.db, INTENTS).await.is_empty());
        f.state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn binding_request_command_counts() {
    for deleting in [false, true] {
        let f = fixture(deleting).await;
        assert!(REQUEST.scope((), request(&f)).await.status().is_success());
        let commands = f.monitor.lock().unwrap().commands.clone();
        assert_eq!(
            commands.len(),
            if deleting { 11 } else { 15 },
            "{commands:?}"
        );
        println!(
            "BINDING_REQUEST deleting={deleting} count={} commands={commands:?}",
            commands.len()
        );
        f.state.db.drop().await.unwrap();
    }
}

async fn measure<T>(
    f: &Fixture,
    label: &str,
    operation: impl std::future::Future<Output = T>,
) -> T {
    f.monitor.lock().unwrap().commands.clear();
    let result = REQUEST.scope((), operation).await;
    let commands = f.monitor.lock().unwrap().commands.clone();
    let expected = match label {
        "api_key_authentication" => 2,
        "binding_list" | "unaudited_delete" => 6,
        "unaudited_create" => 10,
        _ => 1,
    };
    assert_eq!(commands.len(), expected, "{label}: {commands:?}");
    println!(
        "BINDING_UNCHANGED_PATH {label} count={} commands={commands:?}",
        commands.len()
    );
    result
}

#[tokio::test]
async fn unaudited_and_hot_path_command_counts() {
    use crate::services::{key_service, node_service};
    let f = fixture(true).await;
    let db = &f.state.db;
    let raw_key = format!("nyxid_ag_{}", "7".repeat(64));
    db.collection::<Document>(API_KEYS)
        .update_one(
            doc! {"_id":&f.ids.api_key_id},
            doc! {"$set":{"key_hash":crate::crypto::token::hash_token(&raw_key),"scopes":"proxy"}},
        )
        .await
        .unwrap();
    measure(
        &f,
        "api_key_authentication",
        key_service::validate_api_key(db, &raw_key),
    )
    .await
    .unwrap();
    assert_eq!(
        measure(
            &f,
            "proxy_binding_lookup",
            agent_binding_service::resolve_credential_override(
                db,
                &f.ids.api_key_id,
                &f.ids.service_id,
                &f.ids.user_id
            )
        )
        .await
        .unwrap(),
        Some(f.ids.credential_id.clone())
    );
    let request_id = uuid::Uuid::new_v4().to_string();
    db.collection::<Document>("approval_requests").insert_one(doc! {
        "_id":&request_id,"user_id":&f.ids.user_id,"service_id":&f.ids.service_id,
        "service_name":"Fixture","service_slug":"fixture","requester_type":"api_key","requester_id":&f.ids.user_id,
        "operation_summary":"fixture","status":"pending","idempotency_key":uuid::Uuid::new_v4().to_string(),
        "created_at":bson::DateTime::now(),"expires_at":bson::DateTime::from_chrono(Utc::now()+chrono::Duration::hours(1))
    }).await.unwrap();
    let mut auth = test_auth_user(&f.ids.user_id);
    auth.auth_method = crate::mw::auth::AuthMethod::ApiKey;
    auth.api_key_id = Some(f.ids.api_key_id.clone());
    let _ = measure(
        &f,
        "approval_decision_poll",
        crate::handlers::approvals::get_request_status(
            State(f.state.clone()),
            auth,
            Path(request_id),
        ),
    )
    .await
    .unwrap();
    let node_id = uuid::Uuid::new_v4().to_string();
    let raw_node = "fixture-node-token";
    db.collection::<Document>("nodes").insert_one(doc! {
        "_id":&node_id,"user_id":&f.ids.user_id,"name":"Fixture node","status":"online","is_active":true,
        "auth_token_hash":crate::crypto::token::hash_token(raw_node),"created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now(),"metrics":{}
    }).await.unwrap();
    measure(
        &f,
        "node_heartbeat",
        node_service::update_heartbeat(db, &node_id, None),
    )
    .await
    .unwrap();
    assert_eq!(
        measure(
            &f,
            "node_websocket_credential_auth",
            node_service::validate_auth_token(db, raw_node)
        )
        .await
        .unwrap()
        .id,
        node_id
    );
    let _ = measure(
        &f,
        "binding_list",
        list_bindings(
            State(f.state.clone()),
            test_auth_user(&f.ids.user_id),
            Path(f.ids.api_key_id.clone()),
        ),
    )
    .await
    .unwrap();
    measure(
        &f,
        "unaudited_delete",
        agent_binding_service::delete_binding(
            db,
            &f.ids.user_id,
            &f.ids.api_key_id,
            &f.binding.as_ref().unwrap().id,
        ),
    )
    .await
    .unwrap();
    measure(
        &f,
        "unaudited_create",
        agent_binding_service::create_binding(
            db,
            &f.ids.user_id,
            &f.ids.api_key_id,
            &f.ids.service_id,
            &f.ids.credential_id,
        ),
    )
    .await
    .unwrap();
    assert!(rows(db, INTENTS).await.is_empty());
    db.drop().await.unwrap();
}

async fn parity(deleting: bool) {
    let f = fixture(deleting).await;
    let mut auth = test_auth_user(&f.ids.user_id);
    auth.auth_method = crate::mw::auth::AuthMethod::ApiKey;
    auth.scope = "write".into();
    auth.api_key_id = Some(f.ids.api_key_id.clone());
    auth.api_key_name = Some("Fixture key 🦀".into());
    auth.ip_address = Some("192.0.2.12".into());
    auth.user_agent = Some("Fixture client/1.0".into());
    let context = crate::turn_trace::TurnContext::for_turn(&uuid::Uuid::new_v4().to_string());
    assert!(
        crate::turn_trace::scope(context, request_as(&f, auth.clone()))
            .await
            .status()
            .is_success()
    );
    let event = if deleting {
        "agent_binding_deleted"
    } else {
        "agent_binding_created"
    };
    let metadata = if let Some(binding) = &f.binding {
        serde_json::json!({"binding_id":binding.id,"api_key_id":f.ids.api_key_id})
    } else {
        let binding = rows(&f.state.db, BINDINGS).await.remove(0);
        serde_json::json!({"binding_id":binding.get_str("_id").unwrap(),"api_key_id":f.ids.api_key_id,
            "user_service_id":f.ids.service_id,"user_api_key_id":f.ids.credential_id})
    };
    let intents = rows(&f.state.db, INTENTS).await;
    assert_eq!(intents.len(), 1);
    let encoded = serde_json::to_string(&intents[0]).unwrap();
    for forbidden in [
        "key_hash",
        "key_prefix",
        "credential_encrypted",
        "access_token",
        "refresh_token",
        "test-credential",
        "deadbeef",
    ] {
        assert!(!encoded.contains(forbidden), "{forbidden}");
    }
    crate::services::security_effect_service::sweep(&f.state.db, &f.state.encryption_keys)
        .await
        .unwrap();
    let mut actual = rows(&f.state.db, "audit_log").await.remove(0);
    let written = audit_service::notify_on_audit_write_for_user(event, &f.ids.user_id);
    crate::turn_trace::scope(context, async {
        audit_service::log_for_user(f.state.db.clone(), &auth, event, Some(metadata));
    })
    .await;
    written.await.unwrap();
    let mut old = rows(&f.state.db, "audit_log")
        .await
        .into_iter()
        .find(|row| row.get_str("_id") != actual.get_str("_id"))
        .unwrap();
    for field in ["_id", "created_at", "seq", "prev_hash", "entry_hash"] {
        actual.remove(field);
        old.remove(field);
    }
    assert_eq!(actual, old, "audit content must not shrink");
    f.state.db.drop().await.unwrap();
}
#[tokio::test]
async fn binding_created_content_parity() {
    parity(false).await;
}
#[tokio::test]
async fn binding_deleted_content_parity() {
    parity(true).await;
}

#[tokio::test]
async fn missing_duplicate_and_concurrent_writes_emit_only_committed_changes() {
    let f = fixture(false).await;
    let (a, b) = tokio::join!(request(&f), request(&f));
    let mut statuses = [a.status().as_u16(), b.status().as_u16()];
    statuses.sort();
    assert_eq!(statuses, [200, 409]);
    assert_eq!(rows(&f.state.db, INTENTS).await.len(), 1);
    let binding = bson::from_document(rows(&f.state.db, BINDINGS).await.remove(0)).unwrap();
    let mut f = Fixture {
        binding: Some(binding),
        ..f
    };
    let (a, b) = tokio::join!(request(&f), request(&f));
    let mut statuses = [a.status().as_u16(), b.status().as_u16()];
    statuses.sort();
    assert_eq!(statuses, [200, 404]);
    assert_eq!(rows(&f.state.db, INTENTS).await.len(), 2);
    assert!(rows(&f.state.db, BINDINGS).await.is_empty());
    assert_eq!(request(&f).await.status(), StatusCode::NOT_FOUND);
    f.binding = None;
    f.ids.credential_id = uuid::Uuid::new_v4().to_string();
    assert_eq!(request(&f).await.status(), StatusCode::NOT_FOUND);
    assert_eq!(rows(&f.state.db, INTENTS).await.len(), 2);
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn response_enrichment_failure_keeps_committed_binding_and_evidence() {
    let f = fixture(false).await;
    // Creation validates the connection and credential; its response later loads the endpoint.
    f.state
        .db
        .collection::<Document>(USER_ENDPOINTS)
        .update_many(doc! {}, doc! {"$set":{"label":17}})
        .await
        .unwrap();
    assert_eq!(
        request(&f).await.status(),
        StatusCode::INTERNAL_SERVER_ERROR
    );
    assert_eq!(rows(&f.state.db, BINDINGS).await.len(), 1);
    assert_eq!(rows(&f.state.db, INTENTS).await.len(), 1);
    let key = rows(&f.state.db, API_KEYS).await.remove(0);
    assert_eq!(key.get_array("allowed_service_ids").unwrap().len(), 1);
    assert_eq!(request(&f).await.status(), StatusCode::CONFLICT);
    assert_eq!(rows(&f.state.db, INTENTS).await.len(), 1);
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn binding_occurrence_caps_and_maxima() {
    use crate::models::security_effect_intent::SecurityEffect;
    use crate::services::{agent_binding_service::audit, audit_service::AuditActor};
    for fill in ["x".repeat(20_000), "🦀".repeat(5_000)] {
        let actor = AuditActor {
            user_id: fill.clone(),
            ip_address: Some(fill.clone()),
            user_agent: Some(fill.clone()),
            api_key_id: Some(fill.clone()),
            api_key_name: Some(fill.clone()),
        };
        let mut binding = fixture_binding(&fill, &fill, &fill, &fill);
        binding.id = fill.clone();
        for deleting in [false, true] {
            let context =
                crate::turn_trace::TurnContext::for_turn(&uuid::Uuid::new_v4().to_string());
            let intent = crate::turn_trace::scope(context, async {
                if deleting {
                    audit::deleted(&actor, &fill, &fill)
                } else {
                    audit::created(&actor, &binding)
                }
            })
            .await
            .unwrap();
            let SecurityEffect::Audit { event } = intent.effect else {
                panic!("audit")
            };
            let bytes = bson::to_vec(&event).unwrap().len();
            let maximum = if deleting { 2996 } else { 3552 };
            assert!(bytes <= maximum);
            if fill.is_ascii() {
                assert_eq!(bytes, maximum);
            }
            assert!(maximum < 16384);
            assert!(
                event
                    .user_agent
                    .as_deref()
                    .unwrap()
                    .ends_with("...[truncated]")
            );
            let metadata = event.event_data.as_ref().unwrap();
            for field in if deleting {
                vec!["binding_id", "api_key_id"]
            } else {
                vec![
                    "binding_id",
                    "api_key_id",
                    "user_service_id",
                    "user_api_key_id",
                ]
            } {
                let value = &metadata[field];
                assert!(value.as_str().unwrap().len() <= 256);
                assert!(value.as_str().unwrap().ends_with("...[truncated]"));
            }
            println!(
                "BINDING_OCCURRENCE_MAX deleting={deleting} ascii={} bytes={bytes} margin={}",
                fill.is_ascii(),
                16384 - bytes
            );
        }
    }
}

#[tokio::test]
async fn idle_relay_is_one_indexed_read() {
    let f = fixture(false).await;
    measure(
        &f,
        "idle_relay",
        crate::services::security_effect_service::sweep(&f.state.db, &f.state.encryption_keys),
    )
    .await
    .unwrap();
    assert!(rows(&f.state.db, INTENTS).await.is_empty());
    f.state.db.drop().await.unwrap();
}

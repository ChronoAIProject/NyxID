//! Monitor real MongoDB commands, including authority reads and transaction
//! commands. Setup/cleanup are excluded identically in both runs.
use super::super::*;
use crate::services::{assistant_nyxagent as engine, channel_turn_delivery as delivery};
use bson::doc;
use mongodb::event::{EventHandler, command::CommandEvent};
use std::{collections::BTreeMap, sync::Mutex};

type Counts = Arc<Mutex<BTreeMap<String, usize>>>;
async fn database() -> (mongodb::Database, Counts) {
    let counts = Arc::new(Mutex::new(BTreeMap::new()));
    let observed = counts.clone();
    let handler = EventHandler::callback(move |event| {
        if let CommandEvent::Started(event) = event {
            let collection = event.command.get_str(&event.command_name).unwrap_or("");
            *observed
                .lock()
                .unwrap()
                .entry(format!("{}:{collection}", event.command_name))
                .or_default() += 1;
        }
    });
    (
        crate::test_utils::connect_transaction_test_database_with_command_handler(
            "turn_trace_parity",
            handler,
        )
        .await,
        counts,
    )
}
fn take(counts: &Counts) -> BTreeMap<String, usize> {
    std::mem::take(&mut *counts.lock().unwrap())
}

#[tokio::test]
async fn admission_continuation_and_settlement_command_count_parity() {
    let mut measured = Vec::new();
    for disabled in [true, false] {
        let (db, counts) = database().await;
        engine::ensure_indexes(&db).await.unwrap();
        let owner = Uuid::new_v4().to_string();
        db.collection(crate::models::user::COLLECTION_NAME)
            .insert_one(crate::test_utils::test_user(
                &owner,
                crate::models::user::UserType::Person,
            ))
            .await
            .unwrap();
        let state = crate::test_utils::test_app_state(db.clone());
        take(&counts);
        let phases = DISABLED
            .scope(
                disabled,
                Ingress::new().scope(async {
                    let row = engine::begin_turn(
                        &db,
                        &owner,
                        &engine::TurnRequest {
                            acknowledgement_id: None,
                            retry_of: None,
                            attachment_ids: vec![],
                            agent_id: None,
                            conversation_id: None,
                            text: "private-prompt-canary".into(),
                            model: None,
                            access_mode: None,
                        },
                        &state.encryption_keys,
                    )
                    .await
                    .unwrap();
                    let admission = take(&counts);
                    let turn = row.active_turn.as_ref().unwrap();
                    let context = TurnContext::for_turn(&turn.turn_id);
                    scope(context, async {
                        crate::services::assistant_turn_observability::request_started(
                            &db,
                            &row,
                            chrono::Utc::now(),
                            chrono::Utc::now() + chrono::Duration::minutes(10),
                        )
                        .await
                        .unwrap();
                        let window =
                            crate::services::assistant_continuation::started(&db, &owner, &row.id)
                                .await
                                .unwrap()
                                .unwrap();
                        crate::services::assistant_continuation::completed(
                            &db,
                            &owner,
                            &row.id,
                            &window,
                            "opaque-call-digest",
                        )
                        .await
                        .unwrap();
                        let continuation = take(&counts);
                        engine::finish_turn(
                            &db,
                            &row,
                            &row.credential_api_key_id,
                            &Uuid::new_v4().to_string(),
                            &engine::TurnResult {
                                text: "private-answer-canary".into(),
                                session_id: None,
                                response_id: None,
                                error: None,
                            },
                        )
                        .await
                        .unwrap();
                        let settlement = take(&counts);
                        vec![admission, continuation, settlement]
                    })
                    .await
                }),
            )
            .await;
        measured.push(phases);
        db.drop().await.unwrap();
    }
    assert_eq!(
        measured[0], measured[1],
        "admission / continuation / settlement command parity"
    );
    assert!(measured[0].iter().all(|phase| !phase.is_empty()));
    eprintln!(
        "turn trace admission/continuation/settlement parity: {:?}",
        measured[0]
    );
}

#[tokio::test]
async fn queued_source_bind_and_delivery_command_count_parity() {
    let mut measured = Vec::new();
    for disabled in [true, false] {
        let (db, counts) = database().await;
        let turn = Uuid::new_v4().to_string();
        let ingress = Uuid::new_v4().to_string();
        let events =
            db.collection::<bson::Document>(crate::models::nyxbot_channel::EVENTS_COLLECTION_NAME);
        events
            .insert_one(
                doc! {"_id":"source", "user_id":"owner", "status":"queued", "turn_id":null,
                "trace_request_id":&ingress, "delivery":{"version":1,"state":"pending"}},
            )
            .await
            .unwrap();
        take(&counts);
        let commands = DISABLED
            .scope(disabled, async {
                let mut session = db.client().start_session().await.unwrap();
                session.start_transaction().await.unwrap();
                let source = delivery::bind_in_session(
                    &db,
                    &mut session,
                    "source",
                    "owner",
                    "conversation",
                    &turn,
                )
                .await
                .unwrap();
                session.commit_transaction().await.unwrap();
                assert_eq!(source.as_deref(), Some(ingress.as_str()));
                source_link(&turn, source.as_deref().unwrap());
                scope(TurnContext::for_turn(&turn), async {
                    assert!(delivery::claim_send(&db, "source", "claim").await.unwrap());
                    delivery::finish(&db, "source", "claim", "unknown")
                        .await
                        .unwrap();
                    // An uncertain delivery must never be retried for tracing.
                    assert!(!delivery::claim_send(&db, "source", "retry").await.unwrap());
                })
                .await;
                take(&counts)
            })
            .await;
        measured.push(commands);
        db.drop().await.unwrap();
    }
    assert_eq!(measured[0], measured[1]);
    assert!(
        measured[0]
            .get("update:nyxbot_events")
            .copied()
            .unwrap_or_default()
            >= 3
    );
    eprintln!("turn trace queued delivery parity: {:?}", measured[0]);
}

#[tokio::test]
async fn refused_admission_does_not_emit_a_candidate_turn() {
    let (db, _) = database().await;
    engine::ensure_indexes(&db).await.unwrap();
    let owner = Uuid::new_v4().to_string();
    db.collection(crate::models::user::COLLECTION_NAME)
        .insert_one(crate::test_utils::test_user(
            &owner,
            crate::models::user::UserType::Person,
        ))
        .await
        .unwrap();
    let state = crate::test_utils::test_app_state(db.clone());
    let mut request = engine::TurnRequest {
        acknowledgement_id: None,
        retry_of: None,
        attachment_ids: vec![],
        agent_id: None,
        conversation_id: None,
        text: "private-input-canary".into(),
        model: None,
        access_mode: None,
    };
    let row = engine::begin_turn(&db, &owner, &request, &state.encryption_keys)
        .await
        .unwrap();
    request.conversation_id = Some(row.id);
    let (result, records) = capture(Ingress::new().scope(engine::begin_turn(
        &db,
        &owner,
        &request,
        &state.encryption_keys,
    )))
    .await;
    assert!(matches!(
        result,
        Err(crate::errors::AppError::AssistantTurnActive)
    ));
    assert!(records.iter().all(|record| record.get("turn_id").is_none()));
    assert!(
        !serde_json::to_string(&records)
            .unwrap()
            .contains("private-input")
    );
    db.drop().await.unwrap();
}

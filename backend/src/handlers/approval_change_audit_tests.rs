use super::*;
use crate::models::service_approval_config::ApprovalMode;
use crate::services::telegram_poller;
use axum::http::HeaderMap;

const CONFIGS: &str = crate::models::service_approval_config::COLLECTION_NAME;
const REQUESTS: &str = crate::models::approval_request::COLLECTION_NAME;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Change {
    Web,
    Telegram,
    Set,
    Delete,
}
use Change::*;

impl Change {
    fn event(self) -> &'static str {
        match self {
            Web | Telegram => "approval_decision",
            Set => "service_approval_config_set",
            Delete => "service_approval_config_deleted",
        }
    }

    fn metadata(self, f: &Fixture) -> serde_json::Value {
        match self {
            Web | Telegram => json!({"request_id":f.request_id,"service_id":f.service,
                "approved":true,"channel":if self == Web {"web"} else {"telegram"}}),
            Set => {
                json!({"service_id":f.service,"service_name":"fixture","user_service_id":f.service,
                "policy_owner_user_id":f.owner,"approval_required":true,"approval_mode":"per_request",
                "approval_rule_count":0,"default_effect":null})
            }
            Delete => {
                json!({"service_id":f.service,"user_service_id":f.service,"policy_owner_user_id":f.owner})
            }
        }
    }

    async fn prepare(self, f: &Fixture, grant_mode: bool) {
        let db = &f.state.db;
        let now = bson::DateTime::now();
        db.collection::<Document>(REQUESTS).insert_one(doc!{
            "_id":&f.request_id,"user_id":&f.owner,"service_id":&f.service,"service_slug":"fixture",
            "service_name":"Fixture","requester_type":"user","requester_id":&f.owner,
            "operation_summary":"Fixture","idempotency_key":"creation","status":"pending",
            "telegram_chat_id":123_i64,"created_at":now,"expires_at":bson::DateTime::from_chrono(chrono::Utc::now()+chrono::Duration::days(1)),
            "approval_mode":if grant_mode {"grant"} else {"per_request"}
        }).await.unwrap();
        if self == Delete || grant_mode {
            db.collection::<Document>(CONFIGS).insert_one(doc!{
                "_id":Uuid::new_v4().to_string(),"user_id":&f.owner,"service_id":&f.service,"service_name":"fixture",
                "approval_mode":"grant","approval_required":true,"created_at":now,"updated_at":now
            }).await.unwrap();
        }
        let us = crate::test_utils::test_user_service(
            &f.service, &f.owner, "fixture", "endpoint", None, None,
        );
        db.collection::<crate::models::user_service::UserService>(
            crate::models::user_service::COLLECTION_NAME,
        )
        .insert_one(us)
        .await
        .unwrap();
    }

    async fn call(self, f: &Fixture, auth: AuthUser) -> crate::errors::AppResult<()> {
        let state = State(f.state.clone());
        match self {
            Web => {
                let mut headers = HeaderMap::new();
                headers.insert("idempotency-key", "fixture".parse().unwrap());
                approvals::decide_request(
                    state,
                    auth,
                    Path(f.request_id.clone()),
                    Default::default(),
                    headers,
                    Json(approvals::DecideRequest {
                        approved: true,
                        duration_sec: None,
                    }),
                )
                .await
                .map(|_| ())
            }
            Telegram => {
                let update = serde_json::from_value(json!({"update_id":1,"callback_query":{
                    "id":"fixture","from":{"id":123},"data":format!("a:{}", f.request_id.replace('-', ""))}})).unwrap();
                telegram_poller::process_update(&f.state, update).await;
                Ok(())
            }
            Set => approvals::set_service_config(
                state,
                auth,
                Default::default(),
                Path(f.service.clone()),
                Query(Default::default()),
                Json(approvals::SetServiceApprovalConfigRequest {
                    disabled: None,
                    expected_revision: None,
                    grant_expiry_days: None,
                    approval_required: Some(true),
                    approval_mode: Some(ApprovalMode::PerRequest),
                    rules: None,
                    default_effect: None,
                }),
            )
            .await
            .map(|_| ()),
            Delete => approvals::delete_service_config(
                state,
                auth,
                Path(f.service.clone()),
                Query(Default::default()),
            )
            .await
            .map(|_| ()),
        }
    }
}

#[tokio::test]
async fn change_request_commands() {
    for (change, grant_mode) in [
        (Web, false),
        (Telegram, false),
        (Web, true),
        (Set, false),
        (Delete, false),
    ] {
        let f = Fixture::new().await;
        change.prepare(&f, grant_mode).await;
        let auth = f.auth();
        let started = std::time::Instant::now();
        REQUEST.scope((), change.call(&f, auth)).await.unwrap();
        let elapsed = started.elapsed();
        let commands = f.monitor.lock().unwrap().commands.clone();
        let transaction = f.monitor.lock().unwrap().transaction_commands.clone();
        let expected = match (change, grant_mode) {
            (Web | Telegram, false) => vec![
                ("findAndModify", REQUESTS),
                ("insert", INTENTS),
                ("commitTransaction", ""),
            ],
            (Web, true) => vec![
                ("findAndModify", REQUESTS),
                ("find", CONFIGS),
                ("find", "notification_channels"),
                ("insert", "notification_channels"),
                // The baseline also reads this policy's remembered-approval expiry.
                ("find", CONFIGS),
                ("insert", GRANTS),
                ("insert", INTENTS),
                ("commitTransaction", ""),
            ],
            // Callers without a revision keep the legacy reconciliation,
            // committed with the policy write.
            (Set, _) => vec![
                ("findAndModify", CONFIGS),
                ("update", GRANTS),
                ("update", REQUESTS),
                ("insert", INTENTS),
                ("commitTransaction", ""),
            ],
            (Delete, _) => vec![
                ("delete", CONFIGS),
                ("update", GRANTS),
                ("update", REQUESTS),
                ("insert", INTENTS),
                ("commitTransaction", ""),
            ],
            _ => unreachable!(),
        };
        assert_eq!(
            transaction
                .iter()
                .map(|(cmd, coll, _, _)| (cmd.as_str(), coll.as_str()))
                .collect::<Vec<_>>(),
            expected
        );
        assert!(
            transaction
                .iter()
                .all(|(_, _, sid, number)| sid == &transaction[0].2 && number == &transaction[0].3)
        );
        let outside = match change {
            Web | Telegram => 1,
            Set => 5,
            Delete => 2,
        };
        assert_eq!(commands.len(), expected.len() + outside, "{commands:?}");
        println!(
            "APPROVAL_REQUEST change={change:?} grant={grant_mode} commands={} elapsed_us={} {commands:?}",
            commands.len(),
            elapsed.as_micros()
        );
        crate::test_utils::drop_test_database(&f.state.db).await;
    }
}

async fn death_and_replay(change: Change) {
    let f = Fixture::new().await;
    change.prepare(&f, matches!(change, Web | Telegram)).await;
    let auth = f.auth();
    let context = crate::turn_trace::TurnContext::for_turn(&Uuid::new_v4().to_string());
    f.monitor.lock().unwrap().stop_after_source = true;
    let stopped = AssertUnwindSafe(REQUEST.scope(
        (),
        crate::turn_trace::scope(context, Box::pin(change.call(&f, auth.clone()))),
    ))
    .catch_unwind()
    .await;
    assert!(stopped.unwrap_err().is::<ProcessStopped>());
    let db = &f.state.db;
    match change {
        Web | Telegram => assert_eq!(f.rows(REQUESTS).await[0].get_str("status"), Ok("approved")),
        Set => assert_eq!(
            f.rows(CONFIGS).await[0].get_str("approval_mode"),
            Ok("per_request")
        ),
        Delete => assert!(f.rows(CONFIGS).await.is_empty()),
    }
    assert!(f.rows(AUDIT).await.is_empty());
    let intents = f.rows(INTENTS).await;
    println!(
        "APPROVAL_DEATH change={change:?} durable_intents={}",
        intents.len()
    );
    assert_eq!(intents.len(), 1);
    if matches!(change, Web | Telegram) {
        assert_eq!(f.rows(GRANTS).await.len(), 2);
        change.call(&f, auth.clone()).await.unwrap();
        assert_eq!(f.rows(INTENTS).await.len(), 1);
        assert_eq!(f.rows(GRANTS).await.len(), 2);
    }
    let actual = relay_twice(&f, &intents[0], change.event()).await;
    assert_eq!(f.rows(AUDIT).await.len(), 1);
    assert_eq!(actual.get_i64("seq"), Ok(1));
    assert_event_content(
        &f,
        actual,
        (change != Telegram).then_some(&auth),
        context,
        change.event(),
        change.metadata(&f),
    )
    .await;
    crate::test_utils::drop_test_database(db).await;
}

async fn relay_twice(f: &Fixture, intent: &Document, event: &str) -> Document {
    let db = &f.state.db;
    security_effect_service::sweep(db, &f.state.encryption_keys)
        .await
        .unwrap();
    db.collection::<Document>(INTENTS)
        .insert_one(intent)
        .await
        .unwrap();
    security_effect_service::sweep(db, &f.state.encryption_keys)
        .await
        .unwrap();
    assert!(f.rows(INTENTS).await.is_empty());
    let rows: Vec<Document> = db
        .collection(AUDIT)
        .find(doc! {"event_type":event})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    let [actual]: [Document; 1] = rows.try_into().expect("exactly one chained occurrence");
    assert_eq!(actual.get_str("_id"), intent.get_str("_id"));
    assert!(actual.get_i64("seq").unwrap() > 0);
    assert!(actual.get_str("entry_hash").is_ok());
    actual
}

async fn assert_event_content(
    f: &Fixture,
    mut actual: Document,
    auth: Option<&AuthUser>,
    context: Option<crate::turn_trace::TurnContext>,
    event: &'static str,
    data: serde_json::Value,
) {
    let db = &f.state.db;
    let data = Some(data);
    let written = audit_service::notify_on_audit_write_for_user(event, &f.owner);
    crate::turn_trace::scope(context, async {
        if let Some(auth) = auth {
            audit_service::log_for_user(db.clone(), auth, event, data);
        } else {
            audit_service::log_async(
                db.clone(),
                Some(f.owner.clone()),
                event.into(),
                data,
                None,
                None,
                None,
                None,
            );
        }
    })
    .await;
    let mut old = db
        .collection::<Document>(AUDIT)
        .find_one(doc! {"_id":written.await.unwrap()})
        .await
        .unwrap()
        .unwrap();
    for field in ["_id", "created_at", "seq", "prev_hash", "entry_hash"] {
        actual.remove(field);
        old.remove(field);
    }
    assert_eq!(actual, old);
}
#[tokio::test]
async fn web_decision_death_and_replay() {
    death_and_replay(Web).await;
}
#[tokio::test]
async fn telegram_decision_death_and_replay() {
    death_and_replay(Telegram).await;
}
#[tokio::test]
async fn config_set_death_and_replay() {
    death_and_replay(Set).await;
}
#[tokio::test]
async fn config_delete_death_and_replay() {
    death_and_replay(Delete).await;
}

#[tokio::test]
async fn web_decision_request_samples() {
    for grant_mode in [false, true] {
        let f = Fixture::new().await;
        Web.prepare(&f, grant_mode).await;
        let requests = f.state.db.collection::<Document>(REQUESTS);
        let mut micros = Vec::new();
        for trial in 0..12 {
            if trial != 0 {
                requests.update_one(doc!{"_id":&f.request_id},doc!{"$set":{"status":"pending"},"$unset":{"decided_at":"","decision_channel":"","decision_idempotency_key":""}}).await.unwrap();
            }
            let auth = f.auth();
            let start = std::time::Instant::now();
            Web.call(&f, auth).await.unwrap();
            if trial != 0 {
                micros.push(start.elapsed().as_micros());
            }
        }
        micros.sort_unstable();
        println!(
            "DECISION_SAMPLES grant={grant_mode} n={} median_us={} range_us={}..{}",
            micros.len(),
            micros[micros.len() / 2],
            micros[0],
            micros.last().unwrap()
        );
        crate::test_utils::drop_test_database(&f.state.db).await;
    }
}

async fn reject_writes(f: &Fixture, collection: &str) {
    f.state
        .db
        .run_command(doc! {"collMod":collection,"validator":{"fixture_required":{"$exists":true}}})
        .await
        .unwrap();
}

#[tokio::test]
async fn rejected_intent_aborts_each_source_and_grant() {
    for change in [Web, Telegram, Set, Delete] {
        let f = Fixture::new().await;
        change.prepare(&f, matches!(change, Web | Telegram)).await;
        let before = [
            f.rows(REQUESTS).await,
            f.rows(CONFIGS).await,
            f.rows(GRANTS).await,
        ];
        reject_writes(&f, INTENTS).await;
        let result = change.call(&f, f.auth()).await;
        if change != Telegram {
            assert!(matches!(
                result,
                Err(crate::errors::AppError::DatabaseError(_))
            ));
        }
        assert_eq!(
            before,
            [
                f.rows(REQUESTS).await,
                f.rows(CONFIGS).await,
                f.rows(GRANTS).await
            ]
        );
        assert!(
            f.rows(crate::models::notification_channel::COLLECTION_NAME)
                .await
                .is_empty()
        );
        assert!(f.rows(INTENTS).await.is_empty());
        assert!(f.rows(AUDIT).await.is_empty());
        crate::test_utils::drop_test_database(&f.state.db).await;
    }
}

#[tokio::test]
async fn rejected_grant_rolls_back_approval_and_channel_creation() {
    let f = Fixture::new().await;
    Web.prepare(&f, true).await;
    let before = f.rows(REQUESTS).await;
    reject_writes(&f, GRANTS).await;
    assert!(matches!(
        Web.call(&f, f.auth()).await,
        Err(crate::errors::AppError::DatabaseError(_))
    ));
    assert_eq!(f.rows(REQUESTS).await, before);
    assert_eq!(f.rows(GRANTS).await.len(), 1);
    assert!(
        f.rows(crate::models::notification_channel::COLLECTION_NAME)
            .await
            .is_empty()
    );
    assert!(f.rows(INTENTS).await.is_empty());
    crate::test_utils::drop_test_database(&f.state.db).await;
}

#[tokio::test]
async fn legacy_mode_conflict_commits_expiry_without_an_approval() {
    let f = Fixture::new().await;
    Web.prepare(&f, true).await;
    f.state
        .db
        .collection::<Document>(CONFIGS)
        .delete_one(doc! {})
        .await
        .unwrap();
    let result = Web.call(&f, f.auth()).await;
    assert!(
        matches!(result,Err(crate::errors::AppError::Conflict(ref message)) if message=="Service approval mode has changed; this request is no longer valid")
    );
    let row = &f.rows(REQUESTS).await[0];
    assert_eq!(row.get_str("status"), Ok("expired"));
    assert_eq!(row.get_str("decision_channel"), Ok("web"));
    assert_eq!(row.get_str("decision_idempotency_key"), Ok("fixture"));
    assert!(row.get_datetime("decided_at").is_ok());
    assert_eq!(f.rows(GRANTS).await.len(), 1);
    assert!(f.rows(INTENTS).await.is_empty());
    crate::test_utils::drop_test_database(&f.state.db).await;
}

#[tokio::test]
async fn cleanup_errors_roll_back_policy_and_retry_commits_one_audit_occurrence() {
    for change in [Set, Delete] {
        let f = Fixture::new().await;
        change.prepare(&f, true).await;
        let configs = f.rows(CONFIGS).await;
        let grants = f.rows(GRANTS).await;
        let requests = f.rows(REQUESTS).await;
        reject_writes(&f, GRANTS).await;
        assert!(matches!(
            change.call(&f, f.auth()).await,
            Err(crate::errors::AppError::DatabaseError(_))
        ));
        assert_eq!(f.rows(CONFIGS).await, configs);
        assert_eq!(f.rows(GRANTS).await, grants);
        assert_eq!(f.rows(REQUESTS).await, requests);
        assert!(f.rows(INTENTS).await.is_empty());
        assert!(f.rows(AUDIT).await.is_empty());
        f.state
            .db
            .run_command(doc! {"collMod":GRANTS,"validator":{}})
            .await
            .unwrap();
        change.call(&f, f.auth()).await.unwrap();
        if change == Set {
            assert_eq!(
                f.rows(CONFIGS).await[0].get_str("approval_mode"),
                Ok("per_request")
            );
        } else {
            assert!(f.rows(CONFIGS).await.is_empty());
        }
        assert_eq!(f.rows(GRANTS).await[0].get_bool("revoked"), Ok(true));
        assert_eq!(f.rows(REQUESTS).await[0].get_str("status"), Ok("expired"));
        let intents = f.rows(INTENTS).await;
        assert_eq!(intents.len(), 1);

        // A legacy DELETE still reports not found after success and appends
        // nothing. A legacy PUT keeps its pre-revision contract: every save is
        // written and audited, so an identical retry appends one new intent.
        let retry = change.call(&f, f.auth()).await;
        if change == Delete {
            assert!(matches!(retry, Err(crate::errors::AppError::NotFound(_))));
            assert_eq!(f.rows(INTENTS).await, intents);
            relay_twice(&f, &intents[0], change.event()).await;
            assert_eq!(f.rows(AUDIT).await.len(), 1);
        } else {
            retry.unwrap();
            let after = f.rows(INTENTS).await;
            assert_eq!(after.len(), 2);
            assert!(after.contains(&intents[0]));
            // Each save is chained exactly once, even when its intent replays.
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
            let mut audited: Vec<_> = f
                .rows(AUDIT)
                .await
                .iter()
                .map(|row| row.get_str("_id").unwrap().to_owned())
                .collect();
            audited.sort();
            let mut expected: Vec<_> = after
                .iter()
                .map(|row| row.get_str("_id").unwrap().to_owned())
                .collect();
            expected.sort();
            assert_eq!(audited, expected);
        }
        crate::test_utils::drop_test_database(&f.state.db).await;
    }
}

#[tokio::test]
async fn web_rejection_replays_the_stored_answer_without_a_grant() {
    let f = Fixture::new().await;
    Web.prepare(&f, true).await;
    let mut headers = HeaderMap::new();
    headers.insert("idempotency-key", "fixture-reject".parse().unwrap());
    for approved in [false, false, true] {
        let result = approvals::decide_request(
            State(f.state.clone()),
            f.auth(),
            Path(f.request_id.clone()),
            Default::default(),
            headers.clone(),
            Json(approvals::DecideRequest {
                approved,
                duration_sec: None,
            }),
        )
        .await;
        if approved {
            assert!(matches!(result, Err(crate::errors::AppError::Conflict(_))));
        } else {
            assert_eq!(result.unwrap().status, "rejected");
        }
    }
    assert_eq!(f.rows(GRANTS).await.len(), 1);
    let intents = f.rows(INTENTS).await;
    assert_eq!(intents.len(), 1);
    let data = intents[0]
        .get_document("event")
        .unwrap()
        .get_document("event_data")
        .unwrap();
    assert_eq!(data.get_bool("approved"), Ok(false));
    crate::test_utils::drop_test_database(&f.state.db).await;
}

#[tokio::test]
async fn web_and_telegram_contend_for_one_transition() {
    let f = Fixture::new().await;
    Web.prepare(&f, true).await;
    let (web, callback) = tokio::join!(Web.call(&f, f.auth()), Telegram.call(&f, f.auth()));
    callback.unwrap();
    let row = &f.rows(REQUESTS).await[0];
    let winner = row.get_str("decision_channel").unwrap();
    if winner == "web" {
        web.unwrap();
    } else {
        assert_eq!(winner, "telegram");
        assert!(matches!(web, Err(crate::errors::AppError::Conflict(_))));
    }
    assert_eq!(f.rows(GRANTS).await.len(), 2);
    let intents = f.rows(INTENTS).await;
    assert_eq!(intents.len(), 1);
    assert_eq!(
        intents[0]
            .get_document("event")
            .unwrap()
            .get_document("event_data")
            .unwrap()
            .get_str("channel"),
        Ok(winner)
    );
    crate::test_utils::drop_test_database(&f.state.db).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn creating_policy_loser_returns_the_winner_and_runs_its_cleanup_without_an_intent() {
    let f = Fixture::new().await;
    Set.prepare(&f, false).await;
    let db = &f.state.db;
    let policies = db.collection::<Document>(CONFIGS);
    policies
        .create_index(
            mongodb::IndexModel::builder()
                .keys(doc! {"user_id":1,"service_id":1})
                .options(
                    mongodb::options::IndexOptions::builder()
                        .unique(true)
                        .build(),
                )
                .build(),
        )
        .await
        .unwrap();
    let now = bson::DateTime::now();
    let winner = doc! {"_id":"winner","user_id":&f.owner,"service_id":&f.service,"service_name":"Winner",
    "approval_mode":"per_request","approval_required":false,"created_at":now,"updated_at":now};
    // Interleave a mixed-fleet creator exactly between the handler's absence
    // read and its upsert. The real compound unique index rejects the loser.
    let other =
        crate::test_utils::connect_transaction_test_database("approval_policy_creator").await;
    interleave_insert(&f, &other, "findAndModify", CONFIGS, winner.clone());
    let response = REQUEST
        .scope(
            (),
            approvals::set_service_config(
                State(f.state.clone()),
                f.auth(),
                Default::default(),
                Path(f.service.clone()),
                Query(Default::default()),
                Json(approvals::SetServiceApprovalConfigRequest {
                    disabled: None,
                    expected_revision: None,
                    grant_expiry_days: None,
                    approval_required: Some(true),
                    approval_mode: Some(ApprovalMode::Grant),
                    rules: None,
                    default_effect: None,
                }),
            ),
        )
        .await
        .unwrap();
    assert!(!response.approval_required);
    assert_eq!(response.approval_mode, ApprovalMode::PerRequest);
    assert_eq!(f.rows(CONFIGS).await, vec![winner]);
    assert_eq!(f.rows(GRANTS).await[0].get_bool("revoked"), Ok(true));
    assert!(f.rows(INTENTS).await.is_empty());
    crate::test_utils::drop_test_database(&other).await;
    crate::test_utils::drop_test_database(db).await;
}

fn interleave_insert(
    f: &Fixture,
    other: &mongodb::Database,
    command: &'static str,
    collection: &'static str,
    row: Document,
) {
    let target = other
        .client()
        .database(f.state.db.name())
        .collection::<Document>(collection);
    f.monitor.lock().unwrap().before_insert = Some(InterleavedInsert {
        command,
        collection,
        insert: Box::new(move || {
            tokio::task::block_in_place(|| {
                tokio::runtime::Handle::current().block_on(async {
                    target.insert_one(row).await.unwrap();
                })
            });
        }),
    });
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_channel_creation_uses_the_winners_grant_expiry() {
    const CHANNELS: &str = crate::models::notification_channel::COLLECTION_NAME;
    let f = Fixture::new().await;
    Web.prepare(&f, true).await;
    let db = &f.state.db;
    db.collection::<Document>(CHANNELS)
        .create_index(
            mongodb::IndexModel::builder()
                .keys(doc! {"user_id":1})
                .options(
                    mongodb::options::IndexOptions::builder()
                        .unique(true)
                        .build(),
                )
                .build(),
        )
        .await
        .unwrap();
    let other =
        crate::test_utils::connect_transaction_test_database("approval_channel_creator").await;
    let now = bson::DateTime::now();
    let winner = doc! {"_id":"winner","user_id":&f.owner,"grant_expiry_days":11,"created_at":now,"updated_at":now};
    interleave_insert(&f, &other, "insert", CHANNELS, winner.clone());
    REQUEST.scope((), Web.call(&f, f.auth())).await.unwrap();
    assert_eq!(f.rows(CHANNELS).await, vec![winner]);
    let grant = db
        .collection::<Document>(GRANTS)
        .find_one(doc! {"approval_request_id":&f.request_id})
        .await
        .unwrap()
        .unwrap();
    let lifetime = grant.get_datetime("expires_at").unwrap().to_chrono()
        - grant.get_datetime("granted_at").unwrap().to_chrono();
    assert_eq!(lifetime, chrono::Duration::days(11));
    assert_eq!(f.rows(GRANTS).await.len(), 2);
    assert_eq!(f.rows(INTENTS).await.len(), 1);
    crate::test_utils::drop_test_database(&other).await;
    crate::test_utils::drop_test_database(db).await;
}

#[path = "approval_nyxbot_audit_tests.rs"]
mod nyxbot;

#[path = "approval_causal_tests.rs"]
mod causal;

#[tokio::test]
async fn web_decision_refuses_already_decided_and_expired_requests() {
    use crate::errors::AppError;

    for expired in [false, true] {
        let f = Fixture::new().await;
        Web.prepare(&f, false).await;
        if expired {
            f.state
                .db
                .collection::<Document>(REQUESTS)
                .update_one(
                    doc! {"_id": &f.request_id},
                    doc! {"$set": {"status": "expired"}},
                )
                .await
                .unwrap();
        } else {
            Telegram.call(&f, f.auth()).await.unwrap();
            assert_eq!(
                f.rows(REQUESTS).await[0].get_str("decision_channel"),
                Ok("telegram")
            );
        }
        let before = f.rows(REQUESTS).await;
        let error = Web.call(&f, f.auth()).await.unwrap_err();
        if expired {
            assert!(matches!(error, AppError::Forbidden(s) if s == "Approval request expired"));
        } else {
            assert!(matches!(error, AppError::Conflict(s) if s == "already_decided"));
        }
        assert_eq!(f.rows(REQUESTS).await, before);
        assert_eq!(f.rows(INTENTS).await.len(), usize::from(!expired));
        crate::test_utils::drop_test_database(&f.state.db).await;
    }
}

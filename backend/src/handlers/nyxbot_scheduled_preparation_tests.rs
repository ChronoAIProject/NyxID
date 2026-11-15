use super::*;
use crate::models::trigger_run::{COLLECTION_NAME as RUNS, RunOutcome, TriggerRun};
use crate::services::trigger_schedule as schedules;

async fn scheduled_fixture() -> (Fixture, MockServer, TriggerRun) {
    let (f, server) = image_fixture("telegram", "direct").await;
    schedules::indexes(&f.state.db).await.unwrap();
    let target = thread(&f).await;
    f.state
        .db
        .collection::<bson::Document>(CONVERSATIONS)
        .update_one(
            doc! {"_id":&f.event.conversation_id},
            doc! {"$set":{"active_turn":null}},
        )
        .await
        .unwrap();
    let run: TriggerRun = bson::from_document(doc! {
        "_id":Uuid::new_v4().to_string(),"trigger_id":Uuid::new_v4().to_string(),"user_id":OWNER,
        "scheduled_at":bson::DateTime::now(),"deadline":bson::DateTime::from_chrono(Utc::now()+ChronoDuration::hours(1)),
        "outcome":"started","thread_id":&f.event.conversation_id,"turn_id":&f.event.turn_id,
        "deliver_to":{"type":"chat","chat_id":&target.id},
        "fence":"test","lease_until":bson::DateTime::now(),
        "expires_at":bson::DateTime::from_chrono(Utc::now()+ChronoDuration::days(7)),
    }).unwrap();
    f.state
        .db
        .collection::<TriggerRun>(RUNS)
        .insert_one(&run)
        .await
        .unwrap();
    f.state.db.collection::<bson::Document>(schedules::WORK).insert_one(doc! {
        "_id":&run.id,"trigger_id":&run.trigger_id,"kind":"run","at":bson::DateTime::now(),"fence":""
    }).await.unwrap();
    Mock::given(path("/bottoken/sendMessage"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"ok":true,"result":{"message_id":100}})),
        )
        .mount(&server)
        .await;
    (f, server, run)
}

async fn settle(f: &Fixture, run: &TriggerRun) {
    let row = engine::get(
        &f.state.db,
        OWNER,
        f.event.conversation_id.as_deref().unwrap(),
    )
    .await
    .unwrap();
    let saved = f
        .state
        .db
        .collection::<crate::models::assistant_message::AssistantMessage>(
            crate::models::assistant_message::COLLECTION_NAME,
        )
        .find_one(doc! {"conversation_id":&row.id,"turn_id":&run.turn_id,"role":"assistant"})
        .await
        .unwrap()
        .unwrap();
    crate::handlers::trigger_scheduler::settle_result(&f.state, &row, run, &saved.text, None)
        .await
        .unwrap();
}

#[tokio::test]
async fn scheduled_text_preparation_crash_survivor_sends_once() {
    let (f, server, run) = scheduled_fixture().await;
    crate::services::durable_wake_tests::drop_after("text_part_prepared", settle(&f, &run)).await;
    assert!(server.received_requests().await.unwrap().is_empty());
    f.state
        .db
        .collection::<bson::Document>(crate::models::channel_text_delivery::COLLECTION_NAME)
        .update_many(
            doc! {},
            doc! {"$set":{
                "parts.0.preparation.lease_until":bson::DateTime::from_millis(0),
            }},
        )
        .await
        .unwrap();
    f.state.db.collection::<bson::Document>(RUNS).update_one(doc! {"_id":&run.id},doc! {"$set":{
        "result_delivery.lease_until":bson::DateTime::from_millis(0),"result_delivery.retry_at":bson::DateTime::from_millis(0),
    }}).await.unwrap();
    f.state
        .db
        .collection::<bson::Document>(schedules::WORK)
        .update_one(
            doc! {"_id":&run.id},
            doc! {"$set":{"at":bson::DateTime::from_millis(0)}},
        )
        .await
        .unwrap();
    crate::handlers::trigger_scheduler::tick(&f.state)
        .await
        .unwrap();
    crate::handlers::trigger_scheduler::tick(&f.state)
        .await
        .unwrap();
    let saved = f
        .state
        .db
        .collection::<TriggerRun>(RUNS)
        .find_one(doc! {"_id":&run.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        server.received_requests().await.unwrap().len(),
        1,
        "survivor must send the proven-unsent scheduled part once"
    );
    assert_eq!(saved.outcome, RunOutcome::Completed);
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn scheduled_text_preparation_normal_send_command_counts() {
    let (mut f, server, run) = scheduled_fixture().await;
    let (db, counts) = crate::services::durable_wake_tests::monitored(&f.state.db).await;
    f.state.db = db;
    settle(&f, &run).await;
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
    let counts = crate::services::durable_wake_tests::take(&counts);
    println!(
        "SCHEDULED_TEXT_COMMANDS total={} {counts:?}",
        counts.values().sum::<usize>()
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn scheduled_text_preparation_preserves_the_prefix_and_never_reopens_the_provider_boundary() {
    for (site, expected_prefix, recover) in [
        ("text_suffix_prepared", 1, true),
        ("text_part_dispatch_committed", 0, false),
        ("text_part_provider_accepted", 1, false),
    ] {
        let (f, server, run) = scheduled_fixture().await;
        let text: String = (0..600)
            .map(|i| format!("Complete saved paragraph {i}.\n\n"))
            .collect();
        f.state
            .db
            .collection::<bson::Document>(crate::models::assistant_message::COLLECTION_NAME)
            .update_one(
                doc! {"conversation_id":&run.thread_id,"turn_id":&run.turn_id,"role":"assistant"},
                doc! {"$set":{"text":text}},
            )
            .await
            .unwrap();
        crate::services::durable_wake_tests::drop_after(site, settle(&f, &run)).await;
        assert_eq!(
            server.received_requests().await.unwrap().len(),
            expected_prefix
        );
        f.state
            .db
            .collection::<bson::Document>(crate::models::channel_text_delivery::COLLECTION_NAME)
            .update_many(
                doc! {},
                doc! {"$set":{
                    "parts.$[p].preparation.lease_until":bson::DateTime::from_millis(0),
                }},
            )
            .array_filters(vec![doc! {"p.state":"pending","p.preparation.version":1}])
            .await
            .unwrap();
        f.state.db.collection::<bson::Document>(RUNS).update_one(doc! {"_id":&run.id},doc! {"$set":{
            "result_delivery.lease_until":bson::DateTime::from_millis(0),"result_delivery.retry_at":bson::DateTime::from_millis(0),
        }}).await.unwrap();
        f.state
            .db
            .collection::<bson::Document>(schedules::WORK)
            .update_one(
                doc! {"_id":&run.id},
                doc! {"$set":{"at":bson::DateTime::from_millis(0)}},
            )
            .await
            .unwrap();
        crate::handlers::trigger_scheduler::tick(&f.state)
            .await
            .unwrap();
        let sent = server.received_requests().await.unwrap();
        if recover {
            assert!(sent.len() > 1);
            assert_eq!(sent.iter().filter(|r| r.body == sent[0].body).count(), 1);
        } else {
            assert_eq!(sent.len(), expected_prefix);
        }
        crate::handlers::trigger_scheduler::tick(&f.state)
            .await
            .unwrap();
        assert_eq!(server.received_requests().await.unwrap().len(), sent.len());
        let saved = f
            .state
            .db
            .collection::<TriggerRun>(RUNS)
            .find_one(doc! {"_id":&run.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            saved.outcome,
            if recover {
                RunOutcome::Completed
            } else {
                RunOutcome::Failed
            }
        );
        f.state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn scheduled_text_preparation_revoked_destination_stops_before_the_first_effect() {
    let (f, server, run) = scheduled_fixture().await;
    crate::services::durable_wake_tests::resume_after(
        "text_part_prepared",
        settle(&f, &run),
        async {
            f.state
                .db
                .collection::<bson::Document>(THREADS)
                .update_one(
                    doc! {"channel_id":&f.event.channel_id},
                    doc! {"$set":{"allow_posts":false}},
                )
                .await
                .unwrap();
        },
    )
    .await;
    assert!(server.received_requests().await.unwrap().is_empty());
    let saved = f
        .state
        .db
        .collection::<TriggerRun>(RUNS)
        .find_one(doc! {"_id":&run.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(saved.outcome, RunOutcome::Failed);
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn scheduled_text_authority_observation_is_discarded_between_effects() {
    let (f, server, run) = scheduled_fixture().await;
    f.state
        .db
        .collection::<bson::Document>(crate::models::assistant_message::COLLECTION_NAME)
        .update_one(
            doc! {"conversation_id":&run.thread_id,"turn_id":&run.turn_id,"role":"assistant"},
            doc! {"$set":{"text":"Saved result paragraph.\n\n".repeat(600)}},
        )
        .await
        .unwrap();
    crate::services::durable_wake_tests::resume_after(
        "text_suffix_prepared",
        settle(&f, &run),
        async {
            f.state
                .db
                .collection::<bson::Document>(THREADS)
                .update_one(
                    doc! {"channel_id":&f.event.channel_id},
                    doc! {"$set":{"allow_posts":false}},
                )
                .await
                .unwrap();
        },
    )
    .await;
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
    f.state.db.drop().await.unwrap();
}

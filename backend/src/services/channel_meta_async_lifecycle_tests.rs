use super::*;
use crate::{
    models::async_service_operation::{AsyncServiceOperation, COLLECTION_NAME as WATCHES},
    services::{assistant_nyxagent as engine, async_service_operation as watches},
};

async fn ready(
    f: &Fixture,
    meta: bool,
    home: Option<&str>,
) -> (AssistantConversation, AsyncServiceOperation) {
    let db = &f.state.db;
    engine::ensure_indexes(db).await.unwrap();
    let original =
        card_sources::begin(f, home, meta.then(|| card_sources::context(&f.bot)), false).await;
    let chat = crate::services::assistant_authority_tests::chat_for_row(
        &f.state,
        &f.bot.user_id,
        &original,
    )
    .await;
    let catalog = async_catalog();
    let watch = watches::reserve(db, &chat, &catalog, &catalog.endpoints[0])
        .await
        .unwrap();
    watches::ready(
        db,
        &watch,
        Some(&f.state.encryption_keys),
        "completed",
        Some("Retained operation result"),
    )
    .await
    .unwrap();
    finish_source_turn(&f.state, &original, "Submitted operation").await;
    let watch = db
        .collection::<AsyncServiceOperation>(WATCHES)
        .find_one(doc! {"_id":&watch.id})
        .await
        .unwrap()
        .unwrap();
    (original, watch)
}
async fn due(f: &Fixture, id: &str) {
    f.state.db.collection::<Document>(WATCHES).update_one(doc! {"_id":id},doc! {"$set":{"poll_at":bson::DateTime::from_millis(0),"lease_id":null,"lease_until":null}}).await.unwrap();
}
async fn sweep(f: &Fixture) {
    crate::handlers::mcp_transport::async_operations::sweep(&f.state)
        .await
        .unwrap();
}

#[tokio::test]
async fn meta_async_suspend_resume_releases_ready_and_queued_delivery_quota_and_preserves_web_event()
 {
    for queued in [false, true] {
        let f = fixture().await;
        let db = &f.state.db;
        let (original, watch) = ready(&f, true, None).await;
        if queued {
            assert!(watches::enqueue(db, &watch).await.unwrap());
        }
        let (_, web) = ready(&f, false, Some(&original.id)).await;
        assert!(watches::enqueue(db, &web).await.unwrap());
        crate::services::channel_meta_service::set_bot_delivery(db, &f.bot, &f.bot.user_id, false)
            .await
            .unwrap();
        let held = db
            .collection::<ChannelBot>(BOTS)
            .find_one(doc! {"_id":&f.bot.id})
            .await
            .unwrap()
            .unwrap();
        crate::services::channel_meta_service::set_bot_delivery(db, &held, &f.bot.user_id, true)
            .await
            .unwrap();
        if queued {
            let request: engine::TurnRequest = serde_json::from_value(
                serde_json::json!({"conversation_id":&original.id,"text":"Process queued result"}),
            )
            .unwrap();
            let mut start = engine::TurnStart::from(&request);
            start.origin = crate::models::assistant_conversation::TurnOrigin::Event;
            assert!(
                engine::begin_turn(db, &f.bot.user_id, start, &f.state.encryption_keys)
                    .await
                    .is_err()
            );
        }
        due(&f, &watch.id).await;
        sweep(&f).await;
        let terminal = db
            .collection::<AsyncServiceOperation>(WATCHES)
            .find_one(doc! {"_id":&watch.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(terminal.state, "cancelled");
        assert_eq!(terminal.reason.as_deref(), Some("authority_lost"));
        assert!(terminal.result_encrypted.is_none());
        assert_eq!(db.collection::<Document>(WATCHES).count_documents(doc! {"user_id":&f.bot.user_id,"state":{"$in":["submitting","waiting","cancelling","ready","queued"]}}).await.unwrap(),1);
        let parent = engine::get(db, &f.bot.user_id, &original.id).await.unwrap();
        assert!(
            !parent
                .pending_events
                .iter()
                .any(|event| event.id == watch.id)
        );
        assert!(parent.pending_events.iter().any(|event| event.id == web.id));
        let recovered = card_sources::begin(&f, Some(&original.id), None, true).await;
        assert_eq!(recovered.active_turn.as_ref().unwrap().events[0].id, web.id);
        assert!(f.server.received_requests().await.unwrap().is_empty());
        db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn meta_async_deadline_and_expired_queued_payload_end_delivery_without_remote_effects() {
    for expired in [false, true] {
        let f = fixture().await;
        let db = &f.state.db;
        let (original, watch) = ready(&f, true, None).await;
        if expired {
            assert!(watches::enqueue(db, &watch).await.unwrap());
        }
        let field = if expired { "created_at" } else { "deadline" };
        let value = if expired {
            bson::DateTime::from_chrono(chrono::Utc::now() - chrono::Duration::hours(5))
        } else {
            bson::DateTime::from_millis(0)
        };
        db.collection::<Document>(WATCHES)
            .update_one(doc! {"_id":&watch.id}, doc! {"$set":{field:value}})
            .await
            .unwrap();
        due(&f, &watch.id).await;
        sweep(&f).await;
        let terminal = db
            .collection::<AsyncServiceOperation>(WATCHES)
            .find_one(doc! {"_id":&watch.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(terminal.state, "cancelled");
        assert_eq!(
            terminal.reason.as_deref(),
            Some(if expired { "result_expired" } else { "timeout" })
        );
        assert!(terminal.result_encrypted.is_none());
        assert!(
            !engine::get(db, &f.bot.user_id, &original.id)
                .await
                .unwrap()
                .pending_events
                .iter()
                .any(|event| event.id == watch.id)
        );
        assert!(f.server.received_requests().await.unwrap().is_empty());
        db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn meta_async_scope_read_error_and_unresolved_proof_retry_without_discarding_ready_result() {
    let f = fixture().await;
    let db = &f.state.db;
    let (_, watch) = ready(&f, true, None).await;
    let before = db
        .collection::<Document>(WATCHES)
        .find_one(doc! {"_id":&watch.id})
        .await
        .unwrap()
        .unwrap();
    db.collection::<Document>(GRANT_SCOPES)
        .update_one(
            doc! {"_id":f.scope.get("_id").cloned()},
            doc! {"$set":{"from_generation":"unresolved"}},
        )
        .await
        .unwrap();
    due(&f, &watch.id).await;
    sweep(&f).await;
    let unresolved = db
        .collection::<Document>(WATCHES)
        .find_one(doc! {"_id":&watch.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(unresolved.get_str("state"), Ok("ready"));
    assert_eq!(
        unresolved.get("result_encrypted"),
        before.get("result_encrypted")
    );
    db.collection::<Document>(GRANT_SCOPES)
        .update_one(
            doc! {"_id":f.scope.get("_id").cloned()},
            doc! {"$set":{"from_generation":f.scope.get("from_generation").cloned()}},
        )
        .await
        .unwrap();
    // A view that raises on this fixture's scope read produces a real Mongo
    // command error without changing server failpoints or other test databases.
    let original = format!("{}.{GRANT_SCOPES}", db.name());
    let backing = format!("{}.scope_read_backing", db.name());
    db.client()
        .database("admin")
        .run_command(doc! {"renameCollection":&original,"to":&backing})
        .await
        .unwrap();
    db.run_command(doc! {"create":GRANT_SCOPES,"viewOn":"scope_read_backing","pipeline":[{"$match":{"$expr":{"$eq":[{"$divide":["$from_generation",0]},0]}}}]}).await.unwrap();
    due(&f, &watch.id).await;
    let claimed = watches::claim(db).await.unwrap().unwrap();
    assert!(matches!(
        watches::meta_lineage::retire_unavailable(db, &claimed).await,
        Err(crate::errors::AppError::DatabaseError(_))
    ));
    watches::defer(db, &claimed).await.unwrap();
    let failed = db
        .collection::<Document>(WATCHES)
        .find_one(doc! {"_id":&watch.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(failed.get_str("state"), Ok("ready"));
    assert_eq!(
        failed.get("result_encrypted"),
        before.get("result_encrypted")
    );
    db.collection::<Document>(GRANT_SCOPES)
        .drop()
        .await
        .unwrap();
    db.client()
        .database("admin")
        .run_command(doc! {"renameCollection":&backing,"to":&original})
        .await
        .unwrap();
    due(&f, &watch.id).await;
    sweep(&f).await;
    assert_eq!(
        db.collection::<Document>(WATCHES)
            .find_one(doc! {"_id":&watch.id})
            .await
            .unwrap()
            .unwrap()
            .get_str("state"),
        Ok("queued")
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn admitted_meta_result_survives_deadline_before_or_after_payload_read_and_settles_delivered()
{
    for read_first in [true, false] {
        let f = fixture().await;
        let db = &f.state.db;
        let (original, watch) = ready(&f, true, None).await;
        assert!(watches::enqueue(db, &watch).await.unwrap());
        let admitted = card_sources::begin(&f, Some(&original.id), None, true).await;
        assert_eq!(
            admitted.active_turn.as_ref().unwrap().events[0].id,
            watch.id
        );
        if read_first {
            assert!(
                watches::input_context(&f.state, &admitted)
                    .await
                    .unwrap()
                    .contains("Retained operation result")
            );
        }
        db.collection::<Document>(WATCHES)
            .update_one(
                doc! {"_id":&watch.id},
                doc! {"$set":{"deadline":bson::DateTime::from_millis(0)}},
            )
            .await
            .unwrap();
        due(&f, &watch.id).await;
        sweep(&f).await;
        let active = db
            .collection::<AsyncServiceOperation>(WATCHES)
            .find_one(doc! {"_id":&watch.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(active.state, "queued");
        assert!(active.result_encrypted.is_some());
        assert!(
            watches::input_context(&f.state, &admitted)
                .await
                .unwrap()
                .contains("Retained operation result")
        );
        finish_source_turn(&f.state, &admitted, "Consumed result successfully").await;
        let delivered = db
            .collection::<AsyncServiceOperation>(WATCHES)
            .find_one(doc! {"_id":&watch.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(delivered.state, "delivered");
        assert!(delivered.result_encrypted.is_none());
        db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn admitted_meta_result_still_obeys_payload_retention_and_grant_erasure() {
    for expired in [true, false] {
        let f = fixture().await;
        let db = &f.state.db;
        let (original, watch) = ready(&f, true, None).await;
        assert!(watches::enqueue(db, &watch).await.unwrap());
        let admitted = card_sources::begin(&f, Some(&original.id), None, true).await;
        assert_eq!(
            admitted.active_turn.as_ref().unwrap().events[0].id,
            watch.id
        );
        if expired {
            db.collection::<Document>(WATCHES).update_one(doc! {"_id":&watch.id},doc! {"$set":{"created_at":bson::DateTime::from_chrono(chrono::Utc::now()-chrono::Duration::hours(5))}}).await.unwrap();
        } else {
            db.collection::<Document>(GRANT_SCOPES)
                .update_one(
                    doc! {"_id":f.scope.get("_id").cloned()},
                    doc! {"$set":{"erasure_requested":true}},
                )
                .await
                .unwrap();
        }
        due(&f, &watch.id).await;
        sweep(&f).await;
        let terminal = db
            .collection::<AsyncServiceOperation>(WATCHES)
            .find_one(doc! {"_id":&watch.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(terminal.state, "cancelled");
        assert!(terminal.result_encrypted.is_none());
        assert_eq!(
            terminal.reason.as_deref(),
            Some(if expired {
                "result_expired"
            } else {
                "authority_lost"
            })
        );
        db.drop().await.unwrap();
    }
}

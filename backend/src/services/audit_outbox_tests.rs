use super::*;
use crate::{
    models::{
        audit_log::COLLECTION_NAME as AUDIT,
        audit_outbox::{AuditOutbox, COLLECTION_NAME as OUTBOX, SystemAuditActor},
    },
    services::{audit_chain_service as chain, audit_state_service as state},
    test_utils::connect_transaction_test_database,
};
use chrono::Utc;

const KEY: [u8; 32] = [2; 32];

fn event() -> AuditEvent {
    AuditEvent {
        operation_id: Uuid::new_v4().to_string(),
        stage: AuditStage::Committed,
        principal: AuditPrincipal::System {
            actor: SystemAuditActor::Maintenance,
        },
        owner: AuditOwner::Platform,
        action: "audit_outbox_test",
        target_id: Some(Uuid::new_v4().to_string()),
    }
}

async fn enqueue(db: &Database, id: &str, event: &AuditEvent) -> AppResult<String> {
    let mut session = db.client().start_session().await?;
    session
        .start_transaction()
        .write_concern(state::majority())
        .await?;
    let result = enqueue_in_session(db, &mut session, id, event).await?;
    state::commit(&mut session).await?;
    Ok(result)
}

#[tokio::test]
async fn audit_pending_replay_refuses_missing_or_rolled_back_allocator() {
    let db = connect_transaction_test_database("audit_pending_replay_boundary").await;
    let event = event();
    let id = Uuid::new_v4().to_string();
    enqueue(&db, &id, &event).await.unwrap();
    db.collection::<bson::Document>(STATE)
        .update_one(
            doc! {"singleton": true},
            doc! {"$set": {"next_order": 0_i64}},
        )
        .await
        .unwrap();
    assert!(matches!(
        enqueue(&db, &id, &event).await,
        Err(AppError::AuditIntegrityConflict)
    ));
    db.collection::<bson::Document>(STATE)
        .delete_many(doc! {})
        .await
        .unwrap();
    assert!(matches!(
        enqueue(&db, &id, &event).await,
        Err(AppError::AuditNotInitialized)
    ));
}

#[tokio::test]
async fn audit_materialized_replay_refuses_head_rollback() {
    let db = connect_transaction_test_database("audit_pending_materialized_rollback").await;
    let event = event();
    let id = Uuid::new_v4().to_string();
    enqueue(&db, &id, &event).await.unwrap();
    assert!(materialize_one(&db, &KEY).await.unwrap());
    db.collection::<bson::Document>(STATE)
        .update_one(doc! {"singleton": true}, doc! {"$set": {"head_seq": 0_i64}})
        .await
        .unwrap();
    assert!(matches!(
        enqueue(&db, &id, &event).await,
        Err(AppError::AuditIntegrityConflict)
    ));
}

#[tokio::test]
async fn audit_pending_reservation_refuses_corrupt_tail_and_missing_delivery() {
    let db = connect_transaction_test_database("audit_pending_delivery_boundary").await;
    let event = event();
    let id = Uuid::new_v4().to_string();
    enqueue(&db, &id, &event).await.unwrap();
    assert!(materialize_one(&db, &KEY).await.unwrap());
    db.collection::<bson::Document>(OUTBOX)
        .delete_many(doc! {})
        .await
        .unwrap();
    assert!(matches!(
        enqueue(&db, &id, &event).await,
        Err(AppError::AuditIntegrityConflict)
    ));
    db.collection::<bson::Document>(AUDIT)
        .delete_many(doc! {})
        .await
        .unwrap();
    let fresh = AuditEvent {
        operation_id: Uuid::new_v4().to_string(),
        ..event
    };
    assert!(matches!(
        enqueue(&db, &Uuid::new_v4().to_string(), &fresh).await,
        Err(AppError::AuditIntegrityConflict)
    ));
}

#[tokio::test]
async fn aborted_business_write_leaves_no_pending_event_or_capacity_reservation() {
    let db = connect_transaction_test_database("audit_enqueue_abort").await;
    let mut session = db.client().start_session().await.unwrap();
    session.start_transaction().await.unwrap();
    db.collection::<bson::Document>("business")
        .insert_one(doc! {"_id": "mutation"})
        .session(&mut session)
        .await
        .unwrap();
    enqueue_in_session(&db, &mut session, &Uuid::new_v4().to_string(), &event())
        .await
        .unwrap();
    session.abort_transaction().await.unwrap();
    let head = state::load(&db).await.unwrap();
    assert_eq!(head.pending_count, 0);
    assert_eq!(head.next_order, 0);
    assert!(head.oldest_pending_at.is_none());
    for name in [PENDING, OUTBOX, AUDIT, "business"] {
        assert_eq!(
            db.collection::<bson::Document>(name)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
    }
    assert!(!materialize_one(&db, &KEY).await.unwrap());
}

#[tokio::test]
async fn logical_retry_survives_materialization_without_releasing_witness_backlog() {
    let db = connect_transaction_test_database("audit_enqueue_retry").await;
    let id = Uuid::new_v4().to_string();
    let event = event();
    enqueue(&db, &id, &event).await.unwrap();
    let reserved = state::load(&db).await.unwrap();
    assert_eq!(reserved.head_seq, 0);
    assert_eq!(reserved.pending_count, 1);
    assert_eq!(
        enqueue(&db, &Uuid::new_v4().to_string(), &event)
            .await
            .unwrap(),
        id
    );
    assert!(materialize_one(&db, &KEY).await.unwrap());
    assert!(!materialize_one(&db, &KEY).await.unwrap());
    assert_eq!(
        enqueue(&db, &Uuid::new_v4().to_string(), &event)
            .await
            .unwrap(),
        id
    );
    let materialized = state::load(&db).await.unwrap();
    assert_eq!(materialized.head_seq, 1);
    assert_eq!(materialized.pending_count, 1);
    assert_eq!(materialized.next_order, 1);
    assert_eq!(materialized.oldest_pending_at, reserved.oldest_pending_at);
    let row = db
        .collection::<AuditLog>(AUDIT)
        .find_one(doc! {})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.id, id);
    assert!(row.user_id.is_none());
    assert_eq!(
        row.event_data.as_ref().unwrap()["principal"]["actor"],
        "maintenance"
    );
    assert_eq!(
        row.event_data.as_ref().unwrap()["owner"]["kind"],
        "platform"
    );
    let delivery = db
        .collection::<AuditOutbox>(OUTBOX)
        .find_one(doc! {})
        .await
        .unwrap()
        .unwrap();
    assert!(delivery.witnessed_at.is_none());
    assert_eq!(Some(delivery.reserved_at), reserved.oldest_pending_at);
    assert_eq!(delivery.order, 1);
    let mut conflict = event;
    conflict.action = "conflicting_action";
    assert!(matches!(
        enqueue(&db, &id, &conflict).await,
        Err(AppError::AuditIntegrityConflict)
    ));
    assert_eq!(state::load(&db).await.unwrap().pending_count, 1);
}

#[tokio::test]
async fn concurrent_materializers_and_direct_appends_preserve_one_reservation_per_event() {
    let db = connect_transaction_test_database("audit_materialize_race").await;
    for _ in 0..4 {
        enqueue(&db, &Uuid::new_v4().to_string(), &event())
            .await
            .unwrap();
    }
    let mut tasks = Vec::new();
    for i in 0..4 {
        let db = db.clone();
        tasks.push(tokio::spawn(async move {
            if i % 2 == 0 {
                // Direct appends may materialize before an older reservation.
                let pending = PendingAuditEvent {
                    id: Uuid::new_v4().to_string(),
                    event: event().safe_projection(),
                    order: 0,
                    reserved_at: Utc::now(),
                    materialized_seq: None,
                };
                chain::append_chained_entry(&db, audit_entry(&pending).unwrap(), &KEY)
                    .await
                    .unwrap();
            }
            while materialize_one(&db, &KEY).await.unwrap() {}
        }));
    }
    for task in tasks {
        task.await.unwrap();
    }
    let head = state::load(&db).await.unwrap();
    assert_eq!(head.head_seq, 6);
    assert_eq!(head.pending_count, 6);
    assert_eq!(head.next_order, 6);
    assert_eq!(
        db.collection::<AuditOutbox>(OUTBOX)
            .count_documents(doc! {})
            .await
            .unwrap(),
        6
    );
    assert_eq!(
        db.collection::<PendingAuditEvent>(PENDING)
            .count_documents(doc! {"materialized_seq": bson::Bson::Null})
            .await
            .unwrap(),
        0
    );
    let report = chain::verify_chain(&db, &KEY, None, None, None)
        .await
        .unwrap();
    assert_eq!(report.checked_count, 6);
    assert!(report.break_info.is_none());
}

#[tokio::test]
async fn failed_materialization_retains_original_event_and_reservation_for_restart() {
    let db = connect_transaction_test_database("audit_materialize_restart").await;
    let first = Uuid::new_v4().to_string();
    enqueue(&db, &first, &event()).await.unwrap();
    assert!(materialize_one(&db, &KEY).await.unwrap());
    let second = Uuid::new_v4().to_string();
    enqueue(&db, &second, &event()).await.unwrap();
    assert!(matches!(
        materialize_one(&db, &[3; 32]).await,
        Err(AppError::AuditIntegrityConflict)
    ));
    let head = state::load(&db).await.unwrap();
    assert_eq!(head.head_seq, 1);
    assert_eq!(head.pending_count, 2);
    assert!(
        db.collection::<PendingAuditEvent>(PENDING)
            .find_one(doc! {"_id": &second})
            .await
            .unwrap()
            .unwrap()
            .materialized_seq
            .is_none()
    );
    // A new session reuses the durable event identity and original capacity.
    assert!(materialize_one(&db, &KEY).await.unwrap());
    let head = state::load(&db).await.unwrap();
    assert_eq!(head.head_seq, 2);
    assert_eq!(head.pending_count, 2);
    assert_eq!(head.next_order, 2);
}

#[tokio::test]
async fn enqueue_rejects_request_material_disguised_as_identity_or_action() {
    let db = connect_transaction_test_database("audit_enqueue_projection").await;
    let mut unsafe_event = event();
    unsafe_event.target_id = Some("provider-secret-or-url".into());
    assert!(
        enqueue(&db, &Uuid::new_v4().to_string(), &unsafe_event)
            .await
            .is_err()
    );
    unsafe_event.target_id = None;
    unsafe_event.action = "invalid\u{1e}action";
    assert!(
        enqueue(&db, &Uuid::new_v4().to_string(), &unsafe_event)
            .await
            .is_err()
    );
    assert_eq!(state::load(&db).await.unwrap().pending_count, 0);
    assert_eq!(
        db.collection::<PendingAuditEvent>(PENDING)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
}

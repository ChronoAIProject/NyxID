//! Replica fault injection for the existing service-history transaction owner.
use super::*;
use crate::{
    models::audit_outbox::{AuditOwner, AuditPrincipal, AuditStage},
    services::{
        audit_outbox_service::{self, AuditEvent},
        audit_runtime::transaction as core,
    },
};
use mongodb::event::{EventHandler, command::CommandEvent};
use std::sync::{Arc, Mutex};
use uuid::Uuid;

async fn mutation(
    db: &mongodb::Database,
    service: &UserService,
    attempts: &std::sync::atomic::AtomicU32,
) -> mongodb::error::Result<()> {
    use std::sync::atomic::Ordering;
    let event_id = Uuid::new_v4();
    let event = AuditEvent {
        operation_id: Uuid::new_v4().to_string(),
        stage: AuditStage::Committed,
        principal: AuditPrincipal::Node {
            id: Uuid::new_v4().to_string(),
        },
        owner: AuditOwner::Person {
            id: service.user_id.clone(),
        },
        action: "audit_service_history_coordinator",
        target_id: Some(service.id.clone()),
    };
    context::scope(
        actor(&service.user_id, &Uuid::new_v4().to_string()),
        async {
            transaction::run(db, async |session| {
                attempts.fetch_add(1, Ordering::SeqCst);
                collection::<Document>(db, "user_services")
                    .update_one(
                        doc! {"_id": &service.id},
                        doc! {"$inc": {"node_priority": 1}},
                    )
                    .session(&mut *session)
                    .await?;
                crate::services::api_key_mutation_service::transaction_result(
                    audit_outbox_service::enqueue_in_session(
                        db,
                        session,
                        &event_id.to_string(),
                        &event,
                    )
                    .await,
                )?;
                Ok(())
            })
            .await
        },
    )
    .await
}

#[tokio::test]
async fn audit_service_history_complete_transaction_retries_raw_conflict_and_unknown_commit_with_bounds()
 {
    use std::sync::atomic::{AtomicU32, Ordering};
    let (base, service) = fixture().await;
    base.collection::<UserService>("user_services")
        .insert_one(&service)
        .await
        .unwrap();
    let commands = Arc::new(Mutex::new(Vec::new()));
    let capture = commands.clone();
    let mut fault = crate::test_utils::fail_command::Guard::acquire(
        &base,
        Some(EventHandler::callback(move |event| {
            if let CommandEvent::Started(event) = event
                && matches!(
                    event.command_name.as_str(),
                    "commitTransaction" | "abortTransaction"
                )
            {
                capture
                    .lock()
                    .unwrap()
                    .push((event.command_name, event.command));
            }
        })),
    )
    .await;
    let db = fault.database().clone();
    for labeled in [false, true] {
        fault.arm(doc! {"times": 2}, doc! {"failCommands": ["update"], "errorCode": 112, "errorLabels": if labeled {vec!["TransientTransactionError"]} else {vec![]}})
            .await
            .unwrap();
        let attempts = AtomicU32::new(0);
        mutation(&db, &service, &attempts).await.unwrap();
        assert_eq!(attempts.load(Ordering::SeqCst), 3);
    }
    assert_eq!(events(&db, &service.id).await.len(), 2);
    fault
        .arm(
            "alwaysOn",
            doc! {"failCommands": ["update"], "errorCode": 112, "errorLabels": []},
        )
        .await
        .unwrap();
    let attempts = AtomicU32::new(0);
    let exhausted = mutation(&db, &service, &attempts).await;
    fault.disarm().await.unwrap();
    assert!(exhausted.is_err());
    assert_eq!(attempts.load(Ordering::SeqCst), core::MAX_BODY_ATTEMPTS);
    assert_eq!(events(&db, &service.id).await.len(), 2);
    for persistent in [false, true] {
        fault.arm(if persistent {bson::Bson::String("alwaysOn".into())} else {bson::Bson::Document(doc! {"times": 1})}, doc! {"failCommands": ["commitTransaction"], "writeConcernError": {"code": 64, "errmsg": "injected uncertain commit"}, "errorLabels": ["UnknownTransactionCommitResult", "TransientTransactionError"]}).await.unwrap();
        commands.lock().unwrap().clear();
        let attempts = AtomicU32::new(0);
        let result = mutation(&db, &service, &attempts).await;
        fault.disarm().await.unwrap();
        assert_eq!(
            attempts.load(Ordering::SeqCst),
            1,
            "uncertainty must not replay the body"
        );
        let commands = commands.lock().unwrap();
        assert!(commands.len() >= 2);
        assert!(
            commands.iter().all(|(name, _)| name == "commitTransaction"),
            "never abort uncertainty"
        );
        for (_, command) in &commands[1..] {
            assert_eq!(command.get("lsid"), commands[0].1.get("lsid"));
            assert_eq!(command.get("txnNumber"), commands[0].1.get("txnNumber"));
        }
        if persistent {
            assert_eq!(commands.len(), core::MAX_COMMIT_ATTEMPTS as usize);
            assert!(
                result
                    .unwrap_err()
                    .contains_label("UnknownTransactionCommitResult")
            );
        } else {
            result.unwrap();
        }
    }
    // Both uncertain commits actually applied once; the entire domain/journal/
    // audit reservation bundle survived without duplicated mutation or event.
    let rows = events(&db, &service.id).await;
    assert_eq!(rows.len(), 4);
    assert_eq!(
        rows.iter().map(|e| e.service_sequence).collect::<Vec<_>>(),
        vec![1, 2, 3, 4]
    );
    let domain = db
        .collection::<Document>("user_services")
        .find_one(doc! {"_id": &service.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(domain.get_i32("node_priority").unwrap(), 4);
    assert_eq!(
        db.collection::<Document>(crate::models::audit_outbox::PENDING_COLLECTION_NAME)
            .count_documents(doc! {})
            .await
            .unwrap(),
        4
    );
}

#[tokio::test]
async fn audit_service_history_implicit_owner_retries_raw_conflict_without_splitting_journal() {
    let (base, service) = fixture().await;
    base.collection::<UserService>("user_services")
        .insert_one(&service)
        .await
        .unwrap();
    let mut fault = crate::test_utils::fail_command::Guard::acquire(&base, None).await;
    let db = fault.database().clone();
    fault
        .arm(
            doc! {"times": 2},
            doc! {"failCommands": ["update"], "errorCode": 112, "errorLabels": []},
        )
        .await
        .unwrap();
    context::scope(
        actor(&service.user_id, &Uuid::new_v4().to_string()),
        async {
            collection::<Document>(&db, "user_services")
                .update_one(
                    doc! {"_id": &service.id},
                    doc! {"$inc": {"node_priority": 1}},
                )
                .await
                .unwrap();
        },
    )
    .await;
    let rows = events(&db, &service.id).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].service_sequence, 1);
}

#[tokio::test]
async fn audit_service_history_integrity_failure_aborts_whole_callback_once() {
    let (db, service) = fixture().await;
    db.collection::<UserService>("user_services")
        .insert_one(&service)
        .await
        .unwrap();
    let mut attempts = 0;
    let result: mongodb::error::Result<()> = context::scope(
        actor(&service.user_id, &Uuid::new_v4().to_string()),
        transaction::run(&db, async |session| {
            attempts += 1;
            collection::<Document>(&db, "user_services")
                .update_one(
                    doc! {"_id": &service.id},
                    doc! {"$inc": {"node_priority": 1}},
                )
                .session(&mut *session)
                .await?;
            crate::services::api_key_mutation_service::transaction_result(Err(
                crate::errors::AppError::AuditIntegrityConflict,
            ))
        }),
    )
    .await;
    assert_eq!(attempts, 1);
    assert!(matches!(
        crate::services::api_key_mutation_service::map_transaction_error(result.unwrap_err()),
        crate::errors::AppError::AuditIntegrityConflict
    ));
    assert!(events(&db, &service.id).await.is_empty());
    assert_eq!(
        db.collection::<Document>("user_services")
            .find_one(doc! {"_id": &service.id})
            .await
            .unwrap()
            .unwrap()
            .get_i32("node_priority")
            .unwrap(),
        0
    );
}

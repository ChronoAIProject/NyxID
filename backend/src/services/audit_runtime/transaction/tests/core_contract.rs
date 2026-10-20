use super::*;
use crate::{
    models::audit_outbox::{
        AuditOwner, AuditPrincipal, AuditStage, PENDING_COLLECTION_NAME as PENDING,
        PendingAuditEvent, SystemAuditActor,
    },
    services::{
        audit_outbox_service::{self, AuditEvent},
        audit_runtime::{AuditRuntime, policy::AuditTier},
    },
};

#[tokio::test]
async fn audit_core_contract_reservation_is_atomic_replayable_and_narrowing_survives_closure() {
    let db = connect_transaction_test_database("audit_core_contract").await;
    let runtime = AuditRuntime::new(Default::default(), false);
    let event_id = Uuid::new_v4();
    let event = AuditEvent {
        operation_id: Uuid::new_v4().to_string(),
        stage: AuditStage::Committed,
        principal: AuditPrincipal::System {
            actor: SystemAuditActor::Maintenance,
        },
        owner: AuditOwner::Platform,
        action: "audit_contract_commit",
        target_id: None,
    };
    // The callback is the caller's existing mutation boundary, not a second
    // audit-only transaction after the business write.
    let context = (&db, &runtime, event_id, &event);
    let result: AppResult<()> = run(&db, &context, |session, (db, runtime, id, event)| {
        Box::pin(async move {
            db.collection::<bson::Document>("business")
                .insert_one(doc! {"_id": &event.operation_id})
                .session(&mut *session)
                .await?;
            audit_outbox_service::enqueue_admitted_in_session(
                db,
                session,
                runtime,
                AuditTier::Standard,
                id,
                event,
            )
            .await?;
            Err(AppError::Conflict("test abort".into()))
        })
    })
    .await;
    assert!(matches!(result, Err(AppError::Conflict(_))));
    assert_eq!(state::load(&db).await.unwrap().pending_count, 0);
    for name in [PENDING, "business"] {
        assert_eq!(
            db.collection::<bson::Document>(name)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
    }
    run(&db, &context, |session, (db, runtime, id, event)| {
        Box::pin(async move {
            db.collection::<bson::Document>("business")
                .insert_one(doc! {"_id": &event.operation_id})
                .session(&mut *session)
                .await?;
            audit_outbox_service::enqueue_admitted_in_session(
                db,
                session,
                runtime,
                AuditTier::Standard,
                id,
                event,
            )
            .await
        })
    })
    .await
    .unwrap();
    let saved = db
        .collection::<PendingAuditEvent>(PENDING)
        .find_one(doc! {"_id": event_id.to_string()})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(saved.event.operation_id, event.operation_id);
    assert_eq!(saved.event.principal, event.principal);
    assert_eq!(saved.event.owner, AuditOwner::Platform);
    // Persisted old age closes new Standard admission; replay needs no new
    // capacity, and Narrowing still records revocation evidence.
    db.collection::<bson::Document>(crate::models::audit_state::COLLECTION_NAME)
        .update_one(doc! {"singleton": true}, doc! {"$set": {
            "witness_enrolled": true,
            "oldest_pending_at": bson::DateTime::from_chrono(Utc::now() - chrono::Duration::hours(2))
        }}).await.unwrap();
    let replay = run(&db, &context, |session, (db, runtime, id, event)| {
        Box::pin(audit_outbox_service::enqueue_admitted_in_session(
            db,
            session,
            runtime,
            AuditTier::Standard,
            id,
            event,
        ))
    })
    .await
    .unwrap();
    assert_eq!(replay, event_id.to_string());
    let new_event = AuditEvent {
        operation_id: Uuid::new_v4().to_string(),
        ..event.clone()
    };
    let new_context = (&db, &runtime, Uuid::new_v4(), &new_event);
    let result = run(&db, &new_context, |session, (db, runtime, id, event)| {
        Box::pin(audit_outbox_service::enqueue_admitted_in_session(
            db,
            session,
            runtime,
            AuditTier::Standard,
            id,
            event,
        ))
    })
    .await;
    assert!(matches!(result, Err(AppError::AuditAdmissionClosed { .. })));
    run(&db, &new_context, |session, (db, runtime, id, event)| {
        Box::pin(audit_outbox_service::enqueue_admitted_in_session(
            db,
            session,
            runtime,
            AuditTier::Narrowing,
            id,
            event,
        ))
    })
    .await
    .unwrap();
    assert_eq!(state::load(&db).await.unwrap().pending_count, 2);
    assert_eq!(
        db.collection::<bson::Document>("business")
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
}

#[tokio::test]
async fn audit_core_contract_owner_resolution_keeps_two_people_and_two_orgs_distinct() {
    use crate::{
        models::{
            org_membership::{COLLECTION_NAME as MEMBERS, OrgMembership, OrgRole},
            user::{COLLECTION_NAME as USERS, User, UserType},
        },
        services::org_service,
        test_utils::{test_membership, test_user},
    };
    let db = connect_transaction_test_database("audit_core_owners").await;
    let (first, second, org_a, org_b) = (
        Uuid::new_v4().to_string(),
        Uuid::new_v4().to_string(),
        Uuid::new_v4().to_string(),
        Uuid::new_v4().to_string(),
    );
    for (id, kind) in [
        (&first, UserType::Person),
        (&second, UserType::Person),
        (&org_a, UserType::Org),
        (&org_b, UserType::Org),
    ] {
        db.collection::<User>(USERS)
            .insert_one(test_user(id, kind))
            .await
            .unwrap();
    }
    for member in [
        test_membership(&org_a, &first, OrgRole::Admin, None),
        test_membership(&org_b, &second, OrgRole::Admin, None),
        test_membership(&org_b, &first, OrgRole::Viewer, None),
    ] {
        db.collection::<OrgMembership>(MEMBERS)
            .insert_one(member)
            .await
            .unwrap();
    }
    assert!(
        !org_service::resolve_owner_access(&db, &first, &second)
            .await
            .unwrap()
            .can_read()
    );
    assert!(
        !org_service::resolve_owner_access(&db, &second, &org_a)
            .await
            .unwrap()
            .can_read()
    );
    let viewer = org_service::resolve_owner_access(&db, &first, &org_b)
        .await
        .unwrap();
    assert!(viewer.can_read());
    assert!(!viewer.can_write());
    let admin = org_service::resolve_owner_access(&db, &first, &org_a)
        .await
        .unwrap();
    assert!(admin.can_write());
    let event = AuditEvent {
        operation_id: Uuid::new_v4().to_string(),
        stage: AuditStage::Committed,
        principal: AuditPrincipal::Person { id: first.clone() },
        owner: AuditOwner::Org { id: org_a.clone() },
        action: "audit_contract_org_change",
        target_id: None,
    };
    let runtime = AuditRuntime::new(Default::default(), false);
    run(
        &db,
        &(&db, &runtime, Uuid::new_v4(), &event),
        |session, (db, runtime, id, event)| {
            Box::pin(audit_outbox_service::enqueue_admitted_in_session(
                db,
                session,
                runtime,
                AuditTier::Standard,
                id,
                event,
            ))
        },
    )
    .await
    .unwrap();
    let saved = db
        .collection::<PendingAuditEvent>(PENDING)
        .find_one(doc! {"event.operation_id": &event.operation_id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(saved.event.principal, AuditPrincipal::Person { id: first });
    assert_eq!(saved.event.owner, AuditOwner::Org { id: org_a });
    assert_ne!(saved.event.owner, AuditOwner::Org { id: org_b });
}

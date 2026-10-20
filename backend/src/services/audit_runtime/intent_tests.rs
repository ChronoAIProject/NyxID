use super::*;
use crate::{
    models::audit_effect_intent::{AuditEffectIntent, COLLECTION_NAME as INTENTS, EffectPhase},
    services::audit_runtime::{intent::EffectReservation, policy::AuditTier},
};

fn reservation() -> EffectReservation {
    EffectReservation {
        event: AuditEvent {
            operation_id: Uuid::new_v4().to_string(),
            stage: AuditStage::Intent,
            principal: AuditPrincipal::System {
                actor: SystemAuditActor::Delivery,
            },
            owner: AuditOwner::Platform,
            action: "audit_test_dispatch",
            target_id: None,
        },
        event_id: Uuid::new_v4(),
        terminal_event_id: Uuid::new_v4(),
    }
}

async fn begin(db: &Database, runtime: &AuditRuntime, request: &EffectReservation) {
    let mut session = db.client().start_session().await.unwrap();
    session
        .start_transaction()
        .write_concern(audit_state_service::majority())
        .await
        .unwrap();
    runtime
        .begin_effect_in_session(db, &mut session, AuditTier::Standard, request)
        .await
        .unwrap();
    runtime
        .begin_effect_in_session(db, &mut session, AuditTier::Standard, request)
        .await
        .unwrap();
    audit_state_service::commit(&mut session).await.unwrap();
}
async fn finish(db: &Database, runtime: &AuditRuntime, operation: &str, stage: AuditStage) {
    let mut session = db.client().start_session().await.unwrap();
    session
        .start_transaction()
        .write_concern(audit_state_service::majority())
        .await
        .unwrap();
    runtime
        .finish_effect_in_session(db, &mut session, operation, stage)
        .await
        .unwrap();
    audit_state_service::commit(&mut session).await.unwrap();
}

#[tokio::test]
async fn audit_intent_reserves_outcome_atomically_and_witnessing_intent_does_not_release_outcome() {
    let f = Fixture::new("audit_intent_capacity").await;
    let request = reservation();
    let mut session = f.db.client().start_session().await.unwrap();
    session
        .start_transaction()
        .write_concern(audit_state_service::majority())
        .await
        .unwrap();
    f.runtime
        .begin_effect_in_session(&f.db, &mut session, AuditTier::Standard, &request)
        .await
        .unwrap();
    f.db.collection::<bson::Document>("test_business")
        .insert_one(doc! {"_id": "mutation"})
        .session(&mut session)
        .await
        .unwrap();
    session.abort_transaction().await.unwrap();
    assert_eq!(
        audit_state_service::load(&f.db)
            .await
            .unwrap()
            .pending_count,
        0
    );
    assert_eq!(
        f.db.collection::<AuditEffectIntent>(INTENTS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        f.db.collection::<bson::Document>("test_business")
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    begin(&f.db, &f.runtime, &request).await;
    let reserved = audit_state_service::load(&f.db).await.unwrap();
    assert_eq!(reserved.pending_count, 2);
    assert_eq!(reserved.next_order, 2);
    assert_eq!(
        f.db.collection::<PendingAuditEvent>(PENDING)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1,
        "terminal is reserved, never recorded before outcome"
    );
    audit_outbox_service::materialize_one(&f.db, &[2; 32])
        .await
        .unwrap();
    f.relay.tick().await.unwrap();
    f.relay.tick().await.unwrap();
    let state = audit_state_service::load(&f.db).await.unwrap();
    assert_eq!(state.pending_count, 1);
    assert_eq!(state.witnessed_seq, 1);
    assert!(state.oldest_pending_at.is_some());
}

#[tokio::test]
async fn audit_intent_dispatch_is_witnessed_once_and_terminal_consumes_reserved_slot() {
    let f = Fixture::new("audit_intent_success").await;
    let request = reservation();
    let operation = &request.event.operation_id;
    begin(&f.db, &f.runtime, &request).await;
    assert!(
        f.runtime.authorize_effect(&f.db, operation).await.is_err(),
        "unmaterialized intent cannot dispatch"
    );
    audit_outbox_service::materialize_one(&f.db, &[2; 32])
        .await
        .unwrap();
    f.relay.tick().await.unwrap();
    f.relay.tick().await.unwrap();
    let effects = AtomicUsize::new(0);
    let permit = f.runtime.authorize_effect(&f.db, operation).await.unwrap();
    permit
        .run(|| async {
            effects.fetch_add(1, Ordering::SeqCst);
        })
        .await
        .unwrap();
    finish(&f.db, &f.runtime, operation, AuditStage::Succeeded).await;
    finish(&f.db, &f.runtime, operation, AuditStage::Succeeded).await;
    assert_eq!(
        audit_state_service::load(&f.db)
            .await
            .unwrap()
            .pending_count,
        1
    );
    assert!(f.runtime.authorize_effect(&f.db, operation).await.is_err());
    assert_eq!(effects.load(Ordering::SeqCst), 1);
    audit_outbox_service::materialize_one(&f.db, &[2; 32])
        .await
        .unwrap();
    f.relay.tick().await.unwrap();
    f.relay.tick().await.unwrap();
    assert_eq!(
        audit_state_service::load(&f.db)
            .await
            .unwrap()
            .pending_count,
        0
    );
    assert_eq!(
        f.provider
            .receipts
            .lock()
            .unwrap()
            .iter()
            .filter(|r| matches!(r.body.event, WitnessEvent::Dispatch { .. }))
            .count(),
        1
    );
}

#[tokio::test]
async fn audit_intent_claim_keeps_its_predecessor_when_unrelated_outbox_work_is_pending() {
    let f = Fixture::new("audit_intent_predecessor").await;
    let request = reservation();
    begin(&f.db, &f.runtime, &request).await;
    audit_outbox_service::materialize_one(&f.db, &[2; 32])
        .await
        .unwrap();
    f.relay.tick().await.unwrap();
    f.relay.tick().await.unwrap();
    f.event().await;
    let permit = f
        .runtime
        .authorize_effect(&f.db, &request.event.operation_id)
        .await
        .unwrap();
    let effects = AtomicUsize::new(0);
    permit
        .run(|| async {
            effects.fetch_add(1, Ordering::SeqCst);
        })
        .await
        .unwrap();
    assert_eq!(effects.load(Ordering::SeqCst), 1);
    assert!(matches!(
        f.provider
            .receipts
            .lock()
            .unwrap()
            .last()
            .unwrap()
            .body
            .event,
        WitnessEvent::Dispatch { .. }
    ));
    finish(
        &f.db,
        &f.runtime,
        &request.event.operation_id,
        AuditStage::Succeeded,
    )
    .await;
}

#[tokio::test]
async fn audit_intent_recovered_claim_never_dispatches_and_paused_original_remains_uncertain() {
    let f = Fixture::new("audit_intent_paused").await;
    let request = reservation();
    let operation = &request.event.operation_id;
    begin(&f.db, &f.runtime, &request).await;
    audit_outbox_service::materialize_one(&f.db, &[2; 32])
        .await
        .unwrap();
    f.relay.tick().await.unwrap();
    f.relay.tick().await.unwrap();
    let paused_original = f.runtime.authorize_effect(&f.db, operation).await.unwrap();
    let effects = Arc::new(AtomicUsize::new(0));
    let entered = Arc::new(tokio::sync::Notify::new());
    let release = Arc::new(tokio::sync::Notify::new());
    let worker_effects = effects.clone();
    let worker_entered = entered.clone();
    let worker_release = release.clone();
    let worker = tokio::spawn(async move {
        paused_original
            .run(|| async {
                worker_entered.notify_one();
                worker_release.notified().await;
                worker_effects.fetch_add(1, Ordering::SeqCst);
            })
            .await
    });
    entered.notified().await;
    let (restarted, _relay) = f.restart();
    assert!(restarted.authorize_effect(&f.db, operation).await.is_err());
    let intent =
        f.db.collection::<AuditEffectIntent>(INTENTS)
            .find_one(doc! {"_id": operation})
            .await
            .unwrap()
            .unwrap();
    assert_eq!(intent.phase, EffectPhase::Terminal);
    assert_eq!(intent.terminal_stage, Some(AuditStage::Uncertain));
    // Uncertainty is honest: a paused original cannot be revoked by a Mongo
    // lease. A new process never gets another permission to execute.
    release.notify_one();
    worker.await.unwrap().unwrap();
    assert!(restarted.authorize_effect(&f.db, operation).await.is_err());
    assert_eq!(effects.load(Ordering::SeqCst), 1);
    assert_eq!(
        f.provider
            .receipts
            .lock()
            .unwrap()
            .iter()
            .filter(|r| matches!(r.body.event, WitnessEvent::Dispatch { .. }))
            .count(),
        1
    );
}

#[tokio::test]
async fn audit_intent_lost_claim_ack_recovery_is_uncertain_without_a_provider_send() {
    let f = Fixture::new("audit_intent_claim_loss").await;
    let request = reservation();
    let operation = &request.event.operation_id;
    begin(&f.db, &f.runtime, &request).await;
    audit_outbox_service::materialize_one(&f.db, &[2; 32])
        .await
        .unwrap();
    f.relay.tick().await.unwrap();
    f.relay.tick().await.unwrap();
    f.provider.lose_claim_ack.store(true, Ordering::SeqCst);
    assert!(f.runtime.authorize_effect(&f.db, operation).await.is_err());
    f.provider.unavailable.store(true, Ordering::SeqCst);
    let (restarted, _relay) = f.restart();
    assert!(restarted.authorize_effect(&f.db, operation).await.is_err());
    assert!(
        restarted
            .diagnostics(
                Some(&audit_state_service::load(&f.db).await.unwrap()),
                chrono::Utc::now()
            )
            .last_contact_secs
            .is_none()
    );
    let intent =
        f.db.collection::<AuditEffectIntent>(INTENTS)
            .find_one(doc! {"_id": operation})
            .await
            .unwrap()
            .unwrap();
    assert_eq!(intent.terminal_stage, Some(AuditStage::Uncertain));
    assert_eq!(
        f.provider
            .receipts
            .lock()
            .unwrap()
            .iter()
            .filter(|r| matches!(r.body.event, WitnessEvent::Dispatch { .. }))
            .count(),
        1
    );
}

#[tokio::test]
async fn audit_intent_corrupt_claim_or_signed_sequence_fork_latches_incident_without_dispatch() {
    for mode in [1, 2] {
        let f = Fixture::new("audit_intent_corrupt_claim").await;
        let request = reservation();
        begin(&f.db, &f.runtime, &request).await;
        audit_outbox_service::materialize_one(&f.db, &[2; 32])
            .await
            .unwrap();
        f.relay.tick().await.unwrap();
        f.relay.tick().await.unwrap();
        f.provider.corrupt_claim_reply.store(mode, Ordering::SeqCst);
        assert!(matches!(
            f.runtime
                .authorize_effect(&f.db, &request.event.operation_id)
                .await,
            Err(AppError::AuditWitnessDiverged)
        ));
        let state = audit_state_service::load(&f.db).await.unwrap();
        assert_eq!(
            f.runtime
                .diagnostics(Some(&state), chrono::Utc::now())
                .witness,
            WitnessState::Diverged
        );
        let (restarted, _relay) = f.restart();
        restarted.restore_incident(&f.db, &state.id).await.unwrap();
        assert_eq!(
            restarted
                .diagnostics(Some(&state), chrono::Utc::now())
                .witness,
            WitnessState::Diverged
        );
        let intent =
            f.db.collection::<AuditEffectIntent>(INTENTS)
                .find_one(doc! {"_id": &request.event.operation_id})
                .await
                .unwrap()
                .unwrap();
        assert_eq!(intent.phase, EffectPhase::ClaimRequested);
        assert!(intent.terminal_stage.is_none());
    }
}

#[tokio::test]
async fn audit_intent_independent_claim_prevents_resend_after_mongo_attempt_rollback() {
    let f = Fixture::new("audit_intent_rollback").await;
    let request = reservation();
    let operation = &request.event.operation_id;
    begin(&f.db, &f.runtime, &request).await;
    audit_outbox_service::materialize_one(&f.db, &[2; 32])
        .await
        .unwrap();
    f.relay.tick().await.unwrap();
    f.relay.tick().await.unwrap();
    let effects = AtomicUsize::new(0);
    f.runtime
        .authorize_effect(&f.db, operation)
        .await
        .unwrap()
        .run(|| async {
            effects.fetch_add(1, Ordering::SeqCst);
        })
        .await
        .unwrap();
    // A restored Mongo backup may say Pending even though the protected claim
    // and the provider effect already happened. The logical UUID stays stable.
    f.db.collection::<AuditEffectIntent>(INTENTS)
        .update_one(
            doc! {"_id": operation},
            doc! {"$set": {"phase": "pending", "attempt": bson::Bson::Null}},
        )
        .await
        .unwrap();
    let (restarted, _relay) = f.restart();
    assert!(restarted.authorize_effect(&f.db, operation).await.is_err());
    assert!(restarted.authorize_effect(&f.db, operation).await.is_err());
    assert_eq!(effects.load(Ordering::SeqCst), 1);
    let intent =
        f.db.collection::<AuditEffectIntent>(INTENTS)
            .find_one(doc! {"_id": operation})
            .await
            .unwrap()
            .unwrap();
    assert_eq!(intent.terminal_stage, Some(AuditStage::Uncertain));
    assert_eq!(
        f.provider
            .receipts
            .lock()
            .unwrap()
            .iter()
            .filter(|r| matches!(r.body.event, WitnessEvent::Dispatch { .. }))
            .count(),
        1
    );
}

#[tokio::test]
async fn audit_intent_terminal_reservation_survives_admission_closure_and_local_mode_needs_no_witness()
 {
    let f = Fixture::new("audit_intent_local").await;
    let defaults = super::super::super::policy::AdmissionLimits::default();
    let limits = super::super::super::policy::AdmissionLimits {
        pending_count: 2,
        standard: super::super::super::policy::TierLimit {
            pending_count: 2,
            ..defaults.standard
        },
        ..defaults
    };
    let local = AuditRuntime::new(limits, false);
    local
        .bind_effects(Uuid::new_v4(), Arc::new(Zeroizing::new([2; 32])), None)
        .unwrap();
    let request = reservation();
    begin(&f.db, &local, &request).await;
    audit_outbox_service::materialize_one(&f.db, &[2; 32])
        .await
        .unwrap();
    let permit = local
        .authorize_effect(&f.db, &request.event.operation_id)
        .await
        .unwrap();
    let effects = AtomicUsize::new(0);
    permit
        .run(|| async {
            effects.fetch_add(1, Ordering::SeqCst);
        })
        .await
        .unwrap();
    finish(
        &f.db,
        &local,
        &request.event.operation_id,
        AuditStage::Succeeded,
    )
    .await;
    assert_eq!(effects.load(Ordering::SeqCst), 1);
    let state = audit_state_service::load(&f.db).await.unwrap();
    assert_eq!(
        local.diagnostics(Some(&state), chrono::Utc::now()).witness,
        WitnessState::Unwitnessed
    );
    assert_eq!(
        state.pending_count, 2,
        "local completion cannot claim external witnessing"
    );
    let configured = AuditRuntime::new(local.limits.clone(), true);
    configured
        .bind_effects(Uuid::new_v4(), Arc::new(Zeroizing::new([2; 32])), None)
        .unwrap();
    let another = reservation();
    let mut session = f.db.client().start_session().await.unwrap();
    session.start_transaction().await.unwrap();
    assert!(
        configured
            .begin_effect_in_session(&f.db, &mut session, AuditTier::Standard, &another)
            .await
            .is_err()
    );
    configured
        .begin_effect_in_session(&f.db, &mut session, AuditTier::Narrowing, &another)
        .await
        .unwrap();
    audit_state_service::commit(&mut session).await.unwrap();
    finish(
        &f.db,
        &configured,
        &another.event.operation_id,
        AuditStage::Cancelled,
    )
    .await;
    assert_eq!(
        audit_state_service::load(&f.db)
            .await
            .unwrap()
            .pending_count,
        4,
        "terminal uses its original slot beyond the hard ceiling"
    );
}

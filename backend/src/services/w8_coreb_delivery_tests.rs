use super::delivery::*;
use crate::{
    errors::{AppError, AppResult},
    models::{
        audit_effect_intent::{AuditEffectIntent, COLLECTION_NAME as INTENTS},
        audit_outbox::{
            AuditApprovalMode, AuditDetail, AuditOwner, AuditPrincipal, AuditStage,
            PENDING_COLLECTION_NAME as PENDING, PendingAuditEvent, ResourceSetKind,
            SystemAuditActor,
        },
    },
    services::{
        audit_outbox_service::{self, AuditEvent, AuditEventSource},
        audit_runtime::{
            AuditRuntime,
            intent::EffectReservation,
            policy::{AdmissionLimits, AuditTier, TierLimit},
            transaction,
        },
        audit_state_service,
    },
};
use mongodb::{Database, bson::doc};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use uuid::Uuid;
use zeroize::Zeroizing;

fn runtime(configured: bool, limits: AdmissionLimits) -> AuditRuntime {
    let runtime = AuditRuntime::new(limits, configured);
    runtime
        .bind_effects(Uuid::new_v4(), Arc::new(Zeroizing::new([2; 32])), None)
        .unwrap();
    runtime
}

async fn fixture() -> (Database, AuditRuntime) {
    let db = crate::test_utils::connect_transaction_test_database("w8_coreb_delivery").await;
    ensure_indexes(&db).await.unwrap();
    (db, runtime(false, Default::default()))
}

fn key() -> DeliveryKey {
    DeliveryKey {
        operation_id: Uuid::new_v4(),
        product_event_id: Uuid::new_v4(),
        destination_id: Uuid::new_v4(),
        channel: Channel::Webhook,
    }
}

fn payload() -> PayloadRef {
    PayloadRef::new(PayloadSource::ConnectLink, Uuid::new_v4(), Uuid::new_v4()).unwrap()
}

fn effect(key: DeliveryKey, principal: AuditPrincipal, owner: AuditOwner) -> EffectReservation {
    EffectReservation {
        event: AuditEvent {
            operation_id: key.operation_id.to_string(),
            stage: AuditStage::Intent,
            principal,
            owner,
            action: "notification_delivery",
            target_id: Some(key.product_event_id.to_string()),
        },
        event_id: Uuid::new_v4(),
        terminal_event_id: Uuid::new_v4(),
    }
}

fn request(key: DeliveryKey, payload: PayloadRef) -> DeliveryRequest {
    DeliveryRequest::new(
        key,
        payload,
        effect(
            key,
            AuditPrincipal::System {
                actor: SystemAuditActor::Delivery,
            },
            AuditOwner::Platform,
        ),
    )
    .unwrap()
}

async fn enqueue<E: AuditEventSource>(
    db: &Database,
    runtime: &AuditRuntime,
    request: &DeliveryRequest<E>,
    tier: AuditTier,
) -> AppResult<QueuedDelivery> {
    transaction::run(
        db,
        &(db, runtime, request, tier),
        |session, (db, runtime, request, tier)| {
            Box::pin(enqueue_in_session(db, session, runtime, *tier, request))
        },
    )
    .await
}

async fn finish(
    db: &Database,
    runtime: &AuditRuntime,
    operation: Uuid,
    outcome: DeliveryOutcome,
) -> AppResult<()> {
    transaction::run(
        db,
        &(db, runtime, operation, outcome),
        |session, (db, runtime, operation, outcome)| {
            Box::pin(finish_in_session(
                db, session, runtime, *operation, *outcome,
            ))
        },
    )
    .await
}

async fn intent(db: &Database, operation: Uuid) -> AuditEffectIntent {
    db.collection::<AuditEffectIntent>(INTENTS)
        .find_one(doc! {"_id": operation.to_string()})
        .await
        .unwrap()
        .unwrap()
}

#[test]
fn w8_coreb_delivery_rejects_invalid_identities_and_untyped_payload_fields() {
    for invalid in [Uuid::nil(), Uuid::new_v5(&Uuid::NAMESPACE_OID, b"not-v4")] {
        assert!(PayloadRef::new(PayloadSource::Approval, invalid, Uuid::new_v4()).is_err());
        assert!(PayloadRef::new(PayloadSource::Approval, Uuid::new_v4(), invalid).is_err());
        for field in 0..3 {
            let mut key = key();
            match field {
                0 => key.operation_id = invalid,
                1 => key.product_event_id = invalid,
                _ => key.destination_id = invalid,
            }
            assert!(
                DeliveryRequest::new(
                    key,
                    payload(),
                    effect(
                        key,
                        AuditPrincipal::System {
                            actor: SystemAuditActor::Delivery
                        },
                        AuditOwner::Platform
                    )
                )
                .is_err()
            );
        }
    }
    let reference = payload();
    for source in [
        PayloadSource::Account,
        PayloadSource::Device,
        PayloadSource::Approval,
        PayloadSource::Machine,
        PayloadSource::Oracle,
        PayloadSource::Connection,
        PayloadSource::ConnectLink,
        PayloadSource::OauthBroker,
        PayloadSource::Channel,
        PayloadSource::Trigger,
        PayloadSource::Automation,
        PayloadSource::DeveloperApp,
        PayloadSource::Cae,
    ] {
        let reference = PayloadRef::new(source, Uuid::new_v4(), Uuid::new_v4()).unwrap();
        let decoded: PayloadRef =
            serde_json::from_value(serde_json::to_value(&reference).unwrap()).unwrap();
        assert_eq!(decoded.source(), source);
    }
    let mut value = serde_json::to_value(&reference).unwrap();
    for field in ["url", "token", "body", "arguments", "metadata"] {
        value[field] = serde_json::json!("not-permitted");
        assert!(serde_json::from_value::<PayloadRef>(value.clone()).is_err());
        value.as_object_mut().unwrap().remove(field);
    }
    value["record_id"] = serde_json::json!("https://example.invalid/payload");
    // Deserialization is data, not authorization; the enqueue input validates again.
    let invalid: PayloadRef = serde_json::from_value(value).unwrap();
    let key = key();
    assert!(
        DeliveryRequest::new(
            key,
            invalid,
            effect(
                key,
                AuditPrincipal::System {
                    actor: SystemAuditActor::Delivery
                },
                AuditOwner::Platform
            )
        )
        .is_err()
    );
}

#[test]
fn w8_coreb_delivery_validates_the_core_event_and_reservation_identity() {
    let key = key();
    for case in 0..6 {
        let mut effect = effect(
            key,
            AuditPrincipal::System {
                actor: SystemAuditActor::Delivery,
            },
            AuditOwner::Platform,
        );
        match case {
            0 => effect.event.operation_id = Uuid::new_v4().to_string(),
            1 => effect.event.stage = AuditStage::Committed,
            2 => effect.terminal_event_id = effect.event_id,
            3 => {
                effect.event.principal = AuditPrincipal::ApiKey {
                    id: Uuid::new_v4().to_string(),
                    owner_id: "unverified".into(),
                }
            }
            4 => {
                effect.event.principal = AuditPrincipal::OracleWorker {
                    id: "unverified:worker".into(),
                }
            }
            _ => effect.event.action = "not an action identifier",
        }
        assert!(DeliveryRequest::new(key, payload(), effect).is_err());
    }
    let effect = effect(
        key,
        AuditPrincipal::System {
            actor: SystemAuditActor::Delivery,
        },
        AuditOwner::Platform,
    );
    assert!(
        DeliveryRequest::new(
            key,
            payload(),
            EffectReservation {
                event: effect.event.with_detail(AuditDetail::ResourceSetChange {
                    kind: ResourceSetKind::Services,
                    added: vec!["not-a-domain-uuid".into()],
                    removed: vec![],
                    truncated: false,
                }),
                event_id: effect.event_id,
                terminal_event_id: effect.terminal_event_id,
            },
        )
        .is_err()
    );
}

#[tokio::test]
async fn w8_coreb_delivery_preserves_owned_and_borrowed_typed_detail_through_completion() {
    let (db, runtime) = fixture().await;
    let key = key();
    let reference = payload();
    let effect = effect(
        key,
        AuditPrincipal::System {
            actor: SystemAuditActor::Delivery,
        },
        AuditOwner::Platform,
    );
    let detail = AuditDetail::ApprovalModeChange {
        old: AuditApprovalMode::WritesOnly,
        new: AuditApprovalMode::All,
    };
    let detailed = effect.event.clone().with_detail(detail.clone());
    let owned = DeliveryRequest::new(
        key,
        reference.clone(),
        EffectReservation {
            event: detailed.clone(),
            event_id: effect.event_id,
            terminal_event_id: effect.terminal_event_id,
        },
    )
    .unwrap();
    let queued = enqueue(&db, &runtime, &owned, AuditTier::Standard)
        .await
        .unwrap();
    let borrowed = DeliveryRequest::new(
        key,
        reference.clone(),
        EffectReservation {
            event: &detailed,
            event_id: effect.event_id,
            terminal_event_id: effect.terminal_event_id,
        },
    )
    .unwrap();
    assert!(
        enqueue(&db, &runtime, &borrowed, AuditTier::Standard)
            .await
            .unwrap()
            == queued
    );
    for changed_detail in [
        None,
        Some(AuditDetail::ApprovalModeChange {
            old: AuditApprovalMode::WritesOnly,
            new: AuditApprovalMode::Off,
        }),
    ] {
        let result = if let Some(detail) = changed_detail {
            let changed = DeliveryRequest::new(
                key,
                reference.clone(),
                EffectReservation {
                    event: effect.event.clone().with_detail(detail),
                    event_id: effect.event_id,
                    terminal_event_id: effect.terminal_event_id,
                },
            )
            .unwrap();
            enqueue(&db, &runtime, &changed, AuditTier::Standard).await
        } else {
            let changed = DeliveryRequest::new(
                key,
                reference.clone(),
                EffectReservation {
                    event: effect.event.clone(),
                    event_id: effect.event_id,
                    terminal_event_id: effect.terminal_event_id,
                },
            )
            .unwrap();
            enqueue(&db, &runtime, &changed, AuditTier::Standard).await
        };
        assert!(matches!(result, Err(AppError::AuditIntegrityConflict)));
    }
    assert_eq!(
        audit_state_service::load(&db).await.unwrap().pending_count,
        2
    );
    assert_eq!(
        intent(&db, key.operation_id).await.event.detail,
        Some(detail.clone())
    );
    assert!(
        audit_outbox_service::materialize_one(&db, &[2; 32])
            .await
            .unwrap()
    );
    queued
        .authorize(&db, &runtime)
        .await
        .unwrap()
        .run(|_| async {})
        .await
        .unwrap();
    finish(&db, &runtime, key.operation_id, DeliveryOutcome::Succeeded)
        .await
        .unwrap();
    assert!(
        audit_outbox_service::materialize_one(&db, &[2; 32])
            .await
            .unwrap()
    );
    for id in [effect.event_id, effect.terminal_event_id] {
        let pending = db
            .collection::<PendingAuditEvent>(PENDING)
            .find_one(doc! {"_id": id.to_string()})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(pending.event.detail, Some(detail.clone()));
        let audit = db
            .collection::<crate::models::audit_log::AuditLog>(
                crate::models::audit_log::COLLECTION_NAME,
            )
            .find_one(doc! {"_id": id.to_string()})
            .await
            .unwrap()
            .unwrap();
        let projected: crate::models::audit_outbox::SafeAuditEvent =
            serde_json::from_value(audit.event_data.clone().unwrap()).unwrap();
        assert_eq!(projected.detail, Some(detail.clone()));
        assert_eq!(
            audit.entry_hash.as_deref(),
            Some(
                crate::services::audit_chain_service::compute_entry_hash(&audit, &[2; 32])
                    .unwrap()
                    .as_str(),
            )
        );
        let evidence = serde_json::to_string(&pending).unwrap();
        assert!(!evidence.contains(reference.record_id()));
        assert!(!evidence.contains(reference.revision_id()));
    }
}

#[tokio::test]
async fn w8_coreb_delivery_enqueue_abort_commit_replay_and_immutable_conflicts() {
    let (db, runtime) = fixture().await;
    let key = key();
    let reference = payload();
    let request = request(key, reference.clone());
    let mut session = db.client().start_session().await.unwrap();
    session.start_transaction().await.unwrap();
    db.collection::<bson::Document>("product_payload_versions")
        .insert_one(doc! {"_id": reference.revision_id(), "body": "PRIVATE_DELIVERY_BODY"})
        .session(&mut session)
        .await
        .unwrap();
    enqueue_in_session(&db, &mut session, &runtime, AuditTier::Standard, &request)
        .await
        .unwrap();
    session.abort_transaction().await.unwrap();
    for collection in [
        COLLECTION_NAME,
        INTENTS,
        PENDING,
        "product_payload_versions",
    ] {
        assert_eq!(
            db.collection::<bson::Document>(collection)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
    }
    assert_eq!(
        audit_state_service::load(&db).await.unwrap().pending_count,
        0
    );
    let queued = enqueue(&db, &runtime, &request, AuditTier::Standard)
        .await
        .unwrap();
    assert_eq!(queued.operation_id(), key.operation_id.to_string());
    let replay = enqueue(&db, &runtime, &request, AuditTier::Standard)
        .await
        .unwrap();
    assert!(queued == replay);
    assert_eq!(
        audit_state_service::load(&db).await.unwrap().pending_count,
        2
    );
    assert_eq!(
        db.collection::<bson::Document>(COLLECTION_NAME)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    for change in 0..6 {
        let mut changed_key = key;
        let mut changed_payload = reference.clone();
        let mut principal = AuditPrincipal::System {
            actor: SystemAuditActor::Delivery,
        };
        let mut owner = AuditOwner::Platform;
        match change {
            0 => changed_payload = payload(),
            1 => changed_key.operation_id = Uuid::new_v4(),
            2 => changed_key.destination_id = Uuid::new_v4(),
            3 => {
                principal = AuditPrincipal::Person {
                    id: Uuid::new_v4().to_string(),
                }
            }
            4 => {
                owner = AuditOwner::Org {
                    id: Uuid::new_v4().to_string(),
                }
            }
            _ => changed_key.channel = Channel::Email,
        }
        let changed = DeliveryRequest::new(
            changed_key,
            changed_payload,
            effect(changed_key, principal, owner),
        )
        .unwrap();
        assert!(matches!(
            enqueue(&db, &runtime, &changed, AuditTier::Standard).await,
            Err(AppError::AuditIntegrityConflict)
        ));
    }
    assert_eq!(
        audit_state_service::load(&db).await.unwrap().pending_count,
        2
    );
}

#[tokio::test]
async fn w8_coreb_delivery_requires_the_unique_dedupe_index_before_enqueue() {
    let (db, runtime) = fixture().await;
    db.collection::<bson::Document>(COLLECTION_NAME)
        .drop_index("notification_delivery_product_destination")
        .await
        .unwrap();
    assert!(
        enqueue(
            &db,
            &runtime,
            &request(key(), payload()),
            AuditTier::Standard
        )
        .await
        .is_err()
    );
    assert_eq!(
        audit_state_service::load(&db).await.unwrap().pending_count,
        0
    );
    assert_eq!(
        db.collection::<bson::Document>(INTENTS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn w8_coreb_delivery_preserves_every_principal_and_two_person_two_org_owners() {
    let (db, runtime) = fixture().await;
    let person_one = Uuid::new_v4().to_string();
    let person_two = Uuid::new_v4().to_string();
    let principals = [
        AuditPrincipal::Person {
            id: person_one.clone(),
        },
        AuditPrincipal::Node {
            id: Uuid::new_v4().to_string(),
        },
        AuditPrincipal::ServiceAccount {
            id: Uuid::new_v4().to_string(),
        },
        AuditPrincipal::OracleWorker {
            id: format!("{}:worker-1", Uuid::new_v4()),
        },
        AuditPrincipal::ApiKey {
            id: Uuid::new_v4().to_string(),
            owner_id: Uuid::new_v4().to_string(),
        },
        AuditPrincipal::System {
            actor: SystemAuditActor::Expiry,
        },
    ];
    let owners = [
        AuditOwner::Person { id: person_one },
        AuditOwner::Person { id: person_two },
        AuditOwner::Org {
            id: Uuid::new_v4().to_string(),
        },
        AuditOwner::Org {
            id: Uuid::new_v4().to_string(),
        },
    ];
    for principal in principals {
        for owner in &owners {
            let key = key();
            let reference = payload();
            let request = DeliveryRequest::new(
                key,
                reference.clone(),
                effect(key, principal.clone(), owner.clone()),
            )
            .unwrap();
            let queued = enqueue(&db, &runtime, &request, AuditTier::Standard)
                .await
                .unwrap();
            assert!(queued.principal() == &principal && queued.owner() == owner);
            let pending = db
                .collection::<PendingAuditEvent>(PENDING)
                .find_one(doc! {"event.operation_id": key.operation_id.to_string()})
                .await
                .unwrap()
                .unwrap();
            assert_eq!(pending.event.principal, principal);
            assert_eq!(&pending.event.owner, owner);
            let evidence = serde_json::to_string(&pending).unwrap();
            for absent in [
                reference.record_id(),
                reference.revision_id(),
                "payload",
                "PRIVATE_DELIVERY_BODY",
            ] {
                assert!(
                    !evidence.contains(absent),
                    "product reference/content entered audit evidence"
                );
            }
        }
    }
}

#[tokio::test]
async fn w8_coreb_delivery_fanout_reserves_one_effect_per_destination_and_channel() {
    let (db, runtime) = fixture().await;
    let base = key();
    let reference = payload();
    for channel in [
        Channel::Email,
        Channel::Fcm,
        Channel::Apns,
        Channel::Telegram,
        Channel::Webhook,
    ] {
        let key = DeliveryKey {
            operation_id: Uuid::new_v4(),
            channel,
            ..base
        };
        let queued = enqueue(
            &db,
            &runtime,
            &request(key, reference.clone()),
            AuditTier::Availability,
        )
        .await
        .unwrap();
        assert_eq!(queued.channel(), channel);
        assert_eq!(queued.product_event_id(), base.product_event_id.to_string());
        assert_eq!(queued.destination_id(), base.destination_id.to_string());
        assert_eq!(queued.payload().source(), PayloadSource::ConnectLink);
    }
    assert_eq!(
        db.collection::<bson::Document>(INTENTS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        5
    );
    assert_eq!(
        audit_state_service::load(&db).await.unwrap().pending_count,
        10
    );
}

#[tokio::test]
async fn w8_coreb_delivery_consumes_core_permit_and_commits_product_outcome_atomically() {
    let (db, runtime) = fixture().await;
    let key = key();
    let reference = payload();
    let queued = enqueue(
        &db,
        &runtime,
        &request(key, reference.clone()),
        AuditTier::Standard,
    )
    .await
    .unwrap();
    assert!(queued.clone().authorize(&db, &runtime).await.is_err());
    assert!(
        audit_outbox_service::materialize_one(&db, &[2; 32])
            .await
            .unwrap()
    );
    let effects = AtomicUsize::new(0);
    let permit = queued.authorize(&db, &runtime).await.unwrap();
    permit
        .run(|delivery| {
            let reference = &reference;
            let effects = &effects;
            async move {
                assert!(delivery.payload() == reference);
                effects.fetch_add(1, Ordering::SeqCst);
            }
        })
        .await
        .unwrap();
    let mut session = db.client().start_session().await.unwrap();
    session.start_transaction().await.unwrap();
    db.collection::<bson::Document>("product_delivery_receipts")
        .insert_one(doc! {"_id": key.operation_id.to_string(), "delivered": true})
        .session(&mut session)
        .await
        .unwrap();
    finish_in_session(
        &db,
        &mut session,
        &runtime,
        key.operation_id,
        DeliveryOutcome::Succeeded,
    )
    .await
    .unwrap();
    session.abort_transaction().await.unwrap();
    assert!(intent(&db, key.operation_id).await.terminal_stage.is_none());
    assert_eq!(
        db.collection::<bson::Document>("product_delivery_receipts")
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    transaction::run(&db, &(&db, &runtime, key), |session, (db, runtime, key)| {
        Box::pin(async move {
            db.collection::<bson::Document>("product_delivery_receipts")
                .insert_one(doc! {"_id": key.operation_id.to_string(), "delivered": true})
                .session(&mut *session)
                .await?;
            finish_in_session(
                db,
                session,
                runtime,
                key.operation_id,
                DeliveryOutcome::Succeeded,
            )
            .await
        })
    })
    .await
    .unwrap();
    finish(&db, &runtime, key.operation_id, DeliveryOutcome::Succeeded)
        .await
        .unwrap();
    assert!(
        finish(
            &db,
            &runtime,
            key.operation_id,
            DeliveryOutcome::DefinitiveFailure
        )
        .await
        .is_err()
    );
    assert!(
        load(&db, key.operation_id)
            .await
            .unwrap()
            .authorize(&db, &runtime)
            .await
            .is_err()
    );
    assert_eq!(
        intent(&db, key.operation_id).await.terminal_stage,
        Some(AuditStage::Succeeded)
    );
    assert_eq!(effects.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn w8_coreb_delivery_recovered_claim_is_uncertain_and_fences_a_paused_sender() {
    let (db, original) = fixture().await;
    let key = key();
    enqueue(
        &db,
        &original,
        &request(key, payload()),
        AuditTier::Standard,
    )
    .await
    .unwrap();
    audit_outbox_service::materialize_one(&db, &[2; 32])
        .await
        .unwrap();
    let paused = load(&db, key.operation_id)
        .await
        .unwrap()
        .authorize(&db, &original)
        .await
        .unwrap();
    let restarted = runtime(false, Default::default());
    assert!(
        load(&db, key.operation_id)
            .await
            .unwrap()
            .authorize(&db, &restarted)
            .await
            .is_err()
    );
    assert_eq!(
        intent(&db, key.operation_id).await.terminal_stage,
        Some(AuditStage::Uncertain)
    );
    let effects = AtomicUsize::new(0);
    assert!(
        paused
            .run(|_| async {
                effects.fetch_add(1, Ordering::SeqCst);
            })
            .await
            .is_err()
    );
    assert!(
        load(&db, key.operation_id)
            .await
            .unwrap()
            .authorize(&db, &original)
            .await
            .is_err()
    );
    finish(
        &db,
        &restarted,
        key.operation_id,
        DeliveryOutcome::Uncertain,
    )
    .await
    .unwrap();
    assert!(
        finish(
            &db,
            &restarted,
            key.operation_id,
            DeliveryOutcome::Succeeded
        )
        .await
        .is_err()
    );
    assert_eq!(effects.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn w8_coreb_delivery_uses_owning_db_tier_and_pre_reserved_completion() {
    let (db, _) = fixture().await;
    let defaults = AdmissionLimits::default();
    let limits = AdmissionLimits {
        pending_count: 2,
        standard: TierLimit {
            pending_count: 2,
            ..defaults.standard
        },
        availability: TierLimit {
            pending_count: 4,
            ..defaults.availability
        },
        ..defaults
    };
    let runtime = runtime(true, limits);
    let first = key();
    let first_request = request(first, payload());
    enqueue(&db, &runtime, &first_request, AuditTier::Standard)
        .await
        .unwrap();
    enqueue(&db, &runtime, &first_request, AuditTier::Standard)
        .await
        .unwrap();
    assert!(
        enqueue(
            &db,
            &runtime,
            &request(key(), payload()),
            AuditTier::Standard
        )
        .await
        .is_err()
    );
    enqueue(
        &db,
        &runtime,
        &request(key(), payload()),
        AuditTier::Availability,
    )
    .await
    .unwrap();
    assert!(
        enqueue(
            &db,
            &runtime,
            &request(key(), payload()),
            AuditTier::Availability
        )
        .await
        .is_err()
    );
    enqueue(
        &db,
        &runtime,
        &request(key(), payload()),
        AuditTier::Narrowing,
    )
    .await
    .unwrap();
    assert!(
        enqueue(&db, &runtime, &request(key(), payload()), AuditTier::Effect)
            .await
            .is_err()
    );
    assert!(
        load(&db, first.operation_id)
            .await
            .unwrap()
            .authorize(&db, &runtime)
            .await
            .is_err()
    );
    finish(
        &db,
        &runtime,
        first.operation_id,
        DeliveryOutcome::Cancelled,
    )
    .await
    .unwrap();
    assert_eq!(
        intent(&db, first.operation_id).await.terminal_stage,
        Some(AuditStage::Cancelled)
    );
    assert_eq!(
        audit_state_service::load(&db).await.unwrap().pending_count,
        6
    );
}

#[tokio::test]
async fn w8_coreb_delivery_retries_the_whole_bundle_and_never_replays_an_unknown_commit() {
    let _guard = audit_state_service::tests::FAIL_COMMAND.lock().await;
    let (db, runtime) = fixture().await;
    let admin = db.client().database("admin");
    let key = key();
    let first_request = request(key, payload());
    admin
        .run_command(
            doc! {"configureFailPoint": "failCommand", "mode": {"times": 2}, "data": {
                "failCommands": ["insert"], "errorCode": 112, "errorLabels": []
            }},
        )
        .await
        .unwrap();
    let bodies = AtomicUsize::new(0);
    let result = transaction::run(
        &db,
        &(&db, &runtime, &first_request, &bodies),
        |session, (db, runtime, request, bodies)| {
            Box::pin(async move {
                bodies.fetch_add(1, Ordering::SeqCst);
                enqueue_in_session(db, session, runtime, AuditTier::Standard, request).await
            })
        },
    )
    .await;
    admin
        .run_command(doc! {"configureFailPoint": "failCommand", "mode": "off"})
        .await
        .unwrap();
    result.unwrap();
    assert_eq!(bodies.load(Ordering::SeqCst), 3);
    admin
        .run_command(
            doc! {"configureFailPoint": "failCommand", "mode": {"times": 1}, "data": {
                "failCommands": ["commitTransaction"],
                "writeConcernError": {"code": 64, "errmsg": "injected uncertain commit"},
                "errorLabels": ["UnknownTransactionCommitResult", "TransientTransactionError"]
            }},
        )
        .await
        .unwrap();
    let next = request(
        DeliveryKey {
            operation_id: Uuid::new_v4(),
            product_event_id: Uuid::new_v4(),
            ..key
        },
        payload(),
    );
    bodies.store(0, Ordering::SeqCst);
    let result = transaction::run(
        &db,
        &(&db, &runtime, &next, &bodies),
        |session, (db, runtime, request, bodies)| {
            Box::pin(async move {
                bodies.fetch_add(1, Ordering::SeqCst);
                enqueue_in_session(db, session, runtime, AuditTier::Standard, request).await
            })
        },
    )
    .await;
    admin
        .run_command(doc! {"configureFailPoint": "failCommand", "mode": "off"})
        .await
        .unwrap();
    result.unwrap();
    assert_eq!(bodies.load(Ordering::SeqCst), 1);
    assert_eq!(
        db.collection::<bson::Document>(COLLECTION_NAME)
            .count_documents(doc! {})
            .await
            .unwrap(),
        2
    );
    assert_eq!(
        db.collection::<bson::Document>(INTENTS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        2
    );
    assert_eq!(
        audit_state_service::load(&db).await.unwrap().pending_count,
        4
    );
}

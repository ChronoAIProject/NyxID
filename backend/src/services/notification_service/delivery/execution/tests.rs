use super::*;
use crate::{
    errors::AppError,
    models::{
        audit_effect_intent::{AuditEffectIntent, COLLECTION_NAME as INTENTS, EffectPhase},
        audit_outbox::{
            AuditOwner, AuditPrincipal, AuditStage, SystemAuditActor, WebhookEventKind,
            WebhookFailureKind,
        },
    },
    services::{
        audit_outbox_service::{self, AuditEvent},
        audit_runtime::{intent::EffectReservation, policy::AuditTier},
        notification_service::delivery::{
            self, DeliveryKey, DeliveryOutcome, DeliveryRequest, PayloadRef, PayloadSource,
        },
    },
};
use mongodb::bson::{self, doc};
use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU8, AtomicUsize, Ordering},
};
use zeroize::Zeroizing;

const PAYLOADS: &str = "test_private_delivery_versions";
const RECEIPTS: &str = "test_private_delivery_receipts";

#[derive(Clone, Copy)]
enum ResultKind {
    Accepted,
    UncertainResponse,
    Rejected,
    Uncertain,
}

struct Consumer {
    kind: ResultKind,
    mismatch: AtomicU8,
    reject_record: AtomicBool,
    sends: AtomicUsize,
    url: String,
}

#[async_trait]
impl DeliveryConsumer for Consumer {
    type Prepared = Zeroizing<String>;
    type Receipt = String;

    async fn resolve(
        &self,
        db: &Database,
        queued: &QueuedDelivery,
    ) -> AppResult<ResolvedDelivery<Self::Prepared>> {
        let saved = db.collection::<bson::Document>(PAYLOADS)
            .find_one(doc! {"_id": queued.payload().record_id(), "revision": queued.payload().revision_id()})
            .await?.ok_or(AppError::AuditIntegrityConflict)?;
        let parse = |field| {
            saved
                .get_str(field)
                .map_err(|_| AppError::AuditIntegrityConflict)
        };
        let destination_id = Uuid::parse_str(parse("destination_id")?)
            .map_err(|_| AppError::AuditIntegrityConflict)?;
        let reference = PayloadRef::new(
            PayloadSource::ConnectLink,
            Uuid::parse_str(parse("_id")?).unwrap(),
            Uuid::parse_str(parse("revision")?).unwrap(),
        )?;
        let mut resolved = ResolvedDelivery {
            binding: DeliveryBinding {
                destination_id,
                payload: reference,
            },
            channel: Channel::Webhook,
            prepared: Zeroizing::new(parse("body")?.to_owned()),
        };
        match self.mismatch.load(Ordering::SeqCst) {
            1 => resolved.binding.destination_id = Uuid::new_v4(),
            2 => {
                resolved.binding.payload =
                    PayloadRef::new(PayloadSource::Account, Uuid::new_v4(), Uuid::new_v4())?
            }
            3 => resolved.channel = Channel::Email,
            4 => resolved.binding.destination_id = Uuid::nil(),
            _ => (),
        }
        Ok(resolved)
    }

    async fn send(
        &self,
        _queued: QueuedDelivery,
        prepared: Self::Prepared,
    ) -> SendOutcome<Self::Receipt> {
        self.sends.fetch_add(1, Ordering::SeqCst);
        match self.kind {
            ResultKind::Accepted | ResultKind::UncertainResponse => {
                match reqwest::Client::new()
                    .post(&self.url)
                    .body(prepared.to_string())
                    .send()
                    .await
                {
                    Ok(response) if response.status().is_success() => match response.text().await {
                        Ok(receipt) => SendOutcome::Accepted(receipt),
                        Err(_) => SendOutcome::Uncertain,
                    },
                    Ok(response) => SendOutcome::uncertain_response(response.status().as_u16())
                        .unwrap_or(SendOutcome::Uncertain),
                    Err(_) => SendOutcome::Uncertain,
                }
            }
            ResultKind::Rejected => SendOutcome::Rejected {
                status: Some(410),
                invalid_destination: true,
            },
            ResultKind::Uncertain => SendOutcome::Uncertain,
        }
    }

    async fn record_outcome_in_session(
        &self,
        db: &Database,
        session: &mut ClientSession,
        queued: &QueuedDelivery,
        outcome: &SendOutcome<Self::Receipt>,
    ) -> AppResult<()> {
        let mut record = doc! {"_id": queued.operation_id()};
        match outcome {
            SendOutcome::Accepted(receipt) => {
                record.insert("status", "accepted");
                record.insert("receipt", receipt);
            }
            SendOutcome::Rejected {
                status,
                invalid_destination,
            } => {
                record.insert("status", "rejected");
                record.insert("http_status", status.map(i32::from));
                record.insert("invalid_destination", invalid_destination);
            }
            SendOutcome::Uncertain => {
                record.insert("status", "uncertain");
            }
            SendOutcome::UncertainResponse { status } => {
                record.insert("status", "uncertain");
                record.insert("http_status", i32::from(*status));
            }
        }
        let receipts = db.collection::<bson::Document>(RECEIPTS);
        if let Some(prior) = receipts
            .find_one(doc! {"_id": queued.operation_id()})
            .session(&mut *session)
            .await?
        {
            if prior != record {
                return Err(AppError::AuditIntegrityConflict);
            }
        } else {
            receipts.insert_one(record).session(&mut *session).await?;
        }
        if self.reject_record.load(Ordering::SeqCst) {
            return Err(AppError::Conflict(
                "Injected product receipt failure".into(),
            ));
        }
        Ok(())
    }

    fn terminal_detail(
        &self,
        outcome: &SendOutcome<Self::Receipt>,
    ) -> AppResult<Option<AuditDetail>> {
        Ok(match outcome {
            SendOutcome::Accepted(_) => None,
            SendOutcome::Rejected { status, .. } => Some(AuditDetail::WebhookDeliveryFailure {
                event: WebhookEventKind::ConnectLinkCompleted,
                reason: WebhookFailureKind::NonSuccessStatus,
                attempts: 1,
                last_http_status: *status,
            }),
            SendOutcome::UncertainResponse { status } => {
                Some(AuditDetail::WebhookDeliveryFailure {
                    event: WebhookEventKind::ConnectLinkCompleted,
                    reason: WebhookFailureKind::NonSuccessStatus,
                    attempts: 1,
                    last_http_status: Some(*status),
                })
            }
            SendOutcome::Uncertain => Some(AuditDetail::WebhookDeliveryFailure {
                event: WebhookEventKind::ConnectLinkCompleted,
                reason: WebhookFailureKind::SendFailed,
                attempts: 1,
                last_http_status: None,
            }),
        })
    }
}

async fn fixture(
    configured: bool,
    kind: ResultKind,
) -> (Database, AuditRuntime, Uuid, Consumer, Arc<AtomicUsize>) {
    let db = crate::test_utils::connect_transaction_test_database("w8_delivery_execution").await;
    delivery::ensure_indexes(&db).await.unwrap();
    let key = crate::services::audit_service::audit_chain_hmac_key().unwrap();
    let runtime = AuditRuntime::new(Default::default(), configured);
    runtime
        .bind_effects(
            Uuid::new_v4(),
            Arc::new(Zeroizing::new(key.try_into().unwrap())),
            None,
        )
        .unwrap();
    let operation = Uuid::new_v4();
    let product = Uuid::new_v4();
    let destination = Uuid::new_v4();
    let version = Uuid::new_v4();
    let request = DeliveryRequest::new(
        DeliveryKey {
            operation_id: operation,
            product_event_id: product,
            destination_id: destination,
            channel: Channel::Webhook,
        },
        PayloadRef::new(PayloadSource::ConnectLink, product, version).unwrap(),
        EffectReservation {
            event: AuditEvent {
                operation_id: operation.to_string(),
                stage: AuditStage::Intent,
                principal: AuditPrincipal::System {
                    actor: SystemAuditActor::Delivery,
                },
                owner: AuditOwner::Platform,
                action: "notification_delivery",
                target_id: Some(product.to_string()),
            },
            event_id: Uuid::new_v4(),
            terminal_event_id: Uuid::new_v4(),
        },
    )
    .unwrap();
    transaction::run(&db, &(&db, &runtime, request), |session, (db, runtime, request)| {
        Box::pin(async move {
            db.collection::<bson::Document>(PAYLOADS).insert_one(doc! {"_id": product.to_string(),
                "revision": version.to_string(), "destination_id": destination.to_string(), "body": "PRIVATE_DELIVERY_BODY"})
                .session(&mut *session).await?;
            delivery::enqueue_in_session(db, session, runtime, AuditTier::Standard, request).await?;
            Ok(())
        })
    }).await.unwrap();
    while audit_outbox_service::materialize_one(&db, key)
        .await
        .unwrap()
    {}
    let network_calls = Arc::new(AtomicUsize::new(0));
    let calls = network_calls.clone();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let router = axum::Router::new().route(
        "/",
        axum::routing::post(move |body: String| {
            let calls = calls.clone();
            async move {
                assert_eq!(body, "PRIVATE_DELIVERY_BODY");
                calls.fetch_add(1, Ordering::SeqCst);
                let status = if matches!(kind, ResultKind::UncertainResponse) {
                    axum::http::StatusCode::SERVICE_UNAVAILABLE
                } else {
                    axum::http::StatusCode::OK
                };
                (status, "PRIVATE_PROVIDER_RECEIPT")
            }
        }),
    );
    tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });
    (
        db,
        runtime,
        operation,
        Consumer {
            kind,
            mismatch: AtomicU8::new(0),
            reject_record: AtomicBool::new(false),
            sends: AtomicUsize::new(0),
            url,
        },
        network_calls,
    )
}

async fn intent(db: &Database, operation: Uuid) -> AuditEffectIntent {
    db.collection::<AuditEffectIntent>(INTENTS)
        .find_one(doc! {"_id": operation.to_string()})
        .await
        .unwrap()
        .unwrap()
}

#[tokio::test]
async fn audit_delivery_execution_sends_once_and_commits_receipt_with_terminal() {
    let (db, runtime, operation, consumer, network) = fixture(false, ResultKind::Accepted).await;
    let outcome = dispatch_one(&db, &runtime, operation, &consumer)
        .await
        .unwrap();
    assert_eq!(outcome.delivery_outcome(), DeliveryOutcome::Succeeded);
    assert_eq!(network.load(Ordering::SeqCst), 1);
    assert_eq!(
        intent(&db, operation).await.terminal_stage,
        Some(AuditStage::Succeeded)
    );
    let queued = load(&db, operation).await.unwrap();
    commit_outcome(&db, &runtime, &queued, &consumer, &outcome)
        .await
        .unwrap();
    assert!(
        dispatch_one(&db, &runtime, operation, &consumer)
            .await
            .is_err()
    );
    assert_eq!(network.load(Ordering::SeqCst), 1);
    assert_eq!(consumer.sends.load(Ordering::SeqCst), 1);
    let receipt = db
        .collection::<bson::Document>(RECEIPTS)
        .find_one(doc! {"_id": operation.to_string()})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        receipt.get_str("receipt").unwrap(),
        "PRIVATE_PROVIDER_RECEIPT"
    );
    let key = crate::services::audit_service::audit_chain_hmac_key().unwrap();
    while audit_outbox_service::materialize_one(&db, key)
        .await
        .unwrap()
    {}
    for collection in [
        INTENTS,
        crate::models::audit_log::COLLECTION_NAME,
        crate::models::audit_outbox::PENDING_COLLECTION_NAME,
    ] {
        let rows = db
            .collection::<bson::Document>(collection)
            .find(doc! {})
            .await
            .unwrap();
        use futures::TryStreamExt;
        for row in rows.try_collect::<Vec<_>>().await.unwrap() {
            let json = serde_json::to_string(&row).unwrap();
            assert!(!json.contains("PRIVATE_DELIVERY_BODY"));
            assert!(!json.contains("PRIVATE_PROVIDER_RECEIPT"));
            assert!(!json.contains(&consumer.url));
        }
    }
}

#[tokio::test]
async fn audit_delivery_execution_mismatched_resolver_never_claims_or_sends() {
    let (db, runtime, operation, consumer, network) = fixture(false, ResultKind::Accepted).await;
    for mismatch in 1..=4 {
        consumer.mismatch.store(mismatch, Ordering::SeqCst);
        assert!(matches!(
            dispatch_one(&db, &runtime, operation, &consumer).await,
            Err(AppError::AuditIntegrityConflict)
        ));
        assert_eq!(intent(&db, operation).await.phase, EffectPhase::Pending);
    }
    assert_eq!(consumer.sends.load(Ordering::SeqCst), 0);
    assert_eq!(network.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn audit_delivery_execution_rejection_and_uncertainty_freeze_terminal_detail() {
    for (kind, stage, expected) in [
        (
            ResultKind::Rejected,
            AuditStage::Failed,
            DeliveryOutcome::DefinitiveFailure,
        ),
        (
            ResultKind::Uncertain,
            AuditStage::Uncertain,
            DeliveryOutcome::Uncertain,
        ),
    ] {
        let (db, runtime, operation, consumer, network) = fixture(false, kind).await;
        let outcome = dispatch_one(&db, &runtime, operation, &consumer)
            .await
            .unwrap();
        assert_eq!(outcome.delivery_outcome(), expected);
        let saved = intent(&db, operation).await;
        assert_eq!(saved.terminal_stage, Some(stage));
        assert_eq!(
            saved.terminal_detail,
            consumer.terminal_detail(&outcome).unwrap()
        );
        let queued = load(&db, operation).await.unwrap();
        commit_outcome(&db, &runtime, &queued, &consumer, &outcome)
            .await
            .unwrap();
        let changed = SendOutcome::Rejected {
            status: Some(400),
            invalid_destination: false,
        };
        assert!(matches!(
            commit_outcome(&db, &runtime, &queued, &consumer, &changed).await,
            Err(AppError::AuditIntegrityConflict)
        ));
        assert!(
            dispatch_one(&db, &runtime, operation, &consumer)
                .await
                .is_err()
        );
        assert_eq!(consumer.sends.load(Ordering::SeqCst), 1);
        assert_eq!(network.load(Ordering::SeqCst), 0);
    }
}

#[tokio::test]
async fn audit_delivery_execution_receipt_failure_rolls_back_terminal_without_resending() {
    let (db, runtime, operation, consumer, network) = fixture(false, ResultKind::Rejected).await;
    consumer.reject_record.store(true, Ordering::SeqCst);
    assert!(
        dispatch_one(&db, &runtime, operation, &consumer)
            .await
            .is_err()
    );
    let saved = intent(&db, operation).await;
    assert_eq!(saved.phase, EffectPhase::Claimed);
    assert_eq!(saved.terminal_stage, None);
    assert_eq!(
        db.collection::<bson::Document>(RECEIPTS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    consumer.reject_record.store(false, Ordering::SeqCst);
    let known_result = SendOutcome::Rejected {
        status: Some(410),
        invalid_destination: true,
    };
    commit_outcome(
        &db,
        &runtime,
        &load(&db, operation).await.unwrap(),
        &consumer,
        &known_result,
    )
    .await
    .unwrap();
    assert_eq!(
        intent(&db, operation).await.terminal_stage,
        Some(AuditStage::Failed)
    );
    assert_eq!(consumer.sends.load(Ordering::SeqCst), 1);
    assert_eq!(network.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn audit_delivery_execution_configured_unreachable_witness_never_dispatches() {
    let (db, runtime, operation, consumer, network) = fixture(true, ResultKind::Accepted).await;
    assert!(matches!(
        dispatch_one(&db, &runtime, operation, &consumer).await,
        Err(AppError::AuditAdmissionClosed { .. })
    ));
    assert_eq!(intent(&db, operation).await.phase, EffectPhase::Pending);
    assert_eq!(consumer.sends.load(Ordering::SeqCst), 0);
    assert_eq!(network.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn audit_delivery_execution_http_5xx_preserves_uncertain_status_and_refuses_changed_replay() {
    let (db, runtime, operation, consumer, network) =
        fixture(false, ResultKind::UncertainResponse).await;
    let outcome = dispatch_one(&db, &runtime, operation, &consumer)
        .await
        .unwrap();
    assert!(matches!(
        outcome,
        SendOutcome::UncertainResponse { status: 503 }
    ));
    assert_eq!(outcome.delivery_outcome(), DeliveryOutcome::Uncertain);
    let saved = intent(&db, operation).await;
    assert_eq!(saved.terminal_stage, Some(AuditStage::Uncertain));
    assert_eq!(
        saved.terminal_detail,
        Some(AuditDetail::WebhookDeliveryFailure {
            event: WebhookEventKind::ConnectLinkCompleted,
            reason: WebhookFailureKind::NonSuccessStatus,
            attempts: 1,
            last_http_status: Some(503),
        })
    );
    let terminal = db
        .collection::<crate::models::audit_outbox::PendingAuditEvent>(
            crate::models::audit_outbox::PENDING_COLLECTION_NAME,
        )
        .find_one(doc! {"event.operation_id": operation.to_string(), "event.stage": "uncertain"})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(terminal.event.detail, saved.terminal_detail);
    let safe_evidence = serde_json::to_string(&terminal).unwrap();
    for private in [
        "PRIVATE_DELIVERY_BODY",
        "PRIVATE_PROVIDER_RECEIPT",
        consumer.url.as_str(),
    ] {
        assert!(!safe_evidence.contains(private));
    }
    let queued = load(&db, operation).await.unwrap();
    commit_outcome(&db, &runtime, &queued, &consumer, &outcome)
        .await
        .unwrap();
    let changed = SendOutcome::uncertain_response(502).unwrap();
    assert!(matches!(
        commit_outcome(&db, &runtime, &queued, &consumer, &changed).await,
        Err(AppError::AuditIntegrityConflict)
    ));
    // The core detail fence independently refuses a different status even when
    // no product receipt callback can reject it first.
    let detail = consumer.terminal_detail(&changed).unwrap();
    let context = (&db, &runtime, operation.to_string(), detail);
    assert!(matches!(
        transaction::run(
            &db,
            &context,
            |session, (db, runtime, operation, detail)| {
                Box::pin(async move {
                    runtime
                        .finish_effect_with_detail_in_session(
                            db,
                            session,
                            operation,
                            AuditStage::Uncertain,
                            detail.clone(),
                        )
                        .await
                })
            }
        )
        .await,
        Err(AppError::AuditIntegrityConflict)
    ));
    assert_eq!(
        intent(&db, operation).await.terminal_detail,
        saved.terminal_detail
    );
    assert!(
        dispatch_one(&db, &runtime, operation, &consumer)
            .await
            .is_err()
    );
    assert_eq!(consumer.sends.load(Ordering::SeqCst), 1);
    assert_eq!(network.load(Ordering::SeqCst), 1);
    crate::test_utils::teardown_test_database(&db).await;
}

#[tokio::test]
async fn audit_delivery_execution_invalid_uncertain_status_never_writes_a_completion() {
    for status in [0, 99, 600, u16::MAX] {
        assert!(matches!(
            SendOutcome::<String>::uncertain_response(status),
            Err(AppError::AuditIntegrityConflict)
        ));
    }
    for status in [100, 599] {
        assert_eq!(
            SendOutcome::<String>::uncertain_response(status)
                .unwrap()
                .delivery_outcome(),
            DeliveryOutcome::Uncertain
        );
    }
    let (db, runtime, operation, consumer, network) = fixture(false, ResultKind::Rejected).await;
    let queued = load(&db, operation).await.unwrap();
    for status in [99, 600] {
        let invalid = SendOutcome::UncertainResponse { status };
        assert!(matches!(
            commit_outcome(&db, &runtime, &queued, &consumer, &invalid).await,
            Err(AppError::AuditIntegrityConflict)
        ));
    }
    assert_eq!(intent(&db, operation).await.phase, EffectPhase::Pending);
    assert_eq!(
        db.collection::<bson::Document>(RECEIPTS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    assert_eq!(consumer.sends.load(Ordering::SeqCst), 0);
    assert_eq!(network.load(Ordering::SeqCst), 0);
    crate::test_utils::teardown_test_database(&db).await;
}

#[tokio::test]
async fn audit_delivery_execution_transport_failure_stays_uncertain_without_status() {
    let (db, runtime, operation, mut consumer, network) =
        fixture(false, ResultKind::Accepted).await;
    let broken = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    consumer.url = format!("http://{}", broken.local_addr().unwrap());
    let close_without_response = tokio::spawn(async move {
        let (connection, _) = broken.accept().await.unwrap();
        drop(connection);
    });
    let outcome = dispatch_one(&db, &runtime, operation, &consumer)
        .await
        .unwrap();
    close_without_response.await.unwrap();
    assert!(matches!(outcome, SendOutcome::Uncertain));
    let saved = intent(&db, operation).await;
    assert_eq!(saved.terminal_stage, Some(AuditStage::Uncertain));
    assert_eq!(
        saved.terminal_detail,
        Some(AuditDetail::WebhookDeliveryFailure {
            event: WebhookEventKind::ConnectLinkCompleted,
            reason: WebhookFailureKind::SendFailed,
            attempts: 1,
            last_http_status: None,
        })
    );
    assert_eq!(consumer.sends.load(Ordering::SeqCst), 1);
    assert_eq!(network.load(Ordering::SeqCst), 0);
    crate::test_utils::teardown_test_database(&db).await;
}

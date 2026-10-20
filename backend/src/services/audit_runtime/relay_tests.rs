//! A signed provider fake implements the documented canonical framing
//! independently. It exercises real Mongo transactions and lost responses.
use super::super::{configuration::WitnessPin, policy::WitnessState};
use super::*;
#[path = "intent_tests.rs"]
mod intent_tests;
use crate::{
    models::audit_outbox::{AuditOwner, AuditPrincipal, AuditStage, SystemAuditActor},
    services::audit_outbox_service::{self, AuditEvent},
};
use async_trait::async_trait;
use ed25519_dalek::{Signer, SigningKey};
use nyxid::{
    models::audit_witness::*,
    services::audit_witness::{
        ObservationChallenge, VerifiedContact, WitnessClient, WitnessResult, now_ms,
    },
};
use std::sync::{
    Mutex as StdMutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};

fn field(out: &mut Vec<u8>, bytes: &[u8]) {
    out.extend_from_slice(&(bytes.len() as u64).to_be_bytes());
    out.extend_from_slice(bytes);
}
fn num(out: &mut Vec<u8>, value: u64) {
    out.extend_from_slice(&value.to_be_bytes());
}
fn encode_link(out: &mut Vec<u8>, value: &AuditLink) {
    num(out, value.key_version.into());
    num(out, value.seq);
    field(out, value.prev_hash.as_bytes());
    field(out, value.entry_hash.as_bytes());
}
fn signed(body: ReceiptBody, key: &Zeroizing<[u8; 32]>) -> SignedReceipt {
    let mut bytes = Vec::new();
    field(&mut bytes, b"nyxid.audit-witness.receipt");
    num(&mut bytes, body.protocol.into());
    field(&mut bytes, body.chain_id.as_bytes());
    num(&mut bytes, body.receipt_seq);
    field(&mut bytes, body.previous_receipt.as_bytes());
    num(&mut bytes, body.witness_time_ms);
    encode_link(&mut bytes, &body.head);
    match &body.event {
        WitnessEvent::Enrollment { head } => {
            num(&mut bytes, 0);
            encode_link(&mut bytes, head);
        }
        WitnessEvent::Advance { request } => {
            num(&mut bytes, 1);
            field(&mut bytes, request.chain_id.as_bytes());
            field(&mut bytes, request.request_id.as_bytes());
            field(&mut bytes, request.previous_receipt.as_bytes());
            num(&mut bytes, request.links.len() as u64);
            for link in &request.links {
                encode_link(&mut bytes, link);
            }
        }
        WitnessEvent::Dispatch { request } => {
            num(&mut bytes, 2);
            field(&mut bytes, request.chain_id.as_bytes());
            field(&mut bytes, request.operation_id.as_bytes());
            field(&mut bytes, request.attempt_id.as_bytes());
            field(&mut bytes, request.process_id.as_bytes());
            num(&mut bytes, request.intent_seq);
            field(&mut bytes, request.intent_hash.as_bytes());
            field(&mut bytes, request.previous_receipt.as_bytes());
        }
    }
    SignedReceipt {
        signature: hex::encode(SigningKey::from_bytes(key).sign(&bytes).to_bytes()),
        body,
    }
}

struct FakeWitness {
    key: Zeroizing<[u8; 32]>,
    pin: WitnessPin,
    receipts: StdMutex<Vec<SignedReceipt>>,
    unavailable: AtomicBool,
    lose_ack: AtomicBool,
    lose_claim_ack: AtomicBool,
    corrupt_claim_reply: AtomicUsize,
    observes: AtomicUsize,
}
impl FakeWitness {
    fn new(chain: Uuid) -> Self {
        let key = Zeroizing::new([7_u8; 32]);
        let head = AuditLink {
            key_version: 1,
            seq: 0,
            prev_hash: ZERO_HASH.into(),
            entry_hash: ZERO_HASH.into(),
        };
        let initial = signed(
            ReceiptBody {
                protocol: 1,
                chain_id: chain,
                receipt_seq: 0,
                previous_receipt: ZERO_HASH.into(),
                witness_time_ms: now_ms().unwrap(),
                head: head.clone(),
                event: WitnessEvent::Enrollment { head },
            },
            &key,
        );
        Self {
            pin: WitnessPin {
                protocol: 1,
                verification_key: hex::encode(
                    SigningKey::from_bytes(&key).verifying_key().to_bytes(),
                ),
                enrollment: initial.clone(),
            },
            key,
            receipts: StdMutex::new(vec![initial]),
            unavailable: AtomicBool::new(false),
            lose_ack: AtomicBool::new(false),
            lose_claim_ack: AtomicBool::new(false),
            corrupt_claim_reply: AtomicUsize::new(0),
            observes: AtomicUsize::new(0),
        }
    }
    fn check_available(&self) -> WitnessResult<()> {
        if self.unavailable.load(Ordering::SeqCst) {
            Err(WitnessError::Unavailable)
        } else {
            Ok(())
        }
    }
}

#[async_trait]
impl WitnessClient for FakeWitness {
    async fn advance(&self, request: AdvanceRequest) -> WitnessResult<SignedReceipt> {
        self.check_available()?;
        let mut receipts = self.receipts.lock().unwrap();
        if let Some(existing) = receipts.iter().find(|r| matches!(&r.body.event, WitnessEvent::Advance {request: prior} if prior.request_id == request.request_id)) {
            return if existing.body.event == (WitnessEvent::Advance {request}) {Ok(existing.clone())} else {Err(WitnessError::Conflict)};
        }
        let last = receipts.last().unwrap();
        if request.chain_id != last.body.chain_id
            || request.previous_receipt != receipt_digest(last)
            || request.links.is_empty()
        {
            return Err(WitnessError::Conflict);
        }
        let mut head = &last.body.head;
        for link in &request.links {
            if link.seq != head.seq + 1
                || link.prev_hash != head.entry_hash
                || link.key_version != head.key_version
            {
                return Err(WitnessError::Conflict);
            }
            head = link;
        }
        let receipt = signed(
            ReceiptBody {
                protocol: 1,
                chain_id: request.chain_id,
                receipt_seq: receipts.len() as u64,
                previous_receipt: request.previous_receipt.clone(),
                witness_time_ms: now_ms()?.max(last.body.witness_time_ms),
                head: head.clone(),
                event: WitnessEvent::Advance { request },
            },
            &self.key,
        );
        receipts.push(receipt.clone());
        if self.lose_ack.swap(false, Ordering::SeqCst) {
            Err(WitnessError::Unavailable)
        } else {
            Ok(receipt)
        }
    }
    async fn observe(&self) -> WitnessResult<VerifiedContact> {
        self.check_available()?;
        self.observes.fetch_add(1, Ordering::SeqCst);
        let challenge = ObservationChallenge::begin(self.pin.enrollment.body.chain_id)?;
        let body = ObservationBody {
            protocol: 1,
            chain_id: self.pin.enrollment.body.chain_id,
            challenge: challenge.request().challenge.clone(),
            witness_time_ms: now_ms()?,
            latest: self.receipts.lock().unwrap().last().unwrap().clone(),
        };
        let mut bytes = Vec::new();
        field(&mut bytes, b"nyxid.audit-witness.observation");
        num(&mut bytes, body.protocol.into());
        field(&mut bytes, body.chain_id.as_bytes());
        field(&mut bytes, body.challenge.as_bytes());
        num(&mut bytes, body.witness_time_ms);
        field(&mut bytes, receipt_digest(&body.latest).as_bytes());
        let signature = hex::encode(SigningKey::from_bytes(&self.key).sign(&bytes).to_bytes());
        challenge.complete(
            SignedObservation { body, signature },
            &self.pin.key().unwrap(),
        )
    }
    async fn history(&self, from_receipt: u64, limit: usize) -> WitnessResult<Vec<SignedReceipt>> {
        self.check_available()?;
        Ok(self
            .receipts
            .lock()
            .unwrap()
            .iter()
            .skip(from_receipt as usize)
            .take(limit)
            .cloned()
            .collect())
    }
    async fn claim_dispatch(&self, request: DispatchRequest) -> WitnessResult<SignedReceipt> {
        self.check_available()?;
        let mut receipts = self.receipts.lock().unwrap();
        if let Some(receipt) = receipts.iter().find(|r| matches!(&r.body.event, WitnessEvent::Dispatch {request: prior} if prior.operation_id == request.operation_id)) {
            return if receipt.body.event == (WitnessEvent::Dispatch {request}) {Ok(receipt.clone())} else {Err(WitnessError::Conflict)};
        }
        let last = receipts.last().unwrap();
        if request.chain_id != last.body.chain_id || request.previous_receipt != receipt_digest(last)
            || !receipts.iter().any(|r| matches!(&r.body.event, WitnessEvent::Advance {request: advance} if advance.links.iter().any(|link| link.seq == request.intent_seq && link.entry_hash == request.intent_hash)))
        { return Err(WitnessError::Conflict); }
        let receipt = signed(
            ReceiptBody {
                protocol: 1,
                chain_id: request.chain_id,
                receipt_seq: receipts.len() as u64,
                previous_receipt: request.previous_receipt.clone(),
                witness_time_ms: now_ms()?.max(last.body.witness_time_ms),
                head: last.body.head.clone(),
                event: WitnessEvent::Dispatch { request },
            },
            &self.key,
        );
        receipts.push(receipt.clone());
        match self.corrupt_claim_reply.swap(0, Ordering::SeqCst) {
            1 => return Err(WitnessError::Corrupt),
            2 => {
                let mut body = receipt.body;
                body.receipt_seq += 1;
                return Ok(signed(body, &self.key));
            }
            _ => {}
        }
        if self.lose_claim_ack.swap(false, Ordering::SeqCst) {
            Err(WitnessError::Unavailable)
        } else {
            Ok(receipt)
        }
    }
}

struct Fixture {
    db: Database,
    provider: Arc<FakeWitness>,
    runtime: Arc<AuditRuntime>,
    relay: Arc<WitnessRelay>,
}
impl Fixture {
    async fn new(name: &str) -> Self {
        let db = crate::test_utils::connect_transaction_test_database(name).await;
        let state = audit_state_service::load(&db).await.unwrap();
        let provider = Arc::new(FakeWitness::new(Uuid::parse_str(&state.id).unwrap()));
        provider.pin.validate().unwrap();
        let runtime = Arc::new(AuditRuntime::new(Default::default(), true));
        let relay = Arc::new(WitnessRelay::new(
            db.clone(),
            runtime.clone(),
            ConfiguredWitness {
                pin: provider.pin.clone(),
                client: provider.clone(),
            },
            Arc::new(Zeroizing::new([2; 32])),
        ));
        runtime
            .bind_effects(
                Uuid::new_v4(),
                Arc::new(Zeroizing::new([2; 32])),
                Some(&relay),
            )
            .unwrap();
        Self {
            db,
            provider,
            runtime,
            relay,
        }
    }
    fn restart(&self) -> (Arc<AuditRuntime>, Arc<WitnessRelay>) {
        let runtime = Arc::new(AuditRuntime::new(Default::default(), true));
        let relay = Arc::new(WitnessRelay::new(
            self.db.clone(),
            runtime.clone(),
            ConfiguredWitness {
                pin: self.provider.pin.clone(),
                client: self.provider.clone(),
            },
            Arc::new(Zeroizing::new([2; 32])),
        ));
        runtime
            .bind_effects(
                Uuid::new_v4(),
                Arc::new(Zeroizing::new([2; 32])),
                Some(&relay),
            )
            .unwrap();
        (runtime, relay)
    }
    async fn event(&self) {
        let mut session = self.db.client().start_session().await.unwrap();
        session
            .start_transaction()
            .write_concern(audit_state_service::majority())
            .await
            .unwrap();
        audit_outbox_service::enqueue_in_session(
            &self.db,
            &mut session,
            &Uuid::new_v4().to_string(),
            &AuditEvent {
                operation_id: Uuid::new_v4().to_string(),
                stage: AuditStage::Committed,
                principal: AuditPrincipal::System {
                    actor: SystemAuditActor::Maintenance,
                },
                owner: AuditOwner::Platform,
                action: "audit_relay_test",
                target_id: None,
            },
        )
        .await
        .unwrap();
        audit_state_service::commit(&mut session).await.unwrap();
        audit_outbox_service::materialize_one(&self.db, &[2; 32])
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn audit_witness_configuration_activation_is_sticky_before_first_contact() {
    let f = Fixture::new("audit_relay_sticky_config").await;
    f.provider.unavailable.store(true, Ordering::SeqCst);
    let config = super::super::AuditConfig {
        witness: Some(ConfiguredWitness {
            pin: f.provider.pin.clone(),
            client: f.provider.clone(),
        }),
        ..Default::default()
    };
    let activated = audit_state_service::initialize(&f.db, &[2; 32], &config)
        .await
        .unwrap();
    assert!(activated.witness_enrolled);
    assert_eq!(activated.witnessed_seq, 0);
    assert_eq!(f.provider.observes.load(Ordering::SeqCst), 0);
    assert!(f.relay.tick().await.is_err());
    let restored = audit_state_service::initialize(&f.db, &[2; 32], &Default::default())
        .await
        .unwrap();
    let without_settings = AuditRuntime::new(Default::default(), false);
    let report = without_settings.diagnostics(Some(&restored), chrono::Utc::now());
    assert_eq!(report.witness, WitnessState::Stale);
    assert_eq!(report.last_contact_secs, None);
    assert_eq!(
        report.admission.effect,
        super::super::policy::Admission::Closed
    );
}

#[tokio::test]
async fn audit_witness_relay_acknowledges_only_observed_history_and_renews_quiet_contact() {
    let f = Fixture::new("audit_relay_ack").await;
    f.event().await;
    assert!(f.relay.tick().await.unwrap());
    assert_eq!(
        audit_state_service::load(&f.db)
            .await
            .unwrap()
            .pending_count,
        1
    );
    assert!(!f.relay.tick().await.unwrap());
    let state = audit_state_service::load(&f.db).await.unwrap();
    assert_eq!(state.pending_count, 0);
    assert!(state.oldest_pending_at.is_none());
    assert_eq!(state.witnessed_seq, 1);
    assert_eq!(
        f.runtime
            .diagnostics(Some(&state), chrono::Utc::now())
            .witness,
        WitnessState::Ready
    );
    f.relay.tick().await.unwrap();
    assert_eq!(f.provider.observes.load(Ordering::SeqCst), 3);
    assert_eq!(
        f.provider.receipts.lock().unwrap().len(),
        2,
        "quiet observation creates no receipts or audit rows"
    );
    let pending =
        f.db.collection::<WitnessDelivery>(DELIVERY)
            .find_one(doc! {})
            .await
            .unwrap()
            .unwrap();
    assert!(pending.pending.is_none());
}

#[tokio::test]
async fn audit_witness_relay_recovers_accepted_batch_after_response_loss_and_unreachable_restart() {
    let f = Fixture::new("audit_relay_restart").await;
    f.event().await;
    f.provider.lose_ack.store(true, Ordering::SeqCst);
    assert!(f.relay.tick().await.is_err());
    let stored =
        f.db.collection::<WitnessDelivery>(DELIVERY)
            .find_one(doc! {})
            .await
            .unwrap()
            .unwrap();
    assert!(stored.pending.is_some());
    f.provider.unavailable.store(true, Ordering::SeqCst);
    let (runtime, relay) = f.restart();
    assert!(relay.tick().await.is_err());
    let state = audit_state_service::load(&f.db).await.unwrap();
    let report = runtime.diagnostics(Some(&state), chrono::Utc::now());
    assert_eq!(report.witness, WitnessState::Stale);
    assert!(report.last_contact_secs.is_none());
    assert_eq!(state.pending_count, 1);
    f.provider.unavailable.store(false, Ordering::SeqCst);
    relay.tick().await.unwrap();
    assert_eq!(
        audit_state_service::load(&f.db)
            .await
            .unwrap()
            .pending_count,
        0
    );
    assert_eq!(
        f.provider.receipts.lock().unwrap().len(),
        2,
        "accepted advance is never duplicated"
    );
}

#[tokio::test]
async fn audit_witness_relay_rejects_deleted_middle_and_tail_against_external_history() {
    for missing in [1_i64, 2, 3] {
        let f = Fixture::new("audit_relay_delete").await;
        for _ in 0..3 {
            f.event().await;
        }
        f.relay.tick().await.unwrap();
        f.relay.tick().await.unwrap();
        f.db.collection::<AuditLog>(LOG)
            .delete_one(doc! {"seq": missing})
            .await
            .unwrap();
        let (runtime, relay) = f.restart();
        assert!(matches!(
            relay.tick().await,
            Err(AppError::AuditWitnessDiverged)
        ));
        let state = audit_state_service::load(&f.db).await.unwrap();
        assert_eq!(
            runtime
                .diagnostics(Some(&state), chrono::Utc::now())
                .witness,
            WitnessState::Diverged
        );
        let restarted = AuditRuntime::new(Default::default(), true);
        restarted.restore_incident(&f.db, &state.id).await.unwrap();
        assert_eq!(
            restarted
                .diagnostics(Some(&state), chrono::Utc::now())
                .witness,
            WitnessState::Diverged
        );
    }
}

#[tokio::test]
async fn audit_witness_relay_detects_database_restore_with_a_self_consistent_old_head() {
    let f = Fixture::new("audit_relay_restore").await;
    let before = audit_state_service::load(&f.db).await.unwrap();
    f.event().await;
    f.relay.tick().await.unwrap();
    f.relay.tick().await.unwrap();
    // Restore the old Mongo boundary, including its empty local tail and relay
    // state. The separately retained witness is deliberately not restored.
    let mut session = f.db.client().start_session().await.unwrap();
    session
        .start_transaction()
        .write_concern(audit_state_service::majority())
        .await
        .unwrap();
    for collection in [LOG, OUTBOX, PENDING, DELIVERY] {
        f.db.collection::<bson::Document>(collection)
            .delete_many(doc! {})
            .session(&mut session)
            .await
            .unwrap();
    }
    f.db.collection::<AuditState>(STATE)
        .replace_one(doc! {"_id": &before.id}, &before)
        .session(&mut session)
        .await
        .unwrap();
    audit_state_service::commit(&mut session).await.unwrap();
    let (runtime, relay) = f.restart();
    assert!(matches!(
        relay.tick().await,
        Err(AppError::AuditWitnessDiverged)
    ));
    assert_eq!(
        runtime
            .diagnostics(Some(&before), chrono::Utc::now())
            .witness,
        WitnessState::Diverged
    );
}

#[tokio::test]
async fn audit_witness_relay_rechecks_persisted_request_and_does_not_sign_changed_outbox() {
    let f = Fixture::new("audit_relay_mutation").await;
    f.event().await;
    f.provider.unavailable.store(true, Ordering::SeqCst);
    // Persist the batch without submitting it, modeling a process stop before send.
    f.provider.unavailable.store(false, Ordering::SeqCst);
    f.relay
        .acknowledge(&f.provider.pin.enrollment)
        .await
        .unwrap_or_else(|_| panic!("ack enrollment"));
    assert!(
        f.relay
            .prepare_batch()
            .await
            .unwrap_or_else(|_| panic!("prepare"))
            .is_some()
    );
    f.db.collection::<bson::Document>(DELIVERY)
        .update_one(
            doc! {},
            doc! {"$set": {"pending.links.0.entry_hash": "a".repeat(64)}},
        )
        .await
        .unwrap();
    assert!(f.relay.tick().await.is_err());
    assert_eq!(
        f.provider.receipts.lock().unwrap().len(),
        1,
        "no forged link reaches witness"
    );
    assert_eq!(
        audit_state_service::load(&f.db)
            .await
            .unwrap()
            .pending_count,
        1
    );
}

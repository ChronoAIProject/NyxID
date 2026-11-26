use super::*;
use crate::models::{chain_verify_status::ChainVerifyOutcome, credits::Credits};
use crate::services::billing::{account_reconciliation, reservation};
use std::sync::Mutex;

pub(crate) mod realtime_ws;
mod round2;

pub(crate) async fn before_checkpoint_write(db: &mongodb::Database) {
    if round2::take_stalled_write(db.name()) {
        std::future::pending::<()>().await;
    }
}

// Complete-event convenience for fixtures; production reuses its relay decoder.
impl crate::services::billing::usage_checkpoint::StreamCheckpoints {
    async fn observe(&mut self, bytes: &[u8]) -> crate::errors::AppResult<()> {
        let mut buffer = String::from_utf8(bytes.to_vec()).unwrap();
        while let Some(event) = crate::services::sse_parser::parse_next_event(&mut buffer) {
            if let Ok(value) = serde_json::from_str(&event.data) {
                self.observe_value(event.event_type.as_deref(), &value)
                    .await;
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Phase {
    AfterCheckpoint,
    BeforePromotion,
    AfterPromotion,
}
#[derive(Clone)]
struct Gate {
    phase: Phase,
    entered: Arc<tokio::sync::Notify>,
    resume: Arc<tokio::sync::Notify>,
    used: Arc<std::sync::atomic::AtomicBool>,
}
impl Gate {
    fn new(phase: Phase) -> Self {
        Self {
            phase,
            entered: Default::default(),
            resume: Default::default(),
            used: Default::default(),
        }
    }
    async fn entered(&self) {
        tokio::time::timeout(std::time::Duration::from_secs(30), self.entered.notified())
            .await
            .unwrap();
    }
}
tokio::task_local! { static GATE: Gate; }
pub(crate) async fn point(phase: Phase) {
    if let Ok(gate) = GATE.try_with(Clone::clone)
        && gate.phase == phase
        && !gate.used.swap(true, Ordering::SeqCst)
    {
        gate.entered.notify_one();
        gate.resume.notified().await;
    }
}
pub(crate) async fn crash_after_checkpoint<T>(future: impl std::future::Future<Output = T>) {
    let gate = Gate::new(Phase::AfterCheckpoint);
    let mut future = Box::pin(GATE.scope(gate.clone(), future));
    tokio::select! {
        _ = &mut future => panic!("executor reached finalization before checkpoint barrier"),
        () = gate.entered() => {},
    }
    drop(future);
}

pub(crate) struct Fixture {
    pub(crate) db: mongodb::Database,
    pub(crate) service: BillingService,
    pub(crate) owner: String,
    pub(crate) commands: Arc<Mutex<Vec<String>>>,
    pub(crate) writes: Arc<Mutex<Vec<(String, String)>>>,
}
impl Fixture {
    pub(crate) async fn new() -> Self {
        let commands = Arc::new(Mutex::new(Vec::new()));
        let recorded = commands.clone();
        let writes = Arc::new(Mutex::new(Vec::new()));
        let recorded_writes = writes.clone();
        let db = crate::test_utils::connect_transaction_test_database_with_command_handler(
            "usage_checkpoints",
            mongodb::event::EventHandler::callback(move |event| {
                if let mongodb::event::command::CommandEvent::Started(event) = event {
                    if matches!(
                        event.command_name.as_str(),
                        "update" | "findAndModify" | "insert" | "delete"
                    ) && let Ok(collection) = event.command.get_str(&event.command_name)
                    {
                        recorded_writes
                            .lock()
                            .unwrap()
                            .push((event.command_name.clone(), collection.to_owned()));
                    }
                    recorded.lock().unwrap().push(event.command_name);
                }
            }),
        )
        .await;
        create_usage_index(&db).await;
        db.collection::<bson::Document>(USAGE_METER)
            .create_indexes(crate::services::billing::stuck_requests::indexes())
            .await
            .unwrap();
        db.collection::<bson::Document>(USAGE_METER)
            .create_index(
                IndexModel::builder()
                    .keys(doc! {"billing_request_id":1})
                    .build(),
            )
            .await
            .unwrap();
        db.collection::<bson::Document>(USAGE_METER)
            .create_indexes(crate::db::billing_hold_indexes())
            .await
            .unwrap();
        db.collection::<bson::Document>("billing_ledger")
            .create_index(
                IndexModel::builder()
                    .keys(doc! {"postings.account":1,"seq":1})
                    .build(),
            )
            .await
            .unwrap();
        insert_fresh_rates(&db).await;
        let owner = insert_owner(&db).await;
        let service = billing_service(
            &db,
            Arc::new(FakeLago::with_wallet_balance(Credits::from_whole(100))),
            0,
        );
        service.ensure_wallet(&owner).await.unwrap();
        Self {
            db,
            service,
            owner,
            commands,
            writes,
        }
    }
    pub(crate) async fn open(&self, funded: bool) -> crate::services::billing::MeteredProxyContext {
        let mut route = route_context(
            &CoverageCase {
                ingress: BillingIngress::LlmGateway,
                scenario: "checkpoint",
                node_intent: NodeIntent::Direct,
                metric: BillingMetric::Tokens,
            },
            &Uuid::new_v4().to_string(),
            &self.owner,
        );
        route.platform_billable = funded;
        route.service_platform_billable = funded;
        let metered = self.service.open(&route).await.unwrap();
        self.service.mark_forwarded(&metered).await.unwrap();
        metered
    }
    pub(crate) async fn row(&self) -> UsageMeterRow {
        self.db
            .collection::<UsageMeterRow>(USAGE_METER)
            .find_one(doc! {})
            .await
            .unwrap()
            .unwrap()
    }
    async fn recover(&self) {
        reservation::recover_retryable_settlements_at(&self.db, Utc::now() + Duration::hours(1))
            .await
            .unwrap();
    }
    pub(crate) async fn clean(&self) {
        account_reconciliation::hold_pass_for_test(&self.db)
            .await
            .unwrap();
        let status = self
            .db
            .collection::<crate::models::chain_verify_status::ChainVerifyStatus>(
                "chain_verify_status",
            )
            .find_one(doc! {"_id":"billing_accounts"})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            status.outcome,
            ChainVerifyOutcome::Ok,
            "{:?}",
            status.break_detail
        );
    }
    pub(crate) fn count(&self) -> usize {
        std::mem::take(&mut *self.commands.lock().unwrap()).len()
    }
    pub(crate) async fn charged(&self, n: i64) {
        let row = self.row().await;
        assert_eq!(row.quantity, Some(n));
        assert!(row.released);
        let wallet = wallet(&self.db, &self.owner).await;
        assert_eq!(wallet.pending_lago_debits, Credits::from_whole(n));
        assert_eq!(wallet.reserved_credits, Credits::ZERO);
        assert_eq!(
            self.db
                .collection::<bson::Document>("billing_ledger")
                .count_documents(doc! {"dedupe_key":format!("usage-settled:{}", row.id)})
                .await
                .unwrap(),
            1
        );
        self.clean().await;
    }
}
fn chat(n: i64) -> Vec<u8> {
    let value = serde_json::json!({"choices":[],"usage":{"prompt_tokens":1,"completion_tokens":n-1,"total_tokens":n}});
    format!("data: {value}\n\n").into_bytes()
}
fn message(kind: &str, usage: serde_json::Value, stop: bool) -> Vec<u8> {
    let v = if kind == "message_start" {
        serde_json::json!({"type":kind,"message":{"usage":usage}})
    } else {
        serde_json::json!({"type":kind,"usage":usage,"delta":{"stop_reason":if stop {Some("end_turn")} else {None}}})
    };
    format!("data: {v}\n\n").into_bytes()
}

#[tokio::test]
async fn usage_checkpoints_cumulative_replace_deduplicate_and_never_expose_interim_final() {
    let f = Fixture::new().await;
    let metered = f.open(true).await;
    let mut recorder = f.service.stream_checkpoints(&metered, true);
    f.count();
    recorder
        .observe(&message(
            "message_start",
            serde_json::json!({"input_tokens":2,"output_tokens":0}),
            false,
        ))
        .await
        .unwrap();
    for n in [4, 4, 3] {
        recorder
            .observe(&message(
                "message_delta",
                serde_json::json!({"output_tokens":n}),
                false,
            ))
            .await
            .unwrap();
    }
    assert_eq!(f.count(), 0, "interim cumulative events do no I/O");
    let row = f.row().await;
    assert!(row.usage_checkpoint.is_none());
    assert_eq!(row.quantity, None);
    assert!(row.pending_platform_usage.is_none());
    assert_eq!(row.pending_resale_quantity, None);
    assert_eq!(row.status, UsageStatus::Forwarded);
    f.recover().await;
    assert!(!f.row().await.released);
    recorder
        .observe(&message(
            "message_delta",
            serde_json::json!({"output_tokens":1}),
            true,
        ))
        .await
        .unwrap();
    tokio::join!(f.recover(), f.recover());
    f.charged(3).await;
    f.recover().await;
    f.charged(3).await;
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn usage_checkpoints_normal_finalization_wins_and_replay_settles_once() {
    let f = Fixture::new().await;
    let metered = f.open(true).await;
    let mut recorder = f.service.stream_checkpoints(&metered, true);
    recorder.observe(&chat(3)).await.unwrap();
    f.service
        .settle(&metered, PlatformUsage::llm_completion(0, 7), None, None)
        .await
        .unwrap();
    f.recover().await;
    f.recover().await;
    f.charged(7).await;
    assert!(
        recorder.observe(&chat(9)).await.is_ok(),
        "a losing checkpoint cannot affect delivery"
    );
    f.charged(7).await;
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn usage_checkpoints_unknown_private_gemini_and_old_shape_keep_unknown() {
    for (value, private) in [
        (
            serde_json::json!({"usageMetadata":{"promptTokenCount":2,"candidatesTokenCount":1,"totalTokenCount":3}}),
            false,
        ),
        (
            serde_json::json!({"usage":{"input_tokens":2,"output_tokens":1,"total_tokens":3}}),
            false,
        ),
        (
            serde_json::from_slice::<serde_json::Value>(&chat(3)[6..chat(3).len() - 2]).unwrap(),
            true,
        ),
        (
            serde_json::json!({"type":"image_generation.partial_image",
                "usage":{"input_tokens":2,"output_tokens":1,"total_tokens":3}}),
            false,
        ),
        (serde_json::Value::Null, false),
    ] {
        let f = Fixture::new().await;
        let metered = f.open(true).await;
        let mut recorder = f.service.stream_checkpoints(&metered, true);
        if private {
            recorder = Default::default();
        }
        f.count();
        recorder
            .observe(format!("data: {value}\n\n").as_bytes())
            .await
            .unwrap();
        assert_eq!(f.count(), 0, "unusable observations do no I/O");
        f.recover().await;
        let row = f.row().await;
        assert!(row.quantity.is_none() && !row.released);
        assert!(row.usage_checkpoint.is_none());
        f.clean().await;
        f.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn usage_checkpoints_split_frames_and_database_rejection_are_best_effort() {
    let f = Fixture::new().await;
    let metered = f.open(true).await;
    let mut recorder = f.service.stream_checkpoints(&metered, true);
    let mut events = crate::services::llm_usage_service::BoundedUsageEvents::default();
    let bytes = chat(3);
    f.count();
    for byte in &bytes[..bytes.len() - 1] {
        assert!(events.push_observed(&[*byte], true).is_empty());
    }
    assert_eq!(f.count(), 0);
    f.db.run_command(doc! {"collMod":USAGE_METER,"validator":{"usage_checkpoint":{"$exists":false}},"validationLevel":"strict"}).await.unwrap();
    for event in events.push_observed(&bytes[bytes.len() - 1..], true) {
        let (kind, value) = event.unwrap();
        recorder.observe_value(kind.as_deref(), &value).await;
    }
    assert!(!recorder.is_enabled());
    assert!(f.row().await.usage_checkpoint.is_none());
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn usage_checkpoints_command_counts() {
    let mut counts = Vec::new();
    for (label, funded, stream, checkpoint) in [
        ("funded_stream_before", true, true, false),
        ("funded_stream_after", true, true, true),
        ("funded_buffered_before", true, false, false),
        ("funded_buffered_after", true, false, true),
        ("metering_stream_before", false, true, false),
        ("metering_stream_after", false, true, true),
    ] {
        let f = Fixture::new().await;
        f.count();
        let metered = f.open(funded).await;
        let admission = f.count();
        let mut recorder = f.service.stream_checkpoints(&metered, stream);
        recorder.observe(b"data: {}\n\n").await.unwrap();
        assert_eq!(f.count(), 0);
        if checkpoint {
            recorder.observe(&chat(3)).await.unwrap();
        }
        let observation = f.count();
        f.service
            .settle(&metered, PlatformUsage::llm_completion(0, 3), None, None)
            .await
            .unwrap();
        let settlement = f.count();
        println!(
            "USAGE_CHECKPOINT_COMMANDS {label}: admission_and_forward={admission} observation={observation} settlement={settlement} total={}",
            admission + observation + settlement
        );
        counts.push((admission, observation, settlement));
        f.db.drop().await.unwrap();
    }
    assert_eq!(counts[0].0, counts[1].0);
    assert_eq!(counts[0].2, counts[1].2);
    assert_eq!(counts[1].1, 1);
    assert_eq!(counts[2], counts[3]);
    assert_eq!(counts[4], counts[5]);
}

#[tokio::test]
async fn usage_checkpoints_promotion_crash_and_normal_finalization_race() {
    for phase in [Phase::BeforePromotion, Phase::AfterPromotion] {
        let f = Fixture::new().await;
        let metered = f.open(true).await;
        f.service
            .stream_checkpoints(&metered, true)
            .observe(&chat(3))
            .await
            .unwrap();
        let gate = Gate::new(phase);
        let request = &metered.route.as_ref().unwrap().billing_request_id;
        let mut recovery = Box::pin(GATE.scope(
            gate.clone(),
            crate::services::billing::usage_checkpoint::recover_request(&f.db, request, None),
        ));
        tokio::select! {
            result = &mut recovery => panic!("missed promotion barrier: {result:?}"),
            () = gate.entered() => {},
        }
        if phase == Phase::BeforePromotion {
            // A real finalizer commits after the transaction's read snapshot.
            f.service
                .settle(&metered, PlatformUsage::llm_completion(0, 7), None, None)
                .await
                .unwrap();
            gate.resume.notify_one();
            recovery.await.unwrap();
            f.recover().await;
            f.charged(7).await;
        } else {
            drop(recovery);
            f.clean().await;
            f.recover().await;
            f.recover().await;
            f.charged(3).await;
        }
        f.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn usage_checkpoints_malformed_report_revokes_prior_finality() {
    for invalid in [
        serde_json::json!(-1),
        serde_json::json!(1.5),
        serde_json::json!("9"),
    ] {
        let f = Fixture::new().await;
        let metered = f.open(true).await;
        let mut recorder = f.service.stream_checkpoints(&metered, true);
        recorder.observe(&chat(3)).await.unwrap();
        f.count();
        let value = serde_json::json!({"choices":[],"usage":{"prompt_tokens":1,"completion_tokens":1,"total_tokens":invalid}});
        recorder
            .observe(format!("data: {value}\n\n").as_bytes())
            .await
            .unwrap();
        assert_eq!(f.count(), 1, "one withdrawal of eligible evidence");
        recorder.observe(&chat(3)).await.unwrap();
        recorder.observe(&chat(4)).await.unwrap();
        assert_eq!(f.count(), 0, "withdrawn observers stay disabled");
        assert!(!f.row().await.usage_checkpoint.unwrap().final_report);
        f.recover().await;
        assert!(f.row().await.quantity.is_none());
        f.clean().await;
        f.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn usage_checkpoints_responses_sequence_prevents_older_report_replacement() {
    let f = Fixture::new().await;
    let metered = f.open(true).await;
    let mut recorder = f.service.stream_checkpoints(&metered, true);
    f.count();
    for (sequence, total) in [(8, 3), (8, 3), (7, 9)] {
        let value = serde_json::json!({"type":"response.completed","sequence_number":sequence,
            "response":{"usage":{"input_tokens":1,"output_tokens":total-1,"total_tokens":total}}});
        recorder
            .observe(format!("data: {value}\n\n").as_bytes())
            .await
            .unwrap();
    }
    assert_eq!(f.count(), 1);
    f.recover().await;
    f.charged(3).await;
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn usage_checkpoints_old_pool_release_wins_without_a_late_debit() {
    let f = Fixture::new().await;
    let metered = f.open(true).await;
    f.service
        .stream_checkpoints(&metered, true)
        .observe(&chat(3))
        .await
        .unwrap();
    let row = f.row().await;
    // Old binaries ignore the observation when releasing unknown pool usage.
    // Simulate their committed result; no new recoverer may charge it afterward.
    f.db.collection::<bson::Document>(USAGE_METER)
        .update_one(
            doc! {"_id":&row.id},
            doc! {"$set":{"released":true,"status":"abandoned","funding.released":true}},
        )
        .await
        .unwrap();
    f.db.collection::<bson::Document>(BILLING_WALLET)
        .update_one(
            doc! {"owner_id":&f.owner},
            doc! {"$inc":{"reserved_credits":-row.reserved_credits}},
        )
        .await
        .unwrap();
    f.recover().await;
    let wallet = wallet(&f.db, &f.owner).await;
    assert_eq!(wallet.pending_lago_debits, Credits::ZERO);
    assert_eq!(wallet.reserved_credits, Credits::ZERO);
    assert!(f.row().await.quantity.is_none());
    f.clean().await;
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn usage_checkpoints_chat_final_choice_replaces_interim_cumulative_report() {
    let f = Fixture::new().await;
    let metered = f.open(true).await;
    let mut recorder = f.service.stream_checkpoints(&metered, true);
    for final_report in [false, true] {
        let value = serde_json::json!({"choices":[{"delta":{},"finish_reason":if final_report {Some("stop")} else {None}}],
            "usage":{"prompt_tokens":1,"completion_tokens":2,"total_tokens":3}});
        recorder
            .observe(format!("data: {value}\n\n").as_bytes())
            .await
            .unwrap();
        assert_eq!(f.row().await.usage_checkpoint.is_some(), final_report);
    }
    f.recover().await;
    f.charged(3).await;
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn usage_checkpoints_unreserved_fallback_without_lago_has_no_writes() {
    let mut f = Fixture::new().await;
    let mut config = test_app_config();
    config.billing_enabled = true;
    config.lago_api_url = None;
    config.lago_api_key = None;
    f.service = BillingService::new(f.db.clone(), Arc::new(config));
    let metered = f.open(true).await;
    assert!(f.row().await.funding.is_none());
    f.count();
    f.service
        .stream_checkpoints(&metered, true)
        .observe(&chat(3))
        .await
        .unwrap();
    assert_eq!(f.count(), 0);
    assert!(f.row().await.usage_checkpoint.is_none());
    f.db.drop().await.unwrap();
}

mod stuck_requests;

use super::*;
use crate::services::{
    billing::realtime_usage::{self, Session},
    llm_usage_service::RealtimeLlmUsageCollector,
};
use std::sync::OnceLock;
mod money;
use money::clean_holds;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Phase {
    BeforeStartCommit,
    BeforeCheckpoint,
    AfterCheckpoint,
    BeforePublication,
    BeforeCommit,
    AfterPublication,
}
#[derive(Clone)]
struct Barrier {
    phase: Phase,
    entered: Arc<tokio::sync::Notify>,
    resume: Arc<tokio::sync::Notify>,
    used: Arc<std::sync::atomic::AtomicBool>,
}
impl Barrier {
    fn new(phase: Phase) -> Self {
        Self {
            phase,
            entered: Default::default(),
            resume: Default::default(),
            used: Default::default(),
        }
    }
}
tokio::task_local! {static BARRIER: Barrier;}
pub(crate) async fn point(phase: Phase) {
    if let Ok(gate) = BARRIER.try_with(Clone::clone)
        && gate.phase == phase
        && !gate.used.swap(true, Ordering::SeqCst)
    {
        gate.entered.notify_one();
        gate.resume.notified().await;
    }
}
fn stalls() -> &'static Mutex<std::collections::HashSet<String>> {
    static STALLS: OnceLock<Mutex<std::collections::HashSet<String>>> = OnceLock::new();
    STALLS.get_or_init(Default::default)
}
pub(crate) fn stall_once(db: &mongodb::Database) {
    stalls().lock().unwrap().insert(db.name().to_owned());
}
pub(crate) async fn before_write(db: &mongodb::Database) {
    point(Phase::BeforeCheckpoint).await;
    if stalls().lock().unwrap().remove(db.name()) {
        std::future::pending::<()>().await;
    }
}
async fn crash<T>(phase: Phase, future: impl std::future::Future<Output = T>) {
    let gate = Barrier::new(phase);
    let mut running = Box::pin(BARRIER.scope(gate.clone(), future));
    tokio::select! {
        _=&mut running=>panic!("completed before crash barrier"),
        ()=gate.entered.notified()=>{},
        _=tokio::time::sleep(std::time::Duration::from_secs(15))=>panic!("barrier timed out"),
    }
    drop(running);
}
pub(crate) fn done(id: &str, tokens: i64) -> String {
    serde_json::json!({"type":"response.done","response":{"id":id,
        "usage":{"input_tokens":1,"output_tokens":tokens-1,"total_tokens":tokens}}})
    .to_string()
}
pub(crate) async fn reserve(
    f: &Fixture,
    metric: BillingMetric,
    funded: bool,
) -> crate::services::billing::MeteredProxyContext {
    let mut route = route_context(
        &CoverageCase {
            ingress: BillingIngress::Proxy,
            scenario: "realtime",
            node_intent: NodeIntent::Direct,
            metric,
        },
        &Uuid::new_v4().to_string(),
        &f.owner,
    );
    route.platform_billable = funded;
    route.service_platform_billable = funded;
    f.service.open(&route).await.unwrap()
}
async fn start(f: &Fixture, metric: BillingMetric) -> Session {
    let metered = reserve(f, metric, true).await;
    f.service
        .mark_realtime_forwarded(&metered, true, true)
        .await
        .unwrap()
        .unwrap()
}
pub(crate) async fn expire(f: &Fixture) {
    f.db.collection::<bson::Document>(USAGE_METER).update_many(doc! {"realtime_session":{"$type":"object"}},doc! {"$set":{
        "realtime_session.close_at":bson::DateTime::from_millis(0),"realtime_session.recover_after":bson::DateTime::from_millis(0)
    }}).await.unwrap();
}
pub(crate) async fn recover(f: &Fixture) {
    // The real reconcile pass completes benefit receipts before settlement retry.
    crate::services::billing::funding::recover_terminal_releases(&f.db)
        .await
        .unwrap();
    // Advance only the existing final-settlement retry cutoff. Realtime
    // eligibility still uses the stored database-time deadline (expire above).
    reservation::recover_retryable_settlements_at(&f.db, Utc::now() + Duration::hours(2))
        .await
        .unwrap();
}
async fn observe(
    session: &mut Session,
    collector: &mut RealtimeLlmUsageCollector,
    text: &str,
    bytes: &mut i64,
) {
    *bytes += text.len() as i64;
    if collector.observe_downstream_text(text) {
        session.observe(&collector.completed_prefix(), *bytes).await;
    }
}
pub(crate) async fn settle_rows(db: &mongodb::Database, rows: Vec<UsageMeterRow>) {
    for row in rows {
        reservation::claim_released_and_settle(db, &row)
            .await
            .unwrap();
    }
}
async fn close(f: &Fixture, session: &Session, collector: RealtimeLlmUsageCollector, bytes: i64) {
    let summary = collector.finalize();
    let mut usage = crate::services::llm_usage_service::platform_usage(
        summary.reported_usage.as_ref(),
        bytes,
        false,
    );
    usage.tokens = summary.token_quantity();
    let rows = session.persist_close(usage).await.unwrap();
    settle_rows(&f.db, rows).await;
}

#[tokio::test]
async fn realtime_ws_crash_returns_hold_and_settles_prefix() {
    let f = Fixture::new().await;
    let mut session = start(&f, BillingMetric::Tokens).await;
    let mut collector = RealtimeLlmUsageCollector::new(true);
    let mut bytes = 0;
    crash(
        Phase::AfterCheckpoint,
        observe(&mut session, &mut collector, &done("r1", 3), &mut bytes),
    )
    .await;
    assert_eq!(f.row().await.quantity, None);
    f.clean().await;
    expire(&f).await;
    recover(&f).await;
    recover(&f).await;
    settled(&f, 3).await;
    // A stale normal closer must not replace the recovered prefix.
    let rows = session
        .persist_close(PlatformUsage::llm_completion(999, 30))
        .await
        .unwrap();
    assert!(rows.is_empty());
    settled(&f, 3).await;
    f.db.drop().await.unwrap();
}
#[tokio::test]
async fn realtime_ws_crash_without_reports_returns_hold() {
    for (metric, quantity) in [
        (BillingMetric::Tokens, 0),
        (BillingMetric::Bytes, 0),
        (BillingMetric::Requests, 1),
    ] {
        let f = Fixture::new().await;
        let _session = start(&f, metric).await;
        assert!(wallet(&f.db, &f.owner).await.reserved_credits > Credits::ZERO);
        expire(&f).await;
        recover(&f).await;
        recover(&f).await;
        settled(&f, quantity).await;
        f.db.drop().await.unwrap();
    }
}
#[tokio::test]
async fn realtime_ws_duplicate_reports_and_normal_close_settle_once() {
    let f = Fixture::new().await;
    let mut session = start(&f, BillingMetric::Tokens).await;
    let mut collector = RealtimeLlmUsageCollector::new(true);
    let mut bytes = 0;
    f.count();
    for _ in 0..3 {
        observe(&mut session, &mut collector, &done("r1", 3), &mut bytes).await;
    }
    assert_eq!(f.count(), 1);
    observe(&mut session, &mut collector, &done("r2", 5), &mut bytes).await;
    assert_eq!(f.count(), 1);
    let before = f.row().await;
    assert_eq!(before.status, UsageStatus::Forwarded);
    assert!(before.quantity.is_none());
    assert!(before.pending_platform_usage.is_none());
    assert!(before.pending_resale_quantity.is_none());
    close(&f, &session, collector, bytes).await;
    expire(&f).await;
    recover(&f).await;
    settled(&f, 8).await;
    assert!(!f.row().await.realtime_session.unwrap().recovered);
    f.db.drop().await.unwrap();
}
#[tokio::test]
async fn realtime_ws_uncovered_segments_round_once_and_discard_unrecorded_tail() {
    let f = Fixture::new().await;
    let mut session = start(&f, BillingMetric::Tokens).await;
    let mut collector = RealtimeLlmUsageCollector::new(true);
    let mut bytes = 0;
    for text in [
        done("known", 3),
        r#"{"type":"response.done","response":{"id":"missing"}}"#.into(),
        r#"{"type":"response.done","response":{"id":"zero","usage":{"total_tokens":0}}}"#.into(),
    ] {
        observe(&mut session, &mut collector, &text, &mut bytes).await;
    }
    let prefix = collector.completed_prefix();
    let expected = prefix.token_quantity();
    assert!(expected > 3);
    // Interleaved/incomplete response traffic after the last boundary is absent
    // from crash billing, while normal close would include its active segment.
    collector.observe_downstream_text(r#"{"type":"response.audio.delta","delta":"unrecorded"}"#);
    assert!(collector.finalize().token_quantity() > expected);
    expire(&f).await;
    recover(&f).await;
    settled(&f, expected).await;
    f.db.drop().await.unwrap();
}
#[tokio::test]
async fn realtime_ws_close_includes_last_active_tail_but_not_control_only_tail() {
    for active in [false, true] {
        let f = Fixture::new().await;
        let mut session = start(&f, BillingMetric::Tokens).await;
        let mut collector = RealtimeLlmUsageCollector::new(true);
        let mut bytes = 0;
        observe(&mut session, &mut collector, &done("r1", 3), &mut bytes).await;
        let text = if active {
            r#"{"type":"response.audio.delta","delta":"tail"}"#
        } else {
            r#"{"type":"session.updated"}"#
        };
        observe(&mut session, &mut collector, text, &mut bytes).await;
        let expected = collector.clone().finalize().token_quantity();
        assert_eq!(expected > 3, active);
        close(&f, &session, collector, bytes).await;
        settled(&f, expected).await;
        f.db.drop().await.unwrap();
    }
}
#[tokio::test]
async fn realtime_ws_crashes_on_both_sides_of_terminal_commit_are_recoverable() {
    for phase in [Phase::BeforeCommit, Phase::AfterPublication] {
        let f = Fixture::new().await;
        let mut session = start(&f, BillingMetric::Tokens).await;
        let mut collector = RealtimeLlmUsageCollector::new(true);
        let mut bytes = 0;
        observe(&mut session, &mut collector, &done("r1", 3), &mut bytes).await;
        crash(
            phase,
            session.persist_close(PlatformUsage::llm_completion(bytes, 3)),
        )
        .await;
        f.clean().await;
        expire(&f).await;
        recover(&f).await;
        settled(&f, 3).await;
        f.db.drop().await.unwrap();
    }
}
#[tokio::test]
async fn realtime_ws_close_and_recovery_compete_on_one_terminal_fence() {
    for close_wins in [true, false] {
        let f = Fixture::new().await;
        let mut session = start(&f, BillingMetric::Tokens).await;
        let mut collector = RealtimeLlmUsageCollector::new(true);
        let mut bytes = 0;
        observe(&mut session, &mut collector, &done("r1", 3), &mut bytes).await;
        expire(&f).await;
        let gate = Barrier::new(Phase::BeforePublication);
        if close_wins {
            let mut loser =
                Box::pin(BARRIER.scope(gate.clone(), realtime_usage::recover_stale(&f.db)));
            tokio::select! {_=&mut loser=>panic!("no barrier"),()=gate.entered.notified()=>{}}
            close(&f, &session, collector, bytes).await;
            gate.resume.notify_one();
            loser.await.unwrap();
        } else {
            let mut loser = Box::pin(BARRIER.scope(
                gate.clone(),
                session.persist_close(PlatformUsage::llm_completion(bytes, 8)),
            ));
            tokio::select! {_=&mut loser=>panic!("no barrier"),()=gate.entered.notified()=>{}}
            recover(&f).await;
            gate.resume.notify_one();
            assert!(loser.await.unwrap().is_empty());
        }
        settled(&f, 3).await;
        assert_eq!(
            f.row().await.realtime_session.unwrap().recovered,
            !close_wins
        );
        f.db.drop().await.unwrap();
    }
}
#[tokio::test]
async fn realtime_ws_live_quiet_session_and_old_shape_are_not_recovered() {
    for legacy in [false, true] {
        let f = Fixture::new().await;
        if legacy {
            let _ = f.open(true).await;
        } else {
            let _ = start(&f, BillingMetric::Tokens).await;
        }
        recover(&f).await;
        let row = f.row().await;
        assert!(!row.released);
        assert!(row.quantity.is_none());
        f.clean().await;
        f.db.drop().await.unwrap();
    }
}
#[tokio::test]
async fn realtime_ws_database_clock_is_authoritative() {
    let f = Fixture::new().await;
    let _ = start(&f, BillingMetric::Tokens).await;
    reservation::recover_retryable_settlements_at(&f.db, Utc::now() + Duration::days(10))
        .await
        .unwrap();
    assert!(!f.row().await.released);
    f.db.drop().await.unwrap();
}
#[tokio::test]
async fn realtime_ws_opt_outs_do_no_extra_database_work() {
    for (funded, eligible) in [(false, true), (true, false)] {
        let f = Fixture::new().await;
        let metered = reserve(&f, BillingMetric::Tokens, funded).await;
        f.count();
        assert!(
            f.service
                .mark_realtime_forwarded(&metered, eligible, true)
                .await
                .unwrap()
                .is_none()
        );
        assert_eq!(f.count(), 1, "only the existing forward marker");
        assert!(f.row().await.realtime_session.is_none());
        f.db.drop().await.unwrap();
    }
    for path in [
        "socket",
        "v1/chat/completions",
        "google.ai.generativelanguage.v1beta.GenerativeService.BidiGenerateContent",
    ] {
        assert!(!realtime_usage::supports_path(path));
    }
    for path in ["/realtime", "v1/realtime/"] {
        assert!(realtime_usage::supports_path(path));
    }
}
#[tokio::test]
async fn realtime_ws_deadline_covers_both_clocks_and_blocked_operations() {
    use std::time::Duration as D;
    for (monotonic, wall) in [(D::from_secs(3601), D::ZERO), (D::ZERO, D::from_secs(3601))] {
        let mut deadline = realtime_usage::BridgeDeadline::new(realtime_usage::MAX_DURATION);
        deadline.age_for_test(monotonic, wall);
        let polled = std::sync::atomic::AtomicBool::new(false);
        assert!(
            deadline
                .run(async {
                    polled.store(true, Ordering::SeqCst);
                    std::future::pending::<()>().await
                })
                .await
                .is_none()
        );
        assert!(!polled.load(Ordering::SeqCst));
    }
    let deadline = realtime_usage::BridgeDeadline::new(D::from_millis(10));
    assert_eq!(deadline.run(async { 42 }).await, Some(42));
    assert!(deadline.run(std::future::pending::<()>()).await.is_none());
}

pub(crate) async fn settled(f: &Fixture, n: i64) {
    if n != 0 {
        f.charged(n).await;
    } else {
        let row = f.row().await;
        assert_eq!(row.quantity, Some(0));
        assert!(row.released);
        let wallet = wallet(&f.db, &f.owner).await;
        assert_eq!(wallet.reserved_credits, Credits::ZERO);
        assert_eq!(wallet.pending_lago_debits, Credits::ZERO);
        assert_eq!(
            f.db.collection::<bson::Document>("billing_ledger")
                .count_documents(doc! {"dedupe_key":format!("usage-settled:{}",row.id)})
                .await
                .unwrap(),
            0,
            "zero charge has no asset posting"
        );
    }
    clean_holds(f).await;
}

#[tokio::test]
async fn realtime_ws_open_prefix_is_not_a_legacy_final_quantity() {
    #[derive(serde::Deserialize)]
    struct OldSettlementView {
        status: UsageStatus,
        forwarded: bool,
        released: bool,
        quantity: Option<i64>,
        pending_platform_usage: Option<PlatformUsage>,
        pending_resale_quantity: Option<i64>,
    }
    let f = Fixture::new().await;
    let mut session = start(&f, BillingMetric::Tokens).await;
    let mut collector = RealtimeLlmUsageCollector::new(true);
    let mut bytes = 0;
    observe(&mut session, &mut collector, &done("one", 3), &mut bytes).await;
    let raw =
        f.db.collection::<bson::Document>(USAGE_METER)
            .find_one(doc! {})
            .await
            .unwrap()
            .unwrap();
    let old: OldSettlementView = bson::from_document(raw).unwrap();
    assert_eq!(old.status, UsageStatus::Forwarded);
    assert!(old.forwarded);
    assert!(!old.released);
    assert!(
        old.quantity.is_none()
            && old.pending_platform_usage.is_none()
            && old.pending_resale_quantity.is_none()
    );
    assert_eq!(
        f.db.collection::<bson::Document>(USAGE_METER)
            .count_documents(doc! {"released":false,"quantity":{"$ne":bson::Bson::Null}})
            .await
            .unwrap(),
        0
    );
    expire(&f).await;
    recover(&f).await;
    settled(&f, 3).await;
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn realtime_ws_configured_resale_without_actual_funding_stays_unmarked() {
    let f = Fixture::new().await;
    let owner = insert_owner(&f.db).await;
    let mut route = route_context(
        &CoverageCase {
            ingress: BillingIngress::Proxy,
            scenario: "unfunded_resale",
            node_intent: NodeIntent::Direct,
            metric: BillingMetric::Tokens,
        },
        &Uuid::new_v4().to_string(),
        &owner,
    );
    route.platform_billable = false;
    route.service_platform_billable = false;
    route.resale = Some(crate::models::service_billing::ResaleSpec {
        metric: BillingMetric::Tokens,
        lago_metric_code: "resale_unfunded".into(),
    });
    for _ in 0..2 {
        let metered = f.service.open(&route).await.unwrap();
        assert!(metered.route.as_ref().unwrap().has_billable_layers());
        assert!(!metered.funded);
        f.count();
        assert!(
            f.service
                .mark_realtime_forwarded(&metered, true, true)
                .await
                .unwrap()
                .is_none()
        );
        assert_eq!(
            f.count(),
            1,
            "only the existing forward marker, including replay"
        );
    }
    assert_eq!(
        f.db.collection::<bson::Document>(USAGE_METER)
            .count_documents(doc! {"realtime_session":{"$type":"object"}})
            .await
            .unwrap(),
        0
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn realtime_ws_funded_replay_keeps_actual_funding() {
    let f = Fixture::new().await;
    let metered = reserve(&f, BillingMetric::Tokens, true).await;
    assert!(metered.funded);
    let replay = f
        .service
        .open(metered.route.as_ref().unwrap())
        .await
        .unwrap();
    assert!(replay.funded);
    let mut session = f
        .service
        .mark_realtime_forwarded(&replay, true, true)
        .await
        .unwrap()
        .unwrap();
    let mut collector = RealtimeLlmUsageCollector::new(true);
    let mut bytes = 0;
    observe(&mut session, &mut collector, &done("one", 3), &mut bytes).await;
    expire(&f).await;
    recover(&f).await;
    settled(&f, 3).await;
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn realtime_ws_crash_before_checkpoint_write_recovers_only_the_durable_zero_prefix() {
    let f = Fixture::new().await;
    let mut session = start(&f, BillingMetric::Tokens).await;
    let mut collector = RealtimeLlmUsageCollector::new(true);
    let mut bytes = 0;
    crash(
        Phase::BeforeCheckpoint,
        observe(
            &mut session,
            &mut collector,
            &done("not-durable", 3),
            &mut bytes,
        ),
    )
    .await;
    assert_eq!(f.row().await.realtime_session.unwrap().sequence, 0);
    expire(&f).await;
    recover(&f).await;
    recover(&f).await;
    settled(&f, 0).await;
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn realtime_ws_crash_before_initialization_commit_uses_unforwarded_abandonment() {
    let f = Fixture::new().await;
    let metered = reserve(&f, BillingMetric::Tokens, true).await;
    crash(
        Phase::BeforeStartCommit,
        f.service.mark_realtime_forwarded(&metered, true, true),
    )
    .await;
    let row = f.row().await;
    assert!(!row.forwarded);
    assert!(row.realtime_session.is_none());
    clean_holds(&f).await;
    assert_eq!(
        reservation::abandon_stale_unforwarded(&f.db, Utc::now() + Duration::hours(2))
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        reservation::abandon_stale_unforwarded(&f.db, Utc::now() + Duration::hours(2))
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        wallet(&f.db, &f.owner).await.reserved_credits,
        Credits::ZERO
    );
    clean_holds(&f).await;
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn realtime_ws_interleaved_response_activity_preserves_arrival_order_accounting() {
    let f = Fixture::new().await;
    let mut session = start(&f, BillingMetric::Tokens).await;
    let mut collector = RealtimeLlmUsageCollector::new(true);
    let mut bytes = 0;
    f.count();
    for event in [
        r#"{"type":"response.created","response":{"id":"a"}}"#.to_owned(),
        r#"{"type":"response.created","response":{"id":"b"}}"#.to_owned(),
        r#"{"type":"response.audio.delta","response_id":"a","delta":"audio"}"#.to_owned(),
        done("b", 3),
        r#"{"type":"response.text.delta","response_id":"a","delta":"text"}"#.to_owned(),
        done("a", 5),
        done("b", 3),
    ] {
        observe(&mut session, &mut collector, &event, &mut bytes).await;
    }
    assert_eq!(f.count(), 2, "only two distinct completion boundaries");
    assert_eq!(collector.finalize().token_quantity(), 8);
    assert_eq!(
        f.row()
            .await
            .realtime_session
            .unwrap()
            .prefix
            .reported_responses,
        2
    );
    expire(&f).await;
    recover(&f).await;
    recover(&f).await;
    settled(&f, 8).await;
    f.db.drop().await.unwrap();
}

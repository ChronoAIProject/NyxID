use super::*;
use crate::models::assistant_voice::{VoiceWindow, WINDOWS};
use crate::models::assistant_voice_session::COLLECTION_NAME as SESSIONS;
use crate::services::billing::voice as windows;
use std::sync::Arc;

tokio::task_local! { pub(crate) static EMPTY_DECISION: bool; }
pub(crate) fn empty_decision() -> bool {
    EMPTY_DECISION.try_with(|x| *x).unwrap_or(false)
}

pub(super) async fn duration(funded: bool) -> (Fixture, String, String) {
    let f = Fixture::new().await;
    crate::services::assistant_voice::ensure_indexes(&f.db)
        .await
        .unwrap();
    let seed = f.open(false).await;
    f.db.collection::<Document>(METERS)
        .delete_many(doc! {})
        .await
        .unwrap();
    let mut ctx = seed.route.unwrap();
    ctx.platform_billable = funded;
    ctx.service_platform_billable = funded;
    ctx.platform_metric = BillingMetric::VoiceSeconds;
    ctx.platform_lago_metric_code = "seconds".into();
    ctx.platform_components = vec![ResaleSpec {
        metric: BillingMetric::Requests,
        lago_metric_code: "requests".into(),
    }];
    for code in ["seconds", "requests"] {
        let mut rate = crate::billing_integration_tests::rate(code, Utc::now());
        rate.credits_per_unit_micros = 10_000;
        rate.credits_per_unit_pico = None;
        f.db.collection("billing_rate_cache")
            .insert_one(rate)
            .await
            .unwrap();
    }
    let sid = uuid::Uuid::new_v4().to_string();
    let id = windows::window_id(&sid, 0).unwrap();
    let meter = windows::reserve(&f.db, &f.service, ctx, &sid, 0)
        .await
        .unwrap();
    f.service.mark_forwarded(&meter).await.unwrap();
    windows::observe(&f.db, &id, 25).await.unwrap();
    f.db.collection::<Document>(WINDOWS)
        .update_many(
            doc! {},
            doc! {"$set":{"deadline":bson::DateTime::from_millis(0)}},
        )
        .await
        .unwrap();
    f.db.collection::<Document>(METERS).update_many(doc!{},doc!{"$set":{"created_at":bson::DateTime::from_millis(0)},"$unset":{"forwarded_owner":""}}).await.unwrap();
    f.db.collection::<Document>("security_replicas").insert_one(doc!{"_id":"survivor","generation_id":"survivor","started_at":bson::DateTime::now(),"observed_at":bson::DateTime::now()}).await.unwrap();
    (f, sid, id)
}
pub(super) async fn window_done(f: &Fixture, id: &str) {
    let w =
        f.db.collection::<VoiceWindow>(WINDOWS)
            .find_one(doc! {"billing_request_id":id})
            .await
            .unwrap()
            .unwrap();
    assert!(w.settled && w.sealed);
    assert_eq!(w.observed_seconds, 25);
    let rows: Vec<UsageMeterRow> =
        f.db.collection(METERS)
            .find(doc! {"billing_request_id":id})
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
    assert_eq!(rows.len(), 2);
    for r in rows {
        assert!(r.released);
        assert_eq!(
            r.quantity,
            Some(if r.metric == BillingMetric::VoiceSeconds {
                25
            } else {
                1
            })
        );
    }
    f.clean().await;
    assert_eq!(
        f.db.collection::<Document>("billing_hold_observations")
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
}
#[tokio::test]
async fn voice_r2_window_deadline_recovers_without_ownership_proof() {
    let mut failures = Vec::new();
    for funded in [false, true] {
        let (f, _, id) = duration(funded).await;
        f.db.collection::<Document>("security_replicas")
            .delete_many(doc! {})
            .await
            .unwrap();
        f.db.collection::<Document>(METERS)
            .update_many(doc! {}, doc! {"$set":{"forwarded_owner":"bad"}})
            .await
            .unwrap();
        let count = windows::reconcile(&f.db).await.unwrap();
        if count != 1 {
            failures.push(funded);
        } else {
            window_done(&f, &id).await;
        }
        f.db.drop().await.unwrap();
    }
    assert!(
        failures.is_empty(),
        "deadline did not finish funded={failures:?}"
    );
}
#[tokio::test]
async fn voice_r2_poison_window_does_not_block_healthy_window() {
    let (f, _, id) = duration(true).await;
    f.db.collection::<Document>(WINDOWS)
        .update_many(doc! {}, doc! {"$set":{"sealed":true}})
        .await
        .unwrap();
    let mut poison =
        f.db.collection::<Document>(WINDOWS)
            .find_one(doc! {})
            .await
            .unwrap()
            .unwrap();
    poison.insert("_id", "poison");
    poison.insert("billing_request_id", "poison");
    poison.insert("deadline", bson::DateTime::from_millis(-1));
    f.db.collection::<Document>(WINDOWS)
        .insert_one(poison)
        .await
        .unwrap();
    f.db.collection::<Document>(METERS).insert_one(doc!{"_id":"poison","billing_request_id":"poison","transaction_id":"poison","layer":"platform","forwarded":true,"metric":"invalid"}).await.unwrap();
    assert_eq!(windows::reconcile(&f.db).await.unwrap(), 1);
    let healthy =
        f.db.collection::<Document>(WINDOWS)
            .find_one(doc! {"billing_request_id":&id})
            .await
            .unwrap()
            .unwrap();
    f.db.collection::<Document>(METERS)
        .delete_one(doc! {"_id":"poison"})
        .await
        .unwrap();
    let settled = healthy.get_bool("settled").unwrap();
    if settled {
        window_done(&f, &id).await;
    }
    f.db.drop().await.unwrap();
    assert!(settled, "first error starved later window");
}
#[tokio::test]
async fn voice_r2_empty_decision_after_evidence_has_no_writes() {
    let (f, sid, id) = duration(true).await;
    session(&f, &sid, false).await;
    let before =
        f.db.collection::<Document>(SESSIONS)
            .find_one(doc! {})
            .await
            .unwrap()
            .unwrap();
    let window =
        f.db.collection::<Document>(WINDOWS)
            .find_one(doc! {})
            .await
            .unwrap()
            .unwrap();
    assert!(
        EMPTY_DECISION
            .scope(true, terminal::publish(&f.db, &id))
            .await
            .unwrap()
            .is_empty()
    );
    let after =
        f.db.collection::<Document>(SESSIONS)
            .find_one(doc! {})
            .await
            .unwrap()
            .unwrap();
    let after_window =
        f.db.collection::<Document>(WINDOWS)
            .find_one(doc! {})
            .await
            .unwrap()
            .unwrap();
    f.db.drop().await.unwrap();
    assert_eq!(before, after, "non-publication changed session");
    assert_eq!(window, after_window, "non-publication sealed window");
}
#[tokio::test]
async fn voice_r2_window_terminal_and_finalize_choose_same_observation() {
    let mut outcomes = Vec::new();
    for terminal_first in [false, true] {
        let (f, sid, id) = duration(true).await;
        session(&f, &sid, false).await;
        f.db.collection::<Document>(SESSIONS)
            .update_one(doc! {}, doc! {"$set":{"observed_seconds":29_i64}})
            .await
            .unwrap();
        if !terminal_first {
            windows::finalize(&f.db, &id, true).await.unwrap();
        }
        let rows = terminal::publish(&f.db, &id).await.unwrap();
        super::super::super::meter::settle_persisted(&f.db, rows)
            .await
            .unwrap();
        windows::finalize(&f.db, &id, true).await.unwrap();
        outcomes.push(
            f.db.collection::<UsageMeterRow>(METERS)
                .find_one(doc! {"metric":"voice_seconds"})
                .await
                .unwrap()
                .unwrap()
                .quantity,
        );
        f.db.drop().await.unwrap();
    }
    assert_eq!(outcomes, vec![Some(25), Some(25)]);
}
#[tokio::test]
async fn voice_r2_nonfinal_checkpoint_classification_is_shared() {
    let (f, _, _) = fixture("response:1", false).await;
    f.db.collection::<Document>(METERS)
        .update_many(
            doc! {},
            doc! {"$set":{"usage_checkpoint":{"final_report":false}}},
        )
        .await
        .unwrap();
    let detail = diagnostics(&f.db).await.unwrap()[0]
        .technical_details
        .clone()
        .unwrap();
    f.db.drop().await.unwrap();
    assert!(
        detail.contains("class=has_nonfinal_checkpoint eligible=true"),
        "{detail}"
    );
}
#[tokio::test]
async fn voice_r2_page_reasons_and_kinds_match_terminal_decisions() {
    let mut failures = Vec::new();
    for (case, suffix, label, eligible) in [
        ("missing_session", "responses", "eligible_voice", true),
        ("per_response", "response:1", "eligible_voice", true),
        ("per_response_pending", "response:1", "eligible_voice", true),
        ("live_takeover", "responses", "voice_live_session", false),
        (
            "expired_lease_live_slot",
            "responses",
            "voice_live_session",
            false,
        ),
        ("expired_lease_closed", "responses", "eligible_voice", true),
        (
            "provider_pending",
            "responses",
            "voice_evidence_pending",
            false,
        ),
        ("unsealed_before", "0", "voice_evidence_pending", false),
        ("unsealed_after", "0", "eligible_voice", true),
        ("confirmed", "0", "eligible_voice", true),
        ("sealed", "0", "eligible_voice", true),
        ("missing_window", "0", "eligible_voice", true),
        ("actor_mismatch", "responses", "voice_actor_mismatch", false),
        ("malformed", "responses", "voice_malformed_evidence", false),
        ("unknown", "other", "excluded_voice_unknown_shape", false),
    ] {
        let (f, sid, id) = fixture(suffix, false).await;
        if case != "missing_session" && case != "unknown" {
            session(&f, &sid, true).await;
        }
        let change = match case {
            "live_takeover" | "expired_lease_live_slot" => {
                doc! {"lease_owner":"survivor","live_slot":true}
            }
            "expired_lease_closed" => doc! {"lease_owner":"survivor","live_slot":false},
            "provider_pending" | "per_response_pending" => {
                doc! {"provider_session_id":"provider","final_usage_confirmed":false,"reconcile_deadline":bson::DateTime::from_chrono(Utc::now()+chrono::Duration::hours(1))}
            }
            "actor_mismatch" => doc! {"user_id":"different"},
            "malformed" => doc! {"generation":"bad"},
            "unsealed_before" | "unsealed_after" => doc! {"final_usage_confirmed":false},
            _ => doc! {},
        };
        if !change.is_empty() {
            f.db.collection::<Document>(SESSIONS)
                .update_one(doc! {}, doc! {"$set":change})
                .await
                .unwrap();
        }
        if ["unsealed_before", "unsealed_after", "confirmed", "sealed"].contains(&case) {
            f.db.collection(WINDOWS)
                .insert_one(VoiceWindow {
                    id: uuid::Uuid::new_v4().to_string(),
                    billing_request_id: id.clone(),
                    session_id: sid,
                    user_id: f.owner.clone(),
                    context_identity: "test".into(),
                    start_second: 0,
                    reserved_seconds: 30,
                    observed_seconds: 12,
                    sealed: case == "sealed",
                    settled: false,
                    uncertain: true,
                    deadline: if case == "unsealed_after" {
                        Utc::now() - chrono::Duration::hours(1)
                    } else {
                        Utc::now() + chrono::Duration::hours(1)
                    },
                })
                .await
                .unwrap();
        }
        let detail = diagnostics(&f.db).await.unwrap()[0]
            .technical_details
            .clone()
            .unwrap();
        let before_session =
            f.db.collection::<Document>(SESSIONS)
                .find_one(doc! {})
                .await
                .unwrap();
        let before_window =
            f.db.collection::<Document>(WINDOWS)
                .find_one(doc! {})
                .await
                .unwrap();
        let rows = terminal::publish(&f.db, &id).await;
        if !eligible {
            assert_eq!(
                before_session,
                f.db.collection::<Document>(SESSIONS)
                    .find_one(doc! {})
                    .await
                    .unwrap(),
                "{case}"
            );
            assert_eq!(
                before_window,
                f.db.collection::<Document>(WINDOWS)
                    .find_one(doc! {})
                    .await
                    .unwrap(),
                "{case}"
            );
        }
        let published = rows.as_ref().is_ok_and(|r| !r.is_empty());
        let kind = if suffix == "0" {
            "window"
        } else if suffix == "other" {
            "unknown"
        } else if suffix.starts_with("response:") {
            "response"
        } else {
            suffix
        };
        if !detail.contains(&format!("class={label} eligible={eligible}"))
            || !detail.contains(&format!("voice_kind={kind}"))
            || published != eligible
        {
            failures.push(format!("{case}: published={published} {detail}"));
        }
        f.db.drop().await.unwrap();
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
#[tokio::test]
// Three pages: enough to cross page boundaries, small enough to finish inside
// the read's time limit on any machine. The 5,000-row timing is a measurement,
// not an assertion.
async fn voice_r2_nonvoice_inventory_has_no_voice_lookup() {
    let f = Fixture::new().await;
    f.open(true).await;
    crate::services::assistant_voice::ensure_indexes(&f.db)
        .await
        .unwrap();
    let raw =
        f.db.collection::<Document>(METERS)
            .find_one(doc! {})
            .await
            .unwrap()
            .unwrap();
    f.db.collection::<Document>(METERS)
        .delete_many(doc! {})
        .await
        .unwrap();
    f.db.collection::<Document>("security_replicas").insert_one(doc!{"_id":"survivor","generation_id":"survivor","started_at":bson::DateTime::now(),"observed_at":bson::DateTime::now()}).await.unwrap();
    let values: Vec<_> = (0..384)
        .map(|n| {
            let mut d = raw.clone();
            d.insert("_id", format!("row-{n:05}"));
            d.insert("billing_request_id", format!("request-{n:05}"));
            d.insert("transaction_id", format!("tx-{n:05}"));
            d.insert("created_at", bson::DateTime::from_millis(0));
            d.remove("forwarded_owner");
            d
        })
        .collect();
    f.db.collection::<Document>(METERS)
        .insert_many(values)
        .await
        .unwrap();
    for run in 0..3 {
        f.count();
        let start = std::time::Instant::now();
        let detail = diagnostics(&f.db).await.unwrap()[0]
            .technical_details
            .clone()
            .unwrap();
        println!(
            "VOICE_R2_NONVOICE run={run} elapsed_ms={} commands={} complete={}",
            start.elapsed().as_millis(),
            f.count(),
            detail.starts_with("complete=true")
        );
        assert!(
            detail.contains("rows=384 distinct_requests=384"),
            "{detail}"
        );
    }
    let pipeline = inventory::pipeline(selection::Source::Wallet, "", inventory::PAGE_ROWS);
    f.db.drop().await.unwrap();
    assert!(
        !pipeline.iter().any(|stage| stage
            .get_document("$lookup")
            .ok()
            .is_some_and(
                |lookup| [SESSIONS, WINDOWS].contains(&lookup.get_str("from").unwrap_or_default())
            )),
        "nonvoice pages execute voice lookups"
    );
}
#[tokio::test]
async fn voice_r2_deferrals_log_one_summary_per_pass() {
    let (f, sid, _) = fixture("responses", false).await;
    session(&f, &sid, true).await;
    f.db.collection::<Document>(SESSIONS)
        .update_many(doc! {}, doc! {"$set":{"generation":"malformed"}})
        .await
        .unwrap();
    // Two distinct poisoned requests; both get the same metadata-only reason.
    let raw: Vec<Document> =
        f.db.collection(METERS)
            .find(doc! {})
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
    let other = uuid::Uuid::new_v4().to_string();
    let mut call =
        f.db.collection::<Document>(SESSIONS)
            .find_one(doc! {})
            .await
            .unwrap()
            .unwrap();
    call.insert("_id", &other);
    f.db.collection::<Document>(SESSIONS)
        .insert_one(call)
        .await
        .unwrap();
    for mut row in raw {
        row.insert("_id", uuid::Uuid::new_v4().to_string());
        row.insert("transaction_id", uuid::Uuid::new_v4().to_string());
        row.insert("billing_request_id", format!("voice:{other}:responses"));
        f.db.collection::<Document>(METERS)
            .insert_one(row)
            .await
            .unwrap();
    }
    f.db.collection::<Document>(STATE)
        .insert_one(doc! {"_id":STATE_ID,"next_source":1})
        .await
        .unwrap();
    let capture = tempfile::NamedTempFile::new().unwrap();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_max_level(tracing::Level::WARN)
        .with_writer(capture.reopen().unwrap())
        .finish();
    let _guard = tracing::subscriber::set_default(subscriber);
    tracing::callsite::rebuild_interest_cache();
    recover(&f.db).await.unwrap();
    let logs = std::fs::read_to_string(capture.path()).unwrap();
    f.db.drop().await.unwrap();
    assert_eq!(logs.lines().count(), 1, "{logs}");
    assert!(
        logs.contains("voice_malformed_evidence") && logs.contains('2'),
        "{logs}"
    );
}
#[tokio::test]
async fn voice_r2_sealed_window_reconcile_command_count() {
    for sealed in [true, false] {
        let (f, _, id) = duration(true).await;
        f.db.collection::<Document>(WINDOWS)
            .update_many(doc! {}, doc! {"$set":{"sealed":sealed}})
            .await
            .unwrap();
        f.count();
        assert_eq!(windows::reconcile(&f.db).await.unwrap(), 1);
        let commands = f.count();
        println!("VOICE_R2_WINDOW_RECONCILE_COMMANDS sealed={sealed} commands={commands}");
        assert_eq!(commands, 64);
        window_done(&f, &id).await;
        f.db.drop().await.unwrap();
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum WindowPhase {
    Sealed,
    Read,
    Settled,
}
#[derive(Clone)]
struct WindowGate {
    phase: WindowPhase,
    entered: Arc<tokio::sync::Notify>,
    resume: Arc<tokio::sync::Notify>,
    used: Arc<std::sync::atomic::AtomicBool>,
}
impl WindowGate {
    fn new(phase: WindowPhase) -> Self {
        Self {
            phase,
            entered: Default::default(),
            resume: Default::default(),
            used: Default::default(),
        }
    }
}
tokio::task_local! {static WINDOW_GATE:WindowGate;}
pub(crate) async fn window_point(phase: WindowPhase) {
    if let Ok(g) = WINDOW_GATE.try_with(Clone::clone)
        && g.phase == phase
        && !g.used.swap(true, std::sync::atomic::Ordering::SeqCst)
    {
        g.entered.notify_one();
        g.resume.notified().await;
    }
}
async fn asserted_once(f: &Fixture, id: &str) {
    window_done(f, id).await;
    let rows: Vec<UsageMeterRow> =
        f.db.collection(METERS)
            .find(doc! {"billing_request_id":id})
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
    for row in rows {
        assert_eq!(
            f.db.collection::<Document>("billing_ledger")
                .count_documents(doc! {"dedupe_key":format!("usage-settled:{}",row.id)})
                .await
                .unwrap(),
            1
        );
    }
}
#[tokio::test]
async fn voice_r2_finalize_crashes_and_terminal_races_at_every_durable_step() {
    for phase in [WindowPhase::Sealed, WindowPhase::Read, WindowPhase::Settled] {
        for crash in [true, false] {
            let (f, sid, id) = duration(true).await;
            session(&f, &sid, false).await;
            f.db.collection::<Document>(SESSIONS)
                .update_one(doc! {}, doc! {"$set":{"observed_seconds":29_i64}})
                .await
                .unwrap();
            let gate = WindowGate::new(phase);
            let mut finalize =
                Box::pin(WINDOW_GATE.scope(gate.clone(), windows::finalize(&f.db, &id, true)));
            tokio::select! {_=&mut finalize=>panic!("missed {phase:?}"),()=gate.entered.notified()=>{}}
            if crash {
                drop(finalize);
                assert_eq!(windows::reconcile(&f.db).await.unwrap(), 1);
            } else {
                let rows = terminal::publish(&f.db, &id).await.unwrap();
                super::super::super::meter::settle_persisted(&f.db, rows)
                    .await
                    .unwrap();
                gate.resume.notify_one();
                finalize.await.unwrap();
            }
            assert_eq!(windows::reconcile(&f.db).await.unwrap(), 0);
            asserted_once(&f, &id).await;
            f.db.drop().await.unwrap();
        }
    }
    for phase in [
        Phase::BeforeTerminal,
        Phase::AfterRows,
        Phase::AfterAudit,
        Phase::BeforeCommit,
        Phase::AfterCommit,
    ] {
        let (f, sid, id) = duration(true).await;
        session(&f, &sid, false).await;
        super::super::recovery_tests::at(phase, terminal::publish(&f.db, &id)).await;
        assert_eq!(windows::reconcile(&f.db).await.unwrap(), 1);
        asserted_once(&f, &id).await;
        assert!(terminal::publish(&f.db, &id).await.unwrap().is_empty());
        f.db.drop().await.unwrap();
    }
    // A real finalizer wins after the terminal transaction took its snapshot.
    let (f, sid, id) = duration(true).await;
    session(&f, &sid, false).await;
    let gate = super::super::recovery_tests::Gate::new(Phase::BeforeTerminal);
    let mut terminal = Box::pin(
        super::super::recovery_tests::GATE.scope(gate.clone(), terminal::publish(&f.db, &id)),
    );
    tokio::select! {_=&mut terminal=>panic!("missed terminal barrier"),()=gate.entered.notified()=>{}}
    windows::finalize(&f.db, &id, true).await.unwrap();
    gate.resume.notify_one();
    assert!(terminal.await.unwrap().is_empty());
    asserted_once(&f, &id).await;
    f.db.drop().await.unwrap();
}
#[tokio::test]
async fn voice_r2_writer_report_bound_and_zero_report_semantics() {
    for count in [
        0,
        crate::services::voice::RESPONSE_USAGE_LIMIT,
        crate::services::voice::RESPONSE_USAGE_LIMIT + 1,
    ] {
        let (f, sid, id) = fixture("responses", false).await;
        session(&f, &sid, false).await;
        let usage: Document = (0..count)
            .map(|n| {
                (
                    format!("response-{n}"),
                    bson::to_bson(&PlatformUsage {
                        tokens: 1,
                        input_tokens: 1,
                        ..Default::default()
                    })
                    .unwrap(),
                )
            })
            .collect();
        f.db.collection::<Document>(SESSIONS)
            .update_one(doc! {}, doc! {"$set":{"responses_usage":usage}})
            .await
            .unwrap();
        let detail = diagnostics(&f.db).await.unwrap()[0]
            .technical_details
            .clone()
            .unwrap();
        let rows = terminal::publish(&f.db, &id).await.unwrap();
        if count > crate::services::voice::RESPONSE_USAGE_LIMIT {
            assert!(rows.is_empty());
            assert!(detail.contains("voice_malformed_evidence"), "{detail}");
        } else {
            super::super::super::meter::settle_persisted(&f.db, rows)
                .await
                .unwrap();
            check(&f, &id, [count as i64, count as i64, 0, 0, 0]).await;
        }
        f.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn voice_r2_undecidable_policy_after_evidence_does_not_fence_session() {
    let (f, sid, id) = duration(true).await;
    session(&f, &sid, false).await;
    let token_id = format!("voice:{sid}:responses");
    // A legacy wallet-only hold without an exact recoverable estimate.
    f.db.collection::<Document>(METERS)
        .update_many(
            doc! {"billing_request_id":&id},
            doc! {"$set":{"billing_request_id":&token_id},"$unset":{"funding":""}},
        )
        .await
        .unwrap();
    let before =
        f.db.collection::<Document>(SESSIONS)
            .find_one(doc! {})
            .await
            .unwrap();
    let decision = terminal::publish_with_policy(
        &f.db,
        &token_id,
        super::super::policy::UnknownConsumptionPolicy::ReservedEstimate,
    )
    .await
    .unwrap();
    assert!(matches!(
        decision,
        terminal::Outcome::Deferred("policy_quantity_unavailable")
    ));
    assert_eq!(
        before,
        f.db.collection::<Document>(SESSIONS)
            .find_one(doc! {})
            .await
            .unwrap()
    );
    assert_eq!(
        f.db.collection::<Document>(METERS)
            .count_documents(doc! {"quantity":Bson::Null,"released":false})
            .await
            .unwrap(),
        2
    );
    f.clean().await;
    f.db.drop().await.unwrap();
}

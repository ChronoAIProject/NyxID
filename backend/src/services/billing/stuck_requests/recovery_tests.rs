use super::*;
use crate::billing_integration_tests::usage_checkpoints::Fixture;
use crate::services::billing::reservation;
use chrono::{Duration, Utc};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Phase {
    BeforeTerminal,
    BeforeHoldlessRow(usize),
    AfterHoldlessRow(usize),
    AfterHoldlessSummary,
    AfterRows,
    AfterAudit,
    BeforeCommit,
    AfterCommit,
}
#[derive(Clone)]
pub(super) struct Gate {
    phase: Phase,
    pub(super) entered: Arc<tokio::sync::Notify>,
    pub(super) resume: Arc<tokio::sync::Notify>,
    used: Arc<AtomicBool>,
}
impl Gate {
    pub(super) fn new(phase: Phase) -> Self {
        Self {
            phase,
            entered: Default::default(),
            resume: Default::default(),
            used: Default::default(),
        }
    }
}
tokio::task_local! {pub(super) static GATE:Gate;}
pub(crate) async fn point(phase: Phase) {
    if let Ok(g) = GATE.try_with(Clone::clone)
        && g.phase == phase
        && !g.used.swap(true, Ordering::SeqCst)
    {
        g.entered.notify_one();
        g.resume.notified().await;
    }
}
pub(super) async fn at<T>(phase: Phase, future: impl std::future::Future<Output = T>) {
    let gate = Gate::new(phase);
    let mut running = Box::pin(GATE.scope(gate.clone(), future));
    tokio::select! {_=&mut running=>panic!("missed {phase:?}"),()=gate.entered.notified()=>{},_ =tokio::time::sleep(std::time::Duration::from_secs(15))=>panic!("barrier timeout")}
    drop(running);
}
async fn dead(f: &Fixture, legacy: bool) {
    f.db.collection::<Document>("security_replicas").update_one(doc!{"_id":"survivor"},doc!{"$set":{"generation_id":"survivor","started_at":bson::DateTime::now(),"observed_at":bson::DateTime::now()}}).upsert(true).await.unwrap();
    let mut update = doc! {"$set":{"created_at":bson::DateTime::from_millis(0),"forwarded_owner":{"generation_id":"dead","ingress":"proxy","marked_at":bson::DateTime::from_millis(0)}}};
    if legacy {
        update
            .get_document_mut("$set")
            .unwrap()
            .remove("forwarded_owner");
        update.insert("$unset", doc! {"forwarded_owner":""});
    }
    f.db.collection::<Document>(METERS)
        .update_many(doc! {}, update)
        .await
        .unwrap();
}
async fn finish(f: &Fixture) {
    super::super::funding::recover_terminal_releases(&f.db)
        .await
        .unwrap();
    reservation::recover_retryable_settlements_at(&f.db, Utc::now() + Duration::hours(2))
        .await
        .unwrap();
}
async fn zero(f: &Fixture) {
    let row = f.row().await;
    assert_eq!(row.quantity, Some(0));
    assert!(row.released);
    let wallet =
        f.db.collection::<crate::models::billing_wallet::BillingWallet>("billing_wallet")
            .find_one(doc! {"owner_id":&f.owner})
            .await
            .unwrap()
            .unwrap();
    assert_eq!(wallet.reserved_credits, Credits::ZERO);
    assert_eq!(wallet.pending_lago_debits, Credits::ZERO);
    assert_eq!(
        f.db.collection::<Document>("audit_log")
            .count_documents(doc! {"event_type":"billing_unknown_request_ended"})
            .await
            .unwrap(),
        1
    );
    let event =
        f.db.collection::<Document>("audit_log")
            .find_one(doc! {"event_type":"billing_unknown_request_ended"})
            .await
            .unwrap()
            .unwrap();
    assert!(event.get("seq").is_some());
    assert!(event.get_str("entry_hash").is_ok());
    clean(f).await;
    assert_eq!(
        f.db.collection::<Document>("billing_hold_observations")
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn stuck_request_exit_crash_matrix_and_paused_finalizer_never_double_charge() {
    for phase in [
        Phase::BeforeTerminal,
        Phase::AfterRows,
        Phase::AfterAudit,
        Phase::BeforeCommit,
        Phase::AfterCommit,
    ] {
        let f = Fixture::new().await;
        let metered = f.open(true).await;
        dead(&f, false).await;
        let row = f.row().await;
        at(phase, terminal::publish(&f.db, &row.billing_request_id)).await;
        clean(&f).await;
        finish(&f).await;
        finish(&f).await;
        zero(&f).await;
        f.service
            .settle(
                &metered,
                crate::models::service_billing::PlatformUsage::llm_completion(0, 17),
                None,
                None,
            )
            .await
            .unwrap();
        zero(&f).await;
        f.db.drop().await.unwrap();
    }
}
#[tokio::test]
async fn stuck_request_exit_finalizer_wins_transaction_race() {
    let f = Fixture::new().await;
    let metered = f.open(true).await;
    dead(&f, false).await;
    let row = f.row().await;
    let gate = Gate::new(Phase::BeforeTerminal);
    let mut running = Box::pin(GATE.scope(
        gate.clone(),
        terminal::publish(&f.db, &row.billing_request_id),
    ));
    tokio::select! {_=&mut running=>panic!("missed barrier"),()=gate.entered.notified()=>{}}
    f.service
        .settle(
            &metered,
            crate::models::service_billing::PlatformUsage::llm_completion(0, 3),
            None,
            None,
        )
        .await
        .unwrap();
    gate.resume.notify_one();
    assert!(running.await.unwrap().is_empty());
    f.charged(3).await;
    assert_eq!(
        f.db.collection::<Document>("audit_log")
            .count_documents(doc! {"event_type":"billing_unknown_request_ended"})
            .await
            .unwrap(),
        0
    );
    f.db.drop().await.unwrap();
}
#[tokio::test]
async fn stuck_request_exit_legacy_and_each_ordinary_ingress_use_normal_settlement() {
    for (ingress, legacy) in [
        ("proxy", false),
        ("llm_gateway", false),
        ("llm_provider", false),
        ("proxy", true),
    ] {
        let f = Fixture::new().await;
        let _metered = f.open(true).await;
        dead(&f, legacy).await;
        if !legacy {
            f.db.collection::<Document>(METERS)
                .update_many(doc! {}, doc! {"$set":{"forwarded_owner.ingress":ingress}})
                .await
                .unwrap();
        }
        finish(&f).await;
        finish(&f).await;
        zero(&f).await;
        f.db.drop().await.unwrap();
    }
}
#[tokio::test]
async fn stuck_request_exit_live_owner_and_protocol_exclusions_are_untouched() {
    for change in [
        doc! {"forwarded_owner.generation_id":"survivor"},
        doc! {"pool_attempt":{}},
        doc! {"realtime_session":{}},
        doc! {"managed_execution_id":"pending-review"},
        doc! {"model_tariff":{}},
        doc! {"metric":"machine_running_seconds"},
        doc! {"metric":"sandbox_cpu_millis_seconds"},
        doc! {"billing_request_id":"voice:session:1"},
        doc! {"quantity":3_i64,"status":"dead_letter"},
        doc! {"pending_platform_usage":{}},
        doc! {"pending_resale_quantity":0_i64},
        doc! {"usage_checkpoint":{"final_report":true,"recovered":false}},
        doc! {"forwarded_owner.marked_at":bson::DateTime::now()},
    ] {
        let f = Fixture::new().await;
        let _metered = f.open(true).await;
        dead(&f, false).await;
        f.db.collection::<Document>(METERS)
            .update_many(doc! {}, doc! {"$set":change})
            .await
            .unwrap();
        let before =
            f.db.collection::<Document>(METERS)
                .find_one(doc! {})
                .await
                .unwrap()
                .unwrap();
        assert!(
            terminal::publish(&f.db, before.get_str("billing_request_id").unwrap())
                .await
                .unwrap()
                .is_empty()
        );
        let after =
            f.db.collection::<Document>(METERS)
                .find_one(doc! {})
                .await
                .unwrap()
                .unwrap();
        assert_eq!(before, after);
        assert_eq!(
            f.db.collection::<Document>("audit_log")
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
        f.db.drop().await.unwrap();
    }
}
#[tokio::test]
async fn stuck_request_exit_final_checkpoint_precedes_unknown_policy() {
    let f = Fixture::new().await;
    let metered = f.open(true).await;
    dead(&f, false).await;
    let mut recorder = f.service.stream_checkpoints(&metered, true);
    recorder.observe_value(None,&serde_json::json!({"choices":[],"usage":{"prompt_tokens":1,"completion_tokens":2,"total_tokens":3}})).await;
    f.db.collection::<Document>(METERS)
        .update_many(
            doc! {},
            doc! {"$set":{"usage_checkpoint.recover_after":bson::DateTime::from_millis(0)}},
        )
        .await
        .unwrap();
    finish(&f).await;
    finish(&f).await;
    f.charged(3).await;
    assert_eq!(
        f.db.collection::<Document>("audit_log")
            .count_documents(doc! {"event_type":"billing_unknown_request_ended"})
            .await
            .unwrap(),
        0
    );
    f.db.drop().await.unwrap();
}
#[tokio::test]
async fn stuck_request_exit_policy_both_choices_preserve_final_evidence() {
    use policy::UnknownConsumptionPolicy::{ReservedEstimate, Waive};
    let f = Fixture::new().await;
    let _metered = f.open(true).await;
    let mut row = f.row().await;
    assert_eq!(Waive.quantity(&row), Some(0));
    let reserved = row.funding.as_ref().unwrap().reserved_quantity.unwrap();
    assert_eq!(ReservedEstimate.quantity(&row), Some(reserved));
    row.funding.as_mut().unwrap().reserved_quantity = None;
    assert_eq!(
        ReservedEstimate.quantity(&row),
        Some(reserved),
        "old exact reservations reconstruct their original quantity"
    );
    let original = row.clone();
    row.reserved_credits = Credits::from_pico(800_000).unwrap();
    row.funding.as_mut().unwrap().credits_per_unit_pico = Some(800_000);
    assert_eq!(
        ReservedEstimate.quantity(&row),
        Some(1),
        "sub-micro reservation remains exact"
    );
    row.reserved_credits = Credits::from_pico(800_001).unwrap();
    assert_eq!(
        ReservedEstimate.quantity(&row),
        None,
        "an ambiguous old amount is never rounded into usage"
    );
    row = original;
    row.quantity = Some(3);
    assert_eq!(Waive.quantity(&row), None);
    assert_eq!(ReservedEstimate.quantity(&row), None);
    row.quantity = None;
    row.funding = None;
    assert_eq!(ReservedEstimate.quantity(&row), None);
    assert_eq!(Waive.quantity(&row), Some(0));
    f.db.drop().await.unwrap();
}
#[tokio::test]
async fn stuck_request_exit_audit_failure_aborts_and_budget_cancels_uncommitted_step() {
    let f = Fixture::new().await;
    let _metered = f.open(true).await;
    dead(&f, false).await;
    f.db.run_command(doc!{"create":"audit_log","validator":{"event_type":{"$ne":"billing_unknown_request_ended"}}}).await.unwrap();
    let row = f.row().await;
    assert!(
        terminal::publish(&f.db, &row.billing_request_id)
            .await
            .is_err()
    );
    assert_eq!(f.row().await.quantity, None);
    clean(&f).await;
    f.db.run_command(doc! {"collMod":"audit_log","validator":{}})
        .await
        .unwrap();
    let gate = Gate::new(Phase::AfterRows);
    let mut run = Box::pin(GATE.scope(gate.clone(), recovery::recover(&f.db)));
    tokio::select! {_=&mut run=>panic!("missed budget barrier"),()=gate.entered.notified()=>{}}
    tokio::time::pause();
    tokio::time::advance(std::time::Duration::from_secs(6)).await;
    run.await.unwrap();
    tokio::time::resume();
    assert_eq!(f.row().await.quantity, None);
    clean(&f).await;
    finish(&f).await;
    zero(&f).await;
    f.db.drop().await.unwrap();
}

async fn mixed(f: &Fixture) -> String {
    use crate::models::billing_target::BillingTargetKind;
    use crate::services::billing::grants::{IssueCreditGrantInput, issue_grants};
    let seed = f.open(false).await;
    let mut route = seed.route.unwrap().with_native_text_bound(2, 1, 32);
    f.db.collection::<Document>(METERS)
        .delete_many(doc! {})
        .await
        .unwrap();
    route.billing_request_id = uuid::Uuid::new_v4().to_string();
    route.platform_billable = true;
    route.service_platform_billable = true;
    let now = bson::DateTime::now();
    f.db.collection::<Document>("usage_allowances").insert_one(doc!{"_id":"included","service_id":route.catalog_service_id.as_deref().unwrap(),"service_slug":route.service_slug.as_deref().unwrap(),"metric":"tokens","quantity":1_i64,"recurrence":"one_time","target_kind":"all_users","is_active":true,"created_by":"admin","created_at":now,"updated_at":now}).await.unwrap();
    issue_grants(
        &f.db,
        IssueCreditGrantInput {
            amount_credits: 1,
            target_kind: BillingTargetKind::SelectedUsers,
            target_user_ids: vec![f.owner.clone()],
            target_org_ids: vec![],
            target_group_ids: vec![],
            all_services: true,
            service_refs: vec![],
            expires_at: None,
            reason: None,
            granted_by: f.owner.clone(),
            amount: None,
            service_limits: vec![],
            request_id: None,
        },
    )
    .await
    .unwrap();
    let tax = doc! {"code":"test","name":"test","rate_percent":Credits::from_whole(9)};
    f.db.collection::<Document>("billing_tax_profiles").update_many(doc!{},doc!{"$set":{"default_taxes":[tax.clone()],"metric_taxes.platform_tokens":[tax],"fingerprint":"unknown-exit-tax"}}).await.unwrap();
    let metered = f.service.open(&route).await.unwrap();
    f.service.mark_forwarded(&metered).await.unwrap();
    dead(f, false).await;
    let grant =
        f.db.collection::<Document>("credit_grants")
            .find_one(doc! {})
            .await
            .unwrap()
            .unwrap();
    grant.get_str("_id").unwrap().to_owned()
}
async fn benefits_returned(f: &Fixture) {
    zero(f).await;
    let grant =
        f.db.collection::<crate::models::credit_grant::CreditGrant>("credit_grants")
            .find_one(doc! {})
            .await
            .unwrap()
            .unwrap();
    assert_eq!(grant.reserved, Credits::ZERO);
    assert_eq!(grant.remaining, Credits::from_whole(1));
    let period =
        f.db.collection::<crate::models::usage_allowance_period::UsageAllowancePeriod>(
            "usage_allowance_periods",
        )
        .find_one(doc! {})
        .await
        .unwrap()
        .unwrap();
    assert_eq!((period.reserved_quantity, period.consumed_quantity), (0, 0));
}
#[tokio::test]
async fn stuck_request_exit_all_benefit_handoffs_return_gross_holds_and_keep_accounts_clean() {
    use crate::billing_integration_tests::hold_release::{Phase as MoneyPhase, crash_money_at};
    for phase in [
        MoneyPhase::AllowanceLocked,
        MoneyPhase::AllowanceApplied,
        MoneyPhase::AllowanceRecorded,
        MoneyPhase::GrantLocked,
        MoneyPhase::GrantApplied,
        MoneyPhase::GrantRecorded,
        MoneyPhase::WalletLocked,
        MoneyPhase::WalletApplied,
        MoneyPhase::WalletReleased,
    ] {
        let f = Fixture::new().await;
        let _grant = mixed(&f).await;
        let row = f.row().await;
        let funding = row.funding.as_ref().unwrap();
        assert_eq!(funding.allowance_reservations.len(), 1);
        assert_eq!(funding.grant_reservations.len(), 1);
        assert!(row.reserved_credits > Credits::ZERO);
        let rows = terminal::publish(&f.db, &row.billing_request_id)
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        crash_money_at(phase, super::super::meter::settle_persisted(&f.db, rows)).await;
        clean(&f).await;
        f.db.collection::<Document>(METERS).update_many(doc!{},doc!{"$set":{"funding.settlement_claimed_at":bson::DateTime::from_millis(0),"settlement_next_retry_at":bson::DateTime::from_millis(0)}}).await.unwrap();
        finish(&f).await;
        finish(&f).await;
        benefits_returned(&f).await;
        f.db.drop().await.unwrap();
    }
}
#[tokio::test]
async fn stuck_request_exit_expired_grant_finishes_in_existing_sweep() {
    let f = Fixture::new().await;
    let grant = mixed(&f).await;
    f.db.collection::<Document>("credit_grants")
        .update_one(
            doc! {"_id":&grant},
            doc! {"$set":{"expires_at":bson::DateTime::from_millis(0)}},
        )
        .await
        .unwrap();
    assert_eq!(
        super::super::grants::expire_due_grants(&f.db, Utc::now())
            .await
            .unwrap(),
        0
    );
    finish(&f).await;
    benefits_returned(&f).await;
    assert_eq!(
        super::super::grants::expire_due_grants(&f.db, Utc::now())
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        super::super::grants::expire_due_grants(&f.db, Utc::now())
            .await
            .unwrap(),
        0
    );
    clean(&f).await;
    f.db.drop().await.unwrap();
}
#[tokio::test]
async fn stuck_request_exit_registry_failure_never_ends_a_request() {
    let f = Fixture::new().await;
    let _metered = f.open(true).await;
    dead(&f, false).await;
    f.db.collection::<Document>("security_replicas")
        .drop()
        .await
        .unwrap();
    f.db.collection::<Document>("registry_source")
        .insert_one(doc! {"bad":"not-a-number"})
        .await
        .unwrap();
    f.db.run_command(doc!{"create":"security_replicas","viewOn":"registry_source","pipeline":[{"$set":{"observed_at":{"$toDate":{"$toInt":"$bad"}}}}]}).await.unwrap();
    let row = f.row().await;
    assert!(
        terminal::publish(&f.db, &row.billing_request_id)
            .await
            .is_err()
    );
    recovery::recover(&f.db).await.unwrap();
    assert_eq!(f.row().await.quantity, None);
    let diagnostics = diagnostics(&f.db).await.unwrap();
    assert!(
        diagnostics[0]
            .technical_details
            .as_ref()
            .unwrap()
            .contains("registry_unavailable"),
        "{}",
        diagnostics[0].technical_details.as_ref().unwrap()
    );
    f.db.drop().await.unwrap();
}
#[tokio::test]
async fn stuck_request_exit_new_admission_is_not_released_by_a_late_finalizer() {
    let f = Fixture::new().await;
    let old = f.open(true).await;
    dead(&f, false).await;
    finish(&f).await;
    zero(&f).await;
    let new = f.open(true).await;
    let wallets =
        f.db.collection::<crate::models::billing_wallet::BillingWallet>("billing_wallet");
    let held = wallets
        .find_one(doc! {"owner_id":&f.owner})
        .await
        .unwrap()
        .unwrap()
        .reserved_credits;
    assert!(held > Credits::ZERO);
    f.service
        .settle(
            &old,
            crate::models::service_billing::PlatformUsage::llm_completion(0, 20),
            None,
            None,
        )
        .await
        .unwrap();
    assert_eq!(
        wallets
            .find_one(doc! {"owner_id":&f.owner})
            .await
            .unwrap()
            .unwrap()
            .reserved_credits,
        held
    );
    f.service
        .settle(
            &new,
            crate::models::service_billing::PlatformUsage::llm_completion(0, 0),
            None,
            None,
        )
        .await
        .unwrap();
    clean(&f).await;
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn stuck_request_exit_two_publishers_commit_one_decision() {
    let f = Fixture::new().await;
    let _metered = f.open(true).await;
    dead(&f, false).await;
    let row = f.row().await;
    let gate = Gate::new(Phase::BeforeTerminal);
    let mut first = Box::pin(GATE.scope(
        gate.clone(),
        terminal::publish(&f.db, &row.billing_request_id),
    ));
    tokio::select! {_=&mut first=>panic!("missed barrier"),()=gate.entered.notified()=>{}}
    let winner = terminal::publish(&f.db, &row.billing_request_id)
        .await
        .unwrap();
    assert_eq!(winner.len(), 1);
    gate.resume.notify_one();
    assert!(first.await.unwrap().is_empty());
    super::super::meter::settle_persisted(&f.db, winner)
        .await
        .unwrap();
    finish(&f).await;
    zero(&f).await;
    f.db.drop().await.unwrap();
}
#[tokio::test]
async fn stuck_request_exit_cancelled_after_commit_resumes_ordinary_settlement() {
    let f = Fixture::new().await;
    let _metered = f.open(true).await;
    dead(&f, false).await;
    // The recovery pass deadline can fall after publishing the terminal
    // quantity and before inline settlement. The next ordinary reconciliation
    // must finish the committed decision without publishing it again.
    at(Phase::AfterCommit, recovery::recover(&f.db)).await;
    let row = f.row().await;
    assert_eq!(row.quantity, Some(0));
    assert!(!row.released);
    recovery::recover(&f.db).await.unwrap();
    finish(&f).await;
    finish(&f).await;
    zero(&f).await;
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn stuck_request_exit_batch_is_bounded_cursor_progresses_and_counts_are_durable() {
    let f = Fixture::new().await;
    for _ in 0..52 {
        let _metered = f.open(true).await;
    }
    dead(&f, false).await;
    f.count();
    let started = std::time::Instant::now();
    recovery::recover(&f.db).await.unwrap();
    let elapsed = started.elapsed();
    let commands = f.count();
    let state =
        f.db.collection::<Document>(STATE)
            .find_one(doc! {"_id":STATE_ID})
            .await
            .unwrap()
            .unwrap();
    let ended = state.get_i64("ended_requests").unwrap();
    assert!((1..=50).contains(&ended));
    println!(
        "STUCK_REQUEST_BATCH requests={ended} commands={commands} elapsed_ms={}",
        elapsed.as_millis()
    );
    for _ in 0..10 {
        recovery::recover(&f.db).await.unwrap();
        if f.db
            .collection::<Document>(METERS)
            .count_documents(doc! {"released":false,"quantity":bson::Bson::Null})
            .await
            .unwrap()
            == 0
        {
            break;
        }
    }
    assert_eq!(
        f.db.collection::<Document>(METERS)
            .count_documents(doc! {"released":false,"quantity":bson::Bson::Null})
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        f.db.collection::<Document>("audit_log")
            .count_documents(doc! {"event_type":"billing_unknown_request_ended"})
            .await
            .unwrap(),
        52
    );
    // A pass can commit its last quantity immediately before its budget
    // cancels inline settlement. Production's ordinary reconciliation finishes
    // that durable decision after any interrupted worker's funding lease
    // expires. Advancing the retry scan alone does not advance that lease's
    // independent UTC clock, so model expiry while retaining its claim fence.
    f.db.collection::<Document>(METERS)
        .update_many(
            doc! {"released":false,"quantity":{"$ne":Bson::Null},
            "funding.settlement_claim_id":{"$type":"string"}},
            doc! {"$set":{"funding.settlement_claimed_at":bson::DateTime::from_millis(0),
            "settlement_next_retry_at":bson::DateTime::from_millis(0)}},
        )
        .await
        .unwrap();
    finish(&f).await;
    finish(&f).await;
    assert_eq!(
        f.db.collection::<Document>(METERS)
            .count_documents(doc! {"released":false})
            .await
            .unwrap(),
        0
    );
    f.count();
    recovery::recover(&f.db).await.unwrap();
    let idle = f.count();
    println!("STUCK_REQUEST_RECOVERY_IDLE_COMMANDS {idle}");
    let d = diagnostics(&f.db).await.unwrap();
    assert!(
        d[0].technical_details
            .as_ref()
            .unwrap()
            .contains("ended_requests=52 ended_rows=52"),
        "{}",
        d[0].technical_details.as_ref().unwrap()
    );
    clean(&f).await;
    f.db.drop().await.unwrap();
}
#[tokio::test]
async fn stuck_request_exit_one_request_command_count() {
    let f = Fixture::new().await;
    let _metered = f.open(true).await;
    dead(&f, false).await;
    let row = f.row().await;
    f.count();
    let rows = terminal::publish(&f.db, &row.billing_request_id)
        .await
        .unwrap();
    let publication = f.count();
    super::super::meter::settle_persisted(&f.db, rows)
        .await
        .unwrap();
    let settlement = f.count();
    println!("STUCK_REQUEST_EXIT_COMMANDS terminal={publication} settlement={settlement}");
    zero(&f).await;
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn stuck_request_exit_component_intent_and_terminal_decision_exclude_each_other() {
    use crate::models::service_billing::{BillingMetric, PlatformUsage, ResaleSpec, ResaleUsage};
    use crate::services::billing::meter;
    for intent_wins in [true, false] {
        let f = Fixture::new().await;
        let seed = f.open(false).await;
        f.db.collection::<Document>(METERS)
            .delete_many(doc! {})
            .await
            .unwrap();
        let mut route = seed.route.unwrap().with_native_text_bound(1, 1, 1);
        route.billing_request_id = uuid::Uuid::new_v4().to_string();
        route.platform_billable = true;
        route.service_platform_billable = true;
        route.platform_components = vec![ResaleSpec {
            metric: BillingMetric::InputTokens,
            lago_metric_code: "stuck_input".into(),
        }];
        route.resale = Some(ResaleSpec {
            metric: BillingMetric::Requests,
            lago_metric_code: "stuck_resale".into(),
        });
        f.db.collection::<crate::models::billing_rate_cache::BillingRateCache>(
            "billing_rate_cache",
        )
        .insert_many([
            crate::billing_integration_tests::rate("stuck_input", Utc::now()),
            crate::billing_integration_tests::rate("stuck_resale", Utc::now()),
        ])
        .await
        .unwrap();
        let metered = f.service.open(&route).await.unwrap();
        f.service.mark_forwarded(&metered).await.unwrap();
        dead(&f, false).await;
        let usage = PlatformUsage {
            requests: 1,
            tokens: 5,
            input_tokens: 2,
            output_tokens: 3,
            ..Default::default()
        };
        let resale = Some(ResaleUsage {
            metric: BillingMetric::Requests,
            quantity: 1,
        });
        if intent_wins {
            let gate = Gate::new(Phase::BeforeTerminal);
            let mut cleanup = Box::pin(GATE.scope(
                gate.clone(),
                terminal::publish(&f.db, &route.billing_request_id),
            ));
            tokio::select! {_=&mut cleanup=>panic!("missed barrier"),()=gate.entered.notified()=>{}}
            meter::save_component_intent(&f.db, &route, &usage, resale.clone(), None)
                .await
                .unwrap();
            gate.resume.notify_one();
            assert!(cleanup.await.unwrap().is_empty());
            finish(&f).await;
        } else {
            let rows = terminal::publish(&f.db, &route.billing_request_id)
                .await
                .unwrap();
            assert_eq!(rows.len(), 3);
            meter::settle_persisted(&f.db, rows).await.unwrap();
            f.service
                .settle(&metered, usage, resale, None)
                .await
                .unwrap();
        }
        assert_eq!(
            f.db.collection::<Document>(METERS)
                .count_documents(doc! {"released":false})
                .await
                .unwrap(),
            0
        );
        let wallet =
            f.db.collection::<crate::models::billing_wallet::BillingWallet>("billing_wallet")
                .find_one(doc! {})
                .await
                .unwrap()
                .unwrap();
        assert_eq!(wallet.reserved_credits, Credits::ZERO);
        assert_eq!(
            wallet.pending_lago_debits,
            Credits::from_whole(if intent_wins { 8 } else { 0 })
        );
        assert_eq!(
            f.db.collection::<Document>("audit_log")
                .count_documents(doc! {"event_type":"billing_unknown_request_ended"})
                .await
                .unwrap(),
            u64::from(!intent_wins)
        );
        clean(&f).await;
        f.db.drop().await.unwrap();
    }
}

async fn clean(f: &Fixture) {
    f.clean().await;
    assert_eq!(
        f.db.collection::<Document>("billing_hold_observations")
            .count_documents(doc! {})
            .await
            .unwrap(),
        0,
        "account and hold conservation both stay clean"
    );
}

#[tokio::test]
async fn stuck_request_exit_selects_held_requests_and_preserves_request_wide_fence() {
    let f = Fixture::new().await;
    let _metered = f.open(true).await;
    dead(&f, false).await;
    let original = f.row().await;
    let raw =
        f.db.collection::<Document>(METERS)
            .find_one(doc! {})
            .await
            .unwrap()
            .unwrap();
    let mut zero = raw.clone();
    zero.insert("_id", "held-sibling");
    zero.insert("transaction_id", "held-sibling");
    // Move the one existing hold to the sibling. A zero-hold coordinator must
    // still be terminally fenced against a resumed ordinary finalizer.
    f.db.collection::<Document>(METERS)
        .update_one(
            doc! {"_id":&original.id},
            doc! {"$set":{"reserved_credits":Credits::ZERO}},
        )
        .await
        .unwrap();
    f.db.collection::<Document>(METERS)
        .insert_one(&zero)
        .await
        .unwrap();
    let rows: Vec<_> = (0..12_000)
        .map(|i| {
            let mut row = zero.clone();
            row.insert("_id", format!("a-empty-{i:05}"));
            row.insert("transaction_id", format!("empty-{i}"));
            row.insert("billing_request_id", format!("empty-{i}"));
            row.insert("reserved_credits", Credits::ZERO);
            if i % 2 == 0 {
                row.insert("wallet_id", Bson::Null);
            }
            row
        })
        .collect();
    f.db.collection::<Document>(METERS)
        .insert_many(rows)
        .await
        .unwrap();
    let untouched =
        f.db.collection::<Document>(METERS)
            .find_one(doc! {"billing_request_id":"empty-0"})
            .await
            .unwrap()
            .unwrap();
    for _ in 0..4 {
        recovery::recover(&f.db).await.unwrap();
    }
    assert_eq!(
        untouched,
        f.db.collection::<Document>(METERS)
            .find_one(doc! {"billing_request_id":"empty-0"})
            .await
            .unwrap()
            .unwrap()
    );
    let ended =
        f.db.collection::<Document>(METERS)
            .find_one(doc! {"_id":&original.id})
            .await
            .unwrap()
            .unwrap();
    assert_eq!(ended.get_i64("quantity").unwrap(), 0);
    assert!(ended.get_bool("released").unwrap());
    assert_eq!(
        f.db.collection::<Document>(METERS)
            .count_documents(doc! {"released":false,"quantity":Bson::Null})
            .await
            .unwrap(),
        12_000
    );
    assert_eq!(
        f.db.collection::<Document>("audit_log")
            .count_documents(doc! {"event_type":"billing_unknown_request_ended"})
            .await
            .unwrap(),
        1
    );
    // The terminal API itself rechecks holds even if handed a hold-less request.
    assert!(
        terminal::publish(&f.db, "empty-0")
            .await
            .unwrap()
            .is_empty()
    );
    f.service
        .settle(
            &_metered,
            crate::models::service_billing::PlatformUsage {
                requests: 1,
                tokens: 5,
                ..Default::default()
            },
            None,
            None,
        )
        .await
        .unwrap();
    let sibling =
        f.db.collection::<Document>(METERS)
            .find_one(doc! {"_id":"held-sibling"})
            .await
            .unwrap()
            .unwrap();
    assert_eq!(sibling.get_i64("quantity").unwrap(), 0);
    assert!(sibling.get_bool("released").unwrap());
    let wallet =
        f.db.collection::<crate::models::billing_wallet::BillingWallet>("billing_wallet")
            .find_one(doc! {"owner_id":&f.owner})
            .await
            .unwrap()
            .unwrap();
    assert_eq!(wallet.reserved_credits, Credits::ZERO);
    assert_eq!(wallet.pending_lago_debits, Credits::ZERO);
    clean(&f).await;
    f.db.drop().await.unwrap();
}

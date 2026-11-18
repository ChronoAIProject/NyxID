mod benefit_repair;
mod round3;

use super::*;
use crate::models::{chain_verify_status::ChainVerifyOutcome, credits::Credits};
use crate::services::billing::{account_reconciliation, funding, reservation};
use std::sync::{Mutex, atomic::AtomicBool};
use tokio::sync::Notify;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Phase {
    BeforeTransaction,
    AfterMeter,
    AfterWallet,
    AfterCommit,
    AllowanceLocked,
    AllowanceApplied,
    AllowanceRecorded,
    GrantLocked,
    GrantApplied,
    GrantRecorded,
    WalletLocked,
    WalletApplied,
    WalletReleased,
    BeforeHoldRepair,
    AfterHoldRepairWallet,
    AfterHoldRepairAudit,
    AfterHoldObservationDelete,
}

#[derive(Clone)]
struct Gate {
    phase: Phase,
    entered: Arc<Notify>,
    resume: Arc<Notify>,
    used: Arc<AtomicBool>,
}
impl Gate {
    fn new(phase: Phase) -> Self {
        Self {
            phase,
            entered: Arc::new(Notify::new()),
            resume: Arc::new(Notify::new()),
            used: Arc::new(AtomicBool::new(false)),
        }
    }
    async fn entered(&self) {
        tokio::time::timeout(std::time::Duration::from_secs(10), self.entered.notified())
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

struct Fixture {
    db: mongodb::Database,
    owner: String,
    service: BillingService,
    commands: Arc<Mutex<Vec<String>>>,
}
impl Fixture {
    async fn new() -> Self {
        let commands = Arc::new(Mutex::new(Vec::new()));
        let recorded = commands.clone();
        let db = crate::test_utils::connect_transaction_test_database_with_command_handler(
            "hold_release",
            mongodb::event::EventHandler::callback(move |event| {
                if let mongodb::event::command::CommandEvent::Started(event) = event {
                    let collection = event.command.get_str(&event.command_name).unwrap_or("");
                    recorded
                        .lock()
                        .unwrap()
                        .push(format!("{}:{collection}", event.command_name));
                }
            }),
        )
        .await;
        create_usage_index(&db).await;
        db.collection::<bson::Document>("audit_log")
            .create_index(
                mongodb::IndexModel::builder()
                    .keys(doc! {"seq":1})
                    .options(
                        mongodb::options::IndexOptions::builder()
                            .name("audit_log_seq_unique".to_owned())
                            .unique(true)
                            .partial_filter_expression(doc! {"seq":{"$exists":true}})
                            .build(),
                    )
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
                mongodb::IndexModel::builder()
                    .keys(doc! {"postings.account":1,"seq":1})
                    .build(),
            )
            .await
            .unwrap();
        insert_fresh_rates(&db).await;
        let owner = insert_owner(&db).await;
        let lago = Arc::new(FakeLago {
            tax_rate: Some("9".into()),
            wallet_balance: Some(Credits::from_whole(100)),
            ..Default::default()
        });
        let service = billing_service(&db, lago, 1);
        service.ensure_wallet(&owner).await.unwrap();
        Self {
            db,
            owner,
            service,
            commands,
        }
    }
    fn request(&self) -> BillingRouteContext {
        route_context(
            &CoverageCase {
                ingress: BillingIngress::Proxy,
                scenario: "hold_release",
                node_intent: NodeIntent::Direct,
                metric: BillingMetric::Requests,
            },
            &Uuid::new_v4().to_string(),
            &self.owner,
        )
    }
    async fn accounts_clean(&self) -> bool {
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
        status.break_detail.is_some()
    }
    async fn holds_clean(&self) {
        self.accounts_clean().await;
        self.accounts_clean().await;
        assert!(
            crate::services::billing::startup_diagnostics(&self.db)
                .await
                .unwrap()
                .iter()
                .all(|d| d.code != "billing_hold_conservation")
        );
        assert_eq!(
            self.db
                .collection::<bson::Document>("billing_hold_observations")
                .count_documents(doc! {})
                .await
                .unwrap(),
            0,
            "no provisional mismatch at a valid protocol step"
        );
    }
    async fn recover(&self) {
        // These are the release/settlement stages of the surviving reconcile pass.
        reservation::abandon_stale_unforwarded(&self.db, Utc::now() + Duration::hours(1))
            .await
            .unwrap();
        funding::recover_terminal_releases(&self.db).await.unwrap();
        reservation::recover_retryable_settlements_at(&self.db, Utc::now() + Duration::hours(1))
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn hold_release_crash_after_meter_claim_returns_wallet_hold() {
    let f = Fixture::new().await;
    let ctx = f.request();
    let metered = f.service.open(&ctx).await.unwrap();
    f.accounts_clean().await;
    let held: Credits = "1.09".parse().unwrap();
    assert_eq!(wallet(&f.db, &f.owner).await.reserved_credits, held);
    let gate = Gate::new(Phase::AfterMeter);
    let mut release = Box::pin(GATE.scope(gate.clone(), f.service.fail(&metered, "cancelled")));
    tokio::select! {
        result = &mut release => panic!("release finished before crash point: {result:?}"),
        () = gate.entered() => {},
    }
    let row =
        f.db.collection::<UsageMeterRow>(USAGE_METER)
            .find_one(doc! {"billing_request_id":&ctx.billing_request_id})
            .await
            .unwrap()
            .unwrap();
    println!(
        "HOLD_RELEASE_CRASH after_meter: released={}, reserved={}",
        row.released,
        wallet(&f.db, &f.owner).await.reserved_credits
    );
    drop(release);
    f.accounts_clean().await;
    f.recover().await;
    f.recover().await;
    f.accounts_clean().await;
    let after = wallet(&f.db, &f.owner).await;
    println!(
        "HOLD_RELEASE_CRASH after_recovery: reserved={}, accounts=Ok",
        after.reserved_credits
    );
    f.db.drop().await.unwrap();
    assert_eq!(
        after.reserved_credits,
        Credits::ZERO,
        "survivor must return the exact hold once"
    );
}

#[tokio::test]
async fn hold_release_command_counts() {
    let f = Fixture::new().await;
    let warm = f.service.open(&f.request()).await.unwrap();
    f.service.fail(&warm, "warmup").await.unwrap();
    f.commands.lock().unwrap().clear();
    let metered = f.service.open(&f.request()).await.unwrap();
    let admission = std::mem::take(&mut *f.commands.lock().unwrap());
    assert_eq!(admission.len(), 18, "{admission:?}");
    f.service.fail(&metered, "release").await.unwrap();
    let release = std::mem::take(&mut *f.commands.lock().unwrap());
    f.service.fail(&metered, "replay").await.unwrap();
    let replay = std::mem::take(&mut *f.commands.lock().unwrap());
    println!(
        "HOLD_RELEASE_COMMANDS admission={}, release={}, replay={}: {release:?}",
        admission.len(),
        release.len(),
        replay.len()
    );
    assert_eq!(release.len(), 5, "{release:?}");
    assert_eq!(replay.len(), 1, "{replay:?}");
    f.accounts_clean().await;
    assert_eq!(
        wallet(&f.db, &f.owner).await.reserved_credits,
        Credits::ZERO
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn usage_capture_fake_refuses_blank_subscription() {
    let provider = FakeLago::default();
    for subscription in ["", " \t\n"] {
        assert!(provider.current_usage("owner", subscription).await.is_err());
    }
}

#[tokio::test]
async fn usage_capture_hot_path_command_counts() {
    let f = Fixture::new().await;
    let warm = f.service.open(&f.request()).await.unwrap();
    f.service.fail(&warm, "warmup").await.unwrap();
    f.commands.lock().unwrap().clear();
    let metered = f.service.open(&f.request()).await.unwrap();
    let admission = std::mem::take(&mut *f.commands.lock().unwrap());
    f.service.mark_forwarded(&metered).await.unwrap();
    let forwarding = std::mem::take(&mut *f.commands.lock().unwrap());
    f.service
        .settle(&metered, PlatformUsage::single_request(0), None, None)
        .await
        .unwrap();
    let settlement = std::mem::take(&mut *f.commands.lock().unwrap());
    let unforwarded = f.service.open(&f.request()).await.unwrap();
    f.commands.lock().unwrap().clear();
    f.service.fail(&unforwarded, "release").await.unwrap();
    let release = std::mem::take(&mut *f.commands.lock().unwrap());
    for commands in [&admission, &forwarding, &settlement, &release] {
        assert!(commands.iter().all(|c| !c.contains("billing_usage_")));
    }
    assert_eq!(
        (
            admission.len(),
            forwarding.len(),
            settlement.len(),
            release.len()
        ),
        (18, 1, 29, 5)
    );
    println!(
        "USAGE_CAPTURE_HOT_PATH admission={} forwarding={} settlement={} release={}",
        admission.len(),
        forwarding.len(),
        settlement.len(),
        release.len()
    );
    f.db.drop().await.unwrap();
}

impl Fixture {
    async fn mixed_request(&self) -> (BillingRouteContext, String) {
        use crate::models::billing_target::BillingTargetKind;
        use crate::models::credit_service_limit::CreditServiceLimit;
        use crate::services::billing::grants::{IssueCreditGrantInput, issue_grants};
        let mut ctx = self.request();
        ctx.platform_metric = BillingMetric::Images;
        ctx.platform_lago_metric_code = "hold_images".into();
        ctx.requested_images = 3;
        self.db
            .collection::<BillingRateCache>(BILLING_RATE_CACHE)
            .insert_one(rate("hold_images", Utc::now()))
            .await
            .unwrap();
        let now = bson::DateTime::now();
        self.db
            .collection::<bson::Document>("usage_allowances")
            .insert_one(doc! {
                "_id": "included", "service_id": ctx.catalog_service_id.as_deref().unwrap(),
                "service_slug": ctx.service_slug.as_deref().unwrap(),
                "metric": "images", "quantity": 1_i64, "recurrence": "one_time",
                "target_kind": "all_users", "is_active": true, "created_by": "admin",
                "created_at": now, "updated_at": now,
            })
            .await
            .unwrap();
        let grants = issue_grants(
            &self.db,
            IssueCreditGrantInput {
                amount_credits: 1,
                target_kind: BillingTargetKind::SelectedUsers,
                target_user_ids: vec![self.owner.clone()],
                target_org_ids: vec![],
                target_group_ids: vec![],
                all_services: true,
                service_refs: vec![],
                expires_at: None,
                reason: None,
                granted_by: self.owner.clone(),
                amount: None,
                service_limits: vec![],
                request_id: None,
            },
        )
        .await
        .unwrap();
        let grant = &grants[0];
        let rule = CreditServiceLimit {
            key: "release-test".into(),
            service_ids: vec![ctx.catalog_service_id.clone().unwrap()],
            service_slugs: vec![ctx.service_slug.clone().unwrap()],
            label: None,
            limit: Credits::from_whole(1),
            consumed: Credits::ZERO,
            reserved: Credits::ZERO,
        };
        self.db
            .collection::<bson::Document>("credit_grants")
            .update_one(
                doc! {"_id":&grant.id},
                doc! {"$set": {"service_limits":bson::to_bson(&vec![rule]).unwrap()}},
            )
            .await
            .unwrap();
        (ctx, grant.id.clone())
    }

    async fn assert_mixed_released(&self, request_id: &str, grant_id: &str) {
        let row = usage_row(&self.db, request_id).await;
        assert!(row.released);
        let funding = row.funding.unwrap();
        assert!(funding.settled);
        assert_eq!(funding.allowance_consumptions.len(), 1);
        assert_eq!(funding.allowance_consumptions[0].quantity, 0);
        assert_eq!(funding.grant_consumptions.len(), 1);
        assert_eq!(funding.grant_consumptions[0].amount, Credits::ZERO);
        let period = self
            .db
            .collection::<crate::models::usage_allowance_period::UsageAllowancePeriod>(
                "usage_allowance_periods",
            )
            .find_one(doc! {})
            .await
            .unwrap()
            .unwrap();
        assert_eq!((period.reserved_quantity, period.consumed_quantity), (0, 0));
        assert!(period.active_settlement.is_none());
        let grant = self
            .db
            .collection::<crate::models::credit_grant::CreditGrant>("credit_grants")
            .find_one(doc! {"_id":grant_id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(grant.remaining, Credits::from_whole(1));
        assert_eq!(grant.reserved, Credits::ZERO);
        assert_eq!(grant.service_limits[0].reserved, Credits::ZERO);
        assert_eq!(grant.service_limits[0].consumed, Credits::ZERO);
        assert!(grant.active_settlement.is_none());
        let wallet = wallet(&self.db, &self.owner).await;
        assert_eq!(wallet.reserved_credits, Credits::ZERO);
        assert_eq!(wallet.pending_lago_debits, Credits::ZERO);
        assert_eq!(wallet.balance_credits, Credits::from_whole(100));
    }
}

#[tokio::test]
async fn hold_release_crash_matrix_returns_exact_mixed_holds_once() {
    for phase in [
        Phase::AfterMeter,
        Phase::AfterWallet,
        Phase::AfterCommit,
        Phase::AllowanceLocked,
        Phase::AllowanceApplied,
        Phase::AllowanceRecorded,
        Phase::GrantLocked,
        Phase::GrantApplied,
        Phase::GrantRecorded,
    ] {
        let f = Fixture::new().await;
        let (ctx, grant_id) = f.mixed_request().await;
        let metered = f.service.open(&ctx).await.unwrap();
        let row = usage_row(&f.db, &ctx.billing_request_id).await;
        assert_eq!(row.reserved_credits, "1.09".parse().unwrap());
        let funding = row.funding.unwrap();
        assert_eq!(funding.allowance_reservations[0].quantity, 1);
        assert_eq!(funding.grant_reservations[0].amount, Credits::from_whole(1));
        f.accounts_clean().await;
        let ledger_count =
            f.db.collection::<bson::Document>("billing_ledger")
                .count_documents(doc! {})
                .await
                .unwrap();
        let gate = Gate::new(phase);
        let mut release = Box::pin(GATE.scope(gate.clone(), f.service.fail(&metered, "crash")));
        tokio::select! {
            result = &mut release => panic!("release missed {phase:?}: {result:?}"),
            () = gate.entered() => {},
        }
        let visible = usage_row(&f.db, &ctx.billing_request_id).await;
        let before_commit = matches!(phase, Phase::AfterMeter | Phase::AfterWallet);
        assert_eq!(visible.released, !before_commit, "{phase:?}");
        assert_eq!(
            wallet(&f.db, &f.owner).await.reserved_credits,
            if before_commit {
                "1.09".parse().unwrap()
            } else {
                Credits::ZERO
            },
            "{phase:?}"
        );
        let deferred = f.accounts_clean().await;
        assert_eq!(
            deferred,
            matches!(
                phase,
                Phase::GrantLocked | Phase::GrantApplied | Phase::GrantRecorded
            )
        );
        f.holds_clean().await;
        drop(release);
        f.recover().await;
        assert!(!f.accounts_clean().await);
        f.assert_mixed_released(&ctx.billing_request_id, &grant_id)
            .await;
        f.recover().await;
        f.service.fail(&metered, "replay").await.unwrap();
        f.accounts_clean().await;
        f.assert_mixed_released(&ctx.billing_request_id, &grant_id)
            .await;
        assert_eq!(
            f.db.collection::<bson::Document>("billing_ledger")
                .count_documents(doc! {})
                .await
                .unwrap(),
            ledger_count
        );
        println!(
            "HOLD_RELEASE_MATRIX {phase:?}: holds=0, replay=unchanged, accounts=Ok, ledger=unchanged"
        );
        f.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn hold_release_stale_caller_cannot_subtract_successor_hold() {
    let f = Fixture::new().await;
    let ctx = f.request();
    let metered = f.service.open(&ctx).await.unwrap();
    let gate = Gate::new(Phase::BeforeTransaction);
    let mut stale = Box::pin(GATE.scope(gate.clone(), f.service.fail(&metered, "stale")));
    tokio::select! {
        result = &mut stale => panic!("missed stale caller barrier: {result:?}"),
        () = gate.entered() => {},
    }
    assert_eq!(
        reservation::abandon_stale_unforwarded(&f.db, Utc::now() + Duration::hours(1))
            .await
            .unwrap(),
        1
    );
    let next = f.service.open(&f.request()).await.unwrap();
    gate.resume.notify_one();
    stale.await.unwrap();
    f.service.fail(&metered, "replay").await.unwrap();
    assert_eq!(
        wallet(&f.db, &f.owner).await.reserved_credits,
        "1.09".parse().unwrap()
    );
    f.accounts_clean().await;
    let (left, right) = tokio::join!(
        f.service.fail(&next, "left"),
        f.service.fail(&next, "right")
    );
    left.unwrap();
    right.unwrap();
    f.recover().await;
    assert_eq!(
        wallet(&f.db, &f.owner).await.reserved_credits,
        Credits::ZERO
    );
    f.accounts_clean().await;
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn hold_release_failed_wallet_update_aborts_meter_transition() {
    let f = Fixture::new().await;
    let ctx = f.request();
    let metered = f.service.open(&ctx).await.unwrap();
    // Fault injection: make the second statement's exact-hold predicate fail.
    f.db.collection::<bson::Document>(BILLING_WALLET)
        .update_one(
            doc! {"owner_id":&f.owner},
            doc! {"$set":{"reserved_credits":Credits::ZERO}},
        )
        .await
        .unwrap();
    assert!(f.service.fail(&metered, "fault").await.is_err());
    let row = usage_row(&f.db, &ctx.billing_request_id).await;
    assert!(!row.released);
    assert_eq!(row.status, UsageStatus::Reserved);
    f.accounts_clean().await;
    f.db.collection::<bson::Document>(BILLING_WALLET)
        .update_one(
            doc! {"owner_id":&f.owner},
            doc! {"$set":{"reserved_credits":row.reserved_credits}},
        )
        .await
        .unwrap();
    f.recover().await;
    f.accounts_clean().await;
    assert_eq!(
        wallet(&f.db, &f.owner).await.reserved_credits,
        Credits::ZERO
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn hold_release_legacy_wallet_only_row_recovers_before_and_after_commit() {
    for phase in [Phase::AfterMeter, Phase::AfterWallet, Phase::AfterCommit] {
        let f = Fixture::new().await;
        let ctx = f.request();
        let metered = f.service.open(&ctx).await.unwrap();
        f.db.collection::<bson::Document>(USAGE_METER)
            .update_one(
                doc! {"billing_request_id":&ctx.billing_request_id},
                doc! {"$unset":{"funding":""}},
            )
            .await
            .unwrap();
        let gate = Gate::new(phase);
        let mut release = Box::pin(GATE.scope(gate.clone(), f.service.fail(&metered, "legacy")));
        tokio::select! {
            result = &mut release => panic!("missed {phase:?}: {result:?}"),
            () = gate.entered() => {},
        }
        f.accounts_clean().await;
        f.holds_clean().await;
        drop(release);
        f.recover().await;
        f.recover().await;
        assert_eq!(
            wallet(&f.db, &f.owner).await.reserved_credits,
            Credits::ZERO
        );
        assert!(usage_row(&f.db, &ctx.billing_request_id).await.released);
        f.accounts_clean().await;
        println!("HOLD_RELEASE_LEGACY {phase:?}: holds=0, accounts=Ok");
        f.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn hold_conservation_historical_release_leak_is_detected() {
    let f = Fixture::new().await;
    let ctx = f.request();
    f.service.open(&ctx).await.unwrap();
    // The old binary committed this CAS before returning the wallet hold.
    f.db.collection::<bson::Document>(USAGE_METER)
        .update_one(
            doc! {"billing_request_id": &ctx.billing_request_id, "released": false},
            doc! {"$set": {"released": true, "status": "failed"}},
        )
        .await
        .unwrap();
    f.recover().await;
    f.accounts_clean().await;
    assert!(
        crate::services::billing::startup_diagnostics(&f.db)
            .await
            .unwrap()
            .iter()
            .all(|d| d.code != "billing_hold_conservation")
    );
    f.commands.lock().unwrap().clear();
    f.accounts_clean().await;
    let repair_commands = std::mem::take(&mut *f.commands.lock().unwrap());
    println!("HOLD_CONSERVATION_REPAIR_COMMANDS {repair_commands:?}");
    let diagnostics = crate::services::billing::startup_diagnostics(&f.db)
        .await
        .unwrap();
    let detected = diagnostics
        .iter()
        .any(|d| d.code == "billing_hold_conservation");
    assert_eq!(
        wallet(&f.db, &f.owner).await.reserved_credits,
        Credits::ZERO
    );
    let audit =
        f.db.collection::<bson::Document>("audit_log")
            .find_one(doc! {"event_type":"billing_wallet_hold_excess_repaired"})
            .await
            .unwrap();
    assert!(audit.is_some(), "repair must have its atomic audit event");
    let verified = crate::services::audit_chain_service::verify_chain(
        &f.db,
        crate::services::audit_service::audit_chain_hmac_key().unwrap(),
        None,
        None,
        Some(100),
    )
    .await
    .unwrap();
    assert_eq!(
        verified.status,
        crate::services::audit_chain_service::AuditChainStatus::Ok
    );
    f.holds_clean().await;
    assert_eq!(
        f.db.collection::<bson::Document>("audit_log")
            .count_documents(doc! {"event_type":"billing_wallet_hold_excess_repaired"})
            .await
            .unwrap(),
        1
    );
    f.db.drop().await.unwrap();
    assert!(
        !detected,
        "repair must clear its diagnostic in the same transaction"
    );
}

#[tokio::test]
async fn hold_conservation_settlement_lock_handoffs_are_exact() {
    for phase in [
        Phase::WalletLocked,
        Phase::WalletApplied,
        Phase::WalletReleased,
        Phase::AllowanceLocked,
        Phase::AllowanceApplied,
        Phase::AllowanceRecorded,
        Phase::GrantLocked,
        Phase::GrantApplied,
        Phase::GrantRecorded,
    ] {
        let f = Fixture::new().await;
        let (ctx, _) = f.mixed_request().await;
        let metered = f.service.open(&ctx).await.unwrap();
        f.service.mark_forwarded(&metered).await.unwrap();
        f.holds_clean().await;
        let gate = Gate::new(phase);
        let usage = PlatformUsage {
            images: 3,
            ..PlatformUsage::single_request(23)
        };
        let mut settle =
            Box::pin(GATE.scope(gate.clone(), f.service.settle(&metered, usage, None, None)));
        tokio::select! {
            result = &mut settle => panic!("settlement missed {phase:?}: {result:?}"),
            () = gate.entered() => {},
        }
        f.holds_clean().await;
        let diagnostics = crate::services::billing::startup_diagnostics(&f.db)
            .await
            .unwrap();
        let backlog = diagnostics
            .iter()
            .find(|d| d.code == "billing_hold_backlog");
        let detail = backlog
            .and_then(|d| d.technical_details.as_deref())
            .unwrap_or("");
        assert!(!detail.contains("complete=false"), "{phase:?}: {detail}");
        for (kind, collection, field, unit) in [
            ("wallet", BILLING_WALLET, "reserved_credits", "credits"),
            ("grant", "credit_grants", "reserved", "credits"),
            (
                "allowance",
                "usage_allowance_periods",
                "reserved_quantity",
                "images",
            ),
        ] {
            let resource =
                f.db.collection::<bson::Document>(collection)
                    .find_one(doc! {})
                    .await
                    .unwrap()
                    .unwrap();
            let counter = if kind == "allowance" {
                Credits::from_whole(resource.get_i64(field).unwrap())
            } else {
                Credits::from_bson(
                    resource.get(field).unwrap().clone(),
                    crate::models::credits::SCALE,
                )
                .unwrap()
            };
            let prefix = format!("{kind} total unit={unit} ");
            if let Some(line) = detail.lines().find(|line| line.starts_with(&prefix)) {
                assert!(
                    line.contains(&format!(" held={counter} ")),
                    "{phase:?}: {line}; stored={counter}"
                );
            } else {
                assert_eq!(
                    counter,
                    Credits::ZERO,
                    "{phase:?}: missing {kind}: {detail}"
                );
            }
        }
        drop(settle);
        f.recover().await;
        f.holds_clean().await;
        println!("HOLD_CONSERVATION transient={phase:?}: no mismatch, accounts=Ok");
        f.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn hold_conservation_deficit_is_reported_but_never_changed() {
    let f = Fixture::new().await;
    f.service.open(&f.request()).await.unwrap();
    f.db.collection::<bson::Document>(BILLING_WALLET)
        .update_one(
            doc! {"owner_id": &f.owner},
            doc! {"$set":{"reserved_credits": Credits::ZERO}},
        )
        .await
        .unwrap();
    f.accounts_clean().await;
    f.accounts_clean().await;
    let diagnostics = crate::services::billing::startup_diagnostics(&f.db)
        .await
        .unwrap();
    assert!(
        diagnostics
            .iter()
            .any(|d| d.code == "billing_hold_conservation")
    );
    assert_eq!(
        wallet(&f.db, &f.owner).await.reserved_credits,
        Credits::ZERO
    );
    f.db.collection::<bson::Document>(BILLING_WALLET)
        .update_one(
            doc! {"owner_id": &f.owner},
            doc! {"$set":{"reserved_credits": "1.09".parse::<Credits>().unwrap()}},
        )
        .await
        .unwrap();
    f.holds_clean().await;
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn hold_conservation_changed_wallet_restarts_confirmation() {
    let f = Fixture::new().await;
    let ctx = f.request();
    f.service.open(&ctx).await.unwrap();
    f.db.collection::<bson::Document>(USAGE_METER)
        .update_one(
            doc! {"billing_request_id": &ctx.billing_request_id},
            doc! {"$set":{"released":true,"status":"failed"}},
        )
        .await
        .unwrap();
    f.accounts_clean().await;
    let next = f.service.open(&f.request()).await.unwrap();
    f.accounts_clean().await;
    assert!(
        crate::services::billing::startup_diagnostics(&f.db)
            .await
            .unwrap()
            .iter()
            .all(|d| d.code != "billing_hold_conservation")
    );
    assert_eq!(
        wallet(&f.db, &f.owner).await.reserved_credits,
        "2.18".parse().unwrap()
    );
    f.service.fail(&next, "done").await.unwrap();
    f.accounts_clean().await;
    assert!(
        crate::services::billing::startup_diagnostics(&f.db)
            .await
            .unwrap()
            .iter()
            .all(|d| d.code != "billing_hold_conservation")
    );
    f.db.drop().await.unwrap();
}

async fn leak_old_release(f: &Fixture) {
    let ctx = f.request();
    f.service.open(&ctx).await.unwrap();
    f.db.collection::<bson::Document>(USAGE_METER)
        .update_one(
            doc! {"billing_request_id": &ctx.billing_request_id},
            doc! {"$set":{"released":true,"status":"failed"}},
        )
        .await
        .unwrap();
}

async fn repair_audits(f: &Fixture) -> u64 {
    f.db.collection::<bson::Document>("audit_log")
        .count_documents(doc! {"event_type":"billing_wallet_hold_excess_repaired"})
        .await
        .unwrap()
}

#[tokio::test]
async fn hold_conservation_repair_loses_to_admission_and_real_release() {
    for admission in [true, false] {
        let f = Fixture::new().await;
        leak_old_release(&f).await;
        let live = f.service.open(&f.request()).await.unwrap();
        f.accounts_clean().await;
        let gate = Gate::new(Phase::BeforeHoldRepair);
        let mut repair = Box::pin(GATE.scope(
            gate.clone(),
            account_reconciliation::hold_pass_for_test(&f.db),
        ));
        tokio::select! {
            result = &mut repair => panic!("repair missed barrier: {result:?}"),
            () = gate.entered() => {},
        }
        if admission {
            f.service.open(&f.request()).await.unwrap();
        } else {
            f.service.fail(&live, "real release").await.unwrap();
        }
        gate.resume.notify_one();
        repair.await.unwrap();
        assert_eq!(repair_audits(&f).await, 0, "changed snapshot cannot repair");
        assert_eq!(
            wallet(&f.db, &f.owner).await.reserved_credits,
            if admission { "3.27" } else { "1.09" }.parse().unwrap()
        );
        f.accounts_clean().await;
        assert_eq!(
            repair_audits(&f).await,
            0,
            "changed observation needs confirmation"
        );
        f.accounts_clean().await;
        assert_eq!(repair_audits(&f).await, 1);
        assert_eq!(
            wallet(&f.db, &f.owner).await.reserved_credits,
            if admission { "2.18" } else { "0" }.parse().unwrap()
        );
        f.holds_clean().await;
        f.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn hold_conservation_old_paused_release_can_create_a_deficit_after_repair() {
    let f = Fixture::new().await;
    leak_old_release(&f).await;
    f.service.open(&f.request()).await.unwrap();
    f.service.open(&f.request()).await.unwrap();
    f.accounts_clean().await;
    f.accounts_clean().await;
    assert_eq!(repair_audits(&f).await, 1);
    assert_eq!(
        wallet(&f.db, &f.owner).await.reserved_credits,
        "2.18".parse().unwrap()
    );
    // This is exactly the old standalone release predicate. No finite wait can
    // revoke the amount already captured in a paused old process's memory.
    let held = "1.09".parse::<Credits>().unwrap();
    let result = f
        .db
        .collection::<bson::Document>(BILLING_WALLET)
        .update_one(
            doc! {"owner_id":&f.owner,"$expr":{"$gte":["$reserved_credits",held]}},
            doc! {"$inc":{"reserved_credits":-held},"$set":{"updated_at":bson::DateTime::now()}},
        )
        .await
        .unwrap();
    assert_eq!(result.modified_count, 1);
    f.accounts_clean().await;
    f.accounts_clean().await;
    assert_eq!(wallet(&f.db, &f.owner).await.reserved_credits, held);
    assert_eq!(repair_audits(&f).await, 1, "never repair a deficit");
    assert!(
        crate::services::billing::startup_diagnostics(&f.db)
            .await
            .unwrap()
            .iter()
            .any(|d| d.code == "billing_hold_conservation")
    );
    println!("HOLD_CONSERVATION mixed old writer: deficit=1.09, detected=true, accounts=Ok");
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn hold_conservation_manual_checks_do_not_confirm_or_repair() {
    let f = Fixture::new().await;
    leak_old_release(&f).await;
    for _ in 0..3 {
        account_reconciliation::run_once(&f.db).await.unwrap();
    }
    assert_eq!(repair_audits(&f).await, 0);
    assert_eq!(
        f.db.collection::<bson::Document>("billing_hold_observations")
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    f.accounts_clean().await;
    account_reconciliation::run_once(&f.db).await.unwrap();
    assert_eq!(repair_audits(&f).await, 0);
    f.accounts_clean().await;
    assert_eq!(repair_audits(&f).await, 1);
    f.holds_clean().await;
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn hold_conservation_query_plans_and_command_counts() {
    let f = Fixture::new().await;
    let (ctx, grant_id) = f.mixed_request().await;
    f.service.open(&ctx).await.unwrap();
    let current = wallet(&f.db, &f.owner).await;
    let row = usage_row(&f.db, &ctx.billing_request_id).await;
    let period_id = &row.funding.as_ref().unwrap().allowance_reservations[0].period_id;
    let queries = [
        doc! {"wallet_id":&current.id,"released":false,"reserved_credits":{"$ne":Credits::ZERO}},
        doc! {"funding.grant_reservations.grant_id":&grant_id,"funding.settled":{"$ne":true}},
        doc! {"funding.allowance_reservations.period_id":period_id,"funding.settled":{"$ne":true}},
    ];
    for (filter, hint) in queries.into_iter().zip([
        doc! {"wallet_id":1,"released":1,"_id":1},
        doc! {"funding.grant_reservations.grant_id":1,"funding.settled":1,"_id":1},
        doc! {"funding.allowance_reservations.period_id":1,"funding.settled":1,"_id":1},
    ]) {
        let explain = f.db.run_command(doc! {
            "explain":{"find":USAGE_METER,"filter": &filter,"hint":hint,"sort":{"_id":1},"limit":10001_i64},
            "verbosity":"executionStats",
        }).await.unwrap();
        let plan = explain
            .get_document("queryPlanner")
            .unwrap()
            .get_document("winningPlan")
            .unwrap();
        let plan_text = format!("{plan:?}");
        assert!(plan_text.contains("IXSCAN"), "{explain:?}");
        assert!(!plan_text.contains("COLLSCAN"), "{explain:?}");
        println!(
            "HOLD_CONSERVATION_PLAN filter={filter:?}, plan={plan:?}, stats={:?}",
            explain.get_document("executionStats").unwrap()
        );
    }
    f.commands.lock().unwrap().clear();
    account_reconciliation::hold_pass_immediately_for_test(&f.db)
        .await
        .unwrap();
    let commands = std::mem::take(&mut *f.commands.lock().unwrap());
    assert_eq!(
        commands.iter().filter(|c| *c == "find:usage_meter").count(),
        3
    );
    assert_eq!(
        commands
            .iter()
            .filter(|c| *c == "find:billing_hold_observations")
            .count(),
        3
    );
    assert!(
        !commands
            .iter()
            .any(|c| c == "update:billing_hold_observations")
    );
    println!("HOLD_CONSERVATION_COMMANDS one wallet/grant/allowance: {commands:?}");
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn hold_conservation_expiry_lock_never_authorizes_repair() {
    let f = Fixture::new().await;
    leak_old_release(&f).await;
    let now = bson::DateTime::now();
    f.db.collection::<bson::Document>(BILLING_WALLET)
        .update_one(
            doc! {"owner_id":&f.owner},
            doc! {"$set":{"active_topup_expiry":{
                "operation_id":"expiry", "processing_token":"holder", "lease_until":now,
                "amount": Credits::from_whole(1), "items":[], "created_at":now,"updated_at":now,
            }}},
        )
        .await
        .unwrap();
    f.accounts_clean().await;
    f.accounts_clean().await;
    assert_eq!(repair_audits(&f).await, 0);
    assert_eq!(
        wallet(&f.db, &f.owner).await.reserved_credits,
        "1.09".parse().unwrap()
    );
    f.db.collection::<bson::Document>(BILLING_WALLET)
        .update_one(
            doc! {"owner_id":&f.owner},
            doc! {"$unset":{"active_topup_expiry":""}},
        )
        .await
        .unwrap();
    f.accounts_clean().await;
    assert_eq!(
        repair_audits(&f).await,
        0,
        "lock change resets the observation"
    );
    f.accounts_clean().await;
    assert_eq!(repair_audits(&f).await, 1);
    f.holds_clean().await;
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn hold_conservation_benefit_counter_excess_is_repaired() {
    let f = Fixture::new().await;
    let (ctx, grant_id) = f.mixed_request().await;
    f.service.open(&ctx).await.unwrap();
    f.db.collection::<bson::Document>("credit_grants")
        .update_one(
            doc! {"_id":&grant_id},
            doc! {"$inc":{"service_limits.0.reserved":Credits::from_whole(1)}},
        )
        .await
        .unwrap();
    f.db.collection::<bson::Document>("usage_allowance_periods")
        .update_one(doc! {}, doc! {"$inc":{"reserved_quantity":1_i64}})
        .await
        .unwrap();
    f.accounts_clean().await;
    f.accounts_clean().await;
    let diagnostics = crate::services::billing::startup_diagnostics(&f.db)
        .await
        .unwrap();
    assert!(
        diagnostics
            .iter()
            .all(|d| d.code != "billing_hold_conservation")
    );
    assert_eq!(benefit_repair::audits(&f).await.len(), 2);
    assert_eq!(repair_audits(&f).await, 0);
    assert_eq!(
        f.db.collection::<bson::Document>("usage_allowance_periods")
            .find_one(doc! {})
            .await
            .unwrap()
            .unwrap()
            .get_i64("reserved_quantity")
            .unwrap(),
        1
    );
    f.holds_clean().await;
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn hold_conservation_repair_crash_keeps_wallet_and_audit_atomic() {
    for phase in [Phase::AfterHoldRepairWallet, Phase::AfterHoldRepairAudit] {
        let f = Fixture::new().await;
        leak_old_release(&f).await;
        f.accounts_clean().await;
        let gate = Gate::new(phase);
        let mut repair = Box::pin(GATE.scope(
            gate.clone(),
            account_reconciliation::hold_pass_for_test(&f.db),
        ));
        tokio::select! {
            result = &mut repair => panic!("repair missed {phase:?}: {result:?}"),
            () = gate.entered() => {},
        }
        assert_eq!(
            wallet(&f.db, &f.owner).await.reserved_credits,
            "1.09".parse().unwrap()
        );
        assert_eq!(repair_audits(&f).await, 0);
        account_reconciliation::run_once(&f.db).await.unwrap();
        drop(repair);
        // A dead process's account lease expires; do not sleep five minutes.
        f.db.collection::<bson::Document>("billing_account_verify_lease")
            .update_one(
                doc! {"_id":"billing_accounts"},
                doc! {"$set":{"lease_until":bson::DateTime::from_millis(0)}},
            )
            .await
            .unwrap();
        f.accounts_clean().await;
        assert_eq!(repair_audits(&f).await, 1);
        assert_eq!(
            wallet(&f.db, &f.owner).await.reserved_credits,
            Credits::ZERO
        );
        f.holds_clean().await;
        f.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn hold_conservation_row_bound_is_incomplete_and_never_repairs() {
    let f = Fixture::new().await;
    f.service.open(&f.request()).await.unwrap();
    let current = wallet(&f.db, &f.owner).await;
    let rows: Vec<_> = (0..10_000).map(|i| {
        let id = format!("bound-{i}");
        doc! {"_id":&id,"transaction_id":id,"billing_owner_id":&f.owner,
            "wallet_id":&current.id,"reserved_credits":"0.000000000001".parse::<Credits>().unwrap(),"released":false}
    }).collect();
    f.db.collection::<bson::Document>(USAGE_METER)
        .insert_many(rows)
        .await
        .unwrap();
    f.db.collection::<bson::Document>(BILLING_WALLET)
        .update_one(
            doc! {"_id":&current.id},
            doc! {"$set":{"reserved_credits":"1.09000001".parse::<Credits>().unwrap()}},
        )
        .await
        .unwrap();
    assert!(!f.accounts_clean().await);
    assert!(!f.accounts_clean().await);
    let diagnostics = crate::services::billing::startup_diagnostics(&f.db)
        .await
        .unwrap();
    assert!(
        diagnostics
            .iter()
            .any(|d| d.code == "billing_hold_conservation_incomplete")
    );
    assert!(
        diagnostics
            .iter()
            .all(|d| d.code != "billing_hold_conservation")
    );
    assert_eq!(repair_audits(&f).await, 0);
    assert_eq!(
        wallet(&f.db, &f.owner).await.reserved_credits,
        "1.09000001".parse().unwrap()
    );
    f.db.collection::<bson::Document>(USAGE_METER)
        .delete_many(doc! {"_id":{"$regex":"^bound-"}})
        .await
        .unwrap();
    f.db.collection::<bson::Document>(BILLING_WALLET)
        .update_one(
            doc! {"_id":&current.id},
            doc! {"$set":{"reserved_credits":"1.09".parse::<Credits>().unwrap()}},
        )
        .await
        .unwrap();
    f.holds_clean().await;
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn hold_conservation_zero_wallet_hold_and_unreserved_overage() {
    for phase in [
        Phase::WalletLocked,
        Phase::WalletApplied,
        Phase::WalletReleased,
    ] {
        let f = Fixture::new().await;
        let (mut ctx, _) = f.mixed_request().await;
        ctx.requested_images = 2; // Fully covered by one allowance and one grant.
        let metered = f.service.open(&ctx).await.unwrap();
        assert_eq!(
            wallet(&f.db, &f.owner).await.reserved_credits,
            Credits::ZERO
        );
        f.holds_clean().await;
        f.service.mark_forwarded(&metered).await.unwrap();
        let gate = Gate::new(phase);
        let usage = PlatformUsage {
            images: 3,
            ..PlatformUsage::single_request(23)
        };
        let mut settle =
            Box::pin(GATE.scope(gate.clone(), f.service.settle(&metered, usage, None, None)));
        tokio::select! {
            result = &mut settle => panic!("overage missed {phase:?}: {result:?}"),
            () = gate.entered() => {},
        }
        f.holds_clean().await;
        drop(settle);
        f.recover().await;
        f.holds_clean().await;
        f.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn hold_conservation_refresh_bookkeeping_does_not_starve_repair() {
    for changed_balance in [false, true] {
        let f = Fixture::new().await;
        leak_old_release(&f).await;
        f.accounts_clean().await;
        let before = wallet(&f.db, &f.owner).await;
        let balance = Credits::from_whole(if changed_balance { 101 } else { 100 });
        assert!(
            crate::services::billing::webhook::refresh_wallet_balance(&f.db, &before, balance,)
                .await
                .unwrap()
                .is_some()
        );
        let after = wallet(&f.db, &f.owner).await;
        assert!(after.snapshot_generation > before.snapshot_generation);
        f.db.collection::<bson::Document>(BILLING_WALLET)
            .update_one(
                doc! {"_id":&before.id},
                doc! {"$set":{"topup_expiry_checked_at":bson::DateTime::now()}},
            )
            .await
            .unwrap();
        f.accounts_clean().await;
        if changed_balance {
            assert_eq!(
                repair_audits(&f).await,
                0,
                "financial movement resets confirmation"
            );
            assert!(
                crate::services::billing::startup_diagnostics(&f.db)
                    .await
                    .unwrap()
                    .iter()
                    .any(|diagnostic| diagnostic.code == "billing_hold_conservation"),
                "financial movement must not hide an unchanged hold mismatch"
            );
            f.accounts_clean().await;
        }
        assert_eq!(repair_audits(&f).await, 1);
        assert_eq!(
            wallet(&f.db, &f.owner).await.reserved_credits,
            Credits::ZERO
        );
        f.holds_clean().await;
        f.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn hold_conservation_repair_retries_concurrent_audit_append() {
    let f = Fixture::new().await;
    leak_old_release(&f).await;
    f.accounts_clean().await;
    let gate = Gate::new(Phase::BeforeHoldRepair);
    let mut repair = Box::pin(GATE.scope(
        gate.clone(),
        account_reconciliation::hold_pass_for_test(&f.db),
    ));
    tokio::select! {
        result = &mut repair => panic!("repair missed audit race: {result:?}"),
        () = gate.entered() => {},
    }
    crate::services::transaction_service::run_db_only(&f.db, &f.db, |session, db| {
        Box::pin(async move {
            crate::services::audit_service::log_system_event_in_session(
                db,
                session,
                crate::services::audit_service::audit_chain_hmac_key().unwrap(),
                "hold_test_concurrent_audit",
                serde_json::json!({}),
            )
            .await
        })
    })
    .await
    .unwrap();
    gate.resume.notify_one();
    repair.await.unwrap();
    assert_eq!(repair_audits(&f).await, 1);
    let verified = crate::services::audit_chain_service::verify_chain(
        &f.db,
        crate::services::audit_service::audit_chain_hmac_key().unwrap(),
        None,
        None,
        Some(100),
    )
    .await
    .unwrap();
    assert_eq!(
        verified.status,
        crate::services::audit_chain_service::AuditChainStatus::Ok
    );
    assert_eq!(verified.checked_count, 2);
    f.holds_clean().await;
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn hold_conservation_rapid_passes_wait_for_database_time() {
    let f = Fixture::new().await;
    leak_old_release(&f).await;
    account_reconciliation::hold_pass_immediately_for_test(&f.db)
        .await
        .unwrap();
    let observations =
        f.db.collection::<bson::Document>("billing_hold_observations");
    let first = observations.find_one(doc! {}).await.unwrap().unwrap();
    for _ in 0..2 {
        account_reconciliation::hold_pass_immediately_for_test(&f.db)
            .await
            .unwrap();
        assert_eq!(repair_audits(&f).await, 0);
        let current = observations.find_one(doc! {}).await.unwrap().unwrap();
        assert_eq!(
            current.get_datetime("stable_since").unwrap(),
            first.get_datetime("stable_since").unwrap()
        );
        assert!(!current.get_bool("persistent").unwrap());
    }
    observations
        .update_many(
            doc! {},
            vec![doc! {"$set":{
                "stable_since":{"$subtract":["$$NOW",100_000_i64]},
            }}],
        )
        .await
        .unwrap();
    account_reconciliation::hold_pass_immediately_for_test(&f.db)
        .await
        .unwrap();
    assert_eq!(repair_audits(&f).await, 0);
    observations
        .update_many(
            doc! {},
            vec![doc! {"$set":{
                "stable_since":{"$subtract":["$$NOW",301_000_i64]},
            }}],
        )
        .await
        .unwrap();
    account_reconciliation::hold_pass_immediately_for_test(&f.db)
        .await
        .unwrap();
    assert_eq!(repair_audits(&f).await, 1);
    f.holds_clean().await;
    f.db.drop().await.unwrap();
}

// Reuse the exact existing money-boundary hooks for Realtime recovery tests.
pub(crate) async fn crash_money_at<T>(phase: Phase, future: impl std::future::Future<Output = T>) {
    let gate = Gate::new(phase);
    let mut running = Box::pin(GATE.scope(gate.clone(), future));
    tokio::select! {
        _=&mut running=>panic!("money operation missed {phase:?}"),
        ()=gate.entered()=>{},
    }
    drop(running);
}

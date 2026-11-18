//! Provider recovery uses the original selections and real MongoDB boundaries.
use super::*;
use crate::{
    errors::AppError,
    models::{
        downstream_service::{DownstreamService, test_helpers::dummy_service},
        service_billing::{PricingSyncStatus, ServiceBilling, ServicePlatformPricing},
    },
    services::billing::{self, lago_client::*},
};
use mongodb::event::{EventHandler, command::CommandEvent};
use serde_json::json;
use std::sync::Mutex;

#[path = "tax_refresh_budget_tests.rs"]
mod tax_refresh_budget_tests;

#[derive(Default)]
struct Provider {
    calls: Mutex<Vec<String>>,
    failure: Mutex<Option<&'static str>>,
    rates: Mutex<Vec<PlanRate>>,
    on_read: Mutex<Option<(mongodb::Database, Step)>>,
}
impl Provider {
    fn call(&self, method: &str, key: &str) -> AppResult<()> {
        self.calls.lock().unwrap().push(format!("{method}:{key}"));
        match *self.failure.lock().unwrap() {
            Some("outage") => Err(AppError::BillingProviderUnavailable("test outage".into())),
            Some("first") if key.starts_with("a-") => {
                Err(AppError::BillingProviderUnavailable("test outage".into()))
            }
            Some("not_yet") => Err(AppError::Conflict("provider readback pending".into())),
            _ => Ok(()),
        }
    }
    async fn race(&self, step: Step) {
        let action = self.on_read.lock().unwrap().take();
        let Some((db, target)) = action else { return };
        assert_eq!(step, target);
        match step {
            Step::Taxes => billing::taxes::invalidate_plan(&db, "standard")
                .await
                .unwrap(),
            Step::Rates => billing::rate_cache::replace(
                &db,
                &BillingRateCache {
                    id: BillingRateCache::cache_id("winner", None),
                    lago_metric_code: "winner".into(),
                    model: None,
                    credits_per_unit_micros: 7,
                    credits_per_unit_pico: Some(7_000_000),
                    synced_at: Utc::now(),
                    retired_at: None,
                },
            )
            .await
            .unwrap(),
            Step::Wallet => {
                db.collection::<Document>(BILLING_WALLET)
                    .update_one(
                        doc! {"_id":"z-good"},
                        doc! {"$inc":{"snapshot_generation":1_i64}},
                    )
                    .await
                    .unwrap();
            }
            _ => unreachable!(),
        }
    }
}
#[async_trait::async_trait]
impl LagoApi for Provider {
    async fn ensure_customer(&self, _: &OwnerProvisionInput) -> AppResult<String> {
        unreachable!()
    }
    async fn ensure_subscription(&self, _: &str, _: &str) -> AppResult<String> {
        unreachable!()
    }
    async fn record_event(&self, e: &LagoEvent) -> Result<LagoAck, LagoError> {
        if let Err(error) = self.call("event", &e.transaction_id) {
            let mut failure = LagoError::retry("test provider failure");
            if matches!(error, AppError::Conflict(_)) {
                failure.status = Some(reqwest::StatusCode::TOO_MANY_REQUESTS);
            }
            return Err(failure);
        }
        Ok(LagoAck {
            transaction_id: e.transaction_id.clone(),
        })
    }
    async fn record_events_batch(&self, _: &[LagoEvent]) -> Result<Vec<LagoAck>, LagoError> {
        unreachable!()
    }
    async fn current_usage(&self, c: &str, s: &str) -> AppResult<LagoUsage> {
        crate::services::billing::lago_client::require_subscription_id(s)?;
        self.call("usage", c)?;
        Ok(LagoUsage {
            customer_id: c.into(),
            subscription_id: s.into(),
            raw: json!({"amount_cents":0}),
        })
    }
    async fn wallet_balance(&self, c: &str) -> AppResult<i64> {
        self.call("balance", c)?;
        self.race(Step::Wallet).await;
        Ok(0)
    }
    async fn entitlements(&self, _: &str) -> AppResult<Vec<Entitlement>> {
        Ok(vec![])
    }
    async fn tax_config(&self, c: &str, _: &str) -> AppResult<LagoTaxConfig> {
        self.call("tax", c)?;
        tax_refresh_budget_tests::on_tax_read(c).await;
        self.race(Step::Taxes).await;
        Ok(billing::taxes::zero_config())
    }
    async fn sync_standard_charge(&self, _: &str, input: &ServicePriceSync) -> AppResult<()> {
        self.call("sync", &input.metric_code)?;
        if *self.failure.lock().unwrap() == Some("lag") {
            return Ok(());
        }
        let pico = billing::amounts::decimal_to_pico(&input.credits_per_unit).unwrap();
        self.rates.lock().unwrap().push(PlanRate {
            lago_metric_code: input.metric_code.clone(),
            credits_per_unit_micros: pico / 1_000_000,
            credits_per_unit_pico: Some(pico),
        });
        Ok(())
    }
    async fn plan_rates(&self, p: &str) -> AppResult<PlanRates> {
        self.call("rates", p)?;
        self.race(Step::Rates).await;
        Ok(PlanRates {
            rates: self.rates.lock().unwrap().clone(),
            rejected: vec![],
        })
    }
    async fn plan_charges(&self, plan: &str) -> AppResult<Vec<serde_json::Value>> {
        self.call("charges", plan)?;
        Ok(self.rates.lock().unwrap().iter().map(|r|json!({
            "billable_metric_code":r.lago_metric_code,"charge_model":"standard",
            "properties":{"amount":Credits::from_pico(i128::from(r.credits_per_unit_pico.unwrap())).unwrap().to_string()},
        })).collect())
    }
    async fn wallet_transactions(&self, _: &str) -> AppResult<Vec<LagoWalletTransaction>> {
        Ok(vec![])
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Step {
    Taxes,
    Prices,
    Machine,
    Push,
    Compare,
    Wallet,
    Rates,
}
impl Step {
    const ALL: [Self; 6] = [
        Self::Taxes,
        Self::Prices,
        Self::Machine,
        Self::Push,
        Self::Wallet,
        Self::Rates,
    ];
    fn name(self) -> &'static str {
        match self {
            Self::Taxes => "taxes_before_recovery",
            Self::Prices => "service_prices",
            Self::Machine => "machine_prices",
            Self::Push => "usage_push",
            Self::Compare => "usage_comparison",
            Self::Wallet => "wallet_balances",
            Self::Rates => "rate_cache",
        }
    }
    async fn run(self, f: &Fixture) -> AppResult<()> {
        let mut stats = ReconcileStats::default();
        match self {
            Self::Taxes => billing::taxes::reconcile(&f.db, f.provider.as_ref(), 300).await,
            Self::Prices => billing::pricing::retry_pending_service_prices(
                &f.db,
                f.provider.as_ref(),
                "standard",
            )
            .await
            .map(|_| ()),
            Self::Machine => f.billing.retry_machine_runtime_rates(4).await.map(|_| ()),
            Self::Push => {
                f.reconciler
                    .push_unacked(f.provider.as_ref(), &mut stats)
                    .await
            }
            Self::Compare => {
                let runtime = crate::services::coordination_service::cluster_lease_runtime();
                let token =
                    LeaseStore::acquire(&f.db, RECONCILE_LEASE, &runtime.holder, runtime.ttl)
                        .await?
                        .expect("fixture lease");
                let result = f
                    .reconciler
                    .compare_finalized_usage(f.provider.as_ref(), &mut stats, &token)
                    .await;
                LeaseStore::release(&f.db, &token).await?;
                result
            }
            Self::Wallet => {
                f.reconciler
                    .refresh_wallet_balances(f.provider.as_ref(), &mut stats)
                    .await
            }
            Self::Rates => {
                f.reconciler
                    .refresh_rate_cache(f.provider.as_ref(), &mut stats)
                    .await
            }
        }
    }
}
struct Fixture {
    db: mongodb::Database,
    provider: Arc<Provider>,
    billing: billing::BillingService,
    reconciler: BillingReconciler,
    commands: Arc<Mutex<Vec<String>>>,
}
impl Fixture {
    async fn new() -> Self {
        let commands = Arc::new(Mutex::new(vec![]));
        let seen = commands.clone();
        let db = crate::test_utils::connect_transaction_test_database_with_command_handler(
            "provider_step_isolation",
            EventHandler::callback(move |e| {
                if let CommandEvent::Started(e) = e {
                    seen.lock().unwrap().push(format!(
                        "{}:{}",
                        e.command_name,
                        e.command.get_str(&e.command_name).unwrap_or("")
                    ));
                }
            }),
        )
        .await;
        billing::ledger::init_billing_ledger_hmac_key(zeroize::Zeroizing::new(
            billing::ledger::TEST_BILLING_LEDGER_HMAC_KEY,
        ));
        let mut config = tests::billing_enabled_config();
        config.lago_plan_code = "standard".into();
        let config = Arc::new(config);
        let provider = Arc::new(Provider::default());
        let mut billing = billing::BillingService::new(db.clone(), config.clone());
        billing.lago = Some(provider.clone());
        let reconciler = BillingReconciler::new(db.clone(), Some(provider.clone()), config);
        Self {
            db,
            provider,
            billing,
            reconciler,
            commands,
        }
    }
    async fn wallet(&self, id: &str) {
        let now = bson::DateTime::now();
        self.db.collection::<Document>(BILLING_WALLET).insert_one(doc!{"_id":id,"owner_id":id,"lago_customer_id":id,"lago_subscription_id":format!("sub-{id}"),"plan_kind":"prepaid","collection_state":"good","balance_credits":Credits::from_whole(10),"balance_synced_at":bson::DateTime::from_millis(0),"created_at":now,"updated_at":now}).await.unwrap();
    }
    async fn healthy(&self, step: Step) -> String {
        match step {
            Step::Taxes | Step::Wallet => {
                self.wallet("z-good").await;
                "z-good".into()
            }
            Step::Prices => {
                let mut s = dummy_service();
                s.id = "z-good".into();
                s.slug = "z-good".into();
                s.billing = Some(ServiceBilling {
                    platform_pricing: Some(ServicePlatformPricing {
                        credits_per_unit: "1".into(),
                        lago_metric_code: "z-good".into(),
                        sync_status: PricingSyncStatus::Pending,
                        sync_error: None,
                    }),
                    ..Default::default()
                });
                self.db
                    .collection::<DownstreamService>("downstream_services")
                    .insert_one(s)
                    .await
                    .unwrap();
                "z-good".into()
            }
            Step::Machine => {
                self.billing
                    .stage_initial_machine_runtime_rate()
                    .await
                    .unwrap()
                    .id
            }
            Step::Push | Step::Compare => {
                let mut r = tests::finalized_row("z-good");
                r.id = "z-good".into();
                r.billing_owner_id = "z-good".into();
                self.db
                    .collection::<UsageMeterRow>(USAGE_METER)
                    .insert_one(r)
                    .await
                    .unwrap();
                "z-good".into()
            }
            Step::Rates => {
                self.provider.rates.lock().unwrap().push(PlanRate {
                    lago_metric_code: "z-good".into(),
                    credits_per_unit_micros: 1,
                    credits_per_unit_pico: Some(1_000_000),
                });
                "z-good".into()
            }
        }
    }
    async fn poison(&self, step: Step) {
        let old = bson::DateTime::from_millis(-1);
        let (collection, row) = match step {
            Step::Taxes | Step::Wallet => (
                BILLING_WALLET,
                doc! {"_id":"a-bad","lago_subscription_id":"sub-bad","balance_synced_at":old},
            ),
            Step::Prices => (
                "downstream_services",
                doc! {"_id":"a-bad","billing":{"platform_pricing":{"sync_status":"pending","credits_per_unit":"1"}}},
            ),
            Step::Machine => (
                "machine_runtime_rates",
                doc! {"_id":1_i64,"retired":false,"sync_status":"pending"},
            ),
            Step::Push => (
                USAGE_METER,
                doc! {"_id":"a-bad","status":"finalized","released":true,"wallet_id":"bad","quantity":1,"lago_acked":false,"updated_at":old},
            ),
            Step::Compare => (
                USAGE_METER,
                doc! {"_id":"a-bad","status":"finalized","released":true,"wallet_id":"bad","quantity":1,"billing_owner_id":1_i64,"lago_metric_code":"bad"},
            ),
            Step::Rates => {
                self.provider.rates.lock().unwrap().push(PlanRate {
                    lago_metric_code: "a-bad".into(),
                    credits_per_unit_micros: 1,
                    credits_per_unit_pico: Some(1_000_000),
                });
                self.db
                    .create_collection("billing_rate_cache")
                    .validator(doc! {"lago_metric_code":{"$ne":"a-bad"}})
                    .await
                    .unwrap();
                return;
            }
        };
        self.db
            .collection::<Document>(collection)
            .insert_one(row)
            .await
            .unwrap();
    }
    async fn healthy_done(&self, step: Step, id: &str) -> bool {
        let (col, filter) = match step {
            Step::Taxes => (
                "billing_tax_profiles",
                doc! {"owner_id":id,"fingerprint":{"$ne":""}},
            ),
            Step::Prices => (
                "downstream_services",
                doc! {"_id":id,"billing.platform_pricing.sync_status":"synced"},
            ),
            Step::Machine => (
                "machine_runtime_rates",
                doc! {"_id":id,"sync_status":"synced"},
            ),
            Step::Push => (USAGE_METER, doc! {"_id":id,"lago_acked":true}),
            Step::Compare => return self.provider.calls.lock().unwrap().is_empty(),
            Step::Wallet => (
                BILLING_WALLET,
                doc! {"_id":id,"balance_synced_at":{"$gt":bson::DateTime::from_millis(0)}},
            ),
            Step::Rates => ("billing_rate_cache", doc! {"lago_metric_code":id}),
        };
        self.db
            .collection::<Document>(col)
            .count_documents(filter)
            .await
            .unwrap()
            == 1
    }
    async fn status(&self, step: Step, result: AppResult<()>, failed: bool) {
        let mut pass = Pass::new(None);
        pass.record(step.name(), result);
        let runtime = ClusterLeaseRuntime::new(
            crate::models::coordination::CoordinationHolder {
                instance_id: "provider-status".into(),
                generation_id: uuid::Uuid::new_v4().to_string(),
            },
            std::time::Duration::from_secs(30),
            std::time::Duration::from_millis(50),
        );
        let token = runtime
            .acquire(&self.db, RECONCILE_LEASE)
            .await
            .unwrap()
            .unwrap();
        pass.release(&self.db, &token).await.unwrap();
        let ds = billing::reconcile_status::diagnostics(&self.db).await;
        assert_eq!(!ds.is_empty(), failed);
        if failed {
            assert!(ds[0].detail.contains("failing_units=1"), "{}", ds[0].detail);
        }
    }
}
async fn prove(step: Step) {
    let f = Fixture::new().await;
    f.poison(step).await;
    let id = f.healthy(step).await;
    let result = step.run(&f).await;
    assert!(
        f.healthy_done(step, &id).await,
        "{step:?}: bad unit blocked healthy work"
    );
    assert!(
        matches!(
            result,
            Err(AppError::BillingRecoveryFailed {
                failed_units: 1,
                ..
            })
        ),
        "{step:?}: {result:?}"
    );
    f.status(step, result, true).await;
    if step == Step::Machine {
        f.db.collection::<Document>("machine_runtime_rates")
            .update_one(
                doc! {"_id":1_i64},
                doc! {"$set":{"sync_claim_until":bson::DateTime::from_millis(0)}},
            )
            .await
            .unwrap();
    }
    let again = step.run(&f).await;
    assert!(
        matches!(again,Err(AppError::BillingRecoveryFailed{failed_units,..}) if failed_units >= 1),
        "{step:?}: poison not retried: {again:?}"
    );
    f.db.drop().await.unwrap();
}
macro_rules! regression {
    ($name:ident,$step:ident) => {
        #[tokio::test]
        async fn $name() {
            prove(Step::$step).await;
        }
    };
}
regression!(provider_step_tax_isolates_bad_wallet, Taxes);
regression!(provider_step_price_isolates_bad_service, Prices);
regression!(provider_step_machine_isolates_bad_claim, Machine);
regression!(provider_step_push_isolates_bad_row, Push);
#[tokio::test]
async fn provider_step_comparison_warms_without_reading_usage() {
    let f = Fixture::new().await;
    f.poison(Step::Compare).await;
    f.healthy(Step::Compare).await;
    f.commands.lock().unwrap().clear();
    Step::Compare.run(&f).await.unwrap();
    assert!(f.provider.calls.lock().unwrap().is_empty());
    assert_eq!(
        f.commands.lock().unwrap().as_slice(),
        [
            "findAndModify:coordination_leases",
            "aggregate:billing_usage_comparison_control",
            "find:security_rollouts",
            "find:security_replicas",
            "findAndModify:billing_usage_comparison_control",
            "find:billing_usage_comparison_control",
            "find:billing_wallet",
            "delete:coordination_leases",
        ],
        "coverage and discovery never read the old aggregate's poisoned meter row"
    );
    f.db.drop().await.unwrap();
}
regression!(provider_step_wallet_isolates_bad_wallet, Wallet);
regression!(provider_step_rate_isolates_failed_publication, Rates);
#[tokio::test]
async fn provider_step_clean_command_counts() {
    for step in Step::ALL.into_iter().chain([Step::Compare]) {
        let f = Fixture::new().await;
        f.commands.lock().unwrap().clear();
        step.run(&f).await.unwrap();
        let idle = f.commands.lock().unwrap().clone();
        let id = f.healthy(step).await;
        f.commands.lock().unwrap().clear();
        step.run(&f).await.unwrap();
        let healthy = f.commands.lock().unwrap().clone();
        let expected = match step {
            Step::Taxes => (1, 8),
            Step::Prices => (2, 18),
            Step::Machine => (1, 17),
            Step::Push => (2, 9),
            Step::Compare => (8, 8), // Six coverage/discovery commands plus the test lease.
            Step::Wallet => (1, 11),
            Step::Rates => (4, 8),
        };
        assert_eq!(
            (idle.len(), healthy.len()),
            expected,
            "{step:?}: command count changed"
        );
        assert!(f.healthy_done(step, &id).await, "{step:?}: invalid fixture");
        println!(
            "PROVIDER_STEP_COUNTS {} idle={} healthy={} idle_commands={idle:?} healthy_commands={healthy:?}",
            step.name(),
            idle.len(),
            healthy.len()
        );
        f.db.drop().await.unwrap();
    }
}
#[tokio::test]
async fn provider_step_outages_fail_visibly() {
    for step in Step::ALL {
        let f = Fixture::new().await;
        f.healthy(step).await;
        *f.provider.failure.lock().unwrap() = Some("outage");
        let result = step.run(&f).await;
        assert!(
            matches!(
                result,
                Err(AppError::BillingRecoveryFailed {
                    failed_units: 1,
                    ..
                })
            ),
            "{step:?}: {result:?}"
        );
        f.status(step, result, true).await;
        f.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn provider_step_plan_contention_is_waiting() {
    for step in [Step::Prices, Step::Machine] {
        let f = Fixture::new().await;
        f.healthy(step).await;
        let runtime = ClusterLeaseRuntime::new(
            crate::models::coordination::CoordinationHolder {
                instance_id: "live-plan-writer".into(),
                generation_id: uuid::Uuid::new_v4().to_string(),
            },
            std::time::Duration::from_secs(60),
            std::time::Duration::from_secs(10),
        );
        let token = runtime
            .acquire(&f.db, "billing:plan:standard")
            .await
            .unwrap()
            .unwrap();
        let result = step.run(&f).await;
        assert!(result.is_ok(), "{step:?}: {result:?}");
        assert!(
            !f.provider
                .calls
                .lock()
                .unwrap()
                .iter()
                .any(|c| c.starts_with("sync:"))
        );
        f.status(step, result, false).await;
        LeaseStore::release(&f.db, &token).await.unwrap();
        f.db.drop().await.unwrap();
    }
}
#[tokio::test]
async fn provider_step_provider_not_yet_and_rate_limit_are_waits() {
    for step in [Step::Prices, Step::Machine, Step::Push] {
        let f = Fixture::new().await;
        let id = f.healthy(step).await;
        *f.provider.failure.lock().unwrap() = Some("not_yet");
        let result = step.run(&f).await;
        assert!(result.is_ok(), "{step:?}: {result:?}");
        assert!(!f.healthy_done(step, &id).await);
        f.status(step, result, false).await;
        if matches!(step, Step::Machine) {
            // The original retry deadline excludes the next pass, without a new marker.
            f.provider.calls.lock().unwrap().clear();
            step.run(&f).await.unwrap();
            assert!(f.provider.calls.lock().unwrap().is_empty());
        }
        f.db.drop().await.unwrap();
    }
}
#[tokio::test]
async fn provider_step_publication_races_are_waits() {
    for step in [Step::Taxes, Step::Rates, Step::Wallet] {
        let f = Fixture::new().await;
        let id = f.healthy(step).await;
        *f.provider.on_read.lock().unwrap() = Some((f.db.clone(), step));
        let result = step.run(&f).await;
        assert!(result.is_ok(), "{step:?}: {result:?}");
        assert!(!f.healthy_done(step, &id).await);
        f.status(step, result, false).await;
        f.db.drop().await.unwrap();
    }
}
#[tokio::test]
async fn provider_step_processing_failure_keeps_retry_and_visits_later_wallet() {
    for step in [Step::Taxes, Step::Wallet, Step::Push] {
        let f = Fixture::new().await;
        match step {
            Step::Taxes | Step::Wallet => f.wallet("a-bad").await,
            _ => {
                let mut row = tests::finalized_row("a-bad");
                row.id = "a-bad".into();
                row.billing_owner_id = "a-bad".into();
                f.db.collection::<UsageMeterRow>(USAGE_METER)
                    .insert_one(row)
                    .await
                    .unwrap();
            }
        }
        let id = f.healthy(step).await;
        *f.provider.failure.lock().unwrap() = Some("first");
        let result = step.run(&f).await;
        assert!(
            matches!(
                result,
                Err(AppError::BillingRecoveryFailed {
                    failed_units: 1,
                    ..
                })
            ),
            "{step:?}: {result:?}"
        );
        assert!(f.healthy_done(step, &id).await);
        f.status(step, result, true).await;
        if matches!(step, Step::Push) {
            let r =
                f.db.collection::<UsageMeterRow>(USAGE_METER)
                    .find_one(doc! {"_id":"a-bad"})
                    .await
                    .unwrap()
                    .unwrap();
            assert_eq!(r.attempt, 1);
            assert!(!r.lago_acked);
            assert_eq!(r.quantity, Some(1));
            f.db.collection::<Document>(USAGE_METER)
                .update_one(
                    doc! {"_id":"a-bad"},
                    doc! {"$set":{"updated_at":bson::DateTime::from_millis(0)}},
                )
                .await
                .unwrap();
        }
        *f.provider.failure.lock().unwrap() = None;
        step.run(&f).await.unwrap();
        assert!(f.healthy_done(step, "a-bad").await);
        f.db.drop().await.unwrap();
    }
}
#[tokio::test]
async fn provider_step_both_tax_pass_entries_report_their_count() {
    let f = Fixture::new().await;
    f.poison(Step::Taxes).await;
    f.healthy(Step::Taxes).await;
    let mut pass = Pass::new(None);
    for name in ["taxes_before_recovery", "taxes_after_prices"] {
        pass.record(name, Step::Taxes.run(&f).await);
    }
    let runtime = ClusterLeaseRuntime::new(
        crate::models::coordination::CoordinationHolder {
            instance_id: "tax-pass".into(),
            generation_id: uuid::Uuid::new_v4().to_string(),
        },
        std::time::Duration::from_secs(30),
        std::time::Duration::from_millis(50),
    );
    let token = runtime
        .acquire(&f.db, RECONCILE_LEASE)
        .await
        .unwrap()
        .unwrap();
    pass.release(&f.db, &token).await.unwrap();
    let ds = billing::reconcile_status::diagnostics(&f.db).await;
    assert!(
        ds.iter().any(|d| d.detail.contains("taxes_before_recovery")
            && d.detail.contains("taxes_after_prices"))
    );
    f.db.drop().await.unwrap();
}
#[tokio::test]
async fn provider_step_selection_caps_are_unchanged() {
    for step in [
        Step::Taxes,
        Step::Prices,
        Step::Push,
        Step::Wallet,
        Step::Machine,
    ] {
        let f = Fixture::new().await;
        f.poison(step).await;
        let (collection, cap) = match step {
            Step::Taxes | Step::Wallet => (BILLING_WALLET, 100),
            Step::Prices => ("downstream_services", 100),
            Step::Push => (USAGE_METER, 100),
            _ => ("machine_runtime_rates", 4),
        };
        let raw =
            f.db.collection::<Document>(collection)
                .find_one(doc! {})
                .await
                .unwrap()
                .unwrap();
        let mut copies = Vec::new();
        for n in 1..cap + 1 {
            let mut r = raw.clone();
            r.insert("_id", format!("a-bad-{n:04}"));
            copies.push(r);
        }
        f.db.collection::<Document>(collection)
            .insert_many(copies)
            .await
            .unwrap();
        // The unsorted machine claim may use the _id index after an exclusion.
        // Keep this cap fixture all-bad; the separate claim-rejection test proves
        // that it can reach healthy work within the same four attempts.
        let id = if step == Step::Machine {
            None
        } else {
            Some(f.healthy(step).await)
        };
        let result = step.run(&f).await;
        assert!(
            matches!(result,Err(AppError::BillingRecoveryFailed{failed_units,..}) if failed_units==cap as u64),
            "{step:?}: {result:?}"
        );
        if let Some(id) = id {
            assert!(
                !f.healthy_done(step, &id).await,
                "{step:?}: exceeded selection cap"
            );
        }
        f.db.drop().await.unwrap();
    }
}
#[tokio::test]
async fn provider_step_machine_rejected_claim_is_excluded_only_in_pass() {
    let f = Fixture::new().await;
    f.db.create_collection("machine_runtime_rates")
        .validator(doc! {"$or":[{"_id":{"$ne":"a-bad"}},{"sync_claim":{"$exists":false}}]})
        .await
        .unwrap();
    f.db.collection::<Document>("machine_runtime_rates")
        .insert_one(doc! {"_id":"a-bad","retired":false,"sync_status":"pending"})
        .await
        .unwrap();
    let good = f.healthy(Step::Machine).await;
    let result = Step::Machine.run(&f).await;
    assert!(
        matches!(
            result,
            Err(AppError::BillingRecoveryFailed {
                failed_units: 1,
                ..
            })
        ),
        "{result:?}"
    );
    assert!(f.healthy_done(Step::Machine, &good).await);
    let bad =
        f.db.collection::<Document>("machine_runtime_rates")
            .find_one(doc! {"_id":"a-bad"})
            .await
            .unwrap()
            .unwrap();
    assert_eq!(
        bad,
        doc! {"_id":"a-bad","retired":false,"sync_status":"pending"}
    );
    f.db.drop().await.unwrap();
}
#[tokio::test]
async fn provider_step_machine_readback_pending_is_waiting() {
    let f = Fixture::new().await;
    let id = f.healthy(Step::Machine).await;
    *f.provider.failure.lock().unwrap() = Some("lag");
    let result = Step::Machine.run(&f).await;
    assert!(result.is_ok(), "{result:?}");
    assert!(!f.healthy_done(Step::Machine, &id).await);
    f.status(Step::Machine, result, false).await;
    f.db.drop().await.unwrap();
}
#[tokio::test]
async fn provider_step_rate_row_retry_keeps_original_epoch() {
    let f = Fixture::new().await;
    f.db.create_collection("billing_rate_cache")
        .validator(doc! {"lago_metric_code":{"$ne":"bad"}})
        .await
        .unwrap();
    let captured = billing::rate_cache::capture(&f.db).await.unwrap();
    let row = BillingRateCache {
        id: BillingRateCache::cache_id("bad", None),
        lago_metric_code: "bad".into(),
        model: None,
        credits_per_unit_micros: 1,
        credits_per_unit_pico: Some(1_000_000),
        synced_at: Utc::now(),
        retired_at: None,
    };
    let mut failures = billing::recovery_batch::Failures::default();
    let first = billing::rate_cache::publish_isolated(
        &f.db,
        billing::rate_cache::Chunk {
            captured,
            plan: "standard",
            diagnostics: None,
            rejected: &[],
            rows: std::slice::from_ref(&row),
        },
        &mut failures,
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(first, (captured + 1, 0));
    assert!(failures.finish(()).is_err());
    let mut changed = row.clone();
    changed.id = BillingRateCache::cache_id("winner", None);
    changed.lago_metric_code = "winner".into();
    billing::rate_cache::replace(&f.db, &changed).await.unwrap();
    let mut failures = billing::recovery_batch::Failures::default();
    assert!(
        billing::rate_cache::publish_isolated(
            &f.db,
            billing::rate_cache::Chunk {
                captured: first.0,
                plan: "standard",
                diagnostics: None,
                rejected: &[],
                rows: std::slice::from_ref(&changed)
            },
            &mut failures
        )
        .await
        .unwrap()
        .is_none()
    );
    assert!(failures.finish(()).is_ok());
    f.db.drop().await.unwrap();
}

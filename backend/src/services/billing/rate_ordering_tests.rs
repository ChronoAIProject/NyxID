//! Paused provider snapshots and real MongoDB publication races.
use super::*;
use crate::{
    errors::AppResult,
    models::{
        downstream_service::{DownstreamService, test_helpers::dummy_service},
        service_billing::{PricingSyncStatus, ServiceBilling, ServicePlatformPricing},
    },
    services::billing::{lago_client::*, pricing},
};
use std::sync::{
    Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use tokio::sync::{Notify, Semaphore};

struct Gate {
    armed: AtomicBool,
    entered: Notify,
    release: Semaphore,
}
impl Gate {
    fn new() -> Self {
        Self {
            armed: AtomicBool::new(false),
            entered: Notify::new(),
            release: Semaphore::new(0),
        }
    }
    fn arm(&self) {
        self.armed.store(true, Ordering::SeqCst);
    }
    async fn pause(&self) {
        if self.armed.swap(false, Ordering::SeqCst) {
            self.entered.notify_one();
            self.release.acquire().await.unwrap().forget();
        }
    }
    async fn entered(&self) {
        tokio::time::timeout(std::time::Duration::from_secs(15), self.entered.notified())
            .await
            .unwrap();
    }
    fn resume(&self) {
        self.release.add_permits(1);
    }
}
tokio::task_local! { static CHUNK_GATE: (u64, Arc<Gate>); }
pub(super) async fn after_publication(count: u64) {
    if let Ok((target, gate)) = CHUNK_GATE.try_with(Clone::clone)
        && count >= target
    {
        gate.pause().await;
    }
}

struct Provider {
    plan: Mutex<PlanRates>,
    before_get: Gate,
    after_get: Gate,
    calls: AtomicUsize,
    retain_removed: AtomicBool,
}
impl Provider {
    fn new() -> Self {
        Self {
            plan: Mutex::new(PlanRates::default()),
            before_get: Gate::new(),
            after_get: Gate::new(),
            calls: AtomicUsize::new(0),
            retain_removed: AtomicBool::new(false),
        }
    }
    fn valid(&self, code: &str, price: i64) {
        *self.plan.lock().unwrap() = PlanRates {
            rates: vec![rate(code, price)],
            rejected: vec![],
        };
    }
    fn rejected(&self, code: &str) {
        *self.plan.lock().unwrap() = PlanRates {
            rates: vec![],
            rejected: vec![code.into()],
        };
    }
}
fn rate(code: &str, price: i64) -> PlanRate {
    PlanRate {
        lago_metric_code: code.into(),
        credits_per_unit_micros: price * 1_000_000,
        credits_per_unit_pico: Some(price * 1_000_000_000_000),
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
    async fn record_event(&self, _: &LagoEvent) -> Result<LagoAck, LagoError> {
        unreachable!()
    }
    async fn record_events_batch(&self, _: &[LagoEvent]) -> Result<Vec<LagoAck>, LagoError> {
        unreachable!()
    }
    async fn current_usage(&self, _: &str, _: &str) -> AppResult<LagoUsage> {
        unreachable!()
    }
    async fn wallet_balance(&self, _: &str) -> AppResult<i64> {
        unreachable!()
    }
    async fn entitlements(&self, _: &str) -> AppResult<Vec<Entitlement>> {
        Ok(vec![])
    }
    async fn plan_rates(&self, _: &str) -> AppResult<PlanRates> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.before_get.pause().await;
        let fetched = self.plan.lock().unwrap().clone();
        self.after_get.pause().await;
        Ok(fetched)
    }
    async fn sync_standard_charge(&self, _: &str, input: &ServicePriceSync) -> AppResult<()> {
        self.valid(&input.metric_code, input.credits_per_unit.parse().unwrap());
        Ok(())
    }
    async fn remove_standard_charge(&self, _: &str, code: &str) -> AppResult<()> {
        if !self.retain_removed.load(Ordering::SeqCst) {
            self.plan
                .lock()
                .unwrap()
                .rates
                .retain(|r| r.lago_metric_code != code);
        }
        Ok(())
    }
}

struct Fixture {
    db: mongodb::Database,
    provider: Arc<Provider>,
    reconciler: BillingReconciler,
    commands: Arc<Mutex<Vec<(String, Document)>>>,
    ids: Mutex<BTreeMap<String, String>>,
}
impl Fixture {
    async fn new(label: &str) -> Self {
        let commands = Arc::new(Mutex::new(Vec::new()));
        let observed = commands.clone();
        let db = crate::test_utils::connect_transaction_test_database_with_command_handler(
            label,
            mongodb::event::EventHandler::callback(move |event| {
                if let mongodb::event::command::CommandEvent::Started(e) = event {
                    observed.lock().unwrap().push((e.command_name, e.command));
                }
            }),
        )
        .await;
        let provider = Arc::new(Provider::new());
        let mut config = crate::test_utils::test_app_config();
        config.billing_enabled = true;
        config.lago_plan_code = "standard".into();
        let reconciler =
            BillingReconciler::new(db.clone(), Some(provider.clone()), Arc::new(config));
        Self {
            db,
            provider,
            reconciler,
            commands,
            ids: Mutex::new(BTreeMap::new()),
        }
    }
    async fn refresh(&self) -> ReconcileStats {
        let mut stats = ReconcileStats::default();
        Box::pin(
            self.reconciler
                .refresh_rate_cache(self.provider.as_ref(), &mut stats),
        )
        .await
        .unwrap();
        stats
    }
    async fn price(&self, code: &str, price: Option<i64>) {
        let mut service = dummy_service();
        service.id = self
            .ids
            .lock()
            .unwrap()
            .entry(code.into())
            .or_insert_with(|| uuid::Uuid::new_v4().to_string())
            .clone();
        service.slug = code.into();
        service.billing = Some(ServiceBilling {
            platform_pricing: price.map(|price| ServicePlatformPricing {
                lago_metric_code: code.into(),
                credits_per_unit: price.to_string(),
                sync_status: PricingSyncStatus::Pending,
                sync_error: None,
            }),
            platform_pricing_cleanup_metric_code: price.is_none().then(|| code.into()),
            ..Default::default()
        });
        self.db
            .collection::<DownstreamService>("downstream_services")
            .replace_one(doc! {"_id":&service.id}, &service)
            .upsert(true)
            .await
            .unwrap();
        assert!(
            Box::pin(pricing::sync_service_price(
                &self.db,
                self.provider.as_ref(),
                "standard",
                &service
            ))
            .await
            .unwrap()
        );
    }
    async fn cached(&self, code: &str) -> Option<BillingRateCache> {
        self.db
            .collection::<BillingRateCache>("billing_rate_cache")
            .find_one(doc! {"_id":BillingRateCache::cache_id(code,None)})
            .await
            .unwrap()
    }
    async fn diagnostic(&self) -> Option<Document> {
        self.db
            .collection::<Document>("billing_rate_diagnostics")
            .find_one(doc! {"_id":"standard"})
            .await
            .unwrap()
    }
}

#[tokio::test]
async fn empty_cache_removal_discards_an_older_fetch() {
    let f = Fixture::new("rate_empty_remove").await;
    f.provider.valid("price", 1);
    f.provider.after_get.arm();
    let (stats, ()) = tokio::join!(
        Box::pin(f.refresh()),
        Box::pin(async {
            f.provider.after_get.entered().await;
            f.price("price", None).await;
            f.provider.after_get.resume();
        })
    );
    assert!(
        f.cached("price").await.is_none(),
        "a missing-code retirement must fence an old insert"
    );
    assert!(
        f.diagnostic().await.is_none(),
        "discarded fetch must not publish diagnostics"
    );
    assert_eq!(stats.rate_cache_refreshes, 0);
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn older_valid_fetch_cannot_undo_newer_rejection() {
    let f = Fixture::new("rate_valid_rejection").await;
    f.provider.valid("price", 1);
    f.provider.after_get.arm();
    let (stats, winner) = tokio::join!(
        Box::pin(f.refresh()),
        Box::pin(async {
            f.provider.after_get.entered().await;
            f.provider.rejected("price");
            f.refresh().await;
            let winner = f.diagnostic().await;
            f.provider.after_get.resume();
            winner
        })
    );
    assert!(
        f.cached("price").await.is_none(),
        "old valid fetch resurrected a rejected price"
    );
    assert_eq!(f.diagnostic().await, winner);
    assert_eq!(stats.rate_cache_refreshes, 0);
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn older_rejection_cannot_delete_newer_refresh() {
    let f = Fixture::new("rate_rejection_valid").await;
    f.provider.rejected("price");
    f.provider.after_get.arm();
    let (_, winner) = tokio::join!(
        Box::pin(f.refresh()),
        Box::pin(async {
            f.provider.after_get.entered().await;
            f.provider.valid("price", 2);
            f.refresh().await;
            let winner = f.diagnostic().await;
            f.provider.after_get.resume();
            winner
        })
    );
    assert_eq!(
        f.cached("price").await.map(|r| r.credits_per_unit_micros),
        Some(2_000_000)
    );
    assert_eq!(f.diagnostic().await, winner);
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn older_rejection_cannot_delete_explicit_price() {
    let f = Fixture::new("rate_rejection_explicit").await;
    f.provider.rejected("price");
    f.provider.after_get.arm();
    tokio::join!(
        Box::pin(f.refresh()),
        Box::pin(async {
            f.provider.after_get.entered().await;
            f.price("price", Some(2)).await;
            f.provider.after_get.resume();
        })
    );
    assert_eq!(
        f.cached("price").await.map(|r| r.credits_per_unit_micros),
        Some(2_000_000)
    );
    assert!(f.diagnostic().await.is_none());
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn change_after_capture_before_provider_read_discards_even_a_fresh_payload() {
    let f = Fixture::new("rate_before_get").await;
    f.provider.before_get.arm();
    let (stats, ()) = tokio::join!(
        Box::pin(f.refresh()),
        Box::pin(async {
            f.provider.before_get.entered().await;
            f.price("price", Some(2)).await;
            f.provider.before_get.resume();
        })
    );
    assert_eq!(
        stats.rate_cache_refreshes, 0,
        "must keep the pre-provider capture"
    );
    assert!(f.diagnostic().await.is_none());
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn retirement_then_resync_defeats_an_older_fetch() {
    let f = Fixture::new("rate_retire_resync").await;
    f.price("price", Some(1)).await;
    f.provider.after_get.arm();
    tokio::join!(
        Box::pin(f.refresh()),
        Box::pin(async {
            f.provider.after_get.entered().await;
            f.price("price", None).await;
            assert!(f.cached("price").await.unwrap().retired_at.is_some());
            f.price("price", Some(2)).await;
            f.provider.after_get.resume();
        })
    );
    let saved = f.cached("price").await.unwrap();
    assert_eq!(saved.credits_per_unit_micros, 2_000_000);
    assert!(saved.retired_at.is_none());
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn refresh_captured_after_retirement_preserves_the_historical_row() {
    let f = Fixture::new("rate_after_retirement").await;
    f.price("price", Some(1)).await;
    f.provider.retain_removed.store(true, Ordering::SeqCst);
    f.price("price", None).await;
    let retired = f.cached("price").await.unwrap();
    f.provider.valid("price", 9);
    f.refresh().await;
    assert_eq!(
        f.cached("price").await,
        Some(retired),
        "provider lag must not revive or reprice a retired row"
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn a_late_former_reconcile_holder_cannot_republish_after_its_successor() {
    let f = Fixture::new("rate_old_holder").await;
    f.provider.valid("price", 1);
    f.provider.after_get.arm();
    tokio::join!(
        Box::pin(f.reconciler.run_once()),
        Box::pin(async {
            f.provider.after_get.entered().await;
            f.db.collection::<Document>(crate::models::coordination::LEASE_COLLECTION_NAME)
                .update_one(
                    doc! {"_id":RECONCILE_LEASE},
                    doc! {"$set":{"expires_at":bson::DateTime::from_millis(0)}},
                )
                .await
                .unwrap();
            f.provider.valid("price", 2);
            Box::pin(f.reconciler.run_once()).await.unwrap();
            f.provider.after_get.resume();
        })
    )
    .0
    .unwrap();
    assert_eq!(
        f.cached("price").await.unwrap().credits_per_unit_micros,
        2_000_000
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn explicit_write_between_chunks_discards_the_remaining_fetch() {
    let f = Fixture::new("rate_chunk_interruption").await;
    f.provider.plan.lock().unwrap().rates =
        (0..205).map(|n| rate(&format!("rate_{n:03}"), 1)).collect();
    let gate = Arc::new(Gate::new());
    gate.arm();
    let (stats, ()) = tokio::join!(
        CHUNK_GATE.scope((100, gate.clone()), Box::pin(f.refresh())),
        Box::pin(async {
            gate.entered().await;
            f.price("rate_150", Some(9)).await;
            gate.resume();
        })
    );
    assert_eq!(
        f.cached("rate_150").await.unwrap().credits_per_unit_micros,
        9_000_000
    );
    assert!(
        f.cached("rate_200").await.is_none(),
        "remaining old chunks must be discarded"
    );
    assert_eq!(stats.rate_cache_refreshes, 100);
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn later_rejection_preserves_retired_historical_prices() {
    let f = Fixture::new("rate_retired_rejection").await;
    f.price("price", Some(1)).await;
    f.price("price", None).await;
    let retired = f.cached("price").await;
    f.provider.rejected("price");
    f.refresh().await;
    assert_eq!(f.cached("price").await, retired);
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn explicit_prices_and_absent_retirement_advance_one_epoch() {
    let f = Fixture::new("rate_explicit_epochs").await;
    f.price("a", Some(1)).await;
    f.price("b", Some(2)).await;
    f.price("missing", None).await;
    let epochs: Vec<Document> =
        f.db.collection::<Document>("billing_rate_epoch")
            .find(doc! {})
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
    assert_eq!(
        epochs.len(),
        1,
        "every explicit cache writer must advance the singleton"
    );
    assert_eq!(epochs[0].get_i64("generation"), Ok(3));
    assert!(f.cached("missing").await.is_none());
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn rate_publication_baseline_command_counts() {
    let f = Fixture::new("rate_counts").await;
    f.provider.valid("price", 1);
    f.refresh().await;
    f.commands.lock().unwrap().clear();
    f.refresh().await;
    println!(
        "RATE_COMMANDS periodic_one_price: {:?}",
        f.commands
            .lock()
            .unwrap()
            .iter()
            .map(|(n, _)| n)
            .collect::<Vec<_>>()
    );
    f.commands.lock().unwrap().clear();
    f.price("price", Some(2)).await;
    println!(
        "RATE_COMMANDS explicit_service_with_source_save: {:?}",
        f.commands
            .lock()
            .unwrap()
            .iter()
            .map(|(n, _)| n)
            .collect::<Vec<_>>()
    );
    f.db.drop().await.unwrap();
}

//! Exercise the complete pass against MongoDB; only Lago is replaced.
use super::{BillingReconciler, tests::billing_enabled_config, tests::finalized_row};
use crate::{
    errors::{AppError, AppResult},
    models::{
        billing_rate_cache::BillingRateCache,
        sandbox_rate_card::{COLLECTION_NAME as CARDS, SandboxRateCard},
        service_billing::PricingSyncStatus,
        usage_meter::{COLLECTION_NAME as USAGE, UsageMeterRow},
    },
    services::{
        billing::{lago_client::*, sandbox_rates, taxes},
        feature_flag_service::{self as flags, CMA_SANDBOX_RATE_SYNC_FLAG_KEY, FlagTarget},
    },
};
use async_trait::async_trait;
use chrono::{Duration, Utc};
use mongodb::bson::{self, Document, doc};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicU8, Ordering},
};

const SUCCESS: u8 = 0;
const FAILURE: u8 = 1;
const TIMEOUT: u8 = 2;

#[derive(Default)]
struct Provider {
    mode: AtomicU8,
    calls: Mutex<Vec<&'static str>>,
}

#[async_trait]
impl LagoApi for Provider {
    async fn ensure_customer(&self, _: &OwnerProvisionInput) -> AppResult<String> {
        panic!("reconciliation must not provision customers")
    }

    async fn ensure_subscription(&self, _: &str, _: &str) -> AppResult<String> {
        panic!("reconciliation must not provision subscriptions")
    }

    async fn sync_standard_charge(&self, _: &str, _: &ServicePriceSync) -> AppResult<()> {
        self.calls.lock().unwrap().push("sync");
        match self.mode.load(Ordering::SeqCst) {
            FAILURE => Err(AppError::BillingProviderUnavailable("test failure".into())),
            TIMEOUT => std::future::pending().await,
            _ => Ok(()),
        }
    }

    async fn tax_config(&self, _: &str, _: &str) -> AppResult<LagoTaxConfig> {
        self.calls.lock().unwrap().push("tax");
        Ok(taxes::zero_config())
    }

    async fn record_event(&self, event: &LagoEvent) -> Result<LagoAck, LagoError> {
        assert_eq!(event.code, "platform_requests");
        self.calls.lock().unwrap().push("push");
        Ok(LagoAck {
            transaction_id: event.transaction_id.clone(),
        })
    }

    async fn record_events_batch(&self, _: &[LagoEvent]) -> Result<Vec<LagoAck>, LagoError> {
        panic!("this pass emits individual events")
    }

    async fn current_usage(&self, customer: &str, subscription: &str) -> AppResult<LagoUsage> {
        self.calls.lock().unwrap().push("usage");
        Ok(LagoUsage {
            customer_id: customer.into(),
            subscription_id: subscription.into(),
            raw: serde_json::json!({}),
        })
    }

    async fn wallet_balance(&self, _: &str) -> AppResult<i64> {
        self.calls.lock().unwrap().push("balance");
        Ok(0)
    }

    async fn entitlements(&self, _: &str) -> AppResult<Vec<Entitlement>> {
        Ok(vec![])
    }

    async fn plan_rates(&self, _: &str) -> AppResult<PlanRates> {
        self.calls.lock().unwrap().push("rates");
        Ok(PlanRates::default())
    }
}

struct Fixture {
    db: mongodb::Database,
    provider: Arc<Provider>,
    reconciler: BillingReconciler,
    row: UsageMeterRow,
}

impl Fixture {
    async fn new(name: &str, mode: u8) -> Self {
        let db = crate::test_utils::connect_transaction_test_database(name).await;
        sandbox_rates::ensure_indexes(&db).await.unwrap();
        sandbox_rates::seed(&db, "standard", &[42; 32])
            .await
            .unwrap();
        let now = bson::DateTime::now();
        db.collection::<Document>(super::BILLING_WALLET)
            .insert_one(doc! {
                "_id": "wallet-1", "owner_id": "owner-1",
                "lago_customer_id": "customer-1", "lago_subscription_id": "subscription-1",
                "plan_kind": "prepaid", "collection_state": "good",
                "balance_synced_at": now, "created_at": now, "updated_at": now,
            })
            .await
            .unwrap();
        let row = finalized_row("ordinary-usage-during-cma-sync");
        db.collection::<UsageMeterRow>(USAGE)
            .insert_one(&row)
            .await
            .unwrap();
        let provider = Arc::new(Provider::default());
        provider.mode.store(mode, Ordering::SeqCst);
        let mut config = billing_enabled_config();
        config.lago_plan_code = "standard".into();
        let reconciler =
            BillingReconciler::new(db.clone(), Some(provider.clone()), Arc::new(config));
        Self {
            db,
            provider,
            reconciler,
            row,
        }
    }

    async fn assert_emitted(&self) {
        let saved = self
            .db
            .collection::<UsageMeterRow>(USAGE)
            .find_one(doc! {"_id": &self.row.id})
            .await
            .unwrap()
            .unwrap();
        assert!(
            saved.lago_acked,
            "ordinary usage must be durably acknowledged"
        );
        let calls = self.provider.calls.lock().unwrap();
        let push = calls.iter().position(|call| *call == "push").unwrap();
        assert!(calls[..push].contains(&"tax"));
        for later in ["usage", "balance", "rates"] {
            assert!(
                calls[push + 1..].contains(&later),
                "{later} must still run: {calls:?}"
            );
        }
    }

    async fn set_sync_enabled(&self, enabled: bool) {
        flags::set_platform_override(
            &self.db,
            CMA_SANDBOX_RATE_SYNC_FLAG_KEY,
            &FlagTarget::Global,
            enabled,
            "admin",
        )
        .await
        .unwrap();
    }
}

async fn assert_sync_pass(mode: u8, reject_sync_marker: bool) {
    let f = Fixture::new("cma_reconcile_isolation", mode).await;
    if reject_sync_marker {
        // A persistence error escapes the per-card provider-error handling.
        // This would abort run_once with the original `.await?` call.
        f.db.run_command(doc! {"collMod": CARDS,
        "validator": {"sync_status": {"$ne": "synced"}}})
            .await
            .unwrap();
    }
    let stats = f
        .reconciler
        .run_once()
        .await
        .expect("CMA sync cannot abort ordinary billing");
    let success = mode == SUCCESS && !reject_sync_marker;
    assert_eq!(stats.service_price_syncs, u64::from(success));
    assert_eq!(stats.pushed, 1);
    f.assert_emitted().await;
    {
        let calls = f.provider.calls.lock().unwrap();
        let last_sync = calls.iter().rposition(|call| *call == "sync").unwrap();
        let push = calls.iter().position(|call| *call == "push").unwrap();
        assert!(
            calls[..last_sync].contains(&"tax"),
            "initial tax refresh ran"
        );
        assert!(
            calls[last_sync + 1..push].contains(&"tax"),
            "taxes invalidated by CMA sync must refresh before ordinary emission: {calls:?}"
        );
    }
    let card = sandbox_rates::effective(&f.db, Utc::now()).await.unwrap();
    assert_eq!(
        card.sync_status,
        if success {
            PricingSyncStatus::Synced
        } else if mode == FAILURE {
            PricingSyncStatus::Failed
        } else {
            PricingSyncStatus::Pending
        }
    );
    if !success {
        let saved =
            f.db.collection::<Document>(CARDS)
                .find_one(doc! {"version": 1})
                .await
                .unwrap()
                .unwrap();
        assert_eq!(
            saved.contains_key("sync_claim_until"),
            mode == TIMEOUT,
            "only an ambiguous timeout must retain its lease"
        );
        if mode == TIMEOUT {
            let calls_before = f.provider.calls.lock().unwrap().clone();
            let stats = f.reconciler.run_once().await.unwrap();
            assert_eq!(
                stats.service_price_syncs, 0,
                "live lease prevents ambiguous replay"
            );
            assert_eq!(
                f.provider
                    .calls
                    .lock()
                    .unwrap()
                    .iter()
                    .filter(|c| **c == "sync")
                    .count(),
                calls_before.iter().filter(|c| **c == "sync").count()
            );
            f.db.collection::<Document>(CARDS).update_one(doc! {"version": 1},
                doc! {"$set": {"sync_claim_until": bson::DateTime::from_chrono(Utc::now() - Duration::seconds(1))}})
                .await.unwrap();
        }
        if reject_sync_marker {
            f.db.run_command(doc! {"collMod": CARDS, "validator": {}})
                .await
                .unwrap();
        }
        f.provider.mode.store(SUCCESS, Ordering::SeqCst);
        let recovered = f.reconciler.run_once().await.unwrap();
        assert_eq!(
            recovered.service_price_syncs, 1,
            "persisted work must retry"
        );
        assert_eq!(recovered.pushed, 0, "ordinary usage must not be duplicated");
        assert_eq!(
            sandbox_rates::effective(&f.db, Utc::now())
                .await
                .unwrap()
                .sync_status,
            PricingSyncStatus::Synced
        );
    }
}

#[tokio::test]
async fn cma_sync_success_refreshes_taxes_and_pushes_ordinary_usage() {
    assert_sync_pass(SUCCESS, false).await;
}

#[tokio::test]
async fn cma_sync_failure_refreshes_taxes_pushes_usage_and_retries() {
    assert_sync_pass(FAILURE, false).await;
}

#[tokio::test]
async fn cma_sync_database_error_does_not_abort_tax_refresh_or_usage_push() {
    assert_sync_pass(SUCCESS, true).await;
}

#[tokio::test]
async fn cma_sync_timeout_refreshes_taxes_pushes_usage_and_retains_retry_lease() {
    // Exercise the real 60-second batch deadline, including MongoDB I/O.
    assert_sync_pass(TIMEOUT, false).await;
}

#[tokio::test]
async fn cma_sync_runtime_disable_preserves_rates_and_continues_reconciliation() {
    let f = Fixture::new("cma_reconcile_sync_flag", SUCCESS).await;
    assert!(flags::cma_sandbox_rate_sync_enabled(&f.db).await.unwrap());
    f.db.collection::<Document>(CARDS)
        .update_one(
            doc! {"version": 1},
            doc! {"$set": {"sync_status": "synced"}},
        )
        .await
        .unwrap();
    let original =
        f.db.collection::<Document>(CARDS)
            .find_one(doc! {"version": 1})
            .await
            .unwrap()
            .unwrap();
    let mut pending = sandbox_rates::effective(&f.db, Utc::now()).await.unwrap();
    pending.id = uuid::Uuid::new_v4().to_string();
    pending.version = 2;
    pending.effective_from += Duration::hours(1);
    pending.sync_status = PricingSyncStatus::Pending;
    for price in &mut pending.prices {
        price.lago_metric_code = format!("cma_{}_v2", price.metric.as_str());
    }
    f.db.collection::<SandboxRateCard>(CARDS)
        .insert_one(&pending)
        .await
        .unwrap();
    let cache = BillingRateCache {
        id: BillingRateCache::cache_id("cma_sandbox_cpu_millis_seconds_v1", None),
        lago_metric_code: "cma_sandbox_cpu_millis_seconds_v1".into(),
        model: None,
        credits_per_unit_pico: Some(10_000),
        credits_per_unit_micros: 0,
        synced_at: Utc::now(),
        retired_at: None,
    };
    let rates =
        f.db.collection::<BillingRateCache>(crate::models::billing_rate_cache::COLLECTION_NAME);
    rates.insert_one(&cache).await.unwrap();
    f.set_sync_enabled(false).await;
    let stats = f.reconciler.run_once().await.unwrap();
    assert_eq!(stats.service_price_syncs, 0);
    assert_eq!(stats.pushed, 1);
    f.assert_emitted().await;
    assert!(!f.provider.calls.lock().unwrap().contains(&"sync"));
    assert_eq!(
        f.db.collection::<Document>(CARDS)
            .find_one(doc! {"version": 1})
            .await
            .unwrap()
            .unwrap(),
        original
    );
    let saved =
        f.db.collection::<Document>(CARDS)
            .find_one(doc! {"version": 2})
            .await
            .unwrap()
            .unwrap();
    assert_eq!(saved, bson::to_document(&pending).unwrap());
    assert_eq!(
        bson::to_document(
            &rates
                .find_one(doc! {"_id": &cache.id})
                .await
                .unwrap()
                .unwrap()
        )
        .unwrap(),
        bson::to_document(&cache).unwrap()
    );
    f.set_sync_enabled(true).await;
    assert_eq!(
        f.reconciler.run_once().await.unwrap().service_price_syncs,
        1
    );
    assert_eq!(
        f.provider
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|c| **c == "sync")
            .count(),
        3
    );
    assert!(
        !flags::find_flag(flags::CMA_SANDBOX_BILLING_FLAG_KEY)
            .unwrap()
            .default_enabled,
        "sync enablement must not enable admission"
    );
}

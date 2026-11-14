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
    pause_push: std::sync::atomic::AtomicBool,
    pause_expiry: std::sync::atomic::AtomicBool,
    expiry_entered: tokio::sync::Notify,
    resume_expiry: tokio::sync::Notify,
    expiry_posts: std::sync::atomic::AtomicUsize,
    pause_sync: std::sync::atomic::AtomicBool,
    sync_entered: tokio::sync::Notify,
    resume_sync: tokio::sync::Notify,
    push_ids: Mutex<Vec<String>>,
    active_pushes: std::sync::atomic::AtomicUsize,
    max_active_pushes: std::sync::atomic::AtomicUsize,
    push_entered: tokio::sync::Notify,
    resume_push: tokio::sync::Notify,
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
        if self.pause_sync.swap(false, Ordering::SeqCst) {
            self.sync_entered.notify_one();
            self.resume_sync.notified().await;
        }
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
        self.push_ids
            .lock()
            .unwrap()
            .push(event.transaction_id.clone());
        let active = self.active_pushes.fetch_add(1, Ordering::SeqCst) + 1;
        self.max_active_pushes.fetch_max(active, Ordering::SeqCst);
        if self.pause_push.swap(false, Ordering::SeqCst) {
            self.push_entered.notify_one();
            self.resume_push.notified().await;
        }
        self.active_pushes.fetch_sub(1, Ordering::SeqCst);
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

    async fn wallet_transactions(&self, _: &str) -> AppResult<Vec<LagoWalletTransaction>> {
        if self.pause_expiry.swap(false, Ordering::SeqCst) {
            self.expiry_entered.notify_one();
            self.resume_expiry.notified().await;
        }
        Ok(vec![])
    }

    async fn void_wallet_credits(
        &self,
        _: &str,
        _: crate::models::credits::Credits,
        _: &str,
    ) -> AppResult<String> {
        self.expiry_posts.fetch_add(1, Ordering::SeqCst);
        Err(AppError::BillingProviderUnavailable(
            "uncertain test dispatch".into(),
        ))
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

#[tokio::test]
async fn reconcile_lease_race_skips_contender_and_releases_for_next_tick() {
    let f = Fixture::new("reconcile_lease_race", SUCCESS).await;
    f.provider.pause_push.store(true, Ordering::SeqCst);
    let first = f.reconciler.clone();
    let holder = tokio::spawn(async move { first.run_once().await });
    tokio::time::timeout(
        std::time::Duration::from_secs(10),
        f.provider.push_entered.notified(),
    )
    .await
    .unwrap();
    let contender = f.reconciler.run_once().await.unwrap();
    f.provider.resume_push.notify_one();
    let winner = holder.await.unwrap().unwrap();
    assert_eq!(
        contender,
        super::ReconcileStats::default(),
        "contender must skip the entire pass"
    );
    assert_eq!(winner.pushed, 1);
    assert_eq!(
        f.provider
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|c| **c == "push")
            .count(),
        1
    );
    let before = f
        .provider
        .calls
        .lock()
        .unwrap()
        .iter()
        .filter(|c| **c == "rates")
        .count();
    f.reconciler.run_once().await.unwrap();
    assert_eq!(
        f.provider
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|c| **c == "rates")
            .count(),
        before + 1
    );
    f.db.drop().await.unwrap();
}

fn lease_runtime(
    instance: &str,
    ttl_ms: u64,
) -> crate::services::coordination_service::ClusterLeaseRuntime {
    crate::services::coordination_service::ClusterLeaseRuntime::new(
        crate::models::coordination::CoordinationHolder {
            instance_id: instance.into(),
            generation_id: uuid::Uuid::new_v4().to_string(),
        },
        std::time::Duration::from_millis(ttl_ms),
        std::time::Duration::from_millis(50),
    )
}

#[tokio::test]
async fn reconcile_lease_renews_during_a_slow_provider_unit() {
    let f = Fixture::new("reconcile_renew", SUCCESS).await;
    f.provider.pause_push.store(true, Ordering::SeqCst);
    let first = f.reconciler.clone();
    let holder =
        tokio::spawn(async move { first.run_once_with_runtime(&lease_runtime("a", 500)).await });
    tokio::time::timeout(
        std::time::Duration::from_secs(10),
        f.provider.push_entered.notified(),
    )
    .await
    .unwrap();
    tokio::time::sleep(std::time::Duration::from_millis(800)).await;
    assert_eq!(
        f.reconciler
            .run_once_with_runtime(&lease_runtime("b", 500))
            .await
            .unwrap(),
        super::ReconcileStats::default()
    );
    f.provider.resume_push.notify_one();
    assert_eq!(holder.await.unwrap().unwrap().pushed, 1);
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn reconcile_lease_loss_finishes_one_unit_then_successor_resumes_after_expiry() {
    use crate::models::coordination::{CoordinationLease, LEASE_COLLECTION_NAME};
    use crate::services::coordination_service::{LeaseStore, LeaseToken};
    let f = Fixture::new("reconcile_loss", SUCCESS).await;
    let mut second = finalized_row("after-lease-loss");
    second.updated_at = f.row.updated_at + Duration::seconds(1);
    f.db.collection::<UsageMeterRow>(USAGE)
        .insert_one(&second)
        .await
        .unwrap();
    f.provider.pause_push.store(true, Ordering::SeqCst);
    let lost = Arc::new(tokio::sync::Notify::new());
    let first = f.reconciler.clone();
    let holder = tokio::spawn(super::LEASE_LOSS_OBSERVED.scope(lost.clone(), async move {
        first.run_once_with_runtime(&lease_runtime("a", 2000)).await
    }));
    tokio::time::timeout(
        std::time::Duration::from_secs(10),
        f.provider.push_entered.notified(),
    )
    .await
    .unwrap();
    let leases = f.db.collection::<CoordinationLease>(LEASE_COLLECTION_NAME);
    let old = leases
        .find_one(doc! {"_id":super::RECONCILE_LEASE})
        .await
        .unwrap()
        .unwrap();
    let old_token = LeaseToken {
        name: old.id,
        holder: old.holder,
        lease_id: old.lease_id,
    };
    // Inject loss of the exact renewal token while retaining an unexpired row.
    // This makes the real Mongo renewal fail, without a global failpoint.
    leases
        .update_one(
            doc! {"_id":super::RECONCILE_LEASE},
            doc! {"$set": {
                "lease_id":"injected-lost-token",
                "expires_at":bson::DateTime::from_chrono(Utc::now()+Duration::seconds(2)),
            }},
        )
        .await
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(1), lost.notified())
        .await
        .unwrap();
    assert_eq!(
        f.reconciler
            .run_once_with_runtime(&lease_runtime("b", 2000))
            .await
            .unwrap(),
        super::ReconcileStats::default()
    );
    f.provider.resume_push.notify_one();
    assert_eq!(
        holder.await.unwrap().unwrap().pushed,
        1,
        "finish the in-flight ACK, do not start row two"
    );
    assert!(
        !f.provider.calls.lock().unwrap().contains(&"rates"),
        "later stages must stop too"
    );
    assert!(
        !f.db
            .collection::<UsageMeterRow>(USAGE)
            .find_one(doc! {"_id":&second.id})
            .await
            .unwrap()
            .unwrap()
            .lago_acked
    );
    assert!(
        leases
            .find_one(doc! {"_id":super::RECONCILE_LEASE})
            .await
            .unwrap()
            .is_some(),
        "stale cleanup must retain the current lease"
    );
    tokio::time::sleep(std::time::Duration::from_millis(2100)).await;
    f.provider.pause_push.store(true, Ordering::SeqCst);
    let next = f.reconciler.clone();
    let successor =
        tokio::spawn(async move { next.run_once_with_runtime(&lease_runtime("b", 2000)).await });
    tokio::time::timeout(
        std::time::Duration::from_secs(10),
        f.provider.push_entered.notified(),
    )
    .await
    .unwrap();
    assert!(!LeaseStore::release(&f.db, &old_token).await.unwrap());
    assert_eq!(
        leases
            .find_one(doc! {"_id":super::RECONCILE_LEASE})
            .await
            .unwrap()
            .unwrap()
            .holder
            .instance_id,
        "b"
    );
    f.provider.resume_push.notify_one();
    assert_eq!(successor.await.unwrap().unwrap().pushed, 1);
    {
        let ids = f.provider.push_ids.lock().unwrap();
        assert_eq!(ids.len(), 2);
        assert_ne!(
            ids[0], ids[1],
            "no usage unit is dispatched by both holders"
        );
        assert_eq!(f.provider.max_active_pushes.load(Ordering::SeqCst), 1);
    }
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn reconcile_lease_command_delta_is_two_and_direct_leaf_has_none() {
    use mongodb::event::{EventHandler, command::CommandEvent};
    let commands = Arc::new(Mutex::new(Vec::<(String, Document)>::new()));
    let observed = commands.clone();
    let db = crate::test_utils::connect_transaction_test_database_with_command_handler(
        "reconcile_commands",
        EventHandler::<CommandEvent>::callback(move |event| {
            if let CommandEvent::Started(event) = event {
                observed
                    .lock()
                    .unwrap()
                    .push((event.command_name, event.command));
            }
        }),
    )
    .await;
    let reconciler = BillingReconciler::new(
        db.clone(),
        None,
        Arc::new(crate::test_utils::test_app_config()),
    );
    reconciler.run_once().await.unwrap(); // warm one-time setup/anchor checks
    commands.lock().unwrap().clear();
    reconciler.run_pass().await.unwrap();
    let plain = commands.lock().unwrap().clone();
    commands.lock().unwrap().clear();
    reconciler.run_once().await.unwrap();
    let leased = commands.lock().unwrap().clone();
    let is_lease = |command: &Document| {
        ["findAndModify", "delete", "update"]
            .iter()
            .any(|key| command.get_str(key).ok() == Some("coordination_leases"))
    };
    let extra: Vec<_> = leased
        .iter()
        .filter(|(_, doc)| is_lease(doc))
        .map(|(name, _)| name.as_str())
        .collect();
    assert_eq!(extra, ["findAndModify", "delete"]);
    assert_eq!(
        leased.len(),
        plain.len() + 2,
        "only lease acquisition/release are added to an unrenewed tick"
    );
    commands.lock().unwrap().clear();
    reconciler
        .refresh_rate_cache(&Provider::default(), &mut super::ReconcileStats::default())
        .await
        .unwrap();
    assert!(
        !commands
            .lock()
            .unwrap()
            .iter()
            .any(|(_, doc)| is_lease(doc)),
        "direct leaf calls do not acquire the supervisor lease"
    );
    eprintln!(
        "reconcile command delta: plain={}, leased={}, added=2; direct leaf lease commands=0",
        plain.len(),
        leased.len()
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn reconcile_lease_loss_leaves_partial_card_pending() {
    use crate::models::coordination::LEASE_COLLECTION_NAME;
    let f = Fixture::new("reconcile_partial_card", SUCCESS).await;
    f.provider.pause_sync.store(true, Ordering::SeqCst);
    let lost = Arc::new(tokio::sync::Notify::new());
    let first = f.reconciler.clone();
    let holder = tokio::spawn(super::LEASE_LOSS_OBSERVED.scope(lost.clone(), async move {
        first.run_once_with_runtime(&lease_runtime("a", 2000)).await
    }));
    tokio::time::timeout(
        std::time::Duration::from_secs(10),
        f.provider.sync_entered.notified(),
    )
    .await
    .unwrap();
    f.db.collection::<Document>(LEASE_COLLECTION_NAME)
        .update_one(
            doc! {"_id":super::RECONCILE_LEASE},
            doc! {"$set":{"lease_id":"injected-loss"}},
        )
        .await
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(1), lost.notified())
        .await
        .unwrap();
    f.provider.resume_sync.notify_one();
    let stopped = holder.await.unwrap().unwrap();
    assert_eq!(stopped.service_price_syncs, 0);
    assert_eq!(stopped.pushed, 0);
    assert_eq!(
        f.provider
            .calls
            .lock()
            .unwrap()
            .iter()
            .filter(|c| **c == "sync")
            .count(),
        1
    );
    let card = sandbox_rates::effective(&f.db, Utc::now()).await.unwrap();
    assert!(card.prices.len() > 1);
    assert_eq!(card.sync_status, PricingSyncStatus::Pending);
    let lock =
        f.db.collection::<Document>(CARDS)
            .find_one(doc! {"version":1})
            .await
            .unwrap()
            .unwrap();
    assert!(
        !lock.contains_key("sync_claim"),
        "completed cooperative stop releases the card claim"
    );
    f.db.collection::<Document>(LEASE_COLLECTION_NAME)
        .update_one(
            doc! {"_id":super::RECONCILE_LEASE},
            doc! {"$set":{"expires_at":bson::DateTime::from_millis(0)}},
        )
        .await
        .unwrap();
    let resumed = f
        .reconciler
        .run_once_with_runtime(&lease_runtime("b", 2000))
        .await
        .unwrap();
    assert_eq!(resumed.service_price_syncs, 1);
    assert_eq!(resumed.pushed, 1);
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn reconcile_lease_simultaneous_start_ticks_have_one_winner() {
    let f = Fixture::new("reconcile_startup_race", SUCCESS).await;
    f.provider.pause_push.store(true, Ordering::SeqCst);
    let start = Arc::new(tokio::sync::Barrier::new(3));
    let mut ticks = tokio::task::JoinSet::new();
    for instance in ["startup-a", "startup-b"] {
        let reconciler = f.reconciler.clone();
        let start = start.clone();
        ticks.spawn(async move {
            start.wait().await;
            reconciler
                .run_once_with_runtime(&lease_runtime(instance, 30_000))
                .await
        });
    }
    start.wait().await;
    tokio::time::timeout(
        std::time::Duration::from_secs(10),
        f.provider.push_entered.notified(),
    )
    .await
    .unwrap();
    let skipped = tokio::time::timeout(std::time::Duration::from_secs(10), ticks.join_next())
        .await
        .unwrap()
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(skipped, super::ReconcileStats::default());
    f.provider.resume_push.notify_one();
    assert_eq!(ticks.join_next().await.unwrap().unwrap().unwrap().pushed, 1);
    assert_eq!(f.provider.push_ids.lock().unwrap().len(), 1);
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn shutdown_reconcile_finishes_current_unit_stops_next_and_releases_token_for_peer() {
    let f = Fixture::new("shutdown_reconcile", SUCCESS).await;
    let mut second = finalized_row("after-drain");
    second.updated_at = f.row.updated_at + Duration::seconds(1);
    f.db.collection::<UsageMeterRow>(USAGE)
        .insert_one(&second)
        .await
        .unwrap();
    f.provider.pause_push.store(true, Ordering::SeqCst);
    let shutdown = crate::services::shutdown::Runtime::default();
    let first = f.reconciler.clone();
    let run_shutdown = shutdown.clone();
    let holder = tokio::spawn(async move {
        first
            .run_once_with_shutdown(&lease_runtime("a", 30_000), &run_shutdown)
            .await
    });
    tokio::time::timeout(
        std::time::Duration::from_secs(10),
        f.provider.push_entered.notified(),
    )
    .await
    .unwrap();
    shutdown.begin_drain();
    assert!(!holder.is_finished());
    f.provider.resume_push.notify_one();
    assert_eq!(holder.await.unwrap().unwrap().pushed, 1);
    assert!(
        !f.db
            .collection::<UsageMeterRow>(USAGE)
            .find_one(doc! {"_id":&second.id})
            .await
            .unwrap()
            .unwrap()
            .lago_acked
    );
    assert_eq!(shutdown.tracked_units(), 0);
    assert_eq!(
        f.reconciler
            .run_once_with_shutdown(&lease_runtime("a", 30_000), &shutdown)
            .await
            .unwrap(),
        super::ReconcileStats::default()
    );
    let peer = lease_runtime("b", 30_000);
    let token = peer
        .acquire(&f.db, super::RECONCILE_LEASE)
        .await
        .unwrap()
        .expect("peer acquires immediately without waiting 30s");
    assert!(
        crate::services::coordination_service::LeaseStore::release(&f.db, &token)
            .await
            .unwrap()
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn shutdown_reconcile_deadline_abort_retains_active_provider_unit_lease_until_ttl() {
    let f = Fixture::new("shutdown_reconcile_abort", SUCCESS).await;
    f.provider.pause_push.store(true, Ordering::SeqCst);
    let shutdown = crate::services::shutdown::Runtime::default();
    let first = f.reconciler.clone();
    let run_shutdown = shutdown.clone();
    let holder = shutdown.spawn(async move {
        first
            .run_once_with_shutdown(&lease_runtime("a", 30_000), &run_shutdown)
            .await
    });
    tokio::time::timeout(
        std::time::Duration::from_secs(10),
        f.provider.push_entered.notified(),
    )
    .await
    .unwrap();
    shutdown.begin_drain();
    shutdown.stop();
    assert!(holder.await.unwrap_err().is_cancelled());
    shutdown.execution_stopped().await;
    assert!(
        lease_runtime("b", 30_000)
            .acquire(&f.db, super::RECONCILE_LEASE)
            .await
            .unwrap()
            .is_none()
    );
    assert!(shutdown.released().is_empty());
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn shutdown_reconcile_expiry_finishes_current_operation_and_preserves_marker() {
    let f = Fixture::new("shutdown_expiry", SUCCESS).await;
    let now = bson::DateTime::now();
    let operation = doc! {
        "operation_id": "expiry-a", "processing_token": "prior-a",
        "lease_until": bson::DateTime::from_millis(0),
        "amount": crate::models::credits::Credits::from_whole(3), "items": [],
        "created_at": now, "updated_at": now,
    };
    f.db.collection::<Document>(super::BILLING_WALLET)
        .update_one(
            doc! {"_id":"wallet-1"},
            doc! {"$set": {"lago_wallet_id":"provider-wallet-1",
            "active_topup_expiry": &operation,
            "topup_expiry_checked_at": bson::DateTime::from_millis(0),
            "pending_topup_expiry_credits": crate::models::credits::Credits::from_whole(3)}},
        )
        .await
        .unwrap();
    let mut next = operation.clone();
    next.insert("operation_id", "expiry-b");
    next.insert("processing_token", "prior-b");
    f.db.collection::<Document>(super::BILLING_WALLET)
        .insert_one(doc! {
            "_id":"wallet-2", "owner_id":"owner-2", "lago_customer_id":"customer-2",
            "lago_wallet_id":"provider-wallet-2", "lago_subscription_id":"subscription-2",
            "plan_kind":"prepaid", "collection_state":"good", "active_topup_expiry":next,
            "topup_expiry_checked_at":now,
            "pending_topup_expiry_credits":crate::models::credits::Credits::from_whole(3),
            "created_at":now, "balance_synced_at":now,
            "updated_at":bson::DateTime::from_chrono(Utc::now() + Duration::seconds(1)),
        })
        .await
        .unwrap();
    f.provider.pause_expiry.store(true, Ordering::SeqCst);
    let shutdown = crate::services::shutdown::Runtime::default();
    let lease = lease_runtime("expiry-a", 30_000);
    let (result, ()) = tokio::join!(
        f.reconciler.run_once_with_shutdown(&lease, &shutdown),
        async {
            tokio::time::timeout(
                std::time::Duration::from_secs(10),
                f.provider.expiry_entered.notified(),
            )
            .await
            .unwrap();
            assert_eq!(shutdown.counts().background, 1);
            shutdown.begin_drain();
            assert!(shutdown.background().is_none());
            f.provider.resume_expiry.notify_one();
        }
    );
    result.unwrap();
    assert_eq!(f.provider.expiry_posts.load(Ordering::SeqCst), 1);
    let current =
        f.db.collection::<Document>(super::BILLING_WALLET)
            .find_one(doc! {"_id":"wallet-1"})
            .await
            .unwrap()
            .unwrap();
    let active = current.get_document("active_topup_expiry").unwrap();
    assert!(active.get_datetime("dispatch_started_at").is_ok());
    assert!(active.get_datetime("unresolved_reported_at").is_ok());
    assert_eq!(
        active.get_datetime("processing_lease_until").unwrap(),
        &bson::DateTime::from_millis(0)
    );
    let next =
        f.db.collection::<Document>(super::BILLING_WALLET)
            .find_one(doc! {"_id":"wallet-2"})
            .await
            .unwrap()
            .unwrap();
    let active = next.get_document("active_topup_expiry").unwrap();
    assert_eq!(active.get_str("processing_token"), Ok("prior-b"));
    assert!(!active.contains_key("dispatch_started_at"));
    assert_eq!(shutdown.tracked_units(), 0);
    let peer = lease_runtime("expiry-peer", 30_000);
    let token = peer
        .acquire(&f.db, super::RECONCILE_LEASE)
        .await
        .unwrap()
        .unwrap();
    assert!(
        crate::services::coordination_service::LeaseStore::release(&f.db, &token)
            .await
            .unwrap()
    );
    f.db.drop().await.unwrap();
}

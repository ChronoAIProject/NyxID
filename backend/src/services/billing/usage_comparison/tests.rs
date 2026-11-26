use super::super::lago_client::{
    Entitlement, LagoAck, LagoError, LagoEvent, LagoUsage, OwnerProvisionInput,
};
use super::*;
use crate::services::coordination_service::{LeaseStore, cluster_lease_runtime};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

#[derive(Clone)]
enum Reply {
    Usage(serde_json::Value),
    Error,
    Wait,
}
#[derive(Clone)]
enum Hook {
    Update(Database, &'static str, Document, Document),
    Clock(bson::DateTime),
}

#[derive(Default)]
struct Provider {
    replies: Mutex<BTreeMap<String, Reply>>,
    calls: Mutex<Vec<(String, String)>>,
    hooks: Mutex<Vec<Hook>>,
    active: AtomicUsize,
    peak: AtomicUsize,
    barrier: Mutex<Option<Arc<tokio::sync::Barrier>>>,
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
    async fn current_usage(&self, customer: &str, subscription: &str) -> AppResult<LagoUsage> {
        super::super::lago_client::require_subscription_id(subscription)?;
        self.calls
            .lock()
            .unwrap()
            .push((customer.into(), subscription.into()));
        let hooks = std::mem::take(&mut *self.hooks.lock().unwrap());
        for hook in hooks {
            match hook {
                Hook::Update(db, collection, filter, update) => {
                    db.collection::<Document>(collection)
                        .update_one(filter, update)
                        .await
                        .unwrap();
                }
                Hook::Clock(now) => TEST_NOW.with(|clock| clock.set(now)),
            }
        }
        let active = self.active.fetch_add(1, Ordering::SeqCst) + 1;
        self.peak.fetch_max(active, Ordering::SeqCst);
        let barrier = self.barrier.lock().unwrap().clone();
        if let Some(barrier) = barrier {
            barrier.wait().await;
        }
        self.active.fetch_sub(1, Ordering::SeqCst);
        match self
            .replies
            .lock()
            .unwrap()
            .get(subscription)
            .cloned()
            .expect("strict fixture subscription")
        {
            Reply::Usage(raw) => Ok(LagoUsage {
                customer_id: customer.into(),
                subscription_id: subscription.into(),
                raw,
            }),
            Reply::Error => Err(AppError::BillingProviderUnavailable(
                "fixture provider failure".into(),
            )),
            Reply::Wait => Err(AppError::RateLimited),
        }
    }
    async fn wallet_balance(&self, _: &str) -> AppResult<i64> {
        unreachable!()
    }
    async fn entitlements(&self, _: &str) -> AppResult<Vec<Entitlement>> {
        unreachable!()
    }
}

fn at(text: &str) -> bson::DateTime {
    bson::DateTime::from_chrono(
        chrono::DateTime::parse_from_rfc3339(text)
            .unwrap()
            .with_timezone(&Utc),
    )
}
fn initial_now() -> bson::DateTime {
    at("2026-11-15T12:00:00Z")
}
fn usage(units: &str) -> serde_json::Value {
    serde_json::json!({"customer_usage":{
        "from_datetime":"2026-11-01T00:00:00Z","to_datetime":"2026-11-30T23:59:59Z",
        "charges_usage":[{"billable_metric":{"code":"requests","aggregation_type":"sum_agg"},"units":units}]
    }})
}
struct Fixture {
    db: Database,
    provider: Provider,
    commands: Arc<Mutex<Vec<(String, Document)>>>,
}
impl Fixture {
    async fn new() -> Self {
        let commands = Arc::new(Mutex::new(Vec::new()));
        let recorded = commands.clone();
        let db = crate::test_utils::connect_transaction_test_database_with_command_handler(
            "usage_comparison",
            mongodb::event::EventHandler::callback(move |event| {
                if let mongodb::event::command::CommandEvent::Started(event) = event {
                    recorded
                        .lock()
                        .unwrap()
                        .push((event.command_name, event.command));
                }
            }),
        )
        .await;
        super::super::usage_delivery::ensure_indexes(&db)
            .await
            .unwrap();
        crate::services::security_readiness_service::ready_test_replica(&db).await;
        crate::services::security_readiness_service::stop_test_replica(&db).await;
        // These tests advance a logical clock; the real TTL monitor must not erase its history.
        db.collection::<Document>(crate::models::security_replica::COLLECTION_NAME)
            .drop_index("security_replica_observations")
            .await
            .unwrap();
        db.collection::<Document>(CONTROL_COLLECTION).insert_one(doc! {"_id":COVERAGE_ID,"ready":true,
            "generation":uuid::Uuid::new_v4().to_string(),"continuous_since":at("2026-10-01T00:00:00Z"),"observed_at":initial_now()}).await.unwrap();
        Self {
            db,
            provider: Provider::default(),
            commands,
        }
    }
    async fn target(&self, id: &str, sequence: i64) {
        self.db.collection::<Document>(crate::models::billing_wallet::COLLECTION_NAME).insert_one(doc! {
            "_id":format!("wallet-{id}"),"owner_id":format!("owner-{id}"),"lago_customer_id":format!("customer-{id}"),"lago_subscription_id":id
        }).await.unwrap();
        self.db.collection::<Document>(COLLECTION_NAME).insert_one(doc! {"_id":id,"owner_id":format!("owner-{id}"),
            "wallet_id":format!("wallet-{id}"),"customer_id":format!("customer-{id}"),"subscription_id":id,
            "acknowledgement_sequence":sequence,"coverage_verified":false,"warming_since":at("2026-10-01T00:00:00Z"),
            "last_acknowledged_at":at("2026-11-15T11:00:00Z")}).await.unwrap();
        self.provider
            .replies
            .lock()
            .unwrap()
            .insert(id.into(), Reply::Usage(usage("1")));
    }
    async fn receipt(&self, id: &str, sequence: i64, second: i64, quantity: &str) {
        let now = initial_now();
        self.db.collection::<Document>(crate::models::billing_usage_delivery::COLLECTION_NAME).insert_one(doc!{
            "_id":uuid::Uuid::new_v4().to_string(),"transaction_id":format!("{id}:{sequence}"),"owner_id":format!("owner-{id}"),
            "wallet_id":format!("wallet-{id}"),"customer_id":format!("customer-{id}"),"subscription_id":id,
            "code":"requests","event_second":second,"properties":{"quantity":quantity},"state":"acknowledged",
            "acknowledgement_sequence":sequence,"prepared_at":now,"resolved_at":now,"acknowledged_at":now,
            "expires_at":bson::DateTime::from_chrono(Utc::now()+chrono::Duration::days(100)),
        }).await.unwrap();
    }
    async fn pass(&self) -> (super::super::reconcile::ReconcileStats, AppResult<()>) {
        let runtime = cluster_lease_runtime();
        let token = LeaseStore::acquire(
            &self.db,
            super::super::reconcile::RECONCILE_LEASE,
            &runtime.holder,
            std::time::Duration::from_secs(365 * 86400),
        )
        .await
        .unwrap()
        .unwrap();
        let mut stats = super::super::reconcile::ReconcileStats::default();
        let result = run(&self.db, &self.provider, &token, &mut stats).await;
        LeaseStore::release(&self.db, &token).await.unwrap();
        (stats, result)
    }
    async fn advance(&self, seconds: i64) {
        let now = TEST_NOW.with(|now| {
            let next = bson::DateTime::from_millis(now.get().timestamp_millis() + seconds * 1000);
            now.set(next);
            next
        });
        self.db
            .collection::<Document>(CONTROL_COLLECTION)
            .update_one(doc! {"_id":COVERAGE_ID}, doc! {"$set":{"observed_at":now}})
            .await
            .unwrap();
    }
    async fn observation(&self, id: &str) -> UsageObservation {
        let row = self
            .db
            .collection::<Document>(COLLECTION_NAME)
            .find_one(doc! {"_id":id})
            .await
            .unwrap()
            .unwrap();
        bson::from_document(row.get_document("comparison").unwrap().clone()).unwrap()
    }
    async fn close(self) {
        self.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn usage_comparison_persistence_and_equality_clear() {
    TEST_NOW
        .scope(std::cell::Cell::new(initial_now()), async {
            let f = Fixture::new().await;
            f.target("sub", 1).await;
            f.receipt(
                "sub",
                1,
                initial_now().to_chrono().timestamp() - 3600,
                "1.25",
            )
            .await;
            let (stats, result) = f.pass().await;
            result.unwrap();
            assert_eq!(stats.drift_alerts, 0);
            assert_eq!(f.observation("sub").await.status, "pending");
            f.advance(300).await;
            let (stats, result) = f.pass().await;
            result.unwrap();
            assert_eq!(stats.drift_alerts, 0);
            f.advance(300).await;
            let (stats, result) = f.pass().await;
            result.unwrap();
            assert_eq!(stats.drift_alerts, 1);
            let observation = f.observation("sub").await;
            assert_eq!(observation.differences["requests"], "+0.25");
            assert!(
                diagnostics(&f.db)
                    .await
                    .unwrap()
                    .iter()
                    .any(|d| d.code == "billing_usage_drift")
            );
            f.provider
                .replies
                .lock()
                .unwrap()
                .insert("sub".into(), Reply::Usage(usage("1.25")));
            f.advance(600).await;
            let (stats, result) = f.pass().await;
            result.unwrap();
            assert_eq!(stats.drift_alerts, 0);
            assert_eq!(f.observation("sub").await.status, "equal");
            assert!(diagnostics(&f.db).await.unwrap().is_empty());
            f.close().await;
        })
        .await;
}

#[tokio::test]
async fn usage_comparison_errors_isolate_and_provider_waits_are_not_failures() {
    TEST_NOW
        .scope(std::cell::Cell::new(initial_now()), async {
            let f = Fixture::new().await;
            for id in ["a", "b", "c"] {
                f.target(id, 0).await;
            }
            f.provider
                .replies
                .lock()
                .unwrap()
                .insert("a".into(), Reply::Error);
            f.provider
                .replies
                .lock()
                .unwrap()
                .insert("b".into(), Reply::Wait);
            f.provider
                .replies
                .lock()
                .unwrap()
                .insert("c".into(), Reply::Usage(usage("0")));
            let (_, result) = f.pass().await;
            assert!(matches!(
                result,
                Err(AppError::BillingRecoveryFailed {
                    failed_units: 1,
                    ..
                })
            ));
            assert_eq!(f.provider.calls.lock().unwrap().len(), 3);
            assert_eq!(f.observation("a").await.status, "error");
            assert_eq!(f.observation("b").await.status, "waiting");
            assert_eq!(f.observation("c").await.status, "equal");
            f.close().await;
        })
        .await;
}

#[tokio::test]
async fn usage_comparison_rotation_limits_and_restart_keep_budget() {
    TEST_NOW
        .scope(std::cell::Cell::new(initial_now()), async {
            let f = Fixture::new().await;
            for n in 0..21 {
                f.target(&format!("sub-{n:02}"), 0).await;
            }
            *f.provider.barrier.lock().unwrap() = Some(Arc::new(tokio::sync::Barrier::new(4)));
            f.pass().await.1.unwrap();
            assert_eq!(f.provider.calls.lock().unwrap().len(), 20);
            assert_eq!(f.provider.peak.load(Ordering::SeqCst), 4);
            *f.provider.barrier.lock().unwrap() = None;
            f.pass().await.1.unwrap();
            assert_eq!(
                f.provider.calls.lock().unwrap().len(),
                20,
                "new lease cannot reset persisted limit"
            );
            f.advance(300).await;
            f.pass().await.1.unwrap();
            assert_eq!(f.provider.calls.lock().unwrap().len(), 21);
            assert!(
                f.provider
                    .calls
                    .lock()
                    .unwrap()
                    .iter()
                    .any(|(_, s)| s == "sub-20")
            );
            f.advance(300).await;
            f.db.collection::<Document>(CONTROL_COLLECTION)
                .update_one(
                    doc! {"_id":SCHEDULE_ID},
                    doc! {"$set":{"day_reads":5759_i64}},
                )
                .await
                .unwrap();
            f.pass().await.1.unwrap();
            assert_eq!(f.provider.calls.lock().unwrap().len(), 22);
            f.advance(300).await;
            f.pass().await.1.unwrap();
            assert_eq!(
                f.provider.calls.lock().unwrap().len(),
                22,
                "daily cap survives another pass"
            );
            f.close().await;
        })
        .await;
}

#[path = "cases.rs"]
mod cases;

#[path = "round2_tests.rs"]
mod round2_tests;

#[path = "round3_tests.rs"]
mod round3_tests;

#[path = "review_tests.rs"]
mod review_tests;

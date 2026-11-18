//! Real MongoDB and Lago's full-array replacement semantics, with controllable reads/writes.
use super::{BillingService, lago_client::LagoClient, pricing};
use crate::models::downstream_service::{
    COLLECTION_NAME as SERVICES, DownstreamService, test_helpers::dummy_service,
};
use crate::models::service_billing::{PricingSyncStatus, ServiceBilling, ServicePlatformPricing};
use crate::services::coordination_service::{LeaseStore, cluster_lease_runtime};
use axum::{
    Json,
    extract::{Path, State},
    routing::{get, post},
};
use mongodb::{
    Database,
    bson::{Document, doc},
};
use serde_json::{Value, json};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, AtomicUsize, Ordering},
};
use std::time::Duration;
use tokio::sync::{Notify, Semaphore};
use uuid::Uuid;

struct Gate {
    armed: AtomicBool,
    entered: Notify,
    release: Semaphore,
}
impl Default for Gate {
    fn default() -> Self {
        Self {
            armed: AtomicBool::new(false),
            entered: Notify::new(),
            release: Semaphore::new(0),
        }
    }
}
impl Gate {
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
        tokio::time::timeout(Duration::from_secs(10), self.entered.notified())
            .await
            .unwrap();
    }
    fn resume(&self) {
        self.release.add_permits(1);
    }
}
#[derive(Default)]
struct Provider {
    charges: Mutex<Vec<Value>>,
    updates: Mutex<Vec<Value>>,
    get_gate: Gate,
    put_gate: Gate,
    tax_gate: Gate,
    tax_reads: AtomicUsize,
    tax_rate: AtomicUsize,
    ignore_put: AtomicBool,
    wrong_flags: AtomicBool,
    stuck_codes: Mutex<Vec<String>>,
}
impl Provider {
    fn codes(&self) -> Vec<String> {
        let mut codes: Vec<_> = self
            .charges
            .lock()
            .unwrap()
            .iter()
            .map(|c| c["billable_metric_code"].as_str().unwrap().to_owned())
            .collect();
        codes.sort();
        codes
    }
}
fn plan(charges: Vec<Value>) -> Value {
    json!({"plan":{"code":"standard","name":"Standard","interval":"monthly","amount_cents":0,"amount_currency":"USD","pay_in_advance":false,"tax_codes":["keep-provider-tax"],"charges":charges}})
}
async fn get_plan(State(p): State<Arc<Provider>>) -> Json<Value> {
    let snapshot = p.charges.lock().unwrap().clone();
    p.get_gate.pause().await;
    Json(plan(snapshot))
}
async fn put_plan(State(p): State<Arc<Provider>>, Json(body): Json<Value>) -> Json<Value> {
    assert!(body["plan"].get("tax_codes").is_none());
    p.updates.lock().unwrap().push(body.clone());
    // An accepted provider job may finish after the HTTP caller disconnects.
    let (send, receive) = tokio::sync::oneshot::channel();
    crate::services::shutdown::process().spawn_worker(async move {
        let response = apply_plan(p, body).await;
        let _ = send.send(response);
    });
    Json(receive.await.unwrap())
}
async fn apply_plan(p: Arc<Provider>, body: Value) -> Value {
    p.put_gate.pause().await;
    if !p.ignore_put.load(Ordering::SeqCst) {
        let stored: Vec<_> = body["plan"]["charges"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| {
                assert!(c.get("tax_codes").is_none());
                let mut c = c.clone();
                let code = c["billable_metric_code"]
                    .as_str()
                    .or_else(|| c["billable_metric_id"].as_str())
                    .unwrap()
                    .to_owned();
                c["billable_metric_code"] = json!(code);
                c["lago_id"] = c
                    .get("id")
                    .cloned()
                    .unwrap_or_else(|| json!(format!("charge-{code}")));
                if p.wrong_flags.load(Ordering::SeqCst) {
                    c["invoiceable"] = json!(false);
                }
                if p.stuck_codes.lock().unwrap().contains(&code) {
                    c["properties"]["amount"] = json!("99");
                }
                c
            })
            .collect();
        *p.charges.lock().unwrap() = stored;
    }
    plan(p.charges.lock().unwrap().clone())
}
async fn metric(Path(code): Path<String>) -> Json<Value> {
    Json(
        json!({"billable_metric":{"lago_id":code,"code":code,"aggregation_type":"sum_agg","field_name":"quantity"}}),
    )
}
async fn update_metric() -> Json<Value> {
    Json(json!({"billable_metric":{}}))
}
async fn create_metric(Json(body): Json<Value>) -> Json<Value> {
    metric(Path(
        body["billable_metric"]["code"].as_str().unwrap().to_owned(),
    ))
    .await
}
struct Fixture {
    db: Database,
    provider: Arc<Provider>,
    billing: BillingService,
    lago: Arc<LagoClient>,
    commands: Arc<AtomicUsize>,
    command_names: Arc<Mutex<Vec<String>>>,
    catalog_documents: Arc<AtomicUsize>,
}
impl Fixture {
    async fn new(name: &str) -> Self {
        let commands = Arc::new(AtomicUsize::new(0));
        let observed = commands.clone();
        let command_names = Arc::new(Mutex::new(Vec::new()));
        let names = command_names.clone();
        let catalog_documents = Arc::new(AtomicUsize::new(0));
        let documents = catalog_documents.clone();
        let db = crate::test_utils::connect_test_database_with_command_handler(
            name,
            mongodb::event::EventHandler::callback(move |event| match event {
                mongodb::event::command::CommandEvent::Started(event) => {
                    observed.fetch_add(1, Ordering::SeqCst);
                    let writes_from_aggregate = event.command_name == "aggregate"
                        && event.command.get_array("pipeline").is_ok_and(|stages| {
                            stages
                                .iter()
                                .filter_map(|stage| stage.as_document())
                                .any(|stage| {
                                    stage.contains_key("$out") || stage.contains_key("$merge")
                                })
                        });
                    names.lock().unwrap().push(if writes_from_aggregate {
                        "aggregate_with_write_stage".into()
                    } else {
                        event.command_name
                    });
                }
                mongodb::event::command::CommandEvent::Succeeded(event) => {
                    if let Ok(cursor) = event.reply.get_document("cursor")
                        && cursor
                            .get_str("ns")
                            .is_ok_and(|ns| ns.ends_with(".downstream_services"))
                    {
                        let count = cursor
                            .get_array("firstBatch")
                            .or_else(|_| cursor.get_array("nextBatch"))
                            .map_or(0, Vec::len);
                        documents.fetch_add(count, Ordering::SeqCst);
                    }
                }
                _ => {}
            }),
        )
        .await
        .unwrap();
        let provider = Arc::new(Provider::default());
        let app = axum::Router::new()
            .route("/api/v1/plans/standard", get(get_plan).put(put_plan))
            .route(
                "/api/v1/billable_metrics/{code}",
                get(metric).put(update_metric),
            )
            .route("/api/v1/billable_metrics", post(create_metric))
            .route("/api/v1/customers/owner", get(tax_admission::customer))
            .route(
                "/api/v1/subscriptions/sub",
                get(tax_admission::subscription),
            )
            .route(
                "/api/v1/billing_entities/entity",
                get(tax_admission::entity),
            )
            .route(
                "/api/v1/subscriptions/sub/entitlements",
                get(tax_admission::entitlements),
            )
            .with_state(provider.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        crate::services::shutdown::process().spawn_worker(async move {
            axum::serve(listener, app).await.unwrap();
        });
        let lago =
            Arc::new(LagoClient::new(format!("http://{address}"), "test-key".into()).unwrap());
        let mut config = crate::test_utils::test_app_config();
        config.lago_plan_code = "standard".into();
        let billing = BillingService::new_with_lago(db.clone(), Arc::new(config), lago.clone());
        Self {
            db,
            provider,
            billing,
            lago,
            commands,
            command_names,
            catalog_documents,
        }
    }
    async fn service(&self, slug: &str) -> DownstreamService {
        let mut s = dummy_service();
        s.id = Uuid::new_v4().to_string();
        s.slug = slug.into();
        s.billing = Some(ServiceBilling {
            platform_pricing: Some(ServicePlatformPricing {
                lago_metric_code: format!("platform_svc_{slug}"),
                credits_per_unit: "1".into(),
                sync_status: PricingSyncStatus::Pending,
                sync_error: None,
            }),
            ..Default::default()
        });
        self.db
            .collection::<DownstreamService>(SERVICES)
            .insert_one(&s)
            .await
            .unwrap();
        s
    }
    async fn status(&self, s: &DownstreamService) -> PricingSyncStatus {
        self.db
            .collection::<DownstreamService>(SERVICES)
            .find_one(doc! {"_id":&s.id})
            .await
            .unwrap()
            .unwrap()
            .billing
            .unwrap()
            .platform_pricing
            .unwrap()
            .sync_status
    }
    async fn retry(&self) {
        pricing::retry_pending_service_prices(&self.db, self.lago.as_ref(), "standard")
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn concurrent_service_writers_preserve_both_charges() {
    let f = Fixture::new("plan_concurrent").await;
    let a = f.service("a").await;
    let b = f.service("b").await;
    f.provider.get_gate.arm();
    let (a_result, ()) = tokio::join!(f.billing.sync_service_price(&a), async {
        f.provider.get_gate.entered().await;
        f.billing.sync_service_price(&b).await.unwrap();
        f.provider.get_gate.resume();
    });
    a_result.unwrap();
    f.retry().await;
    assert_eq!(
        f.provider.codes(),
        ["platform_svc_a", "platform_svc_b"],
        "a stale full-array PUT dropped the other synced price"
    );
    assert_eq!(f.status(&a).await, PricingSyncStatus::Synced);
    assert_eq!(f.status(&b).await, PricingSyncStatus::Synced);
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn concurrent_add_and_remove_do_not_resurrect_removed_charge() {
    let f = Fixture::new("plan_add_remove").await;
    let a = f.service("a").await;
    let mut b = f.service("b").await;
    f.billing.sync_service_price(&b).await.unwrap();
    b.billing.as_mut().unwrap().platform_pricing = None;
    b.billing
        .as_mut()
        .unwrap()
        .platform_pricing_cleanup_metric_code = Some("platform_svc_b".into());
    f.db.collection::<DownstreamService>(SERVICES)
        .replace_one(doc! {"_id":&b.id}, &b)
        .await
        .unwrap();
    f.provider.get_gate.arm();
    let (result, ()) = tokio::join!(f.billing.sync_service_price(&a), async {
        f.provider.get_gate.entered().await;
        f.billing.sync_service_price(&b).await.unwrap();
        f.provider.get_gate.resume();
    });
    result.unwrap();
    f.retry().await;
    assert_eq!(
        f.provider.codes(),
        ["platform_svc_a"],
        "an older add resurrected a removed charge after cleanup completed"
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn request_contending_for_plan_stays_pending_then_retries() {
    let f = Fixture::new("plan_contended").await;
    let a = f.service("a").await;
    let held = cluster_lease_runtime()
        .acquire(&f.db, "billing:plan:standard")
        .await
        .unwrap()
        .unwrap();
    f.commands.store(0, Ordering::SeqCst);
    let changed = f.billing.sync_service_price(&a).await.unwrap();
    println!(
        "contended request MongoDB commands: {}",
        f.commands.load(Ordering::SeqCst)
    );
    assert!(
        !changed,
        "contended request must return saved pending state"
    );
    assert_eq!(f.status(&a).await, PricingSyncStatus::Pending);
    assert!(f.provider.updates.lock().unwrap().is_empty());
    LeaseStore::release(&f.db, &held).await.unwrap();
    f.retry().await;
    assert_eq!(f.status(&a).await, PricingSyncStatus::Synced);
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn late_put_after_takeover_repairs_already_synced_charge() {
    let f = Fixture::new("plan_late_put").await;
    let a = f.service("a").await;
    let b = f.service("b").await;
    f.provider.put_gate.arm();
    let (result, ()) = tokio::join!(f.billing.sync_service_price(&a), async {
        f.provider.put_gate.entered().await;
        f.db.collection::<Document>("coordination_leases")
            .update_one(
                doc! {"_id":"billing:plan:standard"},
                doc! {"$set":{"expires_at":mongodb::bson::DateTime::from_millis(0)}},
            )
            .await
            .unwrap();
        assert!(f.billing.sync_service_price(&b).await.unwrap());
        assert_eq!(f.status(&b).await, PricingSyncStatus::Synced);
        f.provider.put_gate.resume();
    });
    result.unwrap();
    f.retry().await;
    assert_eq!(
        f.provider.codes(),
        ["platform_svc_a", "platform_svc_b"],
        "pending-only retry cannot repair a lost synced charge"
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn successful_put_with_mismatched_readback_never_marks_synced() {
    let f = Fixture::new("plan_readback").await;
    let a = f.service("a").await;
    f.provider.ignore_put.store(true, Ordering::SeqCst);
    assert!(
        !f.billing.sync_service_price(&a).await.unwrap(),
        "HTTP success is not proof of charge installation"
    );
    assert_eq!(f.status(&a).await, PricingSyncStatus::Pending);
    assert_eq!(
        f.db.collection::<Document>("billing_rate_cache")
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    f.provider.ignore_put.store(false, Ordering::SeqCst);
    f.provider.wrong_flags.store(true, Ordering::SeqCst);
    f.db.collection::<Document>("coordination_leases")
        .update_one(
            doc! {"_id":"billing:plan:standard"},
            doc! {"$set":{"expires_at":mongodb::bson::DateTime::from_millis(0)}},
        )
        .await
        .unwrap();
    assert!(
        !f.billing.sync_service_price(&a).await.unwrap(),
        "matching amount alone is not the full desired charge"
    );
    assert_eq!(f.status(&a).await, PricingSyncStatus::Pending);
    assert_eq!(
        f.provider.updates.lock().unwrap().len(),
        2,
        "both readback mismatches were exercised"
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn healthy_plan_request_command_count() {
    let f = Fixture::new("plan_commands").await;
    let a = f.service("a").await;
    f.commands.store(0, Ordering::SeqCst);
    assert!(f.billing.sync_service_price(&a).await.unwrap());
    println!(
        "healthy single-price request MongoDB commands: {}",
        f.commands.load(Ordering::SeqCst)
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn machine_price_request_contending_for_plan_preserves_pending() {
    let f = Fixture::new("plan_runtime_contended").await;
    let rate = f
        .billing
        .stage_initial_machine_runtime_rate()
        .await
        .unwrap();
    let held = cluster_lease_runtime()
        .acquire(&f.db, "billing:plan:standard")
        .await
        .unwrap()
        .unwrap();
    f.commands.store(0, Ordering::SeqCst);
    f.billing.sync_machine_runtime_rate(&rate.id).await.unwrap();
    println!(
        "contended machine-price request MongoDB commands: {}",
        f.commands.load(Ordering::SeqCst)
    );
    assert_eq!(
        f.db.collection::<crate::models::machine_runtime_billing::MachineRuntimeRate>(
            crate::models::machine_runtime_billing::RATE_COLLECTION
        )
        .find_one(doc! {"_id":&rate.id})
        .await
        .unwrap()
        .unwrap()
        .sync_status,
        PricingSyncStatus::Pending
    );
    assert!(f.provider.updates.lock().unwrap().is_empty());
    LeaseStore::release(&f.db, &held).await.unwrap();
    f.billing.sync_machine_runtime_rate(&rate.id).await.unwrap();
    assert_eq!(
        f.db.collection::<crate::models::machine_runtime_billing::MachineRuntimeRate>(
            crate::models::machine_runtime_billing::RATE_COLLECTION
        )
        .find_one(doc! {"_id":&rate.id})
        .await
        .unwrap()
        .unwrap()
        .sync_status,
        PricingSyncStatus::Synced
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn healthy_machine_price_request_command_count() {
    let f = Fixture::new("plan_runtime_commands").await;
    let rate = f
        .billing
        .stage_initial_machine_runtime_rate()
        .await
        .unwrap();
    f.commands.store(0, Ordering::SeqCst);
    f.billing.sync_machine_runtime_rate(&rate.id).await.unwrap();
    println!(
        "healthy machine-price request MongoDB commands: {}",
        f.commands.load(Ordering::SeqCst)
    );
    assert_eq!(
        f.db.collection::<crate::models::machine_runtime_billing::MachineRuntimeRate>(
            crate::models::machine_runtime_billing::RATE_COLLECTION
        )
        .find_one(doc! {"_id":&rate.id})
        .await
        .unwrap()
        .unwrap()
        .sync_status,
        PricingSyncStatus::Synced
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn cancelled_request_late_put_keeps_dirty_plan_repairable() {
    let f = Fixture::new("plan_cancelled_put").await;
    let a = f.service("a").await;
    let b = f.service("b").await;
    f.provider.put_gate.arm();
    let mut request = Box::pin(f.billing.sync_service_price(&a));
    tokio::select! {
        result=&mut request => panic!("request finished before held PUT: {result:?}"),
        ()=f.provider.put_gate.entered()=>{}
    }
    drop(request);
    let state =
        f.db.collection::<Document>("billing_plan_sync")
            .find_one(doc! {"_id":"standard"})
            .await
            .unwrap()
            .unwrap();
    assert!(state.get_str("active_attempt").is_ok());
    assert!(
        f.db.collection::<Document>("coordination_leases")
            .find_one(doc! {"_id":"billing:plan:standard"})
            .await
            .unwrap()
            .is_some(),
        "cancellation must leave the lease to TTL"
    );
    f.db.collection::<Document>("coordination_leases")
        .update_one(
            doc! {"_id":"billing:plan:standard"},
            doc! {"$set":{"expires_at":mongodb::bson::DateTime::from_millis(0)}},
        )
        .await
        .unwrap();
    assert!(f.billing.sync_service_price(&b).await.unwrap());
    // Complete recovery before the old provider job arrives. Both local prices
    // are now synced and dirty is clear; the next clean-plan audit must still
    // find the late overwrite because there is no pending price to select.
    f.retry().await;
    super::plan_sync::repair(&f.db, f.lago.as_ref(), "standard", "standard")
        .await
        .unwrap();
    assert_eq!(f.status(&a).await, PricingSyncStatus::Synced);
    assert_eq!(f.status(&b).await, PricingSyncStatus::Synced);
    let verified =
        f.db.collection::<Document>("billing_plan_sync")
            .find_one(doc! {"_id":"standard"})
            .await
            .unwrap()
            .unwrap();
    assert_eq!(verified.get_bool("dirty"), Ok(false));
    f.provider.put_gate.resume();
    tokio::time::timeout(Duration::from_secs(5), async {
        while f.provider.codes() != ["platform_svc_a"] {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    f.retry().await;
    assert_eq!(f.provider.codes(), ["platform_svc_a", "platform_svc_b"]);
    let state =
        f.db.collection::<Document>("billing_plan_sync")
            .find_one(doc! {"_id":"standard"})
            .await
            .unwrap()
            .unwrap();
    assert_eq!(state.get_bool("dirty"), Ok(false));
    assert_eq!(state.get_bool("uncertain"), Ok(true));
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn plan_lease_renews_through_provider_and_cache_unit() {
    let f = Fixture::new("plan_renewal").await;
    let mut runtime = cluster_lease_runtime().clone();
    runtime.ttl = Duration::from_secs(2);
    runtime.renew_interval = Duration::from_millis(200);
    let input = super::lago_client::ServicePriceSync {
        metric_code: "unit".into(),
        metric_name: "Unit".into(),
        metric_description: "Unit".into(),
        credits_per_unit: "1".into(),
    };
    let cache = Gate::default();
    cache.arm();
    f.provider.put_gate.arm();
    let (result, ()) = tokio::join!(
        super::plan_sync::run_with_runtime(
            &f.db,
            "standard",
            None,
            super::plan_sync::CacheWrite::Rate,
            &runtime,
            async {
                super::taxes::sync_charge(&f.db, f.lago.as_ref(), "standard", &input).await?;
                cache.pause().await;
                f.db.collection::<Document>("cache_unit_probe")
                    .insert_one(doc! {"_id":"cache committed"})
                    .await?;
                Ok(())
            }
        ),
        async {
            f.provider.put_gate.entered().await;
            tokio::time::sleep(Duration::from_secs(3)).await;
            assert!(
                cluster_lease_runtime()
                    .acquire(&f.db, "billing:plan:standard")
                    .await
                    .unwrap()
                    .is_none(),
                "lease must renew while PUT is held past its initial TTL"
            );
            f.provider.put_gate.resume();
            cache.entered().await;
            assert!(
                cluster_lease_runtime()
                    .acquire(&f.db, "billing:plan:standard")
                    .await
                    .unwrap()
                    .is_none(),
                "lease must still be held before cache publication"
            );
            cache.resume();
        }
    );
    assert_eq!(result.unwrap(), Some(()));
    assert!(
        f.db.collection::<Document>("coordination_leases")
            .find_one(doc! {"_id":"billing:plan:standard"})
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        f.db.collection::<Document>("cache_unit_probe")
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn lost_holder_does_not_regress_successors_synced_state() {
    let f = Fixture::new("plan_stale_status").await;
    let a = f.service("a").await;
    f.provider.put_gate.arm();
    let (old, ()) = tokio::join!(f.billing.sync_service_price(&a), async {
        f.provider.put_gate.entered().await;
        f.db.collection::<Document>("coordination_leases")
            .update_one(
                doc! {"_id":"billing:plan:standard"},
                doc! {"$set":{"expires_at":mongodb::bson::DateTime::from_millis(0)}},
            )
            .await
            .unwrap();
        assert!(f.billing.sync_service_price(&a).await.unwrap());
        f.provider.put_gate.resume();
    });
    assert!(!old.unwrap());
    assert_eq!(f.status(&a).await, PricingSyncStatus::Synced);
    assert_eq!(
        f.db.collection::<Document>("billing_plan_charges")
            .count_documents(doc! {"plan_code":"standard","metric_code":"platform_svc_a"})
            .await
            .unwrap(),
        1
    );
    f.db.drop().await.unwrap();
}

pub(super) async fn expire_plan_lease(db: &Database, plan: &str) {
    db.collection::<Document>("coordination_leases")
        .update_one(
            doc! {"_id":format!("billing:plan:{plan}")},
            doc! {"$set":{"expires_at":mongodb::bson::DateTime::from_millis(0)}},
        )
        .await
        .unwrap();
}

async fn interrupted_resync_repairs_invalidation(machine: bool) {
    let f = Fixture::new(if machine {
        "plan_runtime_resync"
    } else {
        "plan_service_resync"
    })
    .await;
    let a = f.service("a").await;
    f.billing.sync_service_price(&a).await.unwrap();
    let a =
        f.db.collection::<DownstreamService>(SERVICES)
            .find_one(doc! {"_id":&a.id})
            .await
            .unwrap()
            .unwrap();
    let runtime = if machine {
        let rate = f
            .billing
            .stage_initial_machine_runtime_rate()
            .await
            .unwrap();
        assert!(f.billing.sync_machine_runtime_rate(&rate.id).await.unwrap());
        Some(rate)
    } else {
        None
    };
    // No pending price can drive recovery of this explicit resync.
    assert_eq!(f.status(&a).await, PricingSyncStatus::Synced);
    f.db.collection::<Document>("billing_tax_profiles")
        .insert_one(doc! {
            "_id":"tax-probe", "plan_code":"standard", "fetched_at":mongodb::bson::DateTime::now()
        })
        .await
        .unwrap();
    f.provider.put_gate.arm();
    let mut request = Box::pin(async {
        if let Some(rate) = &runtime {
            f.billing.sync_machine_runtime_rate(&rate.id).await
        } else {
            f.billing.sync_service_price(&a).await
        }
    });
    tokio::select! {
        result=&mut request => panic!("resync completed before held PUT: {result:?}"),
        ()=f.provider.put_gate.entered()=>{}
    }
    let state =
        f.db.collection::<Document>("billing_plan_sync")
            .find_one(doc! {"_id":"standard"})
            .await
            .unwrap()
            .unwrap();
    assert_eq!(state.get_bool("dirty"), Ok(true));
    assert!(state.get_str("active_attempt").is_ok());
    // An inline tax fetch completes while the provider attempt is in flight.
    f.db.collection::<Document>("billing_tax_profiles")
        .update_one(
            doc! {"_id":"tax-probe"},
            doc! {"$set":{"fetched_at":mongodb::bson::DateTime::now()}},
        )
        .await
        .unwrap();
    // Simulate an older cache publisher: recovery must restore verified data,
    // even though this source was already synced before resync started.
    f.db.collection::<Document>("billing_rate_cache")
        .update_one(
            doc! {"lago_metric_code":"platform_svc_a"},
            doc! {"$set":{"credits_per_unit_pico":0_i64,"credits_per_unit_micros":0_i64}},
        )
        .await
        .unwrap();
    drop(request);
    expire_plan_lease(&f.db, "standard").await;
    // The matching provider plan must not short-circuit tax/cache completion.
    f.retry().await;
    let tax =
        f.db.collection::<Document>("billing_tax_profiles")
            .find_one(doc! {"_id":"tax-probe"})
            .await
            .unwrap()
            .unwrap();
    assert_eq!(
        tax.get_datetime("fetched_at").unwrap().timestamp_millis(),
        0,
        "dirty recovery must replay the interrupted after-invalidation even with no pending prices"
    );
    let cache =
        f.db.collection::<Document>("billing_rate_cache")
            .find_one(doc! {"lago_metric_code":"platform_svc_a"})
            .await
            .unwrap()
            .unwrap();
    assert_eq!(
        cache.get_i64("credits_per_unit_pico"),
        Ok(1_000_000_000_000)
    );
    let state =
        f.db.collection::<Document>("billing_plan_sync")
            .find_one(doc! {"_id":"standard"})
            .await
            .unwrap()
            .unwrap();
    assert_eq!(state.get_bool("dirty"), Ok(false));
    if let Some(rate) = runtime {
        assert_eq!(
            f.db.collection::<Document>("billing_rate_cache")
                .count_documents(doc! {"lago_metric_code":rate.lago_metric_code})
                .await
                .unwrap(),
            0,
            "machine sync retains its existing cache behavior"
        );
    }
    f.provider.put_gate.resume();
    // Allow the deliberately detached mock provider job to finish before drop.
    tokio::time::sleep(Duration::from_millis(50)).await;
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn already_synced_service_resync_retries_dirty_tax_and_cache_completion() {
    Box::pin(interrupted_resync_repairs_invalidation(false)).await;
}

#[tokio::test]
async fn already_synced_machine_resync_retries_dirty_tax_and_cache_completion() {
    Box::pin(interrupted_resync_repairs_invalidation(true)).await;
}

#[path = "plan_sync_audit_tests.rs"]
mod audit;

#[path = "plan_writer_tax_tests.rs"]
mod tax_admission;

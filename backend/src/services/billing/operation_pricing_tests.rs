//! Operation prices: route selection, normalization, Lago charge lifecycle,
//! plan round-trips and allowance funding.
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use futures::TryStreamExt;
use mongodb::bson::{Document, doc};
use serde_json::{Value, json};
use uuid::Uuid;

use super::lago_client::{
    Entitlement, LagoAck, LagoApi, LagoClient, LagoError, LagoEvent, LagoUsage,
    OwnerProvisionInput, ServicePriceSync,
};
use super::{BillingIngress, BillingRouteContext, NodeIntent, pricing};
use crate::errors::{AppError, AppResult};
use crate::models::billing_rate_cache::{BillingRateCache, COLLECTION_NAME as RATES};
use crate::models::downstream_service::{
    COLLECTION_NAME as CATALOG, DownstreamService, test_helpers::dummy_service,
};
use crate::models::service_billing::{
    BillingMetric, LanePriceComponent, LanePricing, OperationPrice, PricingSyncStatus,
    ServiceBilling,
};
use crate::models::usage_meter::CredentialClass;

use PricingSyncStatus::{Failed, Pending, Synced};

fn price(operation: &str, credits: &str, status: PricingSyncStatus) -> OperationPrice {
    OperationPrice {
        operation: operation.into(),
        credits_per_unit: credits.into(),
        lago_metric_code: format!("op_{operation}"),
        sync_status: status,
        sync_error: None,
    }
}

fn lane(status: PricingSyncStatus, operations: Vec<OperationPrice>) -> LanePricing {
    LanePricing {
        metric: BillingMetric::Requests,
        credits_per_unit: "1".into(),
        lago_metric_code: "base".into(),
        sync_status: status,
        sync_error: None,
        components: vec![],
        operations,
    }
}

fn route(
    billing: &ServiceBilling,
    class: CredentialClass,
    operation: Option<&str>,
) -> BillingRouteContext {
    BillingRouteContext::new(
        BillingIngress::Proxy,
        "request".into(),
        "owner".into(),
        "actor".into(),
        None,
        None,
        Some("catalog".into()),
        Some("service".into()),
        NodeIntent::Direct,
        "bearer".into(),
        class,
        BillingMetric::Requests,
        Some(billing),
        false,
    )
    .with_operation(operation)
}

#[test]
fn synced_operation_price_replaces_only_the_primary_request_rate() {
    let mut byok = lane(
        Synced,
        vec![
            price("search", "3", Synced),
            price("lookup", "0", Synced),
            price("export", "9", Pending),
            price("import", "9", Failed),
        ],
    );
    byok.components.push(LanePriceComponent {
        metric: BillingMetric::Bytes,
        credits_per_unit: "1".into(),
        lago_metric_code: "bytes".into(),
        sync_status: Synced,
        sync_error: None,
    });
    let billing = ServiceBilling {
        byok_pricing: Some(byok),
        platform_key_pricing: Some(LanePricing {
            lago_metric_code: "pk".into(),
            ..lane(Synced, vec![])
        }),
        ..Default::default()
    };
    let selected = route(&billing, CredentialClass::UserOwned, Some("search"));
    assert!(selected.prices_operations());
    assert_eq!(selected.operation.as_deref(), Some("search"));
    assert_eq!(selected.platform_metric, BillingMetric::Requests);
    // Additive components keep their own rows.
    assert_eq!(
        selected
            .platform_specs()
            .map(|(_, code)| code)
            .collect::<Vec<_>>(),
        ["op_search", "bytes"]
    );
    // An explicit zero is a price that makes the operation free.
    let free = route(&billing, CredentialClass::UserOwned, Some("lookup"));
    assert_eq!(free.platform_lago_metric_code, "op_lookup");
    for fallback in [Some("export"), Some("import"), Some("unknown"), None] {
        let base = route(&billing, CredentialClass::UserOwned, fallback);
        assert_eq!(base.platform_lago_metric_code, "base");
        assert_eq!(base.operation, None);
        assert!(base.service_platform_billable);
    }
    // Each lane is priced independently.
    let master = route(
        &billing,
        CredentialClass::NyxidManagedMaster,
        Some("search"),
    );
    assert!(!master.prices_operations());
    assert_eq!(master.platform_lago_metric_code, "pk");
}

#[test]
fn unsynced_primary_and_non_request_lanes_ignore_operation_prices() {
    let pending = ServiceBilling {
        platform_billable: true,
        platform_metric: Some(BillingMetric::Requests),
        byok_pricing: Some(lane(Pending, vec![price("search", "3", Synced)])),
        ..Default::default()
    };
    let fallback = route(&pending, CredentialClass::UserOwned, Some("search"));
    assert!(!fallback.prices_operations());
    assert_eq!(fallback.operation, None);
    assert_eq!(
        fallback.platform_lago_metric_code,
        route(&pending, CredentialClass::UserOwned, None).platform_lago_metric_code
    );
    assert_ne!(fallback.platform_lago_metric_code, "op_search");

    let tokens = ServiceBilling {
        byok_pricing: Some(LanePricing {
            metric: BillingMetric::Tokens,
            ..lane(Synced, vec![price("search", "3", Synced)])
        }),
        ..Default::default()
    };
    let unaffected = route(&tokens, CredentialClass::UserOwned, Some("search"));
    assert!(!unaffected.prices_operations());
    assert_eq!(unaffected.platform_lago_metric_code, "base");
}

#[test]
fn credentials_only_restriction_applies_after_operation_selection() {
    let billing = ServiceBilling {
        platform_charge_nyxid_credentials_only: true,
        byok_pricing: Some(lane(Synced, vec![price("search", "3", Synced)])),
        ..Default::default()
    };
    let own_key = route(&billing, CredentialClass::UserOwned, Some("search"));
    assert!(!own_key.service_platform_billable);
    assert!(matches!(
        own_key.require_platform_price(),
        Err(AppError::BillingNotConfigured(_))
    ));
    let shared_app = route(
        &billing,
        CredentialClass::NyxidPlatformOauthApp,
        Some("search"),
    );
    assert!(shared_app.service_platform_billable);
    assert_eq!(shared_app.platform_lago_metric_code, "op_search");
}

fn requested(operation: &str, credits: &str) -> OperationPrice {
    OperationPrice {
        lago_metric_code: String::new(),
        ..price(operation, credits, Pending)
    }
}

#[test]
fn normalization_keeps_unchanged_sync_state_and_queues_removed_codes() {
    let code = |key: &str| format!("platform_svc_svc_byok_op_{key}");
    let synced = |operation: &str, key: &str, credits: &str| OperationPrice {
        lago_metric_code: code(key),
        ..price(operation, credits, Synced)
    };
    let current = ServiceBilling {
        byok_pricing: Some(LanePricing {
            lago_metric_code: "platform_svc_svc_byok".into(),
            ..lane(
                Synced,
                vec![
                    synced("Search", "search", "3"),
                    synced("lookup", "lookup", "2"),
                    synced("export", "export", "1"),
                ],
            )
        }),
        ..Default::default()
    };
    let mut update = ServiceBilling {
        byok_pricing: Some(lane(
            Pending,
            vec![
                requested(" Search ", "3.000"),
                requested("lookup", "2.5"),
                requested("Import-Data", "4"),
            ],
        )),
        ..Default::default()
    };
    pricing::normalize_lane_pricing("svc", Some(&current), &mut update).unwrap();
    let operations = &update.byok_pricing.as_ref().unwrap().operations;
    assert_eq!(operations[0], synced("Search", "search", "3"));
    assert_eq!(
        (
            operations[1].credits_per_unit.as_str(),
            operations[1].sync_status
        ),
        ("2.5", Pending)
    );
    assert_eq!(operations[2].operation, "Import-Data");
    assert_eq!(operations[2].lago_metric_code, code("import_data"));
    assert_eq!(update.component_cleanup_metric_codes, [code("export")]);

    // Removing the lane queues every operation charge and the primary charge.
    let mut cleared = ServiceBilling::default();
    pricing::normalize_lane_pricing("svc", Some(&current), &mut cleared).unwrap();
    assert_eq!(
        cleared.component_cleanup_metric_codes,
        [code("search"), code("lookup"), code("export")]
    );
    assert_eq!(
        cleared.byok_pricing_cleanup_metric_code.as_deref(),
        Some("platform_svc_svc_byok")
    );

    let mut blank = ServiceBilling {
        byok_pricing: Some(lane(Pending, vec![requested("  ", "1")])),
        ..Default::default()
    };
    assert!(matches!(
        pricing::normalize_lane_pricing("svc", None, &mut blank),
        Err(AppError::ValidationError(_))
    ));
}

#[derive(Default)]
struct RecordingLago {
    synced: Mutex<Vec<ServicePriceSync>>,
    removed: Mutex<Vec<String>>,
    fail_removals: AtomicBool,
}

#[async_trait]
impl LagoApi for RecordingLago {
    async fn sync_standard_charge(&self, _plan: &str, input: &ServicePriceSync) -> AppResult<()> {
        self.synced.lock().unwrap().push(input.clone());
        Ok(())
    }
    async fn remove_standard_charge(&self, _plan: &str, code: &str) -> AppResult<()> {
        if self.fail_removals.load(Ordering::SeqCst) {
            return Err(AppError::BillingProviderUnavailable("test".into()));
        }
        self.removed.lock().unwrap().push(code.into());
        Ok(())
    }
    async fn ensure_customer(&self, owner: &OwnerProvisionInput) -> AppResult<String> {
        Ok(owner.external_customer_id.clone())
    }
    async fn ensure_subscription(&self, customer: &str, plan: &str) -> AppResult<String> {
        Ok(format!("{customer}:{plan}"))
    }
    async fn record_event(&self, event: &LagoEvent) -> Result<LagoAck, LagoError> {
        Ok(LagoAck {
            transaction_id: event.transaction_id.clone(),
        })
    }
    async fn record_events_batch(&self, events: &[LagoEvent]) -> Result<Vec<LagoAck>, LagoError> {
        Ok(events
            .iter()
            .map(|event| LagoAck {
                transaction_id: event.transaction_id.clone(),
            })
            .collect())
    }
    async fn current_usage(&self, customer: &str, subscription: &str) -> AppResult<LagoUsage> {
        Ok(LagoUsage {
            customer_id: customer.into(),
            subscription_id: subscription.into(),
            raw: json!({}),
        })
    }
    async fn wallet_balance(&self, _customer: &str) -> AppResult<i64> {
        Ok(100)
    }
    async fn entitlements(&self, _subscription: &str) -> AppResult<Vec<Entitlement>> {
        Ok(vec![Entitlement {
            code: "*".into(),
            raw: json!({}),
        }])
    }
}

/// Normalizes `prices` against the stored service, saves and returns it.
async fn save(
    db: &mongodb::Database,
    catalog: &mut DownstreamService,
    byok: Option<LanePricing>,
) -> DownstreamService {
    let mut billing = ServiceBilling {
        byok_pricing: byok,
        ..Default::default()
    };
    pricing::normalize_lane_pricing(&catalog.slug, catalog.billing.as_ref(), &mut billing).unwrap();
    catalog.billing = Some(billing);
    db.collection::<DownstreamService>(CATALOG)
        .replace_one(doc! {"_id": &catalog.id}, &*catalog)
        .upsert(true)
        .await
        .unwrap();
    catalog.clone()
}

async fn reload(db: &mongodb::Database, catalog: &DownstreamService) -> DownstreamService {
    db.collection::<DownstreamService>(CATALOG)
        .find_one(doc! {"_id": &catalog.id})
        .await
        .unwrap()
        .unwrap()
}

async fn rates(db: &mongodb::Database) -> HashMap<String, (Option<i64>, bool)> {
    db.collection::<BillingRateCache>(RATES)
        .find(doc! {})
        .await
        .unwrap()
        .try_collect::<Vec<_>>()
        .await
        .unwrap()
        .into_iter()
        .map(|rate| {
            (
                rate.lago_metric_code,
                (rate.credits_per_unit_pico, rate.retired_at.is_some()),
            )
        })
        .collect()
}

fn byok(operations: Vec<OperationPrice>) -> Option<LanePricing> {
    Some(LanePricing {
        lago_metric_code: String::new(),
        ..lane(Pending, operations)
    })
}

const PICO: i64 = 1_000_000_000_000;

#[tokio::test]
async fn operation_charges_sync_reprice_retry_cleanup_and_retire_with_their_lane() {
    let db = crate::test_utils::connect_test_database("operation_charge_lifecycle")
        .await
        .expect("MongoDB required");
    let lago = RecordingLago::default();
    let mut catalog = dummy_service();
    catalog.id = Uuid::new_v4().to_string();
    catalog.slug = "svc".into();
    let code = |key: &str| format!("platform_svc_svc_byok_op_{key}");

    // Create: the primary and each operation get their own charge and rate.
    let saved = save(
        &db,
        &mut catalog,
        byok(vec![requested("search", "3"), requested("lookup", "2")]),
    )
    .await;
    assert!(
        pricing::sync_service_price(&db, &lago, "standard", &saved)
            .await
            .unwrap()
    );
    let lane = reload(&db, &catalog)
        .await
        .billing
        .unwrap()
        .byok_pricing
        .unwrap();
    assert_eq!(lane.sync_status, Synced);
    assert!(
        lane.operations
            .iter()
            .all(|price| price.sync_status == Synced)
    );
    let synced: HashMap<_, _> = lago
        .synced
        .lock()
        .unwrap()
        .iter()
        .map(|input| {
            (
                input.metric_code.clone(),
                (input.credits_per_unit.clone(), input.metric_name.clone()),
            )
        })
        .collect();
    assert_eq!(synced[&code("search")].0, "3");
    assert_eq!(synced[&code("lookup")].0, "2");
    assert!(synced[&code("search")].1.contains("search"));
    assert_ne!(synced[&code("search")].1, synced["platform_svc_svc_byok"].1);
    let live = rates(&db).await;
    assert_eq!(live[&code("search")], (Some(3 * PICO), false));
    assert_eq!(live[&code("lookup")], (Some(2 * PICO), false));

    // A failed operation sync is retried by reconciliation on its own.
    db.collection::<Document>(CATALOG)
        .update_one(
            doc! {"_id": &catalog.id},
            doc! {"$set": {"billing.byok_pricing.operations.0.sync_status": "failed"}},
        )
        .await
        .unwrap();
    assert_eq!(
        pricing::retry_pending_service_prices(&db, &lago, "standard")
            .await
            .unwrap(),
        1
    );
    catalog = reload(&db, &catalog).await;
    assert_eq!(
        catalog
            .billing
            .as_ref()
            .unwrap()
            .byok_pricing
            .as_ref()
            .unwrap()
            .operations[0]
            .sync_status,
        Synced
    );

    // Reprice one operation and remove the other while removal is failing.
    lago.fail_removals.store(true, Ordering::SeqCst);
    save(&db, &mut catalog, byok(vec![requested("search", "5")])).await;
    pricing::retry_pending_service_prices(&db, &lago, "standard")
        .await
        .unwrap();
    catalog = reload(&db, &catalog).await;
    let billing = catalog.billing.as_ref().unwrap();
    assert_eq!(billing.component_cleanup_metric_codes, [code("lookup")]);
    assert_eq!(
        billing.byok_pricing.as_ref().unwrap().operations[0].sync_status,
        Synced
    );
    let pending_cleanup = rates(&db).await;
    assert_eq!(pending_cleanup[&code("search")], (Some(5 * PICO), false));
    assert_eq!(pending_cleanup[&code("lookup")], (Some(2 * PICO), false));

    // The durable marker retries until the charge is removed, then retires the rate.
    lago.fail_removals.store(false, Ordering::SeqCst);
    pricing::retry_pending_service_prices(&db, &lago, "standard")
        .await
        .unwrap();
    catalog = reload(&db, &catalog).await;
    assert!(
        catalog
            .billing
            .as_ref()
            .unwrap()
            .component_cleanup_metric_codes
            .is_empty()
    );
    assert_eq!(*lago.removed.lock().unwrap(), [code("lookup")]);
    let retired = rates(&db).await;
    assert_eq!(
        retired[&code("lookup")],
        (Some(2 * PICO), true),
        "history keeps pricing from the retired rate"
    );

    // Removing the whole lane removes and retires its operation charges too.
    save(&db, &mut catalog, None).await;
    pricing::retry_pending_service_prices(&db, &lago, "standard")
        .await
        .unwrap();
    catalog = reload(&db, &catalog).await;
    let billing = catalog.billing.as_ref().unwrap();
    assert!(billing.component_cleanup_metric_codes.is_empty());
    assert!(billing.byok_pricing_cleanup_metric_code.is_none());
    let removed = lago.removed.lock().unwrap().clone();
    assert!(removed.contains(&code("search")));
    assert!(removed.contains(&"platform_svc_svc_byok".to_string()));
    let after = rates(&db).await;
    assert!(after[&code("search")].1 && after["platform_svc_svc_byok"].1);
    db.drop().await.unwrap();
}

#[tokio::test]
async fn stale_operation_sync_marks_live_price_pending_or_queues_removed_code() {
    let db = crate::test_utils::connect_test_database("operation_stale_sync")
        .await
        .expect("MongoDB required");
    let lago = RecordingLago::default();
    let mut catalog = dummy_service();
    catalog.id = Uuid::new_v4().to_string();
    catalog.slug = "svc".into();
    let stale = save(&db, &mut catalog, byok(vec![requested("search", "3")])).await;
    // A newer admin save repriced the operation and synced while this older
    // sync was in flight; its late provider write may have landed last.
    save(&db, &mut catalog, byok(vec![requested("search", "4")])).await;
    db.collection::<Document>(CATALOG)
        .update_one(
            doc! {"_id": &catalog.id},
            doc! {"$set": {"billing.byok_pricing.operations.0.sync_status": "synced"}},
        )
        .await
        .unwrap();
    pricing::sync_service_price(&db, &lago, "standard", &stale)
        .await
        .unwrap();
    let raw = db
        .collection::<Document>(CATALOG)
        .find_one(doc! {"_id": &catalog.id})
        .await
        .unwrap()
        .unwrap();
    let operation = raw
        .get_document("billing")
        .and_then(|billing| billing.get_document("byok_pricing"))
        .and_then(|lane| lane.get_array("operations"))
        .unwrap()[0]
        .as_document()
        .unwrap()
        .clone();
    assert_eq!(operation.get_str("credits_per_unit").unwrap(), "4");
    assert_eq!(operation.get_str("sync_status").unwrap(), "pending");
    assert_eq!(
        operation.keys().collect::<Vec<_>>(),
        [
            "operation",
            "credits_per_unit",
            "lago_metric_code",
            "sync_status"
        ]
    );

    // A newer save removed the operation: the stale charge must be cleaned up.
    let stale = reload(&db, &catalog).await;
    save(&db, &mut catalog, byok(vec![])).await;
    db.collection::<Document>(CATALOG)
        .update_one(
            doc! {"_id": &catalog.id},
            doc! {"$set": {"billing.component_cleanup_metric_codes": []}},
        )
        .await
        .unwrap();
    pricing::sync_service_price(&db, &lago, "standard", &stale)
        .await
        .unwrap();
    assert_eq!(
        reload(&db, &catalog)
            .await
            .billing
            .unwrap()
            .component_cleanup_metric_codes,
        ["platform_svc_svc_byok_op_search"]
    );
    db.drop().await.unwrap();
}

/// A Lago plan endpoint that, like Lago, replaces the whole charges array on
/// update. It records every update so the round-trip can be asserted.
#[derive(Clone, Default)]
struct PlanMock {
    charges: Arc<Mutex<Vec<Value>>>,
    metrics: Arc<Mutex<HashMap<String, String>>>,
    updates: Arc<Mutex<Vec<Vec<Value>>>>,
}

async fn spawn_plan_mock(mock: PlanMock) -> String {
    use axum::extract::{Path, State};
    use axum::routing::{get, post};
    async fn metric(
        State(mock): State<PlanMock>,
        Path(code): Path<String>,
    ) -> Result<axum::Json<Value>, axum::http::StatusCode> {
        let id = mock.metrics.lock().unwrap().get(&code).cloned();
        id.map(|id| {
            axum::Json(json!({"billable_metric": {
                "lago_id": id, "code": code, "aggregation_type": "sum_agg", "field_name": "quantity"
            }}))
        })
        .ok_or(axum::http::StatusCode::NOT_FOUND)
    }
    async fn create_metric(
        State(mock): State<PlanMock>,
        axum::Json(body): axum::Json<Value>,
    ) -> axum::Json<Value> {
        let code = body["billable_metric"]["code"]
            .as_str()
            .unwrap()
            .to_string();
        let id = format!("metric-{code}");
        mock.metrics
            .lock()
            .unwrap()
            .insert(code.clone(), id.clone());
        axum::Json(json!({"billable_metric": {
            "lago_id": id, "code": code, "aggregation_type": "sum_agg", "field_name": "quantity"
        }}))
    }
    async fn update_metric() -> axum::Json<Value> {
        axum::Json(json!({"billable_metric": {}}))
    }
    fn plan(charges: Vec<Value>) -> Value {
        json!({"plan": {
            "lago_id": "plan-id", "name": "Standard", "code": "standard", "interval": "monthly",
            "amount_cents": 2500, "amount_currency": "USD", "pay_in_advance": false,
            "charges": charges,
        }})
    }
    async fn get_plan(State(mock): State<PlanMock>) -> axum::Json<Value> {
        axum::Json(plan(mock.charges.lock().unwrap().clone()))
    }
    async fn update_plan(
        State(mock): State<PlanMock>,
        axum::Json(body): axum::Json<Value>,
    ) -> axum::Json<Value> {
        let sent = body["plan"]["charges"].as_array().unwrap().clone();
        mock.updates.lock().unwrap().push(sent.clone());
        let codes: HashMap<_, _> = mock
            .metrics
            .lock()
            .unwrap()
            .iter()
            .map(|(code, id)| (id.clone(), code.clone()))
            .collect();
        let stored: Vec<Value> = sent
            .into_iter()
            .map(|charge| {
                let id = charge["id"]
                    .as_str()
                    .map_or_else(|| format!("charge-{}", Uuid::new_v4()), str::to_owned);
                let code = charge["billable_metric_code"].as_str().map_or_else(
                    || codes[charge["billable_metric_id"].as_str().unwrap()].clone(),
                    str::to_owned,
                );
                json!({"lago_id": id, "billable_metric_code": code,
                    "charge_model": "standard", "properties": charge["properties"]})
            })
            .collect();
        *mock.charges.lock().unwrap() = stored.clone();
        axum::Json(plan(stored))
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let app = axum::Router::new()
        .route(
            "/api/v1/billable_metrics/{code}",
            get(metric).put(update_metric),
        )
        .route("/api/v1/billable_metrics", post(create_metric))
        .route("/api/v1/plans/standard", get(get_plan).put(update_plan))
        .with_state(mock);
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    format!("http://{address}")
}

#[tokio::test]
async fn operation_charges_round_trip_the_full_plan_charge_array_with_ids() {
    let db = crate::test_utils::connect_test_database("operation_plan_round_trip")
        .await
        .expect("MongoDB required");
    let mock = PlanMock::default();
    *mock.charges.lock().unwrap() = vec![json!({
        "lago_id": "legacy-charge-id", "billable_metric_code": "platform_tokens",
        "charge_model": "standard", "properties": {"amount": "0.000005"},
    })];
    let lago = LagoClient::new(spawn_plan_mock(mock.clone()).await, "test-key".into()).unwrap();
    let mut catalog = dummy_service();
    catalog.id = Uuid::new_v4().to_string();
    catalog.slug = "svc".into();
    let saved = save(
        &db,
        &mut catalog,
        byok(vec![requested("search", "3"), requested("lookup", "0")]),
    )
    .await;
    assert!(
        pricing::sync_service_price(&db, &lago, "standard", &saved)
            .await
            .unwrap()
    );
    // Every update re-sent each existing charge with its id; only the newly
    // added charge had none.
    let updates = mock.updates.lock().unwrap().clone();
    assert_eq!(updates.len(), 3);
    for (index, sent) in updates.iter().enumerate() {
        assert_eq!(sent.len(), index + 2);
        assert_eq!(sent[0]["id"], "legacy-charge-id");
        assert!(sent[..=index].iter().all(|charge| charge["id"].is_string()));
        assert!(sent[index + 1].get("id").is_none());
    }
    let charges = mock.charges.lock().unwrap().clone();
    let amounts: HashMap<_, _> = charges
        .iter()
        .map(|charge| {
            (
                charge["billable_metric_code"].as_str().unwrap().to_string(),
                charge["properties"]["amount"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    assert_eq!(amounts["platform_svc_svc_byok_op_search"], "3");
    assert_eq!(amounts["platform_svc_svc_byok_op_lookup"], "0");
    assert_eq!(amounts["platform_svc_svc_byok"], "1");
    let ids_before: HashMap<_, _> = charges
        .iter()
        .map(|charge| {
            (
                charge["billable_metric_code"].clone(),
                charge["lago_id"].clone(),
            )
        })
        .collect();

    // Removing one operation keeps every other charge and its id. Cleanup
    // runs first; reconciliation then re-sends the live charges in place.
    save(&db, &mut catalog, byok(vec![requested("search", "3")])).await;
    let before = mock.updates.lock().unwrap().len();
    pricing::retry_pending_service_prices(&db, &lago, "standard")
        .await
        .unwrap();
    let after = mock.updates.lock().unwrap()[before..].to_vec();
    assert!(!after.is_empty());
    for sent in &after {
        assert_eq!(sent.len(), 3);
        for charge in sent {
            assert_ne!(
                charge["billable_metric_code"],
                "platform_svc_svc_byok_op_lookup"
            );
            assert_eq!(charge["id"], ids_before[&charge["billable_metric_code"]]);
        }
    }
    db.drop().await.unwrap();
}

#[tokio::test]
async fn request_allowance_covers_an_operation_priced_request() {
    use crate::models::billing_target::BillingTargetKind;
    use crate::models::service_billing::PlatformUsage;
    use crate::models::usage_allowance::AllowanceRecurrence;
    use crate::services::billing::{BillingService, allowances, ledger};
    let db = crate::test_utils::connect_transaction_test_database("operation_allowance").await;
    ledger::init_billing_ledger_hmac_key(zeroize::Zeroizing::new(
        ledger::TEST_BILLING_LEDGER_HMAC_KEY,
    ));
    let owner = Uuid::new_v4().to_string();
    db.collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
        .insert_one(crate::test_utils::test_user(
            &owner,
            crate::models::user::UserType::Person,
        ))
        .await
        .unwrap();
    let now = mongodb::bson::DateTime::now();
    db.collection::<Document>(crate::models::billing_wallet::COLLECTION_NAME)
        .insert_one(doc! {
            "_id": Uuid::new_v4().to_string(), "owner_id": &owner, "lago_customer_id": &owner,
            "lago_wallet_id": "wallet", "lago_subscription_id": "plan", "plan_kind": "prepaid",
            "balance_credits": crate::models::credits::Credits::from_whole(100),
            "reserved_credits": crate::models::credits::Credits::ZERO,
            "pending_lago_debits": crate::models::credits::Credits::ZERO,
            "overdraft_cap_credits": crate::models::credits::Credits::ZERO,
            "has_payment_instrument": false, "suspended": false, "collection_state": "good",
            "balance_synced_at": now, "created_at": now, "updated_at": now,
        })
        .await
        .unwrap();
    let lago = Arc::new(RecordingLago::default());
    let mut catalog = dummy_service();
    catalog.id = Uuid::new_v4().to_string();
    catalog.slug = "svc".into();
    let saved = save(&db, &mut catalog, byok(vec![requested("search", "7")])).await;
    pricing::sync_service_price(&db, lago.as_ref(), "standard", &saved)
        .await
        .unwrap();
    catalog = reload(&db, &catalog).await;
    allowances::create_allowance(
        &db,
        allowances::CreateAllowanceInput {
            service_ref: catalog.id.clone(),
            metric: Some(BillingMetric::Requests),
            quantity: 1,
            recurrence: AllowanceRecurrence::Monthly,
            target_kind: BillingTargetKind::AllUsers,
            target_user_ids: vec![],
            target_org_ids: Vec::new(),
            target_group_ids: Vec::new(),
            created_by: owner.clone(),
        },
    )
    .await
    .unwrap();
    let mut config = crate::test_utils::test_app_config();
    config.billing_enabled = true;
    let billing = BillingService::new_with_lago(db.clone(), Arc::new(config), lago);
    let ctx = BillingRouteContext::new(
        BillingIngress::Proxy,
        Uuid::new_v4().to_string(),
        owner.clone(),
        owner.clone(),
        None,
        None,
        Some(catalog.id.clone()),
        Some(catalog.slug.clone()),
        NodeIntent::Direct,
        "bearer".into(),
        CredentialClass::UserOwned,
        BillingMetric::Requests,
        catalog.billing.as_ref(),
        false,
    )
    .with_operation(Some("search"));
    assert_eq!(
        ctx.platform_lago_metric_code,
        "platform_svc_svc_byok_op_search"
    );
    let metered = billing.open(&ctx).await.unwrap();
    billing.mark_forwarded(&metered).await.unwrap();
    billing
        .settle_deferred(
            &metered,
            PlatformUsage {
                requests: 1,
                ..Default::default()
            },
            None,
            None,
        )
        .await
        .unwrap();
    let row = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let row = db
                .collection::<crate::models::usage_meter::UsageMeterRow>(
                    crate::models::usage_meter::COLLECTION_NAME,
                )
                .find_one(doc! {"billing_request_id": &ctx.billing_request_id})
                .await
                .unwrap()
                .unwrap();
            if row.released {
                return row;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("settled operation row");
    assert_eq!(row.operation.as_deref(), Some("search"));
    let funding = row.funding.unwrap();
    assert_eq!(funding.allowance_funded_quantity, Some(1));
    assert_eq!(
        funding.allowance_funded,
        Some(crate::models::credits::Credits::from_whole(7))
    );
    assert_eq!(
        funding.wallet_funded,
        Some(crate::models::credits::Credits::ZERO)
    );
    db.drop().await.unwrap();
}

use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use axum::body::Body;
use axum::http::{Request, StatusCode};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use chrono::{Duration, Utc};
use hmac::{Hmac, Mac};
use mongodb::IndexModel;
use mongodb::bson::{self, doc};
use mongodb::options::IndexOptions;
use sha2::Sha256;
use tower::ServiceExt;
use uuid::Uuid;

use crate::errors::{AppError, AppResult};
use crate::models::billing_rate_cache::{BillingRateCache, COLLECTION_NAME as BILLING_RATE_CACHE};
use crate::models::billing_wallet::{
    BillingWallet, COLLECTION_NAME as BILLING_WALLET, CollectionState, PlanKind,
};
use crate::models::service_billing::{BillingMetric, PlatformUsage};
use crate::models::usage_meter::{
    COLLECTION_NAME as USAGE_METER, CredentialClass, UsageMeterRow, UsageStatus,
};
use crate::models::user::{COLLECTION_NAME as USERS, User, UserType};
use crate::services::billing::lago_client::{
    Entitlement, LagoAck, LagoApi, LagoError, LagoEvent, LagoUsage, LagoWallet, OwnerProvisionInput,
};
use crate::services::billing::route_inventory::{
    ALL_BILLING_INGRESSES, BILLING_ROUTE_INVENTORY, BILLING_SENSITIVE_HANDLER_PREFIXES,
    BillingIngress, BillingRoutePolicy,
};
use crate::services::billing::{BillingRouteContext, BillingService, NodeIntent};
use crate::test_utils::{
    connect_test_database, test_app_config, test_app_state_with_config, test_user,
};

#[derive(Clone, Copy)]
struct CoverageCase {
    ingress: BillingIngress,
    scenario: &'static str,
    node_intent: NodeIntent,
    metric: BillingMetric,
}

const COVERAGE_CASES: &[CoverageCase] = &[
    CoverageCase {
        ingress: BillingIngress::LlmGateway,
        scenario: "buffered",
        node_intent: NodeIntent::Direct,
        metric: BillingMetric::Tokens,
    },
    CoverageCase {
        ingress: BillingIngress::LlmGateway,
        scenario: "streamed-usage-omitted",
        node_intent: NodeIntent::Direct,
        metric: BillingMetric::Tokens,
    },
    CoverageCase {
        ingress: BillingIngress::LlmProvider,
        scenario: "buffered",
        node_intent: NodeIntent::Direct,
        metric: BillingMetric::Tokens,
    },
    CoverageCase {
        ingress: BillingIngress::LlmProvider,
        scenario: "streamed",
        node_intent: NodeIntent::Direct,
        metric: BillingMetric::Tokens,
    },
    CoverageCase {
        ingress: BillingIngress::Proxy,
        scenario: "direct-buffered",
        node_intent: NodeIntent::Direct,
        metric: BillingMetric::Bytes,
    },
    CoverageCase {
        ingress: BillingIngress::Proxy,
        scenario: "direct-streamed",
        node_intent: NodeIntent::Direct,
        metric: BillingMetric::Bytes,
    },
    CoverageCase {
        ingress: BillingIngress::Proxy,
        scenario: "node-buffered",
        node_intent: NodeIntent::Node,
        metric: BillingMetric::Bytes,
    },
    CoverageCase {
        ingress: BillingIngress::Proxy,
        scenario: "node-streamed",
        node_intent: NodeIntent::Node,
        metric: BillingMetric::Bytes,
    },
    CoverageCase {
        ingress: BillingIngress::Proxy,
        scenario: "direct-websocket",
        node_intent: NodeIntent::Direct,
        metric: BillingMetric::Bytes,
    },
    CoverageCase {
        ingress: BillingIngress::Proxy,
        scenario: "node-websocket",
        node_intent: NodeIntent::Node,
        metric: BillingMetric::Bytes,
    },
    CoverageCase {
        ingress: BillingIngress::Proxy,
        scenario: "direct-llm-stream-usage-omitted",
        node_intent: NodeIntent::Direct,
        metric: BillingMetric::Tokens,
    },
    CoverageCase {
        ingress: BillingIngress::Mcp,
        scenario: "direct",
        node_intent: NodeIntent::Direct,
        metric: BillingMetric::Requests,
    },
    CoverageCase {
        ingress: BillingIngress::Mcp,
        scenario: "node",
        node_intent: NodeIntent::Node,
        metric: BillingMetric::Requests,
    },
    CoverageCase {
        ingress: BillingIngress::Mcp,
        scenario: "node-streamed",
        node_intent: NodeIntent::Node,
        metric: BillingMetric::Requests,
    },
    CoverageCase {
        ingress: BillingIngress::SshExec,
        scenario: "node-exec",
        node_intent: NodeIntent::Node,
        metric: BillingMetric::Bytes,
    },
    CoverageCase {
        ingress: BillingIngress::SshTunnel,
        scenario: "direct-tunnel",
        node_intent: NodeIntent::Direct,
        metric: BillingMetric::Bytes,
    },
    CoverageCase {
        ingress: BillingIngress::SshTunnel,
        scenario: "node-tunnel",
        node_intent: NodeIntent::Node,
        metric: BillingMetric::Bytes,
    },
    CoverageCase {
        ingress: BillingIngress::SshWebTerminal,
        scenario: "node-terminal",
        node_intent: NodeIntent::Node,
        metric: BillingMetric::Bytes,
    },
];

#[derive(Default)]
struct FakeLago {
    wallet_creates: AtomicUsize,
}

#[async_trait]
impl LagoApi for FakeLago {
    async fn ensure_customer(&self, owner: &OwnerProvisionInput) -> AppResult<String> {
        Ok(owner.external_customer_id.clone())
    }

    async fn ensure_subscription(&self, customer_id: &str, plan_code: &str) -> AppResult<String> {
        Ok(format!("{customer_id}:{plan_code}"))
    }

    async fn ensure_wallet(&self, customer_id: &str) -> AppResult<LagoWallet> {
        self.wallet_creates.fetch_add(1, Ordering::SeqCst);
        Ok(LagoWallet {
            id: format!("{customer_id}:wallet"),
            balance_credits: 10_000,
        })
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

    async fn current_usage(
        &self,
        customer_id: &str,
        subscription_id: &str,
    ) -> AppResult<LagoUsage> {
        Ok(LagoUsage {
            customer_id: customer_id.to_string(),
            subscription_id: subscription_id.to_string(),
            raw: serde_json::json!({}),
        })
    }

    async fn wallet_balance(&self, _customer_id: &str) -> AppResult<i64> {
        Ok(10_000)
    }

    async fn entitlements(&self, _subscription_id: &str) -> AppResult<Vec<Entitlement>> {
        Ok(vec![Entitlement {
            code: "*".to_string(),
            raw: serde_json::json!({}),
        }])
    }
}

#[tokio::test]
async fn billing_route_coverage_smoke() {
    assert_route_inventory_matches_router();
    assert_coverage_cases_are_exhaustive();

    let Some(db) = connect_test_database("billing_route_coverage").await else {
        eprintln!("skipping billing route coverage smoke: no local MongoDB available");
        return;
    };
    create_usage_index(&db).await;
    insert_fresh_rates(&db).await;

    let lago = Arc::new(FakeLago::default());
    let service = billing_service(&db, lago.clone(), 7);

    for case in COVERAGE_CASES {
        let owner_id = Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_one(test_user(&owner_id, UserType::Person))
            .await
            .expect("insert billing coverage owner");
        let request_id = format!(
            "{}-{}-{}",
            case.ingress.as_str(),
            case.scenario,
            Uuid::new_v4()
        );
        let ctx = route_context(case, &request_id, &owner_id);

        let metered = service.open(&ctx).await.expect("open route meter");
        assert!(metered.is_enabled(), "{} must be metered", case.scenario);
        let reserved = usage_row(&db, &request_id).await;
        assert_eq!(reserved.status, UsageStatus::Reserved);
        assert!(!reserved.forwarded);
        assert!(reserved.wallet_id.is_some());
        assert_eq!(
            metered.route.as_ref().map(|route| route.ingress),
            Some(case.ingress)
        );

        service
            .mark_forwarded(&metered)
            .await
            .expect("mark route forwarded");
        let forwarded = usage_row(&db, &request_id).await;
        assert_eq!(forwarded.status, UsageStatus::Forwarded);
        assert!(forwarded.forwarded);

        let (usage, expected_quantity) = usage_for(case);
        service
            .settle(&metered, usage.clone(), None, None)
            .await
            .expect("settle route");
        service
            .settle(&metered, usage, None, None)
            .await
            .expect("idempotent settle replay");

        let settled = usage_row(&db, &request_id).await;
        assert_eq!(settled.status, UsageStatus::Finalized);
        assert!(settled.forwarded);
        assert!(settled.released);
        assert_eq!(settled.quantity, Some(expected_quantity));
        assert_eq!(settled.transaction_id, format!("{request_id}:platform"));
        assert_eq!(
            db.collection::<UsageMeterRow>(USAGE_METER)
                .count_documents(doc! { "billing_request_id": &request_id })
                .await
                .expect("count route rows"),
            1,
            "settle replay must not create another charge for {}",
            case.scenario
        );

        let wallet = wallet(&db, &owner_id).await;
        assert_eq!(wallet.overdraft_cap_credits, 7);
        assert_eq!(wallet.reserved_credits, 0);
        assert_eq!(wallet.pending_lago_debits, expected_quantity);
    }

    assert_eq!(
        lago.wallet_creates.load(Ordering::SeqCst),
        COVERAGE_CASES.len(),
        "wallet-less owners must be provisioned before every billable route"
    );
}

#[tokio::test]
async fn billing_gate_rejects_missing_and_stale_rate_cache_entries() {
    let Some(db) = connect_test_database("billing_rate_gate_regression").await else {
        return;
    };
    create_usage_index(&db).await;
    let lago = Arc::new(FakeLago::default());
    let service = billing_service(&db, lago, 0);

    let missing_owner = insert_owner(&db).await;
    let missing_request = Uuid::new_v4().to_string();
    let missing_ctx = route_context(
        &CoverageCase {
            ingress: BillingIngress::Proxy,
            scenario: "missing-rate",
            node_intent: NodeIntent::Direct,
            metric: BillingMetric::Requests,
        },
        &missing_request,
        &missing_owner,
    );
    assert!(matches!(
        service.open(&missing_ctx).await,
        Err(AppError::BillingNotConfigured(message)) if message.contains("missing")
    ));
    assert_no_usage_row(&db, &missing_request).await;

    db.collection::<BillingRateCache>(BILLING_RATE_CACHE)
        .insert_one(BillingRateCache {
            id: BillingRateCache::cache_id("platform_requests", None),
            lago_metric_code: "platform_requests".to_string(),
            model: None,
            credits_per_unit_micros: 1_000_000,
            synced_at: Utc::now() - Duration::seconds(901),
        })
        .await
        .expect("insert stale rate");
    let stale_owner = insert_owner(&db).await;
    let stale_request = Uuid::new_v4().to_string();
    let stale_ctx = route_context(
        &CoverageCase {
            ingress: BillingIngress::Proxy,
            scenario: "stale-rate",
            node_intent: NodeIntent::Direct,
            metric: BillingMetric::Requests,
        },
        &stale_request,
        &stale_owner,
    );
    assert!(matches!(
        service.open(&stale_ctx).await,
        Err(AppError::BillingNotConfigured(message)) if message.contains("stale")
    ));
    assert_no_usage_row(&db, &stale_request).await;
}

#[tokio::test]
async fn settle_after_midstream_suspension_remains_durable() {
    let Some(db) = connect_test_database("billing_settle_after_suspend").await else {
        return;
    };
    create_usage_index(&db).await;
    insert_fresh_rates(&db).await;
    let service = billing_service(&db, Arc::new(FakeLago::default()), 0);
    let owner_id = insert_owner(&db).await;
    let request_id = Uuid::new_v4().to_string();
    let case = CoverageCase {
        ingress: BillingIngress::Proxy,
        scenario: "stream-suspended",
        node_intent: NodeIntent::Direct,
        metric: BillingMetric::Bytes,
    };
    let metered = service
        .open(&route_context(&case, &request_id, &owner_id))
        .await
        .expect("open stream meter");
    service
        .mark_forwarded(&metered)
        .await
        .expect("mark stream forwarded");
    db.collection::<BillingWallet>(BILLING_WALLET)
        .update_one(
            doc! { "owner_id": &owner_id },
            doc! { "$set": { "suspended": true, "collection_state": "suspended" } },
        )
        .await
        .expect("suspend wallet mid-stream");

    service
        .settle(&metered, PlatformUsage::single_request(23), None, None)
        .await
        .expect("settle forwarded stream after suspension");

    let row = usage_row(&db, &request_id).await;
    assert_eq!(row.status, UsageStatus::Finalized);
    assert!(row.released);
    assert_eq!(row.quantity, Some(23));
    let saved_wallet = wallet(&db, &owner_id).await;
    assert!(saved_wallet.suspended);
    assert_eq!(saved_wallet.pending_lago_debits, 23);
}

#[tokio::test]
async fn card_backed_wallet_cannot_reserve_past_the_overdraft_cap() {
    let Some(db) = connect_test_database("billing_overdraft_cap").await else {
        return;
    };
    create_usage_index(&db).await;
    insert_fresh_rates(&db).await;
    let service = billing_service(&db, Arc::new(FakeLago::default()), 2);
    let owner_id = insert_owner(&db).await;
    service
        .ensure_wallet(&owner_id)
        .await
        .expect("provision card-backed test wallet");
    db.collection::<BillingWallet>(BILLING_WALLET)
        .update_one(
            doc! { "owner_id": &owner_id },
            doc! { "$set": {
                "plan_kind": "subscription",
                "balance_credits": 0_i64,
                "has_payment_instrument": true,
            } },
        )
        .await
        .expect("configure card-backed wallet");
    let case = CoverageCase {
        ingress: BillingIngress::Mcp,
        scenario: "overdraft-cap",
        node_intent: NodeIntent::Direct,
        metric: BillingMetric::Requests,
    };

    for _ in 0..2 {
        let request_id = Uuid::new_v4().to_string();
        service
            .open(&route_context(&case, &request_id, &owner_id))
            .await
            .expect("reserve within overdraft cap");
    }
    let denied_request = Uuid::new_v4().to_string();
    assert!(matches!(
        service
            .open(&route_context(&case, &denied_request, &owner_id))
            .await,
        Err(AppError::WalletSuspended)
    ));
    assert_no_usage_row(&db, &denied_request).await;

    let saved_wallet = wallet(&db, &owner_id).await;
    assert_eq!(saved_wallet.plan_kind, PlanKind::Subscription);
    assert_eq!(saved_wallet.overdraft_cap_credits, 2);
    assert_eq!(saved_wallet.reserved_credits, 2);
    assert!(saved_wallet.suspended);
    assert_eq!(saved_wallet.collection_state, CollectionState::Suspended);
}

#[tokio::test]
async fn settle_failure_is_replayed_once_by_reconcile() {
    let Some(db) = connect_test_database("billing_route_settle_recovery").await else {
        return;
    };
    create_usage_index(&db).await;
    insert_fresh_rates(&db).await;
    let service = billing_service(&db, Arc::new(FakeLago::default()), 0);
    let owner_id = insert_owner(&db).await;
    let request_id = Uuid::new_v4().to_string();
    let case = CoverageCase {
        ingress: BillingIngress::Mcp,
        scenario: "settle-recovery",
        node_intent: NodeIntent::Direct,
        metric: BillingMetric::Requests,
    };
    let metered = service
        .open(&route_context(&case, &request_id, &owner_id))
        .await
        .expect("open MCP meter");
    service
        .mark_forwarded(&metered)
        .await
        .expect("mark MCP forwarded");
    let held_wallet = wallet(&db, &owner_id).await;
    db.collection::<BillingWallet>(BILLING_WALLET)
        .delete_one(doc! { "_id": &held_wallet.id })
        .await
        .expect("remove wallet to force settlement failure");

    assert!(
        service
            .settle(&metered, PlatformUsage::single_request(9), None, None)
            .await
            .is_err()
    );
    let failed = usage_row(&db, &request_id).await;
    assert_eq!(failed.status, UsageStatus::Failed);
    assert!(!failed.released);
    assert_eq!(failed.quantity, Some(1));

    db.collection::<BillingWallet>(BILLING_WALLET)
        .insert_one(&held_wallet)
        .await
        .expect("restore wallet before reconcile");
    db.collection::<UsageMeterRow>(USAGE_METER)
        .update_one(
            doc! { "_id": &failed.id },
            doc! { "$set": {
                "settlement_next_retry_at": bson::DateTime::from_chrono(Utc::now() - Duration::seconds(1))
            } },
        )
        .await
        .expect("make settlement retry due");

    let stats = service
        .reconciler()
        .run_once()
        .await
        .expect("reconcile failed settlement");
    assert_eq!(stats.recovered_settlements, 1);
    let recovered = usage_row(&db, &request_id).await;
    assert_eq!(recovered.status, UsageStatus::Finalized);
    assert!(recovered.released);

    service
        .settle(&metered, PlatformUsage::single_request(9), None, None)
        .await
        .expect("replay completed settle");
    let saved_wallet = wallet(&db, &owner_id).await;
    assert_eq!(saved_wallet.reserved_credits, 0);
    assert_eq!(saved_wallet.pending_lago_debits, 1);
}

#[tokio::test]
async fn lago_webhook_signature_is_verified_at_the_mounted_route() {
    let Some(db) = connect_test_database("billing_webhook_signature_route").await else {
        return;
    };
    let secret = "integration-webhook-secret";
    let mut config = test_app_config();
    config.lago_webhook_secret = Some(secret.to_string());
    let state = test_app_state_with_config(db, config);
    let (_, private) = crate::routes::build_router(1024 * 1024, 1024 * 1024);
    let app = private.with_state(state);
    let body = br#"{"webhook_type":"integration.signature_probe"}"#;

    let valid = app
        .clone()
        .oneshot(webhook_request(body, &lago_signature(secret, body)))
        .await
        .expect("call mounted Lago webhook");
    assert_eq!(valid.status(), StatusCode::OK);

    let invalid = app
        .oneshot(webhook_request(body, &lago_signature("wrong-secret", body)))
        .await
        .expect("call mounted Lago webhook with invalid signature");
    assert_eq!(invalid.status(), StatusCode::UNAUTHORIZED);
}

fn assert_route_inventory_matches_router() {
    let routes_source = include_str!("routes.rs");
    let mounted = extract_sensitive_handlers(routes_source);
    let classified: BTreeSet<&str> = BILLING_ROUTE_INVENTORY
        .iter()
        .map(|entry| entry.handler)
        .collect();
    assert_eq!(
        mounted, classified,
        "every billing-sensitive mounted handler must be classified Metered or Exempt"
    );
    assert_eq!(classified.len(), BILLING_ROUTE_INVENTORY.len());
}

fn assert_coverage_cases_are_exhaustive() {
    let covered: BTreeSet<BillingIngress> =
        COVERAGE_CASES.iter().map(|case| case.ingress).collect();
    let expected: BTreeSet<BillingIngress> = ALL_BILLING_INGRESSES.iter().copied().collect();
    assert_eq!(covered, expected);

    for entry in BILLING_ROUTE_INVENTORY {
        assert!(!entry.route.is_empty());
        if let BillingRoutePolicy::Metered(ingress) = entry.policy {
            assert!(
                covered.contains(&ingress),
                "{} has no durable lifecycle coverage",
                entry.route
            );
        }
    }
}

fn extract_sensitive_handlers(source: &str) -> BTreeSet<&str> {
    let mut handlers = BTreeSet::new();
    for prefix in BILLING_SENSITIVE_HANDLER_PREFIXES {
        let mut remaining = source;
        while let Some(offset) = remaining.find(prefix) {
            let candidate = &remaining[offset..];
            let end = candidate
                .find(|character: char| {
                    !(character.is_ascii_alphanumeric() || character == '_' || character == ':')
                })
                .unwrap_or(candidate.len());
            handlers.insert(&candidate[..end]);
            remaining = &candidate[end..];
        }
    }
    handlers
}

fn billing_service(
    db: &mongodb::Database,
    lago: Arc<FakeLago>,
    default_overdraft_cap_credits: i64,
) -> BillingService {
    let mut config = test_app_config();
    config.billing_enabled = true;
    config.billing_rate_cache_ttl_secs = 900;
    config.billing_default_overdraft_cap_credits = default_overdraft_cap_credits;
    BillingService::new_with_lago(db.clone(), Arc::new(config), lago)
}

fn route_context(case: &CoverageCase, request_id: &str, owner_id: &str) -> BillingRouteContext {
    BillingRouteContext::new(
        case.ingress,
        request_id.to_string(),
        owner_id.to_string(),
        owner_id.to_string(),
        Some(Uuid::new_v4().to_string()),
        Some(format!("{}-user-service", case.ingress.as_str())),
        Some(format!("{}-catalog", case.ingress.as_str())),
        Some(format!("{}-{}", case.ingress.as_str(), case.scenario)),
        case.node_intent,
        "test".to_string(),
        if matches!(case.node_intent, NodeIntent::Direct) {
            CredentialClass::UserOwned
        } else {
            CredentialClass::NodeManaged
        },
        case.metric,
        None,
        false,
    )
}

fn usage_for(case: &CoverageCase) -> (PlatformUsage, i64) {
    match case.metric {
        BillingMetric::Tokens => {
            if case.scenario.contains("usage-omitted") {
                let usage = if case.ingress == BillingIngress::Proxy {
                    crate::handlers::proxy::llm_platform_usage(None, 19)
                } else {
                    crate::handlers::llm_gateway::llm_platform_usage(None, 19)
                };
                let expected = usage.tokens;
                (usage, expected)
            } else {
                (PlatformUsage::llm_completion(19, 11), 11)
            }
        }
        BillingMetric::Requests => (PlatformUsage::single_request(9), 1),
        BillingMetric::Bytes => (PlatformUsage::single_request(23), 23),
    }
}

async fn create_usage_index(db: &mongodb::Database) {
    db.collection::<UsageMeterRow>(USAGE_METER)
        .create_index(
            IndexModel::builder()
                .keys(doc! { "transaction_id": 1 })
                .options(IndexOptions::builder().unique(true).build())
                .build(),
        )
        .await
        .expect("create usage transaction index");
}

async fn insert_fresh_rates(db: &mongodb::Database) {
    let now = Utc::now();
    db.collection::<BillingRateCache>(BILLING_RATE_CACHE)
        .insert_many([
            rate("platform_tokens", now),
            rate("platform_requests", now),
            rate("platform_bytes", now),
        ])
        .await
        .expect("insert billing coverage rates");
}

fn rate(metric: &str, synced_at: chrono::DateTime<Utc>) -> BillingRateCache {
    BillingRateCache {
        id: BillingRateCache::cache_id(metric, None),
        lago_metric_code: metric.to_string(),
        model: None,
        credits_per_unit_micros: 1_000_000,
        synced_at,
    }
}

async fn insert_owner(db: &mongodb::Database) -> String {
    let owner_id = Uuid::new_v4().to_string();
    db.collection::<User>(USERS)
        .insert_one(test_user(&owner_id, UserType::Person))
        .await
        .expect("insert billing owner");
    owner_id
}

async fn usage_row(db: &mongodb::Database, request_id: &str) -> UsageMeterRow {
    db.collection::<UsageMeterRow>(USAGE_METER)
        .find_one(doc! { "billing_request_id": request_id })
        .await
        .expect("query usage row")
        .expect("usage row exists")
}

async fn assert_no_usage_row(db: &mongodb::Database, request_id: &str) {
    let count = db
        .collection::<UsageMeterRow>(USAGE_METER)
        .count_documents(doc! { "billing_request_id": request_id })
        .await
        .expect("count usage rows");
    assert_eq!(count, 0);
}

async fn wallet(db: &mongodb::Database, owner_id: &str) -> BillingWallet {
    db.collection::<BillingWallet>(BILLING_WALLET)
        .find_one(doc! { "owner_id": owner_id })
        .await
        .expect("query wallet")
        .expect("wallet exists")
}

fn lago_signature(secret: &str, body: &[u8]) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).expect("HMAC secret");
    mac.update(body);
    BASE64_STANDARD.encode(mac.finalize().into_bytes())
}

fn webhook_request(body: &[u8], signature: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri("/api/v1/webhooks/lago")
        .header("content-type", "application/json")
        .header("x-lago-signature-algorithm", "hmac")
        .header("x-lago-signature", signature)
        .body(Body::from(body.to_vec()))
        .expect("build Lago webhook request")
}

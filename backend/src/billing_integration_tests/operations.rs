//! Operation prices on mounted proxy, MCP and LLM routes.
use super::*;
use crate::models::credits::Credits;
use crate::models::downstream_service::OfferingKind;
use crate::models::service_billing::{LanePricing, OperationPrice, PricingSyncStatus};
use crate::models::service_endpoint::{COLLECTION_NAME as SERVICE_ENDPOINTS, ServiceEndpoint};
use crate::test_utils::{connect_test_database_with_command_handler, test_service_endpoint};
use futures::TryStreamExt;
use mongodb::bson::Document;
use mongodb::event::{EventHandler, command::CommandEvent};
use std::sync::Mutex;

const SLUG: &str = "priced-operations";
const BASE_CODE: &str = "platform_svc_priced-operations_byok";

fn code(slug: &str, operation: &str) -> String {
    crate::services::billing::pricing::operation_metric_code(slug, "byok", operation).unwrap()
}

/// `(operation, whole credits, synced)` on the `slug` BYOK lane.
fn prices(slug: &str, rows: &[(&str, i64, bool)]) -> Vec<OperationPrice> {
    rows.iter()
        .map(|(operation, credits, synced)| OperationPrice {
            operation: (*operation).into(),
            credits_per_unit: credits.to_string(),
            lago_metric_code: code(slug, operation),
            sync_status: if *synced {
                PricingSyncStatus::Synced
            } else {
                PricingSyncStatus::Pending
            },
            sync_error: None,
        })
        .collect()
}

fn requests_lane(slug: &str, operations: Vec<OperationPrice>) -> ServiceBilling {
    ServiceBilling {
        byok_pricing: Some(LanePricing {
            metric: BillingMetric::Requests,
            credits_per_unit: "1".into(),
            lago_metric_code: format!("platform_svc_{slug}_byok"),
            sync_status: PricingSyncStatus::Synced,
            sync_error: None,
            components: vec![],
            operations,
        }),
        ..Default::default()
    }
}

async fn insert_rates(db: &mongodb::Database, rates: &[(String, i64)]) {
    for (code, credits) in rates {
        db.collection::<BillingRateCache>(BILLING_RATE_CACHE)
            .insert_one(BillingRateCache {
                id: BillingRateCache::cache_id(code, None),
                lago_metric_code: code.clone(),
                model: None,
                credits_per_unit_micros: credits * 1_000_000,
                credits_per_unit_pico: None,
                synced_at: Utc::now(),
                retired_at: None,
            })
            .await
            .unwrap();
    }
}

/// Gives the owner a funded wallet and a personal BYOK key on `user_service`,
/// so requests bill the BYOK lane as `UserOwned`.
async fn fund_and_connect(state: &crate::AppState, owner: &str, user_service: &str) {
    let now = bson::DateTime::now();
    let encrypted = state
        .encryption_keys
        .encrypt(b"operation-byok-secret")
        .await
        .unwrap();
    let key_id = Uuid::new_v4().to_string();
    state.db.collection::<Document>("user_api_keys").insert_one(doc! {
        "_id": &key_id, "user_id": owner, "label": "Operation BYOK", "credential_type": "bearer",
        "status": "active", "credential_epoch": 1_i64,
        "credential_encrypted": bson::Binary { subtype: bson::spec::BinarySubtype::Generic, bytes: encrypted },
        "created_at": now, "updated_at": now,
    }).await.unwrap();
    state
        .db
        .collection::<Document>(USER_SERVICES)
        .update_one(
            doc! {"_id": user_service},
            doc! {"$set": {"api_key_id": &key_id, "auth_method": "bearer", "auth_key_name": "Authorization"}},
        )
        .await
        .unwrap();
    state.db.collection::<Document>(BILLING_WALLET).insert_one(doc! {
        "_id": Uuid::new_v4().to_string(), "owner_id": owner, "lago_customer_id": owner,
        "lago_wallet_id": "operation-wallet", "lago_subscription_id": "operation-plan", "plan_kind": "prepaid",
        "balance_credits": Credits::from_whole(100), "reserved_credits": Credits::ZERO,
        "pending_lago_debits": Credits::ZERO, "overdraft_cap_credits": Credits::ZERO,
        "has_payment_instrument": false, "suspended": false, "collection_state": "good",
        "balance_synced_at": now, "created_at": now, "updated_at": now,
    }).await.unwrap();
}

fn mounted(state: &crate::AppState) -> Router {
    let (public, private) = crate::routes::build_router_with_state(state.clone());
    public.merge(private).with_state(state.clone())
}

struct Priced {
    db: mongodb::Database,
    state: crate::AppState,
    app: Router,
    owner: String,
    token: String,
    catalog: DownstreamService,
    _downstream: tokio::task::JoinHandle<()>,
}

async fn priced(
    db: mongodb::Database,
    offering_kind: OfferingKind,
    operations: &[(&str, i64, bool)],
) -> Priced {
    create_usage_index(&db).await;
    let owner = insert_owner(&db).await;
    let (url, downstream) = start_billing_downstream().await;
    let mut catalog = crate::models::downstream_service::test_helpers::dummy_service();
    catalog.id = Uuid::new_v4().to_string();
    catalog.slug = SLUG.into();
    catalog.base_url = url.clone();
    catalog.offering_kind = offering_kind;
    catalog.billing = Some(requests_lane(SLUG, prices(SLUG, operations)));
    db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .insert_one(&catalog)
        .await
        .unwrap();
    let endpoints: Vec<ServiceEndpoint> = [
        ("search", "/search"),
        ("lookup", "/lookup"),
        ("export", "/export"),
        ("item", "/items/{id}"),
        ("item_me", "/items/me"),
    ]
    .into_iter()
    .map(|(name, path)| test_service_endpoint(&catalog.id, name, "GET", path))
    .collect();
    db.collection::<ServiceEndpoint>(SERVICE_ENDPOINTS)
        .insert_many(endpoints)
        .await
        .unwrap();
    let mut rates = vec![(BASE_CODE.to_string(), 1)];
    rates.extend(
        operations
            .iter()
            .filter(|(_, _, synced)| *synced)
            .map(|(operation, credits, _)| (code(SLUG, operation), *credits)),
    );
    insert_rates(&db, &rates).await;
    let state = billing_route_state(db.clone(), Arc::new(FakeLago::default()), 0);
    let user_service = insert_route_service(&db, &owner, SLUG, &url, Some(&catalog.id), None).await;
    fund_and_connect(&state, &owner, &user_service.id).await;
    Priced {
        token: route_access_token(&state, &owner),
        app: mounted(&state),
        db,
        state,
        owner,
        catalog,
        _downstream: downstream,
    }
}

async fn get(fixture: &Priced, path: &str) {
    call_mounted_route(
        &fixture.app,
        route_request(
            Method::GET,
            &format!("/api/v1/proxy/s/{SLUG}{path}"),
            &fixture.token,
            Body::empty(),
        ),
    )
    .await;
}

async fn settled_rows(db: &mongodb::Database, slug: &str, count: u64) -> Vec<UsageMeterRow> {
    assert_route_settled_count(db, slug, BillingMetric::Requests, count).await;
    db.collection::<UsageMeterRow>(USAGE_METER)
        .find(doc! {"service_slug": slug})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap()
}

/// Each expected `(lago code, operation, credits)` row was reserved and settled
/// at that price.
fn assert_priced(rows: &[UsageMeterRow], expected: &[(String, Option<&str>, i64)]) {
    for (code, operation, credits) in expected {
        let row = rows
            .iter()
            .find(|row| &row.lago_metric_code == code && row.operation.as_deref() == *operation)
            .expect("a row for each priced request");
        assert_eq!(row.credential_class, CredentialClass::UserOwned);
        assert_eq!(row.quantity, Some(1));
        assert_eq!(row.reserved_credits, Credits::from_whole(*credits));
        assert_eq!(
            row.funding
                .as_ref()
                .and_then(|funding| funding.total_charge),
            Some(Credits::from_whole(*credits))
        );
    }
}

#[tokio::test]
async fn tool_proxy_reserves_settles_and_meters_the_published_operation_price() {
    let Some(db) = connect_test_database("billing_operation_tool_proxy").await else {
        return;
    };
    let fixture = priced(
        db,
        OfferingKind::Tool,
        &[
            ("search", 3, true),
            ("lookup", 0, true),
            ("export", 9, false),
        ],
    )
    .await;
    for path in ["/search", "/lookup", "/export"] {
        get(&fixture, path).await;
    }
    let rows = settled_rows(&fixture.db, SLUG, 3).await;
    assert_priced(
        &rows,
        &[
            (code(SLUG, "search"), Some("search"), 3),
            // Explicit zero makes the operation free.
            (code(SLUG, "lookup"), Some("lookup"), 0),
            // A pending operation price keeps the base rate.
            (BASE_CODE.to_string(), None, 1),
        ],
    );
    let wallet = wallet(&fixture.db, &fixture.owner).await;
    assert_eq!(wallet.pending_lago_debits, Credits::from_whole(4));
    assert_eq!(wallet.reserved_credits, Credits::ZERO);
    fixture.db.drop().await.unwrap();
}

#[tokio::test]
async fn catalog_proxy_prices_the_most_specific_active_operation() {
    let Some(db) = connect_test_database("billing_operation_catalog_proxy").await else {
        return;
    };
    let fixture = priced(
        db,
        OfferingKind::default(),
        &[("search", 3, true), ("item_me", 5, true)],
    )
    .await;
    for path in ["/search", "/items/me", "/items/7", "/unlisted"] {
        get(&fixture, path).await;
    }
    let rows = settled_rows(&fixture.db, SLUG, 4).await;
    assert_priced(
        &rows,
        &[
            (code(SLUG, "search"), Some("search"), 3),
            (code(SLUG, "item_me"), Some("item_me"), 5),
            (BASE_CODE.to_string(), None, 1),
        ],
    );
    assert_eq!(
        rows.iter()
            .filter(|row| row.lago_metric_code == BASE_CODE && row.operation.is_none())
            .count(),
        2,
        "an unpriced endpoint and an unlisted path both use the base rate"
    );
    assert_eq!(
        wallet(&fixture.db, &fixture.owner)
            .await
            .pending_lago_debits,
        Credits::from_whole(10)
    );
    fixture.db.drop().await.unwrap();
}

#[tokio::test]
async fn proxy_reads_service_endpoints_only_when_the_lane_prices_operations() {
    let commands = Arc::new(Mutex::new(Vec::<(String, Document)>::new()));
    let observed = commands.clone();
    let handler = EventHandler::<CommandEvent>::callback(move |event| {
        if let CommandEvent::Started(event) = event {
            observed
                .lock()
                .unwrap()
                .push((event.command_name, event.command));
        }
    });
    let Some(db) =
        connect_test_database_with_command_handler("billing_operation_endpoint_reads", handler)
            .await
    else {
        return;
    };
    let fixture = priced(db, OfferingKind::default(), &[]).await;
    let endpoint_reads = || {
        commands
            .lock()
            .unwrap()
            .iter()
            .filter(|(name, command)| {
                name == "find" && command.get_str("find").ok() == Some(SERVICE_ENDPOINTS)
            })
            .count()
    };
    commands.lock().unwrap().clear();
    get(&fixture, "/search").await;
    settled_rows(&fixture.db, SLUG, 1).await;
    assert_eq!(endpoint_reads(), 0);

    // Pricing an operation makes the same request resolve its operation.
    fixture
        .db
        .collection::<Document>(DOWNSTREAM_SERVICES)
        .update_one(
            doc! {"_id": &fixture.catalog.id},
            doc! {"$set": {"billing.byok_pricing.operations":
            bson::to_bson(&prices(SLUG, &[("search", 3, true)])).unwrap()}},
        )
        .await
        .unwrap();
    insert_rates(&fixture.db, &[(code(SLUG, "search"), 3)]).await;
    get(&fixture, "/search").await;
    let rows = settled_rows(&fixture.db, SLUG, 2).await;
    assert_eq!(endpoint_reads(), 1);
    assert!(
        rows.iter()
            .any(|row| row.operation.as_deref() == Some("search"))
    );
    fixture.db.drop().await.unwrap();
}

#[tokio::test]
async fn mcp_tool_call_is_priced_by_its_endpoint_name() {
    let Some(db) = connect_test_database("billing_operation_mcp").await else {
        return;
    };
    let fixture = priced(db, OfferingKind::default(), &[("search", 3, true)]).await;
    let session = fixture
        .state
        .mcp_sessions
        .create_with_proxy_access(&fixture.owner, true, false)
        .await
        .unwrap()
        .unwrap();
    let token = crate::crypto::jwt::generate_access_token(
        &fixture.state.jwt_keys,
        &fixture.state.config,
        &Uuid::parse_str(&fixture.owner).unwrap(),
        "openid profile proxy",
        None,
        None,
        None,
        None,
        None,
    )
    .unwrap();
    let call = serde_json::json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {
        "name": "nyx__call_tool",
        "arguments": {"tool_name": format!("{SLUG}__search"), "arguments_json": "{}"},
    }});
    let response = fixture
        .app
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::POST)
                .uri("/mcp")
                .header("authorization", format!("Bearer {token}"))
                .header("mcp-session-id", &session)
                .header("content-type", "application/json")
                .body(Body::from(call.to_string()))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: serde_json::Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 1_000_000).await.unwrap()).unwrap();
    assert_ne!(body["result"]["isError"], true, "the tool call succeeds");
    let rows = settled_rows(&fixture.db, SLUG, 1).await;
    assert_priced(&rows, &[(code(SLUG, "search"), Some("search"), 3)]);
    fixture.db.drop().await.unwrap();
}

#[tokio::test]
async fn llm_provider_and_gateway_requests_price_the_matched_operation() {
    let Some(db) = connect_test_database("billing_operation_llm").await else {
        return;
    };
    create_usage_index(&db).await;
    let owner = insert_owner(&db).await;
    let (url, _downstream) = start_billing_downstream().await;
    let mut catalog = insert_llm_route_service(&db, &owner, &url).await;
    catalog.billing = Some(requests_lane(
        &catalog.slug,
        prices(&catalog.slug, &[("chat_completions", 4, true)]),
    ));
    db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .replace_one(doc! {"_id": &catalog.id}, &catalog)
        .await
        .unwrap();
    db.collection::<ServiceEndpoint>(SERVICE_ENDPOINTS)
        .insert_one(test_service_endpoint(
            &catalog.id,
            "chat_completions",
            "POST",
            "/chat/completions",
        ))
        .await
        .unwrap();
    insert_rates(
        &db,
        &[
            (format!("platform_svc_{}_byok", catalog.slug), 1),
            (code(&catalog.slug, "chat_completions"), 4),
        ],
    )
    .await;
    let state = billing_route_state(db.clone(), Arc::new(FakeLago::default()), 0);
    let user_service = db
        .collection::<UserService>(USER_SERVICES)
        .find_one(doc! {"slug": "billing-llm-route"})
        .await
        .unwrap()
        .unwrap();
    fund_and_connect(&state, &owner, &user_service.id).await;
    let app = mounted(&state);
    let token = route_access_token(&state, &owner);
    for path in [
        "/api/v1/llm/deepseek/v1/chat/completions",
        "/api/v1/llm/gateway/v1/chat/completions",
    ] {
        let body = serde_json::json!({
            "model": "deepseek-chat",
            "messages": [{"role": "user", "content": "operation price"}],
        });
        call_mounted_route(
            &app,
            route_request(Method::POST, path, &token, Body::from(body.to_string())),
        )
        .await;
    }
    let rows = settled_rows(&db, &catalog.slug, 2).await;
    assert!(rows.iter().all(|row| {
        row.lago_metric_code == code(&catalog.slug, "chat_completions")
            && row.operation.as_deref() == Some("chat_completions")
            && row.reserved_credits == Credits::from_whole(4)
    }));
    db.drop().await.unwrap();
}

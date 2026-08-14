use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use async_trait::async_trait;
use axum::body::{Body, to_bytes};
use axum::extract::ws::WebSocketUpgrade;
use axum::http::{Method, Request, StatusCode};
use axum::response::IntoResponse;
use axum::routing::{any, get};
use axum::{Json, Router};
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64_STANDARD};
use chrono::{Duration, Utc};
use futures::{SinkExt, StreamExt};
use hmac::{Hmac, Mac};
use mongodb::IndexModel;
use mongodb::bson::{self, doc};
use mongodb::options::IndexOptions;
use sha2::Sha256;
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tower::ServiceExt;
use uuid::Uuid;

use crate::errors::{AppError, AppResult};
use crate::models::approval_request::{
    ApprovalRequest, COLLECTION_NAME as APPROVAL_REQUESTS, ExactServiceApprovalBinding,
};
use crate::models::audit_log::{AuditLog, COLLECTION_NAME as AUDIT_LOG};
use crate::models::billing_rate_cache::{BillingRateCache, COLLECTION_NAME as BILLING_RATE_CACHE};
use crate::models::billing_wallet::{
    BillingWallet, COLLECTION_NAME as BILLING_WALLET, CollectionState, PlanKind,
};
use crate::models::downstream_service::{
    COLLECTION_NAME as DOWNSTREAM_SERVICES, DownstreamService,
};
use crate::models::node::{COLLECTION_NAME as NODES, Node, NodeMetrics, NodeStatus};
use crate::models::notification_channel::{
    COLLECTION_NAME as NOTIFICATION_CHANNELS, NotificationChannel,
};
use crate::models::provider_config::{COLLECTION_NAME as PROVIDER_CONFIGS, ProviderConfig};
use crate::models::service_approval_config::ApprovalMode;
use crate::models::service_billing::{BillingMetric, PlatformUsage, ServiceBilling};
use crate::models::service_endpoint::{
    COLLECTION_NAME as SERVICE_ENDPOINTS, EndpointRisk, OperationResponseContract, ServiceEndpoint,
};
use crate::models::ssh_auth_mode::SshAuthMode;
use crate::models::usage_meter::{
    COLLECTION_NAME as USAGE_METER, CredentialClass, UsageMeterRow, UsageStatus,
};
use crate::models::user::{COLLECTION_NAME as USERS, User, UserType};
use crate::models::user_endpoint::{COLLECTION_NAME as USER_ENDPOINTS, UserEndpoint};
use crate::models::user_provider_token::{
    COLLECTION_NAME as USER_PROVIDER_TOKENS, UserProviderToken,
};
use crate::models::user_service::{COLLECTION_NAME as USER_SERVICES, UserService};
use crate::services::billing::lago_client::{
    Entitlement, LagoAck, LagoApi, LagoError, LagoEvent, LagoUsage, LagoWallet, OwnerProvisionInput,
};
use crate::services::billing::route_inventory::{
    ALL_BILLING_INGRESSES, BILLING_ROUTE_INVENTORY, BillingIngress, BillingRoutePolicy,
};
use crate::services::billing::{BillingRouteContext, BillingService, NodeIntent};
use crate::services::node_ws_manager::{NodeOutboundMessage, NodeProxyResponse, NodeSshExecResult};
use crate::test_utils::{
    connect_test_database, test_app_config, test_app_state_with_config, test_user,
    test_user_endpoint, test_user_service,
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
    assert_http_egress_classification_is_fail_closed();
    let mut exercised_routes = BTreeSet::new();

    let Some(db) = connect_test_database("billing_route_coverage").await else {
        eprintln!("skipping billing route coverage smoke: no local MongoDB available");
        return;
    };
    create_usage_index(&db).await;
    insert_fresh_rates(&db).await;

    let lago = Arc::new(FakeLago::default());
    let owner_id = insert_owner(&db).await;
    let (downstream_url, direct_hops, agent_tool_hits, downstream) =
        start_billing_downstream().await;
    let mut proxy_catalog = crate::models::downstream_service::test_helpers::dummy_service();
    proxy_catalog.id = Uuid::new_v4().to_string();
    proxy_catalog.slug = "billing-proxy-catalog".to_string();
    proxy_catalog.name = "Billing proxy route boundary".to_string();
    proxy_catalog.base_url = downstream_url.clone();
    db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .insert_one(&proxy_catalog)
        .await
        .expect("insert route proxy catalog service");
    let now = Utc::now();
    db.collection::<ServiceEndpoint>(SERVICE_ENDPOINTS)
        .insert_one(ServiceEndpoint {
            id: Uuid::new_v4().to_string(),
            service_id: proxy_catalog.id.clone(),
            name: "agent_health".to_string(),
            description: Some("Read local route health".to_string()),
            method: "GET".to_string(),
            path: "/agent-health".to_string(),
            parameters: None,
            request_body_schema: None,
            request_content_type: None,
            request_body_required: false,
            response_description: Some("Local health result".to_string()),
            response: OperationResponseContract {
                content_types: vec!["application/json".to_string()],
                binary_artifact: Some(false),
            },
            risk: Some(EndpointRisk::Read),
            supports_idempotency_key: false,
            is_active: true,
            created_at: now,
            updated_at: now,
        })
        .await
        .expect("insert agent route typed endpoint");
    let proxy = insert_route_service(
        &db,
        &owner_id,
        "billing-proxy-route",
        &downstream_url,
        Some(&proxy_catalog.id),
        None,
    )
    .await;
    let llm_catalog = insert_llm_route_service(&db, &owner_id, &downstream_url).await;
    let mut direct_catalog = crate::models::downstream_service::test_helpers::dummy_service();
    direct_catalog.id = Uuid::new_v4().to_string();
    direct_catalog.slug = crate::services::assistant_direct::DIRECT_LLM_SLUG.to_string();
    direct_catalog.name = "Direct Chrono-LLM route boundary".to_string();
    direct_catalog.base_url = format!("{downstream_url}/direct");
    direct_catalog.service_category = "internal".to_string();
    direct_catalog.streaming_supported = true;
    direct_catalog.billing = Some(ServiceBilling {
        platform_billable: true,
        platform_metric: Some(BillingMetric::Tokens),
        ..Default::default()
    });
    db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .insert_one(&direct_catalog)
        .await
        .expect("insert direct assistant catalog service");
    crate::services::feature_flag_service::set_platform_override(
        &db,
        crate::services::feature_flag_service::DIRECT_CHAT_ENGINE_FLAG_KEY,
        &crate::services::feature_flag_service::FlagTarget::Global,
        true,
        "billing-route-test",
    )
    .await
    .expect("enable direct assistant route for billing smoke");

    let state = billing_route_state(db.clone(), lago, 100);
    let token = route_access_token(&state, &owner_id);
    let (_, private) = crate::routes::build_router(
        state.config.proxy_max_body_size,
        state.config.public_proxy_max_body_size,
    );
    let app = private.with_state(state.clone());

    let direct_body = serde_json::json!({
        "messages": [{"role": "user", "content": "route boundary"}],
        "model": "gpt-5.5",
        "skill_slug": "nyxid",
    });
    let direct_response = call_mounted_route(
        &app,
        route_request(
            Method::POST,
            "/api/v1/assistant/direct/completions",
            &token,
            Body::from(direct_body.to_string()),
        ),
    )
    .await;
    assert!(String::from_utf8_lossy(&direct_response).contains("\"total_tokens\":149"));
    exercised_routes.insert("/api/v1/assistant/direct/completions");
    assert_direct_reported_usage(&db, &direct_catalog).await;

    let agent_body = serde_json::json!({
        "messages": [{"role": "user", "content": "route boundary agent"}],
        "model": "gpt-5.5",
        "skill_slug": "nyxid",
    });
    let access_token_response = app
        .clone()
        .oneshot(route_request(
            Method::POST,
            "/api/v1/assistant/direct/agent",
            &token,
            Body::from(agent_body.to_string()),
        ))
        .await
        .expect("call session-only agent route with access token");
    assert_eq!(access_token_response.status(), StatusCode::FORBIDDEN);

    let session = crate::services::token_service::create_session(
        &db,
        &owner_id,
        Some("127.0.0.1"),
        Some("billing-route-smoke"),
    )
    .await
    .expect("create billing route smoke session");
    let agent_response = call_mounted_route(
        &app,
        session_route_request(
            Method::POST,
            "/api/v1/assistant/direct/agent",
            &session.session_token,
            Body::from(agent_body.to_string()),
        ),
    )
    .await;
    let agent_sse = String::from_utf8_lossy(&agent_response);
    assert!(agent_sse.contains("\"type\":\"run.started\""));
    assert!(agent_sse.contains("\"stage\":\"plan\",\"status\":\"completed\""));
    assert!(agent_sse.contains("\"stage\":\"execute\",\"status\":\"completed\""));
    assert!(agent_sse.contains("\"stage\":\"final\",\"status\":\"completed\""));
    assert!(agent_sse.contains("\"type\":\"done\",\"status\":\"completed\""));
    assert!(agent_sse.contains("data: [DONE]"));
    let tool_started = agent_sse.find("\"type\":\"tool.started\"").unwrap();
    let tool_completed = agent_sse.find("\"type\":\"tool.completed\"").unwrap();
    let final_text = agent_sse.find("Chrono Sandbox is healthy.").unwrap();
    let done = agent_sse.find("\"type\":\"done\"").unwrap();
    assert!(tool_started < tool_completed && tool_completed < final_text && final_text < done);
    assert_eq!(agent_tool_hits.load(Ordering::SeqCst), 1);
    assert_eq!(direct_hops.load(Ordering::SeqCst), 5);
    assert_direct_settled_usage_count(&db, &direct_catalog, 5).await;
    exercised_routes.insert("/api/v1/assistant/direct/agent");

    call_mounted_route(
        &app,
        route_request(
            Method::GET,
            "/api/v1/proxy/s/billing-proxy-route/buffered",
            &token,
            Body::empty(),
        ),
    )
    .await;
    exercised_routes.insert("/api/v1/proxy/s/{slug}/{*path}");
    assert_route_settled(&db, &proxy.slug, BillingMetric::Requests).await;

    call_mounted_route(
        &app,
        route_request(
            Method::GET,
            "/api/v1/proxy/s/billing-proxy-route/stream",
            &token,
            Body::empty(),
        ),
    )
    .await;
    assert_route_settled_count(&db, &proxy.slug, BillingMetric::Requests, 2).await;

    for (path, mounted_route) in [
        (
            format!("/api/v1/proxy/s/{}", proxy.slug),
            "/api/v1/proxy/s/{slug}",
        ),
        (
            format!("/api/v1/proxy/{}/buffered", proxy_catalog.id),
            "/api/v1/proxy/{service_id}/{*path}",
        ),
        (
            format!("/api/v1/proxy/{}", proxy_catalog.id),
            "/api/v1/proxy/{service_id}",
        ),
    ] {
        call_mounted_route(
            &app,
            route_request(Method::GET, &path, &token, Body::empty()),
        )
        .await;
        exercised_routes.insert(mounted_route);
    }
    assert_route_settled_count(&db, &proxy.slug, BillingMetric::Requests, 5).await;

    let node = insert_route_node(&state, &owner_id, "billing-route-node").await;
    let node_proxy = insert_route_service(
        &db,
        &owner_id,
        "billing-node-proxy-route",
        "https://node-route.invalid",
        None,
        Some(&node.id),
    )
    .await;
    let (node_tx, node_rx) = mpsc::channel(256);
    state.node_ws_manager.register_connection(&node.id, node_tx);
    let node_responder = spawn_node_http_responder(&state, &node.id, node_rx, 3);

    call_mounted_route(
        &app,
        route_request(
            Method::GET,
            "/api/v1/proxy/s/billing-node-proxy-route/buffered",
            &token,
            Body::empty(),
        ),
    )
    .await;
    assert_route_settled(&db, &node_proxy.slug, BillingMetric::Requests).await;
    call_mounted_route(
        &app,
        route_request(
            Method::GET,
            "/api/v1/proxy/s/billing-node-proxy-route/stream",
            &token,
            Body::empty(),
        ),
    )
    .await;
    assert_route_settled_count(&db, &node_proxy.slug, BillingMetric::Requests, 2).await;

    for (path, stream, mounted_route) in [
        (
            "/api/v1/llm/deepseek/v1/chat/completions",
            false,
            "/api/v1/llm/{provider_slug}/v1/{*path}",
        ),
        (
            "/api/v1/llm/deepseek/v1/stream",
            true,
            "/api/v1/llm/{provider_slug}/v1/{*path}",
        ),
        (
            "/api/v1/llm/gateway/v1/chat/completions",
            false,
            "/api/v1/llm/gateway/v1/{*path}",
        ),
        (
            "/api/v1/llm/gateway/v1/stream",
            true,
            "/api/v1/llm/gateway/v1/{*path}",
        ),
    ] {
        let body = serde_json::json!({
            "model": "deepseek-chat",
            "messages": [{"role": "user", "content": "route boundary"}],
            "stream": stream,
        });
        call_mounted_route(
            &app,
            route_request(Method::POST, path, &token, Body::from(body.to_string())),
        )
        .await;
        exercised_routes.insert(mounted_route);
    }
    assert_route_settled_count(&db, &llm_catalog.slug, BillingMetric::Tokens, 4).await;

    let mcp = insert_route_service(
        &db,
        &owner_id,
        "billing-mcp-route",
        &downstream_url,
        None,
        None,
    )
    .await;
    let initialize = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 0,
        "method": "initialize",
        "params": {
            "protocolVersion": "2025-03-26",
            "capabilities": {},
            "clientInfo": {"name": "billing-route-smoke", "version": "1"},
        },
    });
    let initialize_response = app
        .clone()
        .oneshot(route_request(
            Method::POST,
            "/mcp",
            &token,
            Body::from(initialize.to_string()),
        ))
        .await
        .expect("initialize mounted MCP route");
    assert_eq!(initialize_response.status(), StatusCode::OK);
    let mcp_session_id = initialize_response
        .headers()
        .get("mcp-session-id")
        .expect("mounted MCP initialize returns a session")
        .clone();
    let _ = to_bytes(initialize_response.into_body(), usize::MAX)
        .await
        .expect("consume MCP initialize response");
    let mcp_body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "nyx__call_tool",
            "arguments": {
                "tool_name": "billing-mcp-route__request",
                "arguments": {
                    "method": "GET",
                    "path": "/mcp-query?tag=a&tag=b&name=Nyx%20ID&empty="
                },
            },
        },
    });
    let mut mcp_request = route_request(
        Method::POST,
        "/mcp",
        &token,
        Body::from(mcp_body.to_string()),
    );
    mcp_request
        .headers_mut()
        .insert("mcp-session-id", mcp_session_id.clone());
    let mcp_response = call_mounted_route(&app, mcp_request).await;
    assert!(
        !String::from_utf8_lossy(&mcp_response).contains("\"isError\":true"),
        "mounted MCP tool call failed: {}",
        String::from_utf8_lossy(&mcp_response)
    );
    assert_route_settled(&db, &mcp.slug, BillingMetric::Requests).await;
    exercised_routes.insert("/mcp (POST)");

    let now = Utc::now();
    db.collection::<NotificationChannel>(NOTIFICATION_CHANNELS)
        .insert_one(NotificationChannel {
            id: Uuid::new_v4().to_string(),
            user_id: owner_id.clone(),
            telegram_chat_id: None,
            telegram_username: None,
            telegram_enabled: false,
            telegram_link_code: None,
            telegram_link_code_expires_at: None,
            approval_timeout_secs: 300,
            grant_expiry_days: 30,
            approval_required: true,
            push_enabled: false,
            push_devices: vec![],
            created_at: now,
            updated_at: now,
        })
        .await
        .expect("enable per-request approval for exact redemption smoke");
    let exact_arguments = serde_json::json!({
        "method": "GET",
        "path": "/exact-billing?source=approval",
    });
    let exact_catalog = crate::services::mcp_service::load_operation_catalog(
        &db,
        state.node_ws_manager.as_ref(),
        &owner_id,
        crate::services::mcp_service::NodeScope::Unrestricted,
        crate::services::mcp_service::ServiceScope::Unrestricted,
    )
    .await
    .expect("load exact redemption catalog");
    let exact_service = exact_catalog
        .services
        .iter()
        .find(|service| service.service_id == mcp.id)
        .expect("billing MCP service is present in exact catalog");
    let exact_endpoint = exact_service
        .endpoints
        .iter()
        .find(|endpoint| endpoint.endpoint_id == "nyx_generic_proxy_v1")
        .expect("generic billing MCP request endpoint");
    let endpoint_id = exact_endpoint.endpoint_id.clone();
    let endpoint_contract_digest =
        crate::services::mcp_service::endpoint_contract_digest(exact_endpoint);
    let operation_digest = crate::services::mcp_service::exact_operation_digest(
        &mcp.id,
        exact_endpoint,
        &exact_arguments,
    );
    let catalog_digest =
        crate::services::mcp_service::operation_catalog_digest(&exact_catalog.services);
    let request_id = Uuid::new_v4().to_string();
    let operation_id = "billing-exact-operation".to_string();
    let operation_generation = 1;
    let effect_idempotency_key = "billing-exact-idempotency".to_string();
    let request_key = crate::services::mcp_service::canonical_sha256(serde_json::json!({
        "contract_version": "nyxid-exact-approval-request.v1",
        "requester_type": "access_token",
        "requester_id": owner_id,
        "actor_user_id": owner_id,
        "operation_id": operation_id,
        "operation_generation": operation_generation,
        "idempotency_key": effect_idempotency_key,
    }));
    db.collection::<ApprovalRequest>(APPROVAL_REQUESTS)
        .insert_one(ApprovalRequest {
            id: request_id.clone(),
            user_id: owner_id.clone(),
            service_id: mcp.id.clone(),
            service_name: exact_service.service_name.clone(),
            service_slug: exact_service.service_slug.clone(),
            requester_type: "access_token".to_string(),
            requester_id: owner_id.clone(),
            requester_label: None,
            operation_summary: "GET /exact-billing".to_string(),
            action_description: None,
            http_method: Some("GET".to_string()),
            resource: Some("/exact-billing".to_string()),
            verb: Some("read".to_string()),
            grant_scope: None,
            tool_name: None,
            tool_call_id: None,
            tool_arguments: None,
            is_destructive: None,
            approval_mode: ApprovalMode::PerRequest,
            status: "approved".to_string(),
            idempotency_key: request_key.clone(),
            notification_channel: None,
            telegram_message_id: None,
            telegram_chat_id: None,
            expires_at: now + Duration::minutes(5),
            decided_at: Some(now),
            decision_channel: Some("web".to_string()),
            decision_idempotency_key: Some("billing-exact-decision".to_string()),
            notify_user_ids: vec![owner_id.clone()],
            from_org_policy: false,
            exact_service: Some(ExactServiceApprovalBinding {
                request_key,
                actor_user_id: owner_id.clone(),
                user_service_id: mcp.id.clone(),
                endpoint_id,
                catalog_digest: catalog_digest.clone(),
                endpoint_contract_digest,
                operation_digest: operation_digest.clone(),
                operation_id: operation_id.clone(),
                operation_generation,
                effect_idempotency_key: effect_idempotency_key.clone(),
                arguments: exact_arguments,
                redemption: None,
            }),
            created_at: now,
        })
        .await
        .expect("insert approved exact-service authority");
    let redeem_body = serde_json::json!({
        "catalog_digest": catalog_digest,
        "operation_digest": operation_digest,
        "operation_id": operation_id,
        "operation_generation": operation_generation,
        "idempotency_key": effect_idempotency_key,
    });
    let redeem_response = call_mounted_route(
        &app,
        route_request(
            Method::POST,
            &format!("/api/v1/approvals/exact-service/requests/{request_id}/redeem"),
            &token,
            Body::from(redeem_body.to_string()),
        ),
    )
    .await;
    let redeemed: serde_json::Value =
        serde_json::from_slice(&redeem_response).expect("parse exact redemption response");
    assert_eq!(redeemed["state"], "redeemed");
    assert_route_settled_count(&db, &mcp.slug, BillingMetric::Requests, 2).await;
    exercised_routes.insert("/api/v1/approvals/exact-service/requests/{request_id}/redeem");
    db.collection::<NotificationChannel>(NOTIFICATION_CHANNELS)
        .delete_one(doc! { "user_id": &owner_id })
        .await
        .expect("clear exact redemption approval policy before later route smoke cases");

    let node_mcp = insert_route_service(
        &db,
        &owner_id,
        "billing-node-mcp-route",
        "https://node-mcp-route.invalid",
        None,
        Some(&node.id),
    )
    .await;
    let node_mcp_body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {
            "name": "nyx__call_tool",
            "arguments": {
                "tool_name": "billing-node-mcp-route__request",
                "arguments": {
                    "method": "GET",
                    "path": "/mcp-query?tag=a&tag=b&name=Nyx%20ID&empty="
                },
            },
        },
    });
    let mut node_mcp_request = route_request(
        Method::POST,
        "/mcp",
        &token,
        Body::from(node_mcp_body.to_string()),
    );
    node_mcp_request
        .headers_mut()
        .insert("mcp-session-id", mcp_session_id.clone());
    let node_mcp_response = call_mounted_route(&app, node_mcp_request).await;
    assert!(
        !String::from_utf8_lossy(&node_mcp_response).contains("\"isError\":true"),
        "mounted node MCP tool call failed: {}",
        String::from_utf8_lossy(&node_mcp_response)
    );
    assert_route_settled(&db, &node_mcp.slug, BillingMetric::Requests).await;
    node_responder.await.expect("node HTTP responder");

    let (ws_downstream_url, ws_downstream) = start_billing_ws_downstream().await;
    let direct_ws = insert_route_service(
        &db,
        &owner_id,
        "billing-direct-ws-route",
        &ws_downstream_url,
        None,
        None,
    )
    .await;
    let (route_address, route_server) = start_mounted_route_server(app.clone()).await;
    exercise_mounted_websocket(
        &route_address,
        "/api/v1/proxy/s/billing-direct-ws-route/socket",
        &token,
    )
    .await;
    assert_route_settled(&db, &direct_ws.slug, BillingMetric::Bytes).await;

    let node_ws = insert_route_service(
        &db,
        &owner_id,
        "billing-node-ws-route",
        "https://node-ws-route.invalid",
        None,
        Some(&node.id),
    )
    .await;
    let (node_ws_tx, node_ws_rx) = mpsc::channel(256);
    state
        .node_ws_manager
        .register_connection(&node.id, node_ws_tx);
    let node_ws_responder = spawn_node_ws_responder(&state, &node.id, node_ws_rx);
    exercise_mounted_websocket(
        &route_address,
        "/api/v1/proxy/s/billing-node-ws-route/socket",
        &token,
    )
    .await;
    node_ws_responder.await.expect("node WebSocket responder");
    assert_route_settled(&db, &node_ws.slug, BillingMetric::Bytes).await;

    let (ssh_host, ssh_port, ssh_target) = start_billing_ssh_target().await;
    let (direct_ssh, direct_ssh_binding) = insert_ssh_route_service(
        &state,
        &owner_id,
        "billing-direct-ssh-route",
        &ssh_host,
        ssh_port,
        SshAuthMode::ProxyOnly,
        None,
    )
    .await;
    exercise_mounted_ssh_websocket(
        &route_address,
        &format!("/api/v1/ssh/{}", direct_ssh.id),
        &token,
    )
    .await;
    assert_route_settled(&db, &direct_ssh_binding.slug, BillingMetric::Bytes).await;
    exercised_routes.insert("/api/v1/ssh/{service_id}");
    ssh_target.await.expect("direct SSH target");

    let (node_tunnel, node_tunnel_binding) = insert_ssh_route_service(
        &state,
        &owner_id,
        "billing-node-ssh-route",
        "node-ssh-route.invalid",
        22,
        SshAuthMode::ProxyOnly,
        Some(&node.id),
    )
    .await;
    let (node_tunnel_tx, node_tunnel_rx) = mpsc::channel(256);
    state
        .node_ws_manager
        .register_connection(&node.id, node_tunnel_tx);
    let node_tunnel_responder = spawn_node_ssh_tunnel_responder(&state, &node.id, node_tunnel_rx);
    exercise_mounted_ssh_websocket(
        &route_address,
        &format!("/api/v1/ssh/{}", node_tunnel.id),
        &token,
    )
    .await;
    node_tunnel_responder
        .await
        .expect("node SSH tunnel responder");
    assert_route_settled(&db, &node_tunnel_binding.slug, BillingMetric::Bytes).await;

    let (node_shell, node_shell_binding) = insert_ssh_route_service(
        &state,
        &owner_id,
        "billing-node-shell-route",
        "node-shell-route.invalid",
        22,
        SshAuthMode::NodeKey,
        Some(&node.id),
    )
    .await;
    let (node_exec_tx, node_exec_rx) = mpsc::channel(256);
    state
        .node_ws_manager
        .register_connection(&node.id, node_exec_tx);
    let node_exec_responder = spawn_node_ssh_exec_responder(&state, &node.id, node_exec_rx);
    let exec_response = reqwest::Client::new()
        .post(format!(
            "http://{route_address}/api/v1/ssh/{}/exec",
            node_shell.id
        ))
        .bearer_auth(&token)
        .json(&serde_json::json!({
            "command": "echo route-boundary",
            "principal": "route",
            "timeout_secs": 5,
        }))
        .send()
        .await
        .expect("call mounted SSH exec route");
    let exec_status = exec_response.status();
    let exec_body = exec_response.text().await.unwrap_or_default();
    assert!(
        exec_status.is_success(),
        "mounted SSH exec returned {}: {}",
        exec_status,
        exec_body
    );
    node_exec_responder.await.expect("node SSH exec responder");
    assert_route_settled(&db, &node_shell_binding.slug, BillingMetric::Bytes).await;
    exercised_routes.insert("/api/v1/ssh/{service_id}/exec");

    let (_mcp_ssh, mcp_ssh_binding) = insert_ssh_route_service(
        &state,
        &owner_id,
        "billing-mcp-ssh-route",
        "mcp-ssh-route.invalid",
        22,
        SshAuthMode::Cert,
        Some(&node.id),
    )
    .await;
    let (mcp_ssh_tx, mcp_ssh_rx) = mpsc::channel(256);
    state
        .node_ws_manager
        .register_connection(&node.id, mcp_ssh_tx);
    let mcp_ssh_responder = spawn_node_ssh_cert_exec_responder(&state, &node.id, mcp_ssh_rx);
    let mcp_ssh_body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "tools/call",
        "params": {
            "name": "nyx__ssh_exec",
            "arguments": {
                "service": mcp_ssh_binding.slug,
                "command": "echo route-boundary",
                "principal": "route",
                "timeout_secs": 5,
            },
        },
    });
    let mut mcp_ssh_request = route_request(
        Method::POST,
        "/mcp",
        &token,
        Body::from(mcp_ssh_body.to_string()),
    );
    mcp_ssh_request
        .headers_mut()
        .insert("mcp-session-id", mcp_session_id);
    let mcp_ssh_response = call_mounted_route(&app, mcp_ssh_request).await;
    assert!(
        !String::from_utf8_lossy(&mcp_ssh_response).contains("\"isError\":true"),
        "mounted MCP SSH exec failed: {}",
        String::from_utf8_lossy(&mcp_ssh_response)
    );
    mcp_ssh_responder.await.expect("MCP SSH exec responder");
    assert_route_settled(&db, &mcp_ssh_binding.slug, BillingMetric::Requests).await;

    let (node_terminal_tx, node_terminal_rx) = mpsc::channel(256);
    state
        .node_ws_manager
        .register_connection(&node.id, node_terminal_tx);
    let node_terminal_responder = spawn_node_terminal_responder(&state, &node.id, node_terminal_rx);
    exercise_mounted_ssh_websocket(
        &route_address,
        &format!("/api/v1/ssh/{}/terminal?principal=route", node_shell.id),
        &token,
    )
    .await;
    node_terminal_responder
        .await
        .expect("node SSH terminal responder");
    assert_route_settled_count(&db, &node_shell_binding.slug, BillingMetric::Bytes, 2).await;
    exercised_routes.insert("/api/v1/ssh/{service_id}/terminal");

    assert_mounted_routes_are_exercised(&exercised_routes);

    route_server.abort();
    ws_downstream.abort();
    downstream.abort();
}

#[tokio::test]
async fn billing_service_lifecycle_regression() {
    let Some(db) = connect_test_database("billing_service_lifecycle").await else {
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
            .settle(&metered, usage, None, None)
            .await
            .expect("settle route");

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
            "settlement must create one charge for {}",
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
async fn buffered_route_preserves_success_when_settlement_failure_is_replayed() {
    let Some(db) = connect_test_database("billing_route_settle_recovery").await else {
        return;
    };
    create_usage_index(&db).await;
    insert_fresh_rates(&db).await;
    let owner_id = insert_owner(&db).await;
    let (downstream_url, forwarded_request, release_response, downstream) =
        start_controlled_billing_downstream().await;
    // Platform charging is opt-in per catalog service, so the recovery
    // route must resolve to a platform_billable catalog entry for the
    // wallet hold this test exercises.
    let mut recovery_catalog = crate::models::downstream_service::test_helpers::dummy_service();
    recovery_catalog.id = Uuid::new_v4().to_string();
    recovery_catalog.slug = "billing-recovery-catalog".to_string();
    recovery_catalog.base_url = downstream_url.clone();
    recovery_catalog.billing = Some(crate::models::service_billing::ServiceBilling {
        platform_billable: true,
        ..Default::default()
    });
    db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .insert_one(&recovery_catalog)
        .await
        .expect("insert recovery catalog service");
    let service = insert_route_service(
        &db,
        &owner_id,
        "billing-recovery-route",
        &downstream_url,
        Some(&recovery_catalog.id),
        None,
    )
    .await;
    let state = billing_route_state(db.clone(), Arc::new(FakeLago::default()), 0);
    let token = route_access_token(&state, &owner_id);
    let (_, private) = crate::routes::build_router(
        state.config.proxy_max_body_size,
        state.config.public_proxy_max_body_size,
    );
    let app = private.with_state(state.clone());

    let route = tokio::spawn(async move {
        app.oneshot(route_request(
            Method::GET,
            "/api/v1/proxy/s/billing-recovery-route/blocked",
            &token,
            Body::empty(),
        ))
        .await
        .expect("call mounted recovery route")
    });
    tokio::time::timeout(std::time::Duration::from_secs(5), forwarded_request)
        .await
        .expect("mounted route reached controlled downstream")
        .expect("controlled downstream reported request");

    let forwarded = wait_for_route_usage_status(&db, &service.slug, UsageStatus::Forwarded).await;
    assert!(forwarded.forwarded);
    assert!(!forwarded.released);
    assert_eq!(forwarded.quantity, None);

    let held_wallet = wallet(&db, &owner_id).await;
    db.collection::<BillingWallet>(BILLING_WALLET)
        .delete_one(doc! { "_id": &held_wallet.id })
        .await
        .expect("remove wallet to force settlement failure");

    release_response
        .send(())
        .expect("release controlled downstream response");
    let response = route.await.expect("mounted recovery route task");
    assert_eq!(response.status(), StatusCode::OK);
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("consume controlled downstream response");
    assert_eq!(body.as_ref(), br#"{"ok":true}"#);

    let failed = wait_for_route_usage_status(&db, &service.slug, UsageStatus::Failed).await;
    assert!(failed.forwarded);
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

    let stats = state
        .billing
        .reconciler()
        .run_once()
        .await
        .expect("reconcile failed settlement");
    assert_eq!(stats.recovered_settlements, 1);
    let recovered = wait_for_route_usage_status(&db, &service.slug, UsageStatus::Finalized).await;
    assert!(recovered.released);

    let replay = state
        .billing
        .reconciler()
        .run_once()
        .await
        .expect("replay completed reconcile sweep");
    assert_eq!(replay.recovered_settlements, 0);
    assert_eq!(
        db.collection::<UsageMeterRow>(USAGE_METER)
            .count_documents(doc! { "billing_request_id": &recovered.billing_request_id })
            .await
            .expect("count recovered route rows"),
        1
    );
    let saved_wallet = wallet(&db, &owner_id).await;
    assert_eq!(saved_wallet.reserved_credits, 0);
    assert_eq!(saved_wallet.pending_lago_debits, 1);
    downstream.abort();
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

async fn start_billing_downstream() -> (
    String,
    Arc<AtomicUsize>,
    Arc<AtomicUsize>,
    tokio::task::JoinHandle<()>,
) {
    async fn respond(
        request: Request<Body>,
        direct_hops: Arc<AtomicUsize>,
        agent_tool_hits: Arc<AtomicUsize>,
    ) -> axum::response::Response {
        let path = request.uri().path().to_string();
        if path == "/mcp-query" {
            assert_eq!(
                request.uri().query(),
                Some("tag=a&tag=b&name=Nyx%20ID&empty=")
            );
        }

        if path == "/agent-health" {
            agent_tool_hits.fetch_add(1, Ordering::SeqCst);
            return Json(serde_json::json!({
                "status": "healthy",
                "opensandbox_connected": true
            }))
            .into_response();
        }

        if path == "/direct/chat/completions" {
            direct_hops.fetch_add(1, Ordering::SeqCst);
            let body = to_bytes(request.into_body(), 512 * 1024)
                .await
                .expect("read direct assistant upstream request");
            let body: serde_json::Value =
                serde_json::from_slice(&body).expect("parse direct assistant upstream request");
            assert_eq!(body["stream"], true);
            assert_eq!(body["stream_options"]["include_usage"], true);
            assert_eq!(body["messages"][0]["role"], "system");
            let system_prompt = body["messages"][0]["content"]
                .as_str()
                .expect("agent system prompt");
            if system_prompt.contains("REPORT PHASE:") {
                return (
                    [(axum::http::header::CONTENT_TYPE, "text/event-stream")],
                    concat!(
                        "data: {\"id\":\"agent-report\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Chrono Sandbox is healthy.\"},\"finish_reason\":\"stop\"}]}\n\n",
                        "data: {\"id\":\"agent-report\",\"choices\":[],\"usage\":{\"prompt_tokens\":9,\"completion_tokens\":4,\"total_tokens\":13}}\n\n",
                        "data: [DONE]\n\n"
                    ),
                )
                    .into_response();
            }
            if body["tool_choice"] == "auto" {
                let has_tool_result = body["messages"]
                    .as_array()
                    .expect("agent messages array")
                    .iter()
                    .any(|message| message["role"] == "tool");
                let response = if has_tool_result {
                    let tool_result = body["messages"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .find(|message| message["role"] == "tool")
                        .unwrap();
                    assert_eq!(tool_result["tool_call_id"], "call-health");
                    let content = tool_result["content"].as_str().unwrap();
                    assert!(content.contains("healthy"));
                    assert!(content.contains("opensandbox_connected"));
                    concat!(
                        "data: {\"id\":\"agent-evidence\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Evidence collected.\"},\"finish_reason\":\"stop\"}]}\n\n",
                        "data: {\"id\":\"agent-evidence\",\"choices\":[],\"usage\":{\"prompt_tokens\":9,\"completion_tokens\":4,\"total_tokens\":13}}\n\n",
                        "data: [DONE]\n\n"
                    )
                } else {
                    concat!(
                        "data: {\"id\":\"agent-tools\",\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call-health\",\"type\":\"function\",\"function\":{\"name\":\"nyx_call_tool\",\"arguments\":\"{\\\"tool_name\\\":\\\"billing-proxy-route__agent_health\\\",\\\"arguments\\\":{}}\"}}]},\"finish_reason\":null}]}\n\n",
                        "data: {\"id\":\"agent-tools\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\n",
                        "data: {\"id\":\"agent-tools\",\"choices\":[],\"usage\":{\"prompt_tokens\":7,\"completion_tokens\":3,\"total_tokens\":10}}\n\n",
                        "data: [DONE]\n\n"
                    )
                };
                return (
                    [(axum::http::header::CONTENT_TYPE, "text/event-stream")],
                    response,
                )
                    .into_response();
            }
            return (
                [(axum::http::header::CONTENT_TYPE, "text/event-stream")],
                include_str!(
                    "../../frontend/src/lib/assistant/__fixtures__/chrono-llm-direct-stream.sse"
                ),
            )
                .into_response();
        }

        if path.contains("stream") {
            return (
                [(axum::http::header::CONTENT_TYPE, "text/event-stream")],
                concat!(
                    "data: {\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n\n",
                    "data: [DONE]\n\n"
                ),
            )
                .into_response();
        }

        Json(serde_json::json!({
            "id": "route-boundary",
            "choices": [{"message": {"role": "assistant", "content": "ok"}}],
            "usage": {
                "prompt_tokens": 2,
                "completion_tokens": 3,
                "total_tokens": 5,
            },
        }))
        .into_response()
    }

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind billing downstream");
    let address = listener.local_addr().expect("billing downstream address");
    let direct_hops = Arc::new(AtomicUsize::new(0));
    let agent_tool_hits = Arc::new(AtomicUsize::new(0));
    let app = Router::new().fallback(any({
        let direct_hops = direct_hops.clone();
        let agent_tool_hits = agent_tool_hits.clone();
        move |request| respond(request, direct_hops.clone(), agent_tool_hits.clone())
    }));
    let server = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("serve billing downstream");
    });
    (
        format!("http://{address}"),
        direct_hops,
        agent_tool_hits,
        server,
    )
}

async fn assert_direct_reported_usage(db: &mongodb::Database, service: &DownstreamService) {
    for _ in 0..100 {
        let usage = db
            .collection::<UsageMeterRow>(USAGE_METER)
            .find_one(doc! {
                "service_slug": &service.slug,
                "metric": "tokens",
                "status": "finalized",
                "forwarded": true,
                "released": true,
            })
            .await
            .expect("query direct assistant usage row");
        let provenance = db
            .collection::<AuditLog>(AUDIT_LOG)
            .find_one(doc! {
                "event_type": "llm_usage_reported",
                "event_data.service_id": &service.id,
            })
            .await
            .expect("query direct assistant reported-usage audit");

        if let (Some(usage), Some(provenance)) = (usage, provenance) {
            assert_eq!(usage.quantity, Some(149));
            let data = provenance.event_data.expect("reported usage audit data");
            assert_eq!(data["prompt_tokens"], 30);
            assert_eq!(data["completion_tokens"], 119);
            assert_eq!(data["total_tokens"], 149);
            assert_eq!(data["path"], "chat/completions");
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("direct assistant route did not settle from the fixture's reported usage provenance");
}

async fn assert_direct_settled_usage_count(
    db: &mongodb::Database,
    service: &DownstreamService,
    expected_count: u64,
) {
    for _ in 0..100 {
        let count = db
            .collection::<UsageMeterRow>(USAGE_METER)
            .count_documents(doc! {
                "service_slug": &service.slug,
                "metric": "tokens",
                "status": "finalized",
                "forwarded": true,
                "released": true,
            })
            .await
            .expect("count direct assistant usage rows");
        if count == expected_count {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("expected {expected_count} finalized direct-assistant token usage rows");
}

async fn start_controlled_billing_downstream() -> (
    String,
    oneshot::Receiver<()>,
    oneshot::Sender<()>,
    tokio::task::JoinHandle<()>,
) {
    let (forwarded_tx, forwarded_rx) = oneshot::channel();
    let (release_tx, release_rx) = oneshot::channel();
    let forwarded_tx = Arc::new(tokio::sync::Mutex::new(Some(forwarded_tx)));
    let release_rx = Arc::new(tokio::sync::Mutex::new(Some(release_rx)));
    let app = Router::new().fallback(any({
        let forwarded_tx = forwarded_tx.clone();
        let release_rx = release_rx.clone();
        move || {
            let forwarded_tx = forwarded_tx.clone();
            let release_rx = release_rx.clone();
            async move {
                if let Some(forwarded_tx) = forwarded_tx.lock().await.take() {
                    let _ = forwarded_tx.send(());
                }
                if let Some(release_rx) = release_rx.lock().await.take() {
                    let _ = release_rx.await;
                }
                Json(serde_json::json!({"ok": true}))
            }
        }
    }));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind controlled billing downstream");
    let address = listener
        .local_addr()
        .expect("controlled billing downstream address");
    let server = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("serve controlled billing downstream");
    });
    (
        format!("http://{address}"),
        forwarded_rx,
        release_tx,
        server,
    )
}

async fn start_billing_ws_downstream() -> (String, tokio::task::JoinHandle<()>) {
    async fn websocket(ws: WebSocketUpgrade) -> impl IntoResponse {
        ws.on_upgrade(|mut socket| async move {
            if let Some(Ok(message)) = socket.recv().await {
                let _ = socket.send(message).await;
            }
            let _ = socket.close().await;
        })
    }

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind billing WebSocket downstream");
    let address = listener
        .local_addr()
        .expect("billing WebSocket downstream address");
    let server = tokio::spawn(async move {
        axum::serve(listener, Router::new().route("/{*path}", get(websocket)))
            .await
            .expect("serve billing WebSocket downstream");
    });
    (format!("http://{address}"), server)
}

async fn start_billing_ssh_target() -> (String, u16, tokio::task::JoinHandle<()>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind billing SSH target");
    let address = listener.local_addr().expect("billing SSH target address");
    let server = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.expect("accept billing SSH tunnel");
        stream
            .write_all(b"SSH-2.0-NyxID-route-test\r\n")
            .await
            .expect("write billing SSH banner");
        let mut buffer = [0_u8; 256];
        if let Ok(read) = stream.read(&mut buffer).await
            && read > 0
        {
            let _ = stream.write_all(&buffer[..read]).await;
        }
    });
    (address.ip().to_string(), address.port(), server)
}

async fn start_mounted_route_server(app: Router) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind mounted route server");
    let address = listener.local_addr().expect("mounted route server address");
    let server = tokio::spawn(async move {
        axum::serve(
            listener,
            app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
        )
        .await
        .expect("serve mounted route app");
    });
    (address.to_string(), server)
}

async fn exercise_mounted_websocket(address: &str, path: &str, token: &str) {
    let mut request = format!("ws://{address}{path}")
        .into_client_request()
        .expect("build mounted WebSocket request");
    request.headers_mut().insert(
        "authorization",
        format!("Bearer {token}")
            .parse()
            .expect("WebSocket authorization header"),
    );
    let (mut socket, response) = tokio_tungstenite::connect_async(request)
        .await
        .expect("upgrade mounted billing WebSocket route");
    assert_eq!(response.status(), StatusCode::SWITCHING_PROTOCOLS);
    socket
        .send(tokio_tungstenite::tungstenite::Message::Text(
            "billing-route-frame".into(),
        ))
        .await
        .expect("send mounted WebSocket frame");
    while let Some(message) = socket.next().await {
        if let tokio_tungstenite::tungstenite::Message::Close(_) =
            message.expect("read mounted WebSocket frame")
        {
            break;
        }
    }
}

async fn exercise_mounted_ssh_websocket(address: &str, path: &str, token: &str) {
    let mut request = format!("ws://{address}{path}")
        .into_client_request()
        .expect("build mounted SSH WebSocket request");
    request.headers_mut().insert(
        "authorization",
        format!("Bearer {token}")
            .parse()
            .expect("SSH WebSocket authorization header"),
    );
    let (mut socket, response) = tokio_tungstenite::connect_async(request)
        .await
        .expect("upgrade mounted SSH route");
    assert_eq!(response.status(), StatusCode::SWITCHING_PROTOCOLS);
    while let Some(message) = socket.next().await {
        match message.expect("read mounted SSH WebSocket frame") {
            tokio_tungstenite::tungstenite::Message::Binary(_) => {
                socket
                    .send(tokio_tungstenite::tungstenite::Message::Binary(
                        b"route-boundary".to_vec().into(),
                    ))
                    .await
                    .expect("send mounted SSH WebSocket bytes");
            }
            tokio_tungstenite::tungstenite::Message::Text(_) => {}
            tokio_tungstenite::tungstenite::Message::Ping(payload) => {
                socket
                    .send(tokio_tungstenite::tungstenite::Message::Pong(payload))
                    .await
                    .expect("reply to mounted SSH WebSocket ping");
            }
            tokio_tungstenite::tungstenite::Message::Close(_) => break,
            _ => {}
        }
    }
}

fn billing_route_state(
    db: mongodb::Database,
    lago: Arc<FakeLago>,
    default_overdraft_cap_credits: i64,
) -> crate::AppState {
    let mut config = test_app_config();
    config.billing_enabled = true;
    config.billing_fail_closed = false;
    config.billing_rate_cache_ttl_secs = 900;
    config.billing_default_overdraft_cap_credits = default_overdraft_cap_credits;
    config.node_hmac_signing_enabled = false;
    let mut state = test_app_state_with_config(db.clone(), config.clone());
    state.billing = Arc::new(BillingService::new_with_lago(db, Arc::new(config), lago));
    state
}

fn route_access_token(state: &crate::AppState, owner_id: &str) -> String {
    crate::crypto::jwt::generate_access_token(
        &state.jwt_keys,
        &state.config,
        &Uuid::parse_str(owner_id).expect("route owner UUID"),
        "proxy",
        None,
        None,
        None,
        None,
        None,
    )
    .expect("generate route access token")
}

fn route_request(method: Method, uri: &str, token: &str, body: Body) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .header("authorization", format!("Bearer {token}"))
        .header("content-type", "application/json")
        .body(body)
        .expect("build mounted route request")
}

fn session_route_request(
    method: Method,
    uri: &str,
    session_token: &str,
    body: Body,
) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .header(
            "cookie",
            format!("{}={session_token}", crate::mw::auth::SESSION_COOKIE_NAME),
        )
        .header("content-type", "application/json")
        .header("origin", "http://localhost:3000")
        .body(body)
        .expect("build mounted session route request")
}

async fn call_mounted_route(app: &Router, request: Request<Body>) -> bytes::Bytes {
    let response = app
        .clone()
        .oneshot(request)
        .await
        .expect("call mounted billing route");
    let status = response.status();
    let body = to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("consume mounted route response");
    assert!(
        status.is_success(),
        "mounted route returned {status}: {}",
        String::from_utf8_lossy(&body)
    );
    body
}

async fn insert_route_service(
    db: &mongodb::Database,
    owner_id: &str,
    slug: &str,
    base_url: &str,
    catalog_service_id: Option<&str>,
    node_id: Option<&str>,
) -> UserService {
    let endpoint = test_user_endpoint(
        &Uuid::new_v4().to_string(),
        owner_id,
        slug,
        base_url,
        None,
        catalog_service_id,
    );
    let service = test_user_service(
        &Uuid::new_v4().to_string(),
        owner_id,
        slug,
        &endpoint.id,
        catalog_service_id,
        node_id,
    );
    db.collection::<UserEndpoint>(USER_ENDPOINTS)
        .insert_one(endpoint)
        .await
        .expect("insert route endpoint");
    db.collection::<UserService>(USER_SERVICES)
        .insert_one(&service)
        .await
        .expect("insert route service");
    service
}

async fn insert_route_node(state: &crate::AppState, owner_id: &str, name: &str) -> Node {
    let now = Utc::now();
    let node = Node {
        id: Uuid::new_v4().to_string(),
        user_id: owner_id.to_string(),
        name: name.to_string(),
        status: NodeStatus::Online,
        auth_token_hash: crate::crypto::token::hash_token("billing-route-node-token"),
        signing_secret_encrypted: None,
        signing_secret_hash: String::new(),
        last_heartbeat_at: Some(now),
        connected_at: Some(now),
        metadata: None,
        metrics: NodeMetrics::default(),
        is_active: true,
        created_at: now,
        updated_at: now,
    };
    state
        .db
        .collection::<Node>(NODES)
        .insert_one(&node)
        .await
        .expect("insert route node");
    node
}

async fn insert_ssh_route_service(
    state: &crate::AppState,
    owner_id: &str,
    slug: &str,
    host: &str,
    port: u16,
    auth_mode: SshAuthMode,
    node_id: Option<&str>,
) -> (DownstreamService, UserService) {
    let db = &state.db;
    let mut service = crate::models::downstream_service::test_helpers::dummy_service();
    service.id = Uuid::new_v4().to_string();
    service.slug = format!("_ssh_{}", Uuid::new_v4().simple());
    service.name = slug.to_string();
    service.base_url = format!("ssh://{host}:{port}");
    service.service_type = "ssh".to_string();
    service.visibility = "private".to_string();
    service.created_by = owner_id.to_string();
    let allowed_principals = vec!["route".to_string()];
    service.ssh_config = Some(
        crate::services::ssh_service::build_ssh_config(
            &state.encryption_keys,
            &service.id,
            None,
            crate::services::ssh_service::SshConfigInput {
                host,
                port,
                certificate_auth_enabled: auth_mode.certificate_auth_enabled(),
                ssh_auth_mode: Some(auth_mode),
                certificate_ttl_minutes: 30,
                allowed_principals: &allowed_principals,
            },
        )
        .await
        .expect("build route SSH config"),
    );
    db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .insert_one(&service)
        .await
        .expect("insert route SSH service");

    let endpoint = test_user_endpoint(
        &Uuid::new_v4().to_string(),
        owner_id,
        slug,
        &service.base_url,
        None,
        Some(&service.id),
    );
    let mut binding = test_user_service(
        &Uuid::new_v4().to_string(),
        owner_id,
        slug,
        &endpoint.id,
        Some(&service.id),
        node_id,
    );
    binding.service_type = "ssh".to_string();
    binding.ssh_auth_mode = auth_mode;
    db.collection::<UserEndpoint>(USER_ENDPOINTS)
        .insert_one(endpoint)
        .await
        .expect("insert route SSH endpoint");
    db.collection::<UserService>(USER_SERVICES)
        .insert_one(&binding)
        .await
        .expect("insert route SSH binding");
    (service, binding)
}

fn spawn_node_http_responder(
    state: &crate::AppState,
    node_id: &str,
    mut receiver: mpsc::Receiver<NodeOutboundMessage>,
    expected_requests: usize,
) -> tokio::task::JoinHandle<()> {
    let manager = state.node_ws_manager.clone();
    let node_id = node_id.to_string();
    tokio::spawn(async move {
        for _ in 0..expected_requests {
            let Some(NodeOutboundMessage::Text(message)) = receiver.recv().await else {
                panic!("expected outbound node proxy request");
            };
            let parsed: serde_json::Value =
                serde_json::from_str(&message).expect("parse outbound node proxy request");
            assert_eq!(parsed["type"].as_str(), Some("proxy_request"));
            let request_id = parsed["request_id"]
                .as_str()
                .expect("node proxy request id");
            let path = parsed["path"].as_str().unwrap_or_default();

            if path == "mcp-query" {
                assert_eq!(
                    parsed["query"].as_str(),
                    Some("tag=a&tag=b&name=Nyx%20ID&empty=")
                );
            }

            if path.contains("stream") {
                assert!(manager.deliver_stream_start(
                    &node_id,
                    request_id,
                    200,
                    vec![("content-type".to_string(), "text/event-stream".to_string())],
                ));
                manager.deliver_stream_chunk(
                    &node_id,
                    request_id,
                    b"data: {\"ok\":true}\n\n".to_vec(),
                );
                manager.deliver_stream_end(&node_id, request_id);
            } else {
                manager.deliver_proxy_response(
                    &node_id,
                    NodeProxyResponse {
                        request_id: request_id.to_string(),
                        status: 200,
                        headers: vec![("content-type".to_string(), "application/json".to_string())],
                        body: br#"{"ok":true}"#.to_vec(),
                    },
                );
            }
        }
    })
}

fn spawn_node_ws_responder(
    state: &crate::AppState,
    node_id: &str,
    mut receiver: mpsc::Receiver<NodeOutboundMessage>,
) -> tokio::task::JoinHandle<()> {
    let manager = state.node_ws_manager.clone();
    let node_id = node_id.to_string();
    tokio::spawn(async move {
        let Some(NodeOutboundMessage::Text(message)) = receiver.recv().await else {
            panic!("expected outbound node WebSocket open");
        };
        let parsed: serde_json::Value =
            serde_json::from_str(&message).expect("parse outbound node WebSocket open");
        assert_eq!(parsed["type"].as_str(), Some("ws_proxy_open"));
        let session_id = parsed["session_id"]
            .as_str()
            .expect("node WebSocket session id")
            .to_string();
        assert!(manager.deliver_ws_proxy_opened(&node_id, &session_id, None));

        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        manager.deliver_ws_proxy_text(
            &node_id,
            &session_id,
            "billing-node-route-frame".to_string(),
        );
        manager.deliver_ws_proxy_closed(
            &node_id,
            &session_id,
            Some(1000),
            Some("complete".to_string()),
        );
    })
}

fn spawn_node_ssh_tunnel_responder(
    state: &crate::AppState,
    node_id: &str,
    mut receiver: mpsc::Receiver<NodeOutboundMessage>,
) -> tokio::task::JoinHandle<()> {
    let manager = state.node_ws_manager.clone();
    let node_id = node_id.to_string();
    tokio::spawn(async move {
        let Some(NodeOutboundMessage::Text(message)) = receiver.recv().await else {
            panic!("expected outbound node SSH tunnel open");
        };
        let parsed: serde_json::Value =
            serde_json::from_str(&message).expect("parse outbound node SSH tunnel open");
        assert_eq!(parsed["type"].as_str(), Some("ssh_tunnel_open"));
        let session_id = parsed["session_id"]
            .as_str()
            .expect("node SSH tunnel session id")
            .to_string();
        assert!(manager.deliver_ssh_tunnel_opened(&node_id, &session_id));
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        manager.deliver_ssh_tunnel_data(
            &node_id,
            &session_id,
            b"SSH-2.0-NyxID-node-route\r\n".to_vec(),
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        manager.deliver_ssh_tunnel_closed(&node_id, &session_id, None);
    })
}

fn spawn_node_ssh_exec_responder(
    state: &crate::AppState,
    node_id: &str,
    mut receiver: mpsc::Receiver<NodeOutboundMessage>,
) -> tokio::task::JoinHandle<()> {
    let manager = state.node_ws_manager.clone();
    let node_id = node_id.to_string();
    tokio::spawn(async move {
        let Some(NodeOutboundMessage::Text(message)) = receiver.recv().await else {
            panic!("expected outbound node SSH exec");
        };
        let parsed: serde_json::Value =
            serde_json::from_str(&message).expect("parse outbound node SSH exec");
        assert_eq!(parsed["type"].as_str(), Some("ssh_node_exec_open"));
        let request_id = parsed["request_id"]
            .as_str()
            .expect("node SSH exec request id")
            .to_string();
        manager.deliver_ssh_node_exec_data(
            &node_id,
            &request_id,
            Some("stdout"),
            b"route-boundary\n".to_vec(),
        );
        manager.deliver_ssh_node_exec_close(&node_id, request_id, 0, 1, false);
    })
}

fn spawn_node_ssh_cert_exec_responder(
    state: &crate::AppState,
    node_id: &str,
    mut receiver: mpsc::Receiver<NodeOutboundMessage>,
) -> tokio::task::JoinHandle<()> {
    let manager = state.node_ws_manager.clone();
    let node_id = node_id.to_string();
    tokio::spawn(async move {
        let Some(NodeOutboundMessage::Text(message)) = receiver.recv().await else {
            panic!("expected outbound certificate SSH exec");
        };
        let parsed: serde_json::Value =
            serde_json::from_str(&message).expect("parse outbound certificate SSH exec");
        assert_eq!(parsed["type"].as_str(), Some("ssh_exec"));
        let request_id = parsed["request_id"]
            .as_str()
            .expect("certificate SSH exec request id")
            .to_string();
        manager.deliver_ssh_exec_result(
            &node_id,
            NodeSshExecResult {
                request_id,
                exit_code: 0,
                stdout: "route-boundary\n".to_string(),
                stderr: String::new(),
                duration_ms: 1,
                timed_out: false,
                error: None,
                error_code: None,
            },
        );
    })
}

fn spawn_node_terminal_responder(
    state: &crate::AppState,
    node_id: &str,
    mut receiver: mpsc::Receiver<NodeOutboundMessage>,
) -> tokio::task::JoinHandle<()> {
    let manager = state.node_ws_manager.clone();
    let node_id = node_id.to_string();
    tokio::spawn(async move {
        let Some(NodeOutboundMessage::Text(message)) = receiver.recv().await else {
            panic!("expected outbound node terminal open");
        };
        let parsed: serde_json::Value =
            serde_json::from_str(&message).expect("parse outbound node terminal open");
        assert_eq!(parsed["type"].as_str(), Some("web_terminal_open"));
        let session_id = parsed["session_id"]
            .as_str()
            .expect("node terminal session id")
            .to_string();
        assert!(manager.deliver_web_terminal_started(&node_id, &session_id));
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        manager.deliver_web_terminal_data(
            &node_id,
            &session_id,
            b"route-boundary terminal\n".to_vec(),
        );
        manager.deliver_web_terminal_closed(&node_id, &session_id, None, None);
    })
}

async fn insert_llm_route_service(
    db: &mongodb::Database,
    owner_id: &str,
    base_url: &str,
) -> DownstreamService {
    let now = Utc::now();
    let provider = ProviderConfig {
        id: Uuid::new_v4().to_string(),
        slug: "deepseek".to_string(),
        name: "DeepSeek".to_string(),
        description: None,
        provider_type: "api_key".to_string(),
        authorization_url: None,
        token_url: None,
        revocation_url: None,
        revocation: None,
        default_scopes: None,
        client_id_encrypted: None,
        client_secret_encrypted: None,
        supports_pkce: false,
        device_code_url: None,
        device_token_url: None,
        device_verification_url: None,
        hosted_callback_url: None,
        api_key_instructions: None,
        api_key_url: None,
        icon_url: None,
        documentation_url: None,
        is_active: true,
        credential_mode: "admin".to_string(),
        token_endpoint_auth_method: "client_secret_post".to_string(),
        extra_auth_params: None,
        device_code_format: "rfc8628".to_string(),
        client_id_param_name: None,
        requires_gateway_url: false,
        created_by: "billing-route-test".to_string(),
        revocation_seed_version: 0,
        created_at: now,
        updated_at: now,
    };
    db.collection::<ProviderConfig>(PROVIDER_CONFIGS)
        .insert_one(&provider)
        .await
        .expect("insert route LLM provider");

    let mut catalog = crate::models::downstream_service::test_helpers::dummy_service();
    catalog.id = Uuid::new_v4().to_string();
    catalog.slug = "llm-deepseek".to_string();
    catalog.name = "DeepSeek route boundary".to_string();
    catalog.base_url = base_url.to_string();
    catalog.provider_config_id = Some(provider.id.clone());
    catalog.streaming_supported = true;
    db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .insert_one(&catalog)
        .await
        .expect("insert route LLM catalog service");

    insert_route_service(
        db,
        owner_id,
        "billing-llm-route",
        base_url,
        Some(&catalog.id),
        None,
    )
    .await;
    db.collection::<UserProviderToken>(USER_PROVIDER_TOKENS)
        .insert_one(UserProviderToken {
            id: Uuid::new_v4().to_string(),
            user_id: owner_id.to_string(),
            provider_config_id: provider.id,
            connection_id: None,
            credential_user_id: None,
            token_type: "api_key".to_string(),
            access_token_encrypted: None,
            refresh_token_encrypted: None,
            token_scopes: None,
            expires_at: None,
            api_key_encrypted: None,
            status: "active".to_string(),
            last_refreshed_at: None,
            last_used_at: None,
            error_message: None,
            label: Some("route boundary".to_string()),
            metadata: None,
            gateway_url: None,
            created_at: now,
            updated_at: now,
        })
        .await
        .expect("insert route LLM readiness token");
    catalog
}

async fn assert_route_settled(db: &mongodb::Database, service_slug: &str, metric: BillingMetric) {
    assert_route_settled_count(db, service_slug, metric, 1).await;
}

async fn assert_route_settled_count(
    db: &mongodb::Database,
    service_slug: &str,
    metric: BillingMetric,
    expected_count: u64,
) {
    for _ in 0..100 {
        let count = db
            .collection::<UsageMeterRow>(USAGE_METER)
            .count_documents(doc! {
                "service_slug": service_slug,
                "metric": bson::to_bson(&metric).expect("serialize billing metric"),
                "status": "finalized",
                "forwarded": true,
                "released": true,
            })
            .await
            .expect("count settled route usage");
        if count == expected_count {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("expected {expected_count} finalized {metric:?} rows for route {service_slug}");
}

fn assert_route_inventory_matches_router() {
    let mounted_specs = crate::routes::mounted_billing_route_inventory();
    let mounted: BTreeSet<_> = mounted_specs.iter().copied().collect();
    let classified: BTreeSet<_> = BILLING_ROUTE_INVENTORY.iter().copied().collect();
    assert_eq!(
        mounted, classified,
        "mounted routes and the Metered/Exempt billing inventory must stay identical"
    );
    assert_eq!(mounted.len(), mounted_specs.len());
}

fn assert_http_egress_classification_is_fail_closed() {
    let request_with_policy = |policy| {
        let mut request = Request::new(Body::empty());
        if let Some(policy) = policy {
            request.extensions_mut().insert(policy);
        }
        request
    };

    let classified = request_with_policy(Some(BillingRoutePolicy::Metered(BillingIngress::Proxy)));
    assert!(crate::handlers::proxy::enforce_proxy_billing_classification(&classified).is_ok());
    for policy in [
        None,
        Some(BillingRoutePolicy::Exempt("test exemption")),
        Some(BillingRoutePolicy::Metered(BillingIngress::LlmGateway)),
    ] {
        let request = request_with_policy(policy);
        assert!(
            matches!(
                crate::handlers::proxy::enforce_proxy_billing_classification(&request),
                Err(AppError::Internal(_))
            ),
            "unclassified, exempt, or mismatched routes must fail before proxy egress"
        );
    }

    for ingress in [BillingIngress::LlmGateway, BillingIngress::LlmProvider] {
        let classified = request_with_policy(Some(BillingRoutePolicy::Metered(ingress)));
        assert!(
            crate::handlers::llm_gateway::enforce_llm_billing_classification(&classified, ingress,)
                .is_ok()
        );
        for policy in [
            None,
            Some(BillingRoutePolicy::Exempt("test exemption")),
            Some(BillingRoutePolicy::Metered(BillingIngress::Proxy)),
        ] {
            let request = request_with_policy(policy);
            assert!(
                matches!(
                    crate::handlers::llm_gateway::enforce_llm_billing_classification(
                        &request, ingress,
                    ),
                    Err(AppError::Internal(_))
                ),
                "unclassified, exempt, or mismatched routes must fail before LLM egress"
            );
        }
    }

    let exempt = request_with_policy(Some(BillingRoutePolicy::Exempt("test exemption")));
    assert!(
        crate::services::billing::route_inventory::enforce_billing_exempt_egress_classification(
            exempt.extensions().get::<BillingRoutePolicy>().copied(),
        )
        .is_ok()
    );
}

fn assert_mounted_routes_are_exercised(exercised_routes: &BTreeSet<&str>) {
    let mounted_routes: BTreeSet<&str> = crate::routes::mounted_billing_route_inventory()
        .iter()
        .filter_map(|entry| match entry.policy {
            BillingRoutePolicy::Metered(_) => Some(entry.route),
            BillingRoutePolicy::Exempt(_) => None,
        })
        .collect();
    assert_eq!(
        exercised_routes, &mounted_routes,
        "every mounted metered route must cross its real route boundary in the smoke test"
    );
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
    // These cases exercise the platform charging gate, which is an admin
    // opt-in per service.
    let billing = crate::models::service_billing::ServiceBilling {
        platform_billable: true,
        ..Default::default()
    };
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
        Some(&billing),
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

async fn usage_row_for_service(db: &mongodb::Database, service_slug: &str) -> UsageMeterRow {
    db.collection::<UsageMeterRow>(USAGE_METER)
        .find_one(doc! { "service_slug": service_slug })
        .await
        .expect("query route usage row")
        .expect("route usage row exists")
}

async fn wait_for_route_usage_status(
    db: &mongodb::Database,
    service_slug: &str,
    expected: UsageStatus,
) -> UsageMeterRow {
    for _ in 0..100 {
        let row = usage_row_for_service(db, service_slug).await;
        if row.status == expected {
            return row;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    panic!("route {service_slug} did not reach {expected:?}");
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

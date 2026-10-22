//! Real Mongo and real proxy/MCP dispatch coverage for an unset preference.
use super::*;
use crate::models::{
    approval_request::{ApprovalRequest, COLLECTION_NAME as REQUESTS},
    downstream_service::{DownstreamService, InferenceWireProtocol, ServiceInference},
    notification_channel::{COLLECTION_NAME as CHANNELS, NotificationChannel},
    service_endpoint::{EndpointRisk, ServiceEndpoint},
    user::UserType,
};
use crate::services::approval_policy::GlobalApprovalMode;
use crate::test_utils::{connect_test_database, test_app_state, test_auth_user, test_user};
use axum::{Json, Router, extract::Path, routing::any};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

struct Fixture {
    state: AppState,
    owner: String,
    service: DownstreamService,
    forwarded: Arc<AtomicUsize>,
    server: tokio::task::JoinHandle<()>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}
impl Fixture {
    async fn new() -> Self {
        let db = connect_test_database("approval_writes_only_dispatch")
            .await
            .expect("real Mongo required");
        let owner = uuid::Uuid::new_v4().to_string();
        db.collection(USERS)
            .insert_one(test_user(&owner, UserType::Person))
            .await
            .unwrap();
        let forwarded = Arc::new(AtomicUsize::new(0));
        let observed = forwarded.clone();
        let upstream = Router::new().route(
            "/{*path}",
            any(move || {
                observed.fetch_add(1, Ordering::SeqCst);
                async { Json(serde_json::json!({"ok":true})) }
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let base_url = format!("http://{}", listener.local_addr().unwrap());
        let server = tokio::spawn(async move {
            axum::serve(listener, upstream).await.unwrap();
        });
        let mut service = crate::models::downstream_service::test_helpers::dummy_service();
        service.id = uuid::Uuid::new_v4().to_string();
        service.slug = "approval-contract-test".into();
        service.base_url = base_url;
        service.service_category = "internal".into();
        service.auth_method = "none".into();
        service.requires_user_credential = false;
        service.forward_access_token = false;
        service.inject_delegation_token = false;
        service.credential_encrypted.clear();
        db.collection::<DownstreamService>("downstream_services")
            .insert_one(&service)
            .await
            .unwrap();
        Self {
            state: test_app_state(db),
            owner,
            service,
            forwarded,
            server,
        }
    }
    async fn http(
        &self,
        method: &str,
        path: &str,
        body: serde_json::Value,
    ) -> crate::errors::AppResult<Response> {
        let mut auth = test_auth_user(&self.owner);
        auth.auth_method = AuthMethod::ApiKey;
        auth.api_key_id = Some("default-policy-agent".into());
        let mut request = Request::builder()
            .method(method)
            .uri("/api/v1/proxy/test")
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        request.extensions_mut().insert(
            crate::services::billing::route_inventory::BillingRoutePolicy::Metered(
                crate::services::billing::BillingIngress::Proxy,
            ),
        );
        Box::pin(crate::handlers::proxy::execute_admin_proxy(
            &self.state,
            &auth,
            &self.service.id,
            path,
            request,
            vec![],
            &mut String::new(),
        ))
        .await
    }
    async fn reject_pending(&self, verb: &str) {
        let pending = tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                if let Some(row) = self
                    .state
                    .db
                    .collection::<ApprovalRequest>(REQUESTS)
                    .find_one(doc! {"service_id":&self.service.id,"status":"pending"})
                    .await
                    .unwrap()
                {
                    break row;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("dispatch must persist a pending approval");
        assert_eq!(serde_json::to_value(&pending.verb).unwrap(), verb);
        assert_eq!(pending.user_id, self.owner);
        assert_eq!(pending.requester_type, "api_key");
        let before = self.forwarded.load(Ordering::SeqCst);
        let Json(decided) = crate::handlers::approvals::decide_request(
            State(self.state.clone()),
            test_auth_user(&self.owner),
            Path(pending.id),
            Default::default(),
            HeaderMap::new(),
            Json(crate::handlers::approvals::DecideRequest {
                approved: false,
                duration_sec: None,
            }),
        )
        .await
        .unwrap();
        assert_eq!(decided.status, "rejected");
        assert_eq!(
            self.forwarded.load(Ordering::SeqCst),
            before,
            "a rejected operation must not forward"
        );
    }
    async fn set_mode(&self, mode: GlobalApprovalMode) {
        let Json(settings) = crate::handlers::notifications::update_settings(
            State(self.state.clone()),
            test_auth_user(&self.owner),
            Json(
                crate::handlers::notifications::UpdateNotificationSettingsRequest {
                    telegram_enabled: None,
                    push_enabled: None,
                    approval_required: None,
                    global_approval_mode: Some(mode),
                    approval_timeout_secs: None,
                    grant_expiry_days: None,
                },
            ),
        )
        .await
        .unwrap();
        assert_eq!(settings.global_approval_mode, mode);
    }
    async fn endpoint(&self, method: &str, name: &str, risk: Option<EndpointRisk>) {
        let now = chrono::Utc::now();
        self.state
            .db
            .collection::<ServiceEndpoint>("service_endpoints")
            .insert_one(ServiceEndpoint {
                data_scope: None,
                cost_class: None,
                execution: Default::default(),
                publication: Default::default(),
                async_operation: None,
                id: uuid::Uuid::new_v4().to_string(),
                service_id: self.service.id.clone(),
                name: name.into(),
                description: None,
                method: method.into(),
                path: format!("/{name}"),
                target_id: None,
                parameters: None,
                request_body_schema: None,
                request_content_type: None,
                request_body_required: false,
                response_description: None,
                response: Default::default(),
                risk,
                supports_idempotency_key: false,
                is_active: true,
                operation_generation: 1,
                created_at: now,
                updated_at: now,
            })
            .await
            .unwrap();
    }
    async fn mcp(&self, name: &str) -> Response {
        let mut auth = McpAuthContext::user(self.owner.clone(), AuthMethod::ApiKey);
        auth.api_key_id = Some("default-policy-agent".into());
        auth.is_api_key = true;
        let request = JsonRpcRequest {
            jsonrpc: JSONRPC_VERSION.into(),
            id: Some(serde_json::json!(1)),
            method: "tools/call".into(),
            params: Some(
                serde_json::json!({"name":format!("{}__{name}",self.service.slug),"arguments":{}}),
            ),
        };
        let permit =
            crate::services::billing::route_inventory::enforce_billing_egress_classification(
                Some(
                    crate::services::billing::route_inventory::BillingRoutePolicy::Metered(
                        crate::services::billing::BillingIngress::Mcp,
                    ),
                ),
                crate::services::billing::BillingIngress::Mcp,
            )
            .unwrap();
        Box::pin(handle_tools_call(
            &self.state,
            &auth,
            None,
            &request,
            false,
            permit,
        ))
        .await
    }
}

#[tokio::test]
async fn unset_global_http_read_write_delete_and_explicit_choices_use_real_dispatch() {
    let f = Fixture::new().await;
    assert_eq!(
        f.http("GET", "items", serde_json::json!({}))
            .await
            .unwrap()
            .status(),
        StatusCode::OK
    );
    assert_eq!(
        f.state
            .db
            .collection::<ApprovalRequest>(REQUESTS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        f.state
            .db
            .collection::<NotificationChannel>(CHANNELS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    for (method, verb) in [("POST", "write"), ("DELETE", "destructive")] {
        let before = f.forwarded.load(Ordering::SeqCst);
        let (result, ()) = tokio::time::timeout(Duration::from_secs(15), async {
            tokio::join!(
                f.http(method, "items", serde_json::json!({})),
                f.reject_pending(verb)
            )
        })
        .await
        .unwrap();
        assert!(
            matches!(result, Err(crate::errors::AppError::ApprovalFailed { .. })),
            "{result:?}"
        );
        assert_eq!(f.forwarded.load(Ordering::SeqCst), before);
    }
    // Approval creation and a settings GET both leave the default unset.
    let Json(settings) = crate::handlers::notifications::get_settings(
        State(f.state.clone()),
        test_auth_user(&f.owner),
    )
    .await
    .unwrap();
    assert_eq!(
        settings.global_approval_mode,
        GlobalApprovalMode::WritesOnly
    );
    let row = f
        .state
        .db
        .collection::<mongodb::bson::Document>(CHANNELS)
        .find_one(doc! {"user_id":&f.owner})
        .await
        .unwrap()
        .unwrap();
    assert!(!row.contains_key("approval_required"));
    f.http("GET", "items", serde_json::json!({})).await.unwrap();
    f.set_mode(GlobalApprovalMode::All).await;
    let (result, ()) = tokio::join!(
        f.http("GET", "items", serde_json::json!({})),
        f.reject_pending("read")
    );
    assert!(matches!(
        result,
        Err(crate::errors::AppError::ApprovalFailed { .. })
    ));
    f.set_mode(GlobalApprovalMode::Off).await;
    f.http("POST", "items", serde_json::json!({}))
        .await
        .unwrap();
    f.set_mode(GlobalApprovalMode::WritesOnly).await;
    f.http("GET", "items", serde_json::json!({})).await.unwrap();
    let (result, ()) = tokio::join!(
        f.http("POST", "items", serde_json::json!({})),
        f.reject_pending("write")
    );
    assert!(matches!(
        result,
        Err(crate::errors::AppError::ApprovalFailed { .. })
    ));
}

#[tokio::test]
async fn unset_global_mcp_uses_trusted_read_contract_and_gates_write_tool() {
    let f = Fixture::new().await;
    f.endpoint("GET", "list_items", None).await;
    f.endpoint("POST", "search_items", Some(EndpointRisk::Read))
        .await;
    f.endpoint("POST", "create_item", Some(EndpointRisk::Write))
        .await;
    for name in ["list_items", "search_items"] {
        let response = f.mcp(name).await;
        let body = axum::body::to_bytes(response.into_body(), 8192)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_ne!(json["result"]["isError"], true, "{json}");
    }
    assert_eq!(f.forwarded.load(Ordering::SeqCst), 2);
    assert_eq!(
        f.state
            .db
            .collection::<ApprovalRequest>(REQUESTS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    let (response, ()) = tokio::time::timeout(Duration::from_secs(15), async {
        tokio::join!(f.mcp("create_item"), f.reject_pending("write"))
    })
    .await
    .unwrap();
    let bytes = axum::body::to_bytes(response.into_body(), 8192)
        .await
        .unwrap();
    let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["result"]["isError"], true, "{json}");
    assert_eq!(f.forwarded.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn unset_global_catalog_inference_allows_stateless_chat_but_gates_provider_storage() {
    let mut f = Fixture::new().await;
    f.service.inference = Some(ServiceInference {
        wire_protocol: InferenceWireProtocol::OpenaiCompletions,
        model_list: true,
        realtime: false,
        voice: None,
    });
    f.state
        .db
        .collection::<DownstreamService>("downstream_services")
        .replace_one(doc! {"_id":&f.service.id}, &f.service)
        .await
        .unwrap();
    // This deliberately has no llm-* name; the catalog contract is authority.
    let request =
        serde_json::json!({"model":"test-model","messages":[{"role":"user","content":"hello"}]});
    f.http("POST", "v1/chat/completions", request.clone())
        .await
        .unwrap();
    assert_eq!(f.forwarded.load(Ordering::SeqCst), 1);
    assert_eq!(
        f.state
            .db
            .collection::<ApprovalRequest>(REQUESTS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    let mut stored = request;
    stored["store"] = serde_json::json!(true);
    let (result, ()) = tokio::time::timeout(Duration::from_secs(15), async {
        tokio::join!(
            f.http("POST", "v1/chat/completions", stored),
            f.reject_pending("write")
        )
    })
    .await
    .unwrap();
    assert!(matches!(
        result,
        Err(crate::errors::AppError::ApprovalFailed { .. })
    ));
    assert_eq!(f.forwarded.load(Ordering::SeqCst), 1);
}

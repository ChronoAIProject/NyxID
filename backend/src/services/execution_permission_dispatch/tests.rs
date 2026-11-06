use super::*;
use crate::{
    models::{
        agent_operation_scope::{AgentOperationScope, ScopedOperation},
        api_key::ApiKey,
        downstream_service::ProxyOperationRule,
        user::UserType,
    },
    test_utils,
};
use axum::{
    Extension, Router,
    body::Body,
    http::{Request, StatusCode},
    routing::{any, post},
};
use tower::ServiceExt;
use wiremock::{Mock, MockServer, ResponseTemplate, matchers::any as any_request};

pub(super) struct Fixture {
    pub(super) state: crate::AppState,
    pub(super) owner: String,
    pub(super) key: String,
    pub(super) raw: String,
    pub(super) connection: String,
    pub(super) catalog: String,
    pub(super) server: MockServer,
}
impl Fixture {
    async fn new(name: &str) -> Self {
        let db = test_utils::connect_transaction_test_database(name).await;
        super::super::security_readiness_service::ready_test_replica(&db).await;
        let owner = Uuid::new_v4().to_string();
        db.collection("users")
            .insert_one(test_utils::test_user(&owner, UserType::Person))
            .await
            .unwrap();
        let server = MockServer::start().await;
        Mock::given(any_request())
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok":true})))
            .mount(&server)
            .await;
        let mut service = test_utils::test_auto_connected_catalog_service();
        service.base_url = server.uri();
        service.slug = "execution-fixture".into();
        let catalog = service.id.clone();
        db.collection("downstream_services")
            .insert_one(service)
            .await
            .unwrap();
        let endpoint = Uuid::new_v4().to_string();
        let connection = Uuid::new_v4().to_string();
        db.collection("user_endpoints")
            .insert_one(test_utils::test_user_endpoint(
                &endpoint,
                &owner,
                "Fixture",
                &server.uri(),
                None,
                None,
            ))
            .await
            .unwrap();
        db.collection("user_services")
            .insert_one(test_utils::test_user_service(
                &connection,
                &owner,
                "execution-fixture",
                &endpoint,
                Some(&catalog),
                None,
            ))
            .await
            .unwrap();
        let created = super::super::key_service::create_api_key(
            &db,
            &owner,
            "Execution fixture",
            "proxy",
            None,
            None,
            None,
            None,
            Some(true),
            Some(false),
            Some(true),
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();
        db.collection("service_approval_configs")
            .insert_one(
                crate::models::service_approval_config::ServiceApprovalConfig {
                    id: Uuid::new_v4().to_string(),
                    user_id: owner.clone(),
                    service_id: catalog.clone(),
                    service_name: "Fixture".into(),
                    approval_required: true,
                    approval_mode: crate::models::service_approval_config::ApprovalMode::PerRequest,
                    rules: vec![],
                    default_effect: None,
                    created_at: Utc::now(),
                    updated_at: Utc::now(),
                },
            )
            .await
            .unwrap();
        let state = test_utils::test_app_state(db);
        Self {
            state,
            owner,
            key: created.id,
            raw: created.full_key,
            connection,
            catalog,
            server,
        }
    }
    async fn enroll(&self, enabled: bool) {
        super::super::feature_flag_admin_service::mutate(
            &self.state.db,
            super::super::feature_flag_service::EXECUTION_PERMISSIONS_FLAG_KEY,
            super::super::feature_flag_admin_service::Mutation::Override {
                kind: crate::models::feature_flag_override::FlagTargetKind::User,
                target_id: Some(self.owner.clone()),
                enabled: Some(enabled),
            },
            &super::super::audit_service::AuditActor {
                user_id: self.owner.clone(),
                api_key_id: None,
                api_key_name: None,
                ip_address: None,
                user_agent: None,
            },
        )
        .await
        .unwrap();
    }
    pub(super) async fn auth(&self) -> crate::mw::auth::AuthUser {
        let key = self
            .state
            .db
            .collection::<ApiKey>("api_keys")
            .find_one(doc! {"_id":&self.key})
            .await
            .unwrap()
            .unwrap();
        crate::mw::auth::api_key_auth_user(&self.state.db, &key, None, None, None)
            .await
            .unwrap()
    }
    async fn grant(&self) -> ApprovalGrant {
        let context = RequestContext::enrolled(Some(1), &self.owner, &self.key, None).unwrap();
        let binding = binding(
            &self.state.db,
            &self.state.encryption_keys,
            &context,
            None,
            &self.key,
            &self.connection,
            &self.owner,
        )
        .await
        .unwrap();
        let mut grant = contract::tests::fixture();
        grant.user_id = self.owner.clone();
        grant.requester_id = self.key.clone();
        grant.service_id = self.catalog.clone();
        let p = grant.execution.as_mut().unwrap().v1.as_mut().unwrap();
        p.subject = Subject {
            actor_user_id: self.owner.clone(),
            principal: Principal::ApiKey(self.key.clone()),
            lane: ExecutionLane::Owner,
        };
        p.binding = binding;
        p.context = ExecutionContext {
            audience: Audience::Private {
                user_id: self.owner.clone(),
            },
            conversation: ConversationScope::SameAudience,
        };
        p.operations = AgentOperationScope {
            revision: 1,
            catalog_service_id: Some(self.catalog.clone()),
            operations: vec![ScopedOperation {
                endpoint_id: None,
                rule: ProxyOperationRule {
                    method: "GET".into(),
                    path_template: "/status".into(),
                    ..Default::default()
                },
                ..Default::default()
            }],
        };
        grant
    }
    pub(super) async fn store(&self, grant: &ApprovalGrant) {
        self.state
            .db
            .collection::<ApprovalGrant>("approval_grants")
            .insert_one(grant)
            .await
            .unwrap();
    }
    fn router(&self) -> Router {
        use super::super::billing::{BillingIngress, route_inventory::BillingRoutePolicy};
        Router::new()
            .route(
                "/proxy/s/{slug}/{*path}",
                any(crate::handlers::proxy::proxy_request_by_slug).layer(Extension(
                    BillingRoutePolicy::Metered(BillingIngress::Proxy),
                )),
            )
            .route(
                "/proxy/{service_id}/{*path}",
                any(crate::handlers::proxy::proxy_request).layer(Extension(
                    BillingRoutePolicy::Metered(BillingIngress::Proxy),
                )),
            )
            .route(
                "/mcp",
                post(crate::handlers::mcp_transport::mcp_post)
                    .layer(Extension(BillingRoutePolicy::Metered(BillingIngress::Mcp))),
            )
            .layer(axum::middleware::from_fn(
                crate::mw::telemetry::telemetry_mw,
            ))
            .with_state(self.state.clone())
    }
    pub(super) async fn call(&self, path: &str) -> (StatusCode, String) {
        let response = self
            .router()
            .oneshot(
                Request::builder()
                    .uri(path)
                    .header("x-api-key", &self.raw)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let text = String::from_utf8(
            axum::body::to_bytes(response.into_body(), 1024 * 1024)
                .await
                .unwrap()
                .to_vec(),
        )
        .unwrap();
        (status, text)
    }
    pub(super) async fn no_prompts(&self) {
        for collection in [
            "assistant_acknowledgements",
            "assistant_agent_consents",
            "approval_requests",
        ] {
            assert_eq!(
                self.state
                    .db
                    .collection::<Document>(collection)
                    .count_documents(doc! {})
                    .await
                    .unwrap(),
                0,
                "{collection}"
            );
        }
        assert_eq!(
            self.state
                .db
                .collection::<Document>("notification_channels")
                .count_documents(doc! {})
                .await
                .unwrap(),
            0,
            "Covered never creates notification defaults"
        );
    }
    async fn target(&self) -> proxy_service::ProxyTarget {
        proxy_service::read_proxy_authority_snapshot_with_access(
            &self.state.db,
            &self.state.encryption_keys,
            &self.owner,
            &self.connection,
            None,
            None,
        )
        .await
        .unwrap()
        .unwrap()
        .target
    }
    async fn prepare(
        &self,
        context: &Arc<RequestContext>,
        target: &proxy_service::ProxyTarget,
    ) -> Arc<Permit> {
        prepare(
            &self.state,
            Some(context),
            None,
            &self.key,
            Effect {
                connection: &self.connection,
                owner: &self.owner,
                target,
                method: "GET",
                path: "status",
                query: None,
                body: b"",
                content_type: None,
                headers: &[],
                endpoint: None,
                floors: &[ExecutionFloor::PersonalServiceApproval],
            },
        )
        .await
        .unwrap()
        .expect("covering permission")
    }
    pub(super) async fn cleanup(self) {
        self.state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn proxy_uuid_and_slug_covered_have_no_prompts_and_one_attempt_each() {
    use tracing::instrument::WithSubscriber;
    let captured = migration_tests::Capture::default();
    let f = Fixture::new("execution_proxy_covered").await;
    f.enroll(true).await;
    f.store(&f.grant().await).await;
    for route in [
        "/proxy/s/execution-fixture/status".into(),
        format!("/proxy/{}/status", f.catalog),
    ] {
        let (status, body) = f.call(&route).with_subscriber(captured.subscriber()).await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }
    assert_eq!(f.server.received_requests().await.unwrap().len(), 2);
    assert_eq!(
        captured.0.lock().unwrap().len(),
        2,
        "one decision per HTTP/MCP occurrence"
    );
    assert!(
        captured
            .0
            .lock()
            .unwrap()
            .iter()
            .all(|event| event["cause"] == "permission_covered")
    );
    f.no_prompts().await;
    f.cleanup().await;
}

#[tokio::test]
async fn revoked_and_explicit_deny_are_prohibited_before_cards_or_provider_effects() {
    let f = Fixture::new("execution_proxy_prohibited").await;
    f.enroll(true).await;
    let mut grant = f.grant().await;
    grant.revoked = true;
    f.store(&grant).await;
    let (status, body) = f.call("/proxy/s/execution-fixture/status").await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    f.state
        .db
        .collection::<Document>("approval_grants")
        .delete_many(doc! {})
        .await
        .unwrap();
    f.state
        .db
        .collection::<Document>("service_approval_configs")
        .update_many(doc! {}, doc! {"$set":{"default_effect":"deny"}})
        .await
        .unwrap();
    let (status, body) = f.call("/proxy/s/execution-fixture/status").await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(f.server.received_requests().await.unwrap().is_empty());
    f.no_prompts().await;
    f.cleanup().await;
}

#[tokio::test]
async fn disable_and_account_replacement_fence_an_admitted_reusable_permission() {
    let f = Fixture::new("execution_egress_drain").await;
    f.enroll(true).await;
    f.store(&f.grant().await).await;
    let target = f.target().await;
    let context = RequestContext::enrolled(Some(1), &f.owner, &f.key, None).unwrap();
    let permit = f.prepare(&context, &target).await;
    f.enroll(false).await;
    assert!(permit.before_send(&target).await.is_err());
    f.enroll(true).await;
    f.state
        .db
        .collection::<Document>("user_services")
        .update_one(
            doc! {"_id":&f.connection},
            doc! {"$set":{"api_key_id":Uuid::new_v4().to_string()}},
        )
        .await
        .unwrap();
    assert!(permit.before_send(&target).await.is_err());
    assert!(f.server.received_requests().await.unwrap().is_empty());
    f.cleanup().await;
}

#[tokio::test]
async fn twenty_duplicate_operation_claims_send_once_and_uncertain_never_replays() {
    let f = Fixture::new("execution_once_race").await;
    f.enroll(true).await;
    let target = f.target().await;
    // Prepare a reusable contract to obtain the same server-owned operation
    // identity/digest that #2195 will bind at human approval, never from headers.
    let grant = f.grant().await;
    f.store(&grant).await;
    let context = RequestContext::enrolled(Some(1), &f.owner, &f.key, None).unwrap();
    let initial = f.prepare(&context, &target).await;
    let mut once = grant.clone();
    let p = once.execution.as_mut().unwrap().v1.as_mut().unwrap();
    p.validity = Validity::Operation {
        operation_id: initial.operation_id.clone(),
        prepared_digest: initial.digest.clone(),
        expires_at: Utc::now() + chrono::Duration::hours(1),
    };
    once.expires_at = contract::legacy_expiry(&p.validity);
    f.state
        .db
        .collection::<ApprovalGrant>("approval_grants")
        .replace_one(doc! {"_id":&once.id}, &once)
        .await
        .unwrap();
    let mut permits = Vec::new();
    for _ in 0..20 {
        let context = Arc::new(RequestContext {
            actor: f.owner.clone(),
            principal: Principal::ApiKey(f.key.clone()),
            operation_id: initial.operation_id.clone(),
            voice_task: false,
            ingress: OnceLock::new(),
            idempotency: OnceLock::new(),
            async_requested: std::sync::atomic::AtomicBool::new(false),
            cancellation: OnceLock::new(),
            connected_resume: OnceLock::new(),
            connected_snapshot: OnceCell::new(),
            wait_guard: OnceLock::new(),
            native: None,
            descendant: None,
            authoring: std::sync::atomic::AtomicBool::new(false),
            generated_correlation: OnceLock::new(),
            resume: None,
            drain: OnceLock::new(),
            authenticated_key: OnceLock::new(),
            source_credential: OnceLock::new(),
            admission: OnceCell::new(),
            snapshot: OnceCell::new(),
            authority: OnceCell::new(),
            policies: tokio::sync::Mutex::new(Vec::new()),
            decision: OnceLock::new(),
            observation: Default::default(),
        });
        permits.push(f.prepare(&context, &target).await);
    }
    let client = &f.state.http_client;
    let url = format!("{}/status", f.server.uri());
    let outcomes = futures::future::join_all(permits.iter().map(|permit| async {
        if permit.before_send(&target).await.is_ok() {
            client.get(&url).send().await.unwrap();
            permit.complete(false).await.unwrap();
            true
        } else {
            false
        }
    }))
    .await;
    assert_eq!(outcomes.iter().filter(|v| **v).count(), 1);
    assert_eq!(f.server.received_requests().await.unwrap().len(), 1);
    assert!(matches!(
        permits[0].before_send(&target).await,
        Err(AppError::Conflict(_))
    ));
    assert_eq!(f.server.received_requests().await.unwrap().len(), 1);
    f.cleanup().await;
}

#[tokio::test]
async fn claimed_before_send_crash_never_resets_the_operation() {
    let f = Fixture::new("execution_claim_crash").await;
    f.enroll(true).await;
    let grant = f.grant().await;
    f.store(&grant).await;
    let target = f.target().await;
    let context = RequestContext::enrolled(Some(1), &f.owner, &f.key, None).unwrap();
    let initial = f.prepare(&context, &target).await;
    let mut once = grant;
    let p = once.execution.as_mut().unwrap().v1.as_mut().unwrap();
    p.validity = Validity::Operation {
        operation_id: initial.operation_id.clone(),
        prepared_digest: initial.digest.clone(),
        expires_at: Utc::now() + chrono::Duration::hours(1),
    };
    once.expires_at = contract::legacy_expiry(&p.validity);
    f.state
        .db
        .collection::<ApprovalGrant>("approval_grants")
        .replace_one(doc! {"_id":&once.id}, &once)
        .await
        .unwrap();
    let ctx = Arc::new(RequestContext {
        actor: f.owner.clone(),
        principal: Principal::ApiKey(f.key.clone()),
        operation_id: initial.operation_id.clone(),
        voice_task: false,
        ingress: OnceLock::new(),
        idempotency: OnceLock::new(),
        async_requested: std::sync::atomic::AtomicBool::new(false),
        cancellation: OnceLock::new(),
        connected_resume: OnceLock::new(),
        connected_snapshot: OnceCell::new(),
        wait_guard: OnceLock::new(),
        native: None,
        descendant: None,
        authoring: std::sync::atomic::AtomicBool::new(false),
        generated_correlation: OnceLock::new(),
        resume: None,
        drain: OnceLock::new(),
        authenticated_key: OnceLock::new(),
        source_credential: OnceLock::new(),
        admission: OnceCell::new(),
        snapshot: OnceCell::new(),
        authority: OnceCell::new(),
        policies: tokio::sync::Mutex::new(Vec::new()),
        decision: OnceLock::new(),
        observation: Default::default(),
    });
    let permit = f.prepare(&ctx, &target).await;
    permit.reserve().await.unwrap();
    assert!(permit.before_send(&target).await.is_err());
    assert!(f.server.received_requests().await.unwrap().is_empty());
    f.cleanup().await;
}

#[tokio::test]
async fn mcp_endpoint_and_generic_preparation_share_the_decision_and_send_once() {
    use tracing::instrument::WithSubscriber;
    let captured = migration_tests::Capture::default();
    let f = Fixture::new("execution_mcp_covered").await;
    f.enroll(true).await;
    let endpoint = test_utils::test_service_endpoint(&f.catalog, "status", "GET", "/status");
    f.state
        .db
        .collection("service_endpoints")
        .insert_one(endpoint)
        .await
        .unwrap();
    f.store(&f.grant().await).await;
    for generic in [false, true] {
        let tool = "execution-fixture__status";
        let params = if generic {
            json!({"name":"nyx__call_tool","arguments":{"tool_name":tool,"arguments_json":"{}"}})
        } else {
            json!({"name":tool,"arguments":{}})
        };
        let response = f
            .router()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/mcp")
                    .header("x-api-key", &f.raw)
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":params})
                            .to_string(),
                    ))
                    .unwrap(),
            )
            .with_subscriber(captured.subscriber())
            .await
            .unwrap();
        let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert!(body.get("error").is_none(), "{body}");
        assert_eq!(body["result"]["isError"], false, "{body}");
    }
    assert_eq!(f.server.received_requests().await.unwrap().len(), 2);
    assert_eq!(
        captured.0.lock().unwrap().len(),
        2,
        "one decision per HTTP/MCP occurrence"
    );
    assert!(
        captured
            .0
            .lock()
            .unwrap()
            .iter()
            .all(|event| event["cause"] == "permission_covered")
    );
    f.no_prompts().await;
    f.cleanup().await;
}

#[tokio::test]
async fn connection_binding_survives_dispatch_locks_but_fences_destination_changes() {
    let f = Fixture::new("execution_binding_lock").await;
    let credential = Uuid::new_v4().to_string();
    f.state
        .db
        .collection::<bson::Document>("user_api_keys")
        .insert_one(doc! {
            "_id": &credential, "user_id": &f.owner, "label": "Fixture",
            "credential_type": "api_key", "status": "active", "credential_epoch": 1_i64,
            "created_at": bson::DateTime::now(), "updated_at": bson::DateTime::now(),
        })
        .await
        .unwrap();
    f.state
        .db
        .collection("agent_service_bindings")
        .insert_one(crate::models::agent_service_binding::AgentServiceBinding {
            id: Uuid::new_v4().to_string(),
            api_key_id: f.key.clone(),
            user_service_id: f.connection.clone(),
            user_api_key_id: credential,
            user_id: f.owner.clone(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        })
        .await
        .unwrap();
    let before = super::super::assistant_service_execution::connection_binding(
        &f.state.db,
        &f.connection,
        &f.key,
    )
    .await
    .unwrap();
    let snapshot = authority::Snapshot::load(&f.state.db, &f.connection, &f.key, None)
        .await
        .unwrap();
    let mut session = f.state.db.client().start_session().await.unwrap();
    session.start_transaction().await.unwrap();
    snapshot.fence(&f.state.db, &mut session).await.unwrap();
    session.commit_transaction().await.unwrap();
    assert_eq!(
        super::super::assistant_service_execution::connection_binding(
            &f.state.db,
            &f.connection,
            &f.key,
        )
        .await
        .unwrap(),
        before,
        "transaction locks must not invalidate an admitted watch or connection consent"
    );
    f.state
        .db
        .collection::<bson::Document>("downstream_services")
        .update_one(
            doc! {"_id": &f.catalog},
            doc! {"$set": {"base_url": "https://changed.invalid"}},
        )
        .await
        .unwrap();
    assert_ne!(
        super::super::assistant_service_execution::connection_binding(
            &f.state.db,
            &f.connection,
            &f.key,
        )
        .await
        .unwrap(),
        before,
        "the destination remains part of the live authority binding"
    );
    f.cleanup().await;
}

#[tokio::test]
async fn unmarked_coordinator_adds_zero_database_commands() {
    use mongodb::event::{EventHandler, command::CommandEvent};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tracing::instrument::WithSubscriber;
    use tracing_subscriber::prelude::*;
    let mut f = Fixture::new("execution_zero_reads").await;
    let key = f
        .state
        .db
        .collection::<ApiKey>("api_keys")
        .find_one(doc! {"_id":&f.key})
        .await
        .unwrap()
        .unwrap();
    let mut target = f.target().await;
    let count = Arc::new(AtomicUsize::new(0));
    let captured = count.clone();
    let database = f.state.db.name().to_owned();
    let handler = EventHandler::callback(move |event| {
        if let CommandEvent::Started(event) = event
            && event.db == database
        {
            captured.fetch_add(1, Ordering::SeqCst);
        }
    });
    let mut options =
        mongodb::options::ClientOptions::parse(std::env::var("NYXID_TEST_DATABASE_URL").unwrap())
            .await
            .unwrap();
    options.command_event_handler = Some(handler);
    f.state.db = mongodb::Client::with_options(options)
        .unwrap()
        .database(f.state.db.name());
    let auth = crate::mw::auth::api_key_auth_user(&f.state.db, &key, None, None, None)
        .await
        .unwrap();
    assert!(auth.execution_permission.is_none());
    assert_eq!(
        count.swap(0, Ordering::SeqCst),
        1,
        "ordinary key authentication retains exactly its one existing person lookup across all collections"
    );
    let descriptor =
        super::super::operation_descriptor::build_http_descriptor("GET", "status", None);
    let captured = migration_tests::Capture::default();
    let subscriber = tracing::Dispatch::new(tracing_subscriber::registry().with(captured.clone()));
    admit_http(
        &f.state,
        &auth,
        Some(&f.connection),
        Some(&f.owner),
        &mut target,
        "GET",
        "status",
        None,
        b"",
        &Default::default(),
        None,
        &descriptor,
        false,
        None,
    )
    .with_subscriber(subscriber.clone())
    .await
    .unwrap();
    assert!(
        prepare(
            &f.state,
            None,
            None,
            &key.id,
            Effect {
                connection: &f.connection,
                owner: &f.owner,
                target: &target,
                method: "GET",
                path: "status",
                query: None,
                body: b"",
                content_type: None,
                headers: &[],
                endpoint: None,
                floors: &[],
            }
        )
        .with_subscriber(subscriber)
        .await
        .unwrap()
        .is_none()
    );
    assert!(
        captured.0.lock().unwrap().is_empty(),
        "unmarked diagnostics create neither events nor consent reads"
    );
    assert_eq!(
        count.load(Ordering::SeqCst),
        0,
        "count every DB command, including unrelated collections"
    );
    let mut native_key = key.clone();
    native_key.purpose = crate::models::api_key::ApiKeyPurpose::PermissionBound;
    let native = crate::mw::auth::api_key_auth_user(&f.state.db, &native_key, None, None, None)
        .await
        .unwrap();
    assert!(native.execution_permission.is_none());
    assert_eq!(
        count.swap(0, Ordering::SeqCst),
        1,
        "native key auth retains only its existing person read"
    );
    admit_http(
        &f.state,
        &native,
        Some(&f.connection),
        Some(&f.owner),
        &mut target,
        "GET",
        "status",
        None,
        b"",
        &Default::default(),
        None,
        &descriptor,
        false,
        None,
    )
    .await
    .unwrap();
    assert_eq!(
        count.load(Ordering::SeqCst),
        0,
        "unenrolled native dispatch adds no DB commands"
    );
    f.cleanup().await;
}

#[tokio::test]
async fn unenrolled_gap_keeps_legacy_and_enrolled_gap_creates_one_v2_review() {
    let f = Fixture::new("execution_gap_legacy").await;
    let channel = super::super::notification_service::get_or_create_channel(&f.state.db, &f.owner)
        .await
        .unwrap();
    f.state
        .db
        .collection::<Document>("notification_channels")
        .update_one(
            doc! {"_id":channel.id},
            doc! {"$set":{"approval_timeout_secs":1_i32}},
        )
        .await
        .unwrap();
    let legacy = f.call("/proxy/s/execution-fixture/status").await;
    assert_eq!(
        f.state
            .db
            .collection::<Document>("approval_requests")
            .count_documents(doc! {"execution_proposal":{"$ne":null}})
            .await
            .unwrap(),
        0
    );
    let legacy_requests = f
        .state
        .db
        .collection::<Document>("approval_requests")
        .count_documents(doc! {})
        .await
        .unwrap();
    f.state
        .db
        .collection::<Document>("approval_requests")
        .delete_many(doc! {})
        .await
        .unwrap();
    f.enroll(true).await;
    let participating = f.call("/proxy/s/execution-fixture/status").await;
    assert_eq!(
        f.state
            .db
            .collection::<Document>("approval_requests")
            .count_documents(doc! {"execution_proposal.version":2})
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        participating.0, legacy.0,
        "legacy={}, participating={}",
        legacy.1, participating.1
    );
    assert_ne!(legacy.0, StatusCode::OK);
    assert_eq!(
        f.state
            .db
            .collection::<Document>("approval_requests")
            .count_documents(doc! {})
            .await
            .unwrap(),
        legacy_requests
    );
    assert!(f.server.received_requests().await.unwrap().is_empty());
    f.cleanup().await;
}

#[tokio::test]
async fn task_roots_fence_actor_epoch_deadline_and_terminal_state() {
    let f = Fixture::new("execution_task_roots").await;
    for root_kind in [
        TaskRootKind::Message,
        TaskRootKind::Invocation,
        TaskRootKind::GroupRequest,
    ] {
        let task = TaskAdmission {
            version: VERSION,
            task: TaskRef {
                root_kind,
                root_id: Uuid::new_v4().to_string(),
            },
            actor_user_id: f.owner.clone(),
            audience: Audience::Private {
                user_id: f.owner.clone(),
            },
            epoch: 1,
            hard_deadline: Utc::now() + chrono::Duration::hours(1),
            cancelled: false,
            parent_turn_id: None,
        };
        let roots = f
            .state
            .db
            .collection::<Document>(tasks::collection(&task.task));
        roots.insert_one(doc! {"_id":&task.task.root_id,"execution_task":bson::to_bson(&task).unwrap(),"cancel_requested":false,"conversation_deleted":false}).await.unwrap();
        tasks::check(&f.state.db, &task).await.unwrap();
        let mut other = task.clone();
        other.actor_user_id = "someone-else".into();
        assert!(tasks::check(&f.state.db, &other).await.is_err());
        other = task.clone();
        other.epoch += 1;
        assert!(tasks::check(&f.state.db, &other).await.is_err());
        other = task.clone();
        other.task.root_id = Uuid::new_v4().to_string();
        assert!(tasks::check(&f.state.db, &other).await.is_err());
        other = task.clone();
        other.hard_deadline = Utc::now() - chrono::Duration::seconds(1);
        assert!(tasks::check(&f.state.db, &other).await.is_err());
        roots
            .update_one(
                doc! {"_id":&task.task.root_id},
                doc! {"$set":{"execution_task_completed":true}},
            )
            .await
            .unwrap();
        assert!(tasks::check(&f.state.db, &task).await.is_err());
        roots.update_one(doc! {"_id":&task.task.root_id}, doc! {"$unset":{"execution_task_completed":""},"$set":{"execution_task.hard_deadline":bson::DateTime::from_chrono(other.hard_deadline)}}).await.unwrap();
        assert!(tasks::check(&f.state.db, &task).await.is_err());
        roots.update_one(doc! {"_id":&task.task.root_id}, doc! {"$set":{"execution_task.hard_deadline":bson::DateTime::from_chrono(task.hard_deadline)}}).await.unwrap();
        roots.update_one(doc! {"_id":&task.task.root_id},doc! {"$set":{"execution_task.cancelled":true},"$inc":{"execution_task.epoch":1_i64}}).await.unwrap();
        assert!(tasks::check(&f.state.db, &task).await.is_err());
    }
    f.cleanup().await;
}

pub(crate) async fn seed_for_chat(
    state: &crate::AppState,
    auth: &crate::mw::auth::AuthUser,
    connection: &str,
    catalog: &str,
    method: &str,
    path: &str,
) -> ApprovalGrant {
    let chat = auth.assistant_chat.as_deref().expect("chat");
    let context = auth.execution_permission.as_ref().expect("enrolled");
    let mut grant = contract::tests::fixture();
    grant.user_id = chat.user_id.clone();
    grant.requester_id = chat.api_key_id.clone();
    grant.service_id = catalog.into();
    let permission = grant.execution.as_mut().unwrap().v1.as_mut().unwrap();
    permission.subject = Subject {
        actor_user_id: chat.user_id.clone(),
        principal: Principal::Agent(chat.agent_id.clone()),
        lane: if chat
            .org_agent_access
            .as_ref()
            .is_some_and(|a| a.shared().is_some())
        {
            ExecutionLane::Shared
        } else {
            ExecutionLane::Owner
        },
    };
    permission.binding = binding(
        &state.db,
        &state.encryption_keys,
        context,
        Some(chat),
        &chat.api_key_id,
        connection,
        &chat.user_id,
    )
    .await
    .unwrap();
    permission.context = ExecutionContext {
        audience: audience(&state.db, Some(chat), &chat.user_id, None)
            .await
            .unwrap(),
        conversation: ConversationScope::Exact {
            conversation_id: chat.conversation_id.clone(),
        },
    };
    permission.operations = AgentOperationScope {
        revision: 1,
        catalog_service_id: Some(catalog.into()),
        operations: vec![ScopedOperation {
            endpoint_id: None,
            rule: ProxyOperationRule {
                method: method.into(),
                path_template: path.into(),
                ..Default::default()
            },
            ..Default::default()
        }],
    };
    if permission.subject.lane == ExecutionLane::Shared {
        permission
            .satisfies
            .push(ExecutionFloor::SharedConnectionConsent);
    }
    state
        .db
        .collection::<ApprovalGrant>("approval_grants")
        .insert_one(&grant)
        .await
        .unwrap();
    grant
}

#[tokio::test]
async fn owner_ask_turn_covered_suppresses_risk_card_and_preserves_live_turn_fence() {
    let f =
        super::super::assistant_authority_tests::orchestrator_fixture("execution_owner_ask").await;
    let server = MockServer::start().await;
    Mock::given(any_request())
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;
    let connection = super::super::assistant_authority_tests::connected(
        &f.state.db,
        &f.owner,
        "execution-owner",
        &server.uri(),
    )
    .await;
    super::super::assistant_settings_service::update(
        &f.state.db,
        &f.owner,
        super::super::assistant_settings_service::Update {
            access_mode: Some(crate::models::assistant_settings::AccessMode::AskEveryTime),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    super::phase_b_tests::enroll(&f.state.db, &f.owner).await;
    let key = f
        .state
        .db
        .collection::<ApiKey>("api_keys")
        .find_one(doc! {"_id":&f.chat.api_key_id})
        .await
        .unwrap()
        .unwrap();
    let auth = crate::mw::auth::api_key_auth_user(&f.state.db, &key, None, None, None)
        .await
        .unwrap();
    seed_for_chat(&f.state, &auth, &connection, &connection, "POST", "/status").await;
    let credential = super::super::assistant_agent_credential_service::load_for_conversation(
        &f.state.db,
        &f.state.encryption_keys,
        &f.owner,
        &f.row.id,
    )
    .await
    .unwrap()
    .unwrap();
    use super::super::billing::{BillingIngress, route_inventory::BillingRoutePolicy};
    let app = Router::new()
        .route(
            "/proxy/s/{slug}/{*path}",
            any(crate::handlers::proxy::proxy_request_by_slug),
        )
        .layer(Extension(BillingRoutePolicy::Metered(
            BillingIngress::Proxy,
        )))
        .layer(axum::middleware::from_fn(
            crate::mw::telemetry::telemetry_mw,
        ))
        .with_state(f.state.clone());
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/proxy/s/execution-owner/status")
                .header("x-api-key", credential.raw_key.as_str())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), 1024 * 1024)
        .await
        .unwrap();
    assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
    assert_eq!(
        f.state
            .db
            .collection::<Document>("assistant_acknowledgements")
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn both_llm_gateways_cover_personal_approval_and_keep_one_provider_attempt() {
    let f = Fixture::new("execution_llm_gateways").await;
    f.enroll(true).await;
    let provider = Uuid::new_v4().to_string();
    let row:crate::models::provider_config::ProviderConfig=bson::from_document(doc! {"_id":&provider,"slug":"openai","name":"Fixture","provider_type":"api_key","is_active":true,"created_by":&f.owner,"created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()}).unwrap();
    f.state
        .db
        .collection("provider_configs")
        .insert_one(row)
        .await
        .unwrap();
    f.state
        .db
        .collection::<Document>("downstream_services")
        .update_one(
            doc! {"_id":&f.catalog},
            doc! {"$set":{"slug":"llm-openai","provider_config_id":&provider,
            "inference":{"wire_protocol":"openai_completions","model_list":true,"realtime":false}}},
        )
        .await
        .unwrap();
    let mut grant = f.grant().await;
    grant
        .execution
        .as_mut()
        .unwrap()
        .v1
        .as_mut()
        .unwrap()
        .operations
        .operations = ["/chat/completions", "/v1/chat/completions"]
        .into_iter()
        .map(|path| ScopedOperation {
            endpoint_id: None,
            rule: ProxyOperationRule {
                method: "POST".into(),
                path_template: path.into(),
                ..Default::default()
            },
            ..Default::default()
        })
        .collect();
    f.store(&grant).await;
    use super::super::billing::{BillingIngress, route_inventory::BillingRoutePolicy};
    let app = Router::new()
        .route(
            "/llm/gateway/v1/{*path}",
            any(crate::handlers::llm_gateway::gateway_request).layer(Extension(
                BillingRoutePolicy::Metered(BillingIngress::LlmGateway),
            )),
        )
        .route(
            "/llm/{provider_slug}/v1/{*path}",
            any(crate::handlers::llm_gateway::llm_proxy_request).layer(Extension(
                BillingRoutePolicy::Metered(BillingIngress::LlmProvider),
            )),
        )
        .with_state(f.state.clone());
    for route in [
        "/llm/openai/v1/chat/completions",
        "/llm/gateway/v1/chat/completions",
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(route)
                    .header("x-api-key", &f.raw)
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({"model":"gpt-4o","messages":[{"role":"user","content":"hello"}]})
                            .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        assert_eq!(
            status,
            StatusCode::OK,
            "{}",
            String::from_utf8_lossy(&bytes)
        );
    }
    assert_eq!(f.server.received_requests().await.unwrap().len(), 2);
    f.no_prompts().await;
    f.state
        .db
        .collection::<Document>("approval_grants")
        .update_one(doc! {"_id":&grant.id}, doc! {"$set":{"revoked":true}})
        .await
        .unwrap();
    for route in [
        "/llm/openai/v1/chat/completions",
        "/llm/gateway/v1/chat/completions",
    ] {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(route)
                    .header("x-api-key", &f.raw)
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({"model":"gpt-4o","messages":[{"role":"user","content":"hello"}]})
                            .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }
    f.no_prompts().await;
    assert_eq!(
        f.server.received_requests().await.unwrap().len(),
        2,
        "revocation adds no provider attempt"
    );
    f.state
        .db
        .collection::<Document>("approval_grants")
        .delete_many(doc! {})
        .await
        .unwrap();
    let channel = super::super::notification_service::get_or_create_channel(&f.state.db, &f.owner)
        .await
        .unwrap();
    f.state
        .db
        .collection::<Document>("notification_channels")
        .update_one(
            doc! {"_id":channel.id},
            doc! {"$set":{"approval_timeout_secs":1_i32}},
        )
        .await
        .unwrap();
    for route in [
        "/llm/openai/v1/chat/completions",
        "/llm/gateway/v1/chat/completions",
    ] {
        let mut observations = Vec::new();
        for enrolled in [false, true] {
            f.enroll(enrolled).await;
            f.state
                .db
                .collection::<Document>("approval_requests")
                .delete_many(doc! {})
                .await
                .unwrap();
            let response = app.clone().oneshot(Request::builder().method("POST").uri(route)
                .header("x-api-key", &f.raw).header("content-type","application/json")
                .body(Body::from(json!({"model":"gpt-4o","messages":[{"role":"user","content":"hello"}]}).to_string())).unwrap())
                .await.unwrap();
            observations.push((
                response.status(),
                f.state
                    .db
                    .collection::<Document>("approval_requests")
                    .count_documents(doc! {})
                    .await
                    .unwrap(),
            ));
        }
        assert_eq!(
            observations[0], observations[1],
            "{route}: either enrollment state returns one pending review and no effect"
        );
        assert_ne!(observations[0].0, StatusCode::OK);
        assert_eq!(observations[0].1, 1);
    }
    assert_eq!(f.server.received_requests().await.unwrap().len(), 2);
    f.cleanup().await;
}

#[tokio::test]
async fn pool_coverage_is_for_the_selected_connection_only() {
    use crate::models::service_pool::{PoolStrategy, ServicePoolMember};
    let f = Fixture::new("execution_pool").await;
    f.enroll(true).await;
    let pool = super::super::service_pool_service::create_pool(
        &f.state.db,
        &f.owner,
        super::super::service_pool_service::CreatePoolInput {
            slug: "execution-pool".into(),
            name: "Execution pool".into(),
            description: None,
            strategy: PoolStrategy::RoundRobin,
            tier_balance: Default::default(),
            member_contract: Default::default(),
            failover: None,
            members: vec![ServicePoolMember {
                user_service_id: f.connection.clone(),
                weight: 1,
                enabled: true,
                priority: 0,
                model: None,
                same_api_compatible: false,
                health_reset_generation: 0,
            }],
            is_active: Some(true),
        },
    )
    .await
    .unwrap();
    f.store(&f.grant().await).await;
    let (status, body) = f.call(&format!("/proxy/s/{}/status", pool.slug)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(f.server.received_requests().await.unwrap().len(), 1);
    f.no_prompts().await;
    f.cleanup().await;
}

#[tokio::test]
async fn covered_requests_never_follow_redirects_or_drift_prepared_inputs() {
    let f = Fixture::new("execution_redirect").await;
    f.enroll(true).await;
    let elsewhere = MockServer::start().await;
    Mock::given(any_request())
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&elsewhere)
        .await;
    f.server.reset().await;
    Mock::given(any_request())
        .respond_with(
            ResponseTemplate::new(307)
                .insert_header("location", format!("{}/other", elsewhere.uri())),
        )
        .expect(1)
        .mount(&f.server)
        .await;
    f.store(&f.grant().await).await;
    let (status, body) = f.call("/proxy/s/execution-fixture/status").await;
    assert_eq!(status, StatusCode::TEMPORARY_REDIRECT, "{body}");
    let target = f.target().await;
    let context = RequestContext::enrolled(Some(1), &f.owner, &f.key, None).unwrap();
    let permit = f.prepare(&context, &target).await;
    for (method, path, body) in [
        ("POST", "status", b"".as_slice()),
        ("GET", "other", b"".as_slice()),
        ("GET", "status", b"changed".as_slice()),
    ] {
        assert!(
            permit
                .validate_effect(&target, method, path, None, body, &Default::default())
                .is_err()
        );
    }
    assert_eq!(f.server.received_requests().await.unwrap().len(), 1);
    f.no_prompts().await;
    f.cleanup().await;
}

#[tokio::test]
async fn concurrent_key_authentication_preserves_coverage_but_live_scope_change_does_not() {
    let f = Fixture::new("execution_key_usage").await;
    f.enroll(true).await;
    f.store(&f.grant().await).await;
    let auth = f.auth().await;
    let target = f.target().await;
    let permit = f
        .prepare(auth.execution_permission.as_ref().unwrap(), &target)
        .await;
    super::super::key_service::validate_api_key(&f.state.db, &f.raw)
        .await
        .unwrap();
    permit.before_send(&target).await.unwrap();
    super::super::api_key_mutation_service::update_one(
        &f.state.db,
        doc! {"_id":&f.key},
        doc! {"$set":{"allow_all_services":false,"allowed_service_ids":[]}},
        None,
    )
    .await
    .unwrap();
    assert!(permit.before_send(&target).await.is_err());
    assert!(f.server.received_requests().await.unwrap().is_empty());
    f.no_prompts().await;
    f.cleanup().await;
}

#[tokio::test]
async fn login_credential_is_live_at_coverage_and_dispatch_without_borrowing_parent_lifetime() {
    use crate::models::api_key_credential::{ApiKeyCredential, CredentialRevokedReason};
    for expire in [false, true] {
        let mut f = Fixture::new("execution_login_credential").await;
        f.enroll(true).await;
        f.store(&f.grant().await).await;
        let child_id = Uuid::new_v4().to_string();
        let raw = super::super::api_key_credential_service::generate_secret();
        let child: ApiKeyCredential = bson::from_document(doc! {
            "_id":&child_id,"api_key_id":&f.key,"user_id":&f.owner,
            "secret_hash":crate::crypto::token::hash_token(&raw),"secret_prefix":"test",
            "label":"Test login","login_request_id":Uuid::new_v4().to_string(),
            "is_active":true,"created_at":bson::DateTime::now(),
            "expires_at":bson::DateTime::from_chrono(Utc::now()+chrono::Duration::hours(1))
        })
        .unwrap();
        f.state
            .db
            .collection::<ApiKeyCredential>("api_key_credentials")
            .insert_one(child)
            .await
            .unwrap();
        f.raw = raw.to_string();
        let (status, body) = f.call("/proxy/s/execution-fixture/status").await;
        assert_eq!(status, StatusCode::OK, "{body}");
        f.state
            .db
            .collection::<crate::models::service_endpoint::ServiceEndpoint>("service_endpoints")
            .insert_one(test_utils::test_service_endpoint(
                &f.catalog, "status", "GET", "/status",
            ))
            .await
            .unwrap();
        let response = f
            .router()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/mcp")
                    .header("x-api-key", &f.raw)
                    .header("content-type", "application/json")
                    .body(Body::from(
                        json!({"jsonrpc":"2.0","id":1,"method":"tools/call",
                "params":{"name":"execution-fixture__status","arguments":{}}})
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let response: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(response["result"]["isError"], false, "{response}");
        let (_, key, credential) = super::super::key_service::validate_api_key(&f.state.db, &f.raw)
            .await
            .unwrap();
        let auth = crate::mw::auth::api_key_auth_user(&f.state.db, &key, credential, None, None)
            .await
            .unwrap();
        let target = f.target().await;
        let permit = f
            .prepare(auth.execution_permission.as_ref().unwrap(), &target)
            .await;
        // Another successful authentication updates child usage only.
        super::super::key_service::validate_api_key(&f.state.db, &f.raw)
            .await
            .unwrap();
        permit.before_send(&target).await.unwrap();
        if expire {
            f.state.db.collection::<Document>("api_key_credentials").update_one(
                doc! {"_id":&child_id}, doc! {"$set":{"expires_at":bson::DateTime::from_chrono(Utc::now()-chrono::Duration::seconds(1))}}
            ).await.unwrap();
        } else {
            super::super::api_key_credential_service::revoke(
                &f.state.db,
                &child_id,
                CredentialRevokedReason::WebRevoke,
            )
            .await
            .unwrap();
        }
        assert!(permit.before_send(&target).await.is_err());
        assert_eq!(
            f.call("/proxy/s/execution-fixture/status").await.0,
            StatusCode::UNAUTHORIZED
        );
        assert_eq!(f.server.received_requests().await.unwrap().len(), 2);
        assert!(
            f.state
                .db
                .collection::<ApiKey>("api_keys")
                .find_one(doc! {"_id":&f.key})
                .await
                .unwrap()
                .unwrap()
                .is_active
        );
        f.no_prompts().await;
        f.cleanup().await;
    }
}

#[tokio::test]
async fn narrowing_key_after_authentication_cannot_establish_coverage() {
    let f = Fixture::new("execution_key_race").await;
    f.enroll(true).await;
    f.store(&f.grant().await).await;
    let auth = f.auth().await;
    f.state.db.collection::<Document>("api_keys").update_one(doc! {"_id":&f.key},doc! {"$set":{"allow_all_services":false,"allow_auto_connected_services":false,"allowed_service_ids":[]}}).await.unwrap();
    let mut target = f.target().await;
    let descriptor =
        super::super::operation_descriptor::build_http_descriptor("GET", "status", None);
    assert!(
        admit_http(
            &f.state,
            &auth,
            Some(&f.connection),
            Some(&f.owner),
            &mut target,
            "GET",
            "status",
            None,
            b"",
            &Default::default(),
            None,
            &descriptor,
            false,
            None
        )
        .await
        .is_err()
    );
    assert!(f.server.received_requests().await.unwrap().is_empty());
    f.no_prompts().await;
    f.cleanup().await;
}

#[tokio::test]
async fn enrollment_disable_fences_even_an_admitted_legacy_gap() {
    let f = Fixture::new("execution_gap_drain").await;
    f.enroll(true).await;
    let auth = f.auth().await;
    let mut target = f.target().await;
    let descriptor =
        super::super::operation_descriptor::build_http_descriptor("GET", "status", None);
    // Raw input cannot be retained, so this remains a real legacy gap even
    // though enrolled, replayable external requests now create v2 proposals.
    let mut headers = axum::http::HeaderMap::new();
    headers.insert(
        axum::http::header::CONTENT_TYPE,
        "application/octet-stream".parse().unwrap(),
    );
    admit_http(
        &f.state,
        &auth,
        Some(&f.connection),
        Some(&f.owner),
        &mut target,
        "GET",
        "status",
        None,
        b"raw payload",
        &headers,
        None,
        &descriptor,
        false,
        None,
    )
    .await
    .unwrap();
    assert!(target.execution_permission.is_none());
    let context = target.execution_request.as_ref().unwrap();
    context.check_drain().await.unwrap();
    f.enroll(false).await;
    assert!(context.check_drain().await.is_err());
    assert!(f.server.received_requests().await.unwrap().is_empty());
    f.no_prompts().await;
    f.cleanup().await;
}

#[tokio::test]
async fn permission_revocation_before_dispatch_has_zero_attempts() {
    let f = Fixture::new("execution_revoke_race").await;
    f.enroll(true).await;
    let grant = f.grant().await;
    f.store(&grant).await;
    let target = f.target().await;
    let context = RequestContext::enrolled(Some(1), &f.owner, &f.key, None).unwrap();
    let permit = f.prepare(&context, &target).await;
    super::super::approval_service::revoke_grant(&f.state.db, &f.owner, &grant.id)
        .await
        .unwrap();
    assert!(permit.before_send(&target).await.is_err());
    assert!(f.server.received_requests().await.unwrap().is_empty());
    f.no_prompts().await;
    f.cleanup().await;
}

#[tokio::test]
async fn operation_candidates_use_independent_claims_and_prefer_reusable_coverage() {
    let f = Fixture::new("execution_candidate_claims").await;
    f.enroll(true).await;
    let reusable = f.grant().await;
    f.store(&reusable).await;
    let target = f.target().await;
    let ctx = RequestContext::enrolled(Some(1), &f.owner, &f.key, None).unwrap();
    let original = f.prepare(&ctx, &target).await;
    let mut once = reusable.clone();
    once.id = Uuid::new_v4().to_string();
    let p = once.execution.as_mut().unwrap().v1.as_mut().unwrap();
    p.validity = Validity::Operation {
        operation_id: original.operation_id.clone(),
        prepared_digest: original.digest.clone(),
        expires_at: Utc::now() + chrono::Duration::hours(1),
    };
    once.expires_at = contract::legacy_expiry(&p.validity);
    once.execution_use = Some(ExecutionUse {
        version: VERSION,
        operation_id: original.operation_id.clone(),
        prepared_digest: original.digest.clone(),
        claim_id: Uuid::new_v4().to_string(),
        state: ExecutionUseState::Unknown,
        claimed_at: Utc::now(),
        dispatched_at: Some(Utc::now()),
        completed_at: None,
    });
    f.store(&once).await;
    let ctx = Arc::new(RequestContext {
        actor: f.owner.clone(),
        principal: Principal::ApiKey(f.key.clone()),
        operation_id: original.operation_id.clone(),
        voice_task: false,
        ingress: OnceLock::new(),
        idempotency: OnceLock::new(),
        async_requested: std::sync::atomic::AtomicBool::new(false),
        cancellation: OnceLock::new(),
        connected_resume: OnceLock::new(),
        connected_snapshot: OnceCell::new(),
        wait_guard: OnceLock::new(),
        native: None,
        descendant: None,
        authoring: std::sync::atomic::AtomicBool::new(false),
        generated_correlation: OnceLock::new(),
        resume: None,
        drain: OnceLock::new(),
        authenticated_key: OnceLock::new(),
        source_credential: OnceLock::new(),
        admission: OnceCell::new(),
        snapshot: OnceCell::new(),
        authority: OnceCell::new(),
        policies: tokio::sync::Mutex::new(Vec::new()),
        decision: OnceLock::new(),
        observation: Default::default(),
    });
    let permit = f.prepare(&ctx, &target).await;
    assert_eq!(permit.grant.id, reusable.id);
    permit.before_send(&target).await.unwrap();
    f.state
        .http_client
        .get(format!("{}/status", f.server.uri()))
        .send()
        .await
        .unwrap();
    let saved = f
        .state
        .db
        .collection::<ApprovalGrant>("approval_grants")
        .find_one(doc! {"_id":&once.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        saved.execution_use.unwrap().state,
        ExecutionUseState::Unknown
    );
    assert_eq!(f.server.received_requests().await.unwrap().len(), 1);
    f.cleanup().await;
}

#[tokio::test]
async fn covered_risk_keeps_webhook_policy_delegation_and_org_instruction_floors() {
    let f = super::super::assistant_authority_tests::orchestrator_fixture("execution_floors").await;
    let connection = super::super::assistant_authority_tests::connected(
        &f.state.db,
        &f.owner,
        "execution-floors",
        "https://example.com",
    )
    .await;
    super::phase_b_tests::enroll(&f.state.db, &f.owner).await;
    let key = f
        .state
        .db
        .collection::<ApiKey>("api_keys")
        .find_one(doc! {"_id":&f.chat.api_key_id})
        .await
        .unwrap()
        .unwrap();
    let auth = crate::mw::auth::api_key_auth_user(&f.state.db, &key, None, None, None)
        .await
        .unwrap();
    seed_for_chat(&f.state, &auth, &connection, &connection, "POST", "/effect").await;
    let target = proxy_service::read_proxy_authority_snapshot_with_access(
        &f.state.db,
        &f.state.encryption_keys,
        &f.owner,
        &connection,
        None,
        None,
    )
    .await
    .unwrap()
    .unwrap()
    .target;
    let chat = auth.assistant_chat.as_deref().unwrap();
    prepare(
        &f.state,
        auth.execution_permission.as_ref(),
        Some(chat),
        &key.id,
        Effect {
            connection: &connection,
            owner: &f.owner,
            target: &target,
            method: "POST",
            path: "effect",
            query: None,
            body: b"",
            content_type: None,
            headers: &[],
            endpoint: None,
            floors: &[ExecutionFloor::AccessModeRisk],
        },
    )
    .await
    .unwrap()
    .unwrap();
    for floor in ["webhook", "policy", "delegation", "org_instruction"] {
        let gate = acks::Gate::Ask(acks::ConfirmationNeed {
            class: acks::OwnerActionClass::Change,
            mode: Some(crate::models::assistant_settings::AccessMode::AskEveryTime),
            webhook: floor == "webhook",
            policy: floor == "policy",
            delegation: floor == "delegation",
            org_instruction: if floor == "org_instruction" {
                acks::OrgInstructionRequirement::Required
            } else {
                acks::OrgInstructionRequirement::None
            },
        });
        assert!(
            risk_gate(chat, &connection, gate).ask().is_some(),
            "{floor} must survive coverage"
        );
    }
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn message_root_admission_recovery_and_stop_fence_persistent_permission() {
    use super::super::assistant_nyxagent as engine;
    let f = super::super::assistant_authority_tests::orchestrator_fixture("execution_task_runtime")
        .await;
    super::phase_b_tests::enroll(&f.state.db, &f.owner).await;
    let mut start = engine::TurnStart::from(&engine::TurnRequest {
        conversation_id: None,
        text: "One logical task".into(),
        agent_id: None,
        model: None,
        access_mode: None,
        attachment_ids: vec![],
        acknowledgement_id: None,
        retry_of: None,
    });
    start.execution_enrollment = true;
    let row = engine::begin_turn(&f.state.db, &f.owner, &start, &f.state.encryption_keys)
        .await
        .unwrap();
    let task = row
        .active_turn
        .as_ref()
        .unwrap()
        .execution_task
        .clone()
        .expect("server root");
    assert_eq!(task.task.root_kind, TaskRootKind::Message);
    tasks::check(&f.state.db, &task).await.unwrap();
    engine::finish_turn(
        &f.state.db,
        &row,
        &row.credential_api_key_id,
        &Uuid::new_v4().to_string(),
        &engine::TurnResult {
            text: String::new(),
            session_id: None,
            response_id: None,
            error: Some(engine::TurnError::internal(
                "assistant_unavailable",
                "stream",
            )),
        },
    )
    .await
    .unwrap();
    let mut retry = start.clone();
    retry.conversation_id = Some(row.id.clone());
    retry.retry_of = Some(row.active_turn.as_ref().unwrap().turn_id.clone());
    retry.execution_enrollment = false;
    let recovered = engine::begin_turn(&f.state.db, &f.owner, &retry, &f.state.encryption_keys)
        .await
        .unwrap();
    let inherited = recovered
        .active_turn
        .as_ref()
        .unwrap()
        .execution_task
        .as_ref()
        .unwrap();
    assert_eq!(inherited.task, task.task);
    assert_eq!(inherited.hard_deadline, task.hard_deadline);
    let other = engine::begin_turn(&f.state.db, &f.owner, &start, &f.state.encryption_keys)
        .await
        .unwrap();
    assert_ne!(
        other
            .active_turn
            .as_ref()
            .unwrap()
            .execution_task
            .as_ref()
            .unwrap()
            .task,
        task.task
    );
    let recovered_chat = acks::for_key(
        &f.state.db,
        &f.owner,
        Some(&recovered.credential_api_key_id),
    )
    .await
    .unwrap()
    .unwrap();
    let need = acks::ConfirmationNeed {
        class: acks::OwnerActionClass::Change,
        mode: Some(crate::models::assistant_settings::AccessMode::AskEveryTime),
        webhook: false,
        policy: false,
        delegation: false,
        org_instruction: acks::OrgInstructionRequirement::None,
    };
    assert!(
        acks::confirm_action(
            &f.state.db,
            &recovered_chat,
            "test_task_action",
            &json!({"value":1}),
            &need,
            "Task action"
        )
        .await
        .unwrap()
        .is_some()
    );
    let cards = f
        .state
        .db
        .collection::<crate::models::assistant_acknowledgement::AssistantAcknowledgement>(
            "assistant_acknowledgements",
        );
    let card = cards
        .find_one(doc! {"conversation_id":&recovered.id,"status":"pending"})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(card.execution_task.as_ref().unwrap().task, task.task);
    tasks::stop(&f.state.db, &recovered).await.unwrap();
    let expired = cards
        .find_one(doc! {"_id":&card.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(expired.status, "expired");
    assert!(
        acks::decide(&f.state.db, &f.owner, &recovered.id, &card.id, true)
            .await
            .is_err()
    );
    assert!(
        acks::confirm_action(
            &f.state.db,
            &recovered_chat,
            "test_task_action",
            &json!({"value":2}),
            &need,
            "Task action"
        )
        .await
        .is_err()
    );
    assert!(tasks::check(&f.state.db, &task).await.is_err());
    assert!(
        engine::begin_turn(&f.state.db, &f.owner, &retry, &f.state.encryption_keys)
            .await
            .is_err()
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn disable_clears_an_inactive_person_marker_before_reactivation() {
    let f = Fixture::new("execution_inactive_disable").await;
    f.enroll(true).await;
    f.state
        .db
        .collection::<Document>("users")
        .update_one(doc! {"_id":&f.owner}, doc! {"$set":{"is_active":false}})
        .await
        .unwrap();
    f.enroll(false).await;
    let row = f
        .state
        .db
        .collection::<Document>("users")
        .find_one(doc! {"_id":&f.owner})
        .await
        .unwrap()
        .unwrap();
    assert!(!row.contains_key("execution_permission_enrollment"));
    f.state
        .db
        .collection::<Document>("users")
        .update_one(doc! {"_id":&f.owner}, doc! {"$set":{"is_active":true}})
        .await
        .unwrap();
    assert!(f.auth().await.execution_permission.is_none());
    f.cleanup().await;
}

#[tokio::test]
async fn oauth_refresh_same_epoch_retains_coverage_but_replacement_fences_it() {
    let f = Fixture::new("execution_refresh_epoch").await;
    f.enroll(true).await;
    let credential = super::super::user_api_key_service::build_api_key(
        &f.state.encryption_keys,
        &f.owner,
        super::super::user_api_key_service::CreateApiKeyParams {
            label: "OAuth fixture",
            credential_type: "oauth2",
            credential: "",
            access_token: Some("test-access-original"),
            refresh_token: Some("test-refresh-original"),
            token_scopes: None,
            expires_at: Some(Utc::now() + chrono::Duration::hours(1)),
            provider_config_id: None,
            connection_id: None,
            oauth_client_id: None,
            oauth_client_secret: None,
            status: "active",
            source: None,
            source_id: None,
        },
    )
    .await
    .unwrap();
    f.state
        .db
        .collection::<crate::models::user_api_key::UserApiKey>("user_api_keys")
        .insert_one(&credential)
        .await
        .unwrap();
    f.state
        .db
        .collection::<Document>("user_services")
        .update_one(
            doc! {"_id":&f.connection},
            doc! {"$set":{"api_key_id":&credential.id}},
        )
        .await
        .unwrap();
    let grant = f.grant().await;
    f.store(&grant).await;
    let target = f.target().await;
    let ctx = RequestContext::enrolled(Some(1), &f.owner, &f.key, None).unwrap();
    let permit = f.prepare(&ctx, &target).await;
    // Real refreshed ciphertext and expiry retain the same credential ID/epoch.
    let refreshed = super::super::credential_record_service::encrypt(
        &f.state.encryption_keys,
        "user_api_keys",
        &credential.id,
        &f.owner,
        "access_token_encrypted",
        b"test-access-refreshed",
    )
    .await
    .unwrap();
    assert_ne!(
        credential.access_token_encrypted.as_deref(),
        Some(refreshed.as_slice())
    );
    f.state.db.collection::<Document>("user_api_keys").update_one(doc! {"_id":&credential.id},doc! {"$set":{
        "access_token_encrypted": bson::Binary {subtype:bson::spec::BinarySubtype::Generic,bytes:refreshed},
        "expires_at":bson::DateTime::from_chrono(Utc::now()+chrono::Duration::hours(2)),"updated_at":bson::DateTime::now()}}).await.unwrap();
    permit.before_send(&target).await.unwrap();
    f.state
        .http_client
        .get(format!("{}/status", f.server.uri()))
        .send()
        .await
        .unwrap();
    f.state
        .db
        .collection::<Document>("user_api_keys")
        .update_one(
            doc! {"_id":&credential.id},
            vec![doc! {"$set":{"credential_epoch":{"$add":["$credential_epoch",1]}}}],
        )
        .await
        .unwrap();
    assert!(permit.before_send(&target).await.is_err());
    assert_eq!(f.server.received_requests().await.unwrap().len(), 1);
    f.no_prompts().await;
    f.cleanup().await;
}

#[tokio::test]
async fn candidate_loading_batches_fifty_rows_and_reuses_the_request_snapshot() {
    use mongodb::event::{EventHandler, command::CommandEvent};
    use std::sync::atomic::{AtomicUsize, Ordering};
    let mut f = Fixture::new("execution_candidate_batch").await;
    f.enroll(true).await;
    let grant = f.grant().await;
    f.store(&grant).await;
    let count = Arc::new(AtomicUsize::new(0));
    let captured = count.clone();
    let database = f.state.db.name().to_owned();
    let observed_database = database.clone();
    let mut options =
        mongodb::options::ClientOptions::parse(std::env::var("NYXID_TEST_DATABASE_URL").unwrap())
            .await
            .unwrap();
    options.command_event_handler = Some(EventHandler::callback(move |event| {
        if let CommandEvent::Started(event) = event
            && event.db == observed_database
        {
            captured.fetch_add(1, Ordering::SeqCst);
        }
    }));
    f.state.db = mongodb::Client::with_options(options)
        .unwrap()
        .database(&database);
    let one = RequestContext::enrolled(Some(1), &f.owner, &f.key, None).unwrap();
    assert_eq!(
        one.candidates(&f.state.db, &f.connection)
            .await
            .unwrap()
            .len(),
        1
    );
    let single_commands = count.swap(0, Ordering::SeqCst);
    assert!(single_commands > 0);
    for _ in 1..50 {
        let mut extra = grant.clone();
        extra.id = Uuid::new_v4().to_string();
        f.store(&extra).await;
    }
    count.store(0, Ordering::SeqCst);
    let many = RequestContext::enrolled(Some(1), &f.owner, &f.key, None).unwrap();
    assert_eq!(
        many.candidates(&f.state.db, &f.connection)
            .await
            .unwrap()
            .len(),
        50
    );
    assert_eq!(
        count.swap(0, Ordering::SeqCst),
        single_commands,
        "candidate count must not add commands"
    );
    assert_eq!(
        many.candidates(&f.state.db, &f.connection)
            .await
            .unwrap()
            .len(),
        50
    );
    assert_eq!(
        count.load(Ordering::SeqCst),
        0,
        "same request reuses its snapshot"
    );
    f.cleanup().await;
}

#[tokio::test]
async fn invocation_admission_has_a_server_root_and_cancel_fences_recovery() {
    use super::super::{assistant_invocation_service as invocations, assistant_nyxagent as engine};
    let f =
        super::super::assistant_authority_tests::orchestrator_fixture("execution_invocation_root")
            .await;
    super::phase_b_tests::enroll(&f.state.db, &f.owner).await;
    let mut start = engine::TurnStart::from(&engine::TurnRequest {
        conversation_id: None,
        text: "One invocation".into(),
        agent_id: None,
        model: None,
        access_mode: None,
        attachment_ids: vec![],
        acknowledgement_id: None,
        retry_of: None,
    });
    start.execution_enrollment = true;
    let admitted = invocations::admit(
        &f.state.db,
        &f.owner,
        "execution-test-root",
        "same-digest",
        &start,
        &f.state.encryption_keys,
    )
    .await
    .unwrap();
    let claim = admitted.claim.unwrap();
    let task = claim
        .conversation
        .active_turn
        .as_ref()
        .unwrap()
        .execution_task
        .as_ref()
        .unwrap();
    assert_eq!(task.task.root_kind, TaskRootKind::Invocation);
    assert_eq!(task.task.root_id, admitted.invocation.id);
    tasks::check(&f.state.db, task).await.unwrap();
    let replay = invocations::admit(
        &f.state.db,
        &f.owner,
        "execution-test-root",
        "same-digest",
        &start,
        &f.state.encryption_keys,
    )
    .await
    .unwrap();
    assert!(replay.claim.is_none());
    assert_eq!(
        replay.invocation.execution_task.as_ref().unwrap().task,
        task.task
    );
    invocations::cancel(&f.state, &f.owner, "execution-test-root")
        .await
        .unwrap();
    assert!(tasks::check(&f.state.db, task).await.is_err());
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn message_delegation_preserves_root_but_new_human_input_does_not_inherit_it() {
    use super::super::{assistant_nyxagent as engine, assistant_team_service as team};
    use crate::models::assistant_conversation::{AssistantConversation, TurnOrigin};
    let f =
        super::super::assistant_authority_tests::orchestrator_fixture("execution_delegation_root")
            .await;
    super::phase_b_tests::enroll(&f.state.db, &f.owner).await;
    let mut start = engine::TurnStart::from(&engine::TurnRequest {
        conversation_id: None,
        text: "A parent task".into(),
        agent_id: None,
        model: None,
        access_mode: None,
        attachment_ids: vec![],
        acknowledgement_id: None,
        retry_of: None,
    });
    start.execution_enrollment = true;
    let parent = engine::begin_turn(&f.state.db, &f.owner, &start, &f.state.encryption_keys)
        .await
        .unwrap();
    let parent_chat = acks::for_key(&f.state.db, &f.owner, Some(&parent.credential_api_key_id))
        .await
        .unwrap()
        .unwrap();
    let task = parent_chat.execution_task.as_ref().unwrap();
    let mut delegated = start.clone();
    delegated.origin = TurnOrigin::Orchestrator;
    delegated.execution_enrollment = false;
    delegated.execution_parent = tasks::parent(&parent_chat);
    let child = engine::begin_turn(&f.state.db, &f.owner, &delegated, &f.state.encryption_keys)
        .await
        .unwrap();
    assert_eq!(
        child
            .active_turn
            .as_ref()
            .unwrap()
            .execution_task
            .as_ref()
            .unwrap()
            .task,
        task.task
    );
    let mut completion = team::event("subagent_settled", "A prior task completed".into(), None);
    completion.execution_task = Some(task.clone());
    engine::push_events(&f.state.db, &f.owner, &f.row.id, vec![completion])
        .await
        .unwrap();
    // The fixture already has a turn; settle it before posting a fresh human message.
    engine::finish_turn(
        &f.state.db,
        &f.row,
        &f.row.credential_api_key_id,
        &Uuid::new_v4().to_string(),
        &engine::TurnResult {
            text: "Done".into(),
            session_id: None,
            response_id: None,
            error: None,
        },
    )
    .await
    .unwrap();
    let mut new_input = start.clone();
    new_input.conversation_id = Some(f.row.id.clone());
    let human = engine::begin_turn(&f.state.db, &f.owner, &new_input, &f.state.encryption_keys)
        .await
        .unwrap();
    assert_ne!(
        human
            .active_turn
            .as_ref()
            .unwrap()
            .execution_task
            .as_ref()
            .unwrap()
            .task,
        task.task
    );
    tasks::stop(&f.state.db, &parent).await.unwrap();
    let stopped = f
        .state
        .db
        .collection::<AssistantConversation>("assistant_conversations")
        .find_one(doc! {"_id":&child.id})
        .await
        .unwrap()
        .unwrap();
    assert!(
        stopped.active_turn.unwrap().stop_requested,
        "stopping the logical task stops its descendants"
    );
    assert!(
        engine::begin_turn(&f.state.db, &f.owner, &delegated, &f.state.encryption_keys)
            .await
            .is_err()
    );
    f.state.db.drop().await.unwrap();
}

#[test]
fn event_turns_partition_distinct_tasks_without_changing_legacy_events() {
    use crate::models::assistant_conversation::TurnOrigin;
    let mut pending = Vec::new();
    let task = TaskAdmission {
        version: VERSION,
        task: TaskRef {
            root_kind: TaskRootKind::Message,
            root_id: "first".into(),
        },
        actor_user_id: "actor".into(),
        audience: Audience::Private {
            user_id: "actor".into(),
        },
        epoch: 1,
        hard_deadline: Utc::now() + chrono::Duration::hours(1),
        cancelled: false,
        parent_turn_id: None,
    };
    let mut first =
        super::super::assistant_team_service::event("permission_decided", "first".into(), None);
    first.execution_task = Some(task.clone());
    let mut second = first.clone();
    second.id = Uuid::new_v4().to_string();
    second.execution_task.as_mut().unwrap().task.root_id = "second".into();
    let selected = tasks::partition_events(&mut pending, vec![first, second], TurnOrigin::Event);
    assert_eq!(selected.len(), 1);
    assert_eq!(pending.len(), 1);
    assert_ne!(
        selected[0].execution_task.as_ref().unwrap().task,
        pending[0].execution_task.as_ref().unwrap().task
    );
}

#[tokio::test]
async fn mcp_generic_http_and_encoded_prepared_endpoint_keep_exact_coverage() {
    for generic in [false, true] {
        let mut f = Fixture::new(if generic {
            "execution_mcp_generic"
        } else {
            "execution_mcp_encoded"
        })
        .await;
        f.enroll(true).await;
        let arguments = if generic {
            f.state
                .db
                .collection::<Document>("user_services")
                .update_one(
                    doc! {"_id":&f.connection},
                    doc! {"$set":{"catalog_service_id":null}},
                )
                .await
                .unwrap();
            f.state
                .db
                .collection::<Document>("service_approval_configs")
                .update_one(
                    doc! {"service_id":&f.catalog},
                    doc! {"$set":{"service_id":&f.connection}},
                )
                .await
                .unwrap();
            f.catalog = f.connection.clone();
            json!({"method":"GET","path":"/status"})
        } else {
            let mut endpoint =
                test_utils::test_service_endpoint(&f.catalog, "status", "GET", "/status/{name}");
            endpoint.parameters = Some(
                json!([{"name":"name","in":"path","required":true,"allowReserved":true,"schema":{"type":"string"}}]),
            );
            f.state
                .db
                .collection::<crate::models::service_endpoint::ServiceEndpoint>("service_endpoints")
                .insert_one(endpoint)
                .await
                .unwrap();
            json!({"name":"hello world"})
        };
        let mut grant = f.grant().await;
        if !generic {
            let rule = &mut grant
                .execution
                .as_mut()
                .unwrap()
                .v1
                .as_mut()
                .unwrap()
                .operations
                .operations[0]
                .rule;
            rule.path_template = "/status/{name}".into();
            let path = CanonicalPath::from_mcp_built("/status/hello%20world").unwrap();
            assert!(
                !super::super::proxy_authorization::rule_matches(rule, "GET", &path),
                "ordinary placeholders must retain their whitespace refusal"
            );
            // Mirror the endpoint's explicitly declared multi-segment grammar.
            // Encoding alone must never widen the operation permission.
            rule.path_parameter_constraints.insert(
                "name".into(),
                crate::models::downstream_service::ProxyPathConstraint::MultiSegment,
            );
        }
        f.store(&grant).await;
        let name = if generic {
            "execution-fixture__request"
        } else {
            "execution-fixture__status"
        };
        let response = tokio::time::timeout(std::time::Duration::from_secs(10),
            f.router().oneshot(Request::builder().method("POST").uri("/mcp").header("x-api-key",&f.raw).header("content-type","application/json")
                .body(Body::from(json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":name,"arguments":arguments}}).to_string())).unwrap()))
            .await.expect("a covered MCP operation must not enter an approval wait").unwrap();
        let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        let body: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(body["result"]["isError"], false, "{body}");
        let requests = f.server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 1);
        assert_eq!(
            requests[0].url.path(),
            if generic {
                "/status"
            } else {
                "/status/hello%20world"
            }
        );
        f.no_prompts().await;
        f.cleanup().await;
    }
}

#[tokio::test]
async fn policy_change_during_admission_cannot_become_the_coverage_baseline() {
    let f = Fixture::new("execution_policy_admission_race").await;
    f.enroll(true).await;
    f.store(&f.grant().await).await;
    let target = f.target().await;
    let context = RequestContext::enrolled(Some(1), &f.owner, &f.key, None).unwrap();
    context
        .authority(&f.state.db, &f.connection, &f.key)
        .await
        .unwrap();
    // A policy writer commits after the coordinator's first policy snapshot.
    f.state
        .db
        .collection::<Document>("service_approval_configs")
        .update_one(
            doc! {"user_id": &f.owner, "service_id": &f.catalog},
            doc! {"$set": {"default_effect": "deny"}},
        )
        .await
        .unwrap();
    let result = prepare(
        &f.state,
        Some(&context),
        None,
        &f.key,
        Effect {
            connection: &f.connection,
            owner: &f.owner,
            target: &target,
            method: "GET",
            path: "status",
            query: None,
            body: b"",
            content_type: None,
            headers: &[],
            endpoint: None,
            floors: &[ExecutionFloor::PersonalServiceApproval],
        },
    )
    .await;
    assert!(result.is_err());
    assert!(context.permit().is_none());
    assert!(f.server.received_requests().await.unwrap().is_empty());
    f.no_prompts().await;
    f.cleanup().await;
}

#[tokio::test]
async fn policy_insert_during_claim_is_not_sent_and_cannot_replay() {
    let f = Fixture::new("execution_policy_insert_race").await;
    f.enroll(true).await;
    let configs = f
        .state
        .db
        .collection::<Document>("service_approval_configs");
    let mut config = configs
        .find_one(doc! {"user_id":&f.owner})
        .await
        .unwrap()
        .unwrap();
    configs.delete_many(doc! {}).await.unwrap();
    let target = f.target().await;
    let mut grant = f.grant().await;
    f.store(&grant).await;
    let original = RequestContext::enrolled(Some(1), &f.owner, &f.key, None).unwrap();
    let initial = f.prepare(&original, &target).await;
    let contract = grant.execution.as_mut().unwrap().v1.as_mut().unwrap();
    contract.validity = Validity::Operation {
        operation_id: initial.operation_id.clone(),
        prepared_digest: initial.digest.clone(),
        expires_at: Utc::now() + chrono::Duration::hours(1),
    };
    grant.expires_at = super::contract::legacy_expiry(&contract.validity);
    f.state
        .db
        .collection::<ApprovalGrant>("approval_grants")
        .replace_one(doc! {"_id":&grant.id}, &grant)
        .await
        .unwrap();
    let mut context = RequestContext::enrolled(Some(1), &f.owner, &f.key, None).unwrap();
    Arc::get_mut(&mut context).unwrap().operation_id = initial.operation_id.clone();
    let permit = f.prepare(&context, &target).await;
    permit.reserve().await.unwrap();
    let mut session = f.state.db.client().start_session().await.unwrap();
    session.start_transaction().await.unwrap();
    permit.claim_in_session(&mut session, true).await.unwrap();
    // The snapshot saw no config row. Mongo cannot write-conflict on that
    // absence; commit a new deny before the dispatch transaction commits.
    config.insert("default_effect", "deny");
    configs.insert_one(config).await.unwrap();
    session.commit_transaction().await.unwrap();
    assert!(permit.after_barrier().await.is_err());
    let terminal = f
        .state
        .db
        .collection::<ApprovalGrant>("approval_grants")
        .find_one(doc! {"_id":&grant.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        terminal.execution_use.unwrap().state,
        ExecutionUseState::NotSent
    );
    assert!(terminal.revoked);
    assert!(permit.before_send(&target).await.is_err());
    assert!(f.server.received_requests().await.unwrap().is_empty());
    f.no_prompts().await;
    f.cleanup().await;
}

#[tokio::test]
async fn specialized_model_transport_claims_once_and_binds_the_actual_request() {
    let f = Fixture::new("execution_codex_send").await;
    f.enroll(true).await;
    f.server.reset().await;
    Mock::given(any_request())
        .respond_with(ResponseTemplate::new(400).set_body_json(json!({"error":"test rejection"})))
        .expect(1)
        .mount(&f.server)
        .await;
    let mut grant = f.grant().await;
    grant
        .execution
        .as_mut()
        .unwrap()
        .v1
        .as_mut()
        .unwrap()
        .operations
        .operations[0]
        .rule
        .method = "POST".into();
    f.store(&grant).await;
    let mut target = f.target().await;
    let body = json!({"model":"test","input":"hello","stream":true});
    let bytes = serde_json::to_vec(&body).unwrap();
    let headers = proxy_service::collect_forward_headers_with_defaults(
        &super::super::chatgpt_translator::execution_header_map(),
        &[],
    );
    let context = RequestContext::enrolled(Some(1), &f.owner, &f.key, None).unwrap();
    let effect = || Effect {
        connection: &f.connection,
        owner: &f.owner,
        target: &target,
        method: "POST",
        path: "status",
        query: None,
        body: &bytes,
        content_type: Some("application/json"),
        headers: &headers,
        endpoint: None,
        floors: &[ExecutionFloor::PersonalServiceApproval],
    };
    let initial = prepare(&f.state, Some(&context), None, &f.key, effect())
        .await
        .unwrap()
        .unwrap();
    let permission = grant.execution.as_mut().unwrap().v1.as_mut().unwrap();
    permission.validity = Validity::Operation {
        operation_id: initial.operation_id.clone(),
        prepared_digest: initial.digest.clone(),
        expires_at: bson::DateTime::now().to_chrono() + chrono::Duration::hours(1),
    };
    grant.expires_at = contract::legacy_expiry(&permission.validity);
    f.state
        .db
        .collection::<ApprovalGrant>("approval_grants")
        .replace_one(doc! {"_id":&grant.id}, &grant)
        .await
        .unwrap();
    let mut context = RequestContext::enrolled(Some(1), &f.owner, &f.key, None).unwrap();
    Arc::get_mut(&mut context)
        .unwrap()
        .operation_id
        .clone_from(&initial.operation_id);
    let permit = prepare(&f.state, Some(&context), None, &f.key, effect())
        .await
        .unwrap()
        .unwrap();
    let wrong = f
        .state
        .http_client
        .post(format!("{}/elsewhere", f.server.uri()))
        .body(bytes)
        .build()
        .unwrap();
    assert!(
        permit
            .validate_specialized_request(&target, &wrong)
            .is_err()
    );
    assert!(f.server.received_requests().await.unwrap().is_empty());
    target.execution_permission = Some(permit);
    let url = format!("{}/status", f.server.uri());
    let response = super::super::chatgpt_translator::send_to_chatgpt_with_api_url(
        &body,
        "test-token",
        true,
        false,
        None,
        &url,
        None,
        None,
        Some(&target),
    )
    .await
    .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(
        super::super::chatgpt_translator::send_to_chatgpt_with_api_url(
            &body,
            "test-token",
            true,
            false,
            None,
            &url,
            None,
            None,
            Some(&target)
        )
        .await
        .is_err()
    );
    assert_eq!(f.server.received_requests().await.unwrap().len(), 1);
    f.no_prompts().await;
    f.cleanup().await;
}

#[tokio::test]
async fn mcp_denial_and_gap_keep_transport_parity_without_provider_effects() {
    for meta in [false, true] {
        let f = Fixture::new(if meta {
            "execution_mcp_meta_gap"
        } else {
            "execution_mcp_endpoint_gap"
        })
        .await;
        f.state
            .db
            .collection::<crate::models::service_endpoint::ServiceEndpoint>("service_endpoints")
            .insert_one(test_utils::test_service_endpoint(
                &f.catalog, "status", "GET", "/status",
            ))
            .await
            .unwrap();
        f.enroll(true).await;
        let mut grant = f.grant().await;
        grant.revoked = true;
        f.store(&grant).await;
        let call = || async {
            let params = if meta {
                json!({"name":"nyx__call_tool","arguments":{"tool_name":"execution-fixture__status","arguments_json":"{}"}})
            } else {
                json!({"name":"execution-fixture__status","arguments":{}})
            };
            let response = f
                .router()
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/mcp")
                        .header("x-api-key", &f.raw)
                        .header("content-type", "application/json")
                        .body(Body::from(
                            json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":params})
                                .to_string(),
                        ))
                        .unwrap(),
                )
                .await
                .unwrap();
            let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
                .await
                .unwrap();
            serde_json::from_slice::<serde_json::Value>(&bytes).unwrap()
        };
        let denied = call().await;
        assert_eq!(denied["result"]["isError"], true, "{denied}");
        f.no_prompts().await;
        f.state
            .db
            .collection::<Document>("approval_grants")
            .delete_many(doc! {})
            .await
            .unwrap();
        let channel =
            super::super::notification_service::get_or_create_channel(&f.state.db, &f.owner)
                .await
                .unwrap();
        f.state
            .db
            .collection::<Document>("notification_channels")
            .update_one(
                doc! {"_id":channel.id},
                doc! {"$set":{"approval_timeout_secs":1_i32}},
            )
            .await
            .unwrap();
        let mut counts = Vec::new();
        for enrolled in [false, true] {
            f.enroll(enrolled).await;
            f.state
                .db
                .collection::<Document>("approval_requests")
                .delete_many(doc! {})
                .await
                .unwrap();
            let gap = call().await;
            assert_eq!(gap["result"]["isError"], true, "{gap}");
            counts.push(
                f.state
                    .db
                    .collection::<Document>("approval_requests")
                    .count_documents(doc! {})
                    .await
                    .unwrap(),
            );
        }
        assert_eq!(
            counts,
            vec![1, 1],
            "the same legacy approval request is required on either path"
        );
        assert!(f.server.received_requests().await.unwrap().is_empty());
        f.cleanup().await;
    }
}

#[path = "proposal_tests.rs"]
pub(super) mod proposal_tests;

#[path = "enrollment_tests.rs"]
mod enrollment_tests;
#[path = "migration_tests.rs"]
mod migration_tests;
#[path = "proposal_v2_tests.rs"]
mod proposal_v2_tests;

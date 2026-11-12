use crate::{
    models::{downstream_service::DownstreamService, user::UserType},
    services::{
        service_dispatch_guard::ServiceDispatchGuard, service_dispatch_registry as registry,
    },
};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

struct Fixture {
    db: Database,
    server: MockServer,
    catalog_server: MockServer,
    user_service_id: String,
    endpoint_id: String,
    owner: String,
    catalog: DownstreamService,
    target: ProxyTarget,
    keys: EncryptionKeys,
    jwt: JwtKeys,
    config: AppConfig,
}

#[tokio::test]
async fn unbound_gateway_selects_a_real_connection_and_registry_requires_one() {
    use crate::services::assistant_cma_gateway::CmaGateway;
    let f = Fixture::new().await;
    assert_ne!(f.catalog.id, f.user_service_id);
    let gateway = CatalogCmaGateway::from_owner_parts(&f.db, &f.keys, &f.jwt, &f.config);
    let connection = gateway.connection(&f.owner).await.unwrap();
    assert_eq!(connection.user_service_id, f.user_service_id);
    assert_eq!(
        connection.execution_base_url,
        format!("{}/", f.server.uri())
    );
    let caller = registry::DispatchCaller {
        actor_user_id: &f.owner,
        resource_owner_id: &f.owner,
        user_service_id: None,
    };
    assert!(matches!(
        registry::admit(
            &f.db,
            &f.keys,
            &f.jwt,
            &f.config,
            &caller,
            true,
            &f.target,
            "GET",
            "/api/v1/agents",
            None,
            &[],
            || Ok(vec![].into()),
            false
        )
        .await,
        Err(AppError::Forbidden(_))
    ));
    assert!(f.server.received_requests().await.unwrap().is_empty());
    assert!(
        f.catalog_server
            .received_requests()
            .await
            .unwrap()
            .is_empty()
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn same_connection_retargeting_keeps_remote_resource_ids_independent() {
    use crate::services::assistant_cma_agent_service as agents;
    let f = Fixture::new().await;
    f.observations().await;
    let body = br#"{"machine_included":false}"#;
    let first = f
        .admit(&f.target, "/api/v1/agents", "target-a-key", body)
        .await
        .unwrap();
    let a = first.creation.as_ref().unwrap();
    change_creation(
        &f.db,
        a,
        CreationChange::Receipt(CreationReceipt::Lifecycle("op_1".into())),
    )
    .await
    .unwrap();
    record_creation_observation(
        &f.db,
        &f.gateway(),
        &f.owner,
        &f.catalog.id,
        "op_1",
        &serde_json::json!({"id":"op_1","agent_id":"agent_1"}),
    )
    .await
    .unwrap();
    let agent_a = agents::register_live(&f.db, &f.gateway(), &f.owner, "agent_1", None)
        .await
        .unwrap();
    Mock::given(method("GET"))
        .and(path("/api/v1/sandboxes/sandbox_1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id":"sandbox_1","owner":&f.owner,"status":"retired"
        })))
        .with_priority(1)
        .mount(&f.server)
        .await;
    let retired = sandboxes::observe(
        &f.db,
        &f.gateway(),
        &f.owner,
        &a.sandbox_ref_id,
        a.sandbox_generation,
    )
    .await
    .unwrap();
    assert!(!retired.occupies_account_slot);
    let saved_a = bson::to_document(&f.saved(&a.id).await).unwrap();
    let server_b = MockServer::start().await;
    f.db.collection::<bson::Document>("user_endpoints")
        .update_one(
            doc! {"_id":&f.endpoint_id},
            doc! {"$set":{"url":server_b.uri()}},
        )
        .await
        .unwrap();
    for (route, value) in [
        (
            "/api/v1/sandboxes/sandbox_1",
            serde_json::json!({"id":"sandbox_1","owner":&f.owner,"status":"active"}),
        ),
        (
            "/api/v1/agents/agent_1",
            serde_json::json!({"id":"agent_1","owner":&f.owner,"sandbox_id":"sandbox_1","status":"ready"}),
        ),
    ] {
        Mock::given(method("GET"))
            .and(path(route))
            .respond_with(ResponseTemplate::new(200).set_body_json(value))
            .mount(&server_b)
            .await;
    }
    let target_b = f.resolved_target().await;
    let gateway_b = CatalogCmaGateway::from_user_service_parts(
        &f.db,
        &f.keys,
        &f.jwt,
        &f.config,
        &f.user_service_id,
        Some(&f.catalog.id),
        Some(&server_b.uri()),
    );
    let second = f
        .admit(&target_b, "/api/v1/agents", "target-b-key", body)
        .await
        .unwrap();
    let b = second.creation.as_ref().unwrap();
    change_creation(
        &f.db,
        b,
        CreationChange::Receipt(CreationReceipt::Lifecycle("op_1".into())),
    )
    .await
    .unwrap();
    record_creation_observation(
        &f.db,
        &gateway_b,
        &f.owner,
        &f.catalog.id,
        "op_1",
        &serde_json::json!({"id":"op_1","agent_id":"agent_1"}),
    )
    .await
    .unwrap();
    let agent_b = agents::register_live(&f.db, &gateway_b, &f.owner, "agent_1", None)
        .await
        .unwrap();
    assert_ne!(agent_a.id, agent_b.id);
    assert_ne!(a.sandbox_ref_id, b.sandbox_ref_id);
    assert_eq!(f.saved(&b.id).await.submission_state, "confirmed");
    assert_eq!(b.user_service_id, a.user_service_id);
    assert_ne!(b.execution_base_url, a.execution_base_url);
    assert_eq!(bson::to_document(&f.saved(&a.id).await).unwrap(), saved_a);
    assert!(gateway_b.replay_creation(&f.keys, a, body).await.is_err());
    assert_eq!(
        sandboxes::get_owned(&f.db, &f.owner, &a.sandbox_ref_id)
            .await
            .unwrap()
            .admission_state,
        crate::models::assistant_cma_sandbox::SandboxAdmissionState::Retired
    );
    f.db.drop().await.unwrap();
}

impl Fixture {
    async fn new() -> Self {
        let db = crate::test_utils::connect_transaction_test_database("cma_fingerprint").await;
        let server = MockServer::start().await;
        let catalog_server = MockServer::start().await;
        let owner = uuid::Uuid::new_v4().to_string();
        db.collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
            .insert_one(crate::test_utils::test_user(&owner, UserType::Person))
            .await
            .unwrap();
        let mut catalog = crate::test_utils::test_auto_connected_catalog_service();
        catalog.slug = "cma".into();
        catalog.base_url = catalog_server.uri();
        catalog.identity_propagation_mode = "jwt".into();
        catalog.identity_include_user_id = false;
        catalog.identity_include_email = false;
        catalog.identity_include_name = false;
        db.collection::<DownstreamService>(crate::models::downstream_service::COLLECTION_NAME)
            .insert_one(&catalog)
            .await
            .unwrap();
        let user_service_id = uuid::Uuid::new_v4().to_string();
        let endpoint_id = uuid::Uuid::new_v4().to_string();
        let endpoint = crate::test_utils::test_user_endpoint(
            &endpoint_id,
            &owner,
            "Connection B",
            &server.uri(),
            None,
            Some(&catalog.id),
        );
        db.collection::<crate::models::user_endpoint::UserEndpoint>("user_endpoints")
            .insert_one(endpoint)
            .await
            .unwrap();
        let mut connection = crate::test_utils::test_user_service(
            &user_service_id,
            &owner,
            "cma-connection-b",
            &endpoint_id,
            Some(&catalog.id),
            None,
        );
        connection.identity_propagation_mode = "jwt".into();
        connection.identity_include_user_id = true;
        connection.identity_include_email = true;
        connection.identity_include_name = true;
        db.collection::<crate::models::user_service::UserService>("user_services")
            .insert_one(connection)
            .await
            .unwrap();
        let keys = crate::test_utils::test_encryption_keys();
        let target = crate::services::proxy_service::resolve_proxy_target_by_user_service_id(
            &db,
            &keys,
            &owner,
            &user_service_id,
            None,
            Some(&catalog.id),
            crate::services::proxy_service::ProxyExecutionContext::new(
                None,
                crate::mw::rate_limit::PlatformUserRateLimitPolicy::disabled(),
            ),
        )
        .await
        .unwrap()
        .unwrap()
        .target;
        crate::services::security_readiness_service::ready_test_replica(&db).await;
        crate::services::feature_flag_service::set_platform_override(
            &db,
            crate::services::feature_flag_service::ASSISTANT_CMA_SANDBOXES_FLAG_KEY,
            &crate::services::feature_flag_service::FlagTarget::User(owner.clone()),
            true,
            &owner,
        )
        .await
        .unwrap();
        Self {
            db,
            server,
            catalog_server,
            user_service_id,
            endpoint_id,
            owner,
            catalog,
            target,
            keys,
            jwt: crate::test_utils::cached_test_jwt_keys(),
            config: crate::test_utils::test_app_config(),
        }
    }

    fn connection_target(service: DownstreamService) -> ProxyTarget {
        ProxyTarget {
            catalog_slug: Some("cma".into()),
            execution_request: None,
            execution_permission: None,
            credential_deferred: false,
            workspace_destinations_pending: false,
            target_id: None,
            base_url: service.base_url.clone(),
            auth_method: "none".into(),
            auth_key_name: String::new(),
            credential: Default::default(),
            service,
            catalog_default_headers: vec![],
            user_service_default_headers: vec![],
            ws_frame_injections: vec![],
            connection_id: None,
        }
    }

    async fn set_auth(&self, method: &str, credential: &str) {
        use crate::services::user_api_key_service::{CreateApiKeyParams, build_api_key};
        let key = build_api_key(
            &self.keys,
            &self.owner,
            CreateApiKeyParams {
                label: "Connection B credential",
                credential_type: "api_key",
                credential,
                access_token: None,
                refresh_token: None,
                token_scopes: None,
                expires_at: None,
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
        self.db
            .collection::<crate::models::user_api_key::UserApiKey>("user_api_keys")
            .insert_one(&key)
            .await
            .unwrap();
        self.db.collection::<bson::Document>("user_services").update_one(
            doc! {"_id":&self.user_service_id},
            doc! {"$set":{"auth_method":method,"auth_key_name":"x-cma-secret","api_key_id":&key.id,"identity_propagation_mode":"none"}},
        ).await.unwrap();
    }

    async fn resolved_target(&self) -> ProxyTarget {
        let resolution: futures::future::BoxFuture<'_, _> = Box::pin(
            crate::services::proxy_service::resolve_proxy_target_by_user_service_id(
                &self.db,
                &self.keys,
                &self.owner,
                &self.user_service_id,
                None,
                Some(&self.catalog.id),
                crate::services::proxy_service::ProxyExecutionContext::new(
                    None,
                    crate::mw::rate_limit::PlatformUserRateLimitPolicy::disabled(),
                ),
            ),
        );
        resolution.await.unwrap().unwrap().target
    }

    fn dispatch_caller(&self) -> registry::DispatchCaller<'_> {
        registry::DispatchCaller {
            actor_user_id: &self.owner,
            resource_owner_id: &self.owner,
            user_service_id: Some(&self.user_service_id),
        }
    }

    async fn admitted_creation(
        &self,
        target: &ProxyTarget,
        key: &str,
        body: &[u8],
    ) -> registry::DispatchAdmission<'_> {
        registry::admit(
            &self.db,
            &self.keys,
            &self.jwt,
            &self.config,
            &self.dispatch_caller(),
            true,
            target,
            "POST",
            "/api/v1/sandboxes",
            None,
            body,
            || Ok(vec![("Idempotency-Key".into(), key.into())].into()),
            false,
        )
        .await
        .unwrap()
    }

    async fn forward_original(
        &self,
        target: &ProxyTarget,
        admission: &registry::DispatchAdmission<'_>,
        request: reqwest::Request,
    ) -> Result<reqwest::Response, crate::services::proxy_service::ForwardRequestError> {
        use crate::services::{
            cloud_response_cache::CloudResponseCache,
            provider_token_exchange_service::TokenExchangeCache, proxy_service,
        };
        let client = reqwest::Client::new();
        let tokens = TokenExchangeCache::new();
        let cache = CloudResponseCache::new(0);
        let caller = self.dispatch_caller();
        let forwarding: futures::future::BoxFuture<'_, _> =
            Box::pin(proxy_service::forward_request_with_resource_proof(
                &client,
                target,
                request.method().clone(),
                request.url().path(),
                request.url().query(),
                request.headers().clone(),
                proxy_service::ProxyBody::Buffered(
                    request
                        .body()
                        .and_then(reqwest::Body::as_bytes)
                        .map(bytes::Bytes::copy_from_slice),
                ),
                vec![],
                vec![],
                None,
                &tokens,
                &cache,
                vec![],
                crate::services::billing::route_inventory::internal_node_dispatch_permit(),
                None,
                None,
                Some(admission),
                Some(&caller),
            ));
        forwarding.await
    }

    async fn creation_for_key(&self, key: &str) -> AssistantCmaCreation {
        self.db
            .collection::<AssistantCmaCreation>(COLLECTION_NAME)
            .find_one(doc! {"user_id":&self.owner,"idempotency_key":key})
            .await
            .unwrap()
            .unwrap()
    }

    fn gateway(&self) -> CatalogCmaGateway<'_> {
        CatalogCmaGateway::from_user_service_parts(
            &self.db,
            &self.keys,
            &self.jwt,
            &self.config,
            &self.user_service_id,
            Some(&self.catalog.id),
            Some(&self.server.uri()),
        )
    }

    async fn admit(
        &self,
        target: &ProxyTarget,
        route: &str,
        key: &str,
        body: &[u8],
    ) -> AppResult<CmaDispatchPermit<'_>> {
        admit(
            &self.db,
            &self.keys,
            &self.jwt,
            &self.config,
            &self.owner,
            &self.user_service_id,
            true,
            target,
            "POST",
            route,
            None,
            body,
            &[("Idempotency-Key".into(), key.into())],
            false,
        )
        .await
    }

    fn request(&self, route: &str, key: &str, body: &[u8]) -> reqwest::Request {
        reqwest::Client::new()
            .post(format!("{}{route}", self.server.uri()))
            .header("Idempotency-Key", key)
            .body(body.to_vec())
            .build()
            .unwrap()
    }

    async fn saved(&self, id: &str) -> AssistantCmaCreation {
        self.db
            .collection::<AssistantCmaCreation>(COLLECTION_NAME)
            .find_one(doc! {"_id":id})
            .await
            .unwrap()
            .unwrap()
    }

    async fn assert_single_intent(&self) {
        assert_eq!(
            self.db
                .collection::<AssistantCmaCreation>(COLLECTION_NAME)
                .count_documents(doc! {"user_id":&self.owner})
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            self.db
                .collection::<AssistantCmaSandbox>(
                    crate::models::assistant_cma_sandbox::COLLECTION_NAME
                )
                .count_documents(doc! {"user_id":&self.owner,"occupies_account_slot":true})
                .await
                .unwrap(),
            1
        );
    }

    async fn observations(&self) {
        Mock::given(method("GET"))
            .and(path("/api/v1/sandboxes/sandbox_1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id":"sandbox_1","owner":&self.owner,"status":"active","machine_included":false
            })))
            .mount(&self.server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v1/agents/agent_1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id":"agent_1","owner":&self.owner,"sandbox_id":"sandbox_1","status":"ready"
            })))
            .mount(&self.server)
            .await;
        Mock::given(method("GET"))
            .and(path("/api/v1/agent-lifecycle/op_1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id":"op_1","status":"ready","agent_id":"agent_1"
            })))
            .mount(&self.server)
            .await;
    }

    async fn post_count(&self) -> usize {
        self.server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .filter(|request| request.method == "POST")
            .count()
    }
}

#[tokio::test]
async fn common_get_dispatch_accepts_connection_identity_differences() {
    use base64::Engine;
    let f = Fixture::new().await;
    let owner = crate::test_utils::test_user(&f.owner, UserType::Person);
    let assertion = crate::services::identity_service::generate_identity_assertion(
        &f.jwt,
        &f.config,
        &crate::services::identity_service::Principal::from(&owner),
        &f.target.service,
        &f.db,
    )
    .await
    .unwrap();
    let claims: Value = serde_json::from_slice(
        &base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(assertion.split('.').nth(1).unwrap())
            .unwrap(),
    )
    .unwrap();
    assert_eq!(claims["email"], owner.email);
    assert_eq!(claims["name"], owner.display_name.unwrap());
    let caller = registry::DispatchCaller {
        actor_user_id: &f.owner,
        resource_owner_id: &f.owner,
        user_service_id: Some(&f.user_service_id),
    };
    // Both REST and MCP enter this registry with their resolved target. Exercise
    // each normalized read route through the shared admission and final guard.
    for route in ["/api/v1/agents", "/api/v1/sandboxes"] {
        Mock::given(method("GET"))
            .and(path(route))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"items":[]})))
            .mount(&f.server)
            .await;
        let admission = registry::admit(
            &f.db,
            &f.keys,
            &f.jwt,
            &f.config,
            &caller,
            true,
            &f.target,
            "GET",
            route,
            None,
            b"",
            || Ok(vec![].into()),
            false,
        )
        .await
        .unwrap();
        let guard = registry::verify(Some(&admission), &f.target, Some(&caller))
            .unwrap()
            .unwrap();
        let request = reqwest::Client::new()
            .get(format!("{}{route}", f.server.uri()))
            .header("X-NyxID-Identity-Token", &assertion)
            .build()
            .unwrap();
        guard.before_send(&request).await.unwrap();
        let response = reqwest::Client::new().execute(request).await.unwrap();
        assert_eq!(
            guard.observe_response(response).await.unwrap().status(),
            200
        );
    }
    let received = f.server.received_requests().await.unwrap();
    assert_eq!(received.len(), 2);
    for request in received {
        assert_eq!(request.headers["x-nyxid-identity-token"], assertion);
    }
    assert_eq!(
        f.db.collection::<AssistantCmaSandbox>(
            crate::models::assistant_cma_sandbox::COLLECTION_NAME
        )
        .count_documents(doc! {})
        .await
        .unwrap(),
        0
    );
    assert!(
        f.catalog_server
            .received_requests()
            .await
            .unwrap()
            .is_empty()
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn connection_b_creation_confirms_once_without_catalog_fingerprint() {
    let f = Fixture::new().await;
    f.observations().await;
    let body = br#"{ "name": "original agent", "machine_included": false }"#;
    let key = "fingerprint-lifecycle-key";
    let first = f
        .admit(&f.target, "/api/v1/agents", key, body)
        .await
        .unwrap();
    let row = first.creation.as_ref().unwrap();
    assert_eq!(row.user_service_id, f.user_service_id);
    assert_eq!(row.execution_base_url, format!("{}/", f.server.uri()));
    assert_ne!(row.execution_base_url, f.catalog.base_url);
    let (left, right) = tokio::join!(
        f.admit(&f.target, "/api/v1/agents", key, body),
        f.admit(&f.target, "/api/v1/agents", key, body)
    );
    for retry in [left.unwrap(), right.unwrap()] {
        assert_eq!(retry.creation.as_ref().unwrap().id, row.id);
        assert_eq!(
            retry.sandbox.as_ref().unwrap().id,
            first.sandbox.as_ref().unwrap().id
        );
        assert_eq!(
            retry.sandbox.as_ref().unwrap().generation,
            row.sandbox_generation
        );
    }
    f.assert_single_intent().await;
    let context =
        RecordContext::new(COLLECTION_NAME, &row.id, &f.owner, &["request_ciphertext"]).unwrap();
    assert_eq!(
        f.keys
            .decrypt_record(row.request_ciphertext.as_deref().unwrap(), &context)
            .await
            .unwrap()
            .as_slice(),
        body
    );
    Mock::given(method("POST"))
        .and(path("/api/v1/agents"))
        .respond_with(ResponseTemplate::new(202).set_body_json(serde_json::json!({
            "id":"op_1","status":"ready","agent_id":"agent_1"
        })))
        .expect(1)
        .mount(&f.server)
        .await;
    let request = f.request("/api/v1/agents", key, body);
    first.before_send(&request).await.unwrap();
    let response = reqwest::Client::new().execute(request).await.unwrap();
    assert_eq!(
        first.observe_response(response).await.unwrap().status(),
        202
    );
    let confirmed = f.saved(&row.id).await;
    assert_eq!(confirmed.submission_state, "confirmed");
    assert_eq!(confirmed.cma_agent_id.as_deref(), Some("agent_1"));
    assert_eq!(confirmed.cma_sandbox_id.as_deref(), Some("sandbox_1"));
    assert!(confirmed.request_ciphertext.is_none());
    assert_eq!(
        f.db.collection::<bson::Document>(crate::models::assistant_cma_agent::COLLECTION_NAME)
            .count_documents(doc! {"user_id":&f.owner,"cma_agent_id":"agent_1"})
            .await
            .unwrap(),
        1
    );
    assert!(
        f.admit(&f.target, "/api/v1/agents", key, body)
            .await
            .is_err()
    );
    f.assert_single_intent().await;
    assert!(
        f.catalog_server
            .received_requests()
            .await
            .unwrap()
            .is_empty()
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn connection_b_recovery_replays_original_bytes_and_key() {
    let f = Fixture::new().await;
    f.observations().await;
    let body = br#"{ "name": "recover exact bytes", "machine_included": false }"#;
    let key = "fingerprint-recovery-key";
    Mock::given(method("POST"))
        .and(path("/api/v1/sandboxes"))
        .respond_with(
            ResponseTemplate::new(201).set_body_json(serde_json::json!({"id":"sandbox_1"})),
        )
        .expect(2)
        .mount(&f.server)
        .await;
    let permit = f
        .admit(&f.target, "/api/v1/sandboxes", key, body)
        .await
        .unwrap();
    let row = permit.creation.as_ref().unwrap();
    let request = f.request("/api/v1/sandboxes", key, body);
    permit.before_send(&request).await.unwrap();
    // The provider accepts the resource, but NyxID does not observe its receipt.
    drop(reqwest::Client::new().execute(request).await.unwrap());
    let result = recover_creation(
        &f.db,
        &f.keys,
        &f.gateway(),
        &f.owner,
        &row.sandbox_ref_id,
        row.sandbox_generation,
    )
    .await
    .unwrap();
    assert!(!result.pending);
    assert_eq!(f.saved(&row.id).await.submission_state, "confirmed");
    for request in f
        .server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .filter(|request| request.method == "POST")
    {
        assert_eq!(request.body, body);
        assert_eq!(request.headers["idempotency-key"], key);
    }
    let again = recover_creation(
        &f.db,
        &f.keys,
        &f.gateway(),
        &f.owner,
        &row.sandbox_ref_id,
        row.sandbox_generation,
    )
    .await
    .unwrap();
    assert!(!again.pending);
    assert_eq!(f.post_count().await, 2);
    f.assert_single_intent().await;
    assert!(
        f.catalog_server
            .received_requests()
            .await
            .unwrap()
            .is_empty()
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn legacy_records_are_isolated_without_guessing_a_connection() {
    let f = Fixture::new().await;
    let legacy = doc! {"_id":"legacy", "user_id":&f.owner,"cma_service_id":&f.catalog.id,
    "source":"nyxbot","occupies_account_slot":true,"deployment_fingerprint":"old-fingerprint"};
    f.db.collection::<bson::Document>(crate::models::assistant_cma_sandbox::COLLECTION_NAME)
        .insert_one(legacy.clone())
        .await
        .unwrap();
    f.db.collection::<bson::Document>(COLLECTION_NAME)
        .insert_one(legacy)
        .await
        .unwrap();
    assert!(
        sandboxes::get_owned(&f.db, &f.owner, "legacy")
            .await
            .is_err()
    );
    let list = sandboxes::list_live(&f.db, &f.gateway(), &f.owner, None)
        .await
        .unwrap();
    assert!(list.sandboxes.is_empty());
    assert_eq!(list.occupied, 1, "old resource slot is not released");
    assert!(
        recover_creation(&f.db, &f.keys, &f.gateway(), &f.owner, "legacy", 1)
            .await
            .is_err()
    );
    let saved =
        f.db.collection::<bson::Document>(COLLECTION_NAME)
            .find_one(doc! {"_id":"legacy"})
            .await
            .unwrap()
            .unwrap();
    assert!(!saved.contains_key("user_service_id"));
    assert!(f.server.received_requests().await.unwrap().is_empty());
    assert!(
        f.catalog_server
            .received_requests()
            .await
            .unwrap()
            .is_empty()
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn connection_retargeting_and_expired_recovery_remain_closed() {
    let f = Fixture::new().await;
    let body = br#"{"machine_included":false}"#;
    let key = "catalog-drift-key";
    for change in ["id", "actual_base_url"] {
        let mut other = Fixture::connection_target(f.target.service.clone());
        if change == "id" {
            other.service.id = uuid::Uuid::new_v4().to_string();
        } else {
            other.base_url = format!("{}/other-deployment", f.server.uri());
        }
        assert!(
            f.admit(&other, "/api/v1/sandboxes", key, body)
                .await
                .is_err()
        );
    }
    let permit = f
        .admit(&f.target, "/api/v1/sandboxes", key, body)
        .await
        .unwrap();
    let row = permit.creation.as_ref().unwrap();
    // Catalog template identity changes must not revoke this personal connection.
    f.db.collection::<bson::Document>(crate::models::downstream_service::COLLECTION_NAME)
        .update_one(doc! {"_id":&f.catalog.id},doc! {"$set":{"identity_include_email":true,"identity_jwt_audience":"template-only-change"}}).await.unwrap();
    permit
        .before_send(&f.request("/api/v1/sandboxes", key, body))
        .await
        .unwrap();
    // The actual connection endpoint is the recovery fence.
    f.db.collection::<bson::Document>("user_endpoints")
        .update_one(
            doc! {"_id":&f.endpoint_id},
            doc! {"$set":{"url":format!("{}/changed",f.server.uri())}},
        )
        .await
        .unwrap();
    assert!(
        permit
            .before_send(&f.request("/api/v1/sandboxes", key, body))
            .await
            .is_err()
    );
    assert!(
        recover_creation(
            &f.db,
            &f.keys,
            &f.gateway(),
            &f.owner,
            &row.sandbox_ref_id,
            row.sandbox_generation
        )
        .await
        .is_err()
    );
    assert_eq!(
        f.saved(&row.id).await.execution_base_url,
        row.execution_base_url
    );
    f.db.collection::<bson::Document>("user_endpoints")
        .update_one(
            doc! {"_id":&f.endpoint_id},
            doc! {"$set":{"url":f.server.uri()}},
        )
        .await
        .unwrap();
    for change in ["method", "url", "query", "body", "key"] {
        let mut request = f.request("/api/v1/sandboxes", key, body);
        match change {
            "method" => *request.method_mut() = reqwest::Method::DELETE,
            "url" => request.url_mut().set_path("/api/v1/agents"),
            "query" => request.url_mut().set_query(Some("changed=1")),
            "body" => *request.body_mut() = Some(reqwest::Body::from("{}")),
            _ => {
                request.headers_mut().insert(
                    "idempotency-key",
                    reqwest::header::HeaderValue::from_static("changed-key"),
                );
            }
        }
        assert!(
            permit
                .validate_original_request(
                    crate::services::service_dispatch_guard::OriginalRequest {
                        method: request.method().as_str(),
                        path: request.url().path(),
                        query: request.url().query(),
                        body: request
                            .body()
                            .and_then(reqwest::Body::as_bytes)
                            .unwrap_or_default(),
                        headers: request.headers(),
                    }
                )
                .is_err(),
            "{change}"
        );
        if matches!(change, "method" | "url" | "key") {
            assert!(
                permit.before_send(&request).await.is_err(),
                "final {change}"
            );
        }
    }
    let mut changed_origin = f.request("/api/v1/sandboxes", key, body);
    changed_origin
        .url_mut()
        .set_host(Some("localhost"))
        .unwrap();
    assert!(permit.before_send(&changed_origin).await.is_err());
    permit
        .before_send(&f.request("/api/v1/sandboxes", key, body))
        .await
        .unwrap();
    f.db.collection::<AssistantCmaCreation>(COLLECTION_NAME).update_one(doc! {"_id":&row.id},
        doc! {"$set":{"first_dispatched_at":bson::DateTime::from_chrono(Utc::now()-chrono::Duration::hours(23))}}).await.unwrap();
    assert!(
        recover_creation(
            &f.db,
            &f.keys,
            &f.gateway(),
            &f.owner,
            &row.sandbox_ref_id,
            row.sandbox_generation
        )
        .await
        .is_err()
    );
    assert_eq!(f.post_count().await, 0);
    let retained = f.saved(&row.id).await;
    assert!(retained.request_ciphertext.is_some());
    assert_eq!(retained.idempotency_key, key);
    f.assert_single_intent().await;
    assert!(
        f.catalog_server
            .received_requests()
            .await
            .unwrap()
            .is_empty()
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn gateway_json_and_sse_use_connection_b_authentication() {
    use crate::services::assistant_cma_gateway::CmaGateway;
    let f = Fixture::new().await;
    f.observations().await;
    f.db.collection::<bson::Document>("user_services").update_one(doc! {"_id":&f.user_service_id},doc! {"$set":{
        "identity_propagation_mode":"none",
        "default_request_headers":[{"name":"x-connection-auth","value":"connection-b-secret","sensitive":true,"overridable":false}],
    }}).await.unwrap();
    let gateway = f.gateway();
    gateway.get_agent(&f.owner, "agent_1").await.unwrap();
    Mock::given(method("GET"))
        .and(path("/api/v2/agents/agent_1/events"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("Content-Type", "text/event-stream")
                .set_body_string(": keepalive\n\n"),
        )
        .mount(&f.server)
        .await;
    let response = gateway
        .open_event_stream(&f.owner, "agent_1", None, None)
        .await
        .unwrap();
    assert_eq!(response.text().await.unwrap(), ": keepalive\n\n");
    let requests = f.server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 2);
    for request in requests {
        assert_eq!(request.headers["x-connection-auth"], "connection-b-secret");
        assert!(!request.headers.contains_key("x-nyxid-identity-token"));
    }
    assert!(
        f.catalog_server
            .received_requests()
            .await
            .unwrap()
            .is_empty()
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn same_remote_agent_ids_do_not_cross_connections() {
    use crate::services::{
        assistant_cma_agent_service as agents, assistant_cma_gateway::CmaGateway,
    };
    let f = Fixture::new().await;
    f.observations().await;
    let permit = f
        .admit(
            &f.target,
            "/api/v1/agents",
            "first-connection-key",
            br#"{"machine_included":false}"#,
        )
        .await
        .unwrap();
    let creation = permit.creation.as_ref().unwrap();
    change_creation(
        &f.db,
        creation,
        CreationChange::Receipt(CreationReceipt::Lifecycle("op_1".into())),
    )
    .await
    .unwrap();
    record_creation_observation(
        &f.db,
        &f.gateway(),
        &f.owner,
        &f.catalog.id,
        "op_1",
        &serde_json::json!({"id":"op_1","agent_id":"agent_1"}),
    )
    .await
    .unwrap();
    let first = agents::register_live(&f.db, &f.gateway(), &f.owner, "agent_1", None)
        .await
        .unwrap();
    let connection = uuid::Uuid::new_v4().to_string();
    let endpoint = uuid::Uuid::new_v4().to_string();
    let server = MockServer::start().await;
    f.db.collection::<crate::models::user_endpoint::UserEndpoint>("user_endpoints")
        .insert_one(crate::test_utils::test_user_endpoint(
            &endpoint,
            &f.owner,
            "Second CMA",
            &server.uri(),
            None,
            Some(&f.catalog.id),
        ))
        .await
        .unwrap();
    f.db.collection::<crate::models::user_service::UserService>("user_services")
        .insert_one(crate::test_utils::test_user_service(
            &connection,
            &f.owner,
            "cma-second",
            &endpoint,
            Some(&f.catalog.id),
            None,
        ))
        .await
        .unwrap();
    let mut parent = sandboxes::get_owned(&f.db, &f.owner, &creation.sandbox_ref_id)
        .await
        .unwrap();
    parent.id = uuid::Uuid::new_v4().to_string();
    parent.user_service_id = connection.clone();
    parent.execution_base_url = format!("{}/", server.uri());
    parent.occupies_account_slot = false;
    parent.idempotency_key = Some("second-connection-key".into());
    f.db.collection::<AssistantCmaSandbox>(crate::models::assistant_cma_sandbox::COLLECTION_NAME)
        .insert_one(parent)
        .await
        .unwrap();
    for (route, body) in [
        (
            "/api/v1/agents/agent_1",
            serde_json::json!({"id":"agent_1","owner":&f.owner,"sandbox_id":"sandbox_1","status":"ready"}),
        ),
        (
            "/api/v1/sandboxes/sandbox_1",
            serde_json::json!({"id":"sandbox_1","owner":&f.owner,"status":"active"}),
        ),
    ] {
        Mock::given(method("GET"))
            .and(path(route))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .mount(&server)
            .await;
    }
    let gateway = CatalogCmaGateway::from_user_service_parts(
        &f.db,
        &f.keys,
        &f.jwt,
        &f.config,
        &connection,
        Some(&f.catalog.id),
        Some(&server.uri()),
    );
    let second = agents::register_live(&f.db, &gateway, &f.owner, "agent_1", None)
        .await
        .unwrap();
    assert_ne!(first.id, second.id);
    assert_eq!(first.user_service_id, f.user_service_id);
    assert_eq!(second.user_service_id, connection);
    assert_eq!(
        gateway.service_id(&f.owner).await.unwrap(),
        first.cma_service_id
    );
    assert!(
        agents::require_bound(&f.db, &gateway, &f.owner, &first.id)
            .await
            .is_err()
    );
    assert!(
        f.catalog_server
            .received_requests()
            .await
            .unwrap()
            .is_empty()
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn gateway_query_auth_preserves_the_admitted_request() {
    use crate::services::assistant_cma_gateway::CmaGateway;
    let f = Fixture::new().await;
    f.observations().await;
    f.set_auth("query", "connection-b-secret").await;
    f.gateway().get_agent(&f.owner, "agent_1").await.unwrap();
    let requests = f.server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    assert!(
        requests[0]
            .url
            .query_pairs()
            .any(|(key, value)| key == "x-cma-secret" && value == "connection-b-secret")
    );
    assert!(
        f.catalog_server
            .received_requests()
            .await
            .unwrap()
            .is_empty()
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn common_body_auth_preserves_original_creation_and_injects_the_secret() {
    let f = Fixture::new().await;
    f.set_auth("body", "connection-b-secret").await;
    Mock::given(method("POST"))
        .and(path("/api/v1/sandboxes"))
        .respond_with(
            ResponseTemplate::new(202).set_body_json(serde_json::json!({"operation_id":"op_auth"})),
        )
        .mount(&f.server)
        .await;
    use crate::services::{
        cloud_response_cache::CloudResponseCache,
        provider_token_exchange_service::TokenExchangeCache, proxy_service,
    };
    let target = proxy_service::resolve_proxy_target_by_user_service_id(
        &f.db,
        &f.keys,
        &f.owner,
        &f.user_service_id,
        None,
        Some(&f.catalog.id),
        proxy_service::ProxyExecutionContext::new(
            None,
            crate::mw::rate_limit::PlatformUserRateLimitPolicy::disabled(),
        ),
    )
    .await
    .unwrap()
    .unwrap()
    .target;
    let body = br#"{ "machine_included": false }"#;
    let caller = registry::DispatchCaller {
        actor_user_id: &f.owner,
        resource_owner_id: &f.owner,
        user_service_id: Some(&f.user_service_id),
    };
    let admission = registry::admit(
        &f.db,
        &f.keys,
        &f.jwt,
        &f.config,
        &caller,
        true,
        &target,
        "POST",
        "/api/v1/sandboxes",
        None,
        body,
        || Ok(vec![("Idempotency-Key".into(), "body-auth-key".into())].into()),
        false,
    )
    .await
    .unwrap();
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert("idempotency-key", "body-auth-key".parse().unwrap());
    proxy_service::forward_request_with_resource_proof(
        &reqwest::Client::new(),
        &target,
        reqwest::Method::POST,
        "/api/v1/sandboxes",
        None,
        headers,
        proxy_service::ProxyBody::Buffered(Some(bytes::Bytes::copy_from_slice(body))),
        vec![],
        vec![],
        None,
        &TokenExchangeCache::new(),
        &CloudResponseCache::new(0),
        vec![],
        crate::services::billing::route_inventory::internal_node_dispatch_permit(),
        None,
        None,
        Some(&admission),
        Some(&caller),
    )
    .await
    .unwrap();
    let requests = f.server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["x-cma-secret"], "connection-b-secret");
    assert_eq!(body["machine_included"], false);
    assert_eq!(requests[0].headers["idempotency-key"], "body-auth-key");
    assert!(
        f.catalog_server
            .received_requests()
            .await
            .unwrap()
            .is_empty()
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn common_query_auth_creation_uses_connection_b_and_retains_original_body() {
    let f = Fixture::new().await;
    f.set_auth("query", "connection-b-secret").await;
    let target = f.resolved_target().await;
    let body = br#"{ "machine_included": false }"#;
    let key = "query-auth-creation";
    Mock::given(method("POST"))
        .and(path("/api/v1/sandboxes"))
        .respond_with(
            ResponseTemplate::new(202)
                .set_body_json(serde_json::json!({"operation_id":"op_query"})),
        )
        .expect(1)
        .mount(&f.server)
        .await;
    let admission = f.admitted_creation(&target, key, body).await;
    let before = f.creation_for_key(key).await;
    f.forward_original(
        &target,
        &admission,
        f.request("/api/v1/sandboxes", key, body),
    )
    .await
    .unwrap();
    let sent = f.server.received_requests().await.unwrap();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0].body, body);
    assert_eq!(sent[0].headers["idempotency-key"], key);
    assert_eq!(
        sent[0].url.query(),
        Some("x-cma-secret=connection-b-secret")
    );
    let after = f.creation_for_key(key).await;
    assert_eq!(after.request_hash, before.request_hash);
    assert!(after.first_dispatched_at.is_some());
    assert!(
        f.catalog_server
            .received_requests()
            .await
            .unwrap()
            .is_empty()
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn gateway_path_auth_uses_the_generic_path_builder_without_relaxing_the_destination() {
    use crate::services::assistant_cma_gateway::CmaGateway;
    let f = Fixture::new().await;
    f.set_auth("path", "connection-b-secret").await;
    Mock::given(method("GET"))
        .and(path(
            "/x-cma-secretconnection-b-secret/api/v1/agents/agent_1",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id":"agent_1","owner":&f.owner,"sandbox_id":"sandbox_1","status":"ready"
        })))
        .expect(1)
        .mount(&f.server)
        .await;
    f.gateway().get_agent(&f.owner, "agent_1").await.unwrap();
    assert_eq!(f.server.received_requests().await.unwrap().len(), 1);
    assert!(
        f.catalog_server
            .received_requests()
            .await
            .unwrap()
            .is_empty()
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn common_dispatch_rejects_replaced_original_request_before_authentication() {
    let f = Fixture::new().await;
    f.set_auth("body", "connection-b-secret").await;
    let target = f.resolved_target().await;
    let body = br#"{"machine_included":false}"#;
    let key = "unchanged-original";
    let admission = f.admitted_creation(&target, key, body).await;
    for change in ["method", "path", "query", "body", "key"] {
        let mut request = f.request("/api/v1/sandboxes", key, body);
        match change {
            "method" => *request.method_mut() = reqwest::Method::DELETE,
            "path" => request.url_mut().set_path("/api/v1/agents"),
            "query" => request.url_mut().set_query(Some("changed=1")),
            "body" => *request.body_mut() = Some(reqwest::Body::from("{}")),
            _ => {
                request
                    .headers_mut()
                    .insert("idempotency-key", "replacement-key".parse().unwrap());
            }
        }
        assert!(
            matches!(f.forward_original(&target, &admission, request).await,
            Err(crate::services::proxy_service::ForwardRequestError::PreDispatchRejected(AppError::Conflict(ref message)))
                if message == "cma_creation_authority_changed"),
            "{change}"
        );
        let row = f.creation_for_key(key).await;
        assert_eq!(row.submission_state, "pending", "{change}");
        assert!(row.first_dispatched_at.is_none(), "{change}");
    }
    assert!(f.server.received_requests().await.unwrap().is_empty());
    assert!(
        f.catalog_server
            .received_requests()
            .await
            .unwrap()
            .is_empty()
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn authentication_failure_keeps_creation_pending_before_dispatch() {
    let f = Fixture::new().await;
    f.set_auth("body", "connection-b-secret").await;
    f.db.collection::<bson::Document>("user_services")
        .update_one(
            doc! {"_id":&f.user_service_id},
            doc! {"$set":{"auth_key_name":""}},
        )
        .await
        .unwrap();
    let target = f.resolved_target().await;
    let body = br#"{"machine_included":false}"#;
    let key = "invalid-auth-construction";
    let admission = f.admitted_creation(&target, key, body).await;
    assert!(
        matches!(f.forward_original(&target, &admission, f.request("/api/v1/sandboxes", key, body)).await,
        Err(crate::services::proxy_service::ForwardRequestError::Application(AppError::Internal(ref message)))
            if message == "Body auth method requires a non-empty auth_key_name")
    );
    let row = f.creation_for_key(key).await;
    assert_eq!(row.submission_state, "pending");
    assert!(row.first_dispatched_at.is_none());
    let parent = sandboxes::get_owned(&f.db, &f.owner, &row.sandbox_ref_id)
        .await
        .unwrap();
    assert_eq!(parent.submission_state, "pending");
    assert_eq!(parent.admission_state, SandboxAdmissionState::Reserved);
    assert!(f.server.received_requests().await.unwrap().is_empty());
    assert!(
        f.catalog_server
            .received_requests()
            .await
            .unwrap()
            .is_empty()
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn final_default_header_cannot_replace_the_creation_idempotency_key() {
    let f = Fixture::new().await;
    f.set_auth("body", "connection-b-secret").await;
    let mut target = f.resolved_target().await;
    target.user_service_default_headers.push(
        crate::models::default_request_header::ResolvedDefaultRequestHeader {
            name: "Idempotency-Key".into(),
            value: "overridden-key".to_owned().into(),
            overridable: false,
            sensitive: false,
        },
    );
    let body = br#"{"machine_included":false}"#;
    let key = "original-key-before-defaults";
    let admission = f.admitted_creation(&target, key, body).await;
    assert!(matches!(
        f.forward_original(
            &target,
            &admission,
            f.request("/api/v1/sandboxes", key, body)
        )
        .await,
        Err(
            crate::services::proxy_service::ForwardRequestError::PreDispatchRejected(
                AppError::Conflict(_)
            )
        )
    ));
    let row = f.creation_for_key(key).await;
    assert_eq!(row.submission_state, "pending");
    assert!(row.first_dispatched_at.is_none());
    assert!(f.server.received_requests().await.unwrap().is_empty());
    assert!(
        f.catalog_server
            .received_requests()
            .await
            .unwrap()
            .is_empty()
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn original_parent_generation_change_is_rejected_at_final_dispatch() {
    let f = Fixture::new().await;
    f.set_auth("body", "connection-b-secret").await;
    let target = f.resolved_target().await;
    let body = br#"{"machine_included":false}"#;
    let key = "generation-changed-after-admission";
    let admission = f.admitted_creation(&target, key, body).await;
    let row = f.creation_for_key(key).await;
    f.db.collection::<bson::Document>(crate::models::assistant_cma_sandbox::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&row.sandbox_ref_id},
            doc! {"$inc":{"generation":1_i64}},
        )
        .await
        .unwrap();
    assert!(matches!(
        f.forward_original(
            &target,
            &admission,
            f.request("/api/v1/sandboxes", key, body)
        )
        .await,
        Err(crate::services::proxy_service::ForwardRequestError::PreDispatchRejected(_))
    ));
    let after = f.creation_for_key(key).await;
    assert_eq!(after.submission_state, "pending");
    assert!(after.first_dispatched_at.is_none());
    assert!(f.server.received_requests().await.unwrap().is_empty());
    assert!(
        f.catalog_server
            .received_requests()
            .await
            .unwrap()
            .is_empty()
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn recovery_keeps_original_body_and_key_with_generic_body_and_query_authentication() {
    for auth in ["body", "query"] {
        let f = Fixture::new().await;
        f.set_auth(auth, "connection-b-secret").await;
        f.observations().await;
        let target = f.resolved_target().await;
        let body = br#"{ "name": "recover authenticated", "machine_included": false }"#;
        let key = "authenticated-recovery";
        let permit = f
            .admit(&target, "/api/v1/sandboxes", key, body)
            .await
            .unwrap();
        let row = permit.creation.as_ref().unwrap();
        // Model a crash after the final transaction fence but before the socket send.
        permit
            .before_send(&f.request("/api/v1/sandboxes", key, body))
            .await
            .unwrap();
        Mock::given(method("POST"))
            .and(path("/api/v1/sandboxes"))
            .respond_with(
                ResponseTemplate::new(201).set_body_json(serde_json::json!({"id":"sandbox_1"})),
            )
            .expect(1)
            .mount(&f.server)
            .await;
        let result = recover_creation(
            &f.db,
            &f.keys,
            &f.gateway(),
            &f.owner,
            &row.sandbox_ref_id,
            row.sandbox_generation,
        )
        .await
        .unwrap();
        assert!(!result.pending);
        let after = f.saved(&row.id).await;
        assert_eq!(after.request_hash, row.request_hash);
        assert_eq!(after.submission_state, "confirmed");
        assert!(after.request_ciphertext.is_none());
        let requests = f.server.received_requests().await.unwrap();
        let posts = requests
            .iter()
            .filter(|request| request.method == "POST")
            .collect::<Vec<_>>();
        assert_eq!(posts.len(), 1);
        assert_eq!(posts[0].headers["idempotency-key"], key);
        if auth == "body" {
            let sent: Value = serde_json::from_slice(&posts[0].body).unwrap();
            assert_eq!(sent["name"], "recover authenticated");
            assert_eq!(sent["machine_included"], false);
            assert_eq!(sent["x-cma-secret"], "connection-b-secret");
        } else {
            assert_eq!(posts[0].body, body);
            assert!(
                posts[0]
                    .url
                    .query_pairs()
                    .any(|(name, value)| name == "x-cma-secret" && value == "connection-b-secret")
            );
        }
        assert!(
            f.catalog_server
                .received_requests()
                .await
                .unwrap()
                .is_empty()
        );
        f.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn gateway_header_bearer_and_basic_auth_use_connection_b() {
    use crate::services::assistant_cma_gateway::CmaGateway;
    for (method, credential, header, expected) in [
        (
            "header",
            "connection-b-secret",
            "x-cma-secret",
            "connection-b-secret",
        ),
        (
            "bearer",
            "connection-b-secret",
            "authorization",
            "Bearer connection-b-secret",
        ),
        (
            "basic",
            "test:password",
            "authorization",
            "Basic dGVzdDpwYXNzd29yZA==",
        ),
    ] {
        let f = Fixture::new().await;
        f.observations().await;
        f.set_auth(method, credential).await;
        f.gateway().get_agent(&f.owner, "agent_1").await.unwrap();
        let requests = f.server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 1, "{method}");
        assert_eq!(requests[0].headers[header], expected, "{method}");
        assert!(
            f.catalog_server
                .received_requests()
                .await
                .unwrap()
                .is_empty()
        );
        f.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn revoked_or_missing_original_connection_does_not_fall_back_to_catalog() {
    use crate::services::assistant_cma_gateway::CmaGateway;
    let f = Fixture::new().await;
    let gateway = f.gateway();
    for (field, value) in [
        ("is_active", bson::Bson::Boolean(false)),
        ("deleted_at", bson::Bson::DateTime(bson::DateTime::now())),
        ("user_id", bson::Bson::String("another-owner".into())),
    ] {
        f.db.collection::<bson::Document>("user_services")
            .update_one(doc! {"_id":&f.user_service_id}, doc! {"$set":{field:value}})
            .await
            .unwrap();
        assert!(
            gateway.get_agent(&f.owner, "agent_1").await.is_err(),
            "{field}"
        );
        f.db.collection::<bson::Document>("user_services")
            .update_one(
                doc! {"_id":&f.user_service_id},
                doc! {"$set":{"is_active":true,"deleted_at":null,"user_id":&f.owner}},
            )
            .await
            .unwrap();
    }
    f.db.collection::<bson::Document>("user_services")
        .delete_one(doc! {"_id":&f.user_service_id})
        .await
        .unwrap();
    assert!(gateway.get_agent(&f.owner, "agent_1").await.is_err());
    assert!(f.server.received_requests().await.unwrap().is_empty());
    assert!(
        f.catalog_server
            .received_requests()
            .await
            .unwrap()
            .is_empty()
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn desktop_rejects_json_frame_authentication_before_opening_an_rfb_socket() {
    let f = Fixture::new().await;
    f.set_auth("bearer", "connection-b-secret").await;
    f.db.collection::<bson::Document>("downstream_services")
        .update_one(
            doc! {"_id":&f.catalog.id},
            doc! {"$set":{"ws_frame_injections":[{
                "trigger":"first_frame_from_downstream", "template":"${credential}"
            }]}},
        )
        .await
        .unwrap();
    assert!(matches!(
        f.gateway().desktop_context(&f.owner, "agent_1").await,
        Err(crate::services::assistant_cma_gateway::CmaGatewayError::InvalidRequest)
    ));
    assert!(f.server.received_requests().await.unwrap().is_empty());
    assert!(
        f.catalog_server
            .received_requests()
            .await
            .unwrap()
            .is_empty()
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn desktop_websocket_authentication_and_rfb_frames_use_connection_b() {
    use axum::{
        Router,
        extract::ws::{Message, WebSocketUpgrade},
        http::HeaderMap,
        routing::get,
    };
    use futures::StreamExt;
    let f = Fixture::new().await;
    f.set_auth("bearer", "desktop-b-secret").await;
    let (sent, mut received) = tokio::sync::mpsc::unbounded_channel();
    let app = Router::new().route(
        "/api/v1/agents/agent_1/desktop/ws",
        get(move |headers: HeaderMap, ws: WebSocketUpgrade| {
            let sent = sent.clone();
            async move {
                sent.send(headers).unwrap();
                ws.on_upgrade(|mut socket| async move {
                    socket
                        .send(Message::Binary(bytes::Bytes::from_static(b"RFB 003.008\n")))
                        .await
                        .unwrap();
                    let _ = socket.recv().await;
                })
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    f.db.collection::<bson::Document>("user_endpoints")
        .update_one(doc! {"_id":&f.endpoint_id}, doc! {"$set":{"url":&base}})
        .await
        .unwrap();
    let gateway = CatalogCmaGateway::from_user_service_parts(
        &f.db,
        &f.keys,
        &f.jwt,
        &f.config,
        &f.user_service_id,
        Some(&f.catalog.id),
        Some(&base),
    );
    let mut socket =
        crate::handlers::assistant_cma_agents::open_desktop(&gateway, &f.owner, "agent_1")
            .await
            .unwrap();
    let headers = received.recv().await.unwrap();
    assert_eq!(headers["authorization"], "Bearer desktop-b-secret");
    assert!(!headers.contains_key("x-nyxid-identity-token"));
    let frame = socket.next().await.unwrap().unwrap();
    assert_eq!(
        frame.into_data(),
        bytes::Bytes::from_static(b"RFB 003.008\n")
    );
    socket.close(None).await.unwrap();
    assert!(
        f.catalog_server
            .received_requests()
            .await
            .unwrap()
            .is_empty()
    );
    server.abort();
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn rest_and_mcp_execution_resolve_connection_b_with_different_catalog_identity() {
    use crate::services::billing::BillingIngress;
    use crate::services::billing::route_inventory::{self, BillingRoutePolicy};
    use crate::services::mcp_service::{self, McpExecContext, NodeScope, ServiceScope};
    let f = Fixture::new().await;
    let state = crate::test_utils::test_app_state(f.db.clone());
    let endpoint = crate::test_utils::test_service_endpoint(
        &f.catalog.id,
        "list_agents",
        "GET",
        "/api/v1/agents",
    );
    f.db.collection::<crate::models::service_endpoint::ServiceEndpoint>("service_endpoints")
        .insert_one(endpoint)
        .await
        .unwrap();
    Mock::given(method("GET"))
        .and(path("/api/v1/agents"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"items":[]})))
        .mount(&f.server)
        .await;
    let auth = crate::test_utils::test_auth_user(&f.owner);
    let request = axum::http::Request::builder()
        .method("GET")
        .uri("/api/v1/proxy/s/cma-connection-b/api/v1/agents")
        .body(axum::body::Body::empty())
        .unwrap();
    let response = crate::handlers::proxy::proxy_request_by_slug_inner(
        &state,
        &auth,
        "cma-connection-b",
        "/api/v1/agents",
        request,
        &mut String::new(),
    )
    .await
    .unwrap();
    assert_eq!(response.status(), 200);
    let catalog = mcp_service::load_operation_catalog(
        &f.db,
        &state.node_ws_manager,
        &f.owner,
        NodeScope::Unrestricted,
        ServiceScope::Unrestricted,
    )
    .await
    .unwrap();
    let service = catalog
        .services
        .iter()
        .find(|service| service.service_id == f.user_service_id)
        .unwrap();
    let endpoint = service
        .endpoints
        .iter()
        .find(|endpoint| endpoint.path == "/api/v1/agents")
        .unwrap();
    let prepared =
        mcp_service::prepare_proxy_tool_call(service, endpoint, &serde_json::json!({})).unwrap();
    let permit = route_inventory::enforce_billing_egress_classification(
        Some(BillingRoutePolicy::Metered(BillingIngress::Mcp)),
        BillingIngress::Mcp,
    )
    .unwrap();
    let (status, _) = mcp_service::execute_tool(
        &state.http_client,
        &f.db,
        &state.encryption_keys,
        &state.node_ws_manager,
        &state.billing,
        &f.owner,
        &f.owner,
        service,
        endpoint,
        prepared,
        &state.jwt_keys,
        &state.config,
        &state.connection_expiry_notifier,
        &state.token_exchange_cache,
        &state.cloud_response_cache,
        &McpExecContext {
            strict_response_utf8: false,
            execution_permission: None,
            assistant_chat: None,
            response_body_limit: None,
            actor_user_id: None,
            caller_token: None,
            delegation_restrictions: Default::default(),
            attribution: None,
            org_agent_access: None,
            agent_owner: None,
            operation_scopes: None,
            api_key_id: None,
            api_key_credential_id: None,
            allow_all_nodes: true,
            allowed_node_ids: &[],
        },
        permit,
    )
    .await
    .unwrap();
    assert_eq!(status, 200);
    let requests = f.server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 2);
    for request in requests {
        assert!(request.headers.contains_key("x-nyxid-identity-token"));
    }
    assert!(
        f.catalog_server
            .received_requests()
            .await
            .unwrap()
            .is_empty()
    );
    f.db.drop().await.unwrap();
}

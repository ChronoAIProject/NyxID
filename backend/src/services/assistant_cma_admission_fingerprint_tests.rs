use crate::{
    models::{downstream_service::DownstreamService, user::UserType},
    services::{
        assistant_cma_gateway::catalog_execution_fingerprint,
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
    owner: String,
    catalog: DownstreamService,
    target: ProxyTarget,
    keys: EncryptionKeys,
    jwt: JwtKeys,
    config: AppConfig,
}

impl Fixture {
    async fn new() -> Self {
        let db = crate::test_utils::connect_transaction_test_database("cma_fingerprint").await;
        let server = MockServer::start().await;
        let owner = uuid::Uuid::new_v4().to_string();
        db.collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
            .insert_one(crate::test_utils::test_user(&owner, UserType::Person))
            .await
            .unwrap();
        let mut catalog = crate::test_utils::test_auto_connected_catalog_service();
        catalog.slug = "cma".into();
        catalog.base_url = server.uri();
        catalog.identity_propagation_mode = "jwt".into();
        catalog.identity_include_user_id = false;
        catalog.identity_include_email = false;
        catalog.identity_include_name = false;
        db.collection::<DownstreamService>(crate::models::downstream_service::COLLECTION_NAME)
            .insert_one(&catalog)
            .await
            .unwrap();
        let mut connection = catalog.clone();
        connection.identity_include_user_id = true;
        connection.identity_include_email = true;
        connection.identity_include_name = true;
        let target = Self::connection_target(connection);
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
            owner,
            catalog,
            target,
            keys: crate::test_utils::test_encryption_keys(),
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

    fn gateway(&self) -> CatalogCmaGateway {
        CatalogCmaGateway::from_parts(&self.db, &self.jwt, &self.config, &self.catalog.id)
    }

    async fn admit(
        &self,
        target: &ProxyTarget,
        route: &str,
        key: &str,
        body: &[u8],
    ) -> AppResult<CmaDispatchPermit> {
        admit(
            &self.db,
            &self.keys,
            &self.jwt,
            &self.config,
            &self.owner,
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
async fn rest_and_mcp_common_get_dispatch_accepts_connection_identity_differences() {
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
        user_service_id: Some("connection-alias"),
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
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn differing_connection_creation_uses_catalog_fingerprint_and_confirms_once() {
    let f = Fixture::new().await;
    f.observations().await;
    let body = br#"{ "name": "original agent", "machine_included": false }"#;
    let key = "fingerprint-lifecycle-key";
    let first = f
        .admit(&f.target, "/api/v1/agents", key, body)
        .await
        .unwrap();
    let row = first.creation.as_ref().unwrap();
    assert_eq!(
        row.deployment_fingerprint,
        catalog_execution_fingerprint(&f.catalog)
    );
    assert_ne!(
        row.deployment_fingerprint,
        catalog_execution_fingerprint(&f.target.service)
    );
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
    let receipt = match f.admit(&f.target, "/api/v1/agents", key, body).await {
        Err(error) => error.response_body(),
        Ok(_) => panic!("confirmed creation must not be dispatched again"),
    };
    assert_eq!(receipt.error_code, 1004);
    let details = receipt.details.unwrap();
    assert_eq!(details["reason"], "cma_creation_already_completed");
    assert_eq!(details["operation_id"], "op_1");
    assert_eq!(details["agent_id"], "agent_1");
    let changed = match f
        .admit(&f.target, "/api/v1/agents", key, br#"{"name":"changed"}"#)
        .await
    {
        Err(error) => error.response_body(),
        Ok(_) => panic!("changed creation must be refused"),
    };
    assert!(changed.details.is_none());
    assert_eq!(f.post_count().await, 1);
    let upload_path = "/api/v1/agents/agent_1/attachments";
    let bytes = b"\0binary model";
    let upload = admit(
        &f.db,
        &f.keys,
        &f.jwt,
        &f.config,
        &f.owner,
        true,
        &f.target,
        "POST",
        upload_path,
        Some("name=room.glb"),
        bytes,
        &[("Idempotency-Key".into(), "upload-original-key".into())],
        false,
    )
    .await
    .unwrap();
    let request = reqwest::Client::new()
        .post(format!("{}{upload_path}?name=room.glb", f.server.uri()))
        .header("Idempotency-Key", "upload-original-key")
        .body(bytes.to_vec())
        .build()
        .unwrap();
    upload.before_send(&request).await.unwrap();
    let changed = reqwest::Client::new()
        .post(format!("{}{upload_path}?name=other.glb", f.server.uri()))
        .header("Idempotency-Key", "upload-original-key")
        .body(bytes.to_vec())
        .build()
        .unwrap();
    assert!(upload.before_send(&changed).await.is_err());
    let changed = reqwest::Client::new()
        .post(format!("{}{upload_path}?name=room.glb", f.server.uri()))
        .header("Idempotency-Key", "upload-original-key")
        .body(b"changed bytes".to_vec())
        .build()
        .unwrap();
    assert!(upload.before_send(&changed).await.is_err());
    f.assert_single_intent().await;
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn lost_receipt_recovery_replays_original_bytes_and_key_with_catalog_fingerprint() {
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
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn valid_old_v1_creation_retries_and_observes_without_another_post() {
    let f = Fixture::new().await;
    f.observations().await;
    let body = br#"{"machine_included":false}"#;
    let key = "old-v1-fingerprint-key";
    // Old admission only persisted rows after both fingerprints matched.
    let old_target = Fixture::connection_target(f.catalog.clone());
    let old = f
        .admit(&old_target, "/api/v1/agents", key, body)
        .await
        .unwrap();
    let row = old.creation.as_ref().unwrap();
    assert_eq!(
        row.deployment_fingerprint,
        catalog_execution_fingerprint(&f.catalog)
    );
    let retry = f
        .admit(&f.target, "/api/v1/agents", key, body)
        .await
        .unwrap();
    assert_eq!(retry.creation.as_ref().unwrap().id, row.id);
    change_creation(&f.db, row, CreationChange::Dispatch)
        .await
        .unwrap();
    change_creation(
        &f.db,
        row,
        CreationChange::Receipt(CreationReceipt::Lifecycle("op_1".into())),
    )
    .await
    .unwrap();
    let recovered = recover_creation(
        &f.db,
        &f.keys,
        &f.gateway(),
        &f.owner,
        &row.sandbox_ref_id,
        row.sandbox_generation,
    )
    .await
    .unwrap();
    assert!(!recovered.pending);
    assert_eq!(f.saved(&row.id).await.submission_state, "confirmed");
    assert_eq!(f.post_count().await, 0);
    f.assert_single_intent().await;
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn catalog_drift_target_mismatch_and_expired_recovery_remain_closed() {
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
    for (field, value) in [
        (
            "base_url",
            bson::Bson::String(format!("{}/changed", f.server.uri())),
        ),
        (
            "identity_jwt_audience",
            bson::Bson::String("changed-audience".into()),
        ),
        ("identity_include_email", bson::Bson::Boolean(true)),
    ] {
        f.db.collection::<bson::Document>(crate::models::downstream_service::COLLECTION_NAME)
            .update_one(doc! {"_id":&f.catalog.id}, doc! {"$set":{field:value}})
            .await
            .unwrap();
        assert!(
            permit
                .before_send(&f.request("/api/v1/sandboxes", key, body))
                .await
                .is_err()
        );
        assert!(
            f.admit(&f.target, "/api/v1/sandboxes", key, body)
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
            f.saved(&row.id).await.deployment_fingerprint,
            row.deployment_fingerprint
        );
        f.db.collection::<DownstreamService>(crate::models::downstream_service::COLLECTION_NAME)
            .replace_one(doc! {"_id":&f.catalog.id}, &f.catalog)
            .await
            .unwrap();
    }
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
        assert!(permit.before_send(&request).await.is_err());
    }
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
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn resolved_cma_destination_does_not_revert_to_stale_connection_snapshot() {
    let mut f = Fixture::new().await;
    f.target.service.base_url = "https://stale-connection.invalid".into();
    let permit = admit(
        &f.db,
        &f.keys,
        &f.jwt,
        &f.config,
        &f.owner,
        true,
        &f.target,
        "GET",
        "/api/v1/agents",
        None,
        b"",
        &[],
        false,
    )
    .await
    .unwrap();
    let request = reqwest::Client::new()
        .get(format!("{}/api/v1/agents", f.server.uri()))
        .build()
        .unwrap();
    permit.before_send(&request).await.unwrap();
    let wrong = reqwest::Client::new()
        .get("https://stale-connection.invalid/api/v1/agents")
        .build()
        .unwrap();
    assert!(permit.before_send(&wrong).await.is_err());
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn org_connection_cma_admission_keeps_actor_and_connection_binding() {
    let f = Fixture::new().await;
    let org = uuid::Uuid::new_v4().to_string();
    let caller = registry::DispatchCaller {
        actor_user_id: &f.owner,
        resource_owner_id: &org,
        user_service_id: Some("org-cma"),
    };
    let admission = registry::admit(
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
        .get(format!("{}/api/v1/agents", f.server.uri()))
        .build()
        .unwrap();
    guard.before_send(&request).await.unwrap();
    let changed = registry::DispatchCaller {
        actor_user_id: &f.owner,
        resource_owner_id: &org,
        user_service_id: Some("another-connection"),
    };
    assert!(registry::verify(Some(&admission), &f.target, Some(&changed)).is_err());
    assert!(
        registry::admit(
            &f.db,
            &f.keys,
            &f.jwt,
            &f.config,
            &caller,
            false,
            &f.target,
            "GET",
            "/api/v1/agents",
            None,
            b"",
            || Ok(vec![].into()),
            false
        )
        .await
        .is_err()
    );
    f.db.drop().await.unwrap();
}

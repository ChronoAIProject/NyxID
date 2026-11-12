const SANDBOX_ROUTE: &str = "/api/v1/sandboxes";
const UNCONFIRMED: &[u8] =
    br#"{ "sandbox_profile": {"id":"profile_1","revision":1}, "name":"trust test" }"#;

fn trust_problem() -> Value {
    serde_json::json!({
        "status":409,
        "type":"https://spec.chrono-ai.fun/cma/1.0/problems/trust-confirmation-required",
        "code":"trust_confirmation_required",
        "detail":"Confirm the exact commands digest before creating this resource.",
        "commands_digest":"a".repeat(64),
    })
}

fn confirmed_body() -> Vec<u8> {
    let mut body: Value = serde_json::from_slice(UNCONFIRMED).unwrap();
    body["trust_confirmed"] = Value::Bool(true);
    body["commands_digest"] = Value::String("a".repeat(64));
    serde_json::to_vec(&body).unwrap()
}

// Faithful to CMA 68f26f17 rest/idempotency.rs: reserve raw-body fingerprint,
// cache errors as well as successes, reject changed bytes under the same key.
type CachedTrustResponses = std::collections::HashMap<String, (String, u16, Value)>;

#[derive(Clone, Default)]
struct TrustCma {
    responses: std::sync::Arc<std::sync::Mutex<CachedTrustResponses>>,
    creates: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

impl wiremock::Respond for TrustCma {
    fn respond(&self, request: &wiremock::Request) -> ResponseTemplate {
        assert_eq!(request.headers["content-type"], "application/json");
        let key = request.headers["idempotency-key"].to_str().unwrap();
        let hash = wire_hash(&request.body);
        let mut responses = self.responses.lock().unwrap();
        if let Some((previous, status, body)) = responses.get(key) {
            if previous == &hash {
                return ResponseTemplate::new(*status).set_body_json(body);
            }
            return ResponseTemplate::new(422).set_body_json(serde_json::json!({
                "status":422,"code":"idempotency_key_reused",
                "detail":"This Idempotency-Key already names a different request."
            }));
        }
        let body: Value = serde_json::from_slice(&request.body).unwrap();
        let (status, response) =
            if body["trust_confirmed"] == true && body["commands_digest"] == "a".repeat(64) {
                self.creates
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                (201, serde_json::json!({"id":"sandbox_1"}))
            } else {
                (409, trust_problem())
            };
        responses.insert(key.into(), (hash, status, response.clone()));
        ResponseTemplate::new(status).set_body_json(response)
    }
}

async fn mock_cma(f: &Fixture) -> TrustCma {
    let cma = TrustCma::default();
    Mock::given(method("POST"))
        .and(path(SANDBOX_ROUTE))
        .respond_with(cma.clone())
        .mount(&f.server)
        .await;
    f.observations().await;
    cma
}

async fn proxy_create(
    f: &Fixture,
    key: &str,
    body: &[u8],
    contextual: bool,
) -> AppResult<(u16, Value)> {
    let mut state = crate::test_utils::test_app_state(f.db.clone());
    let mut keys = crate::test_utils::test_encryption_keys();
    keys.set_contextual_record_writes(contextual);
    state.encryption_keys = std::sync::Arc::new(keys);
    let mut auth = crate::test_utils::test_auth_user(&f.owner);
    auth.auth_method = crate::mw::auth::AuthMethod::AccessToken;
    auth.scope = "proxy".into();
    let request = axum::http::Request::builder()
        .method("POST")
        .uri(format!("/api/v1/proxy/s/cma{SANDBOX_ROUTE}"))
        .header("Content-Type", "application/json")
        .header("Idempotency-Key", key)
        .extension(
            crate::services::billing::route_inventory::BillingRoutePolicy::Metered(
                crate::services::billing::BillingIngress::Proxy,
            ),
        )
        .body(axum::body::Body::from(body.to_vec()))
        .unwrap();
    let mut operation = String::new();
    let proxy = Box::pin(crate::handlers::proxy::proxy_request_inner(
        &state,
        &auth,
        &f.catalog.id,
        SANDBOX_ROUTE,
        request,
        &mut operation,
    ));
    // Personal proxy writes require approval by default. Exercise that real
    // decision path instead of waiting for an absent human or bypassing it.
    let approve = async {
        use crate::models::approval_request::{ApprovalRequest, COLLECTION_NAME as REQUESTS};
        loop {
            let requests = f.db.collection::<ApprovalRequest>(REQUESTS);
            if let Some(pending) = requests
                .find_one(doc! {"user_id":&f.owner,"service_id":&f.catalog.id,"status":"pending"})
                .await
                .unwrap()
            {
                assert_eq!(pending.user_id, f.owner);
                assert!(pending.execution_proposal.is_none());
                if crate::services::approval_service::process_decision(
                    &f.db,
                    &state.config,
                    &state.http_client,
                    None,
                    None,
                    &pending.id,
                    true,
                    None,
                    None,
                    "web",
                )
                .await
                .is_err()
                {
                    // Concurrent test callers may both observe the same card.
                    let decided = requests
                        .find_one(doc! {"_id":&pending.id})
                        .await
                        .unwrap()
                        .unwrap();
                    assert_eq!(decided.status, "approved");
                }
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    };
    let response = tokio::time::timeout(Duration::from_secs(30), async {
        tokio::select! {
            result = proxy => result,
            () = approve => unreachable!("approval driver runs until the proxy completes"),
        }
    })
    .await
    .expect("proxy must complete after the owner decision")?;
    let status = response.status().as_u16();
    let bytes = axum::body::to_bytes(response.into_body(), 4096)
        .await
        .unwrap();
    Ok((status, serde_json::from_slice(&bytes).unwrap()))
}

async fn assert_absent(f: &Fixture, row: &AssistantCmaCreation) {
    let current = f.saved(&row.id).await;
    assert_eq!(current.submission_state, "confirmed_not_created");
    assert!(current.request_ciphertext.is_none() && current.request_context_version.is_none());
    assert!(current.operation_id.is_none() && current.cma_sandbox_id.is_none());
    let parent = sandboxes::get_owned(&f.db, &f.owner, &row.sandbox_ref_id)
        .await
        .unwrap();
    assert!(!parent.occupies_account_slot);
    assert_eq!(parent.submission_state, "confirmed_not_created");
    assert!(parent.request_ciphertext.is_none() && parent.request_context_version.is_none());
}

async fn unknown_refusal(f: &Fixture, contextual: bool) -> AssistantCmaCreation {
    let permit = f
        .admit(&f.target, SANDBOX_ROUTE, "original-key", UNCONFIRMED)
        .await
        .unwrap();
    let mut request = f.request(SANDBOX_ROUTE, "original-key", UNCONFIRMED);
    request.headers_mut().insert(
        reqwest::header::CONTENT_TYPE,
        reqwest::header::HeaderValue::from_static("application/json"),
    );
    permit.before_send(&request).await.unwrap();
    let response = reqwest::Client::new().execute(request).await.unwrap();
    assert_eq!(response.status(), 409);
    assert_eq!(response.json::<Value>().await.unwrap(), trust_problem());
    // Simulates the old observer (or loss of the response before observing it).
    let row = f.saved(&permit.creation.as_ref().unwrap().id).await;
    assert_eq!(row.submission_state, "unknown");
    assert_eq!(row.request_context_version, contextual.then_some(3));
    assert!(row.request_ciphertext.is_some());
    row
}

#[test]
fn trust_absence_requires_exact_sandbox_problem_and_no_resource_identifiers() {
    let problem = trust_problem();
    assert!(creation_refusal_confirms_no_resource(
        SANDBOX_ROUTE,
        409,
        &problem
    ));
    for route in [
        "/api/v1/agents",
        "/api/v1/workspaces/w/agents",
        "/api/v1/sandboxes/s/workspaces",
        "/api/v1/sandboxes/s/workspaces/inline",
    ] {
        assert!(!creation_refusal_confirms_no_resource(route, 409, &problem));
    }
    for status in [200, 201, 400, 403, 422, 500] {
        assert!(!creation_refusal_confirms_no_resource(
            SANDBOX_ROUTE,
            status,
            &problem
        ));
    }
    for field in ["id", "operation_id", "sandbox_id", "agent_id"] {
        let mut altered = problem.clone();
        altered[field] = Value::Null;
        assert!(creation_refusal_confirms_no_resource(
            SANDBOX_ROUTE,
            409,
            &altered
        ));
        for value in [
            serde_json::json!("resource_1"),
            serde_json::json!(""),
            serde_json::json!(false),
            serde_json::json!({}),
        ] {
            altered[field] = value;
            assert!(!creation_refusal_confirms_no_resource(
                SANDBOX_ROUTE,
                409,
                &altered
            ));
        }
    }
    for field in ["status", "type", "code", "commands_digest"] {
        let mut altered = problem.clone();
        altered.as_object_mut().unwrap().remove(field);
        assert!(!creation_refusal_confirms_no_resource(
            SANDBOX_ROUTE,
            409,
            &altered
        ));
    }
    for (field, value) in [
        ("status", serde_json::json!(422)),
        ("status", serde_json::json!("409")),
        (
            "type",
            serde_json::json!("https://other.test/trust-confirmation-required"),
        ),
        ("code", serde_json::json!("permission_denied")),
        ("commands_digest", serde_json::json!("A".repeat(64))),
        ("commands_digest", serde_json::json!("a".repeat(63))),
    ] {
        let mut altered = problem.clone();
        altered[field] = value;
        assert!(!creation_refusal_confirms_no_resource(
            SANDBOX_ROUTE,
            409,
            &altered
        ));
    }
    let mut alias = problem;
    alias.as_object_mut().unwrap().remove("code");
    alias["error"] = serde_json::json!("trust_confirmation_required");
    assert!(!creation_refusal_confirms_no_resource(
        SANDBOX_ROUTE,
        409,
        &alias
    ));
}

#[tokio::test]
async fn trust_mock_same_key_changed_body_is_rejected_by_upstream_contract() {
    let server = MockServer::start().await;
    let cma = TrustCma::default();
    Mock::given(method("POST"))
        .respond_with(cma.clone())
        .mount(&server)
        .await;
    let client = reqwest::Client::new();
    for _ in 0..2 {
        let response = client
            .post(server.uri())
            .header("Content-Type", "application/json")
            .header("Idempotency-Key", "K1")
            .body(UNCONFIRMED)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), 409);
        assert_eq!(response.json::<Value>().await.unwrap(), trust_problem());
    }
    let response = client
        .post(server.uri())
        .header("Content-Type", "application/json")
        .header("Idempotency-Key", "K1")
        .body(confirmed_body())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 422);
    assert_eq!(
        response.json::<Value>().await.unwrap()["code"],
        "idempotency_key_reused"
    );
    let response = client
        .post(server.uri())
        .header("Content-Type", "application/json")
        .header("Idempotency-Key", "K2")
        .body(confirmed_body())
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 201);
    assert_eq!(cma.creates.load(std::sync::atomic::Ordering::SeqCst), 1);
}

#[tokio::test]
async fn trust_proxy_three_step_retry_preserves_upstream_key_and_frees_slot_both_formats() {
    for contextual in [false, true] {
        let mut f = Fixture::new().await;
        f.keys.set_contextual_record_writes(contextual);
        let cma = mock_cma(&f).await;
        assert_eq!(
            proxy_create(&f, "original-key", UNCONFIRMED, contextual)
                .await
                .unwrap(),
            (409, trust_problem())
        );
        let row =
            f.db.collection::<AssistantCmaCreation>(COLLECTION_NAME)
                .find_one(doc! {"user_id":&f.owner,"idempotency_key":"original-key"})
                .await
                .unwrap()
                .unwrap();
        assert_absent(&f, &row).await;
        // Exact K1 retries preserve bytes and receive CMA's cached problem.
        assert_eq!(
            proxy_create(&f, "original-key", UNCONFIRMED, contextual)
                .await
                .unwrap(),
            (409, trust_problem())
        );
        let retry = f.saved(&row.id).await;
        assert!(retry.submission_generation > row.submission_generation);
        assert_absent(&f, &retry).await;
        let error = proxy_create(&f, "original-key", &confirmed_body(), contextual)
            .await
            .unwrap_err();
        assert!(matches!(error, AppError::Conflict(ref message)
            if message.starts_with("cma_creation_authority_changed:")
            && message.contains("new Idempotency-Key")));
        assert_eq!(f.post_count().await, 2);
        assert_eq!(
            proxy_create(&f, "confirmed-key", &confirmed_body(), contextual)
                .await
                .unwrap()
                .0,
            201
        );
        assert_eq!(cma.creates.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!(f.post_count().await, 3);
        let created =
            f.db.collection::<AssistantCmaCreation>(COLLECTION_NAME)
                .find_one(doc! {"idempotency_key":"confirmed-key"})
                .await
                .unwrap()
                .unwrap();
        assert_eq!(created.submission_state, "confirmed");
        assert_eq!(created.cma_sandbox_id.as_deref(), Some("sandbox_1"));
        assert_ne!(created.id, row.id);
        assert!(created.sandbox_generation > retry.sandbox_generation);
        // An old in-flight reply cannot cross the original submission fence.
        assert!(
            change_creation(
                &f.db,
                &row,
                CreationChange::Receipt(CreationReceipt::Sandbox("late_sandbox".into()))
            )
            .await
            .is_err()
        );
        f.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn trust_stranded_slot_recovers_before_changed_same_key_or_new_key_both_formats() {
    for contextual in [false, true] {
        for same_key_first in [false, true] {
            let mut f = Fixture::new().await;
            f.keys.set_contextual_record_writes(contextual);
            let cma = mock_cma(&f).await;
            let row = unknown_refusal(&f, contextual).await;
            // Readers select the stored format even after writer cutover changes.
            f.keys.set_contextual_record_writes(!contextual);
            if same_key_first {
                let error = proxy_create(&f, "original-key", &confirmed_body(), !contextual)
                    .await
                    .unwrap_err();
                assert!(
                    matches!(error, AppError::Conflict(ref message) if message.contains("new Idempotency-Key"))
                );
                assert_absent(&f, &row).await;
                assert_eq!(f.post_count().await, 2);
            }
            assert_eq!(
                proxy_create(&f, "confirmed-key", &confirmed_body(), !contextual)
                    .await
                    .unwrap()
                    .0,
                201
            );
            assert_absent(&f, &row).await;
            assert_eq!(f.post_count().await, 3);
            assert_eq!(cma.creates.load(std::sync::atomic::Ordering::SeqCst), 1);
            let posts = f
                .server
                .received_requests()
                .await
                .unwrap()
                .into_iter()
                .filter(|r| r.method == "POST")
                .collect::<Vec<_>>();
            assert_eq!(posts[0].body, UNCONFIRMED);
            assert_eq!(posts[1].body, UNCONFIRMED);
            assert_eq!(posts[1].headers["idempotency-key"], "original-key");
            assert_eq!(posts[2].headers["idempotency-key"], "confirmed-key");
            assert_eq!(posts[2].body, confirmed_body());
            f.db.drop().await.unwrap();
        }
    }
}

#[tokio::test]
async fn trust_explicit_recovery_is_terminal_and_idempotent_both_formats() {
    for contextual in [false, true] {
        let mut f = Fixture::new().await;
        f.keys.set_contextual_record_writes(contextual);
        let cma = mock_cma(&f).await;
        let row = unknown_refusal(&f, contextual).await;
        for _ in 0..2 {
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
            assert_absent(&f, &row).await;
        }
        assert_eq!(f.post_count().await, 2);
        assert_eq!(cma.creates.load(std::sync::atomic::Ordering::SeqCst), 0);
        f.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn trust_spoof_with_any_resource_id_keeps_unknown_intent_and_slot() {
    for field in ["id", "operation_id", "sandbox_id", "agent_id"] {
        let f = Fixture::new().await;
        let mut spoof = trust_problem();
        spoof[field] = serde_json::json!("possibly_created");
        Mock::given(method("POST"))
            .and(path(SANDBOX_ROUTE))
            .respond_with(ResponseTemplate::new(409).set_body_json(&spoof))
            .mount(&f.server)
            .await;
        assert_eq!(
            proxy_create(&f, "original-key", UNCONFIRMED, false)
                .await
                .unwrap(),
            (409, spoof)
        );
        assert!(
            proxy_create(&f, "new-key", &confirmed_body(), false)
                .await
                .is_err()
        );
        let row =
            f.db.collection::<AssistantCmaCreation>(COLLECTION_NAME)
                .find_one(doc! {"idempotency_key":"original-key"})
                .await
                .unwrap()
                .unwrap();
        assert_eq!(row.submission_state, "unknown");
        assert!(row.request_ciphertext.is_some());
        f.assert_single_intent().await;
        assert_eq!(
            f.post_count().await,
            2,
            "only exact old request was replayed"
        );
        f.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn trust_automatic_recovery_refuses_expired_or_changed_deployment_before_replay() {
    for expired in [false, true] {
        let f = Fixture::new().await;
        mock_cma(&f).await;
        let row = unknown_refusal(&f, true).await;
        if expired {
            f.db.collection::<AssistantCmaCreation>(COLLECTION_NAME).update_one(
                doc! {"_id":&row.id}, doc! {"$set":{"first_dispatched_at":bson::DateTime::from_chrono(Utc::now()-chrono::Duration::hours(24))}}
            ).await.unwrap();
        } else {
            f.db.collection::<AssistantCmaCreation>(COLLECTION_NAME)
                .update_one(
                    doc! {"_id":&row.id},
                    doc! {"$set":{"deployment_fingerprint":"another-deployment"}},
                )
                .await
                .unwrap();
        }
        let error = proxy_create(&f, "new-key", &confirmed_body(), true)
            .await
            .unwrap_err();
        let expected = if expired {
            "cma_creation_replay_expired"
        } else {
            "cma_configuration_required"
        };
        assert!(matches!(error, AppError::Conflict(ref message) if message.starts_with(expected)));
        assert_eq!(f.post_count().await, 1);
        assert_eq!(f.saved(&row.id).await.submission_state, "unknown");
        assert!(f.saved(&row.id).await.request_ciphertext.is_some());
        f.assert_single_intent().await;
        f.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn trust_recovery_real_cached_creation_never_frees_slot_or_sends_new_request() {
    let f = Fixture::new().await;
    let cma = mock_cma(&f).await;
    let body = confirmed_body();
    let permit = f
        .admit(&f.target, SANDBOX_ROUTE, "original-key", &body)
        .await
        .unwrap();
    let mut request = f.request(SANDBOX_ROUTE, "original-key", &body);
    request.headers_mut().insert(
        reqwest::header::CONTENT_TYPE,
        reqwest::header::HeaderValue::from_static("application/json"),
    );
    permit.before_send(&request).await.unwrap();
    assert_eq!(
        reqwest::Client::new()
            .execute(request)
            .await
            .unwrap()
            .status(),
        201
    );
    let error = proxy_create(&f, "new-key", &body, true).await.unwrap_err();
    assert!(
        matches!(error, AppError::Conflict(ref message) if message.contains("cma_sandbox_account_slot_occupied"))
    );
    assert_eq!(
        f.saved(&permit.creation.as_ref().unwrap().id)
            .await
            .submission_state,
        "confirmed"
    );
    f.assert_single_intent().await;
    assert_eq!(f.post_count().await, 2);
    assert_eq!(cma.creates.load(std::sync::atomic::Ordering::SeqCst), 1);
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn trust_receipt_and_absence_race_commit_only_one_outcome() {
    for contextual in [false, true] {
        let mut f = Fixture::new().await;
        f.keys.set_contextual_record_writes(contextual);
        mock_cma(&f).await;
        let row = unknown_refusal(&f, contextual).await;
        let (receipt, absent) = tokio::join!(
            change_creation(
                &f.db,
                &row,
                CreationChange::Receipt(CreationReceipt::Sandbox("sandbox_1".into()))
            ),
            change_creation(&f.db, &row, CreationChange::NoResource),
        );
        assert_ne!(
            receipt.is_ok(),
            absent.is_ok(),
            "one transaction must fence the other"
        );
        let current = f.saved(&row.id).await;
        if receipt.is_ok() {
            assert_eq!(current.cma_sandbox_id.as_deref(), Some("sandbox_1"));
            assert_eq!(current.submission_state, "submitted");
            f.assert_single_intent().await;
            assert!(
                change_creation(&f.db, &row, CreationChange::NoResource)
                    .await
                    .is_err()
            );
        } else {
            assert_absent(&f, &row).await;
            assert!(
                change_creation(
                    &f.db,
                    &row,
                    CreationChange::Receipt(CreationReceipt::Sandbox("sandbox_1".into()))
                )
                .await
                .is_err()
            );
        }
        f.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn trust_concurrent_new_key_recoverers_leave_one_real_creation() {
    let f = Fixture::new().await;
    let cma = mock_cma(&f).await;
    let row = unknown_refusal(&f, true).await;
    let body = confirmed_body();
    let (a, b) = tokio::join!(
        proxy_create(&f, "key-a", &body, true),
        proxy_create(&f, "key-b", &body, true)
    );
    assert!(a.is_ok() || b.is_ok());
    assert_ne!(a.is_ok(), b.is_ok());
    assert_absent(&f, &row).await;
    assert_eq!(cma.creates.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(
        f.db.collection::<AssistantCmaSandbox>(
            crate::models::assistant_cma_sandbox::COLLECTION_NAME
        )
        .count_documents(doc! {"user_id":&f.owner,"occupies_account_slot":true})
        .await
        .unwrap(),
        1
    );
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn trust_automatic_recovery_skips_undispatched_and_lifecycle_creations() {
    for (route, dispatched) in [(SANDBOX_ROUTE, false), ("/api/v1/agents", true)] {
        let f = Fixture::new().await;
        mock_cma(&f).await;
        let permit = f
            .admit(&f.target, route, "original-key", UNCONFIRMED)
            .await
            .unwrap();
        if dispatched {
            permit
                .before_send(&f.request(route, "original-key", UNCONFIRMED))
                .await
                .unwrap();
        }
        let before =
            bson::to_document(&f.saved(&permit.creation.as_ref().unwrap().id).await).unwrap();
        let error = proxy_create(&f, "new-key", &confirmed_body(), true)
            .await
            .unwrap_err();
        assert!(
            matches!(error, AppError::Conflict(ref message) if message.contains("cma_sandbox_account_slot_occupied"))
        );
        assert_eq!(f.post_count().await, 0);
        assert_eq!(
            bson::to_document(&f.saved(&permit.creation.as_ref().unwrap().id).await).unwrap(),
            before
        );
        f.assert_single_intent().await;
        f.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn trust_automatic_recovery_transient_failure_retains_original_until_proven_absent() {
    let f = Fixture::new().await;
    let cma = mock_cma(&f).await;
    let row = unknown_refusal(&f, true).await;
    let before = f.saved(&row.id).await;
    f.server.reset().await;
    Mock::given(method("POST"))
        .and(path(SANDBOX_ROUTE))
        .respond_with(
            ResponseTemplate::new(503)
                .set_body_json(serde_json::json!({"code":"temporarily_unavailable"})),
        )
        .mount(&f.server)
        .await;
    assert!(
        proxy_create(&f, "new-key", &confirmed_body(), true)
            .await
            .is_err()
    );
    let after = f.saved(&row.id).await;
    assert_eq!(after.submission_state, "unknown");
    assert_eq!(after.request_ciphertext, before.request_ciphertext);
    assert_eq!(
        after.request_context_version,
        before.request_context_version
    );
    assert_eq!(after.first_dispatched_at, before.first_dispatched_at);
    assert_eq!(after.submission_generation, before.submission_generation);
    f.assert_single_intent().await;
    let requests = f.server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].body, UNCONFIRMED);
    assert_eq!(requests[0].headers["idempotency-key"], "original-key");
    f.server.reset().await;
    Mock::given(method("POST"))
        .and(path(SANDBOX_ROUTE))
        .respond_with(cma.clone())
        .mount(&f.server)
        .await;
    f.observations().await;
    assert_eq!(
        proxy_create(&f, "new-key", &confirmed_body(), true)
            .await
            .unwrap()
            .0,
        201
    );
    assert_absent(&f, &row).await;
    assert_eq!(f.post_count().await, 2);
    assert_eq!(cma.creates.load(std::sync::atomic::Ordering::SeqCst), 1);
    f.db.drop().await.unwrap();
}

#[tokio::test]
async fn trust_exact_original_unknown_retry_adds_no_automatic_recovery_send() {
    for contextual in [false, true] {
        let mut f = Fixture::new().await;
        f.keys.set_contextual_record_writes(contextual);
        let cma = mock_cma(&f).await;
        let row = unknown_refusal(&f, contextual).await;
        assert_eq!(
            proxy_create(&f, "original-key", UNCONFIRMED, contextual)
                .await
                .unwrap(),
            (409, trust_problem())
        );
        assert_eq!(
            f.post_count().await,
            2,
            "one original send and one caller-requested replay"
        );
        assert_absent(&f, &row).await;
        assert_eq!(
            f.saved(&row.id).await.submission_generation,
            row.submission_generation
        );
        assert_eq!(cma.creates.load(std::sync::atomic::Ordering::SeqCst), 0);
        f.db.drop().await.unwrap();
    }
}

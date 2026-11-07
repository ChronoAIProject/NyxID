mod images_tests;
use super::adapters::Native;
use super::*;
use crate::{
    models::{
        downstream_service::{PlatformKeyAudience, PlatformKeyConfig, test_helpers::dummy_service},
        platform_model_tariff::*,
        usage_meter::UsageMeterRow,
    },
    test_utils,
};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path, path_regex},
};

pub(super) fn sa() -> String {
    json!({"type":"service_account","project_id":"private-project","private_key_id":"fixture-id","private_key":test_utils::TEST_GCP_SA_PRIVATE_KEY,"client_email":"fixture@private-project.iam.gserviceaccount.com","client_id":"fixture-client","token_uri":"https://oauth2.googleapis.com/token"}).to_string()
}
struct Allowed;
#[async_trait]
impl Authority for Allowed {
    async fn check(&self, _: &Plan) -> AppResult<()> {
        Ok(())
    }
}
struct Refused;
#[async_trait]
impl Authority for Refused {
    async fn check(&self, _: &Plan) -> AppResult<()> {
        Err(registry::unavailable())
    }
}

fn configuration(native: Native, vertex: bool) -> (Plan, Request) {
    let (mut service, mut tariff, _) = billing::model_tariffs::tests::fixture();
    let name = match native {
        Native::Gemini => "gemini-2.5-flash",
        Native::Claude => "claude-sonnet-4-5",
        Native::Responses => "gpt-5",
        Native::GptOss => "gpt-oss-120b",
    };
    tariff.public_model = format!("nyxid/{name}");
    tariff.contract.family = native.family();
    tariff.contract.cache_read = native != Native::GptOss;
    tariff.contract.version = match native {
        Native::Gemini => "gemini.generate.v1",
        Native::Claude => "anthropic.messages.v1",
        Native::Responses => "openai.responses.v1",
        Native::GptOss => "open_maas.certified.v1",
    }
    .into();
    tariff.contract.fixture_sha256 = native.digest();
    tariff.contract.cache_write_ttls = if native == Native::Claude {
        vec![CacheTtl::FiveMinutes, CacheTtl::OneHour]
    } else {
        vec![]
    };
    tariff.contract.requests = true;
    tariff.bounds = RequestBounds {
        max_input_tokens: 1024,
        max_output_tokens: 128,
        max_candidates: 1,
        max_tool_iterations: 1,
        max_images: 0,
        max_request_bytes: 8192,
        max_response_bytes: 65536,
    };
    tariff.components = billing::model_usage::selectors(&tariff.contract)
        .unwrap()
        .into_iter()
        .enumerate()
        .map(|(i, selector)| TariffComponent {
            key: format!("unit{i}"),
            metric: billing::model_usage::metric(selector),
            selector,
            credits_per_unit: "0".into(),
            explicit_free: true,
            max_quantity: if selector == Selector::Requests {
                1
            } else {
                1024
            },
            lago_metric_code: String::new(),
        })
        .collect();
    billing::model_tariffs::validate(&mut tariff, &service.slug).unwrap();
    service.auth_method = "none".into();
    service.credential_encrypted.clear();
    service.identity_propagation_mode = "none".into();
    service.platform_key.as_mut().unwrap().enabled = true;
    let model = PlatformModel {
        id: tariff.model_id.clone(),
        public_id: tariff.public_model.clone(),
        display_name: name.into(),
        description: String::new(),
        family: match native {
            Native::Gemini => ModelFamily::Gemini,
            Native::Claude => ModelFamily::Claude,
            Native::Responses => ModelFamily::Openai,
            Native::GptOss => ModelFamily::OpenModel,
        },
        capabilities: Capabilities {
            protocols: vec![CallerProtocol::OpenaiChat],
            input_modalities: vec![crate::models::platform_model::Modality::Text],
            output_modalities: vec![crate::models::platform_model::Modality::Text],
            context_tokens: Some(1024),
            output_tokens: Some(128),
            streaming: Some(true),
            tools: Some(true),
            structured_output: None,
        },
        lifecycle: Lifecycle::Active,
        enabled: true,
        revision: 1,
        authorization_revision: 1,
        routing_revision_id: None,
        billing_service_id: Some(service.id.clone()),
        entitlements: Entitlements {
            all_people: true,
            rules: vec![],
        },
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    let target = match native {
        Native::Gemini => RouteTarget::VertexPublisher {
            publisher: VertexPublisher::Google,
            project: "private-project".into(),
            location: "global".into(),
            native_model: name.into(),
        },
        Native::Claude if vertex => RouteTarget::VertexPublisher {
            publisher: VertexPublisher::Anthropic,
            project: "private-project".into(),
            location: "us-east5".into(),
            native_model: name.into(),
        },
        Native::Claude => RouteTarget::DirectProvider {
            provider: DirectProvider::Anthropic,
            native_model: name.into(),
        },
        Native::Responses => RouteTarget::DirectProvider {
            provider: DirectProvider::Openai,
            native_model: name.into(),
        },
        Native::GptOss => RouteTarget::VertexOpenapiMaas {
            project: "private-project".into(),
            location: "us-central1".into(),
            publisher: "openai".into(),
            native_model: "gpt-oss-120b-maas".into(),
        },
    };
    let route = ModelRoute {
        id: uuid::Uuid::new_v4().to_string(),
        credential_service_id: uuid::Uuid::new_v4().to_string(),
        enabled: true,
        priority: 0,
        weight: 1,
        target,
    };
    let selected = billing::model_tariffs::select(
        &service,
        &model.id,
        CredentialClass::NyxidManagedMaster,
        &BTreeMap::new(),
        &[tariff],
        &HashMap::new(),
        Utc::now(),
        900,
    )
    .unwrap();
    let request=Request::parse(&http::Method::POST,"chat/completions",json!({"model":model.public_id,"max_tokens":128,"messages":[{"role":"user","content":"Say hello"}]})).unwrap();
    (
        Plan {
            model,
            offering: service,
            tariff: selected,
            routes: vec![route],
        },
        request,
    )
}
pub(crate) async fn fixture(native: Native, vertex: bool) -> (crate::AppState, Plan, Request) {
    let db = test_utils::connect_transaction_test_database("platform_model_execution").await;
    ensure_indexes(&db).await.unwrap();
    crate::services::service_concurrency_service::ensure_indexes(&db)
        .await
        .unwrap();
    db.collection::<Document>("usage_meter")
        .create_index(
            mongodb::IndexModel::builder()
                .keys(doc! {"transaction_id":1})
                .options(
                    mongodb::options::IndexOptions::builder()
                        .unique(true)
                        .build(),
                )
                .build(),
        )
        .await
        .unwrap();
    let state = test_utils::test_app_state(db);
    let (plan, request) = configuration(native, vertex);
    state
        .db
        .collection::<DownstreamService>("downstream_services")
        .insert_one(&plan.offering)
        .await
        .unwrap();
    state
        .db
        .collection::<ModelTariff>(crate::models::platform_model_tariff::COLLECTION_NAME)
        .insert_one(plan.tariff.tariff())
        .await
        .unwrap();
    let mut credential = dummy_service();
    credential.id = plan.routes[0].credential_service_id.clone();
    credential.visibility = "private".into();
    credential.is_active = true;
    credential.auth_method = "bearer".into();
    credential.identity_propagation_mode = "none".into();
    credential.provider_config_id = None;
    credential.requires_user_credential = false;
    credential.inject_delegation_token = false;
    credential.forward_access_token = false;
    credential.platform_key = Some(PlatformKeyConfig {
        purpose: PlatformKeyPurpose::ModelRoute,
        enabled: true,
        audience: PlatformKeyAudience::Restricted,
        ..Default::default()
    });
    let raw = if matches!(plan.routes[0].target, RouteTarget::DirectProvider { .. }) {
        "test-direct-secret".into()
    } else {
        sa()
    };
    credential.credential_encrypted = crate::services::catalog_credential_record_service::encrypt(
        &state.encryption_keys,
        &credential.id,
        crate::services::catalog_credential_record_service::owner(&credential).unwrap(),
        raw.as_bytes(),
    )
    .await
    .unwrap();
    credential.secret_context_versions =
        crate::services::catalog_credential_record_service::initial_versions(
            &state.encryption_keys,
            true,
        );
    state
        .db
        .collection::<DownstreamService>("downstream_services")
        .insert_one(credential)
        .await
        .unwrap();
    (state, plan, request)
}
async fn token_mock(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"access_token":"test-google-token","token_type":"Bearer","expires_in":3600}),
        ))
        .mount(server)
        .await;
}
fn context<'a>(
    state: &'a crate::AppState,
    runtime: &'a Runtime,
    authority: &'a dyn Authority,
) -> Context<'a> {
    Context {
        db: &state.db,
        keys: &state.encryption_keys,
        billing: &state.billing,
        actor: "00000000-0000-4000-8000-000000000001",
        api_key: None,
        request_id: uuid::Uuid::new_v4().to_string(),
        authority,
        cancel: CancellationToken::new(),
        runtime,
    }
}

#[test]
fn regional_global_urls_and_vertex_envelopes_are_closed() {
    for (native, vertex, part) in [
        (
            Native::Gemini,
            true,
            "https://aiplatform.googleapis.com/v1/projects/private-project/locations/global/publishers/google/models/gemini-2.5-flash:generateContent",
        ),
        (
            Native::Claude,
            true,
            "https://us-east5-aiplatform.googleapis.com/v1/projects/private-project/locations/us-east5/publishers/anthropic/models/claude-sonnet-4-5:rawPredict",
        ),
        (
            Native::GptOss,
            true,
            "https://us-central1-aiplatform.googleapis.com/v1/projects/private-project/locations/us-central1/endpoints/openapi/chat/completions",
        ),
    ] {
        let (plan, request) = configuration(native, vertex);
        let prepared =
            adapters::prepare(&plan.model, &plan.routes[0], plan.tariff.tariff(), &request)
                .unwrap();
        assert_eq!(prepared.url, part);
        if native == Native::GptOss {
            assert_eq!(prepared.body["model"], "openai/gpt-oss-120b-maas");
            assert_eq!(prepared.body["max_tokens"], request.max_output);
            assert!(prepared.body.get("max_completion_tokens").is_none());
            assert!(prepared.body.get("stream_options").is_none());
        }
        if native == Native::Claude {
            assert!(prepared.body.get("model").is_none());
            assert_eq!(prepared.body["anthropic_version"], "vertex-2023-10-16");
        }
    }
}
#[test]
fn reject_lossy_unbounded_and_uncertified_requests() {
    let (plan, request) = configuration(Native::Responses, false);
    for (key, value) in [
        ("previous_response_id", json!("private")),
        ("n", json!(2)),
        ("tools", json!([{"type":"web_search"}])),
        ("max_tokens", json!(-1)),
        ("response_format", json!({"type":"json_object"})),
    ] {
        let mut body = request.body.clone();
        body[key] = value;
        assert!(
            Request::parse(&http::Method::POST, "chat/completions", body).is_err(),
            "{key}"
        );
    }
    let mut small = plan.tariff.tariff().clone();
    small.bounds.max_input_tokens = 1023;
    assert!(request.validate_bounds(&plan.model, &small).is_err());
    small = plan.tariff.tariff().clone();
    small.contract.fixture_sha256 = "0".repeat(64);
    assert!(adapters::prepare(&plan.model, &plan.routes[0], &small, &request).is_err());
}
#[test]
fn responses_output_minimum_is_checked_before_admission_without_restricting_claude() {
    for (native, bound, allowed) in [
        (Native::Responses, 1, false),
        (Native::Responses, 15, false),
        (Native::Responses, 16, true),
        (Native::Claude, 1, true),
    ] {
        let (plan, mut request) = configuration(native, false);
        request.max_output = bound;
        request.body["max_tokens"] = json!(bound);
        assert_eq!(
            adapters::prepare(&plan.model, &plan.routes[0], plan.tariff.tariff(), &request).is_ok(),
            allowed,
        );
    }
}

#[tokio::test]
async fn all_five_provider_routes_meter_once_and_strip_private_metadata() {
    for (native, vertex) in [
        (Native::Gemini, true),
        (Native::Claude, true),
        (Native::GptOss, true),
        (Native::Responses, false),
        (Native::Claude, false),
    ] {
        let (state, plan, request) = fixture(native, vertex).await;
        let server = MockServer::start().await;
        token_mock(&server).await;
        Mock::given(method("POST"))
            .and(path_regex("^/v1/.*"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_raw(native.fixture(), "application/json")
                    .insert_header("x-goog-project-id", "private-project"),
            )
            .expect(1)
            .mount(&server)
            .await;
        let runtime = Runtime::mocked(&server.uri());
        let context = context(&state, &runtime, &Allowed);
        let accepted = dispatch(&context, &plan, &request).await.unwrap();
        let result = consume(&context, &plan, accepted, None).await.unwrap();
        assert_eq!(result["model"], plan.model.public_id);
        assert_eq!(result["choices"][0]["message"]["content"], "Hello");
        let text = result.to_string();
        let requests = server.received_requests().await.unwrap();
        let sent = requests
            .iter()
            .find(|r| r.url.path().starts_with("/v1/"))
            .unwrap();
        if native == Native::Claude && !vertex {
            assert_eq!(sent.headers["x-api-key"], "test-direct-secret");
            assert!(!sent.headers.contains_key("authorization"));
        } else {
            let expected = if vertex || matches!(native, Native::Gemini | Native::GptOss) {
                "Bearer test-google-token"
            } else {
                "Bearer test-direct-secret"
            };
            assert_eq!(sent.headers["authorization"], expected);
        }
        for hidden in [
            "private-project",
            "private-native",
            "private-response",
            "test-direct-secret",
            "test-google-token",
            "system_fingerprint",
        ] {
            assert!(!text.contains(hidden), "leaked metadata");
        }
        let rows: Vec<UsageMeterRow> = state
            .db
            .collection("usage_meter")
            .find(doc! {})
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        assert_eq!(rows.len(), plan.tariff.tariff().components.len());
        assert_eq!(
            rows.iter()
                .filter(|r| r.pending_platform_usage.is_some()
                    || r.model_tariff.as_ref().unwrap().evidence.is_some())
                .count(),
            1
        );
        assert!(
            rows.iter()
                .all(|r| r.model.as_deref() == Some(plan.model.public_id.as_str()))
        );
        server.verify().await;
    }
}
#[tokio::test]
async fn proven_zero_refusal_releases_only_that_attempt_and_falls_back() {
    let (state, mut plan, request) = fixture(Native::Responses, false).await;
    let mut backup = plan.routes[0].clone();
    backup.id = uuid::Uuid::new_v4().to_string();
    plan.routes.push(backup);
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(ResponseTemplate::new(429).set_body_json(
            json!({"error":{"type":"rate_limit_error","code":"rate_limit_exceeded"}}),
        ))
        .up_to_n_times(1)
        .with_priority(1)
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw(Native::Responses.fixture(), "application/json"),
        )
        .with_priority(2)
        .expect(1)
        .mount(&server)
        .await;
    let runtime = Runtime::mocked(&server.uri());
    let context = context(&state, &runtime, &Allowed);
    let accepted = dispatch(&context, &plan, &request).await.unwrap();
    consume(&context, &plan, accepted, None).await.unwrap();
    let first: Vec<UsageMeterRow> = state
        .db
        .collection("usage_meter")
        .find(doc! {"last_error":"model_zero_consumption_refusal"})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert_eq!(first.len(), plan.tariff.tariff().components.len());
    assert!(first.iter().all(|r| r.released && !r.forwarded));
    server.verify().await;
}
#[tokio::test]
async fn cooled_routes_do_not_hide_a_healthy_later_route() {
    let (state, mut plan, request) = fixture(Native::Responses, false).await;
    for _ in 0..4 {
        let mut route = plan.routes[0].clone();
        route.id = uuid::Uuid::new_v4().to_string();
        plan.routes.push(route);
    }
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw(Native::Responses.fixture(), "application/json"),
        )
        .expect(1)
        .mount(&server)
        .await;
    let runtime = Runtime::mocked(&server.uri());
    let version = credential_version(
        &credential(&state.db, &plan.routes[0].credential_service_id)
            .await
            .unwrap(),
    );
    for route in plan.routes.iter().take(4) {
        runtime
            .cool(format!("{}:{version}", route.id), Duration::from_secs(60))
            .await;
    }
    let context = context(&state, &runtime, &Allowed);
    let accepted = dispatch(&context, &plan, &request).await.unwrap();
    consume(&context, &plan, accepted, None).await.unwrap();
    server.verify().await;
}

#[tokio::test]
async fn fallback_never_sends_more_than_four_eligible_attempts() {
    let (state, mut plan, request) = fixture(Native::Responses, false).await;
    for _ in 0..4 {
        let mut route = plan.routes[0].clone();
        route.id = uuid::Uuid::new_v4().to_string();
        plan.routes.push(route);
    }
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(ResponseTemplate::new(429).set_body_json(json!({
            "error": {"type": "rate_limit_error", "code": "rate_limit_exceeded"},
        })))
        .expect(4)
        .mount(&server)
        .await;
    let runtime = Runtime::mocked(&server.uri());
    assert!(
        dispatch(&context(&state, &runtime, &Allowed), &plan, &request)
            .await
            .is_err()
    );
    server.verify().await;
}

#[tokio::test]
async fn ambiguous_refusal_never_falls_back_and_recovery_keeps_holds() {
    let (state, mut plan, request) = fixture(Native::Responses, false).await;
    let mut backup = plan.routes[0].clone();
    backup.id = uuid::Uuid::new_v4().to_string();
    plan.routes.push(backup);
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(ResponseTemplate::new(503).set_body_json(json!({"error":"private-account"})))
        .expect(1)
        .mount(&server)
        .await;
    let runtime = Runtime::mocked(&server.uri());
    let context = context(&state, &runtime, &Allowed);
    assert!(dispatch(&context, &plan, &request).await.is_err());
    assert_eq!(recover(&state.db).await.unwrap(), 0);
    let rows: Vec<UsageMeterRow> = state
        .db
        .collection("usage_meter")
        .find(doc! {})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert!(
        rows.iter()
            .all(|r| r.forwarded && !r.released && r.quantity.is_none())
    );
    assert_eq!(diagnostics(&state.db).await.unwrap().len(), 1);
    server.verify().await;
}
#[tokio::test]
async fn missing_usage_and_cancellation_do_not_become_zero_or_fallback() {
    for cancelled in [false, true] {
        let (state, mut plan, request) = fixture(Native::Responses, false).await;
        let mut backup = plan.routes[0].clone();
        backup.id = uuid::Uuid::new_v4().to_string();
        plan.routes.push(backup);
        let server = MockServer::start().await;
        let mut result: Value = serde_json::from_str(Native::Responses.fixture()).unwrap();
        result.as_object_mut().unwrap().remove("usage");
        Mock::given(method("POST"))
            .and(path("/v1/responses"))
            .respond_with(ResponseTemplate::new(200).set_body_json(result))
            .expect(1)
            .mount(&server)
            .await;
        let runtime = Runtime::mocked(&server.uri());
        let context = context(&state, &runtime, &Allowed);
        let accepted = dispatch(&context, &plan, &request).await.unwrap();
        if cancelled {
            context.cancel.cancel();
        }
        assert!(consume(&context, &plan, accepted, None).await.is_err());
        assert_eq!(diagnostics(&state.db).await.unwrap().len(), 1);
        server.verify().await;
    }
}
#[tokio::test]
async fn authority_failure_and_pre_cancel_never_create_a_meter_or_send() {
    let (state, plan, request) = fixture(Native::Responses, false).await;
    let server = MockServer::start().await;
    let runtime = Runtime::mocked(&server.uri());
    let context = context(&state, &runtime, &Refused);
    assert!(dispatch(&context, &plan, &request).await.is_err());
    context.cancel.cancel();
    assert!(dispatch(&context, &plan, &request).await.is_err());
    assert_eq!(
        state
            .db
            .collection::<Document>("usage_meter")
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn structured_credential_cannot_be_forwarded_to_a_direct_provider() {
    let (state, plan, request) = fixture(Native::Responses, false).await;
    let server = MockServer::start().await;
    let runtime = Runtime::mocked(&server.uri());
    let mut row = credential(&state.db, &plan.routes[0].credential_service_id)
        .await
        .unwrap();
    row.credential_encrypted = crate::services::catalog_credential_record_service::encrypt(
        &state.encryption_keys,
        &row.id,
        crate::services::catalog_credential_record_service::owner(&row).unwrap(),
        sa().as_bytes(),
    )
    .await
    .unwrap();
    state
        .db
        .collection::<DownstreamService>("downstream_services")
        .replace_one(doc! {"_id": &row.id}, row)
        .await
        .unwrap();
    assert!(
        dispatch(&context(&state, &runtime, &Allowed), &plan, &request)
            .await
            .is_err()
    );
    assert!(server.received_requests().await.unwrap().is_empty());
    assert_eq!(
        state
            .db
            .collection::<Document>("usage_meter")
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
}
fn events(native: Native) -> String {
    let response: Value = serde_json::from_str(native.fixture()).unwrap();
    let values = match native {
        Native::Responses => vec![
            json!({"type":"response.output_text.delta","sequence_number":1,"delta":"Hello"}),
            json!({"type":"response.completed","sequence_number":2,"response":response}),
        ],
        Native::Claude => vec![
            json!({"type":"message_start","message":{"usage":response["usage"]}}),
            json!({"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}),
            json!({"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"Hello"}}),
            json!({"type":"content_block_stop","index":0}),
            json!({"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":20}}),
            json!({"type":"message_stop"}),
        ],
        Native::Gemini => vec![response],
        Native::GptOss => vec![
            json!({"choices":[{"index":0,"delta":{"content":"Hello"},"finish_reason":null}]}),
            json!({"choices":[{"index":0,"delta":{},"finish_reason":"stop"}]}),
            json!({"choices":[],"usage":response["usage"]}),
        ],
    };
    let mut frames = values
        .into_iter()
        .map(|v| format!("data: {v}\r\n\r\n"))
        .collect::<String>();
    if native == Native::GptOss {
        frames.push_str("data: [DONE]\n\n");
    }
    frames
}
#[test]
fn native_sse_byte_framing_final_usage_and_duplicate_sequence() {
    for native in [
        Native::Gemini,
        Native::Claude,
        Native::Responses,
        Native::GptOss,
    ] {
        let mut decoder = stream::Decoder::new(native);
        let mut text = String::new();
        for byte in events(native).bytes() {
            for t in decoder.push(&[byte]).unwrap() {
                text.push_str(&t);
            }
        }
        let (output, evidence) = decoder.finish(true, "public-request").unwrap();
        assert_eq!(text, "Hello");
        assert_eq!(output.unwrap().text, "Hello");
        assert_eq!(evidence.input, Some(100));
    }
    let mut decoder = stream::Decoder::new(Native::Responses);
    let frame=b"data: {\"type\":\"response.output_text.delta\",\"sequence_number\":1,\"delta\":\"hello\"}\n\n";
    assert_eq!(decoder.push(frame).unwrap(), ["hello"]);
    assert!(decoder.push(frame).unwrap().is_empty());
    assert!(decoder.finish(false, "id").is_err());
}
#[tokio::test]
async fn all_native_streams_settle_before_public_done_without_native_ids() {
    for native in [
        Native::Gemini,
        Native::Claude,
        Native::Responses,
        Native::GptOss,
    ] {
        let (state, plan, mut request) = fixture(native, true).await;
        request.streaming = true;
        request.body["stream"] = json!(true);
        let server = MockServer::start().await;
        token_mock(&server).await;
        Mock::given(method("POST"))
            .and(path_regex("^/v1/.*"))
            .respond_with(
                ResponseTemplate::new(200).set_body_raw(events(native), "text/event-stream"),
            )
            .expect(1)
            .mount(&server)
            .await;
        let runtime = Runtime::mocked(&server.uri());
        let context = context(&state, &runtime, &Allowed);
        let accepted = dispatch(&context, &plan, &request).await.unwrap();
        let (tx, mut rx) = tokio::sync::mpsc::channel(32);
        consume(&context, &plan, accepted, Some(&tx)).await.unwrap();
        drop(tx);
        let mut output = String::new();
        while let Some(chunk) = rx.recv().await {
            output.push_str(std::str::from_utf8(&chunk.unwrap()).unwrap());
        }
        assert!(output.contains("[DONE]"));
        assert!(output.contains(&plan.model.public_id));
        assert!(!output.contains("private-"));
        assert!(output.contains("Hello"));
        server.verify().await;
    }
}
#[tokio::test]
async fn expired_attempt_diagnosed_once_without_releasing_and_known_intent_wins() {
    let (state, plan, request) = fixture(Native::Responses, false).await;
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw(Native::Responses.fixture(), "application/json"),
        )
        .mount(&server)
        .await;
    let runtime = Runtime::mocked(&server.uri());
    let context = context(&state, &runtime, &Allowed);
    let accepted = dispatch(&context, &plan, &request).await.unwrap();
    drop(accepted);
    state.db.collection::<Document>(ATTEMPTS).update_many(doc!{},doc!{"$set":{"lease_until":bson::DateTime::from_chrono(Utc::now()-chrono::Duration::seconds(1))}}).await.unwrap();
    assert_eq!(recover(&state.db).await.unwrap(), 1);
    assert_eq!(recover(&state.db).await.unwrap(), 0);
    assert_eq!(
        state
            .db
            .collection::<Document>("usage_meter")
            .count_documents(doc! {"released":true})
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn paid_provider_evidence_uses_exact_funding_tax_and_one_durable_settlement() {
    paid_provider_evidence(false).await;
}
#[tokio::test]
async fn paid_image_evidence_uses_exact_funding_tax_ledger_lago_and_recovery() {
    paid_provider_evidence(true).await;
}
async fn paid_provider_evidence(images: bool) {
    use crate::models::{
        billing_rate_cache::BillingRateCache,
        billing_target::BillingTargetKind,
        billing_tax_profile::{BillingTaxProfile, TaxComponent},
        credits::Credits,
        usage_allowance::{AllowanceRecurrence, UsageAllowance},
    };
    let (state, mut plan, mut request) = fixture(Native::Responses, false).await;
    if images {
        request = images_tests::image_plan(&mut plan, false);
        crate::services::security_readiness_service::ready_test_replica(&state.db).await;
    }
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/subscriptions/subscription/entitlements"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"entitlements":[{"code":"*"}]})),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw(Native::Responses.fixture(), "application/json"),
        )
        .expect(1)
        .mount(&server)
        .await;
    let db = &state.db;
    // The shared test helper starts after cutover. This fixture seeds legacy
    // balances deliberately, so run the real migration and its opening ledger
    // postings before admission; a ready marker would leave grants unspendable.
    db.collection::<Document>("billing_migrations")
        .delete_one(doc! {"_id": billing::exact_migration::BILLING_MARKER})
        .await
        .unwrap();
    for (keys, unique, sparse) in [
        (doc! {"seq": 1}, true, false),
        (doc! {"dedupe_key": 1}, true, true),
        (doc! {"postings.account": 1, "seq": 1}, false, false),
    ] {
        db.collection::<Document>("billing_ledger")
            .create_index(
                mongodb::IndexModel::builder()
                    .keys(keys)
                    .options(
                        mongodb::options::IndexOptions::builder()
                            .unique(unique)
                            .sparse(sparse)
                            .build(),
                    )
                    .build(),
            )
            .await
            .unwrap();
    }
    let actor = "00000000-0000-4000-8000-000000000001";
    let now = bson::DateTime::now();
    db.collection::<crate::models::user::User>("users")
        .insert_one(test_utils::test_user(
            actor,
            crate::models::user::UserType::Person,
        ))
        .await
        .unwrap();
    db.collection::<Document>("billing_wallet").insert_one(doc!{"_id":"wallet","owner_id":actor,"lago_customer_id":actor,"lago_wallet_id":"wallet","lago_subscription_id":"subscription","plan_kind":"prepaid","balance_credits":100_i64,"reserved_credits":0_i64,"pending_lago_debits":0_i64,"pending_topup_expiry_credits":0_i64,"overdraft_cap_credits":0_i64,"has_payment_instrument":false,"suspended":false,"collection_state":"good","balance_synced_at":now,"created_at":now,"updated_at":now}).await.unwrap();
    db.collection::<Document>("credit_grants").insert_one(doc!{"_id":"grant","batch_id":"batch","recipient_user_id":actor,"target_kind":"selected_users","amount_credits":1_i64,"amount_micros":50_000_i64,"remaining_micros":50_000_i64,"reserved_micros":0_i64,"scope":{"all_services":true,"service_ids":[],"service_slugs":[]},"granted_by":"admin","status":"active","created_at":now,"updated_at":now,"issued_ledgered_at":now}).await.unwrap();
    billing::ledger::init_billing_ledger_hmac_key(Zeroizing::new(
        billing::ledger::TEST_BILLING_LEDGER_HMAC_KEY,
    ));
    billing::exact_migration::run(db).await.unwrap();
    let active_grants = billing::grants::list_active_for_user(db, actor, Utc::now())
        .await
        .unwrap();
    assert_eq!(active_grants.len(), 1);
    assert_eq!(active_grants[0].remaining.to_string(), "0.05");

    db.collection::<Document>("feature_flag_overrides").insert_one(doc!{"_id":uuid::Uuid::new_v4().to_string(),"org_user_id":bson::Bson::Null,"flag_key":"experimental:billing","target_kind":"global","target_key":bson::Bson::Null,"enabled":true,"created_at":now,"updated_at":now,"updated_by":actor}).await.unwrap();
    let mut tariff = plan.tariff.tariff().clone();
    let mut rates = HashMap::new();
    for c in &mut tariff.components {
        c.explicit_free = false;
        c.credits_per_unit = "0.001000001".into();
        rates.insert(
            c.lago_metric_code.clone(),
            BillingRateCache {
                id: BillingRateCache::cache_id(&c.lago_metric_code, None),
                lago_metric_code: c.lago_metric_code.clone(),
                model: None,
                credits_per_unit_micros: 1000,
                credits_per_unit_pico: Some(1_000_001_000),
                retired_at: None,
                synced_at: Utc::now(),
            },
        );
    }
    billing::model_tariffs::validate(&mut tariff, &plan.offering.slug).unwrap();
    db.collection::<ModelTariff>(crate::models::platform_model_tariff::COLLECTION_NAME)
        .replace_one(doc! {"_id":&tariff.id}, &tariff)
        .await
        .unwrap();
    db.collection::<BillingRateCache>("billing_rate_cache")
        .insert_many(rates.values())
        .await
        .unwrap();
    plan.tariff = billing::model_tariffs::select(
        &plan.offering,
        &plan.model.id,
        CredentialClass::NyxidManagedMaster,
        &BTreeMap::new(),
        &[tariff],
        &rates,
        Utc::now(),
        900,
    )
    .unwrap();
    let tax = TaxComponent {
        code: "gst".into(),
        name: "GST".into(),
        rate_percent: Credits::from_whole(9),
    };
    db.collection::<BillingTaxProfile>("billing_tax_profiles")
        .insert_one(BillingTaxProfile {
            id: actor.into(),
            owner_id: actor.into(),
            subscription_id: "subscription".into(),
            plan_code: "starter".into(),
            billing_entity_code: "merchant".into(),
            default_taxes: vec![tax.clone()],
            metric_taxes: rates
                .keys()
                .map(|c| (c.clone(), vec![tax.clone()]))
                .collect(),
            fingerprint: "fixture".into(),
            fetched_at: Utc::now(),
            refresh_failed_at: None,
        })
        .await
        .unwrap();
    db.collection::<UsageAllowance>("usage_allowances")
        .insert_one(UsageAllowance {
            id: "allowance".into(),
            bundle_id: None,
            service_id: plan.offering.id.clone(),
            service_slug: plan.offering.slug.clone(),
            metric: BillingMetric::InputTokens,
            quantity: 10,
            recurrence: AllowanceRecurrence::Daily,
            target_kind: BillingTargetKind::AllUsers,
            target_user_ids: vec![],
            target_org_ids: vec![],
            target_group_ids: vec![],
            is_active: true,
            created_by: actor.into(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        })
        .await
        .unwrap();
    let mut config = state.config.clone();
    config.billing_enabled = true;
    config.lago_api_url = Some(server.uri());
    config.lago_api_key = Some("fixture-lago".into());
    let billing = BillingService::new(db.clone(), Arc::new(config.clone()));
    let runtime = Runtime::mocked(&server.uri());
    let mut context = context(&state, &runtime, &Allowed);
    context.billing = &billing;
    let accepted = dispatch(&context, &plan, &request).await.unwrap();
    let metered = accepted.metered.clone();
    consume(&context, &plan, accepted, None).await.unwrap();
    let evidence = adapters::evidence(
        Native::Responses,
        &serde_json::from_str::<Value>(Native::Responses.fixture()).unwrap()["usage"],
    )
    .unwrap();
    let expected_usage = plan.tariff.usage(&evidence).unwrap();
    let vector = expected_usage.model_charges.as_deref().unwrap();
    // PR 2 stores the complete immutable evidence vector once, on the
    // coordinator. Each component's quantity is materialized before returning;
    // its funding/ledger settlement remains deferred and recoverable.
    let durable: Vec<UsageMeterRow> = db
        .collection("usage_meter")
        .find(doc! {"billing_request_id": &metered.route.as_ref().unwrap().billing_request_id})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert_eq!(durable.len(), 4);
    for row in &durable {
        assert_eq!(
            row.quantity.as_ref(),
            vector.quantities.get(&row.lago_metric_code)
        );
        assert_eq!(
            row.model_tariff.as_ref().unwrap().tariff_id,
            vector.tariff_id
        );
    }
    let coordinators: Vec<_> = durable
        .iter()
        .filter_map(|row| row.model_tariff.as_ref())
        .filter(|binding| binding.evidence.is_some())
        .collect();
    assert_eq!(coordinators.len(), 1);
    assert_eq!(coordinators[0].evidence.as_deref(), Some(vector));
    assert_eq!(
        coordinators[0].evidence_digest.as_deref(),
        Some(hex::encode(Sha256::digest(serde_json::to_vec(vector).unwrap())).as_str()),
        "the complete evidence vector is durable before the response returns",
    );
    // Race/replay the identical durable vector through the production coordinator.
    billing
        .settle(
            &metered,
            plan.tariff.usage(&evidence).unwrap(),
            None,
            Some(plan.model.public_id.clone()),
        )
        .await
        .unwrap();
    // A duplicate settlement intent is a no-op while the original deferred
    // worker owns it. Await its durable result, not the replay's return value.
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            if db
                .collection::<Document>("usage_meter")
                .count_documents(doc! {"released": true, "funding.settled": true})
                .await
                .unwrap()
                == 4
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("deferred component settlements complete through the ordinary coordinator");
    let rows: Vec<UsageMeterRow> = db
        .collection("usage_meter")
        .find(doc! {})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert_eq!(rows.len(), 4);
    let mut allowance = Credits::ZERO;
    let mut grant = Credits::ZERO;
    let mut wallet = Credits::ZERO;
    for row in &rows {
        let f = row.funding.as_ref().unwrap();
        assert!(f.settled);
        assert_eq!(
            f.wallet_charge_credits.unwrap(),
            f.wallet_funded
                .unwrap()
                .checked_add(f.wallet_tax_credits.unwrap())
                .unwrap()
        );
        allowance = allowance.checked_add(f.allowance_funded.unwrap()).unwrap();
        grant = grant.checked_add(f.grant_funded.unwrap()).unwrap();
        wallet = wallet.checked_add(f.wallet_funded.unwrap()).unwrap();
        assert_eq!(
            f.total_charge.unwrap(),
            f.allowance_funded
                .unwrap()
                .checked_add(f.grant_funded.unwrap())
                .unwrap()
                .checked_add(f.wallet_funded.unwrap())
                .unwrap()
        );
    }
    assert_eq!(allowance.to_string(), "0.01000001");
    assert_eq!(grant.to_string(), "0.05");
    assert_eq!(wallet.to_string(), "0.061000111");
    assert_eq!(
        db.collection::<Document>("billing_ledger")
            .count_documents(doc! {"movement": "usage_settled"})
            .await
            .unwrap(),
        4,
        "each wallet-funded component has one durable settlement posting",
    );
    let count = db
        .collection::<Document>("billing_ledger")
        .count_documents(doc! {})
        .await
        .unwrap();
    billing
        .settle(
            &metered,
            plan.tariff.usage(&evidence).unwrap(),
            None,
            Some(plan.model.public_id.clone()),
        )
        .await
        .unwrap();
    assert_eq!(
        db.collection::<Document>("billing_ledger")
            .count_documents(doc! {})
            .await
            .unwrap(),
        count
    );
    let verified = billing::ledger::verify_chain(
        db,
        &billing::ledger::TEST_BILLING_LEDGER_HMAC_KEY,
        None,
        None,
        None,
    )
    .await
    .unwrap();
    assert_eq!(verified.status, billing::ledger::BillingLedgerStatus::Ok);
    billing::account_reconciliation::run_once(db).await.unwrap();
    let verified_accounts = db
        .collection::<Document>("chain_verify_status")
        .find_one(doc! {"_id":"billing_accounts"})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(verified_accounts.get_str("outcome").unwrap(), "ok");

    // Real HTTP Lago client: a failure preserves the settled debit; retry emits
    // only the exact wallet-funded quantity with the stable tariff code/id.
    let failed = Mock::given(method("POST"))
        .and(path("/api/v1/events"))
        .respond_with(ResponseTemplate::new(503))
        .mount_as_scoped(&server)
        .await;
    let lago = Arc::new(
        billing::lago_client::LagoClient::new(server.uri(), "fixture-lago".into()).unwrap(),
    );
    let reconciler =
        billing::reconcile::BillingReconciler::new(db.clone(), Some(lago), Arc::new(config));
    let stale = bson::DateTime::from_chrono(Utc::now() - chrono::Duration::seconds(60));
    db.collection::<Document>("usage_meter")
        .update_many(doc! {}, doc! {"$set":{"updated_at":stale}})
        .await
        .unwrap();
    let failed_stats = reconciler.push_usage_for_test().await.unwrap();
    assert!(failed_stats.retried > 0);
    drop(failed);
    Mock::given(method("POST"))
        .and(path("/api/v1/events"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .mount(&server)
        .await;
    db.collection::<Document>("usage_meter")
        .update_many(doc! {}, doc! {"$set":{"updated_at":stale}})
        .await
        .unwrap();
    assert_eq!(
        reconciler.push_usage_for_test().await.unwrap().pushed,
        failed_stats.retried
    );
    assert_eq!(reconciler.push_usage_for_test().await.unwrap().pushed, 0);
    let events = server
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .filter(|r| r.url.path() == "/api/v1/events")
        .collect::<Vec<_>>();
    assert_eq!(events.len() as u64, failed_stats.retried * 2);
    for event in events {
        let value: Value = serde_json::from_slice(&event.body).unwrap();
        let actual = &value["event"];
        let row = rows
            .iter()
            .find(|r| r.transaction_id == actual["transaction_id"].as_str().unwrap())
            .unwrap();
        let expected =
            billing::lago_client::LagoEvent::from_usage_row(row, Some("subscription".into()))
                .unwrap();
        assert_eq!(actual, &serde_json::to_value(expected).unwrap());
        assert!(row.funding.as_ref().unwrap().wallet_funded.unwrap() > Credits::ZERO);
    }
    assert_eq!(
        db.collection::<Document>("billing_ledger")
            .count_documents(doc! {})
            .await
            .unwrap(),
        count
    );
    let mut remote_usage: Vec<_> = rows
        .iter()
        .filter_map(|row| {
            let quantity = row.funding.as_ref()?.lago_billable_quantity_micros?;
            (quantity > 0).then(|| {
                json!({
                    "code": row.lago_metric_code,
                    "units": Credits::from_pico(i128::from(quantity) * 1_000_000)
                        .unwrap().to_string(),
                })
            })
        })
        .collect();
    assert!(!remote_usage.is_empty());
    let matched = Mock::given(method("GET"))
        .and(path(format!("/api/v1/customers/{actor}/current_usage")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"fees": remote_usage})))
        .expect(remote_usage.len() as u64)
        .mount_as_scoped(&server)
        .await;
    assert_eq!(
        reconciler
            .compare_usage_for_test()
            .await
            .unwrap()
            .drift_alerts,
        0
    );
    drop(matched);
    // A deliberate one-unit discrepancy must be detected, proving the zero
    // above came from comparing the wallet-only quantities, not a skipped read.
    let original: Credits = remote_usage[0]["units"].as_str().unwrap().parse().unwrap();
    remote_usage[0]["units"] = json!(
        original
            .checked_add(Credits::from_whole(1))
            .unwrap()
            .to_string()
    );
    Mock::given(method("GET"))
        .and(path(format!("/api/v1/customers/{actor}/current_usage")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"fees": remote_usage})))
        .expect(remote_usage.len() as u64)
        .mount(&server)
        .await;
    assert_eq!(
        reconciler
            .compare_usage_for_test()
            .await
            .unwrap()
            .drift_alerts,
        1
    );
    server.verify().await;
}

#[test]
fn truncated_conflicting_and_incomplete_streams_never_invent_usage() {
    let mut claude = stream::Decoder::new(Native::Claude);
    let start =
        json!({"type":"message_start","message":{"usage":{"input_tokens":100,"output_tokens":1}}});
    claude
        .push(format!("data: {start}\n\n").as_bytes())
        .unwrap();
    assert!(
        claude
            .push(b"data: {\"type\":\"message_stop\"}\n\n")
            .is_err()
    );
    assert!(claude.finish(true, "id").is_err());
    let mut response = stream::Decoder::new(Native::Responses);
    response.push(b"data: {\"type\":\"response.output_text.delta\",\"sequence_number\":1,\"delta\":\"a\"}\n\n").unwrap();
    assert!(response.push(b"data: {\"type\":\"response.output_text.delta\",\"sequence_number\":1,\"delta\":\"b\"}\n\n").is_err());
    assert!(response.finish(false, "id").is_err());
    let mut maas = stream::Decoder::new(Native::GptOss);
    maas.push(events(Native::GptOss).as_bytes()).unwrap();
    assert!(maas.push(b"data: [DONE]\n\n").is_err());
}

#[tokio::test]
async fn visible_stream_failure_keeps_holds_and_never_selects_backup() {
    let (state, mut plan, mut request) = fixture(Native::Responses, false).await;
    request.streaming = true;
    request.body["stream"] = json!(true);
    let mut backup = plan.routes[0].clone();
    backup.id = uuid::Uuid::new_v4().to_string();
    plan.routes.push(backup);
    let server = MockServer::start().await;
    let frames = "data: {\"type\":\"response.output_text.delta\",\"sequence_number\":1,\"delta\":\"Hello\"}\n\n";
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(frames, "text/event-stream"))
        .expect(1)
        .mount(&server)
        .await;
    let runtime = Runtime::mocked(&server.uri());
    let context = context(&state, &runtime, &Allowed);
    let accepted = dispatch(&context, &plan, &request).await.unwrap();
    let (tx, mut rx) = tokio::sync::mpsc::channel(8);
    assert!(consume(&context, &plan, accepted, Some(&tx)).await.is_err());
    drop(tx);
    let visible = rx.recv().await.unwrap().unwrap();
    assert!(std::str::from_utf8(&visible).unwrap().contains("Hello"));
    assert!(rx.recv().await.is_none());
    let count = state
        .db
        .collection::<Document>("usage_meter")
        .count_documents(doc! {"forwarded":true,"released":false,"quantity":bson::Bson::Null})
        .await
        .unwrap();
    assert_eq!(count as usize, plan.tariff.tariff().components.len());
    assert_eq!(diagnostics(&state.db).await.unwrap().len(), 1);
    server.verify().await;
}

#[tokio::test]
async fn oversized_response_and_invalid_counters_keep_unknown_holds() {
    for oversized in [false, true] {
        let (state, plan, request) = fixture(Native::Responses, false).await;
        let server = MockServer::start().await;
        let mut body: Value = serde_json::from_str(Native::Responses.fixture()).unwrap();
        if oversized {
            body["output"][0]["content"][0]["text"] = json!("x".repeat(65537));
        } else {
            body["usage"]["input_tokens"] = json!(-1);
        }
        Mock::given(method("POST"))
            .and(path("/v1/responses"))
            .respond_with(ResponseTemplate::new(200).set_body_json(body))
            .expect(1)
            .mount(&server)
            .await;
        let runtime = Runtime::mocked(&server.uri());
        let context = context(&state, &runtime, &Allowed);
        let accepted = dispatch(&context, &plan, &request).await.unwrap();
        assert!(consume(&context, &plan, accepted, None).await.is_err());
        assert_eq!(diagnostics(&state.db).await.unwrap().len(), 1);
        server.verify().await;
    }
}

#[test]
fn statuses_without_certified_zero_consumption_envelopes_never_allow_fallback() {
    for status in [401, 403, 429] {
        assert!(!adapters::zero_consumption(
            Native::Responses,
            status,
            b"{}"
        ));
        assert!(!adapters::zero_consumption(
            Native::Responses,
            status,
            b"<html>proxy error</html>"
        ));
        assert!(!adapters::zero_consumption(
            Native::Responses,
            status,
            Native::Responses.fixture().as_bytes()
        ));
    }
    assert!(adapters::zero_consumption(
        Native::Responses,
        429,
        br#"{"error":{"code":"rate_limit_exceeded","type":"tokens","message":"private"}}"#
    ));
    assert!(adapters::zero_consumption(
        Native::Gemini,
        429,
        br#"{"error":{"code":429,"status":"RESOURCE_EXHAUSTED","message":"private"}}"#
    ));
    assert!(adapters::zero_consumption(
        Native::Claude,
        401,
        br#"{"type":"error","error":{"type":"authentication_error","message":"private"}}"#
    ));
    assert!(!adapters::zero_consumption(
        Native::Responses,
        429,
        br#"{"error":{"code":"rate_limit_exceeded"},"usage":{"input_tokens":1}}"#
    ));
}

#[tokio::test]
async fn recovery_reclaims_a_crashed_lease_and_recognizes_committed_evidence() {
    let (state, plan, request) = fixture(Native::Responses, false).await;
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw(Native::Responses.fixture(), "application/json"),
        )
        .expect(1)
        .mount(&server)
        .await;
    let runtime = Runtime::mocked(&server.uri());
    let context = context(&state, &runtime, &Allowed);
    let accepted = dispatch(&context, &plan, &request).await.unwrap();
    let id = accepted
        .metered
        .route
        .as_ref()
        .unwrap()
        .billing_request_id
        .clone();
    consume(&context, &plan, accepted, None).await.unwrap();
    state.db.collection::<Document>(ATTEMPTS).update_one(doc!{"_id":&id},
        doc!{"$set":{"state":"recovering","recovery_token":"crashed-worker","lease_until":bson::DateTime::from_chrono(Utc::now()-chrono::Duration::seconds(1))}}).await.unwrap();
    assert_eq!(recover(&state.db).await.unwrap(), 1);
    assert_eq!(recover(&state.db).await.unwrap(), 0);
    let row = state
        .db
        .collection::<Document>(ATTEMPTS)
        .find_one(doc! {"_id":&id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.get_str("state").unwrap(), "evidence_saved");
    assert!(diagnostics(&state.db).await.unwrap().is_empty());
    server.verify().await;
}

#[tokio::test]
async fn authority_loss_after_materialization_or_funding_never_sends() {
    struct RevokeAt {
        checks: std::sync::atomic::AtomicUsize,
        at: usize,
    }
    #[async_trait]
    impl Authority for RevokeAt {
        async fn check(&self, _: &Plan) -> AppResult<()> {
            let n = self
                .checks
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
                + 1;
            if n >= self.at {
                Err(registry::unavailable())
            } else {
                Ok(())
            }
        }
    }
    for at in [2, 3] {
        let (state, plan, request) = fixture(Native::Responses, false).await;
        let server = MockServer::start().await;
        let runtime = Runtime::mocked(&server.uri());
        let authority = RevokeAt {
            checks: Default::default(),
            at,
        };
        let context = context(&state, &runtime, &authority);
        assert!(dispatch(&context, &plan, &request).await.is_err());
        assert!(server.received_requests().await.unwrap().is_empty());
        assert_eq!(
            state
                .db
                .collection::<Document>("usage_meter")
                .count_documents(doc! {"released":false})
                .await
                .unwrap(),
            0
        );
    }
}

#[test]
fn maas_certification_uses_only_its_exact_model_and_documented_aggregate_evidence() {
    let (plan, request) = configuration(Native::GptOss, true);
    let mut tariff = plan.tariff.tariff().clone();
    tariff.contract.cache_read = true;
    assert!(adapters::prepare(&plan.model, &plan.routes[0], &tariff, &request).is_err());
    tariff.contract.cache_read = false;
    tariff.contract.split_reasoning = true;
    assert!(adapters::prepare(&plan.model, &plan.routes[0], &tariff, &request).is_err());
    let mut wrong = plan.model.clone();
    wrong.public_id = "nyxid/gpt-oss-20b".into();
    assert!(registry::validate_route(&wrong, &plan.routes[0]).is_err());
    let mut unverified: Value = serde_json::from_str(Native::GptOss.fixture()).unwrap();
    unverified["usage"]["prompt_tokens_details"] = json!({"cached_tokens":10});
    assert!(adapters::evidence(Native::GptOss, &unverified["usage"]).is_err());
}

use super::*;
use crate::{
    models::{
        assistant_conversation::AssistantConversation, downstream_service::DownstreamService,
    },
    services::{
        assistant_agent_credential_service as credentials, assistant_nyxagent as engine,
        platform_model_execution::adapters::Native,
    },
    test_utils,
};
use axum::{
    Router,
    routing::{get, post},
};
use bson::{Document, doc};
use std::sync::Arc;
use tower::ServiceExt;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

const PERSON: &str = "00000000-0000-4000-8000-000000000001";
fn capabilities() -> Value {
    json!({"platform_models":{"protocol":adapter::PROTOCOL,"version":1,"text":true,
        "function_tools":true,"image_input":true,"streaming":true,"compaction":true}})
}
fn start(model: &str) -> engine::TurnRequest {
    engine::TurnRequest {
        acknowledgement_id: None,
        retry_of: None,
        attachment_ids: vec![],
        agent_id: None,
        conversation_id: None,
        text: "Tell me something".into(),
        model: Some(model.into()),
        access_mode: None,
    }
}
async fn setup() -> (
    AppState,
    execution::Plan,
    AssistantConversation,
    credentials::AssistantCredential,
) {
    let (state, plan, _, _) = super::super::platform_model_gateway::tests::enabled().await;
    engine::ensure_indexes(&state.db).await.unwrap();
    let mut service = crate::models::downstream_service::test_helpers::dummy_service();
    service.id = uuid::Uuid::new_v4().to_string();
    service.slug = engine::SERVICE_SLUG.into();
    service.service_category = "internal".into();
    service.auth_method = "none".into();
    service.requires_user_credential = false;
    service.credential_encrypted.clear();
    service.forward_access_token = true;
    service.inject_delegation_token = false;
    state
        .db
        .collection::<DownstreamService>("downstream_services")
        .insert_one(service)
        .await
        .unwrap();
    let row = engine::begin_turn(
        &state.db,
        PERSON,
        &start(&plan.model.public_id),
        &state.encryption_keys,
    )
    .await
    .unwrap();
    let credential =
        credentials::load_for_conversation(&state.db, &state.encryption_keys, PERSON, &row.id)
            .await
            .unwrap()
            .expect("admitted conversation credential");
    (state, plan, row, credential)
}
async fn set_person_approvals(state: &AppState, person: &str, required: bool) {
    let mut channel =
        crate::services::notification_service::get_or_create_channel(&state.db, person)
            .await
            .unwrap();
    channel.approval_required = Some(required);
    channel.approval_timeout_secs = 5;
    state
        .db
        .collection::<crate::models::notification_channel::NotificationChannel>(
            "notification_channels",
        )
        .replace_one(doc! {"_id": &channel.id}, channel)
        .await
        .unwrap();
}

fn router(state: AppState, runtime: Arc<execution::Runtime>) -> Router {
    Router::new()
        .route(
            "/api/v1/llm/gateway/v1/chat/completions",
            post(
                move |State(state): State<AppState>,
                      auth: AuthUser,
                      headers: HeaderMap,
                      body: bytes::Bytes| {
                    let runtime = runtime.clone();
                    async move {
                        let value = serde_json::from_slice(&body).unwrap();
                        super::super::platform_model_gateway::execute_with_runtime(
                            state,
                            auth,
                            "chat/completions".into(),
                            Method::POST,
                            None,
                            headers,
                            body,
                            value,
                            CancellationToken::new(),
                            runtime,
                        )
                        .await
                    }
                },
            ),
        )
        .route(
            "/api/v1/llm/gateway/v1/{*path}",
            get(crate::handlers::llm_gateway::gateway_request),
        )
        .with_state(state)
}
fn request(
    credential: &credentials::AssistantCredential,
    row: &AssistantConversation,
    model: &str,
    occurrence: &str,
) -> Request<Body> {
    Request::post("/api/v1/llm/gateway/v1/chat/completions")
        .header(
            "authorization",
            format!("Bearer {}", credential.raw_key.as_str()),
        )
        .header(
            adapter::CONTEXT_HEADER,
            row.active_turn
                .as_ref()
                .unwrap()
                .platform_model_context
                .as_ref()
                .unwrap(),
        )
        .header(adapter::OCCURRENCE_HEADER, occurrence)
        .header("content-type", "application/json")
        .body(Body::from(
            json!({"model":model,"messages":[{"role":"user","content":"hello"}],"max_tokens":128})
                .to_string(),
        ))
        .unwrap()
}
async fn provider(expected: u64) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_raw(Native::Responses.fixture(), "application/json"),
        )
        .expect(expected)
        .mount(&server)
        .await;
    server
}
#[tokio::test]
async fn exact_thread_key_requires_paired_bridge_and_bills_only_the_actor() {
    let (state, plan, row, key) = setup().await;
    let server = provider(1).await;
    let app = router(
        state.clone(),
        Arc::new(execution::Runtime::mocked(&server.uri())),
    );
    let response = app
        .clone()
        .oneshot(request(
            &key,
            &row,
            &plan.model.public_id,
            &uuid::Uuid::new_v4().to_string(),
        ))
        .await
        .unwrap();
    assert!(
        !response.status().is_success(),
        "unverified bridge must not infer"
    );
    adapter::activate_bridge(&state.db, &row, &capabilities())
        .await
        .unwrap();
    let response = app
        .oneshot(request(
            &key,
            &row,
            &plan.model.public_id,
            &uuid::Uuid::new_v4().to_string(),
        ))
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 65536)
        .await
        .unwrap();
    assert!(
        status.is_success(),
        "{status}: {}",
        String::from_utf8_lossy(&bytes)
    );
    let value: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(value["model"], plan.model.public_id);
    assert!(!String::from_utf8_lossy(&bytes).contains("private"));
    let usage = state
        .db
        .collection::<Document>("usage_meter")
        .find_one(doc! {"service_id":&plan.offering.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(usage.get_str("billing_owner_id").unwrap(), PERSON);
    assert_eq!(
        state
            .db
            .collection::<Document>("approval_requests")
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        state
            .db
            .collection::<Document>("assistant_acknowledgements")
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    state.db.drop().await.unwrap();
}
#[tokio::test]
async fn twenty_duplicate_occurrences_make_one_provider_attempt() {
    let (state, plan, row, key) = setup().await;
    adapter::activate_bridge(&state.db, &row, &capabilities())
        .await
        .unwrap();
    let server = provider(1).await;
    let app = router(
        state.clone(),
        Arc::new(execution::Runtime::mocked(&server.uri())),
    );
    let occurrence = uuid::Uuid::new_v4().to_string();
    let responses = futures::future::join_all((0..20).map(|_| {
        app.clone()
            .oneshot(request(&key, &row, &plan.model.public_id, &occurrence))
    }))
    .await;
    let successes = responses
        .into_iter()
        .filter(|r| r.as_ref().unwrap().status().is_success())
        .count();
    assert_eq!(successes, 1);
    assert_eq!(
        state
            .db
            .collection::<Document>("assistant_service_execution")
            .count_documents(doc! {"kind":"platform_model_occurrence"})
            .await
            .unwrap(),
        1
    );
    state.db.drop().await.unwrap();
}
#[tokio::test]
async fn stopped_turn_wrong_key_and_revoked_entitlement_make_no_provider_calls() {
    let (state, plan, row, key) = setup().await;
    adapter::activate_bridge(&state.db, &row, &capabilities())
        .await
        .unwrap();
    let server = provider(0).await;
    let app = router(
        state.clone(),
        Arc::new(execution::Runtime::mocked(&server.uri())),
    );
    let second = engine::begin_turn(
        &state.db,
        PERSON,
        &start(engine::DEFAULT_MODEL),
        &state.encryption_keys,
    )
    .await
    .unwrap();
    let other =
        credentials::load_for_conversation(&state.db, &state.encryption_keys, PERSON, &second.id)
            .await
            .unwrap()
            .expect("admitted conversation credential");
    assert!(
        !app.clone()
            .oneshot(request(
                &other,
                &row,
                &plan.model.public_id,
                &uuid::Uuid::new_v4().to_string()
            ))
            .await
            .unwrap()
            .status()
            .is_success()
    );
    state
        .db
        .collection::<Document>("assistant_conversations")
        .update_one(
            doc! {"_id":&row.id},
            doc! {"$set":{"active_turn.stop_requested":true}},
        )
        .await
        .unwrap();
    assert!(
        !app.clone()
            .oneshot(request(
                &key,
                &row,
                &plan.model.public_id,
                &uuid::Uuid::new_v4().to_string()
            ))
            .await
            .unwrap()
            .status()
            .is_success()
    );
    state
        .db
        .collection::<Document>("assistant_conversations")
        .update_one(
            doc! {"_id":&row.id},
            doc! {"$set":{"active_turn.stop_requested":false}},
        )
        .await
        .unwrap();
    state
        .db
        .collection::<Document>(crate::models::platform_model::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&plan.model.id},
            doc! {"$set":{"entitlements.all_people":false}},
        )
        .await
        .unwrap();
    assert!(
        !app.oneshot(request(
            &key,
            &row,
            &plan.model.public_id,
            &uuid::Uuid::new_v4().to_string()
        ))
        .await
        .unwrap()
        .status()
        .is_success()
    );
    assert_eq!(
        adapter::public_selection(&state.db, PERSON, &plan.model.public_id)
            .await
            .unwrap(),
        ""
    );
    assert_eq!(
        state
            .db
            .collection::<Document>("platform_model_attempts")
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    state.db.drop().await.unwrap();
}
#[tokio::test]
async fn guests_and_unready_replicas_refuse_platform_admission() {
    let (state, plan, row, key) = setup().await;
    let agent = crate::services::assistant_team_service::ensure_nyxbot(&state.db, PERSON)
        .await
        .unwrap();
    assert!(matches!(
        adapter::validate_selection(&state.db, PERSON, &agent, &plan.model.public_id, true).await,
        Err(AppError::AssistantModelUnavailable)
    ));
    let mut bad = capabilities();
    bad["platform_models"]["image_input"] = json!(false);
    assert!(
        adapter::activate_bridge(&state.db, &row, &bad)
            .await
            .is_err()
    );
    adapter::activate_bridge(&state.db, &row, &capabilities())
        .await
        .unwrap();
    state
        .db
        .collection::<Document>("assistant_conversations")
        .update_one(doc! {"_id":&row.id}, doc! {"$set":{"guest_turn":true}})
        .await
        .unwrap();
    let server = provider(0).await;
    let app = router(
        state.clone(),
        Arc::new(execution::Runtime::mocked(&server.uri())),
    );
    assert!(
        !app.oneshot(request(
            &key,
            &row,
            &plan.model.public_id,
            &uuid::Uuid::new_v4().to_string()
        ))
        .await
        .unwrap()
        .status()
        .is_success()
    );
    assert_eq!(
        state
            .db
            .collection::<Document>("usage_meter")
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    state
        .db
        .collection::<Document>("feature_flag_overrides")
        .update_many(
            doc! {"flag_key":registry::FLAG},
            doc! {"$set":{"enabled":false}},
        )
        .await
        .unwrap();
    assert!(
        adapter::validate_selection(&state.db, PERSON, &agent, &plan.model.public_id, false)
            .await
            .is_err()
    );
    assert!(
        adapter::validate_selection(&state.db, PERSON, &agent, engine::DEFAULT_MODEL, false)
            .await
            .is_ok()
    );
    state.db.drop().await.unwrap();
}
#[test]
fn bridge_capabilities_are_closed_and_image_bounds_only_narrow() {
    assert!(adapter::bridge_ready(&capabilities()));
    for field in [
        "text",
        "function_tools",
        "image_input",
        "streaming",
        "compaction",
    ] {
        let mut cap = capabilities();
        cap["platform_models"][field] = json!(false);
        assert!(!adapter::bridge_ready(&cap));
    }
    let mut bad = capabilities();
    bad["platform_models"]["version"] = json!(2);
    assert!(!adapter::bridge_ready(&bad));
    assert!(
        adapter::image_capabilities(json!({"input_image":{"max_images":5}}), None)["input_image"]
            .is_null()
    );
}

#[tokio::test]
async fn runtime_contract_bridge_uses_exact_thread_key_and_public_manifest() {
    let (state, _, row, key) = setup().await;
    let selected = adapter::selection_plan(&state, PERSON, &row.model)
        .await
        .expect("selection must respect the certified native output minimum");
    assert_eq!(selected.model.public_id, row.model);
    assert_eq!(
        state
            .db
            .collection::<Document>("platform_model_attempts")
            .count_documents(doc! {})
            .await
            .unwrap(),
        0,
        "read-only selection never dispatches a provider request"
    );
    let server = provider(2).await;
    let app = router(
        state.clone(),
        Arc::new(execution::Runtime::mocked(&server.uri())),
    );
    let seen = Arc::new(tokio::sync::Mutex::new(Vec::<Value>::new()));
    let sink = seen.clone();
    let callback = app.clone();
    let expected_key = key.raw_key.clone();
    let outer=Router::new().route("/v1/capabilities",get(|| async {Json(capabilities())}))
        .route("/v1/responses",post(move |headers:HeaderMap,Json(body):Json<Value>| {
            let app=callback.clone();let seen=sink.clone();let expected=expected_key.clone();
            async move {
                assert_eq!(headers["authorization"],format!("Bearer {}",expected.as_str()));
                let handle=body["nyxid_model_context"]["handle"].as_str().unwrap();
                let manifest=app.clone().oneshot(Request::get("/api/v1/llm/gateway/v1/assistant-model-context")
                    .header("authorization",headers["authorization"].clone()).header(adapter::CONTEXT_HEADER,handle)
                    .body(Body::empty()).unwrap()).await.unwrap();
                assert!(manifest.status().is_success());
                let manifest:Value=serde_json::from_slice(&axum::body::to_bytes(manifest.into_body(),16384).await.unwrap()).unwrap();
                assert_eq!(manifest["model"],body["model"]);
                assert_eq!(manifest["protocol"],adapter::PROTOCOL);
                assert!(!manifest.to_string().contains("private"));
                assert!(manifest.get("actor").is_none());
                // The bridge makes distinct calls for a model step and a bounded
                // compaction step, each with the same server context and key.
                for content in ["Answer this turn", "Compact the retained public history"] {
                    let callback=app.clone().oneshot(Request::post("/api/v1/llm/gateway/v1/chat/completions")
                        .header("authorization",headers["authorization"].clone())
                        .header(adapter::CONTEXT_HEADER,handle).header(adapter::OCCURRENCE_HEADER,uuid::Uuid::new_v4().to_string())
                        .header("content-type","application/json").body(Body::from(json!({"model":manifest["model"],
                            "messages":[{"role":"user","content":content}],"max_tokens":128}).to_string())).unwrap()).await.unwrap();
                    let status=callback.status();let bytes=axum::body::to_bytes(callback.into_body(),65536).await.unwrap();
                    assert!(status.is_success(),"{status}: {}",String::from_utf8_lossy(&bytes));
                    seen.lock().await.push(serde_json::from_slice(&bytes).unwrap());
                }
                let sid=uuid::Uuid::new_v4().simple().to_string();
                let completed=json!({"type":"response.completed","sequence_number":1,"response":{
                    "id":format!("resp_{sid}_{}",uuid::Uuid::new_v4().simple()),"conversation":{"id":format!("conv_{sid}")},
                    "status":"completed","output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Bridge completed"}]}]}});
                ([("content-type","text/event-stream")],format!("data: {completed}\n\n"))
            }
        }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let task = tokio::spawn(async move { axum::serve(listener, outer).await.unwrap() });
    state
        .db
        .collection::<Document>("downstream_services")
        .update_one(
            doc! {"slug":engine::SERVICE_SLUG},
            doc! {"$set":{"base_url":format!("http://{address}")}},
        )
        .await
        .unwrap();
    let (sender, _) = tokio::sync::broadcast::channel(256);
    let permit = state.direct_chat_limiter.try_acquire(PERSON).await.unwrap();
    super::super::assistant_nyxagent::run_turn(
        state.clone(),
        test_utils::test_auth_user(PERSON),
        row.clone(),
        "Tell me something".into(),
        key,
        Some(super::super::assistant_nyxagent::SERVER_TURN_POLICY),
        permit,
        crate::services::assistant_nyxagent_runtime::Events {
            checkpoint: None,
            sender,
            cursor: 0,
            reset_notified: false,
        },
    )
    .await;
    let messages = engine::messages(&state.db, PERSON, &row.id, 100, None)
        .await
        .unwrap();
    let failures = messages
        .iter()
        .filter(|m| m.error_code.is_some())
        .map(|m| json!({"code":m.error_code,"detail":m.error_detail}))
        .collect::<Vec<_>>();
    assert_eq!(seen.lock().await.len(), 2, "{failures:?}");
    assert!(
        messages.iter().any(|m| m.text == "Bridge completed"),
        "{messages:?}"
    );
    assert!(
        engine::get(&state.db, PERSON, &row.id)
            .await
            .unwrap()
            .active_turn
            .is_none()
    );
    task.abort();
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn org_shared_and_collaboration_specialists_keep_original_person_and_live_authority() {
    use crate::models::{
        assistant_agent::GuestAccess,
        assistant_sharing::{ShareRole, ShareSubject},
    };
    use crate::services::{
        assistant_team_service as team, feature_flag_service as flags, org_agent_tests::Fixture,
        shared_agent_service as sharing,
    };
    for lane in ["org", "shared", "collaboration"] {
        let shared = lane != "org";
        let (state, plan, _, _) = setup().await;
        let f = Fixture::with_db(state.db.clone()).await;
        let owner = if shared { &f.admin } else { &f.org };
        flags::set_platform_override(
            &state.db,
            "assistant:org-agents",
            &flags::FlagTarget::Global,
            true,
            &f.admin,
        )
        .await
        .unwrap();
        let mut create = f.request("model-specialist");
        create.targets.platform_service_ids = vec![plan.offering.id.clone()];
        let (agent, _) =
            team::create_specialist_for(&state.db, &state.encryption_keys, &f.admin, owner, create)
                .await
                .unwrap()
                .unwrap();
        state
            .db
            .collection::<Document>("assistant_agents")
            .update_one(
                doc! {"_id":&agent.id},
                doc! {"$set":{"model":&plan.model.public_id}},
            )
            .await
            .unwrap();
        if shared {
            sharing::ensure_indexes(&state.db).await.unwrap();
            flags::set_platform_override(
                &state.db,
                sharing::FLAG,
                &flags::FlagTarget::Global,
                true,
                &f.admin,
            )
            .await
            .unwrap();
            sharing::update(
                &state.db,
                &f.admin,
                &agent.id,
                sharing::SharingUpdate {
                    expected_version: 0,
                    enabled: true,
                    audience: vec![sharing::Audience {
                        subject_kind: ShareSubject::Person,
                        subject_id: format!("email:{}@example.com", f.member),
                        role: ShareRole::User,
                    }],
                    shared_access: std::collections::BTreeMap::from([(
                        plan.offering.id.clone(),
                        GuestAccess::Read,
                    )]),
                },
                true,
            )
            .await
            .unwrap();
        }
        let mut input = start(&plan.model.public_id);
        input.agent_id = Some(agent.id.clone());
        let row = if lane == "collaboration" {
            use crate::services::collaboration_group_service as groups;
            groups::ensure_indexes(&state.db).await.unwrap();
            flags::set_platform_override(
                &state.db,
                flags::NYXAGENT_ENGINE_FLAG_KEY,
                &flags::FlagTarget::Global,
                true,
                &f.admin,
            )
            .await
            .unwrap();
            let group = groups::membership::create(
                &state.db,
                &f.member,
                "Model group",
                std::slice::from_ref(&agent.id),
            )
            .await
            .unwrap();
            let request = groups::requests::enqueue_with_uploads(
                &state.db,
                &f.member,
                &group.id,
                "Think",
                Some(std::slice::from_ref(&agent.id)),
                &[],
            )
            .await
            .unwrap();
            let begun = groups::runtime::begin(
                &state.db,
                &state.encryption_keys,
                &f.member,
                &group.id,
                &request.id,
                &agent.id,
            )
            .await
            .unwrap();
            begun.claim.conversation
        } else {
            engine::begin_turn(&state.db, &f.member, &input, &state.encryption_keys)
                .await
                .unwrap()
        };
        assert_eq!(row.user_id, f.member);
        assert_eq!(row.agent_owner_id.as_deref(), Some(owner.as_str()));
        let key = credentials::load_for_conversation(
            &state.db,
            &state.encryption_keys,
            &f.member,
            &row.id,
        )
        .await
        .unwrap()
        .expect("admitted conversation credential");
        adapter::activate_bridge(&state.db, &row, &capabilities())
            .await
            .unwrap();
        let server = provider(1).await;
        let app = router(
            state.clone(),
            Arc::new(execution::Runtime::mocked(&server.uri())),
        );
        let response = app
            .clone()
            .oneshot(request(
                &key,
                &row,
                &plan.model.public_id,
                &uuid::Uuid::new_v4().to_string(),
            ))
            .await
            .unwrap();
        let status = response.status();
        let body = axum::body::to_bytes(response.into_body(), 65536)
            .await
            .unwrap();
        assert!(
            status.is_success(),
            "{status}: {}",
            String::from_utf8_lossy(&body)
        );
        let usage = state
            .db
            .collection::<Document>("usage_meter")
            .find_one(doc! {"service_id":&plan.offering.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(usage.get_str("billing_owner_id").unwrap(), f.member);
        if shared {
            sharing::update(
                &state.db,
                &f.admin,
                &agent.id,
                sharing::SharingUpdate {
                    expected_version: 1,
                    enabled: false,
                    audience: vec![],
                    shared_access: Default::default(),
                },
                false,
            )
            .await
            .unwrap();
        } else {
            f.revoke(&f.member).await;
        }
        assert!(
            !app.oneshot(request(
                &key,
                &row,
                &plan.model.public_id,
                &uuid::Uuid::new_v4().to_string()
            ))
            .await
            .unwrap()
            .status()
            .is_success()
        );
        state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn image_input_requires_this_turn_read_and_live_retention() {
    use crate::services::{
        assistant_acknowledgement_service as acks, assistant_team_service as team,
        assistant_upload_service as uploads,
    };
    use base64::Engine;
    let (state, mut plan, _, _) = setup().await;
    let mut image_request = execution::tests::images_tests::image_plan(&mut plan, false);
    state
        .db
        .collection::<crate::models::platform_model::PlatformModel>(
            crate::models::platform_model::COLLECTION_NAME,
        )
        .replace_one(doc! {"_id":&plan.model.id}, &plan.model)
        .await
        .unwrap();
    state
        .db
        .collection::<crate::models::platform_model_tariff::ModelTariff>(
            crate::models::platform_model_tariff::COLLECTION_NAME,
        )
        .replace_one(doc! {"_id":&plan.tariff.tariff().id}, plan.tariff.tariff())
        .await
        .unwrap();
    let agent = team::ensure_nyxbot(&state.db, PERSON).await.unwrap();
    let mut session = state.db.client().start_session().await.unwrap();
    session.start_transaction().await.unwrap();
    let home = team::create_thread_for(
        &state.db,
        &state.encryption_keys,
        PERSON,
        &agent,
        "Image test",
        &mut session,
    )
    .await
    .unwrap();
    session.commit_transaction().await.unwrap();
    let mut bytes = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgb8(image::RgbImage::new(3, 2))
        .write_to(&mut bytes, image::ImageFormat::Png)
        .unwrap();
    let bytes = bytes.into_inner();
    let upload = uploads::upload(
        &state.db,
        &state.encryption_keys,
        PERSON,
        &home.id,
        "photo.png",
        bytes.clone(),
    )
    .await
    .unwrap();
    engine::change_model(&state.db, PERSON, &home.id, &plan.model.public_id)
        .await
        .unwrap();
    let mut input = start(&plan.model.public_id);
    input.conversation_id = Some(home.id);
    input.model = None;
    input.attachment_ids = vec![upload.id.clone()];
    let row = engine::begin_turn(&state.db, PERSON, &input, &state.encryption_keys)
        .await
        .unwrap();
    let key =
        credentials::load_for_conversation(&state.db, &state.encryption_keys, PERSON, &row.id)
            .await
            .unwrap()
            .expect("admitted conversation credential");
    adapter::activate_bridge(&state.db, &row, &capabilities())
        .await
        .unwrap();
    let handle = row
        .active_turn
        .as_ref()
        .unwrap()
        .platform_model_context
        .as_ref()
        .unwrap();
    let chat = acks::for_key(&state.db, PERSON, Some(&key.api_key_id))
        .await
        .unwrap()
        .unwrap();
    let uri = format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(&bytes)
    );
    image_request.body = json!({"model":plan.model.public_id,"max_tokens":128,"messages":[{"role":"user","content":[{"type":"image_url","image_url":{"url":uri}}]}]});
    let digests = adapter::image_digests(&image_request.body).unwrap();
    assert!(
        adapter::check_images(&state.db, &chat, handle, &digests)
            .await
            .is_err(),
        "No provenance before the authorized download"
    );
    assert_eq!(
        uploads::chat_bytes(&state.db, &state.encryption_keys, &chat, &upload.id, true)
            .await
            .unwrap()
            .1,
        bytes
    );
    adapter::check_images(&state.db, &chat, handle, &digests)
        .await
        .unwrap();
    assert!(
        adapter::check_images(
            &state.db,
            &chat,
            &uuid::Uuid::new_v4().to_string(),
            &digests
        )
        .await
        .is_err()
    );
    let server = provider(1).await;
    let app = router(
        state.clone(),
        Arc::new(execution::Runtime::mocked(&server.uri())),
    );
    let send = || {
        let base = request(
            &key,
            &row,
            &plan.model.public_id,
            &uuid::Uuid::new_v4().to_string(),
        );
        let (parts, _) = base.into_parts();
        Request::from_parts(parts, Body::from(image_request.body.to_string()))
    };
    let response = app.clone().oneshot(send()).await.unwrap();
    let status = response.status();
    let body = axum::body::to_bytes(response.into_body(), 65536)
        .await
        .unwrap();
    assert!(
        status.is_success(),
        "{status}: {}",
        String::from_utf8_lossy(&body)
    );
    let sent = server.received_requests().await.unwrap();
    assert!(!sent[0].headers.contains_key(adapter::CONTEXT_HEADER));
    assert!(
        sent[0]
            .body_json::<Value>()
            .unwrap()
            .to_string()
            .contains("data:image/png;base64,")
    );
    // Bound uploads use current policy and bound_at; expires_at is only a
    // legacy fallback and must not be mistaken for the current retention fence.
    let updated = state.db.collection::<Document>(crate::models::assistant_upload::COLLECTION_NAME)
        .update_one(doc! {"_id":&upload.id}, doc! {"$set":{
            "bound_at":bson::DateTime::from_chrono(chrono::Utc::now()-chrono::Duration::days(31))
        }}).await.unwrap();
    assert_eq!(updated.matched_count, 1);
    assert!(matches!(
        uploads::for_egress(&state.db, &chat, &upload.id).await,
        Err(AppError::AssistantAttachmentExpired)
    ));
    assert!(!app.oneshot(send()).await.unwrap().status().is_success());
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn discovery_selection_and_model_grants_share_live_public_entitlements() {
    use crate::services::assistant_team_service as team;
    let (state, plan, _, _) = setup().await;
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/capabilities"))
        .respond_with(ResponseTemplate::new(200).set_body_json(capabilities()))
        .mount(&server)
        .await;
    state
        .db
        .collection::<Document>("downstream_services")
        .update_one(
            doc! {"slug":engine::SERVICE_SLUG},
            doc! {"$set":{"base_url":server.uri()}},
        )
        .await
        .unwrap();
    let Json(list) = list(
        State(state.clone()),
        test_utils::test_auth_user(PERSON),
        Query(ListQuery {
            after: None,
            limit: Some(1),
            agent_id: None,
            conversation_id: None,
        }),
    )
    .await
    .unwrap();
    let public = serde_json::to_value(&list).unwrap();
    assert_eq!(public["models"][0]["id"], plan.model.public_id);
    assert!(public["models"][0]["retail_prices"].is_array());
    for private in [
        "credential_service_id",
        "routing_revision_id",
        "private-project",
        "private-request",
    ] {
        assert!(!public.to_string().contains(private));
    }
    let targets = team::resolve_targets(
        &state.db,
        &state.node_ws_manager,
        PERSON,
        PERSON,
        std::slice::from_ref(&plan.model.public_id),
    )
    .await
    .unwrap();
    assert_eq!(
        targets.platform_service_ids.as_slice(),
        std::slice::from_ref(&plan.offering.id)
    );
    let nyxbot = team::ensure_nyxbot(&state.db, PERSON).await.unwrap();
    let mut specialist = nyxbot.clone();
    specialist.kind = crate::models::assistant_agent::AgentKind::Specialist;
    assert!(
        adapter::validate_selection(&state.db, PERSON, &specialist, &plan.model.public_id, false)
            .await
            .is_err()
    );
    specialist
        .grants
        .platform_service_ids
        .push(plan.offering.id.clone());
    adapter::validate_selection(&state.db, PERSON, &specialist, &plan.model.public_id, false)
        .await
        .unwrap();
    state
        .db
        .collection::<Document>(crate::models::platform_model::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&plan.model.id},
            doc! {"$set":{"entitlements.all_people":false}},
        )
        .await
        .unwrap();
    assert!(
        adapter::validate_selection(&state.db, PERSON, &specialist, &plan.model.public_id, false)
            .await
            .is_err()
    );
    assert_eq!(
        adapter::public_selection(&state.db, PERSON, &plan.model.public_id)
            .await
            .unwrap(),
        ""
    );
    specialist.model = plan.model.public_id.clone();
    let summary = team::summaries_for_agents(&state.db, PERSON, vec![specialist.clone()], 0)
        .await
        .unwrap();
    assert!(summary[0].model_unavailable);
    assert!(summary[0].model.is_empty());
    assert!(summary[0].services.is_empty());
    state
        .db
        .collection::<Document>(crate::models::platform_model::COLLECTION_NAME)
        .delete_one(doc! {"_id":&plan.model.id})
        .await
        .unwrap();
    let summary = team::summaries_for_agents(&state.db, PERSON, vec![specialist], 0)
        .await
        .unwrap();
    assert!(
        summary[0].services.is_empty(),
        "deleted models cannot expose internal offering slugs"
    );
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn lost_provider_result_never_reopens_an_occurrence_and_narrowed_scopes_refuse() {
    use crate::models::agent_operation_scope::{AgentOperationScope, OperationScopes};
    let (state, plan, row, key) = setup().await;
    adapter::activate_bridge(&state.db, &row, &capabilities())
        .await
        .unwrap();
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(
            ResponseTemplate::new(200).set_body_raw("broken response", "application/json"),
        )
        .expect(1)
        .mount(&server)
        .await;
    let app = router(
        state.clone(),
        Arc::new(execution::Runtime::mocked(&server.uri())),
    );
    let occurrence = uuid::Uuid::new_v4().to_string();
    assert!(
        !app.clone()
            .oneshot(request(&key, &row, &plan.model.public_id, &occurrence))
            .await
            .unwrap()
            .status()
            .is_success()
    );
    assert_eq!(
        app.clone()
            .oneshot(request(
                &key,
                &row,
                &plan.model.public_id,
                &occurrence.to_uppercase()
            ))
            .await
            .unwrap()
            .status(),
        http::StatusCode::CONFLICT
    );
    let scopes = OperationScopes::from([(
        plan.offering.id.clone(),
        AgentOperationScope {
            revision: 1,
            catalog_service_id: Some(plan.offering.id.clone()),
            operations: vec![],
        },
    )]);
    state
        .db
        .collection::<Document>("assistant_agents")
        .update_one(
            doc! {"_id":row.agent_id.as_ref().unwrap()},
            doc! {"$set":{"operation_scopes":bson::to_bson(&scopes).unwrap()}},
        )
        .await
        .unwrap();
    assert!(
        !app.oneshot(request(
            &key,
            &row,
            &plan.model.public_id,
            &uuid::Uuid::new_v4().to_string()
        ))
        .await
        .unwrap()
        .status()
        .is_success()
    );
    assert_eq!(
        state
            .db
            .collection::<Document>("approval_requests")
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn invocation_selection_rechecks_entitlements_and_cancellation_fences_callbacks() {
    use crate::services::assistant_invocation_service as invocations;
    let (state, plan, _, _) = setup().await;
    invocations::ensure_indexes(&state.db).await.unwrap();
    let input = engine::TurnStart::from(&start(&plan.model.public_id));
    let occurrence = uuid::Uuid::new_v4().to_string();
    let admitted = invocations::admit(
        &state.db,
        PERSON,
        &occurrence,
        "request-digest",
        &input,
        &state.encryption_keys,
    )
    .await
    .unwrap();
    let claim = admitted.claim.unwrap();
    let row = claim.conversation;
    assert!(
        row.active_turn
            .as_ref()
            .unwrap()
            .platform_model_context
            .is_some()
    );
    adapter::activate_bridge(&state.db, &row, &capabilities())
        .await
        .unwrap();
    let server = provider(0).await;
    let app = router(
        state.clone(),
        Arc::new(execution::Runtime::mocked(&server.uri())),
    );
    invocations::cancel(&state, PERSON, &occurrence)
        .await
        .unwrap();
    assert!(
        !app.oneshot(request(
            &claim.credential,
            &row,
            &plan.model.public_id,
            &uuid::Uuid::new_v4().to_string()
        ))
        .await
        .unwrap()
        .status()
        .is_success()
    );
    state
        .db
        .collection::<Document>(crate::models::platform_model::COLLECTION_NAME)
        .update_one(doc! {"_id":&plan.model.id}, doc! {"$set":{"enabled":false}})
        .await
        .unwrap();
    assert!(
        invocations::admit(
            &state.db,
            PERSON,
            &uuid::Uuid::new_v4().to_string(),
            "next-digest",
            &input,
            &state.encryption_keys
        )
        .await
        .is_err()
    );
    assert_eq!(
        state
            .db
            .collection::<Document>(crate::models::assistant_invocation::COLLECTION_NAME)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1,
        "denied admission must roll back its invocation"
    );
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn existing_personal_approval_and_explicit_deny_are_not_bypassed() {
    let (state, plan, row, key) = setup().await;
    adapter::activate_bridge(&state.db, &row, &capabilities())
        .await
        .unwrap();
    set_person_approvals(&state, PERSON, true).await;
    let server = provider(0).await;
    let app = router(
        state.clone(),
        Arc::new(execution::Runtime::mocked(&server.uri())),
    );
    let response = app
        .clone()
        .oneshot(request(
            &key,
            &row,
            &plan.model.public_id,
            &uuid::Uuid::new_v4().to_string(),
        ))
        .await
        .unwrap();
    assert!(!response.status().is_success());
    let approval = state
        .db
        .collection::<Document>("approval_requests")
        .find_one(doc! {"service_id":&plan.offering.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(approval.get_str("user_id").unwrap(), PERSON);
    assert_eq!(
        approval.get_str("service_name").unwrap(),
        plan.model.display_name
    );
    assert!(!approval.to_string().contains("private-project"));
    state.db.collection::<Document>("service_approval_configs").insert_one(doc! {
        "_id":uuid::Uuid::new_v4().to_string(), "user_id":PERSON,"service_id":&plan.offering.id,
        "service_name":"Model","approval_required":false,"approval_mode":"per_request",
        "rules":[],"default_effect":"deny","created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()
    }).await.unwrap();
    let before = state
        .db
        .collection::<Document>("approval_requests")
        .count_documents(doc! {})
        .await
        .unwrap();
    assert_eq!(
        app.oneshot(request(
            &key,
            &row,
            &plan.model.public_id,
            &uuid::Uuid::new_v4().to_string()
        ))
        .await
        .unwrap()
        .status(),
        http::StatusCode::FORBIDDEN
    );
    assert_eq!(
        state
            .db
            .collection::<Document>("approval_requests")
            .count_documents(doc! {})
            .await
            .unwrap(),
        before
    );
    assert_eq!(
        state
            .db
            .collection::<Document>("usage_meter")
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn runtime_without_image_capable_bridge_settles_before_any_inference() {
    let (state, _, row, key) = setup().await;
    let server = MockServer::start().await;
    let mut cap = capabilities();
    cap["platform_models"]["image_input"] = json!(false);
    Mock::given(method("GET"))
        .and(path("/v1/capabilities"))
        .respond_with(ResponseTemplate::new(200).set_body_json(cap))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&server)
        .await;
    state
        .db
        .collection::<Document>("downstream_services")
        .update_one(
            doc! {"slug":engine::SERVICE_SLUG},
            doc! {"$set":{"base_url":server.uri()}},
        )
        .await
        .unwrap();
    let (sender, _) = tokio::sync::broadcast::channel(256);
    let permit = state.direct_chat_limiter.try_acquire(PERSON).await.unwrap();
    super::super::assistant_nyxagent::run_turn(
        state.clone(),
        test_utils::test_auth_user(PERSON),
        row.clone(),
        "Try the selected model".into(),
        key,
        Some(super::super::assistant_nyxagent::SERVER_TURN_POLICY),
        permit,
        crate::services::assistant_nyxagent_runtime::Events {
            checkpoint: None,
            sender,
            cursor: 0,
            reset_notified: false,
        },
    )
    .await;
    let messages = engine::messages(&state.db, PERSON, &row.id, 100, None)
        .await
        .unwrap();
    assert!(
        messages
            .iter()
            .any(|m| m.error_code.as_deref() == Some("model_unavailable"))
    );
    assert!(
        engine::get(&state.db, PERSON, &row.id)
            .await
            .unwrap()
            .active_turn
            .is_none()
    );
    assert_eq!(
        state
            .db
            .collection::<Document>("platform_model_attempts")
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn discovery_degrades_for_unready_and_unknown_fleets_without_authorizing_execution() {
    let (state, mut plan, row, key) = setup().await;
    let _image_request = execution::tests::images_tests::image_plan(&mut plan, false);
    state
        .db
        .collection::<crate::models::platform_model::PlatformModel>(
            crate::models::platform_model::COLLECTION_NAME,
        )
        .replace_one(doc! {"_id":&plan.model.id}, &plan.model)
        .await
        .unwrap();
    state
        .db
        .collection::<crate::models::platform_model_tariff::ModelTariff>(
            crate::models::platform_model_tariff::COLLECTION_NAME,
        )
        .replace_one(doc! {"_id":&plan.tariff.tariff().id}, plan.tariff.tariff())
        .await
        .unwrap();
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/capabilities"))
        .respond_with(ResponseTemplate::new(200).set_body_json(capabilities()))
        .expect(1)
        .mount(&server)
        .await;
    state
        .db
        .collection::<Document>("downstream_services")
        .update_one(
            doc! {"slug":engine::SERVICE_SLUG},
            doc! {"$set":{"base_url":server.uri()}},
        )
        .await
        .unwrap();
    let query = || {
        Query(ListQuery {
            after: None,
            limit: Some(10),
            agent_id: None,
            conversation_id: None,
        })
    };
    let Json(ready) = list(
        State(state.clone()),
        test_utils::test_auth_user(PERSON),
        query(),
    )
    .await
    .unwrap();
    assert_eq!(ready.models.len(), 1);
    assert_eq!(ready.models[0].availability, "request_dependent");
    assert!(
        ready.models[0]
            .retail_prices
            .iter()
            .any(|p| p.image_input.is_some())
    );

    let initial_usage = state
        .db
        .collection::<Document>("usage_meter")
        .count_documents(doc! {})
        .await
        .unwrap();
    adapter::activate_bridge(&state.db, &row, &capabilities())
        .await
        .unwrap();
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&server)
        .await;
    let app = router(
        state.clone(),
        Arc::new(execution::Runtime::mocked(&server.uri())),
    );
    let replicas = state
        .db
        .collection::<Document>(crate::models::security_replica::COLLECTION_NAME);
    let template = replicas.find_one(doc! {}).await.unwrap().unwrap();
    for unknown in [false, true] {
        if unknown {
            // The bounded real readiness reader returns the same typed error
            // as an unconfigured runtime. Do not mutate global test runtime.
            let rows = (0..257)
                .map(|_| {
                    let mut row = template.clone();
                    row.insert("_id", uuid::Uuid::new_v4().to_string());
                    row.insert("observed_at", bson::DateTime::now());
                    row
                })
                .collect::<Vec<_>>();
            replicas.insert_many(rows).await.unwrap();
        } else {
            replicas
                .update_many(doc! {}, doc! {"$pull":{"capabilities":adapter::READINESS}})
                .await
                .unwrap();
        }
        assert!(matches!(
            adapter::require_ready(&state.db).await,
            Err(AppError::SecurityReadinessUnavailable)
        ));
        let Json(result) = list(
            State(state.clone()),
            test_utils::test_auth_user(PERSON),
            query(),
        )
        .await
        .unwrap();
        assert_eq!(result.models.len(), 1);
        let model = &result.models[0];
        assert_eq!(model.id, plan.model.public_id);
        assert_eq!(model.availability, "unavailable");
        assert!(!model.retail_prices.is_empty());
        assert!(model.retail_prices.iter().all(|p| p.image_input.is_none()));
        let response = app
            .clone()
            .oneshot(request(
                &key,
                &row,
                &plan.model.public_id,
                &uuid::Uuid::new_v4().to_string(),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), http::StatusCode::SERVICE_UNAVAILABLE);
    }
    assert_eq!(
        state
            .db
            .collection::<Document>("usage_meter")
            .count_documents(doc! {})
            .await
            .unwrap(),
        initial_usage,
        "unready discovery and refused execution create no usage"
    );
    assert_eq!(
        state
            .db
            .collection::<Document>("platform_model_attempts")
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    state.db.drop().await.unwrap();
}

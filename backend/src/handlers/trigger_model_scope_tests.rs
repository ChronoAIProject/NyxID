//! Real trigger admission -> NyxAgent -> authenticated model proxy callbacks.
use super::*;
use crate::mw::auth::AuthUser;
use axum::{
    Extension,
    body::Body,
    extract::{Path, Query, State},
    http::{Request, StatusCode},
    response::Response,
    routing::any,
};
use tower::ServiceExt;

async fn callback(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((slug, path)): Path<(String, String)>,
    request: Request<Body>,
) -> AppResult<Response> {
    Box::pin(crate::handlers::proxy::proxy_request_by_slug_inner(
        &state,
        &auth,
        &slug,
        &path,
        request,
        &mut String::new(),
    ))
    .await
}

#[tokio::test]
async fn automation_model_2099_new_webhook_specialist_both_profiles() {
    for profile in ["chat", "channel"] {
        Box::pin(journey(false, profile, false, false)).await;
    }
}

#[tokio::test]
async fn automation_model_2099_new_webhook_nyxbot_both_profiles() {
    for profile in ["chat", "channel"] {
        Box::pin(journey(true, profile, false, false)).await;
    }
}

#[tokio::test]
async fn automation_model_2099_existing_schedule_keeps_model_authority() {
    Box::pin(journey(false, "chat", true, false)).await;
}

#[tokio::test]
async fn automation_model_2099_stateful_callback_keeps_closed_failure() {
    Box::pin(journey(false, "chat", false, true)).await;
}

async fn journey(nyxbot: bool, profile: &str, scheduled: bool, stateful: bool) {
    let (state, _, old_server) = setup("automation_model_2099").await;
    old_server.abort();
    let agent = if nyxbot {
        team::ensure_nyxbot(&state.db, OWNER).await.unwrap()
    } else {
        specialist(&state).await
    };
    state
        .db
        .collection::<Document>("assistant_agents")
        .update_one(
            doc! {"_id": &agent.id},
            doc! {"$set": {"model": format!("nyxagent/{profile}")}},
        )
        .await
        .unwrap();
    let provider = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/responses"))
        .respond_with(
            wiremock::ResponseTemplate::new(200)
                .set_body_json(json!({"id":"model-response","output":[]})),
        )
        .expect(if stateful { 0 } else { 1 })
        .mount(&provider)
        .await;
    let mut model = crate::models::downstream_service::test_helpers::dummy_service();
    model.id = Uuid::new_v4().to_string();
    model.slug = if profile == "chat" {
        "chrono-llm-public"
    } else {
        "llm-deepseek"
    }
    .into();
    model.base_url = provider.uri();
    model.auth_method = "none".into();
    model.requires_user_credential = false;
    model.credential_encrypted.clear();
    model.inference = Some(
        serde_json::from_value(json!({"wire_protocol":"openai_responses","model_list":true}))
            .unwrap(),
    );
    state
        .db
        .collection::<DownstreamService>(SERVICES)
        .insert_one(&model)
        .await
        .unwrap();
    let connection = crate::services::assistant_authority_tests::connected(
        &state.db,
        OWNER,
        &model.slug,
        &provider.uri(),
    )
    .await;
    state
        .db
        .collection::<Document>("user_services")
        .update_one(
            doc! {"_id": &connection},
            doc! {"$set":{"catalog_service_id": &model.id}},
        )
        .await
        .unwrap();
    assert_eq!(
        state
            .db
            .collection::<Document>("service_endpoints")
            .count_documents(doc! {"service_id":&model.id})
            .await
            .unwrap(),
        0
    );
    // A non-inference service remains outside a specialist's empty grants.
    let tool = crate::services::assistant_authority_tests::connected(
        &state.db,
        OWNER,
        "ungranted-tool",
        &provider.uri(),
    )
    .await;
    let callbacks = Router::new()
        .route("/api/v1/proxy/s/{slug}/{*path}", any(callback))
        .layer(Extension(
            crate::handlers::assistant_nyxagent::SERVER_TURN_POLICY,
        ))
        .with_state(state.clone());
    let observed = Arc::new(Mutex::new(Vec::<Value>::new()));
    let sink = observed.clone();
    let db = state.db.clone();
    let slug = model.slug.clone();
    let profile = profile.to_owned();
    let source = if profile == "chat" {
        include_str!("../../specs/assistant/codex/0.165.0-chat.json")
    } else {
        include_str!("../../specs/assistant/codex/0.165.0-channel.json")
    };
    let fixture: Value = serde_json::from_str(source).unwrap();
    let fake = Router::new().route("/v1/responses", post(move |headers: HeaderMap, Json(body): Json<Value>| {
        let callbacks = callbacks.clone();
        let db = db.clone();
        let sink = sink.clone();
        let slug = slug.clone();
        let profile = profile.clone();
        let connection = connection.clone();
        let tool = tool.clone();
        let mut generation = fixture.clone();
        async move {
            assert_eq!(body["model"], format!("nyxagent/{profile}"));
            let bearer = headers["authorization"].clone();
            let raw = bearer.to_str().unwrap().strip_prefix("Bearer ").unwrap();
            let (_, key, _) = crate::services::key_service::validate_api_key(&db, raw).await.unwrap();
            assert!(crate::services::assistant_agent_credential_service::is_conversation_key(&db, OWNER, &key.id).await.unwrap());
            let chat = crate::services::assistant_acknowledgement_service::for_key(&db, OWNER, Some(&key.id)).await.unwrap().unwrap();
            assert!(chat.turn_live && !chat.turn_stopped && !chat.guest);
            assert_eq!(chat.confirmation_policy.is_some(), !scheduled);
            if !nyxbot {
                assert!(!key.allow_all_services);
                assert!(!key.allowed_service_ids.contains(&connection));
                assert!(!key.allowed_service_ids.contains(&tool));
                assert!(key.allowed_platform_service_ids.is_empty());
                let refused = callbacks.clone().oneshot(Request::builder().method("GET")
                    .uri("/api/v1/proxy/s/ungranted-tool/data").header("authorization", bearer.clone())
                    .body(Body::empty()).unwrap()).await.unwrap();
                assert_eq!(refused.status(), StatusCode::FORBIDDEN);
            }
            generation["input"] = body["input"].clone();
            generation["stream"] = false.into();
            if !scheduled && !stateful {
                // Neither the inference catalog nor the thread key turns an
                // effectful/unknown request into thinking. Exercise the same
                // real proxy before the eventual valid callback.
                for (path, field, value, method_override) in [
                    ("responses", "store", json!(true), false),
                    ("responses", "previous_response_id", json!("response-handle"), false),
                    ("responses", "tools", json!([{"type":"computer_use_preview"}]), false),
                    ("responses", "model", json!(""), false),
                    ("responses", "stream", json!(false), true),
                    ("files", "stream", json!(false), false),
                ] {
                    let mut rejected = generation.clone();
                    rejected[field] = value;
                    let mut request = Request::builder().method("POST")
                        .uri(format!("/api/v1/proxy/s/{slug}/{path}"))
                        .header("authorization", bearer.clone())
                        .header("content-type", "application/json");
                    if method_override {
                        request = request.header("x-http-method-override", "DELETE");
                    }
                    let refusal = callbacks.clone().oneshot(request.body(Body::from(rejected.to_string())).unwrap()).await.unwrap();
                    assert_eq!(refusal.status(), StatusCode::FORBIDDEN, "{path}/{field}");
                }
                let write = callbacks.clone().oneshot(Request::builder().method("POST")
                    .uri("/api/v1/proxy/s/ungranted-tool/data").header("authorization", bearer.clone())
                    .header("content-type", "application/json").body(Body::from("{}"))
                    .unwrap()).await.unwrap();
                assert_eq!(write.status(), StatusCode::FORBIDDEN);
                if nyxbot {
                    let bytes = axum::body::to_bytes(write.into_body(), 4096).await.unwrap();
                    assert!(std::str::from_utf8(&bytes).unwrap().contains("Webhook service changes must use MCP"));
                }
            }
            if stateful { generation["store"] = true.into(); }
            let response = callbacks.oneshot(Request::builder().method("POST")
                .uri(format!("/api/v1/proxy/s/{slug}/responses"))
                .header("authorization", bearer).header("content-type", "application/json")
                .body(Body::from(generation.to_string())).unwrap()).await.unwrap();
            let status = response.status();
            let bytes = axum::body::to_bytes(response.into_body(), 1024*1024).await.unwrap();
            let result: Value = serde_json::from_slice(&bytes).unwrap();
            sink.lock().await.push(json!({"status":status.as_u16(),"result":result}));
            let failed = status != StatusCode::OK;
            let session = format!("conv_{}", Uuid::new_v4().simple());
            let response = format!("resp_{}_{}", &session[5..], Uuid::new_v4().simple());
            let event = json!({"type":if failed {"response.failed"} else {"response.completed"}, "sequence_number":1,
                "response":{"id":response,"conversation":{"id":session},"status":if failed {"failed"} else {"completed"},
                "error":if failed {json!({"code":"turn_failed","message":"untrusted provider message"})} else {Value::Null},
                "output":if failed {json!([])} else {json!([{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Work finished"}]}])}}});
            Sse::new(futures::stream::iter([Ok::<_,Infallible>(Event::default().data(event.to_string()))])).into_response()
        }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, fake).await.unwrap();
    });
    state
        .db
        .collection::<Document>(SERVICES)
        .update_one(
            doc! {"slug":engine::SERVICE_SLUG},
            doc! {"$set":{"base_url":format!("http://{address}")}},
        )
        .await
        .unwrap();
    let trigger = if scheduled {
        let fresh = team::agent(&state.db, OWNER, &agent.id).await.unwrap();
        team::home_thread(&state.db, &state.encryption_keys, &fresh)
            .await
            .unwrap();
        let trigger = automation(&state, &agent.id, Some(ThreadPolicy::Home)).await;
        schedules::enqueue_now(&state.db, &trigger).await.unwrap();
        trigger
    } else {
        let created = trigger_service::create(
            &state.db,
            &state.encryption_keys,
            trigger_service::CreateInput {
                source: TriggerSource::Webhook,
                setup_watch_id: None,
                schedule: None,
                overlap: OverlapPolicy::Skip,
                user_id: OWNER.into(),
                label: "Model scope regression".into(),
                user_service_id: None,
                verification: TriggerVerification::Token {
                    location: crate::models::trigger::TriggerTokenLocation::Bearer,
                },
                delivery: TriggerDelivery::Assistant {
                    confirmation_policy: Default::default(),
                    agent_id: agent.id.clone(),
                    thread_policy: Some(ThreadPolicy::New),
                    instruction: "Summarise the event".into(),
                    deliver_to: DeliverTo::Thread,
                },
            },
        )
        .await
        .unwrap();
        let _ = crate::handlers::trigger_webhooks::receive_trigger(
            State(state.clone()),
            Path(created.trigger.id.clone()),
            Query(Default::default()),
            Request::builder()
                .header(
                    "authorization",
                    format!("Bearer {}", created.raw_secret.as_str()),
                )
                .body(Body::from(
                    r#"{"event_id":"2099","message":"private webhook payload"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
        created.trigger
    };
    tick(&state).await.unwrap();
    let run = completed(&state, &trigger).await;
    let calls = observed.lock().await;
    assert_eq!(calls.len(), 1);
    assert_eq!(
        calls[0]["status"],
        if stateful { 403 } else { 200 },
        "callback result: {}",
        calls[0]["result"]
    );
    assert_eq!(
        run.outcome,
        if stateful {
            RunOutcome::Failed
        } else {
            RunOutcome::Completed
        }
    );
    let row = engine::get(&state.db, OWNER, run.thread_id.as_deref().unwrap())
        .await
        .unwrap();
    assert_eq!(row.automation_thread, !scheduled);
    assert_eq!(row.user_id, OWNER);
    let Json(history) = crate::handlers::triggers::runs(
        State(state.clone()),
        crate::test_utils::test_auth_user(OWNER),
        Path(trigger.id.clone()),
        Query(crate::handlers::triggers::RunsQuery {
            before: None,
            before_id: None,
        }),
    )
    .await
    .unwrap();
    let history = serde_json::to_value(history).unwrap();
    let saved_run = history["runs"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["id"] == run.id)
        .unwrap();
    if stateful {
        let reply = state
            .db
            .collection::<crate::models::assistant_message::AssistantMessage>("assistant_messages")
            .find_one(doc! {"conversation_id":&row.id,"turn_id":&run.turn_id,"role":"assistant"})
            .await
            .unwrap()
            .unwrap();
        let reason = reply.turn_metadata.unwrap().failure_reason.unwrap();
        assert_eq!(reason.stage, "model_proxy");
        assert_eq!(reason.code, "api_key_scope_forbidden");
        assert_eq!(reason.http_status, Some(403));
        assert_eq!(run.failure_reason.as_ref().unwrap().code, reason.code);
        assert_eq!(saved_run["reason"], "assistant_unavailable");
        assert_eq!(
            saved_run["failure_reason"],
            json!({
                "source":"nyxid", "stage":"model_proxy", "code":"api_key_scope_forbidden",
                "http_status":403, "nyxid_error_code":9000,
            })
        );
        let serialized = saved_run.to_string();
        assert!(!serialized.contains("private webhook payload"));
        assert!(!serialized.contains("untrusted provider message"));
    } else {
        assert!(run.failure_reason.is_none());
        assert!(saved_run["failure_reason"].is_null());
    }
    // Old run documents remain readable without the additive diagnostic.
    let mut old = mongodb::bson::to_document(&run).unwrap();
    old.remove("failure_reason");
    assert!(
        mongodb::bson::from_document::<TriggerRun>(old)
            .unwrap()
            .failure_reason
            .is_none()
    );
    assert_eq!(
        state
            .db
            .collection::<Document>("assistant_acknowledgements")
            .count_documents(doc! {"conversation_id":&row.id})
            .await
            .unwrap(),
        0
    );
    server.abort();
    state.db.drop().await.unwrap();
}

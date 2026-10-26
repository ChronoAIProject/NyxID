//! Engine -> fake NyxAgent -> authenticated NyxID model/MCP callbacks.
use super::*;
use crate::services::assistant_authority_tests::{connected, direct_fixture, orchestrator_fixture};
use axum::{Extension, routing::any};
use tower::ServiceExt;

async fn callback_proxy(
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
async fn guest_1995_model_callback_through_real_turn_pipeline() {
    Box::pin(guest_pipeline(
        false,
        &["guest", "approval", "stateful", "concurrency", "catalog"],
    ))
    .await;
}

#[tokio::test]
async fn guest_1995_owner_model_control() {
    Box::pin(guest_pipeline(false, &["owner"])).await;
}

#[tokio::test]
async fn guest_1995_nyxbot_model_works_but_guest_tool_calls_stay_refused() {
    Box::pin(guest_pipeline(
        true,
        &[
            "owner",
            "guest",
            "approval",
            "stateful",
            "concurrency",
            "catalog",
        ],
    ))
    .await;
}

async fn guest_pipeline(nyxbot: bool, scenarios: &[&str]) {
    let f = if nyxbot {
        Box::pin(orchestrator_fixture("guest_1995_nyxbot")).await
    } else {
        Box::pin(direct_fixture("guest_1995_pipeline")).await
    };
    let key = crate::services::key_service::get_api_key(
        &f.state.db,
        &f.owner,
        &f.row.credential_api_key_id,
    )
    .await
    .unwrap();
    if !nyxbot {
        // The model is deliberately outside the specialist's tool grants.
        assert!(!key.allow_all_services && !key.allow_auto_connected_services);
        assert!(key.allowed_service_ids.is_empty());
    }
    // Production defaults, not the authority fixture's approval exemption.
    f.state
        .db
        .collection::<bson::Document>("notification_channels")
        .delete_many(doc! {"user_id": &f.owner})
        .await
        .unwrap();
    let callbacks = Router::new()
        .route("/api/v1/proxy/s/{slug}/{*path}", any(callback_proxy))
        .route(
            "/mcp",
            post(crate::handlers::mcp_transport::mcp_post).layer(Extension(
                crate::services::billing::route_inventory::BillingRoutePolicy::Metered(
                    crate::services::billing::route_inventory::BillingIngress::Mcp,
                ),
            )),
        )
        .layer(Extension(SERVER_TURN_POLICY))
        .with_state(f.state.clone());
    let observed = Arc::new(Mutex::new(Vec::<Value>::new()));
    let sink = observed.clone();
    let model_input = Arc::new(Mutex::new(
        json!({"model":"test","store":false,"input":"test","stream":false}),
    ));
    let input_source = model_input.clone();
    let agent = Router::new().route("/v1/responses", post(move |headers: HeaderMap, Json(body): Json<Value>| {
        let callbacks = callbacks.clone();
        let sink = sink.clone();
        let input_source = input_source.clone();
        async move {
            let bearer = headers.get("authorization").unwrap().clone();
            // NyxAgent's nyxid::Auth uses X-API-Key for MCP and Bearer for
            // model proxy calls. Both carry this conversation's exact key.
            let thread_key = bearer.to_str().unwrap().strip_prefix("Bearer ").unwrap();
            let mut observations = Vec::new();
            for method in ["initialize", "tools/list"] {
                let response = callbacks.clone().oneshot(Request::builder().method("POST").uri("/mcp")
                    .header("x-api-key", thread_key).header("content-type", "application/json")
                    .body(Body::from(json!({"jsonrpc":"2.0","id":1,"method":method,
                        "params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"test-agent","version":"1"}}}).to_string())).unwrap()).await.unwrap();
                let status = response.status().as_u16();
                let bytes = axum::body::to_bytes(response.into_body(), 2 * 1024 * 1024).await.unwrap();
                observations.push(json!({"stage":method,"status":status,"body":serde_json::from_slice::<Value>(&bytes).unwrap()}));
            }
            let response = callbacks.clone().oneshot(Request::builder().method("POST").uri("/api/v1/proxy/s/guest-model/v1/responses")
                .header("authorization", bearer.clone()).header("content-type", "application/json")
                .body(Body::from(input_source.lock().await.to_string())).unwrap()).await.unwrap();
            let status = response.status().as_u16();
            let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
            observations.push(json!({"stage":"model","status":status,"body":serde_json::from_slice::<Value>(&bytes).unwrap()}));
            if nyxbot && body["instructions"].as_str().is_some_and(|text| text.contains("you use no tools or services")) {
                let response = callbacks.oneshot(Request::builder().method("POST").uri("/mcp")
                    .header("x-api-key", thread_key).header("content-type", "application/json")
                    .body(Body::from(json!({"jsonrpc":"2.0","id":2,"method":"tools/call",
                        "params":{"name":"nyx__search_tools","arguments":{"query":"services"}}}).to_string())).unwrap()).await.unwrap();
                let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
                observations.push(json!({"stage":"guest_tool_call","body":serde_json::from_slice::<Value>(&bytes).unwrap()}));
            }
            sink.lock().await.push(json!({"calls":observations,"request":body}));
            let failed = status != 200;
            let event = json!({"type":if failed {"response.failed"} else {"response.completed"},"sequence_number":0,
                "response":{"id":RESPONSE,"conversation":{"id":SESSION},"status":if failed {"failed"} else {"completed"},
                    "error":if failed {json!({"code":"turn_failed","message":"untrusted upstream detail"})} else {Value::Null},
                    "output":if failed {json!([])} else {json!([{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Guest reply"}]}])}}});
            Sse::new(futures::stream::iter([Ok::<_, Infallible>(Event::default().data(event.to_string()))])).into_response()
        }
    }));
    let server = register_upstream(&f.state, agent).await;
    let provider = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .respond_with(
            wiremock::ResponseTemplate::new(200)
                .set_body_json(json!({"id":"model-response","output":[]})),
        )
        .mount(&provider)
        .await;
    let mut model = crate::models::downstream_service::test_helpers::dummy_service();
    model.id = Uuid::new_v4().to_string();
    model.slug = "guest-model".into();
    model.base_url = provider.uri();
    model.auth_method = "none".into();
    model.requires_user_credential = false;
    model.credential_encrypted.clear();
    model.inference = Some(
        serde_json::from_value(json!({"wire_protocol":"openai_responses","model_list":true}))
            .unwrap(),
    );
    f.state
        .db
        .collection::<DownstreamService>(SERVICES)
        .insert_one(&model)
        .await
        .unwrap();
    let connection = connected(&f.state.db, &f.owner, &model.slug, &provider.uri()).await;
    f.state
        .db
        .collection::<bson::Document>("user_services")
        .update_one(
            doc! {"_id":&connection},
            doc! {"$set":{"catalog_service_id":&model.id}},
        )
        .await
        .unwrap();
    f.state
        .db
        .collection::<bson::Document>("assistant_conversations")
        .update_one(
            doc! {"_id":&f.row.id},
            doc! {"$set":{"active_turn":bson::Bson::Null}},
        )
        .await
        .unwrap();
    for &scenario in scenarios {
        let provider_calls_before = provider.received_requests().await.unwrap().len();
        let guest = scenario != "owner";
        *model_input.lock().await =
            json!({"model":"test","store": scenario == "stateful","input":"test","stream":false});
        f.state
            .db
            .collection::<bson::Document>("service_approval_configs")
            .delete_many(doc! {})
            .await
            .unwrap();
        if scenario == "approval" {
            // The HTTP proxy resolves approval policy by catalog service ID;
            // the saved connection ID only binds execution authority.
            crate::services::approval_service::set_service_approval_config(
                &f.state.db,
                &f.owner,
                &model.id,
                "model",
                Some(true),
                None,
                None,
                None,
            )
            .await
            .unwrap();
        }
        if scenario == "catalog" {
            f.state
                .db
                .collection::<bson::Document>("user_services")
                .delete_one(doc! {"_id": &connection})
                .await
                .unwrap();
        }
        let mut lease = None;
        if scenario == "concurrency" {
            let policy = crate::models::service_concurrency::ServiceConcurrencyPolicy {
                service_id: model.id.clone(),
                default_limit: Some(1),
                users: vec![],
                orgs: vec![],
            };
            crate::services::service_concurrency_service::set_policy(
                &f.state.db,
                &model.id,
                Some(&policy),
            )
            .await
            .unwrap();
            lease = crate::services::service_concurrency_service::acquire_policy(
                &f.state.db,
                Some(&policy),
                &f.owner,
            )
            .await
            .unwrap();
        } else {
            crate::services::service_concurrency_service::set_policy(&f.state.db, &model.id, None)
                .await
                .unwrap();
        }
        let input: engine::TurnRequest = serde_json::from_value(
            json!({"conversation_id":&f.row.id,"text":"Where is the shop?"}),
        )
        .unwrap();
        let mut start = engine::TurnStart::from(&input);
        start.guest = guest;
        start.origin = crate::models::assistant_conversation::TurnOrigin::Channel;
        let permit = f
            .state
            .direct_chat_limiter
            .try_acquire(&f.owner)
            .await
            .unwrap();
        let (row, mut events) = Box::pin(start_turn(
            &f.state,
            test_auth_user(&f.owner),
            &start,
            Some(SERVER_TURN_POLICY),
            permit,
        ))
        .await
        .unwrap();
        let completed = tokio::time::timeout(Duration::from_secs(30), async {
            loop {
                let event = events.recv().await.unwrap();
                if event["event"] == "turn.completed" {
                    break event;
                }
            }
        })
        .await
        .unwrap();
        let messages = engine::messages(&f.state.db, &f.owner, &row.id, 10, None)
            .await
            .unwrap();
        let reply = messages
            .iter()
            .rev()
            .find(|m| m.role == "assistant")
            .unwrap();
        let seen = observed.lock().await;
        let calls = &seen.last().unwrap()["calls"];
        if guest && nyxbot {
            assert_eq!(reply.activities.len(), 1);
            assert_eq!(reply.activities[0].status, "error");
        } else {
            assert!(reply.activities.is_empty());
        }
        assert_eq!(reply.turn_metadata.as_ref().unwrap().attempts, Some(1));
        assert_eq!(
            seen.last().unwrap()["request"]["model"],
            seen[0]["request"]["model"]
        );
        assert_eq!(calls[0]["status"], 200, "{calls}");
        assert!(calls[0]["body"]["error"].is_null(), "{calls}");
        assert!(calls[1]["body"]["result"]["tools"].is_array(), "{calls}");
        if guest && nyxbot {
            // Discovery may list tools; every actual NyxBot guest call must
            // remain refused, even when the model ignores its instructions.
            assert_eq!(calls[3]["body"]["result"]["isError"], true);
            assert!(
                seen.last().unwrap()["request"]["instructions"]
                    .as_str()
                    .unwrap()
                    .contains("you use no tools or services")
            );
        }
        let expected = match scenario {
            "approval" | "stateful" => Some((403, "api_key_scope_forbidden", 9000)),
            "concurrency" => Some((429, "service_concurrency_limited", 12600)),
            _ => None,
        };
        if let Some((status, code, numeric)) = expected {
            assert_eq!(calls[2]["status"], status, "{scenario}: {}", calls[2]);
            assert_eq!(
                provider.received_requests().await.unwrap().len(),
                provider_calls_before,
                "{scenario} must be refused before reaching the provider"
            );
            assert_eq!(reply.status, "failed", "{scenario}: {calls}");
            assert_eq!(reply.error_code.as_deref(), Some("assistant_unavailable"));
            assert_eq!(reply.upstream_error_code.as_deref(), Some("turn_failed"));
            assert_eq!(reply.error_detail.as_ref().unwrap().stage, "stream");
            let reason = reply
                .turn_metadata
                .as_ref()
                .unwrap()
                .failure_reason
                .as_ref()
                .unwrap();
            assert_eq!(reason.source, "nyxid");
            assert_eq!(reason.code, code);
            assert_eq!(reason.http_status, Some(status));
            assert_eq!(reason.nyxid_error_code, Some(numeric));
            let dto = serde_json::to_value(MessageResponse::from(reply.clone())).unwrap();
            assert_eq!(dto["turn_metadata"]["failure_reason"]["code"], code);
            assert_eq!(completed["turn_metadata"]["failure_reason"]["code"], code);
            assert!(!dto.to_string().contains("untrusted upstream detail"));
        } else {
            assert_eq!(
                reply.status, "completed",
                "scenario={scenario}; callbacks={calls}; error={:?}",
                reply.error_code
            );
            assert_eq!(
                provider.received_requests().await.unwrap().len(),
                provider_calls_before + 1,
                "{scenario}: {calls}"
            );
            assert!(
                reply
                    .turn_metadata
                    .as_ref()
                    .unwrap()
                    .failure_reason
                    .is_none()
            );
        }
        drop(lease);
    }
    server.abort();
    f.state.db.drop().await.unwrap();
}

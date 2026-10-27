//! Engine -> fake NyxAgent -> authenticated NyxID model/MCP callbacks.
use super::*;
use crate::services::assistant_authority_tests::{connected, direct_fixture, orchestrator_fixture};
use axum::{Extension, routing::any};
use tower::ServiceExt;

#[path = "assistant_owner_model_tests.rs"]
mod owner_model_tests;

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
        &[
            "owner",
            "guest",
            "approval",
            "stateful",
            "concurrency",
            "owner_catalog",
            "catalog",
        ],
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
            "owner_catalog",
            "catalog",
        ],
    ))
    .await;
}

#[tokio::test]
async fn guest_1995_route_binding_rejects_other_byok_models_and_unadmitted_input() {
    Box::pin(guest_pipeline(
        false,
        &[
            "owner",
            "probes",
            "guest_new_thread",
            "missing_input",
            "guest",
            "catalog_unbound",
            "owner_catalog",
            "catalog",
        ],
    ))
    .await;
}

#[tokio::test]
async fn guest_1995_never_learns_a_route_from_a_guest() {
    Box::pin(guest_pipeline(
        false,
        &["unbound", "unbound", "owner", "guest"],
    ))
    .await;
}

#[tokio::test]
async fn guest_1995_fresh_catalog_master_route_bootstraps_without_owner() {
    Box::pin(guest_pipeline(false, &["bootstrap_master"])).await;
}

#[tokio::test]
async fn guest_1995_existing_thread_platform_profile_bootstraps_without_owner() {
    Box::pin(guest_pipeline(false, &["bootstrap_legacy"])).await;
}

#[tokio::test]
async fn guest_1995_fresh_platform_connection_bootstraps_without_owner() {
    Box::pin(guest_pipeline(false, &["bootstrap_platform"])).await;
}

#[tokio::test]
async fn guest_1995_fresh_byok_route_needs_owner_and_has_safe_reply() {
    Box::pin(guest_pipeline(false, &["bootstrap_byok"])).await;
}

async fn guest_pipeline(nyxbot: bool, scenarios: &[&str]) {
    let f = if nyxbot {
        Box::pin(orchestrator_fixture("guest_1995_nyxbot")).await
    } else {
        Box::pin(direct_fixture("guest_1995_pipeline")).await
    };
    let bootstrap = scenarios
        .iter()
        .any(|scenario| scenario.starts_with("bootstrap_"));
    let model_slug = if bootstrap {
        "chrono-llm-public"
    } else {
        "guest-model"
    };
    let model_path = if bootstrap {
        "responses"
    } else {
        "v1/responses"
    };
    let model_name = if bootstrap { "gpt-5.6-sol" } else { "test" };
    if scenarios.contains(&"bootstrap_legacy") {
        // Existing threads keep their configured profile when the agent's
        // default changes. Bootstrap must follow that server-recorded turn.
        f.state
            .db
            .collection::<bson::Document>("assistant_agents")
            .update_one(
                doc! {"_id":&f.chat.agent_id},
                doc! {"$set":{"model":"nyxagent/research","owner_model_route":{
                    "profile":"nyxagent/research","catalog_service_id":"other-profile-catalog",
                    "connection_id":bson::Bson::Null,"operation":"other-profile-operation","execution":"other-profile-execution"
                }}},
            )
            .await
            .unwrap();
    }
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
    let scenario_source = Arc::new(Mutex::new(String::new()));
    let current_scenario = scenario_source.clone();
    let callback_db = f.state.db.clone();
    let callback_conversation = f.row.id.clone();
    let agent = Router::new().route("/v1/responses", post(move |headers: HeaderMap, Json(body): Json<Value>| {
        let callbacks = callbacks.clone();
        let sink = sink.clone();
        let input_source = input_source.clone();
        let scenario_source = scenario_source.clone();
        let db = callback_db.clone();
        let conversation = callback_conversation.clone();
        async move {
            let scenario = scenario_source.lock().await.clone();
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
            if scenario == "missing_input" {
                db.collection::<bson::Document>("assistant_service_execution").delete_many(doc! {
                    "conversation_id":&conversation,"kind":"input_admission"
                }).await.unwrap();
            }
            let response = callbacks.clone().oneshot(Request::builder().method("POST").uri(format!("/api/v1/proxy/s/{model_slug}/{model_path}"))
                .header("authorization", bearer.clone()).header("content-type", "application/json").header("accept", "*/*")
                .body(Body::from(input_source.lock().await.to_string())).unwrap()).await.unwrap();
            let status = response.status().as_u16();
            let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
            observations.push(json!({"stage":"model","status":status,"body":serde_json::from_slice::<Value>(&bytes).unwrap()}));
            if scenario.starts_with("bootstrap_") && scenario != "bootstrap_byok" {
                for (slug, path, model, store, override_method) in [
                    ("unrelated-owner-model", model_path, model_name, false, false),
                    (model_slug, model_path, "unrelated-model", false, false),
                    (model_slug, "v1/models", model_name, false, false),
                    (model_slug, model_path, model_name, true, false),
                    (model_slug, model_path, model_name, false, true),
                    (model_slug, "v1/responses", model_name, false, false),
                ] {
                    let mut request = Request::builder().method("POST").uri(format!("/api/v1/proxy/s/{slug}/{path}"))
                        .header("authorization", bearer.clone()).header("content-type", "application/json").header("accept", "*/*");
                    if override_method { request = request.header("x-http-method-override", "DELETE"); }
                    let response = callbacks.clone().oneshot(request.body(Body::from(json!({
                        "model":model,"store":store,"input":"unrelated generation","stream":false
                    }).to_string())).unwrap()).await.unwrap();
                    assert_eq!(response.status(), StatusCode::FORBIDDEN, "bootstrap probe {slug}/{path}, model={model}");
                }
            }
            if scenario == "probes" {
                // The exact same authenticated key and live turn can use only
                // the owner-established route, not another owner BYOK account.
                for (slug, path, model, store, override_method) in [
                    ("guest-unrelated", "v1/responses", "test", false, false),
                    ("guest-expensive", "v1/responses", "test", false, false),
                    ("guest-model", "v1/responses", "expensive-model", false, false),
                    ("guest-model", "v1/responses", "", false, false),
                    ("guest-model", "v1/models", "test", false, false),
                    ("guest-model", "v1/responses", "test", true, false),
                    ("guest-model", "v1/responses", "test", false, true),
                ] {
                    let mut request = Request::builder().method("POST").uri(format!("/api/v1/proxy/s/{slug}/{path}"))
                        .header("authorization", bearer.clone()).header("content-type", "application/json").header("accept", "*/*");
                    if override_method { request = request.header("x-http-method-override", "DELETE"); }
                    let response = callbacks.clone().oneshot(request.body(Body::from(json!({
                        "model":model,"store":store,"input":"unrelated generation","stream":false
                    }).to_string())).unwrap()).await.unwrap();
                    assert_eq!(response.status(), StatusCode::FORBIDDEN, "probe {slug}/{path}, model={model}, store={store}, override={override_method}");
                }
            }
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
    let mut provider_mock = wiremock::Mock::given(wiremock::matchers::method("POST"));
    if bootstrap {
        provider_mock = provider_mock.and(wiremock::matchers::header(
            "authorization",
            "Bearer test-only-master",
        ));
    }
    provider_mock
        .respond_with(
            wiremock::ResponseTemplate::new(200)
                .set_body_json(json!({"id":"model-response","output":[]})),
        )
        .mount(&provider)
        .await;
    let mut model = crate::models::downstream_service::test_helpers::dummy_service();
    model.id = Uuid::new_v4().to_string();
    model.slug = model_slug.into();
    model.base_url = provider.uri();
    model.auth_method = "none".into();
    model.requires_user_credential = false;
    model.credential_encrypted.clear();
    model.inference = Some(
        serde_json::from_value(json!({"wire_protocol":"openai_responses","model_list":true}))
            .unwrap(),
    );
    if bootstrap {
        model.auth_method = "bearer".into();
        model.service_category = "internal".into();
        model.credential_encrypted = f
            .state
            .encryption_keys
            .encrypt(b"test-only-master")
            .await
            .unwrap();
        if scenarios.contains(&"bootstrap_platform") {
            model.platform_key =
                Some(serde_json::from_value(json!({"enabled":true,"audience":"public"})).unwrap());
        }
    }
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
    if scenarios.contains(&"bootstrap_platform") {
        f.state.db.collection::<bson::Document>("user_services").update_one(doc!{"_id":&connection},
            doc!{"$set":{"credential_binding":"platform","auth_method":"bearer","source":"auto_provision"}}).await.unwrap();
    }
    if bootstrap {
        use crate::services::user_api_key_service::{CreateApiKeyParams, create_api_key};
        let key = create_api_key(
            &f.state.db,
            &f.state.encryption_keys,
            &f.owner,
            CreateApiKeyParams {
                label: "owner BYOK",
                credential_type: "bearer",
                credential: "test-only-byok",
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
        let mut unrelated = model.clone();
        unrelated.id = Uuid::new_v4().to_string();
        unrelated.slug = "unrelated-owner-model".into();
        unrelated.platform_key = None;
        unrelated.service_category = "connection".into();
        unrelated.requires_user_credential = true;
        unrelated.credential_encrypted.clear();
        f.state
            .db
            .collection::<DownstreamService>(SERVICES)
            .insert_one(&unrelated)
            .await
            .unwrap();
        let other = connected(&f.state.db, &f.owner, &unrelated.slug, &provider.uri()).await;
        f.state.db.collection::<bson::Document>("user_services").update_one(doc!{"_id":other},
            doc!{"$set":{"catalog_service_id":unrelated.id,"credential_binding":"user","auth_method":"bearer","api_key_id":&key.id}}).await.unwrap();
        if scenarios.contains(&"bootstrap_byok") {
            f.state.db.collection::<bson::Document>("user_services").update_one(doc!{"_id":&connection},
                doc!{"$set":{"credential_binding":"user","auth_method":"bearer","api_key_id":&key.id}}).await.unwrap();
        }
    }
    if scenarios.contains(&"probes") {
        let mut unrelated = model.clone();
        unrelated.id = Uuid::new_v4().to_string();
        unrelated.slug = "guest-unrelated".into();
        unrelated.auth_method = "bearer".into();
        unrelated.requires_user_credential = true;
        f.state
            .db
            .collection::<DownstreamService>(SERVICES)
            .insert_one(&unrelated)
            .await
            .unwrap();
        for (slug, catalog) in [
            ("guest-unrelated", &unrelated.id),
            ("guest-expensive", &model.id),
        ] {
            let byok = connected(&f.state.db, &f.owner, slug, &provider.uri()).await;
            use crate::services::user_api_key_service::{CreateApiKeyParams, create_api_key};
            let credential = create_api_key(
                &f.state.db,
                &f.state.encryption_keys,
                &f.owner,
                CreateApiKeyParams {
                    label: "unrelated owner BYOK",
                    credential_type: "bearer",
                    credential: "test-only-byok",
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
            f.state
                .db
                .collection::<bson::Document>("user_services")
                .update_one(
                    doc! {"_id":byok},
                    doc! {"$set":{"catalog_service_id":catalog,"api_key_id":credential.id,
                    "auth_method":"bearer","credential_binding":"user"}},
                )
                .await
                .unwrap();
        }
        // Establish the agent's intended model connection through the owner's
        // saved order. An unordered same-catalog alternative otherwise triggers
        // service_priority_required before the owner can establish any route.
        f.state
            .db
            .collection::<bson::Document>("service_preferences")
            .insert_one(doc! {
                "_id":&f.owner,"ordered":[&connection],
                "created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now(),
            })
            .await
            .unwrap();
    }
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
        let guest = !scenario.starts_with("owner");
        *current_scenario.lock().await = scenario.to_owned();
        let route_before = f
            .state
            .db
            .collection::<bson::Document>("assistant_agents")
            .find_one(doc! {"_id":&f.chat.agent_id})
            .await
            .unwrap()
            .unwrap()
            .get("owner_model_route")
            .cloned();
        *model_input.lock().await = json!({"model":model_name,"store": scenario == "stateful","input":"test","stream":false});
        model_input.lock().await["client_metadata"] = json!({
            "root_turn_id":Uuid::new_v4().to_string(),"session_id":Uuid::new_v4().to_string(),
            "thread_id":Uuid::new_v4().to_string(),"turn_id":Uuid::new_v4().to_string(),
            "x-codex-installation-id":"test-installation","x-codex-turn-metadata":"{}",
            "x-codex-window-id":Uuid::new_v4().to_string(),
        });
        f.state
            .db
            .collection::<bson::Document>("service_approval_configs")
            .delete_many(doc! {})
            .await
            .unwrap();
        if !guest {
            // This synthetic, non-platform model route has an existing owner
            // approval. Codex metadata retains legacy write classification for
            // owners; it must not silently bypass the writes-only default.
            crate::services::approval_service::set_service_approval_config(
                &f.state.db,
                &f.owner,
                &model.id,
                "model",
                Some(false),
                None,
                None,
                None,
            )
            .await
            .unwrap();
        }
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
        if matches!(
            scenario,
            "catalog"
                | "catalog_unbound"
                | "owner_catalog"
                | "bootstrap_master"
                | "bootstrap_legacy"
        ) {
            f.state
                .db
                .collection::<bson::Document>("user_services")
                .delete_one(doc! {"_id": &connection})
                .await
                .unwrap();
            // Remove the unrelated same-catalog BYOK alternative as well: this
            // scenario specifically tests the catalog-only path.
            f.state
                .db
                .collection::<bson::Document>("user_services")
                .delete_many(doc! {"catalog_service_id":&model.id})
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
        let input: engine::TurnRequest =
            serde_json::from_value(if scenario == "guest_new_thread" {
                json!({"agent_id":&f.chat.agent_id,"text":"Where is the shop?"})
            } else {
                json!({"conversation_id":&f.row.id,"text":"Where is the shop?"})
            })
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
        if guest {
            let current = engine::get(&f.state.db, &f.owner, &row.id).await.unwrap();
            assert!(
                current.owner_model_route == row.owner_model_route,
                "guest must not create or replace a thread route"
            );
            let route_after = f
                .state
                .db
                .collection::<bson::Document>("assistant_agents")
                .find_one(doc! {"_id":&f.chat.agent_id})
                .await
                .unwrap()
                .unwrap()
                .get("owner_model_route")
                .cloned();
            assert_eq!(
                route_before, route_after,
                "guest may not establish or replace a model route"
            );
            assert_eq!(f.state.db.collection::<bson::Document>("assistant_service_execution")
                .count_documents(doc! {"conversation_id":&row.id,"turn_id":completed["turn_id"].as_str(),"kind":"model_consent"})
                .await.unwrap(), 0, "guest may not mint owner consent");
        }
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
            "approval" => Some((403, "api_key_scope_forbidden", 9000)),
            "stateful" | "missing_input" | "unbound" | "catalog_unbound" | "bootstrap_byok" => {
                Some((403, "guest_model_route_required", 9000))
            }
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
            let mapped = if code == "guest_model_route_required" {
                code
            } else {
                "assistant_unavailable"
            };
            assert_eq!(reply.error_code.as_deref(), Some(mapped));
            if code == "guest_model_route_required" {
                assert_eq!(
                    completed["error"]["message"],
                    "This assistant isn't available to guests yet."
                );
            }
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
                "scenario={scenario}; model_callback={}; error={:?}",
                calls[2], reply.error_code
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

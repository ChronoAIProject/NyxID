//! Owner callbacks keep pre-extension admission, even with Codex telemetry.
use super::*;
use crate::models::usage_meter::{CredentialClass, UsageMeterRow, UsageStatus};

#[derive(Clone, Copy)]
enum Route {
    Platform,
    Byok,
    Multiple,
}

#[tokio::test]
async fn owner_1995_platform_metadata_keeps_legacy_admission_and_body() {
    Box::pin(owner_pipeline(Route::Platform, false)).await;
}

#[tokio::test]
async fn owner_1995_byok_metadata_keeps_legacy_admission_and_body() {
    Box::pin(owner_pipeline(Route::Byok, false)).await;
}

#[tokio::test]
async fn owner_1995_multiple_connections_keep_the_owner_route_without_new_selection() {
    Box::pin(owner_pipeline(Route::Multiple, false)).await;
}

#[tokio::test]
async fn owner_1995_metadata_model_callbacks_survive_steering_and_continuation() {
    Box::pin(owner_pipeline(Route::Platform, true)).await;
}

#[tokio::test]
async fn owner_1995_metadata_preserves_required_default_and_explicit_deny_policy() {
    use crate::models::service_approval_config::{ApprovalEffect, ApprovalVerb};
    use crate::services::{approval_service, operation_descriptor};
    let f = Box::pin(direct_fixture("owner_1995_approval_default")).await;
    f.state
        .db
        .collection::<bson::Document>("notification_channels")
        .delete_many(doc! {"user_id":&f.owner})
        .await
        .unwrap();
    let mut service = crate::models::downstream_service::test_helpers::dummy_service();
    service.inference =
        Some(serde_json::from_value(json!({"wire_protocol":"openai_responses"})).unwrap());
    let body = serde_json::to_vec(&generation("hello")).unwrap();
    let mut descriptor =
        operation_descriptor::build_http_descriptor("POST", "responses", Some(&body));
    operation_descriptor::apply_inference_contract(
        &mut descriptor,
        &service,
        Some(&body),
        false,
        false,
    );
    assert_eq!(descriptor.verb, ApprovalVerb::Write);
    for auto_connected in [false, true] {
        let policy = approval_service::resolve_org_aware_approval(
            &f.state.db,
            &f.owner,
            &f.owner,
            &service.id,
            &descriptor,
            auto_connected,
        )
        .await
        .unwrap();
        assert_eq!(
            policy.required, !auto_connected,
            "legacy writes-only versus platform default"
        );
    }
    approval_service::set_service_approval_config(
        &f.state.db,
        &f.owner,
        &service.id,
        "model",
        Some(true),
        None,
        None,
        Some(&ApprovalEffect::Deny),
    )
    .await
    .unwrap();
    for auto_connected in [false, true] {
        let policy = approval_service::resolve_org_aware_approval(
            &f.state.db,
            &f.owner,
            &f.owner,
            &service.id,
            &descriptor,
            auto_connected,
        )
        .await
        .unwrap();
        assert_eq!(policy.effect, ApprovalEffect::Deny);
    }
    f.state.db.drop().await.unwrap();
}

fn generation(input: &str) -> Value {
    json!({"model":"gpt-5.6-sol","store":false,"stream":false,"input":input,
        "instructions":"Owner model round","tools":[],"tool_choice":"auto",
        "parallel_tool_calls":true,"include":["reasoning.encrypted_content"],
        "reasoning":{"effort":"low"},"prompt_cache_key":Uuid::new_v4().to_string(),
        "client_metadata":{"root_turn_id":Uuid::new_v4().to_string(),
            "session_id":Uuid::new_v4().to_string(),"thread_id":Uuid::new_v4().to_string(),
            "turn_id":Uuid::new_v4().to_string(),"x-codex-installation-id":"test-installation",
            "x-codex-turn-metadata":"{\"sandbox_mode\":\"read-only\"}",
            "x-codex-window-id":Uuid::new_v4().to_string()}})
}

async fn byok_connection(
    f: &crate::services::assistant_authority_tests::Fixture,
    catalog: &str,
    slug: &str,
    url: &str,
    credential: &str,
) -> String {
    use crate::services::user_api_key_service::{CreateApiKeyParams, create_api_key};
    let id = connected(&f.state.db, &f.owner, slug, url).await;
    let key = create_api_key(
        &f.state.db,
        &f.state.encryption_keys,
        &f.owner,
        CreateApiKeyParams {
            label: slug,
            credential_type: "bearer",
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
    f.state
        .db
        .collection::<bson::Document>("user_services")
        .update_one(
            doc! {"_id":&id},
            doc! {"$set":{"catalog_service_id":catalog,"api_key_id":key.id,
            "credential_binding":"user","auth_method":"bearer"}},
        )
        .await
        .unwrap();
    id
}

async fn owner_pipeline(route: Route, continue_and_steer: bool) {
    let mut f = Box::pin(direct_fixture("owner_1995_model_transport")).await;
    f.state.billing = Arc::new(crate::billing_integration_tests::billing_service(
        &f.state.db,
        Arc::new(crate::billing_integration_tests::FakeLago::default()),
        1000,
    ));
    for metric in ["platform_tokens", "platform_requests", "platform_bytes"] {
        f.state
            .db
            .collection("billing_rate_cache")
            .insert_one(crate::billing_integration_tests::rate(metric, Utc::now()))
            .await
            .unwrap();
    }
    let provider = wiremock::MockServer::start().await;
    let platform = matches!(route, Route::Platform);
    let slug = if platform {
        "chrono-llm-public"
    } else {
        "owner-model"
    };
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/responses"))
        .and(wiremock::matchers::header(
            "authorization",
            if platform {
                "Bearer test-platform"
            } else {
                "Bearer test-byok"
            },
        ))
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(json!({
            "id":"model-response","model":"gpt-5.6-sol","output":[],
            "usage":{"input_tokens":7,"output_tokens":3,"total_tokens":10}
        })))
        .mount(&provider)
        .await;
    let mut catalog = crate::models::downstream_service::test_helpers::dummy_service();
    catalog.id = Uuid::new_v4().to_string();
    catalog.slug = slug.into();
    catalog.base_url = provider.uri();
    catalog.auth_method = "bearer".into();
    catalog.requires_user_credential = !platform;
    catalog.credential_encrypted.clear();
    catalog.billing = Some(crate::models::service_billing::ServiceBilling {
        platform_billable: true,
        platform_charge_nyxid_credentials_only: false,
        ..Default::default()
    });
    catalog.service_category = if platform { "internal" } else { "connection" }.into();
    catalog.inference =
        Some(serde_json::from_value(json!({"wire_protocol":"openai_responses"})).unwrap());
    if platform {
        catalog.platform_key =
            Some(serde_json::from_value(json!({"enabled":true,"audience":"public"})).unwrap());
        catalog.credential_encrypted = f
            .state
            .encryption_keys
            .encrypt(b"test-platform")
            .await
            .unwrap();
    }
    f.state
        .db
        .collection::<DownstreamService>(SERVICES)
        .insert_one(&catalog)
        .await
        .unwrap();
    let connection = if platform {
        let id = connected(&f.state.db, &f.owner, slug, &provider.uri()).await;
        f.state
            .db
            .collection::<bson::Document>("user_services")
            .update_one(
                doc! {"_id":&id},
                doc! {"$set":{"catalog_service_id":&catalog.id,"credential_binding":"platform",
                "auth_method":"bearer","source":"auto_provision"}},
            )
            .await
            .unwrap();
        id
    } else {
        byok_connection(&f, &catalog.id, slug, &provider.uri(), "test-byok").await
    };
    if matches!(route, Route::Multiple) {
        let other = byok_connection(
            &f,
            &catalog.id,
            "owner-other-model",
            &provider.uri(),
            "test-other-byok",
        )
        .await;
        // Existing owner preference, not new selection/consent created by the
        // callback. Both connections remain outside specialist tool grants.
        f.state
            .db
            .collection::<bson::Document>("service_preferences")
            .insert_one(doc! {
                "_id":&f.owner,"ordered":[&connection,other],
                "created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now(),
            })
            .await
            .unwrap();
    }
    // Platform defaults work without a global preference. BYOK retains the
    // fixture's existing owner approval exemption; required policies are
    // covered separately by the exact-body approval regression.
    if platform {
        f.state
            .db
            .collection::<bson::Document>("notification_channels")
            .delete_many(doc! {"user_id":&f.owner})
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
    assert!(!key.allow_all_services && key.allowed_service_ids.is_empty());
    crate::billing_integration_tests::create_usage_index(&f.state.db).await;
    let callbacks = Router::new()
        .route("/api/v1/proxy/s/{slug}/{*path}", any(callback_proxy))
        .layer(Extension(SERVER_TURN_POLICY))
        .with_state(f.state.clone());
    let sent = Arc::new(Mutex::new(Vec::<Vec<u8>>::new()));
    let steers = Arc::new(Mutex::new(Vec::<Value>::new()));
    let calls = Arc::new(Mutex::new(Vec::<Value>::new()));
    let ready = Arc::new(tokio::sync::Semaphore::new(0));
    let resume = Arc::new(tokio::sync::Semaphore::new(0));
    let rounds = Arc::new(AtomicUsize::new(0));
    let steer_sink = steers.clone();
    let upstream = Router::new()
        .route("/v1/capabilities", axum::routing::get(|| async { Json(json!({"steer":{
            "version":1,"protocol":"nyxagent-steer-v1","input":["text"],"max_chars":32000,"max_per_turn":20
        }})) }))
        .route("/v1/conversations/{id}/steer", post(move |Json(body): Json<Value>| {
            let steer_sink = steer_sink.clone();
            async move {
                steer_sink.lock().await.push(body.clone());
                Json(json!({"object":"conversation.steer","conversation_id":SESSION,
                    "response_id":body["expected_response_id"],"status":"accepted"}))
            }
        }))
        .route("/v1/responses", post({
            let sent = sent.clone(); let calls = calls.clone(); let steers = steers.clone();
            let ready = ready.clone(); let resume = resume.clone();
            move |headers: HeaderMap, Json(body): Json<Value>| {
                let callbacks = callbacks.clone(); let sent = sent.clone(); let calls = calls.clone();
                let ready = ready.clone(); let resume = resume.clone(); let steers = steers.clone();
                let attempt = rounds.fetch_add(1, Ordering::SeqCst);
                async move {
                    calls.lock().await.push(body);
                    let stream = async_stream::stream! {
                        let response = format!("resp_{}_{}", &SESSION[5..], if attempt == 0 {"a".repeat(32)} else {"b".repeat(32)});
                        yield Ok::<_, Infallible>(Event::default().data(json!({"type":"response.created","sequence_number":0,
                            "response":{"id":response,"conversation":{"id":SESSION},"status":"in_progress"}}).to_string()));
                        for round in 0..if attempt == 0 { 2 } else { 1 } {
                            let guidance = steers.lock().await.last().map(|body| body["input"].as_str().unwrap().to_owned()).unwrap_or_default();
                            let model = generation(&format!("round {attempt}/{round}: {guidance}"));
                            let bytes = serde_json::to_vec_pretty(&model).unwrap();
                            sent.lock().await.push(bytes.clone());
                            let callback = callbacks.clone().oneshot(Request::builder().method("POST")
                                .uri(format!("/api/v1/proxy/s/{slug}/responses"))
                                .header("authorization", headers["authorization"].clone())
                                .header("content-type","application/json").header("accept","*/*")
                                .body(Body::from(bytes)).unwrap()).await.unwrap();
                            let status = callback.status();
                            let result = axum::body::to_bytes(callback.into_body(), 1024*1024).await.unwrap();
                            assert_eq!(status, StatusCode::OK, "owner callback: {}", String::from_utf8_lossy(&result));
                            if continue_and_steer && attempt == 0 && round == 0 {
                                ready.add_permits(1);
                                resume.acquire().await.unwrap().forget();
                            }
                        }
                        let continuing = continue_and_steer && attempt == 0;
                        yield Ok::<_, Infallible>(Event::default().data(json!({
                            "type":if continuing {"response.failed"} else {"response.completed"},"sequence_number":1,
                            "response":{"id":response,"conversation":{"id":SESSION},"status":if continuing {"failed"} else {"completed"},
                                "error":if continuing {json!({"code":"tool_budget_exhausted"})} else {Value::Null},
                                "output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Done"}]}]}
                        }).to_string()));
                    };
                    Sse::new(stream)
                }
            }
        }));
    let server = register_upstream(&f.state, upstream).await;
    f.state
        .db
        .collection::<bson::Document>("assistant_conversations")
        .update_one(
            doc! {"_id":&f.row.id},
            doc! {"$set":{"active_turn":bson::Bson::Null}},
        )
        .await
        .unwrap();
    let input: engine::TurnRequest =
        serde_json::from_value(json!({"conversation_id":f.row.id,"text":"Hello"})).unwrap();
    let start = engine::TurnStart::from(&input);
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
    if continue_and_steer {
        tokio::time::timeout(Duration::from_secs(20), ready.acquire())
            .await
            .unwrap()
            .unwrap()
            .forget();
        let running = engine::get(&f.state.db, &f.owner, &row.id).await.unwrap();
        assert!(
            running
                .active_turn
                .as_ref()
                .unwrap()
                .running_response
                .is_some()
        );
        let mut request = Request::builder().method("POST").body(Body::from(json!({
            "text":"Use the new direction","turn_id":row.active_turn.as_ref().unwrap().turn_id,
            "clientRequestId":Uuid::new_v4().to_string()
        }).to_string())).unwrap();
        request.extensions_mut().insert(SERVER_TURN_POLICY);
        let response = steer(
            State(f.state.clone()),
            test_auth_user(&f.owner),
            Path(row.id.clone()),
            request,
        )
        .await
        .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        resume.add_permits(1);
    }
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
    assert_eq!(completed["status"], "completed", "{completed}");
    let model_calls = if continue_and_steer { 3 } else { 2 };
    let received = provider.received_requests().await.unwrap();
    let sent = sent.lock().await;
    assert_eq!(received.len(), model_calls);
    for (request, expected) in received.iter().zip(sent.iter()) {
        assert_eq!(
            &request.body, expected,
            "forwarded body must be byte-for-byte unchanged"
        );
    }
    let messages = engine::messages(&f.state.db, &f.owner, &row.id, 20, None)
        .await
        .unwrap();
    let reply = messages
        .iter()
        .rev()
        .find(|message| message.role == "assistant")
        .unwrap();
    assert!(reply.error_code.is_none() && reply.activities.is_empty());
    assert_eq!(
        calls.lock().await.len(),
        1 + usize::from(continue_and_steer)
    );
    assert_eq!(
        reply.turn_id.as_str(),
        row.active_turn.as_ref().unwrap().turn_id.as_str()
    );
    if continue_and_steer {
        assert_eq!(steers.lock().await.len(), 1);
        assert!(messages.iter().any(|message| {
            message
                .steering
                .as_ref()
                .is_some_and(|s| s.outcome == "applied")
        }));
        let next: Value = serde_json::from_slice(&sent[1]).unwrap();
        assert!(
            next["input"]
                .as_str()
                .unwrap()
                .contains("Use the new direction")
        );
        assert_eq!(calls.lock().await[1]["conversation"], SESSION);
    }
    assert_eq!(f.state.db.collection::<bson::Document>("assistant_service_execution")
        .count_documents(doc!{"conversation_id":&row.id,"kind":{"$in":["model_consent","selection","card_binding","unavailable_bucket"]}})
        .await.unwrap(),0,"no new model consent, selection, card or fallback bucket");
    assert_eq!(
        f.state
            .db
            .collection::<bson::Document>("assistant_acknowledgements")
            .count_documents(doc! {"conversation_id":&row.id,"status":"pending"})
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        f.state
            .db
            .collection::<bson::Document>("approval_requests")
            .count_documents(doc! {"status":"pending"})
            .await
            .unwrap(),
        0
    );
    let final_row = engine::get(&f.state.db, &f.owner, &row.id).await.unwrap();
    assert_eq!(
        final_row
            .owner_model_route
            .as_ref()
            .unwrap()
            .connection_id
            .as_deref(),
        Some(connection.as_str())
    );
    let usage: Vec<UsageMeterRow> = crate::billing_integration_tests::wait_for_charged_usage(
        &f.state.db,
        doc! {"service_id": &catalog.id},
        model_calls,
    )
    .await;
    for usage in usage {
        assert_eq!(usage.status, UsageStatus::Finalized);
        assert_eq!(usage.billing_owner_id, f.owner);
        assert_eq!(usage.actor_user_id, f.owner);
        assert_eq!(usage.user_service_id.as_deref(), Some(connection.as_str()));
        assert_eq!(
            usage.credential_class,
            if platform {
                CredentialClass::NyxidManagedMaster
            } else {
                CredentialClass::UserOwned
            }
        );
    }
    server.abort();
    f.state.db.drop().await.unwrap();
}

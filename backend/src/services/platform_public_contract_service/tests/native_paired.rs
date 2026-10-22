//! Real local NyxAgent/Codex, NyxID MCP/funding and durable native signup/final sends.
//! The provider emits a deterministic four-step script; it never calls a model.
use super::*;
use crate::{
    models::{
        billing_wallet::{BillingWallet, COLLECTION_NAME as WALLETS},
        platform_manager_delivery::{
            COLLECTION_NAME as DELIVERIES, DeliveryState, PlatformManagerDelivery,
        },
        platform_public_budget::{LOOPS, PublicBudgetLoop},
        platform_public_turn::{COLLECTION_NAME as TURNS, PlatformPublicTurn},
    },
    services::{
        platform_bot_runtime_control_service as controls,
        platform_entry_admin_service::NativeConversionOutcome,
        platform_entry_gateway_service::tests::setup::SetupFixture,
        platform_entry_native::tests as native,
        platform_native_reply_service::{NativeTestTransport, with_transport_for_test},
        platform_public_tools as tools,
    },
};
use axum::{
    body::{Body, to_bytes},
    http::Request,
    middleware::{self, Next},
    routing::{get, post},
};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use tokio::sync::Mutex;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};
use zeroize::Zeroizing;

const CANONICAL_ANSWER: &str = "NyxID manages sign-in and connected services. Review the private account link sent separately.";

struct NoOwner;
#[async_trait::async_trait]
impl crate::services::assistant_nyxagent_runtime::OwnerTurnSettledHook for NoOwner {
    async fn settled(
        &self,
        _: crate::models::assistant_conversation::AssistantConversation,
        _: ReplyRef,
    ) {
        panic!("public-only fixture");
    }
}

fn scripted_events(model: &str, number: usize, item: Value) -> String {
    let id = format!("resp_native_pair_{number}");
    let mut added = item.clone();
    added["status"] = "in_progress".into();
    if item["type"] == "function_call" {
        added["arguments"] = "".into();
    } else {
        added["content"] = json!([]);
    }
    let mut events = vec![
        json!({"type":"response.created","response":{"id":id,"model":model,"status":"in_progress","output":[]}}),
        json!({"type":"response.output_item.added","output_index":0,"item":added}),
    ];
    if item["type"] == "function_call" {
        events.push(json!({"type":"response.function_call_arguments.delta","output_index":0,"item_id":item["id"],"delta":item["arguments"]}));
        events.push(json!({"type":"response.function_call_arguments.done","output_index":0,"item_id":item["id"],"arguments":item["arguments"]}));
    } else {
        events.push(json!({"type":"response.content_part.added","output_index":0,"item_id":item["id"],"content_index":0,"part":{"type":"output_text","text":"","annotations":[]}}));
        events.push(json!({"type":"response.output_text.delta","output_index":0,"item_id":item["id"],"content_index":0,"delta":item["content"][0]["text"]}));
        events.push(json!({"type":"response.output_text.done","output_index":0,"item_id":item["id"],"content_index":0,"text":item["content"][0]["text"]}));
        events.push(json!({"type":"response.content_part.done","output_index":0,"item_id":item["id"],"content_index":0,"part":item["content"][0]}));
    }
    events.push(json!({"type":"response.output_item.done","output_index":0,"item":item}));
    events.push(json!({"type":"response.completed","response":{"id":id,"object":"response","created_at":1,"status":"completed","model":model,"output":[item],
        "usage":{"input_tokens":8192,"output_tokens":512,"total_tokens":8704,"output_tokens_details":{"reasoning_tokens":128}}}}));
    events
        .into_iter()
        .map(|event| {
            format!(
                "event: {}\ndata: {event}\n\n",
                event["type"].as_str().unwrap()
            )
        })
        .collect()
}

#[test]
#[ignore = "requires coordinated A8 upstream binary/policy and pinned Codex wrapper; all service/provider egress local"]
fn real_codex_native_v2_discovery_readiness_faq_search_read_signup_answer_use_actual_mcp_funding_and_durable_native_delivery()
 {
    crate::test_utils::run_native_journey_test(
        "real_codex_native_v2_discovery_readiness_faq_search_read_signup_answer_use_actual_mcp_funding_and_durable_native_delivery",
        || async {
            use base64::Engine;
            let setup = SetupFixture::new(0).await;
            let NativeConversionOutcome::Converted(profile) = setup.convert(None).await.unwrap()
            else {
                panic!("actual native gateway conversion");
            };
            let mut state = setup.state().clone();
            let db = state.db.clone();
            crate::db::ensure_indexes(&db).await.unwrap();
            let provider = MockServer::start().await;
            let delivery = MockServer::start().await;
            let lago = MockServer::start().await;
            Mock::given(method("GET"))
                .and(path(
                    "/api/v1/subscriptions/public-subscription/entitlements",
                ))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_json(json!({"entitlements":[{"code":"*"}]})),
                )
                .mount(&lago)
                .await;
            let callback_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let callback_base = format!("http://{}", callback_listener.local_addr().unwrap());
            state.config.base_url = callback_base.clone();
            state.config.billing_enabled = true;
            state.config.lago_api_url = Some(lago.uri());
            state.config.lago_api_key = Some("local-pairing-lago-key".into());
            state.billing = Arc::new(crate::services::billing::BillingService::new(
                state.db.clone(),
                Arc::new(state.config.clone()),
            ));
            state.jwk_json = crate::test_utils::cached_test_jwk();
            assert_eq!(state.jwk_json["kid"], state.jwt_keys.kid);
            crate::services::feature_flag_service::set_platform_override(
                &db,
                crate::services::feature_flag_service::BILLING_FLAG_KEY,
                &crate::services::feature_flag_service::FlagTarget::Global,
                true,
                setup.actor(),
            )
            .await
            .unwrap();
            let transport = Arc::new(NativeTestTransport {
                adapter: Arc::new(
                    crate::services::channel_adapters::discord::DiscordAdapter::media_test_adapter(
                        &delivery.uri(),
                    ),
                ),
                discord_base: delivery.uri(),
            });
            let provider_sends = Arc::new(AtomicUsize::new(0));
            let observed_sends = provider_sends.clone();
            Mock::given(method("POST"))
        .and(path("/webhooks/100/native-paired-interaction"))
        .respond_with(move |_: &wiremock::Request| {
            let n = observed_sends.fetch_add(1, Ordering::SeqCst);
            assert!(n < 2, "provider resend");
            ResponseTemplate::new(200).set_body_json(json!({"id":if n == 0 {"protected-signup-local-send"} else {"public-final-local-send"}}))
        })
        .expect(2)
        .mount(&delivery)
        .await;
            let mcp_calls = Arc::new(Mutex::new(Vec::<String>::new()));
            let callback_outcomes = Arc::new(Mutex::new(Vec::<(String, u16)>::new()));
            let observed = mcp_calls.clone();
            let scoped = transport.clone();
            let mcp = post(crate::handlers::mcp_transport::mcp_post)
                .layer(axum::Extension(
                    crate::services::billing::route_inventory::BillingRoutePolicy::Metered(
                        crate::services::billing::BillingIngress::Mcp,
                    ),
                ))
                .layer(middleware::from_fn(
                    move |request: Request<Body>, next: Next| {
                        let observed = observed.clone();
                        let scoped = scoped.clone();
                        async move {
                            let (parts, body) = request.into_parts();
                            let bytes =
                                Zeroizing::new(to_bytes(body, 1_000_000).await.unwrap().to_vec());
                            let rpc: Value = serde_json::from_slice(&bytes).unwrap();
                            if rpc["method"] == "tools/call" {
                                let name = rpc["params"]["arguments"]["tool_name"]
                                    .as_str()
                                    .or_else(|| rpc["params"]["name"].as_str())
                                    .unwrap();
                                observed.lock().await.push(name.to_owned());
                            }
                            let request = Request::from_parts(parts, Body::from(bytes.to_vec()));
                            with_transport_for_test(scoped, next.run(request)).await
                        }
                    },
                ));
            let observed_callbacks = callback_outcomes.clone();
            let callback = axum::Router::new()
                .route("/mcp", mcp)
                .route(
                    "/api/v1/public-nyxbot/models/{service_id}/{*path}",
                    post(crate::handlers::platform_public_model::dispatch),
                )
                .route(
                    "/.well-known/jwks.json",
                    get(crate::handlers::oidc_discovery::jwks),
                )
                .layer(middleware::from_fn(
                    move |request: Request<Body>, next: Next| {
                        let observed = observed_callbacks.clone();
                        async move {
                            let path = request.uri().path().to_owned();
                            let response = next.run(request).await;
                            observed
                                .lock()
                                .await
                                .push((path, response.status().as_u16()));
                            response
                        }
                    },
                ))
                .with_state(state.clone());
            let (stop_callback, callback_stopped) = tokio::sync::oneshot::channel();
            let callback_worker = tokio::spawn(async move {
                axum::serve(callback_listener, callback)
                    .with_graceful_shutdown(async {
                        let _ = callback_stopped.await;
                    })
                    .await
                    .unwrap();
            });
            let port = {
                let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
                listener.local_addr().unwrap().port()
            };
            let runtime_base = format!("http://127.0.0.1:{port}");
            let mut outer = outer();
            outer.base_url = runtime_base.clone();
            let mut model = model();
            model.base_url = format!("{}/v1", provider.uri());
            model.credential_encrypted = state
                .encryption_keys
                .encrypt(b"local-pairing-model-key")
                .await
                .unwrap();
            model.billing=Some(serde_json::from_value(json!({"platform_key_pricing":{"metric":"input_tokens","credits_per_unit":"0.000005","lago_metric_code":"public_input","sync_status":"synced","components":[
        {"metric":"output_tokens","credits_per_unit":"0.00002","lago_metric_code":"public_output","sync_status":"synced"},
        {"metric":"cache_read_tokens","credits_per_unit":"0.0000008","lago_metric_code":"public_cache_read","sync_status":"synced"},
        {"metric":"cache_write_tokens","credits_per_unit":"0.000006","lago_metric_code":"public_cache_write","sync_status":"synced"},
        {"metric":"images","credits_per_unit":"0.2","lago_metric_code":"public_images","sync_status":"synced"}]}})).unwrap());
            db.collection::<DownstreamService>(SERVICES)
                .insert_many([&outer, &model])
                .await
                .unwrap();
            cache_rates(
                &db,
                &[
                    ("public_input", "0.000005"),
                    ("public_output", "0.00002"),
                    ("public_cache_read", "0.0000008"),
                    ("public_cache_write", "0.000006"),
                    ("public_images", "0.2"),
                ],
            )
            .await;
            db.collection::<Document>(WALLETS).insert_one(doc! {"_id":uuid::Uuid::new_v4().to_string(),"owner_id":setup.funding(),
        "lago_customer_id":"public-customer","lago_subscription_id":"public-subscription","lago_wallet_id":"public-wallet",
        "plan_kind":"prepaid","balance_credits":Credits::from_whole(20).decimal(),"reserved_credits":Credits::ZERO.decimal(),
        "collection_state":"good","balance_synced_at":bson::DateTime::now(),"created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now()}).await.unwrap();
            let model_id = "gpt-6.1-sol".to_owned();
            let response_model = model_id.clone();
            let count = Arc::new(AtomicUsize::new(0));
            let observed_count = count.clone();
            let captured = Arc::new(std::sync::Mutex::new(Vec::<Value>::new()));
            let observed_models = captured.clone();
            let articles: Value =
                serde_json::from_str(include_str!("../../../../content/platform-public-faq.json"))
                    .unwrap();
            let article = articles[0]["id"].clone();
            let defs = tools::definitions();
            let expected_tools=Value::Array(defs["tools"].as_array().unwrap().iter().map(|tool|json!({"type":"function","name":tool["name"],"description":tool["description"],"parameters":tool["inputSchema"]})).collect());
            Mock::given(method("POST")).and(path("/v1/responses")).respond_with(move |request:&wiremock::Request| {
        let body:Value=serde_json::from_slice(&request.body).unwrap();
        if body.get("tools").is_none() {
            return ResponseTemplate::new(200).set_body_json(json!({"id":"resp_native_pair_probe","object":"response","status":"completed","model":response_model,"output":[{"id":"msg_native_pair_probe","type":"message","role":"assistant","content":[{"type":"output_text","text":"OK"}]}],"usage":{"input_tokens":20,"output_tokens":3,"total_tokens":23}}));
        }
        assert_eq!(body["tools"],expected_tools);
        assert!(request.body.len()<=8192,"actual normalized callback exceeds reviewed bound");
        assert!(!body.to_string().contains("nyx_pli_"));
        observed_models.lock().unwrap().push(body);
        let n=observed_count.fetch_add(1,Ordering::SeqCst);
        assert!(n<4,"real runtime exceeded four model calls");
        let item=if n<3 {
            let (name,args)=match n { 0=>("public_faq_search",json!({"query":"NyxID"})),1=>("public_faq_read",json!({"article_id":article})),_=>("public_signup_link",json!({})) };
            json!({"type":"function_call","id":format!("fc_native_{n}"),"call_id":format!("call_native_{n}"),"name":name,"arguments":args.to_string(),"status":"completed"})
        } else { json!({"id":"msg_native_pair","type":"message","role":"assistant","status":"completed","phase":"final_answer","content":[{"type":"output_text","text":CANONICAL_ANSWER,"annotations":[]}]}) };
        ResponseTemplate::new(200).set_body_raw(scripted_events(&response_model,n,item).into_bytes(),"text/event-stream")
    }).mount(&provider).await;
            let isolated = tempfile::tempdir().unwrap();
            let log_path = std::env::temp_dir().join(format!(
                "nyxid-native-paired-runtime-{}.log",
                uuid::Uuid::new_v4()
            ));
            let log = std::fs::File::create(&log_path).unwrap();
            let mut child = tokio::process::Command::new(
                std::env::var("NYXAGENT_PUBLIC_TEST_BINARY")
                    .expect("coordinated A8 upstream binary required"),
            )
            .env_clear()
            .env("PATH", std::env::var("PATH").unwrap())
            .env("HOME", isolated.path())
            .env("ENVIRONMENT", "test")
            .env("PORT", port.to_string())
            .env(
                "POLICY_FILE",
                std::env::var("NYXAGENT_PUBLIC_TEST_POLICY").unwrap(),
            )
            .env(
                "CODEX_APP_SERVER_BINARY",
                std::env::var("CODEX_APP_SERVER_BINARY").unwrap(),
            )
            .env("CODEX_RUNTIME_VERSION", "rust-v0.160.1")
            .env("RUNTIME_CACHE", isolated.path().join("runtimes"))
            .env("NYXID_BASE_URL", &callback_base)
            .env("NYXID_ISSUER", &state.config.jwt_issuer)
            .env(
                "SESSION_ENCRYPTION_KEY",
                base64::engine::general_purpose::STANDARD.encode(rand::random::<[u8; 32]>()),
            )
            .env(
                "SESSION_BINDING_KEY",
                base64::engine::general_purpose::STANDARD.encode(rand::random::<[u8; 32]>()),
            )
            .env("STORAGE_BACKEND", "memory")
            .env("MAX_CONCURRENT_TURNS", "1")
            .current_dir(isolated.path())
            .stdin(std::process::Stdio::null())
            .stdout(log.try_clone().unwrap())
            .stderr(log)
            .kill_on_drop(true)
            .spawn()
            .unwrap();
            tokio::time::timeout(std::time::Duration::from_secs(15), async {
                loop {
                    assert!(
                        child.try_wait().unwrap().is_none(),
                        "local runtime exited: {}",
                        log_path.display()
                    );
                    if state
                        .http_client
                        .get(format!("{runtime_base}/healthz"))
                        .send()
                        .await
                        .is_ok_and(|response| response.status().is_success())
                    {
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(25)).await;
                }
            })
            .await
            .unwrap();
            let reviewed = controls::reviewed_policy(
                &db,
                setup.actor(),
                &profile.id,
                &RuntimePatch {
                    public_deployment_digest: None,
                    initialize: true,
                    expected_profile_generation: Some(profile.generation),
                    expected: ExpectedRuntimeRevisions {
                        manager_authority_epoch: 0,
                        personal_control_revision: 0,
                        public_policy_revision: 0,
                    },
                    personal_enabled: Some(true),
                    public_enabled: Some(true),
                    emergency_stopped: None,
                    public_policy: PolicyPatch::Update(PublicPolicyPatch {
                        sponsor_owner_user_id: Some(setup.funding().into()),
                        nyxagent_profile: Some("nyxagent/public-guide".into()),
                        total_daily_credits: Some(Credits::from_whole(10)),
                        sender_daily_credits: Some("0.5".parse().unwrap()),
                    }),
                },
            )
            .await
            .unwrap();
            let (discovery, deployed) = discover(&state, &reviewed).await.unwrap();
            assert_eq!(discovery.profiles.len(), 1);
            let (deployed_outer, manifest) = deployed.unwrap();
            assert_eq!(deployed_outer.id, outer.id);
            let binding = policy::binding(&reviewed).unwrap();
            assert!(matches!(
                binding.entry,
                crate::models::platform_entry_projection::StoredEntryIdentity::NativeV2 { .. }
            ));
            assert_eq!(binding.profile_id, profile.id);
            let operation = &manifest.profiles[0].model_operations[0];
            assert_eq!(operation.model_id, model_id);
            assert_eq!(operation.max_input_bytes, 8192);
            assert_eq!(operation.max_output_tokens, 512);
            let producers = crate::services::platform_manager_transport::compose(
                state.clone(),
                Arc::new(NoOwner),
                crate::handlers::agent_skills::preload_context(state.clone()),
            );
            let readiness = producers
                .public
                .probe_public_readiness(&db, &reviewed)
                .await;
            if !matches!(&readiness, Ok(PublicReadiness::Ready(_))) {
                let probes: Vec<PublicProbe> = db
                    .collection::<PublicProbe>(PROBES)
                    .find(doc! {"binding.profile_id":&profile.id})
                    .await
                    .unwrap()
                    .try_collect()
                    .await
                    .unwrap();
                let safe_probes: Vec<_> = probes
            .iter()
            .map(|probe| json!({"state":probe.state,"exact_model_observed":probe.exact_model_observed,"meter_count":probe.meter_row_ids.len()}))
            .collect();
                let ready_records = db
                    .collection::<Document>(DEPLOYMENT_RECORDS)
                    .count_documents(doc! {"profile_id":&profile.id,"status":"ready"})
                    .await
                    .unwrap();
                let provider_paths: Vec<_> = provider
                    .received_requests()
                    .await
                    .unwrap()
                    .iter()
                    .map(|request| format!("{} {}", request.method, request.url.path()))
                    .collect();
                eprintln!(
                    "native initial readiness probes={safe_probes:?} ready_records={ready_records} provider_paths={provider_paths:?} callbacks={:?} runtime_log={}",
                    callback_outcomes.lock().await,
                    log_path.display(),
                );
            }
            let PublicReadiness::Ready(proof) = readiness.unwrap() else {
                panic!("real opaque v2 readiness");
            };
            let actor = crate::services::audit_service::AuditActor::from_auth_user(
                &crate::test_utils::test_auth_user(setup.actor()),
            );
            let controls = controls::update(
                &db,
                &actor,
                state.audit_chain_hmac_key.as_slice(),
                &profile.id,
                &controls::RuntimeChange {
                    public_deployment_digest: Some(proof.policy_digest),
                    initialize: true,
                    expected_profile_generation: Some(profile.generation),
                    expected_manager_authority_epoch: 0,
                    expected_personal_control_revision: 0,
                    expected_public_policy_revision: 0,
                    personal_enabled: Some(true),
                    public_enabled: Some(true),
                    emergency_stopped: None,
                    public_policy: Some(Some(
                        crate::models::platform_bot_runtime_controls::PublicRuntimePolicy {
                            sponsor_owner_user_id: setup.funding().into(),
                            nyxagent_profile: "nyxagent/public-guide".into(),
                            total_daily_credits: Credits::from_whole(10),
                            sender_daily_credits: "0.5".parse().unwrap(),
                        },
                    )),
                },
                &controls::ReadinessAdapter(producers.public.as_ref()),
            )
            .await
            .unwrap();
            let before: BillingWallet = db
                .collection::<BillingWallet>(WALLETS)
                .find_one(doc! {"owner_id":setup.funding()})
                .await
                .unwrap()
                .unwrap();
            let bot = crate::services::channel_bot_service::get_bot(&db, &profile.channel_bot_id)
                .await
                .unwrap();
            let mut payload = native::discord_message(
                "ask",
                Some((
                    "question",
                    "What is NyxID? Help me review linking my account.",
                )),
            );
            payload["token"] = "native-paired-interaction".into();
            let bytes = serde_json::to_vec(&payload).unwrap();
            let signing = ed25519_dalek::SigningKey::from_bytes(&[37; 32]);
            let headers = native::discord_headers(&signing, &bytes, Utc::now().timestamp());
            crate::services::platform_entry_native::authenticate_discord_ack(
                &db,
                transport.adapter.as_ref(),
                &bot.id,
                &headers,
                &bytes,
            )
            .await
            .unwrap();
            let batch = crate::services::platform_entry_native::prepare(
                &db,
                &state.encryption_keys,
                transport.adapter.as_ref(),
                &bot,
                &headers,
                &bytes,
                profile.native_entry.as_ref().unwrap(),
            )
            .await
            .unwrap();
            let mut session = db.client().start_session().await.unwrap();
            session.start_transaction().await.unwrap();
            let admission = crate::services::platform_entry_receipts::record_in_session(
                &db,
                &mut session,
                &state.encryption_keys,
                state.auth_device_hmac_key.as_slice(),
                crate::services::platform_entry_receipts::NativeReceiptInput {
                    profile_id: &profile.id,
                    epoch: controls.manager_authority_epoch,
                    configuration: profile.native_entry.as_ref().unwrap(),
                    batch: &batch,
                    index: 0,
                    relay_message_id: &uuid::Uuid::new_v4().to_string(),
                },
            )
            .await
            .unwrap();
            session.commit_transaction().await.unwrap();
            let IngressAdmission::Claimed {
                audience: ManagerAudience::Public(ingress),
                ..
            } = producers
                .transport
                .admit_manager_update(&db, &admission.verified_update())
                .await
                .unwrap()
            else {
                panic!("real native public admission");
            };
            let context = producers
                .public
                .prepare_public_context(&db, &ingress)
                .await
                .unwrap();
            let destination = ReplyDestination {
                manager: ingress.manager.clone(),
                sender: ingress.sender.clone(),
                authority: ReplyAuthority::Public {
                    ingress: ingress.clone(),
                    public_session_id: context.public_session_id.clone(),
                    turn: context.turn.clone(),
                },
                expires_at:
                    crate::services::platform_public_credential_service::public_reply_expiry(
                        &db, &context,
                    )
                    .await
                    .unwrap(),
            };
            let (events, mut received) = tokio::sync::mpsc::channel(8);
            let final_reply = tokio::time::timeout(std::time::Duration::from_secs(100), async {
        let started = producers
            .public
            .run_public_turn(
                &db,
                context.clone(),
                TurnInput {
                    channel_uploads: Vec::new(),
                    text: admission.text().into(),
                },
                events,
            )
            .await
            .unwrap();
        assert_eq!(started, context.turn);
        let mut reply = None;
        let mut settled = false;
        while let Some(event) = received.recv().await {
            match event {
                ExecutionEvent::ReplyReady(ready) => {
                    assert!(reply.is_none(), "duplicate final reply event");
                    assert_eq!(ready.turn, context.turn);
                    let saved_turn = db
                        .collection::<PlatformPublicTurn>(TURNS)
                        .find_one(doc! {"_id":&ready.turn.turn_id})
                        .await
                        .unwrap()
                        .unwrap();
                    assert_eq!(saved_turn.conversation_id, ready.turn.conversation_id);
                    assert_eq!(saved_turn.public_session_id, context.public_session_id);
                    assert_eq!(saved_turn.reply_id.as_deref(), Some(ready.reply_id.as_str()));
                    if saved_turn.outcome.as_deref() != Some("settled") {
                        let budget = db
                            .collection::<PublicBudgetLoop>(LOOPS)
                            .find_one(doc! {"turn_id":&ready.turn.turn_id})
                            .await
                            .unwrap();
                        let budget = budget.map(|row|json!({
                            "calls":row.calls,"tool_calls":row.tool_calls,
                            "input_tokens":row.input_tokens,"output_tokens":row.output_tokens,
                            "closed":row.closed,
                        }));
                        let provider_paths: Vec<_> = provider
                            .received_requests()
                            .await
                            .unwrap()
                            .iter()
                            .map(|request|format!("{} {}",request.method,request.url.path()))
                            .collect();
                        eprintln!(
                            "native public turn failed; callbacks={:?} mcp_calls={:?} budget={budget:?} provider_paths={provider_paths:?} runtime_log={}",
                            callback_outcomes.lock().await,
                            mcp_calls.lock().await,
                            log_path.display(),
                        );
                    }
                    assert_eq!(
                        saved_turn.outcome.as_deref(),
                        Some("settled"),
                        "public turn must succeed before final delivery; model_callbacks={}",
                        count.load(Ordering::SeqCst),
                    );
                    let saved_answer = crate::services::platform_public_record_service::decrypt_turn(&state.encryption_keys, &saved_turn, "reply_ciphertext")
                    .await
                    .unwrap();
                    assert!(
                        saved_answer.as_str() == CANONICAL_ANSWER,
                        "settled public answer differs from the scripted canonical answer",
                    );
                    let mut retained_delivery = None;
                    loop {
                        let outcome = with_transport_for_test(
                            transport.clone(),
                            crate::services::platform_manager_transport::deliver_saved(
                                &state,
                                &producers,
                                &destination,
                                &ready,
                            ),
                        )
                        .await
                        .unwrap();
                        let stored = db
                            .collection::<PlatformManagerDelivery>(DELIVERIES)
                            .find_one(doc! {"reference.kind":"public_reply","reference.reply_id":&ready.reply_id})
                            .await
                            .unwrap()
                            .unwrap();
                        if let Some(id) = &retained_delivery {
                            assert_eq!(&stored.id, id, "retry must reuse durable delivery");
                        } else {
                            retained_delivery = Some(stored.id.clone());
                        }
                        match outcome {
                            DeliveryOutcome::NativeSent { platform_message_ids } => {
                                assert_eq!(stored.state, DeliveryState::Sent);
                                assert_eq!(platform_message_ids, ["public-final-local-send"]);
                                break;
                            }
                            DeliveryOutcome::Deferred { retry_at } => {
                                assert_eq!(stored.state, DeliveryState::Pending);
                                assert_eq!(stored.retry_at.timestamp_millis(), retry_at.timestamp_millis());
                                let wait = (stored.retry_at - Utc::now())
                                    .to_std()
                                    .unwrap_or_default();
                                tokio::time::sleep(wait + std::time::Duration::from_millis(20)).await;
                            }
                            _ => panic!("final public reply did not send or retain a paced retry"),
                        }
                    }
                    reply = Some(ready);
                }
                ExecutionEvent::Settled { turn } => {
                    assert_eq!(turn, context.turn);
                    assert!(!settled, "duplicate settlement event");
                    settled = true;
                }
            }
        }
        assert!(
            settled,
            "execution channel closed without explicit settlement"
        );
        reply.expect("execution settled without final reply")
    })
    .await
    .expect("public execution, final delivery and completion exceeded shared bound");
            let duplicate = with_transport_for_test(
                transport.clone(),
                crate::services::platform_manager_transport::deliver_saved(
                    &state,
                    &producers,
                    &destination,
                    &final_reply,
                ),
            )
            .await
            .unwrap();
            assert!(matches!(duplicate, DeliveryOutcome::Duplicate));
            assert_eq!(count.load(Ordering::SeqCst), 4);
            assert_eq!(
                *mcp_calls.lock().await,
                ["public_faq_search", "public_faq_read", "public_signup_link"]
            );
            let sends = delivery.received_requests().await.unwrap();
            assert_eq!(sends.len(), 2);
            assert_eq!(provider_sends.load(Ordering::SeqCst), 2);
            assert_eq!(
                sends[0].url.path(),
                "/webhooks/100/native-paired-interaction"
            );
            for send in &sends {
                assert_eq!(send.url.path(), "/webhooks/100/native-paired-interaction");
                assert!(!send.headers.contains_key("authorization"));
            }
            let final_posted: Value = serde_json::from_slice(&sends[1].body).unwrap();
            assert!(
                final_posted["content"] == CANONICAL_ANSWER,
                "delivered public answer differs from the scripted canonical answer",
            );
            assert!(
                !final_posted.to_string().contains("nyx_pli_")
                    && !final_posted.to_string().contains("intent_token")
            );
            let posted: Value = serde_json::from_slice(&sends[0].body).unwrap();
            let link = Zeroizing::new(posted["content"].as_str().unwrap().to_owned());
            assert!(link.contains("#intent_token=nyx_pli_"));
            let rows: Vec<PlatformManagerDelivery> = db
        .collection::<PlatformManagerDelivery>(DELIVERIES)
        .find(doc! {"reference.kind":"public_signup","reference.turn_id":&context.turn.turn_id})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].state, DeliveryState::Sent);
            assert_eq!(
                rows[0].platform_message_ids,
                ["protected-signup-local-send"]
            );
            let final_rows: Vec<PlatformManagerDelivery> = db
        .collection::<PlatformManagerDelivery>(DELIVERIES)
        .find(doc! {"reference.kind":"public_reply","reference.turn_id":&context.turn.turn_id})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
            assert_eq!(final_rows.len(), 1);
            assert_eq!(final_rows[0].state, DeliveryState::Sent);
            assert_eq!(
                final_rows[0].platform_message_ids,
                ["public-final-local-send"]
            );
            let turn = db
                .collection::<PlatformPublicTurn>(TURNS)
                .find_one(doc! {"_id":&context.turn.turn_id})
                .await
                .unwrap()
                .unwrap();
            assert_eq!(turn.outcome.as_deref(), Some("settled"));
            let answer = crate::services::platform_public_record_service::decrypt_turn(
                &state.encryption_keys,
                &turn,
                "reply_ciphertext",
            )
            .await
            .unwrap();
            assert!(
                answer.as_str() == CANONICAL_ANSWER,
                "retained public answer differs from the scripted canonical answer",
            );
            assert!(!answer.contains("nyx_pli_") && !answer.contains("intent_token"));
            let budget = db
                .collection::<PublicBudgetLoop>(LOOPS)
                .find_one(doc! {"turn_id":&context.turn.turn_id})
                .await
                .unwrap()
                .unwrap();
            assert_eq!(budget.maximum, "0.4276224".parse().unwrap());
            assert_eq!(budget.calls, 4);
            assert_eq!(budget.tool_calls, 3);
            assert!(budget.closed);
            assert_eq!(budget.spent, "0.2048".parse().unwrap());
            assert_eq!(budget.held, Credits::ZERO);
            let after: BillingWallet = db
                .collection::<BillingWallet>(WALLETS)
                .find_one(doc! {"owner_id":setup.funding()})
                .await
                .unwrap()
                .unwrap();
            assert_eq!(after.balance_credits, before.balance_credits);
            assert_eq!(after.reserved_credits, Credits::ZERO);
            assert_eq!(
                after
                    .pending_lago_debits
                    .checked_sub(before.pending_lago_debits)
                    .unwrap(),
                budget.spent
            );
            assert_eq!(
                before
                    .available_credits()
                    .unwrap()
                    .checked_sub(after.available_credits().unwrap())
                    .unwrap(),
                budget.spent
            );
            assert_eq!(captured.lock().unwrap().len(), 4);
            assert_eq!(
                provider.received_requests().await.unwrap().len(),
                5,
                "one funded readiness callback plus four actual model callbacks"
            );
            child.kill().await.unwrap();
            child.wait().await.unwrap();
            let _ = stop_callback.send(());
            tokio::time::timeout(std::time::Duration::from_secs(5), callback_worker)
                .await
                .unwrap()
                .unwrap();
            eprintln!("native paired runtime log: {}", log_path.display());
            setup.close().await;
        },
    );
}

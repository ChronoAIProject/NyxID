//! Signed native relay -> engine -> NyxAgent -> authenticated model proxy.
use super::*;
use crate::mw::auth::AuthUser;
use axum::{Extension, http::Request, routing::any};
use tower::ServiceExt;

#[path = "nyxbot_guest_channel_tests.rs"]
mod channel_tests;

async fn model_callback(
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
async fn guest_1995_lark_owner_then_guest_default_profile() {
    Box::pin(lark_model_turns(engine::DEFAULT_MODEL, false)).await;
}

#[tokio::test]
async fn guest_1995_lark_owner_then_guest_configured_profile() {
    Box::pin(lark_model_turns("nyxagent/research", false)).await;
}

#[tokio::test]
async fn guest_1995_lark_legacy_channel_profile_records_thread_route() {
    Box::pin(lark_model_turns(engine::DEFAULT_MODEL, true)).await;
}

async fn lark_model_turns(profile: &str, legacy: bool) {
    use crate::models::assistant_agent::AssistantAgent;
    use crate::services::assistant_team_service::{CreateRequest, GrantTargets, create_specialist};
    let (state, _, old_server) = setup("guest_1995_lark_model").await;
    old_server.abort();
    crate::test_utils::set_test_approval_preference(&state.db, OWNER, false).await;
    let (specialist, _) = create_specialist(
        &state.db,
        &state.encryption_keys,
        OWNER,
        CreateRequest {
            machines: None,
            logins: None,
            name: "office-agent".into(),
            description: "Group specialist".into(),
            display_name: None,
            persona: None,
            targets: GrantTargets {
                service_ids: vec![],
                platform_service_ids: vec![],
                slugs: vec![],
                ids_by_request: Default::default(),
            },
            account_read: false,
            specialty: None,
            created_by: "nyxbot",
        },
    )
    .await
    .unwrap()
    .unwrap();
    state
        .db
        .collection::<AssistantAgent>("assistant_agents")
        .update_one(doc! {"_id":&specialist.id}, doc! {"$set":{"model":profile}})
        .await
        .unwrap();
    let (channel, _) = channel(&state, "direct").await;
    link(&state, OWNER, &channel.id, &specialist).await.unwrap();
    state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .update_one(
            doc! {"_id":&channel.id},
            doc! {"$set":{"owner_sender_ids":["ou_owner"]}},
        )
        .await
        .unwrap();
    TEST_BOT_USER_IDS
        .lock()
        .unwrap()
        .insert(channel.channel_bot_id.clone(), "ou_bot".into());
    let channel = load_channel(&state, OWNER, &channel.id).await.unwrap();

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
    model.slug = "relay-model".into();
    model.base_url = provider.uri();
    model.auth_method = "none".into();
    model.requires_user_credential = false;
    model.credential_encrypted.clear();
    model.inference =
        Some(serde_json::from_value(json!({"wire_protocol":"openai_responses"})).unwrap());
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
        .collection::<bson::Document>("user_services")
        .update_one(
            doc! {"_id":&connection},
            doc! {"$set":{"catalog_service_id":&model.id}},
        )
        .await
        .unwrap();

    let callbacks = Router::new()
        .route("/api/v1/proxy/s/{slug}/{*path}", any(model_callback))
        .layer(Extension(
            crate::handlers::assistant_nyxagent::SERVER_TURN_POLICY,
        ))
        .with_state(state.clone());
    let observations: Calls = Default::default();
    let sink = observations.clone();
    let db = state.db.clone();
    let upstream = Router::new().route("/v1/responses", post(move |headers: HeaderMap, Json(body): Json<Value>| {
        let callbacks = callbacks.clone(); let sink = sink.clone(); let db = db.clone();
        async move {
            let bearer = headers.get("authorization").unwrap().clone();
            let raw = bearer.to_str().unwrap().strip_prefix("Bearer ").unwrap();
            let (_, key, _) = crate::services::key_service::validate_api_key(&db, raw).await.unwrap();
            let row = db.collection::<AssistantConversation>(CONVERSATIONS).find_one(doc!{"credential_api_key_id":&key.id}).await.unwrap().unwrap();
            let turn = row.active_turn.as_ref().unwrap();
            assert!(!turn.stop_requested);
            for kind in ["input_admission", "model_attempt"] {
                let mut filter = doc!{"conversation_id":&row.id,"turn_id":&turn.turn_id,"api_key_id":&key.id,"kind":kind};
                if kind == "model_attempt" { filter.insert("active", true); }
                assert!(db.collection::<bson::Document>("assistant_service_execution").find_one(filter).await.unwrap().is_some(), "missing {kind}");
            }
            // NyxAgent's bridge uses reqwest JSON with these headers. Prompt,
            // instructions and function tools differ between owner and guest.
            let generation = json!({"model":"provider-model","store":false,"stream":true,
                "input":body["input"], "instructions":body["instructions"],
                "tools":[{"type":"function","name":"nyxid_tool","parameters":{"type":"object"}}],
                "parallel_tool_calls":true,"reasoning":{"effort":"low"},
                "include":["reasoning.encrypted_content"],"prompt_cache_key":Uuid::new_v4().to_string(),"tool_choice":"auto",
                // Codex 0.160.1's actual wire shape; NyxAgent's personal bridge
                // forwards this object (only public-mode callbacks remove it).
                "client_metadata":{"root_turn_id":Uuid::new_v4().to_string(),"session_id":Uuid::new_v4().to_string(),
                    "thread_id":Uuid::new_v4().to_string(),"turn_id":Uuid::new_v4().to_string(),
                    "x-codex-installation-id":"test-installation","x-codex-turn-metadata":"{\"sandbox_mode\":\"read-only\"}",
                    "x-codex-window-id":Uuid::new_v4().to_string()}});
            let response = callbacks.oneshot(Request::builder().method("POST").uri("/api/v1/proxy/s/relay-model/responses")
                .header("authorization",bearer).header("content-type","application/json").header("accept","*/*")
                .body(Body::from(generation.to_string())).unwrap()).await.unwrap();
            let status = response.status().as_u16();
            let bytes = axum::body::to_bytes(response.into_body(), 1024*1024).await.unwrap();
            sink.lock().await.push(json!({"status":status,"body":serde_json::from_slice::<Value>(&bytes).unwrap(),
                "profile":body["model"],"conversation":row.id,"guest":row.guest_turn}));
            let failed = status != 200;
            let session = format!("conv_{}", Uuid::new_v4().simple());
            let event = json!({"type":if failed {"response.failed"} else {"response.completed"},"sequence_number":0,
                "response":{"id":format!("resp_{}_{}", &session[5..],Uuid::new_v4().simple()),"conversation":{"id":session},
                    "status":if failed {"failed"} else {"completed"},
                    "error":if failed {json!({"code":"turn_failed","message":"private upstream detail"})} else {Value::Null},
                    "output":if failed {json!([])} else {json!([{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Hello"}]}])}}});
            axum::response::Sse::new(futures::stream::iter([Ok::<_,Infallible>(Event::default().data(event.to_string()))])).into_response()
        }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, upstream).await.unwrap();
    });
    state
        .db
        .collection::<DownstreamService>(SERVICES)
        .update_one(
            doc! {"slug":engine::SERVICE_SLUG},
            doc! {"$set":{"base_url":format!("http://{address}")}},
        )
        .await
        .unwrap();
    let turns = if legacy {
        vec![
            ("seed", "ou_owner"),
            ("owner", "ou_owner"),
            ("guest", "ou_guest"),
        ]
    } else {
        vec![("owner", "ou_owner"), ("guest", "ou_guest")]
    };
    for (id, sender) in turns {
        if legacy && id == "owner" {
            // A pre-upgrade shared channel retains the old global profile,
            // while its specialist now has a different configured profile.
            state.db.collection::<AssistantAgent>("assistant_agents").update_one(doc!{"_id":&specialist.id},
                doc!{"$set":{"model":"nyxagent/research","owner_model_route":bson::Bson::Null}}).await.unwrap();
            state
                .db
                .collection::<AssistantConversation>(CONVERSATIONS)
                .update_many(
                    doc! {"agent_id":&specialist.id},
                    doc! {"$unset":{"owner_model_route":""}},
                )
                .await
                .unwrap();
        }
        let (_guard, completed) = watch_relay_callback(&channel.id, id);
        let body = json!({"message_id":id,"correlation_id":format!("jti-{id}"),"platform":"lark",
            "agent":{"api_key_id":channel.route_api_key_id,"name":"route"},
            "conversation":{"id":"route","platform_id":"oc_test","type":"group"},
            "sender":{"platform_id":sender},"content":{"type":"text","text":format!("@_user_1 hello {id}")},
            "timestamp":"2026-09-30T00:00:00Z",
            "raw_platform_data":{"event":{"sender":{"sender_type":"user","sender_id":{"open_id":sender}},
                "message":{"mentions":[{"key":"@_user_1","id":{"open_id":"ou_bot"},"name":"Helper bot"}]}}}});
        let bytes = serde_json::to_vec(&body).unwrap();
        let token = crate::crypto::jwt::generate_relay_callback_token(
            &state.jwt_keys,
            &state.config,
            &format!("jti-{id}"),
            &channel.route_api_key_id,
            id,
            "lark",
            &sha256_hex(&bytes),
        )
        .unwrap();
        let mut headers = HeaderMap::new();
        headers.insert("x-nyxid-callback-token", token.parse().unwrap());
        assert_eq!(
            relay_callback(
                State(state.clone()),
                Path(channel.id.clone()),
                headers,
                Bytes::from(bytes)
            )
            .await
            .status(),
            StatusCode::ACCEPTED
        );
        tokio::time::timeout(Duration::from_secs(20), completed)
            .await
            .unwrap()
            .unwrap();
    }
    let all_seen = observations.lock().await;
    let seen = if legacy {
        &all_seen[1..]
    } else {
        all_seen.as_slice()
    };
    assert_eq!(seen.len(), 2, "{seen:?}");
    assert_eq!(seen[0]["status"], 200, "{seen:?}");
    assert_eq!(seen[0]["guest"], false);
    assert_eq!(seen[1]["guest"], true);
    assert_eq!(seen[0]["conversation"], seen[1]["conversation"]);
    assert_eq!(
        seen[1]["status"], 200,
        "owner success must establish guest route: {seen:?}"
    );
    assert_eq!(
        seen[0]["profile"], profile,
        "channel must inherit specialist profile"
    );
    let agent = state
        .db
        .collection::<AssistantAgent>("assistant_agents")
        .find_one(doc! {"_id":&specialist.id})
        .await
        .unwrap()
        .unwrap();
    if legacy {
        assert!(
            agent.owner_model_route.is_none(),
            "thread override must not replace agent default"
        );
    } else {
        assert_eq!(agent.owner_model_route.unwrap().profile, profile);
    }
    let row = state
        .db
        .collection::<AssistantConversation>(CONVERSATIONS)
        .find_one(doc! {"_id":seen[0]["conversation"].as_str().unwrap()})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.owner_model_route.unwrap().profile, profile);
    assert_eq!(
        provider.received_requests().await.unwrap().len(),
        if legacy { 3 } else { 2 }
    );
    server.abort();
    TEST_BOT_USER_IDS
        .lock()
        .unwrap()
        .remove(&channel.channel_bot_id);
    state.db.drop().await.unwrap();
}

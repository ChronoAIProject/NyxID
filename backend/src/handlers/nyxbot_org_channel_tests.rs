//! Exercises the existing provider protocol with independent org execution identity.
use super::*;
use crate::services::assistant_team_service as team;
use crate::services::{feature_flag_service as flags, org_channel_service as org};

#[tokio::test]
async fn org_channel_transport_moves_wait_for_member_and_guest_execution_actors() {
    let (state, _, server) = setup("org_channel_move_active_actors").await;
    let (row, _, _, org_id) = org_channel(&state, false).await;
    let member_id = Uuid::new_v4().to_string();
    let binding = row.org_specialist.as_ref().unwrap();
    let conversations = state
        .db
        .collection::<bson::Document>("assistant_conversations");
    for (field, actor) in [
        ("reply_channel", member_id.as_str()),
        ("channel", org_id.as_str()),
    ] {
        let id = Uuid::new_v4().to_string();
        conversations
            .insert_one(doc! {"_id":&id,"user_id":actor,
                "active_turn":{"started_at":bson::DateTime::now()},
                field:{"nyxbot_channel_id":&row.id,"org_execution":{
                    "org_id":&org_id,"revision":"retired-binding"}}
            })
            .await
            .unwrap();
        assert!(!channel_answering(&state, &row).await.unwrap());
        conversations
            .update_one(
                doc! {"_id":&id},
                doc! {"$set":{
                    format!("{field}.org_execution.revision"):&binding.revision
                }},
            )
            .await
            .unwrap();
        assert!(channel_answering(&state, &row).await.unwrap(), "{field}");
        conversations
            .update_one(
                doc! {"_id":&id},
                doc! {"$set":{
                    "active_turn.started_at":bson::DateTime::from_chrono(Utc::now()
                        - ChronoDuration::seconds(engine::ACTIVE_TURN_TTL_SECS + 1))
                }},
            )
            .await
            .unwrap();
        assert!(!channel_answering(&state, &row).await.unwrap());
        conversations.delete_one(doc! {"_id":&id}).await.unwrap();
    }
    let mut personal = row.clone();
    personal.org_specialist = None;
    let id = Uuid::new_v4().to_string();
    conversations
        .insert_one(doc! {"_id":&id,"user_id":&member_id,
            "active_turn":{"started_at":bson::DateTime::now()},
            "channel":{"nyxbot_channel_id":&row.id}
        })
        .await
        .unwrap();
    assert!(!channel_answering(&state, &personal).await.unwrap());
    conversations
        .update_one(doc! {"_id":&id}, doc! {"$set":{"user_id":OWNER}})
        .await
        .unwrap();
    assert!(channel_answering(&state, &personal).await.unwrap());
    server.abort();
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn org_channel_human_connect_and_disconnect_keep_same_org_transport_authority() {
    use tower::ServiceExt;
    let (state, _, server) = setup("org_channel_human_lifecycle").await;
    let (_, _, agent, org_id) = org_channel(&state, false).await;
    flags::set_platform_override(
        &state.db,
        flags::NYXAGENT_ENGINE_FLAG_KEY,
        &flags::FlagTarget::Global,
        true,
        OWNER,
    )
    .await
    .unwrap();
    let mut bot = bot_doc("lark", "Organization support");
    bot.insert("user_id", &org_id);
    let bot_id = bot.get_str("_id").unwrap().to_owned();
    state
        .db
        .collection::<bson::Document>("channel_bots")
        .insert_one(bot)
        .await
        .unwrap();
    let personal_bot = bot_doc("lark", "Personal bot");
    let personal_id = personal_bot.get_str("_id").unwrap().to_owned();
    state
        .db
        .collection::<bson::Document>("channel_bots")
        .insert_one(personal_bot)
        .await
        .unwrap();
    let token = crate::crypto::jwt::generate_access_token(
        &state.jwt_keys,
        &state.config,
        &Uuid::parse_str(OWNER).unwrap(),
        "openid",
        None,
        None,
        None,
        None,
        None,
    )
    .unwrap();
    let app = Router::new()
        .route("/api/v1/assistant/nyxagent/channels", post(connect_channel))
        .route(
            "/api/v1/assistant/nyxagent/channels/{id}",
            axum::routing::delete(disconnect_channel),
        )
        .with_state(state.clone());
    let request = |method: &str, path: &str, body: Value| {
        axum::http::Request::builder()
            .method(method)
            .uri(path)
            .header("authorization", format!("Bearer {token}"))
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    };
    let path = "/api/v1/assistant/nyxagent/channels";
    let refused = Box::pin(app.clone().oneshot(request(
        "POST",
        path,
        json!({"bot_id":personal_id,"agent_id":agent.id}),
    )))
    .await
    .unwrap();
    assert!(
        !refused.status().is_success(),
        "A personal bot cannot expose an org specialist"
    );
    assert!(
        active_for_bot(&state, OWNER, &personal_id)
            .await
            .unwrap()
            .is_none()
    );
    let created = Box::pin(app.clone().oneshot(request(
        "POST",
        path,
        json!({"bot_id":bot_id,"agent_id":agent.id}),
    )))
    .await
    .unwrap();
    let status = created.status();
    let body = body_text(created).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let body: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(body["channel_agent"]["org_specialist"]["org_id"], org_id);
    assert_eq!(
        body["channel_agent"]["org_specialist"]["guest_enabled"],
        false
    );
    let row = active_for_bot(&state, OWNER, &bot_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(row.user_id, OWNER);
    assert_eq!(row.bot_owner_id.as_deref(), Some(org_id.as_str()));
    assert!(row.owner_sender_ids.is_empty());
    assert_eq!(
        key_service::get_api_key(&state.db, &org_id, &row.route_api_key_id)
            .await
            .unwrap()
            .user_id,
        org_id
    );
    let disconnected = Box::pin(app.oneshot(request(
        "DELETE",
        &format!("{path}/{}", row.id),
        Value::Null,
    )))
    .await
    .unwrap();
    let status = disconnected.status();
    let body = body_text(disconnected).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert!(
        active_for_bot(&state, OWNER, &bot_id)
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        state
            .db
            .collection::<bson::Document>("api_keys")
            .count_documents(doc! {"_id":&row.route_api_key_id,"is_active":true})
            .await
            .unwrap(),
        0
    );
    server.abort();
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn org_specialist_provider_uses_confirmed_member_thread_dedup_and_live_authority() {
    let (state, calls, server) = setup("org_specialist_provider").await;
    let (row, key, agent, org_id) = org_channel(&state, false).await;
    assert_eq!(
        put_binding(
            State(state.clone()),
            Path("bnd_turns".into()),
            bearer(&key),
            Json(binding_body(&key, OWNER))
        )
        .await
        .status(),
        StatusCode::OK
    );
    ensure_org_partition(&state, &key).await;
    let denied = body_text(
        respond(
            &state,
            &key,
            &event("hello", "7", "org-unverified"),
            "org-unverified",
        )
        .await,
    )
    .await;
    assert!(
        denied.contains("Verify your organization membership"),
        "{denied}"
    );
    assert!(calls.lock().await.is_empty());
    let challenge = org::links::issue(&state.db, OWNER, &row).await.unwrap();
    let observed = body_text(
        respond(
            &state,
            &key,
            &event(challenge["code"].as_str().unwrap(), "7", "org-code"),
            "org-code",
        )
        .await,
    )
    .await;
    assert!(observed.contains("Return to NyxID"), "{observed}");
    assert!(calls.lock().await.is_empty());
    org::links::confirm(&state.db, OWNER, &row, challenge["id"].as_str().unwrap())
        .await
        .unwrap();
    let body = event("Summarize today's request", "7", "org-first");
    let answer = body_text(respond(&state, &key, &body, "org-first").await).await;
    assert!(answer.contains("Here is your answer"), "{answer}");
    assert_eq!(calls.lock().await.len(), 1);
    let stored:AssistantConversation=state.db.collection(CONVERSATIONS).find_one(doc!{"user_id":OWNER,"agent_id":&agent.id,"reply_channel.org_execution":{"$type":"object"}}).await.unwrap().unwrap();
    assert_eq!(stored.agent_owner_id.as_deref(), Some(org_id.as_str()));
    assert!(!stored.guest_turn && stored.channel.is_none());
    assert!(
        stored
            .reply_channel
            .as_ref()
            .unwrap()
            .org_execution
            .as_ref()
            .is_some_and(|e| e.actor_id == OWNER && !e.guest)
    );
    let retry = respond(&state, &key, &body, "org-first").await;
    let _ = body_text(retry).await;
    assert_eq!(
        calls.lock().await.len(),
        1,
        "provider retry must not start another turn"
    );
    state
        .db
        .collection::<bson::Document>("org_memberships")
        .update_one(
            doc! {"org_user_id":&org_id,"member_user_id":OWNER},
            doc! {"$set":{"role":"member"}},
        )
        .await
        .unwrap();
    let refused = respond(
        &state,
        &key,
        &event("again", "7", "org-demoted"),
        "org-demoted",
    )
    .await;
    assert!(!refused.status().is_success());
    assert_eq!(calls.lock().await.len(), 1);
    server.abort();
    state.db.drop().await.unwrap();
}

async fn org_channel(
    state: &AppState,
    native: bool,
) -> (
    NyxbotChannel,
    String,
    crate::models::assistant_agent::AssistantAgent,
    String,
) {
    crate::services::security_readiness_service::ready_test_replica(&state.db).await;
    org::ensure_indexes(&state.db).await.unwrap();
    for flag in [org::FLAG, "assistant:org-agents"] {
        flags::set_platform_override(&state.db, flag, &flags::FlagTarget::Global, true, OWNER)
            .await
            .unwrap();
    }
    let org_id = Uuid::new_v4().to_string();
    state
        .db
        .collection(USERS)
        .insert_one(test_user(&org_id, UserType::Org))
        .await
        .unwrap();
    state
        .db
        .collection("org_memberships")
        .insert_one(crate::test_utils::test_membership(
            &org_id,
            OWNER,
            crate::models::org_membership::OrgRole::Admin,
            None,
        ))
        .await
        .unwrap();
    let (agent, _) = team::create_specialist_for(
        &state.db,
        &state.encryption_keys,
        OWNER,
        &org_id,
        team::CreateRequest {
            name: "org-channel-specialist".into(),
            description: "Organization support".into(),
            display_name: None,
            persona: None,
            targets: Default::default(),
            account_read: false,
            machines: None,
            logins: None,
            specialty: None,
            created_by: "user",
        },
    )
    .await
    .unwrap()
    .unwrap();
    let (mut row, key) = if native {
        native_tests::native_channel(state).await
    } else {
        channel(state, "gateway").await
    };
    state
        .db
        .collection::<bson::Document>("channel_bots")
        .update_one(
            doc! {"_id":&row.channel_bot_id},
            doc! {"$set":{"user_id":&org_id}},
        )
        .await
        .unwrap();
    let bot = channel_bot_service::get_bot(&state.db, &row.channel_bot_id)
        .await
        .unwrap();
    let binding = org::link_target(&state.db, OWNER, &bot, &agent)
        .await
        .unwrap()
        .unwrap();
    state
        .db
        .collection::<bson::Document>("api_keys")
        .update_one(
            doc! {"_id":&row.route_api_key_id},
            doc! {"$set":{"user_id":&org_id}},
        )
        .await
        .unwrap();
    state
        .db
        .collection::<bson::Document>("channel_conversations")
        .update_one(
            doc! {"_id":&row.route_id},
            doc! {"$set":{"user_id":&org_id}},
        )
        .await
        .unwrap();
    state.db.collection::<bson::Document>(CHANNELS).update_one(doc!{"_id":&row.id},doc!{"$set":{"bot_owner_id":&org_id,"agent_id":&agent.id,"org_specialist":bson::to_bson(&binding).unwrap()}}).await.unwrap();
    row.bot_owner_id = Some(org_id.clone());
    row.agent_id = Some(agent.id.clone());
    row.org_specialist = Some(binding);
    (row, key, agent, org_id)
}

async fn ensure_org_partition(state: &AppState, key: &str) {
    let response = put_conversation(
        State(state.clone()),
        Path(("bnd_turns".into(), PARTITION.into())),
        bearer(key),
    )
    .await;
    let status = response.status();
    assert_eq!(status, StatusCode::OK, "{}", body_text(response).await);
}

#[tokio::test]
async fn org_channel_cold_guest_model_setup_and_real_platform_model_bill_only_the_org() {
    use crate::models::credits::Credits;
    use tower::ServiceExt;
    let (mut state, _, old) = setup("org_channel_cold_model").await;
    old.abort();
    let (row, key, agent, org_id) = org_channel(&state, false).await;
    crate::test_utils::set_test_approval_preference(&state.db, &org_id, false).await;
    crate::billing_integration_tests::create_usage_index(&state.db).await;
    state.billing = Arc::new(crate::billing_integration_tests::billing_service(
        &state.db,
        Arc::new(crate::billing_integration_tests::FakeLago::default()),
        1000,
    ));
    state.billing.ensure_wallet(&org_id).await.unwrap();
    for metric in ["platform_tokens", "platform_requests", "platform_bytes"] {
        let mut rate = crate::billing_integration_tests::rate(metric, Utc::now());
        // A realistic model price leaves room for setup plus the first answer
        // within the intentionally small one-credit binding budget.
        rate.credits_per_unit_micros = 1;
        state
            .db
            .collection("billing_rate_cache")
            .insert_one(rate)
            .await
            .unwrap();
    }
    let provider = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST")).respond_with(wiremock::ResponseTemplate::new(200)
        .set_body_json(json!({"id":"model-answer","model":"fixture-model","output":[],"usage":{"input_tokens":7,"output_tokens":3,"total_tokens":10}}))).mount(&provider).await;
    let catalog = state
        .db
        .collection::<DownstreamService>(SERVICES)
        .find_one(doc! {"slug":"fixture-model"})
        .await
        .unwrap()
        .unwrap();
    state
        .db
        .collection::<bson::Document>(SERVICES)
        .update_one(
            doc! {"_id":&catalog.id},
            doc! {"$set":{
            "base_url":provider.uri(),"platform_key":{"enabled":true,"audience":"public"},
            "billing":{"platform_billable":true,"platform_charge_nyxid_credentials_only":false}}},
        )
        .await
        .unwrap();
    let connection = crate::services::assistant_authority_tests::connected(
        &state.db,
        &org_id,
        "fixture-model",
        &provider.uri(),
    )
    .await;
    state.db.collection::<bson::Document>("user_services").update_one(doc!{"_id":&connection},doc!{"$set":{
        "catalog_service_id":&catalog.id,"credential_binding":"platform","auth_method":"bearer","source":"manual"}}).await.unwrap();
    org::management::update(
        &state.db,
        OWNER,
        &row,
        org::management::GuestSettings {
            enabled: true,
            service_ids: vec![],
            daily_credits: Credits::from_whole(1),
        },
    )
    .await
    .unwrap();
    state
        .db
        .collection::<bson::Document>(CHANNELS)
        .update_one(
            doc! {"_id":&row.id},
            doc! {"$set":{"private_chats":"everyone"}},
        )
        .await
        .unwrap();
    let callbacks = Router::new()
        .route(
            "/api/v1/proxy/s/{slug}/{*path}",
            axum::routing::any(guest_model_tests::model_callback),
        )
        .layer(axum::Extension(
            crate::handlers::assistant_nyxagent::SERVER_TURN_POLICY,
        ))
        .with_state(state.clone());
    let observations: Calls = Default::default();
    let sink = observations.clone();
    let db = state.db.clone();
    let upstream=Router::new().route("/v1/responses",post(move |headers:HeaderMap,Json(body):Json<Value>| {
        let callbacks=callbacks.clone();let sink=sink.clone();let db=db.clone();
        async move {
            let bearer=headers["authorization"].clone();
            let (_,key,_)=key_service::validate_api_key(&db,bearer.to_str().unwrap().strip_prefix("Bearer ").unwrap()).await.unwrap();
            let conversation=db.collection::<AssistantConversation>(CONVERSATIONS).find_one(doc!{"credential_api_key_id":&key.id}).await.unwrap().unwrap();
            let response=callbacks.oneshot(axum::http::Request::builder().method("POST").uri("/api/v1/proxy/s/fixture-model/responses")
                .header("authorization",bearer).header("content-type","application/json").header("accept","*/*")
                .body(Body::from(json!({"model":"fixture-model","store":false,"input":body["input"],"stream":false}).to_string())).unwrap()).await.unwrap();
            let status=response.status();let bytes=axum::body::to_bytes(response.into_body(),1024*1024).await.unwrap();
            sink.lock().await.push(json!({"actor":conversation.user_id,"setup":conversation.guest_model_setup,"guest":conversation.guest_turn,"status":status.as_u16(),"body":String::from_utf8_lossy(&bytes)}));
            let session=format!("conv_{}",Uuid::new_v4().simple());
            let event=json!({"type":"response.completed","sequence_number":0,"response":{"id":format!("resp_{}_{}",&session[5..],Uuid::new_v4().simple()),
                "conversation":{"id":session},"status":"completed","output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Organization answer"}]}]}});
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
        .collection::<bson::Document>(SERVICES)
        .update_one(
            doc! {"slug":engine::SERVICE_SLUG},
            doc! {"$set":{"base_url":format!("http://{address}")}},
        )
        .await
        .unwrap();
    assert_eq!(
        put_binding(
            State(state.clone()),
            Path("bnd_turns".into()),
            bearer(&key),
            Json(binding_body(&key, OWNER))
        )
        .await
        .status(),
        StatusCode::OK
    );
    ensure_org_partition(&state, &key).await;
    let body = event("Hello, first visitor", "7", "org-cold-first");
    let response = body_text(respond(&state, &key, &body, "org-cold-first").await).await;
    assert!(
        response.contains("Organization answer"),
        "{response}; callbacks={:?}",
        observations.lock().await
    );
    let seen = observations.lock().await.clone();
    assert_eq!(
        seen.len(),
        2,
        "one hidden setup and one guest model call: {seen:?}"
    );
    assert_eq!(seen[0]["setup"], true);
    assert_eq!(seen[1]["guest"], true);
    for call in &seen {
        assert_eq!(call["actor"], org_id);
        assert_eq!(call["status"], 200, "{call:?}");
    }
    assert_eq!(
        state
            .db
            .collection::<bson::Document>("assistant_acknowledgements")
            .count_documents(doc! {"user_id":&org_id})
            .await
            .unwrap(),
        0,
        "delegated model setup must not ask a public guest for an approval"
    );
    let stored = state
        .db
        .collection::<crate::models::assistant_agent::AssistantAgent>("assistant_agents")
        .find_one(doc! {"_id":&agent.id})
        .await
        .unwrap()
        .unwrap();
    assert!(stored.owner_model_route.is_some());
    let rows: Vec<crate::models::usage_meter::UsageMeterRow> = state
        .db
        .collection("usage_meter")
        .find(doc! {"service_slug":"fixture-model"})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert_eq!(rows.len(), 2);
    for usage in rows {
        assert_eq!(usage.billing_owner_id, org_id);
        assert!(usage.org_channel_guest.is_some());
        assert_eq!(
            usage.credential_class,
            crate::models::usage_meter::CredentialClass::NyxidManagedMaster
        );
    }
    assert_eq!(
        state
            .db
            .collection::<bson::Document>("billing_wallet")
            .count_documents(doc! {"owner_id":OWNER})
            .await
            .unwrap(),
        0,
        "transport principal must not become a payer"
    );
    server.abort();
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn org_channel_native_discord_recovery_keeps_member_identity_and_refuses_revoked_member() {
    let (state, calls, server) = setup("org_channel_native_discord").await;
    let (row, key, agent, org_id) = org_channel(&state, true).await;
    let member = Uuid::new_v4().to_string();
    state
        .db
        .collection(USERS)
        .insert_one(test_user(&member, UserType::Person))
        .await
        .unwrap();
    state
        .db
        .collection("org_memberships")
        .insert_one(crate::test_utils::test_membership(
            &org_id,
            &member,
            crate::models::org_membership::OrgRole::Member,
            None,
        ))
        .await
        .unwrap();
    let challenge = org::links::issue(&state.db, &member, &row).await.unwrap();
    let observed = body_text(
        respond(
            &state,
            &key,
            &native_tests::native_event(&row, "700", challenge["code"].as_str().unwrap()),
            "700",
        )
        .await,
    )
    .await;
    assert!(observed.contains("Return to NyxID"), "{observed}");
    org::links::confirm(&state.db, &member, &row, challenge["id"].as_str().unwrap())
        .await
        .unwrap();
    let event = native_tests::native_event(&row, "701", "Member's native request");
    let response = respond(&state, &key, &event, "701").await;
    assert_eq!(response.status(), StatusCode::OK);
    let response_id = response.headers()["x-cma-response-id"]
        .to_str()
        .unwrap()
        .to_owned();
    let answer = body_text(response).await;
    assert!(answer.contains("Here is your answer"), "{answer}");
    assert_eq!(calls.lock().await.len(), 1);
    let stored = state
        .db
        .collection::<AssistantConversation>(CONVERSATIONS)
        .find_one(doc! {"user_id":&member,"agent_id":&agent.id})
        .await
        .unwrap()
        .unwrap();
    assert!(stored.channel.is_none());
    assert!(
        super::repeated_question(
            &state,
            &row,
            &stored.id,
            "already-settled",
            stored.reply_channel.as_ref().unwrap(),
        )
        .await
        .unwrap()
        .is_none(),
        "repeat detection must read the actor's conversation, not the transport admin's"
    );
    let source = state
        .db
        .collection::<NyxbotEvent>(EVENTS)
        .find_one(
            doc! {"_id":stored.reply_channel.as_ref().unwrap().native_event_id.as_ref().unwrap()},
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(source.event_id, "discord:100:701");
    assert!(
        crate::services::nyxbot_response_recovery::load(&state.db, &row, &response_id)
            .await
            .unwrap()
            .is_some()
    );
    let retry = body_text(respond(&state, &key, &event, "701").await).await;
    assert!(retry.contains("Here is your answer"));
    assert_eq!(calls.lock().await.len(), 1);
    let gateway = wiremock::MockServer::start().await;
    let mut service = crate::models::downstream_service::test_helpers::dummy_service();
    service.id = Uuid::new_v4().to_string();
    service.slug = GATEWAY_SLUG.into();
    service.base_url = gateway.uri();
    state
        .db
        .collection::<DownstreamService>(SERVICES)
        .insert_one(service)
        .await
        .unwrap();
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path(
            "/events/opaque-native-ref/replies",
        ))
        .respond_with(
            wiremock::ResponseTemplate::new(200)
                .set_body_json(json!({"delivery":{"outcome":"confirmed"}})),
        )
        .expect(1)
        .mount(&gateway)
        .await;
    native_responses::send_update(
        &state,
        &row,
        stored.reply_channel.as_ref().unwrap(),
        &stored.id,
        "Member update",
    )
    .await
    .unwrap();
    crate::services::org_service::revoke_membership(&state.db, &org_id, &member)
        .await
        .unwrap();
    assert!(
        crate::services::nyxbot_response_recovery::load(&state.db, &row, &response_id)
            .await
            .is_err()
    );
    assert_eq!(calls.lock().await.len(), 1);
    assert!(
        native_responses::send_update(
            &state,
            &row,
            stored.reply_channel.as_ref().unwrap(),
            &stored.id,
            "Revoked update"
        )
        .await
        .is_err()
    );
    assert_eq!(gateway.received_requests().await.unwrap().len(), 1);
    server.abort();
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn org_channel_signed_relay_uses_verified_actor_and_deduplicates() {
    let (state, calls, server) = setup("org_channel_relay_identity").await;
    let (mut row, _, agent, org_id) = org_channel(&state, false).await;
    row.transport = "direct".into();
    row.platform = "lark".into();
    state
        .db
        .collection::<bson::Document>("channel_bots")
        .update_one(
            doc! {"_id":&row.channel_bot_id},
            doc! {"$set":{"platform":"lark"}},
        )
        .await
        .unwrap();
    state
        .db
        .collection::<bson::Document>("channel_conversations")
        .update_one(
            doc! {"_id":&row.route_id},
            doc! {"$set":{"platform":"lark"}},
        )
        .await
        .unwrap();
    state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .replace_one(doc! {"_id":&row.id}, &row)
        .await
        .unwrap();
    let challenge = org::links::issue(&state.db, OWNER, &row).await.unwrap();
    for (id, text) in [
        ("verify", challenge["code"].as_str().unwrap()),
        ("question", "Relay member request"),
    ] {
        let body = json!({"message_id":id,"correlation_id":format!("relay-{id}"),"platform":"lark",
            "agent":{"api_key_id":row.route_api_key_id,"name":"route"},
            "conversation":{"id":row.route_id,"platform_id":"org-private","type":"private"},
            "sender":{"platform_id":"org-member"},"content":{"type":"text","text":text},
            "raw_platform_data":{"event":{"sender":{"sender_type":"user"}}}});
        let bytes = serde_json::to_vec(&body).unwrap();
        let token = crate::crypto::jwt::generate_relay_callback_token(
            &state.jwt_keys,
            &state.config,
            &format!("relay-{id}"),
            &row.route_api_key_id,
            id,
            "lark",
            &sha256_hex(&bytes),
        )
        .unwrap();
        let mut headers = HeaderMap::new();
        headers.insert("x-nyxid-callback-token", token.parse().unwrap());
        // A producer cannot forge a member event without NyxID's signed envelope.
        assert_eq!(
            relay_callback(
                State(state.clone()),
                Path(row.id.clone()),
                HeaderMap::new(),
                Bytes::from(bytes.clone())
            )
            .await
            .status(),
            StatusCode::UNAUTHORIZED
        );
        let (_watch, done) = watch_relay_callback(&row.id, id);
        assert_eq!(
            relay_callback(
                State(state.clone()),
                Path(row.id.clone()),
                headers.clone(),
                Bytes::from(bytes.clone())
            )
            .await
            .status(),
            StatusCode::ACCEPTED
        );
        tokio::time::timeout(std::time::Duration::from_secs(20), done)
            .await
            .unwrap()
            .unwrap();
        if id == "verify" {
            assert!(calls.lock().await.is_empty());
            org::links::confirm(&state.db, OWNER, &row, challenge["id"].as_str().unwrap())
                .await
                .unwrap();
        } else {
            assert_eq!(calls.lock().await.len(), 1);
            assert_eq!(
                relay_callback(
                    State(state.clone()),
                    Path(row.id.clone()),
                    headers,
                    Bytes::from(bytes)
                )
                .await
                .status(),
                StatusCode::ACCEPTED
            );
            assert_eq!(calls.lock().await.len(), 1);
        }
    }
    let conversation = state.db.collection::<AssistantConversation>(CONVERSATIONS)
        .find_one(doc! {"user_id":OWNER,"agent_id":&agent.id,"reply_channel.org_execution":{"$type":"object"}})
        .await.unwrap().unwrap();
    assert_eq!(
        conversation.agent_owner_id.as_deref(),
        Some(org_id.as_str())
    );
    assert!(conversation.channel.is_none() && !conversation.guest_turn);
    assert!(conversation.active_turn.is_none());
    let reply = state
        .db
        .collection::<bson::Document>("assistant_messages")
        .find_one(doc! {"conversation_id":&conversation.id,"role":"assistant"})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(reply.get_str("text").unwrap(), "Here is your answer");
    server.abort();
    state.db.drop().await.unwrap();
}

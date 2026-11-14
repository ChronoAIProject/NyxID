//! Exercises the existing provider protocol with independent org execution identity.
use super::*;
use crate::services::assistant_team_service as team;
use crate::services::{feature_flag_service as flags, org_channel_service as org};

#[path = "nyxbot_org_context_tests.rs"]
mod context_2187;

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

pub(super) async fn org_channel(
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
            voice_instructions: None,
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
            expected_revision: None,
            enabled: true,
            service_ids: Some(vec![]),
            daily_credits: Some(Credits::from_whole(1)),
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
            sink.lock().await.push(json!({"actor":conversation.user_id,"setup":conversation.guest_model_setup,"guest":conversation.guest_turn,"status":status.as_u16(),"body":String::from_utf8_lossy(&bytes),
                "contexts":turn_context_tests::contexts(&body),"input":body["input"],
                "persisted_context":conversation.active_turn.as_ref().unwrap().channel_context}));
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
    assert!(seen[0]["contexts"].as_array().unwrap().is_empty());
    let contexts = seen[1]["contexts"].as_array().unwrap();
    assert_eq!(contexts.len(), 1);
    assert_eq!(contexts[0]["identity"]["platform"], row.platform);
    assert_eq!(
        contexts[0]["identity"]["channel_bot_id"],
        row.channel_bot_id
    );
    assert_eq!(contexts[0]["sender"]["classification"], "org_guest");
    assert_eq!(contexts[0]["conversation"]["kind"], "private");
    assert_eq!(
        seen[1]["persisted_context"]["channel_bot_id"],
        row.channel_bot_id
    );
    let input = seen[1]["input"].as_str().unwrap();
    assert!(input.contains("The linking administrator supplies transport only"));
    assert!(input.contains("Guest model and tool use is charged to the organization"));
    assert!(
        input.contains(
            "Guest execution cannot use account tools, machines, saved logins or approvals"
        )
    );
    assert!(!input.contains("NyxID verified this sender as the owner."));
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

async fn org_guest_group_dm_group(followed: bool, failed_record: bool, route_retry: bool) {
    use crate::models::channel_message::ChannelMessage;
    use crate::models::channel_thread::{
        ChannelThreadFacts, ThreadAddress, ThreadKind, ThreadSenderKind,
    };
    use crate::services::channel_platform::InboundMessage;
    use crate::services::channel_thread_follow_service as follow;
    use futures::TryStreamExt;
    use tower::ServiceExt;

    let (mut state, _, old) = setup(&format!(
        "org_guest_surface_{followed}_{failed_record}_{route_retry}"
    ))
    .await;
    old.abort();
    let (mut row, key, agent, org_id) = org_channel(&state, false).await;
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
        rate.credits_per_unit_micros = 1;
        state
            .db
            .collection("billing_rate_cache")
            .insert_one(rate)
            .await
            .unwrap();
    }
    let provider = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(json!({
            "id":"trace-model-answer","model":"fixture-model","output":[],
            "usage":{"input_tokens":7,"output_tokens":3,"total_tokens":10}
        })))
        .mount(&provider)
        .await;
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
                "billing":{"platform_billable":true,"platform_charge_nyxid_credentials_only":false}
            }},
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
    state
        .db
        .collection::<bson::Document>("user_services")
        .update_one(
            doc! {"_id":&connection},
            doc! {"$set":{
                "catalog_service_id":&catalog.id,"credential_binding":"platform",
                "auth_method":"bearer","source":"manual"
            }},
        )
        .await
        .unwrap();
    org::management::update(
        &state.db,
        OWNER,
        &row,
        org::management::GuestSettings {
            expected_revision: None,
            enabled: true,
            service_ids: Some(vec![]),
            daily_credits: Some(crate::models::credits::Credits::from_whole(1)),
        },
    )
    .await
    .unwrap();

    row = load_channel(&state, OWNER, &row.id).await.unwrap();
    assert!(row.org_specialist.as_ref().unwrap().guests.enabled);
    state
        .db
        .collection::<bson::Document>("api_keys")
        .update_one(
            doc! {"_id": &row.route_api_key_id},
            doc! {"$set": {"callback_url": "https://example.test/callback"}},
        )
        .await
        .unwrap();
    row.platform = "lark".into();
    row.gateway_channel_id = Some("cmaeg1.ch.test".into());
    row.gateway_bot_id = Some("ou_bot".into());
    row.gateway_version = Some(1);
    row.gateway_bot_mentions.supported = true;
    row.gateway_bot_mentions.enabled = true;
    row.gateway_bot_mentions.checked_at = Some(Utc::now());
    row.gateway_threads.supported = true;
    row.gateway_threads.version = Some(1);
    row.gateway_threads.enabled = true;
    row.gateway_threads.checked_at = Some(Utc::now());
    state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .replace_one(doc! {"_id":&row.id}, &row)
        .await
        .unwrap();
    state
        .db
        .collection::<bson::Document>("channel_bots")
        .update_one(
            doc! {"_id":&row.channel_bot_id},
            doc! {"$set":{"platform":"lark","user_id":&org_id,"platform_bot_id":"ou_bot", "bot_token_encrypted": bson::Binary { subtype: bson::spec::BinarySubtype::Generic, bytes: state.encryption_keys.encrypt(b"app:secret").await.unwrap() }}},
        )
        .await
        .unwrap();
    state
        .db
        .collection::<bson::Document>("channel_conversations")
        .update_one(
            doc! {"_id":row.route_id.as_deref().unwrap()},
            doc! {"$set":{"platform":"lark","platform_conversation_type":"group","user_id":&org_id}},
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
    let drifted = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let upstream=Router::new().route("/v1/responses",post(move |headers:HeaderMap,Json(body):Json<Value>| {
        let callbacks=callbacks.clone();let sink=sink.clone();let db=db.clone();let drifted=drifted.clone();
        async move {
            let bearer=headers["authorization"].clone();
            let (_,key,_)=key_service::validate_api_key(&db,bearer.to_str().unwrap().strip_prefix("Bearer ").unwrap()).await.unwrap();
            let conversation=db.collection::<AssistantConversation>(CONVERSATIONS).find_one(doc!{"credential_api_key_id":&key.id}).await.unwrap().unwrap();
            if route_retry && conversation.guest_turn && !drifted.swap(true,std::sync::atomic::Ordering::SeqCst) {
                // Drift after runtime readiness but before the real callback: the
                // callback must record route_mismatch and force a fresh setup.
                db.collection::<bson::Document>(crate::services::assistant_service_execution::COLLECTION)
                    .update_many(doc! {"kind":crate::services::assistant_guest_model_setup::KIND,"user_id":&conversation.user_id},
                        doc! {"$set":{"route.execution":"stale-execution"}}).await.unwrap();
                db.collection::<bson::Document>("assistant_agents").update_one(doc! {"_id":&conversation.agent_id},
                    doc! {"$set":{"owner_model_route.execution":"stale-execution"}}).await.unwrap();
            }
            let response=callbacks.oneshot(axum::http::Request::builder().method("POST").uri("/api/v1/proxy/s/fixture-model/responses")
                .header("authorization",bearer).header("content-type","application/json").header("accept","*/*")
                .body(Body::from(json!({"model":"fixture-model","store":false,"input":body["input"],"stream":false}).to_string())).unwrap()).await.unwrap();
            let status=response.status();let bytes=axum::body::to_bytes(response.into_body(),1024*1024).await.unwrap();
            sink.lock().await.push(json!({"conversation_id":conversation.id,"api_key_id":key.id,"actor":conversation.user_id,"setup":conversation.guest_model_setup,"guest":conversation.guest_turn,"status":status.as_u16(),"body":String::from_utf8_lossy(&bytes),
                "contexts":turn_context_tests::contexts(&body),"input":body["input"],
                "persisted_context":conversation.active_turn.as_ref().unwrap().channel_context}));
            let session=format!("conv_{}",Uuid::new_v4().simple());
            let event=json!({"type":if status.is_success(){"response.completed"}else{"response.failed"},"sequence_number":0,"response":{"id":format!("resp_{}_{}",&session[5..],Uuid::new_v4().simple()),
                "conversation":{"id":session},"status":if status.is_success(){"completed"}else{"failed"},"error":if status.is_success(){Value::Null}else{json!({"code":"model_error","message":"Model callback refused"})},"output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"Organization answer"}]}]}});
            let setup = conversation.guest_model_setup;
            axum::response::Sse::new(futures::stream::once(async move {
                if setup {tokio::time::sleep(std::time::Duration::from_millis(350)).await;}
                Ok::<_,Infallible>(Event::default().data(event.to_string()))
            })).into_response()
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
            Json(binding_body(&key, OWNER)),
        )
        .await
        .status(),
        StatusCode::OK
    );
    ensure_org_partition(&state, &key).await;
    flags::set_platform_override(
        &state.db,
        flags::NYXBOT_THREAD_FOLLOW_FLAG_KEY,
        &flags::FlagTarget::Global,
        true,
        OWNER,
    )
    .await
    .unwrap();

    // The gateway follow path resolves a Lark adapter before it reads the
    // retained source. The test adapter performs no provider I/O here.
    let platform = wiremock::MockServer::start().await;
    let gateway = wiremock::MockServer::start().await;
    let mut service = crate::models::downstream_service::test_helpers::dummy_service();
    service.id = Uuid::new_v4().to_string();
    service.slug = "cmaeg".into();
    service.requires_user_credential = false;
    service.service_category = "internal".into();
    service.base_url = gateway.uri();
    state
        .db
        .collection::<DownstreamService>(SERVICES)
        .insert_one(service)
        .await
        .unwrap();
    wiremock::Mock::given(wiremock::matchers::method("PUT"))
        .respond_with(|request: &wiremock::Request| {
            let body: Value = request.body_json().unwrap();
            wiremock::ResponseTemplate::new(200).set_body_json(json!({
                "version": body["expected_version"].as_i64().unwrap() + 1,
                "definition": body
            }))
        })
        .mount(&gateway)
        .await;
    wiremock::Mock::given(wiremock::matchers::path(
        "/open-apis/auth/v3/tenant_access_token/internal",
    ))
    .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(json!({
        "code":0,"tenant_access_token":"test-token","expire":7200
    })))
    .mount(&platform)
    .await;
    thread_follow::TEST_ADAPTERS.lock().unwrap().insert(
        row.channel_bot_id.clone(),
        Arc::new(
            crate::services::channel_adapters::lark::LarkFamilyAdapter::media_test_adapter(
                &platform.uri(),
                "lark",
            ),
        ),
    );

    let parent_partition = chats::group_partition("oc_chat", None);
    let parent = chats::record_chat(
        &state,
        &row,
        &parent_partition,
        &chats::ChatFacts {
            kind: "group",
            chat_id: "oc_chat".into(),
            thread_id: None,
            owner: true,
            title: Some("Trace group".into()),
        },
        None,
    )
    .await
    .unwrap();
    state
        .db
        .collection::<bson::Document>(THREADS)
        .update_one(
            doc! {"_id":&parent.id},
            doc! {"$set":{"members":"everyone","threads":if followed {"follow"} else {"off"}}},
        )
        .await
        .unwrap();

    let mut event_id = Uuid::new_v4().to_string();
    let facts = ChannelThreadFacts {
        version: 1,
        kind: ThreadKind::Native,
        chat_id: "oc_chat".into(),
        message_id: "om_message".into(),
        root_id: Some("om_root".into()),
        native_thread_id: Some("omt_thread".into()),
        sender_kind: ThreadSenderKind::Human,
        address: ThreadAddress::Mention,
        ..Default::default()
    };
    let inbound = InboundMessage {
        embedded_attachments: Vec::new(),
        embedded_attachments_omitted: 0,
        platform_message_id: facts.message_id.clone(),
        conversation_id: facts.chat_id.clone(),
        conversation_type: "group".into(),
        sender_platform_id: "guest-sender".into(),
        sender_display_name: Some("Visitor".into()),
        content_type: "text".into(),
        text: Some("Hello from a followed thread".into()),
        attachments: vec![],
        reply_to_platform_message_id: None,
        thread_id: facts.native_thread_id.clone(),
        raw_data: json!({}),
    };
    let mut source = crate::services::channel_relay_service::inbound_metadata(
        &row.channel_bot_id,
        row.route_id.as_deref().unwrap(),
        &org_id,
        "lark",
        &inbound,
        &row.route_api_key_id,
        &event_id,
    );
    source.thread_context = Some(facts.clone());
    state
        .db
        .collection::<ChannelMessage>(crate::models::channel_message::COLLECTION_NAME)
        .insert_one(&source)
        .await
        .unwrap();

    let route = crate::services::channel_routing_service::resolve_agent(
        &state.db,
        &row.channel_bot_id,
        "oc_chat",
        Some("guest-sender"),
        &org_id,
    )
    .await
    .unwrap()
    .expect("thread fixture must have an eligible live relay route");
    assert_eq!(route.api_key_id, row.route_api_key_id);
    assert_eq!(route.conversation.id, source.conversation_id);

    let mut body = event("Hello from a followed thread", "guest-sender", &event_id);
    body["conversation"] = json!(PARTITION);
    body["event_context"]["conversation_id"] = json!(PARTITION);
    let activity = &mut body["event_context"]["activity"];
    activity["source"] = json!({
        "type":"nyxid_relay","platform":"lark","route_id":row.route_id
    });
    activity["conversation"] = json!({"id":"oc_chat","kind":"group"});
    activity["actor"] = json!({"id":"guest-sender","display_name":"Visitor","kind":"human"});
    activity["kind"]["mentions_bot"] = json!(true);
    activity["thread"] = serde_json::to_value(&facts).unwrap();
    activity["event_id"] = json!(event_id);

    async fn next_source(
        state: &AppState,
        source: &mut ChannelMessage,
        body: &mut Value,
        text: &str,
    ) -> String {
        let id = Uuid::new_v4().to_string();
        let message = format!("om_{}", Uuid::new_v4().simple());
        source.id = id.clone();
        source.platform_message_id = Some(message.clone());
        source.thread_context.as_mut().unwrap().message_id = message.clone();
        state
            .db
            .collection::<ChannelMessage>(crate::models::channel_message::COLLECTION_NAME)
            .insert_one(&*source)
            .await
            .unwrap();
        body["metadata"]["cmaeg.event"] = json!(id);
        body["event_context"]["activity"]["event_id"] = json!(id);
        body["event_context"]["activity"]["thread"]["message_id"] = json!(message);
        body["input"][0]["content"][0]["text"] = json!(text);
        body["event_context"]["activity"]["kind"]["text"] = json!(text);
        id
    }
    let records = state
        .db
        .collection::<bson::Document>(crate::services::assistant_service_execution::COLLECTION);
    if failed_record {
        records.insert_one(doc! {"_id":"old-failed-setup","kind":crate::services::assistant_guest_model_setup::KIND,
            "user_id":&org_id,"agent_id":&agent.id,"profile":&agent.model,"status":"failed",
            "reason":"model_connection_unavailable","claim":"old-claim","model_failures":0_i64,
            "until":bson::DateTime::from_chrono(Utc::now()+ChronoDuration::seconds(60)),
            "expires_at":bson::DateTime::from_chrono(Utc::now()+ChronoDuration::days(7))}).await.unwrap();
        let refused = body_text(Box::pin(respond(&state, &key, &body, &event_id)).await).await;
        assert!(!refused.contains("Organization answer"));
        assert!(observations.lock().await.is_empty());
        let unchanged = records
            .find_one(doc! {"_id":"old-failed-setup"})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(unchanged.get_str("claim"), Ok("old-claim"));
        assert_eq!(unchanged.get_str("status"), Ok("failed"));
        assert_eq!(
            state
                .db
                .collection::<bson::Document>(CONVERSATIONS)
                .count_documents(doc! {"user_id":&org_id,"guest_model_setup":true})
                .await
                .unwrap(),
            0,
            "backoff must not start hidden setup"
        );
        // Simulate elapsed time only; the next guest request owns the automatic retry.
        records.update_one(doc! {"_id":"old-failed-setup"},doc! {"$set":{"until":bson::DateTime::from_chrono(Utc::now()-ChronoDuration::seconds(1))}}).await.unwrap();
        event_id = next_source(&state, &mut source, &mut body, "First guest after backoff").await;
    }
    if followed {
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/open-apis/im/v1/messages/om_root/reply"))
            .and(wiremock::matchers::body_partial_json(json!({"reply_in_thread":true})))
            .respond_with(|_: &wiremock::Request| wiremock::ResponseTemplate::new(200).set_body_json(json!({
                "code":0,"data":{"message_id":format!("om_guest_answer_{}",Uuid::new_v4().simple())}})))
            .expect(3).mount(&platform).await;
    }
    let mut group_conversation = None;
    let mut private_conversation = None;
    for phase in 0..3 {
        if phase == 1 {
            let partition = "conv_00000000000000000000000000002454";
            assert_eq!(
                put_conversation(
                    State(state.clone()),
                    Path(("bnd_turns".into(), partition.into())),
                    bearer(&key)
                )
                .await
                .status(),
                StatusCode::OK
            );
            let id = Uuid::new_v4().to_string();
            let mut private = event("Guest DM between group mentions", "guest-sender", &id);
            private["conversation"] = json!(partition);
            private["event_context"]["conversation_id"] = json!(partition);
            private["event_context"]["activity"]["source"] =
                body["event_context"]["activity"]["source"].clone();
            private["event_context"]["activity"]["conversation"] =
                json!({"id":"oc_private","kind":"private"});
            let answer = body_text(Box::pin(respond(&state, &key, &private, &id)).await).await;
            assert!(answer.contains("Organization answer"), "{answer}");
            private_conversation = state
                .db
                .collection::<NyxbotEvent>(EVENTS)
                .find_one(doc! {"event_id":id})
                .await
                .unwrap()
                .unwrap()
                .conversation_id;
            assert_ne!(private_conversation, group_conversation);
            continue;
        }
        if phase == 2 {
            event_id = next_source(&state, &mut source, &mut body, "Group mention after DM").await;
        }
        let response = body_text(Box::pin(respond(&state, &key, &body, &event_id)).await).await;
        assert!(
            response.contains("Organization answer"),
            "{response}; calls={:?}",
            observations.lock().await
        );
        assert_eq!(
            response.contains("\"thread_reply\":true"),
            followed,
            "{response}"
        );
        let event = state
            .db
            .collection::<NyxbotEvent>(EVENTS)
            .find_one(doc! {"event_id":&event_id})
            .await
            .unwrap()
            .unwrap();
        assert!(event.turn_id.is_some());
        let guest = state
            .db
            .collection::<AssistantConversation>(CONVERSATIONS)
            .find_one(doc! {"_id":&event.conversation_id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(guest.user_id, org_id);
        assert!(guest.guest_turn);
        assert_eq!(guest.channel.as_ref().unwrap().thread.is_some(), followed);
        group_conversation = event.conversation_id.clone();
        assert_ne!(group_conversation, private_conversation);
        if followed {
            assert_eq!(event.conversation_id, event.resolved_conversation_id);
            let binding = guest.channel.as_ref().unwrap().thread.as_ref().unwrap();
            let child = state
                .db
                .collection::<NyxbotThread>(THREADS)
                .find_one(doc! {"_id":&binding.child_id,"record_scope":follow::SCOPE})
                .await
                .unwrap()
                .unwrap();
            assert_ne!(child.follow.context_status.as_deref(), Some("pending"));
            let (headers, jti) =
                gateway_thread_tests::thread_reply_headers(&state, &row, &event_id);
            if phase == 2 {
                Box::pin(deliver_to_for(
                    &state,
                    &guest,
                    guest.channel.as_ref().unwrap(),
                    "Guest supplemental result",
                    None,
                    supplemental::Purpose::TeamResult,
                ))
                .await;
                assert_eq!(
                    state
                        .db
                        .collection::<bson::Document>(crate::models::audit_log::COLLECTION_NAME)
                        .count_documents(doc! {
                            "event_type":"nyxbot_channel_supplemental_delivery",
                            "event_data.conversation_id":&guest.id,
                            "event_data.channel_event_id":&event.id,
                            "event_data.outcome":"confirmed",
                            "event_data.egress":"local_relay",
                        })
                        .await
                        .unwrap(),
                    1
                );
                assert_eq!(
                    state
                        .db
                        .collection::<bson::Document>("reply_token_uses")
                        .count_documents(doc! {"_id":&jti})
                        .await
                        .unwrap(),
                    0
                );
            }
            let posted = Box::pin(gateway_thread_tests::post_thread_answer(
                &state, &row, &event_id, &headers,
            ))
            .await;
            assert_eq!(
                posted.status(),
                StatusCode::OK,
                "{}",
                body_text(posted).await
            );
            let requests = platform.received_requests().await.unwrap();
            let sends: Vec<_> = requests
                .iter()
                .filter(|r| r.url.path() == "/open-apis/im/v1/messages/om_root/reply")
                .collect();
            assert_eq!(sends.len(), if phase == 0 { 1 } else { 3 });
            assert!(
                sends
                    .iter()
                    .all(|r| r.body_json::<Value>().unwrap()["reply_in_thread"] == true)
            );
            let listed = thread_controls::list(&state, OWNER, &row.id, None, &Default::default())
                .await
                .unwrap();
            assert!(
                listed["threads"][0]["conversation_id"].is_null(),
                "guest execution must not become the admin's private lane"
            );
        }
        if phase == 0 {
            assert_eq!(
                observations.lock().await.len(),
                if route_retry { 4 } else { 2 },
                "cold setup and guest, with optional forced setup/retry"
            );
        }
    }
    let seen = observations.lock().await.clone();
    assert_eq!(
        seen.len(),
        if route_retry { 6 } else { 4 },
        "setup and guest attempts: {seen:?}"
    );
    assert_eq!(
        seen.iter().filter(|call| call["setup"] == true).count(),
        if route_retry { 2 } else { 1 }
    );
    assert_eq!(seen[0]["setup"], true);
    assert_eq!(
        seen[0]["input"],
        crate::services::assistant_guest_model_setup::INSTRUCTION
    );
    assert_eq!(seen[0]["contexts"], json!([]));
    assert!(seen[0]["persisted_context"].is_null());
    for call in &seen {
        assert_eq!(call["actor"], org_id);
    }
    let refused: Vec<_> = seen.iter().filter(|call| call["status"] != 200).collect();
    assert_eq!(refused.len(), usize::from(route_retry));
    if route_retry {
        assert!(
            refused[0]["body"]
                .as_str()
                .unwrap()
                .contains("guest_model_route_required")
        );
    }
    for setup in seen.iter().filter(|call| call["setup"] == true) {
        let id = setup["conversation_id"].as_str().unwrap();
        let settled=state.db.collection::<bson::Document>(crate::models::audit_log::COLLECTION_NAME)
            .find_one(doc! {"event_type":"assistant_turn_failed","event_data.conversation_id":id,"event_data.code":"cancelled"}).await.unwrap();
        assert!(
            settled.is_some(),
            "setup must stop through real settlement: {id}"
        );
    }
    let ready=records.find_one(doc! {"kind":crate::services::assistant_guest_model_setup::KIND,"user_id":&org_id,"agent_id":&agent.id}).await.unwrap().unwrap();
    assert_eq!(ready.get_str("status"), Ok("ready"));
    assert_ne!(ready.get_str("claim"), Ok("old-claim"));
    assert!(ready.get_document("route").is_ok());
    assert_eq!(
        state
            .db
            .collection::<bson::Document>(CONVERSATIONS)
            .count_documents(doc! {"user_id":&org_id,"guest_model_setup":true})
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        state
            .db
            .collection::<bson::Document>("assistant_agent_credentials")
            .count_documents(doc! {"conversation_id":seen[0]["conversation_id"].as_str().unwrap()})
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        state
            .db
            .collection::<bson::Document>("api_keys")
            .count_documents(doc! {"_id":seen[0]["api_key_id"].as_str().unwrap(),"is_active":true})
            .await
            .unwrap(),
        0
    );
    let usage: Vec<crate::models::usage_meter::UsageMeterRow> = state
        .db
        .collection("usage_meter")
        .find(doc! {"service_slug":"fixture-model"})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert_eq!(usage.len(), if route_retry { 5 } else { 4 });
    assert!(
        usage
            .iter()
            .all(|u| u.billing_owner_id == org_id && u.org_channel_guest.is_some())
    );
    assert_eq!(
        state
            .db
            .collection::<bson::Document>("billing_wallet")
            .count_documents(doc! {"owner_id":OWNER})
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        state
            .db
            .collection::<bson::Document>("assistant_acknowledgements")
            .count_documents(doc! {"user_id":&org_id})
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        state
            .db
            .collection::<bson::Document>(EVENTS)
            .count_documents(doc! {"channel_id":&row.id,"guest_setup_payload":{"$exists":true}})
            .await
            .unwrap(),
        0,
        "waiting guest sources must be consumed"
    );
    if followed {
        platform.verify().await;
    }
    crate::handlers::channel_relay::TEST_REPLY_ADAPTERS
        .lock()
        .unwrap()
        .remove(&row.channel_bot_id);
    thread_follow::TEST_ADAPTERS
        .lock()
        .unwrap()
        .remove(&row.channel_bot_id);
    server.abort();
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn qa_cold_org_guest_followed_group_then_dm_then_group_delivers_and_bills_org() {
    Box::pin(org_guest_group_dm_group(true, false, false)).await;
}
#[tokio::test]
async fn cold_org_guest_group_without_thread_binding_initializes_and_bills_org() {
    Box::pin(org_guest_group_dm_group(false, false, false)).await;
}
#[tokio::test]
async fn failed_org_guest_setup_respects_backoff_then_recovers_on_next_thread_guest() {
    Box::pin(org_guest_group_dm_group(true, true, false)).await;
}

#[tokio::test]
async fn cold_org_guest_thread_route_drift_forces_setup_and_retries_same_guest() {
    Box::pin(org_guest_group_dm_group(true, false, true)).await;
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
    {
        let observed = calls.lock().await;
        let contexts = turn_context_tests::contexts(&observed[0]);
        assert_eq!(contexts.len(), 1);
        assert_eq!(contexts[0]["identity"]["platform"], "discord");
        assert_eq!(
            contexts[0]["identity"]["channel_bot_id"],
            row.channel_bot_id
        );
        assert_eq!(contexts[0]["sender"]["classification"], "org_member");
        assert_eq!(contexts[0]["sender"]["kind"], "human");
        assert!(
            !observed[0]["input"]
                .as_str()
                .unwrap()
                .contains("NyxID verified this sender as the owner.")
        );
    }
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
    {
        let observed = calls.lock().await;
        let contexts = turn_context_tests::contexts(&observed[0]);
        assert_eq!(contexts.len(), 1);
        assert_eq!(contexts[0]["identity"]["platform"], "lark");
        assert_eq!(
            contexts[0]["identity"]["channel_bot_id"],
            row.channel_bot_id
        );
        assert_eq!(contexts[0]["sender"]["classification"], "org_member");
        assert_eq!(contexts[0]["sender"]["kind"], "human");
        assert!(
            observed[0]["input"]
                .as_str()
                .unwrap()
                .contains("The linking administrator supplies transport only")
        );
    }
    server.abort();
    state.db.drop().await.unwrap();
}

#[path = "nyxbot_org_access_tests.rs"]
mod access_tests;

#[tokio::test]
async fn supplemental_org_specialist_uses_org_route_and_admitted_member() {
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::path_regex};
    Box::pin(async {
        let (state,_,server)=setup("supplemental_org_specialist").await;
        let (channel,key,agent,org_id)=org_channel(&state,false).await;
        put_binding(State(state.clone()),Path("bnd_turns".into()),bearer(&key),Json(binding_body(&key,OWNER))).await;
        ensure_org_partition(&state,&key).await;
        let challenge=org::links::issue(&state.db,OWNER,&channel).await.unwrap();
        org::links::receive(&state.db,&channel,"7",challenge["code"].as_str().unwrap(),None).await.unwrap();
        org::links::confirm(&state.db,OWNER,&channel,challenge["id"].as_str().unwrap()).await.unwrap();
        state.db.collection::<bson::Document>("channel_messages").insert_one(doc!{"_id":"org-source","conversation_id":&channel.route_id,"user_id":&org_id,"channel_bot_id":&channel.channel_bot_id,"direction":"inbound","platform":"telegram","platform_conversation_id":"chat-1","platform_message_id":"42","sender_platform_id":"7","agent_api_key_id":&channel.route_api_key_id,"content_type":"text","created_at":bson::DateTime::now()}).await.unwrap();
        assert!(body_text(Box::pin(respond(&state,&key,&event("request","7","org-source"),"org-source")).await).await.contains("Here is your answer"));
        let event=state.db.collection::<NyxbotEvent>(EVENTS).find_one(doc!{"channel_id":&channel.id,"event_id":"org-source"}).await.unwrap().unwrap();
        let mut conversation=engine::get(&state.db,OWNER,event.conversation_id.as_deref().unwrap()).await.unwrap();
        let origin=event.delivery.as_ref().unwrap().origin.clone();
        assert_eq!(origin.org_execution.as_ref().unwrap().actor_id,OWNER);
        assert_eq!(conversation.agent_id.as_deref(),Some(agent.id.as_str()));
        conversation.active_turn=Some(bson::from_document(doc!{"turn_id":&event.turn_id,"origin":"channel","asked_from":bson::to_bson(&origin).unwrap(),"channel_event_id":&event.id,"started_at":bson::DateTime::now(),"stop_requested":false}).unwrap());
        let cipher=state.encryption_keys.encrypt(b"test-token").await.unwrap();
        state.db.collection::<bson::Document>("channel_bots").update_one(doc!{"_id":&channel.channel_bot_id},doc!{"$set":{"bot_token_encrypted":bson::Binary{subtype:bson::spec::BinarySubtype::Generic,bytes:cipher}}}).await.unwrap();
        let platform=MockServer::start().await;
        Mock::given(path_regex("/bot.*/sendMessage")).respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok":true,"result":{"message_id":43}}))).mount(&platform).await;
        crate::handlers::channel_relay::TEST_REPLY_ADAPTERS.lock().unwrap().insert(channel.channel_bot_id.clone(),Arc::new(crate::services::channel_adapters::telegram::TelegramAdapter::media_test_adapter(&platform.uri())));
        deliver_to_for(&state,&conversation,&origin,"notice",None,supplemental::Purpose::MachineDesktopNotice).await;
        assert_eq!(platform.received_requests().await.unwrap().len(),1);
        state.db.collection::<bson::Document>("org_memberships").update_one(doc!{"org_user_id":&org_id,"member_user_id":OWNER},doc!{"$set":{"revoked_at":bson::DateTime::now()}}).await.unwrap();
        deliver_to_for(&state,&conversation,&origin,"revoked",None,supplemental::Purpose::MachineDesktopNotice).await;
        assert_eq!(platform.received_requests().await.unwrap().len(),1);
        server.abort();state.db.drop().await.unwrap();
    }).await;
}

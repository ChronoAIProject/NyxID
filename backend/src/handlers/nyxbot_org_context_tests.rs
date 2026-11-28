//! Real org channel ingress, source queue transactions and model-bound context.
use super::*;
use crate::models::assistant_agent::AssistantAgent;
use crate::models::org_membership::OrgRole;

const GROUP_PARTITION: &str = "conv_11111111111111111111111111111111";

pub(super) async fn member(state: &AppState, row: &NyxbotChannel, org_id: &str) -> String {
    let id = Uuid::new_v4().to_string();
    state
        .db
        .collection(USERS)
        .insert_one(test_user(&id, UserType::Person))
        .await
        .unwrap();
    state
        .db
        .collection("org_memberships")
        .insert_one(crate::test_utils::test_membership(
            org_id,
            &id,
            OrgRole::Member,
            None,
        ))
        .await
        .unwrap();
    let issued = org::links::issue(&state.db, &id, row).await.unwrap();
    assert!(
        org::links::receive(
            &state.db,
            row,
            "7",
            issued["code"].as_str().unwrap(),
            Some("Verified member")
        )
        .await
        .unwrap()
    );
    org::links::confirm(&state.db, &id, row, issued["id"].as_str().unwrap())
        .await
        .unwrap();
    id
}

async fn extra_channel(
    state: &AppState,
    org_id: &str,
    agent: &AssistantAgent,
) -> (NyxbotChannel, String) {
    let (mut row, key) = channel(state, "gateway").await;
    state
        .db
        .collection::<bson::Document>("channel_bots")
        .update_one(
            doc! {"_id":&row.channel_bot_id},
            doc! {"$set":{"user_id":org_id}},
        )
        .await
        .unwrap();
    let bot = channel_bot_service::get_bot(&state.db, &row.channel_bot_id)
        .await
        .unwrap();
    row.org_specialist = org::link_target(&state.db, OWNER, &bot, agent)
        .await
        .unwrap();
    row.bot_owner_id = Some(org_id.into());
    row.agent_id = Some(agent.id.clone());
    state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .replace_one(doc! {"_id":&row.id}, &row)
        .await
        .unwrap();
    for (collection, id) in [
        ("api_keys", row.route_api_key_id.as_str()),
        ("channel_conversations", row.route_id.as_deref().unwrap()),
    ] {
        state
            .db
            .collection::<bson::Document>(collection)
            .update_one(doc! {"_id":id}, doc! {"$set":{"user_id":org_id}})
            .await
            .unwrap();
    }
    (row, key)
}

async fn bind(state: &AppState, key: &str) {
    assert_eq!(
        put_binding(
            State(state.clone()),
            Path("bnd_turns".into()),
            bearer(key),
            Json(binding_body(key, OWNER))
        )
        .await
        .status(),
        StatusCode::OK
    );
    ensure_org_partition(state, key).await;
}

fn request(id: &str, text: &str) -> engine::TurnRequest {
    engine::TurnRequest {
        acknowledgement_id: None,
        retry_of: None,
        attachment_ids: vec![],
        conversation_id: Some(id.into()),
        agent_id: None,
        text: text.into(),
        model: None,
        access_mode: None,
    }
}

async fn finish(state: &AppState, row: &AssistantConversation) {
    engine::finish_turn(
        &state.db,
        row,
        &row.credential_api_key_id,
        &Uuid::new_v4().to_string(),
        &engine::TurnResult {
            text: "Blocking work complete".into(),
            session_id: None,
            response_id: None,
            error: None,
        },
    )
    .await
    .unwrap();
}

async fn drain(state: &AppState, actor: &str, id: &str) {
    let mut start = TurnStart::from(&request(id, ""));
    start.origin = TurnOrigin::Event;
    let Started::Turn { receiver, .. } = start_server_turn(
        state,
        actor,
        start,
        Pool::Team {
            owner: actor,
            limit: 1,
        },
    )
    .await
    .unwrap() else {
        panic!("queued org turn must start")
    };
    final_reply(receiver).await.unwrap();
    settle(state, id).await;
}

fn assert_member(call: &Value, bot: &str, kind: &str, count: usize) -> Vec<Value> {
    let contexts = turn_context_tests::contexts(call);
    assert_eq!(contexts.len(), count);
    for context in &contexts {
        assert_eq!(context["identity"]["platform"], "telegram");
        assert_eq!(context["identity"]["channel_bot_id"], bot);
        assert_eq!(context["conversation"]["kind"], kind);
        assert_eq!(context["sender"]["classification"], "org_member");
        assert_eq!(context["sender"]["kind"], "human");
    }
    let input = call["input"].as_str().unwrap();
    assert!(input.contains("The linking administrator supplies transport only"));
    assert!(!input.contains("NyxID verified this sender as the owner."));
    contexts
}

#[tokio::test]
async fn identity_2187_org_member_queue_keeps_each_binding_and_rejects_laundering_or_revocation() {
    let (state, calls, server) = setup("identity_2187_org_member_queue").await;
    let _server = tokio_util::task::AbortOnDropHandle::new(server);
    let (a, a_key, agent, org_id) = org_channel(&state, false).await;
    let actor = member(&state, &a, &org_id).await;
    bind(&state, &a_key).await;
    let (b, b_key) = extra_channel(&state, &org_id, &agent).await;
    let issued = org::links::issue(&state.db, &actor, &b).await.unwrap();
    assert!(
        org::links::receive(&state.db, &b, "7", issued["code"].as_str().unwrap(), None)
            .await
            .unwrap()
    );
    org::links::confirm(&state.db, &actor, &b, issued["id"].as_str().unwrap())
        .await
        .unwrap();
    bind(&state, &b_key).await;
    let answer = body_text(
        respond(
            &state,
            &a_key,
            &event("Initial member question", "7", "org-context-seed"),
            "org-context-seed",
        )
        .await,
    )
    .await;
    assert!(answer.contains("Here is your answer"), "{answer}");
    let home = state
        .db
        .collection::<AssistantConversation>(CONVERSATIONS)
        .find_one(doc! {"user_id":&actor,"agent_id":&agent.id,"channel":bson::Bson::Null})
        .await
        .unwrap()
        .unwrap();
    settle(&state, &home.id).await;
    assert_member(&calls.lock().await[0], &a.channel_bot_id, "private", 1);
    assert_ne!(
        actor, a.user_id,
        "member execution must differ from transport ownership"
    );
    let blocked = engine::begin_turn(
        &state.db,
        &actor,
        &request(&home.id, "Ongoing app work"),
        &state.encryption_keys,
    )
    .await
    .unwrap();
    for (key, id) in [
        (&a_key, "org-context-a1"),
        (&a_key, "org-context-a2"),
        (&b_key, "org-context-b"),
    ] {
        let response = respond(
            &state,
            key,
            &event("Same queued member question", "7", id),
            id,
        )
        .await;
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "{}",
            body_text(response).await
        );
    }
    assert_eq!(calls.lock().await.len(), 1);
    let queued = engine::get(&state.db, &actor, &home.id).await.unwrap();
    assert_eq!(
        queued.pending_events.len(),
        2,
        "different org bindings cannot coalesce"
    );
    let first = &queued.pending_events[0];
    assert_eq!(first.source_event_ids.len(), 2);
    assert_eq!(first.channel_contexts.len(), 2);
    assert!(first.text.starts_with("Queued verified channel message"));
    assert_eq!(queued.pending_events[1].source_event_ids.len(), 1);
    assert_eq!(
        queued.reply_channel.as_ref().unwrap().nyxbot_channel_id,
        b.id
    );
    for source in &first.channel_contexts {
        assert!(first.source_event_ids.contains(&source.source_event_id));
        assert_eq!(source.channel_bot_id, a.channel_bot_id);
        assert_eq!(
            source
                .org_origin
                .as_ref()
                .unwrap()
                .org_execution
                .as_ref()
                .unwrap()
                .actor_id,
            actor
        );
    }
    // Render the exact queued carrier while its latest reply preference points
    // elsewhere. This does not authorize either source from that preference.
    let mut prepared = queued.clone();
    let turn = prepared.active_turn.as_mut().unwrap();
    turn.channel_context = None;
    turn.channel_event_id = None;
    turn.events = vec![first.clone()];
    let notes = crate::services::channel_turn_context::turn_notes(&state.db, &prepared)
        .await
        .unwrap();
    assert_member(&json!({"input":notes}), &a.channel_bot_id, "private", 2);
    let other_org = Uuid::new_v4().to_string();
    state
        .db
        .collection(USERS)
        .insert_one(test_user(&other_org, UserType::Org))
        .await
        .unwrap();
    state
        .db
        .collection("org_memberships")
        .insert_one(crate::test_utils::test_membership(
            &other_org,
            &actor,
            OrgRole::Member,
            None,
        ))
        .await
        .unwrap();
    let mut wrong_org = prepared.clone();
    wrong_org.agent_owner_id = Some(other_org);
    assert!(matches!(
        crate::services::channel_turn_context::turn_notes(&state.db, &wrong_org).await,
        Err(AppError::Forbidden(_))
    ));
    let mut downgraded = prepared.clone();
    downgraded.active_turn.as_mut().unwrap().events[0].channel_contexts[0].org_origin = None;
    assert!(matches!(
        crate::services::channel_turn_context::turn_notes(&state.db, &downgraded).await,
        Err(AppError::Forbidden(_))
    ));
    state
        .db
        .collection::<bson::Document>(CHANNELS)
        .update_one(
            doc! {"_id":&a.id},
            doc! {"$set":{"org_specialist.revision":"revoked-binding"}},
        )
        .await
        .unwrap();
    assert!(matches!(
        crate::services::channel_turn_context::turn_notes(&state.db, &prepared).await,
        Err(AppError::Forbidden(_))
    ));
    state
        .db
        .collection::<bson::Document>(CHANNELS)
        .update_one(
            doc! {"_id":&a.id},
            doc! {"$set":{"org_specialist":bson::to_bson(&a.org_specialist).unwrap()}},
        )
        .await
        .unwrap();
    state
        .db
        .collection::<bson::Document>("channel_bots")
        .update_one(
            doc! {"_id":&a.channel_bot_id},
            doc! {"$set":{"platform_bot_username":"current-org-bot","label":"Same editable label"}},
        )
        .await
        .unwrap();
    // Revoke/prune B before settlement can wake it. Having separately proved
    // rendering under B's preference above, explicitly select A for execution.
    state
        .db
        .collection::<bson::Document>(CHANNELS)
        .update_one(
            doc! {"_id":&b.id},
            doc! {"$set":{"org_specialist.revision":"revoked-binding"}},
        )
        .await
        .unwrap();
    assert_eq!(
        crate::services::channel_thread_follow_service::prune_queue(&state.db, &actor, &home.id)
            .await
            .unwrap(),
        1
    );
    engine::set_reply_channel(&state.db, &actor, &home.id, &first.reply_to[0])
        .await
        .unwrap();
    finish(&state, &blocked).await;
    drain(&state, &actor, &home.id).await;
    {
        let observed = calls.lock().await;
        assert_eq!(observed.len(), 2);
        let contexts = assert_member(&observed[1], &a.channel_bot_id, "private", 2);
        for (context, source) in contexts.iter().zip(&first.channel_contexts) {
            assert_eq!(context["source_event_id"], source.source_event_id);
            assert_eq!(context["event_id"], first.id);
            assert_eq!(context["identity"]["bot_username"], "current-org-bot");
        }
        assert!(
            !observed[1]["input"]
                .as_str()
                .unwrap()
                .contains(&b.channel_bot_id)
        );
    }
    let raw = state
        .db
        .collection::<AssistantConversation>(CONVERSATIONS)
        .find_one(doc! {"_id":&home.id})
        .await
        .unwrap()
        .unwrap();
    assert!(raw.pending_events.is_empty());
    assert_eq!(
        calls.lock().await.len(),
        2,
        "revoked queued binding cannot execute"
    );
}

fn group_event(text: &str, id: &str) -> Value {
    let mut body = event(text, "7", id);
    body["conversation"] = json!(GROUP_PARTITION);
    body["event_context"]["conversation_id"] = json!(GROUP_PARTITION);
    body["event_context"]["activity"]["conversation"] =
        json!({"id":"org-shared-chat","kind":"group"});
    body["event_context"]["activity"]["kind"]["mentions_bot"] = json!(true);
    body
}

#[tokio::test]
async fn org_lark_provider_source_alias_resolves_under_the_route_read_authority() {
    Box::pin(org_lark_source_result(false)).await;
}

#[tokio::test]
async fn org_lark_detached_reply_lookup_distinguishes_admission_from_gateway_receipt() {
    Box::pin(org_lark_source_result(true)).await;
}

async fn org_lark_source_result(detach_stream: bool) {
    Box::pin(async {
        use crate::{
            crypto::jwt,
            models::channel_message::{COLLECTION_NAME as MESSAGES, ChannelMessage},
            services::{channel_platform::InboundMessage, channel_relay_service},
        };
        use http_body_util::BodyExt;
        use tower::ServiceExt;
        use wiremock::{
            Mock, MockServer, ResponseTemplate,
            matchers::{method, path},
        };

        let (state, calls, server) = setup("org_lark_source_result").await;
        let _server = tokio_util::task::AbortOnDropHandle::new(server);
        let (mut channel, key, _, org_id) = org_channel(&state, false).await;
        assert_eq!(channel.user_id, OWNER);
        assert_eq!(channel.bot_owner_id.as_deref(), Some(org_id.as_str()));
        member(&state, &channel, &org_id).await;
        let viewer = Uuid::new_v4();
        state
            .db
            .collection(USERS)
            .insert_one(test_user(&viewer.to_string(), UserType::Person))
            .await
            .unwrap();
        state
            .db
            .collection("org_memberships")
            .insert_one(crate::test_utils::test_membership(
                &org_id,
                &viewer.to_string(),
                OrgRole::Viewer,
                None,
            ))
            .await
            .unwrap();
        channel.platform = "lark".into();
        channel.gateway_bot_id = Some("ou_fixture_bot".into());
        channel.gateway_channel_id = Some("cmaeg1.ch.test".into());
        let gateway = MockServer::start().await;
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
        Mock::given(method("GET"))
            .and(path("/channel-capabilities"))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                json!({"relay_authoring":{"response_recovery_providers":["nyxbot"]}}),
            ))
            .mount(&gateway)
            .await;
        let reply_path = "/reply-targets/cmaeg1.rt.fixture/messages";
        if detach_stream {
            Mock::given(method("POST"))
                .and(path("/events/cmaeg1.ev.sealed-reference/reply-target"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                    "reply_target_ref": "cmaeg1.rt.fixture",
                    "expires_at": (Utc::now() + ChronoDuration::hours(1)).timestamp(),
                })))
                .mount(&gateway)
                .await;
            Mock::given(method("POST"))
                .and(path(reply_path))
                .respond_with(
                    ResponseTemplate::new(409)
                        .set_body_json(json!({"detail":"routed_reply_in_progress"})),
                )
                .mount(&gateway)
                .await;
        }
        let route = channel.route_id.as_deref().unwrap();
        state
            .db
            .collection::<NyxbotChannel>(CHANNELS)
            .replace_one(doc! {"_id": &channel.id}, &channel)
            .await
            .unwrap();
        for (collection, id) in [
            ("channel_bots", channel.channel_bot_id.as_str()),
            ("channel_conversations", route),
        ] {
            state
                .db
                .collection::<bson::Document>(collection)
                .update_one(doc! {"_id": id}, doc! {"$set": {"platform": "lark"}})
                .await
                .unwrap();
        }
        bind(&state, &key).await;
        assert_eq!(
            put_conversation(
                State(state.clone()),
                Path(("bnd_turns".into(), GROUP_PARTITION.into())),
                bearer(&key),
            )
            .await
            .status(),
            StatusCode::OK,
        );
        let event_id = Uuid::new_v4().to_string();
        let question = "A synthetic organization question";
        let inbound = InboundMessage {
            platform_message_id: "om_source_fixture".into(),
            conversation_id: "org-shared-chat".into(),
            conversation_type: "group".into(),
            sender_platform_id: "7".into(),
            sender_display_name: None,
            content_type: "text".into(),
            text: Some(question.into()),
            attachments: vec![],
            embedded_attachments: vec![],
            embedded_attachments_omitted: 0,
            reply_to_platform_message_id: None,
            thread_id: None,
            raw_data: json!({}),
        };
        let source = channel_relay_service::inbound_metadata(
            &channel.channel_bot_id,
            route,
            &org_id,
            "lark",
            &inbound,
            &channel.route_api_key_id,
            &event_id,
        );
        state
            .db
            .collection::<ChannelMessage>(MESSAGES)
            .insert_one(&source)
            .await
            .unwrap();
        let mut request = group_event(question, &event_id);
        request["event_context"]["activity"]["source"] =
            json!({"type": "nyxid_relay", "platform": "lark", "route_id": route});
        request["metadata"] = json!({
            "cmaeg.channel": &sha256_hex("cmaeg1.ch.test")[..16], "cmaeg.event": event_id,
            "cmaeg.partition":"0123456789abcdef", "cmaeg.actor":"7",
        });
        let response = respond(&state, &key, &request, &event_id).await;
        assert_eq!(response.status(), StatusCode::OK);
        let response_id = response.headers()["x-cma-response-id"]
            .to_str()
            .unwrap()
            .to_owned();
        let mut stream = response.into_body();
        let created = stream.frame().await.unwrap().unwrap().into_data().unwrap();
        let created = String::from_utf8(created.to_vec()).unwrap();
        let created: Value = serde_json::from_str(
            created
                .lines()
                .find_map(|line| line.strip_prefix("data: "))
                .unwrap(),
        )
        .unwrap();
        assert_eq!(created["type"], "response.created");
        assert_eq!(created["response"]["id"], response_id);
        assert_eq!(created["response"]["status"], "queued");
        if detach_stream {
            // Drop after acceptance, as the replica gateway does. Drive the
            // normal delivery worker until it has recorded the routed 409.
            drop(stream);
            tokio::time::timeout(Duration::from_secs(10), async {
                let events = state.db.collection::<bson::Document>(EVENTS);
                loop {
                    let event = events
                        .find_one(doc! {"relay_message_id": &source.id})
                        .await
                        .unwrap()
                        .expect("admitted relay event");
                    if event
                        .get_document("delivery")
                        .ok()
                        .and_then(|delivery| delivery.get_document("gateway_observation").ok())
                        .is_some_and(|receipt| receipt.get_i32("status") == Ok(409))
                    {
                        break;
                    }
                    late_delivery::process(&state, event.get_str("_id").unwrap()).await;
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
            })
            .await
            .expect("local writer must record gateway admission");
        } else {
            let rest = axum::body::to_bytes(stream, 64 * 1024).await.unwrap();
            let rest = std::str::from_utf8(&rest).unwrap();
            assert!(rest.contains("Here is your answer"), "{rest}");
            let completed = rest
                .lines()
                .filter_map(|line| line.strip_prefix("data: "))
                .map(|data| serde_json::from_str::<Value>(data).unwrap())
                .find(|frame| frame["type"] == "response.completed")
                .unwrap();
            assert_eq!(completed["response"]["id"], response_id);
        }
        let [call] = calls
            .lock()
            .await
            .clone()
            .try_into()
            .expect("single provider run");
        let contexts = turn_context_tests::contexts(&call);
        assert_eq!(contexts.len(), 1);
        assert_eq!(contexts[0]["sender"]["classification"], "org_member");
        assert_eq!(contexts[0]["identity"]["platform"], "lark");
        let alias = contexts[0]["source_event_id"].as_str().unwrap();
        assert_eq!(alias.len(), 64);
        let token = jwt::generate_access_token(
            &state.jwt_keys,
            &state.config,
            &viewer,
            "openid account:read",
            None,
            None,
            None,
            None,
            None,
        )
        .unwrap();
        let (_, private) = crate::routes::build_router_with_state(state.clone());
        let reader = private.with_state(state.clone());
        let read = || async {
            let result = reader
                .clone()
                .oneshot(
                    axum::http::Request::get(format!(
                        "/api/v1/channel-conversations/{route}/messages?source_event_id={alias}"
                    ))
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
                )
                .await
                .unwrap();
            let status = result.status();
            let output = body_text(result).await;
            assert_eq!(status, StatusCode::OK, "{output}");
            serde_json::from_str::<Value>(&output).unwrap()
        };
        let mapping = read().await;
        assert_eq!(mapping["source_event_id"], alias);
        assert_eq!(mapping["inbound_message_id"], source.id);
        assert_eq!(mapping["platform_message_id"], inbound.platform_message_id);
        assert_eq!(mapping["sender_platform_id"], inbound.sender_platform_id);
        assert_eq!(mapping["turn_status"], "completed");
        assert_eq!(mapping["response_id"], response_id);
        assert_eq!(
            mapping["reply_status"],
            if detach_stream {
                "pending"
            } else {
                "handed_to_gateway"
            }
        );
        assert!(mapping["reply_evidence"].is_null());
        assert!(mapping["reply_kind"].is_null());
        if detach_stream {
            let before = gateway
                .received_requests()
                .await
                .unwrap()
                .into_iter()
                .filter(|request| request.url.path() == reply_path)
                .collect::<Vec<_>>();
            assert_eq!(before.len(), 1);
            assert_eq!(before[0].headers["idempotency-key"], alias);
            assert_eq!(
                before[0].body_json::<Value>().unwrap()["text"],
                "Here is your answer"
            );
            gateway.reset().await;
            Mock::given(method("POST"))
                .and(path(reply_path))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_json(json!({"delivery":{"outcome":"confirmed"}})),
                )
                .mount(&gateway)
                .await;
            // Advance only the polling schedule. The receipt and terminal
            // delivery state must come from the real worker.
            state
                .db
                .collection::<bson::Document>(EVENTS)
                .update_one(
                    doc! {"_id": alias},
                    doc! {"$set":{
                        "delivery.receipt_due_at": bson::DateTime::from_millis(0),
                    }},
                )
                .await
                .unwrap();
            late_delivery::sweep(&state).await.unwrap();
            let after = gateway.received_requests().await.unwrap();
            assert_eq!(after.len(), 1);
            assert_eq!(after[0].url.path(), reply_path);
            assert_eq!(after[0].headers["idempotency-key"], alias);
            assert_eq!(before[0].body, after[0].body);
            let observed = read().await;
            assert_eq!(observed["response_id"], response_id);
            assert_eq!(observed["turn_status"], "completed");
            assert_eq!(observed["reply_status"], "confirmed");
            assert_eq!(observed["reply_evidence"], "gateway_reply_receipt");
            assert_eq!(observed["reply_kind"], "completed_turn_output");
        }
        assert_eq!(
            state
                .db
                .collection::<ChannelMessage>(MESSAGES)
                .count_documents(doc! {
                    "reply_to_message_id": &source.id, "direction":"outbound",
                })
                .await
                .unwrap(),
            0,
        );
        crate::test_utils::drop_test_database(&state.db).await;
    })
    .await;
}

#[tokio::test]
async fn identity_2187_org_group_transcript_is_current_and_rendered_once_for_coalesced_sources() {
    let (state, calls, server) = setup("identity_2187_org_group_queue").await;
    let _server = tokio_util::task::AbortOnDropHandle::new(server);
    let (channel, key, agent, org_id) = org_channel(&state, false).await;
    let actor = member(&state, &channel, &org_id).await;
    bind(&state, &key).await;
    assert_eq!(
        put_conversation(
            State(state.clone()),
            Path(("bnd_turns".into(), GROUP_PARTITION.into())),
            bearer(&key)
        )
        .await
        .status(),
        StatusCode::OK
    );
    let answer = body_text(
        respond(
            &state,
            &key,
            &group_event("Public initial request", "org-group-seed"),
            "org-group-seed",
        )
        .await,
    )
    .await;
    assert!(answer.contains("Here is your answer"), "{answer}");
    let expected_partition = chats::group_partition("org-shared-chat", None);
    let row = state
        .db
        .collection::<AssistantConversation>(CONVERSATIONS)
        .find_one(doc! {
            "user_id": &actor,
            "agent_id": &agent.id,
            "channel.nyxbot_channel_id": &channel.id,
            "channel.partition": &expected_partition
        })
        .await
        .unwrap()
        .unwrap();
    settle(&state, &row.id).await;
    {
        let observed = calls.lock().await;
        assert_member(&observed[0], &channel.channel_bot_id, "group", 1);
        let input = observed[0]["input"].as_str().unwrap();
        assert!(input.contains("Public initial request"));
        assert_eq!(
            input
                .matches("Everyone in this chat sees only the public transcript below.")
                .count(),
            1
        );
    }
    let origin = row.channel.as_ref().unwrap();
    let mut start = TurnStart::from(&request(&row.id, "Ongoing group work"));
    start.origin = TurnOrigin::Channel;
    start.channel = Some(origin.clone());
    start.org_access = Some(
        org::validate(&state.db, origin, &actor, &agent.id)
            .await
            .unwrap(),
    );
    let blocked = engine::begin_turn(&state.db, &actor, start, &state.encryption_keys)
        .await
        .unwrap();
    for id in ["org-group-q1", "org-group-q2"] {
        let response = respond(&state, &key, &group_event("Queued public request", id), id).await;
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "{}",
            body_text(response).await
        );
    }
    let queued = engine::get(&state.db, &actor, &row.id).await.unwrap();
    assert_eq!(queued.pending_events.len(), 1);
    assert_eq!(queued.pending_events[0].channel_contexts.len(), 2);
    finish(&state, &blocked).await;
    drain(&state, &actor, &row.id).await;
    let observed = calls.lock().await;
    assert_eq!(observed.len(), 2);
    let contexts = assert_member(&observed[1], &channel.channel_bot_id, "group", 2);
    assert_ne!(
        contexts[0]["source_event_id"],
        contexts[1]["source_event_id"]
    );
    for context in &contexts {
        assert_eq!(context["event_id"], queued.pending_events[0].id);
        assert!(
            queued.pending_events[0]
                .source_event_ids
                .contains(&context["source_event_id"].as_str().unwrap().to_owned())
        );
    }
    let input = observed[1]["input"].as_str().unwrap();
    assert!(input.contains("Queued public request"));
    assert!(input.contains("Never publish private memory, tool traces or another actor's cards."));
    assert_eq!(
        input
            .matches("Everyone in this chat sees only the public transcript below.")
            .count(),
        1
    );
    assert!(!input.contains("Queued owner channel message"));
}

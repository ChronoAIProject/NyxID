//! Real org channel ingress, source queue transactions and model-bound context.
use super::*;
use crate::models::assistant_agent::AssistantAgent;
use crate::models::org_membership::OrgRole;

const GROUP_PARTITION: &str = "conv_11111111111111111111111111111111";

async fn member(state: &AppState, row: &NyxbotChannel, org_id: &str) -> String {
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

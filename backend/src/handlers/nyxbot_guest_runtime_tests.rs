use super::*;
use crate::services::{channel_guest_access as guest, feature_flag_admin_service as flags};

pub(super) async fn set_guest_flag(state: &AppState, enabled: Option<bool>) -> AppResult<()> {
    crate::test_utils::init_test_audit_chain_hmac_key();
    let actor = guest_test_actor();
    flags::mutate(
        &state.db,
        guest::FLAG,
        flags::Mutation::Override {
            kind: crate::models::feature_flag_override::FlagTargetKind::Global,
            target_id: None,
            enabled,
        },
        &actor,
    )
    .await?;
    Ok(())
}

fn guest_test_actor() -> crate::services::audit_service::AuditActor {
    crate::services::audit_service::AuditActor {
        user_id: OWNER.into(),
        ip_address: None,
        user_agent: None,
        api_key_id: None,
        api_key_name: None,
    }
}

pub(super) async fn enable_guest_test_bot(state: &AppState, bot_id: &str) {
    crate::test_utils::init_test_audit_chain_hmac_key();
    let updated = state
        .db
        .collection::<bson::Document>("channel_bots")
        .update_one(
            doc! {"_id": bot_id},
            doc! {"$set": {"guest_access_v1": true, "guest_access_v1_revision": 1_i64}},
        )
        .await
        .unwrap();
    assert_eq!(updated.matched_count, 1);
    guest::set_pilot(
        &state.db,
        guest::PilotSelection {
            bot_ids: Some(vec![bot_id.to_owned()]),
            fleet: false,
        },
        &guest_test_actor(),
    )
    .await
    .unwrap();
    set_guest_flag(state, Some(true)).await.unwrap();
}

async fn verified_telegram_guest(
    state: &AppState,
    channel: &NyxbotChannel,
    key: &str,
    text: &str,
    event_id: &str,
) -> Response {
    use crate::{
        models::channel_message::{COLLECTION_NAME as MESSAGES, ChannelMessage},
        services::{channel_platform::InboundMessage, channel_relay_service::inbound_metadata},
    };
    let message = InboundMessage {
        embedded_attachments: Vec::new(),
        embedded_attachments_omitted: 0,
        platform_message_id: event_id.into(),
        conversation_id: "chat-1".into(),
        conversation_type: "private".into(),
        sender_platform_id: "42".into(),
        sender_display_name: None,
        content_type: "text".into(),
        text: Some(text.into()),
        attachments: Vec::new(),
        reply_to_platform_message_id: None,
        thread_id: None,
        raw_data: json!({}),
    };
    let mut source = inbound_metadata(
        &channel.channel_bot_id,
        channel.route_id.as_deref().unwrap(),
        &channel.user_id,
        &channel.platform,
        &message,
        &channel.route_api_key_id,
        event_id,
    );
    // The trusted relay has already verified the sender. This fixture keeps
    // the shared audience so the test can use its existing skill selection.
    source.guest_transport_audience = Some(guest::AnswerAudience::Shared);
    source.original_message = Some(true);
    state
        .db
        .collection::<ChannelMessage>(MESSAGES)
        .insert_one(source)
        .await
        .unwrap();
    Box::pin(respond(state, key, &event(text, "42", event_id), event_id)).await
}

#[tokio::test]
async fn nyxbot_guest_skill_reads_follow_the_live_chat_specialist() {
    use crate::{
        models::{assistant_agent, catalog_skill_revision::SkillReference},
        services::{
            assistant_authority_tests::chat_for_row, assistant_team_service as team,
            channel_guest_access::skills,
        },
    };

    let (state, _, server) = setup("guest_skills_chat_override").await;
    let db = &state.db;
    let (row, key) = channel(&state, "gateway").await;
    crate::services::security_readiness_service::ready_test_replica(db).await;
    chats::set_private_chats(&state, OWNER, &row.id, "everyone")
        .await
        .unwrap();
    let binding = put_binding(
        State(state.clone()),
        Path("bnd_chats".into()),
        bearer(&key),
        Json(binding_body(&key, OWNER)),
    )
    .await;
    assert_eq!(binding.status(), StatusCode::OK);
    ensure_partition(&state, &key, PARTITION).await;

    let default_agent = team::ensure_nyxbot(db, OWNER).await.unwrap();
    let (specialist, _) = team::create_specialist(
        db,
        &state.encryption_keys,
        OWNER,
        team::CreateRequest {
            machines: None,
            logins: None,
            name: "support".into(),
            description: "Help channel visitors".into(),
            display_name: None,
            persona: None,
            voice_instructions: None,
            targets: Default::default(),
            account_read: false,
            specialty: None,
            created_by: "user",
        },
    )
    .await
    .unwrap()
    .unwrap();
    let skill_id = Uuid::new_v4().to_string();
    let agents = db.collection::<bson::Document>(assistant_agent::COLLECTION_NAME);
    let pin = |skill_id: &str, digest: char| SkillReference {
        source: "ornn".into(),
        skill_id: skill_id.into(),
        name: "Channel guide".into(),
        version: "1.0.0".into(),
        sha256: digest.to_string().repeat(64),
        dependencies: vec![],
        disabled: false,
    };
    let default_pin = pin(&skill_id, 'a');
    let override_pin = pin(&skill_id, 'b');
    let private_pin = pin(&Uuid::new_v4().to_string(), 'c');
    for (agent_id, pins) in [
        (&default_agent.id, vec![&default_pin]),
        (&specialist.id, vec![&override_pin, &private_pin]),
    ] {
        agents
            .update_one(
                doc! {"_id": agent_id},
                doc! {"$set": {"skills": bson::to_bson(&pins).unwrap(), "skills_revision": 1_i64}},
            )
            .await
            .unwrap();
    }
    enable_guest_test_bot(&state, &row.channel_bot_id).await;
    let thread = db
        .collection::<NyxbotThread>(THREADS)
        .find_one(doc! {"channel_id": &row.id, "partition": PARTITION})
        .await
        .unwrap()
        .unwrap();
    chats::update_chat(
        &state,
        OWNER,
        &thread.id,
        &chats::ChatSettings {
            agent_id: Some(specialist.id.clone()),
            ..Default::default()
        },
    )
    .await
    .unwrap();

    let inventory =
        serde_json::to_value(skills::get(db, OWNER, &row.channel_bot_id).await.unwrap()).unwrap();
    assert_eq!(inventory["standing_approval_skill_ids"], json!([&skill_id]));
    assert!(
        inventory["available"]
            .as_array()
            .unwrap()
            .iter()
            .any(|entry| { entry["skill_id"] == private_pin.skill_id })
    );
    let selection = |approve_override| {
        serde_json::from_value(json!({
            "agent_id": &default_agent.id, "expected_revision": 0, "skills_revision": 1,
            "attachments": [
                {"skill_id": &skill_id, "audiences": ["shared"], "all_requests_approved": true},
                {"skill_id": &private_pin.skill_id, "audiences": ["private"], "all_requests_approved": approve_override},
            ],
        }))
        .unwrap()
    };
    assert!(matches!(
        skills::set(
            &state,
            &guest_test_actor(),
            &row.channel_bot_id,
            selection(true)
        )
        .await,
        Err(AppError::ValidationError(_))
    ));
    skills::set(
        &state,
        &guest_test_actor(),
        &row.channel_bot_id,
        selection(false),
    )
    .await
    .unwrap();

    let staged_clear = skills::prepare(
        db,
        OWNER,
        &row.channel_bot_id,
        &serde_json::from_value(json!({
            "agent_id": &default_agent.id, "expected_revision": 1, "skills_revision": 1,
            "attachments": [],
        }))
        .unwrap(),
    )
    .await
    .unwrap();

    let worker = guest::test_hooks::pause_worker(db);
    let response = Box::pin(verified_telegram_guest(
        &state,
        &row,
        &key,
        "Show the channel guide",
        "guest-skill-chat-override",
    ))
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    tokio::time::timeout(Duration::from_secs(30), worker.wait_for_provider())
        .await
        .expect("guest reached the provider on the override specialist");
    let conversation = channel_conversations(&state, &row.id)
        .await
        .into_iter()
        .find(|row| guest::is_hosted_conversation(&row.id))
        .unwrap();
    let chat = chat_for_row(&state, OWNER, &conversation).await;
    assert_eq!(chat.agent_id, specialist.id);
    assert_eq!(
        chat.channel_guest.as_ref().unwrap().audience,
        guest::AnswerAudience::Shared,
    );
    let visible = skills::visible_agent(db, &chat).await.unwrap();
    assert_eq!(visible.skills, [override_pin.clone()]);
    let path = format!("/api/v1/skills/{skill_id}/versions/1.0.0/download");
    let permit = skills::authorize(db, &chat, &skill_id, &path)
        .await
        .unwrap();
    assert_eq!(permit.sha256, override_pin.sha256);
    assert!(!permit.all_requests_approved);
    assert!(permit.revalidate(db).await.is_ok());
    let private_path = format!(
        "/api/v1/skills/{}/versions/1.0.0/download",
        private_pin.skill_id
    );
    assert!(matches!(
        skills::authorize(db, &chat, &private_pin.skill_id, &private_path).await,
        Err(AppError::Forbidden(_))
    ));

    chats::update_chat(
        &state,
        OWNER,
        &thread.id,
        &chats::ChatSettings {
            agent_id: Some("default".into()),
            ..Default::default()
        },
    )
    .await
    .unwrap();
    assert!(matches!(
        permit.revalidate(db).await,
        Err(AppError::Forbidden(_))
    ));
    assert!(matches!(
        skills::visible_agent(db, &chat).await,
        Err(AppError::Forbidden(_))
    ));
    let context = (db, row.channel_bot_id.as_str(), &staged_clear);
    let apply = crate::services::transaction_service::run_db_only(
        db,
        &context,
        |tx, &(db, bot_id, staged_clear)| {
            Box::pin(skills::apply_prepared_in_session(
                db,
                tx,
                bot_id,
                staged_clear,
            ))
        },
    )
    .await;
    assert!(matches!(apply, Err(AppError::Conflict(_))));
    let fresh = Box::pin(verified_telegram_guest(
        &state,
        &row,
        &key,
        "Continue with the new specialist",
        "guest-skill-reassigned",
    ))
    .await;
    assert_eq!(fresh.status(), StatusCode::OK);
    tokio::time::timeout(Duration::from_secs(30), worker.wait_for_provider())
        .await
        .expect("fresh guest work reaches the new specialist while the old turn is settling");
    let rows = channel_conversations(&state, &row.id).await;
    let current = rows
        .iter()
        .find(|row| {
            guest::is_hosted_conversation(&row.id)
                && row.agent_id.as_deref() == Some(default_agent.id.as_str())
        })
        .expect("reassignment must start a conversation for the new specialist");
    assert_ne!(current.id, conversation.id);
    assert_ne!(
        current.credential_api_key_id,
        conversation.credential_api_key_id
    );
    assert!(
        rows.iter()
            .any(|row| row.id == conversation.id && row.active_turn.is_some())
    );
    let current_chat = chat_for_row(&state, OWNER, current).await;
    let original_actor = &chat.channel_guest.as_ref().unwrap().actor;
    assert_eq!(
        original_actor.assurance,
        guest::GuestAssurance::ProviderVerified
    );
    assert_eq!(
        &current_chat.channel_guest.as_ref().unwrap().actor,
        original_actor
    );

    assert_eq!(
        skills::visible_agent(db, &current_chat)
            .await
            .unwrap()
            .skills,
        [default_pin]
    );

    drop(worker);
    assert!(!body_text(response).await.contains("Here is your answer"));
    assert!(body_text(fresh).await.contains("Here is your answer"));
    assert_hosted_native_replay_checks_each_answer_frame(&state, &row.id, &key, &current.id).await;
    server.abort();
    state.db.drop().await.unwrap();
}

async fn assert_hosted_native_replay_checks_each_answer_frame(
    state: &AppState,
    channel_id: &str,
    key: &str,
    conversation_id: &str,
) {
    use crate::{
        models::nyxbot_gateway_response::{COLLECTION_NAME as RESPONSES, GatewayResponse},
        services::nyxbot_response_recovery as recovery,
    };
    use futures::StreamExt;

    let row = load_channel(state, OWNER, channel_id).await.unwrap();
    let events = state.db.collection::<NyxbotEvent>(EVENTS);
    let event = events
        .find_one(doc! {"channel_id": channel_id, "conversation_id": conversation_id})
        .await
        .unwrap()
        .unwrap();
    let id = "hosted_answer_replay";
    let mut payload = recovery::Payload::new(id, "hosted-test", Default::default());
    payload.finish_text(Some("Hosted answer payload"), false);
    state
        .db
        .collection::<GatewayResponse>(RESPONSES)
        .insert_one(GatewayResponse {
            id: Uuid::new_v4().to_string(),
            response_id: id.into(),
            user_id: row.user_id.clone(),
            channel_id: row.id.clone(),
            binding_id: row.binding_id.clone().unwrap(),
            partition: event.partition,
            event_key: event.id.clone(),
            request_hash: "test".into(),
            conversation_id: Some(conversation_id.into()),
            terminal: true,
            answer_delivery: None,
            revision: 0,
            payload_ciphertext: payload.seal_legacy(&state.encryption_keys).await.unwrap(),
            payload_context_version: None,
            created_at: Utc::now(),
            expires_at: Utc::now() + ChronoDuration::hours(1),
        })
        .await
        .unwrap();

    let accepted =
        native_responses::stream(state.clone(), row.clone(), bearer(key), id.into(), None);
    assert!(body_text(accepted).await.contains("Hosted answer payload"));

    let paused = native_responses::stream(state.clone(), row, bearer(key), id.into(), None);
    let mut chunks = paused.into_body().into_data_stream();
    let created = chunks.next().await.unwrap().unwrap();
    assert!(String::from_utf8_lossy(&created).contains("response.created"));
    assert_eq!(
        events
            .delete_one(doc! {"_id": &event.id})
            .await
            .unwrap()
            .deleted_count,
        1
    );

    let mut remainder = String::new();
    while let Some(chunk) = chunks.next().await {
        remainder.push_str(&String::from_utf8_lossy(&chunk.unwrap()));
    }
    assert!(remainder.contains("provider_unavailable"), "{remainder}");
    assert!(!remainder.contains("Hosted answer payload"), "{remainder}");
}

#[tokio::test]
async fn nyxbot_guest_runtime_rollback_admits_fresh_legacy_work_while_stale_worker_is_busy() {
    let (state, calls, server) = setup("guest_runtime_active_rollback").await;
    let (row, key) = channel(&state, "gateway").await;
    assert!(matches!(
        set_guest_flag(&state, Some(true)).await,
        Err(AppError::SecurityReadinessUnavailable)
    ));
    crate::services::security_readiness_service::ready_test_replica(&state.db).await;
    chats::set_private_chats(&state, OWNER, &row.id, "everyone")
        .await
        .unwrap();
    let binding = put_binding(
        State(state.clone()),
        Path("bnd_chats".into()),
        bearer(&key),
        Json(binding_body(&key, OWNER)),
    )
    .await;
    let status = binding.status();
    assert_eq!(status, StatusCode::OK, "{}", body_text(binding).await);
    ensure_partition(&state, &key, PARTITION).await;
    let baseline = event("Ordinary baseline question", "42", "guest-runtime-before");
    assert!(
        body_text(respond(&state, &key, &baseline, "guest_runtime_before").await)
            .await
            .contains("Here is your answer")
    );
    let ordinary = channel_conversations(&state, &row.id).await.remove(0);
    assert!(ordinary.id.starts_with("nyxa-"));

    enable_guest_test_bot(&state, &row.channel_bot_id).await;
    let guest::GuestMode::AddOn {
        activation_epoch, ..
    } = guest::select_mode(&state.db, &row.channel_bot_id)
        .await
        .unwrap()
    else {
        panic!("expected add-on mode")
    };
    let old = event("Blocked add-on request", "42", "guest-runtime-stale");
    let stale_worker = guest::test_hooks::pause_worker(&state.db);
    let blocked_model = calls.lock().await;
    let old_response = respond(&state, &key, &old, "guest_runtime_stale").await;
    assert_eq!(old_response.status(), StatusCode::OK);
    tokio::time::timeout(Duration::from_secs(30), stale_worker.wait_for_provider())
        .await
        .expect("the old guest work must reach the mock provider before disabling");
    let addon = channel_conversations(&state, &row.id)
        .await
        .into_iter()
        .find(|row| guest::is_hosted_conversation(&row.id))
        .expect("add-on conversation");
    let active = addon.active_turn.as_ref().expect("add-on model is busy");
    assert_eq!(
        active.channel_guest.as_ref().unwrap().activation_epoch,
        activation_epoch
    );
    assert_ne!(ordinary.credential_api_key_id, addon.credential_api_key_id);
    guest::require_live_request(
        &state.db,
        OWNER,
        &addon.id,
        &active.turn_id,
        &addon.credential_api_key_id,
    )
    .await
    .unwrap();

    set_guest_flag(&state, Some(false)).await.unwrap();
    assert_eq!(
        guest::select_mode(&state.db, &row.channel_bot_id)
            .await
            .unwrap(),
        guest::GuestMode::Legacy
    );
    assert!(
        guest::revalidate(&state.db, active.channel_guest.as_ref().unwrap())
            .await
            .is_err()
    );
    assert!(
        guest::require_live_request(
            &state.db,
            OWNER,
            &addon.id,
            &active.turn_id,
            &addon.credential_api_key_id,
        )
        .await
        .is_err(),
        "old authenticated worker must fail its final model dispatch check"
    );
    drop(blocked_model);
    let legacy = event("Fresh ordinary request", "42", "guest-runtime-after");
    let response = respond(&state, &key, &legacy, "guest_runtime_after").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(body_text(response).await.contains("Here is your answer"));
    let rows = channel_conversations(&state, &row.id).await;
    assert_eq!(rows.len(), 2);
    let stale = rows.iter().find(|row| row.id == addon.id).unwrap();
    assert!(
        stale.active_turn.is_some(),
        "the old turn is deliberately unsettled"
    );
    let fresh = rows.iter().find(|row| row.id == ordinary.id).unwrap();
    assert_eq!(fresh.credential_api_key_id, ordinary.credential_api_key_id);

    drop(stale_worker);
    assert!(
        !body_text(old_response)
            .await
            .contains("Here is your answer")
    );
    let replay = respond(&state, &key, &old, "guest_runtime_stale").await;
    assert!(!body_text(replay).await.contains("Here is your answer"));

    set_guest_flag(&state, Some(true)).await.unwrap();
    assert!(
        matches!(guest::select_mode(&state.db, &row.channel_bot_id).await.unwrap(),
        guest::GuestMode::AddOn { activation_epoch: current, .. } if current > activation_epoch)
    );
    assert!(
        guest::revalidate(&state.db, active.channel_guest.as_ref().unwrap())
            .await
            .is_err()
    );
    set_guest_flag(&state, None).await.unwrap();
    assert_eq!(
        guest::select_mode(&state.db, &row.channel_bot_id)
            .await
            .unwrap(),
        guest::GuestMode::Legacy
    );
    server.abort();
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn nyxbot_guest_selector_outage_keeps_verified_owner_and_fences_guest_replay() {
    let (state, calls, server) = setup("guest_selector_outage_owner").await;
    let (row, key) = channel(&state, "gateway").await;
    chats::set_private_chats(&state, OWNER, &row.id, "everyone")
        .await
        .unwrap();
    state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .update_one(
            doc! {"_id": &row.id},
            doc! {"$set": {"owner_sender_ids": ["7"]}},
        )
        .await
        .unwrap();
    let binding = put_binding(
        State(state.clone()),
        Path("bnd_chats".into()),
        bearer(&key),
        Json(binding_body(&key, OWNER)),
    )
    .await;
    assert_eq!(binding.status(), StatusCode::OK);
    ensure_partition(&state, &key, PARTITION).await;
    crate::services::security_readiness_service::ready_test_replica(&state.db).await;
    enable_guest_test_bot(&state, &row.channel_bot_id).await;
    let activations = state
        .db
        .collection::<bson::Document>("channel_guest_activation");
    activations
        .update_one(doc! {"_id": guest::FLAG}, doc! {"$set": {"enabled": false}})
        .await
        .unwrap();
    assert!(matches!(
        guest::select_mode(&state.db, &row.channel_bot_id).await,
        Err(AppError::SecurityReadinessUnavailable)
    ));

    let owner = event("Owner during selector outage", "7", "owner-selector-outage");
    let answer = body_text(respond(&state, &key, &owner, "owner-selector-outage").await).await;
    assert!(answer.contains("Here is your answer"), "{answer}");
    assert_eq!(calls.lock().await.len(), 1);

    let blocked = event(
        "Guest during selector outage",
        "42",
        "guest-selector-outage",
    );
    let denied = body_text(respond(&state, &key, &blocked, "guest-selector-outage").await).await;
    assert!(!denied.contains("Here is your answer"), "{denied}");
    assert_eq!(calls.lock().await.len(), 1);
    let source = dispatch_identity::dispatch_key(&row.id, "guest-selector-outage");
    assert_eq!(
        guest::hosted_admission(&state.db, &source, &row.id)
            .await
            .unwrap(),
        guest::HostedAdmission::Unavailable,
    );

    set_guest_flag(&state, Some(false)).await.unwrap();
    let replay = body_text(respond(&state, &key, &blocked, "guest-selector-outage").await).await;
    assert!(!replay.contains("Here is your answer"), "{replay}");
    assert_eq!(calls.lock().await.len(), 1);
    // An exact relay receipt carries its original outage to hosted admission,
    // even if the separate selector has recovered before the gateway handoff.
    let route = row.route_id.as_deref().unwrap();
    let mut delayed = event(
        "Guest before selector recovery",
        "42",
        "guest-delayed-handoff",
    );
    delayed["event_context"]["activity"]["source"]["route_id"] = json!(route);
    state.db.collection::<bson::Document>(crate::models::channel_message::COLLECTION_NAME)
        .insert_one(doc! {
            "_id": "guest-delayed-handoff", "direction": "inbound", "channel_bot_id": &row.channel_bot_id,
            "user_id": OWNER, "conversation_id": route, "agent_api_key_id": &row.route_api_key_id,
            "platform": &row.platform, "platform_conversation_id": "chat-1",
            "sender_platform_id": "42", "original_message": true,
            "hosted_guest_selector_unavailable": true,
        }).await.unwrap();
    let mut mismatched = delayed.clone();
    mismatched["event_context"]["activity"]["actor"]["id"] = json!("7");
    let wrong_source = respond(&state, &key, &mismatched, "misbound-selector-handoff").await;
    assert_eq!(wrong_source.status(), StatusCode::FORBIDDEN);
    let denied = body_text(respond(&state, &key, &delayed, "guest-delayed-handoff").await).await;
    assert!(!denied.contains("Here is your answer"), "{denied}");
    assert_eq!(calls.lock().await.len(), 1);

    let fresh = event("Fresh ordinary guest", "42", "guest-selector-recovered");
    let answer = body_text(respond(&state, &key, &fresh, "guest-selector-recovered").await).await;
    assert!(answer.contains("Here is your answer"), "{answer}");
    assert_eq!(calls.lock().await.len(), 2);
    server.abort();
    state.db.drop().await.unwrap();
}

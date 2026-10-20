//! Source acceptance tests use the real Mongo transaction and gateway transport.
use super::*;
use crate::models::assistant_conversation::{AgentEvent, TurnOrigin};
use crate::services::{assistant_team_service as team, channel_thread_follow_service as follow};

pub(super) async fn source(f: &Fixture, name: &str) -> NyxbotEvent {
    let mut event = f.event.clone();
    event.id = Uuid::new_v4().to_string();
    event.event_id = name.into();
    event.status = "running".into();
    event.turn_id = None;
    let d = event.delivery.as_mut().unwrap();
    d.state = "pending".into();
    d.target_ciphertext = None;
    d.target_expires_at = None;
    event.event_context_ciphertext = Some(
        f.state
            .encryption_keys
            .encrypt(
                serde_json::to_vec(&json!({"event_ref": format!("cmaeg1.ev.{name}")}))
                    .unwrap()
                    .as_slice(),
            )
            .await
            .unwrap(),
    );
    Mock::given(method("POST"))
        .and(path(format!("/events/cmaeg1.ev.{name}/reply-target")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "reply_target_ref": format!("cmaeg1.rt.{name}"),
            "expires_at": (Utc::now()+ChronoDuration::hours(23)).timestamp()
        })))
        .mount(&f.gateway)
        .await;
    Mock::given(method("POST"))
        .and(path(format!("/reply-targets/cmaeg1.rt.{name}/messages")))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"delivery":{"outcome":"confirmed"}})),
        )
        .mount(&f.gateway)
        .await;
    f.state
        .db
        .collection::<NyxbotEvent>(EVENTS)
        .insert_one(&event)
        .await
        .unwrap();
    event
}
pub(super) fn work(event: &NyxbotEvent, question: &str) -> AgentEvent {
    let mut work = team::bound_event("message", format!("Question: {question}"), None, None);
    work.question_key = engine::question_key(question);
    work.reply_to = vec![event.delivery.as_ref().unwrap().origin.clone()];
    work.source_event_ids = vec![event.id.clone()];
    work
}
async fn queue(f: &Fixture, event: &NyxbotEvent, question: &str) -> bool {
    delivery::enqueue(
        &f.state.db,
        OWNER,
        f.event.conversation_id.as_deref().unwrap(),
        work(event, question),
    )
    .await
    .unwrap()
}
async fn row(f: &Fixture) -> AssistantConversation {
    engine::get(
        &f.state.db,
        OWNER,
        f.event.conversation_id.as_deref().unwrap(),
    )
    .await
    .unwrap()
}
async fn record(f: &Fixture, id: &str) -> NyxbotEvent {
    f.state
        .db
        .collection(EVENTS)
        .find_one(doc! {"_id":id})
        .await
        .unwrap()
        .unwrap()
}

#[tokio::test]
async fn accepted_queued_sources_mint_original_targets_without_a_turn_or_budget() {
    let f = fixture("source_mint_without_turn").await;
    let a = source(&f, "first").await;
    let b = source(&f, "second").await;
    assert!(queue(&f, &a, "first question").await);
    assert!(queue(&f, &b, "second question").await);
    late_delivery::sweep(&f.state).await.unwrap();
    for (event, target) in [(&a, "cmaeg1.rt.first"), (&b, "cmaeg1.rt.second")] {
        let saved = record(&f, &event.id).await;
        assert_eq!(saved.status, "queued");
        assert!(saved.turn_id.is_none());
        let d = saved.delivery.unwrap();
        assert!(d.checked_at > event.created_at);
        let plain = f
            .state
            .encryption_keys
            .decrypt(d.target_ciphertext.as_ref().unwrap())
            .await
            .unwrap();
        assert!(String::from_utf8(plain).unwrap().contains(target));
    }
    assert!(sends(&f).await.is_empty());
    let mut queued = row(&f).await;
    assert!(
        crate::services::async_service_operation::select_events(&mut queued, TurnOrigin::User)
            .is_empty()
    );
    assert_eq!(queued.pending_events.len(), 2);
    assert_eq!(
        crate::services::async_service_operation::select_events(&mut queued, TurnOrigin::Event)
            .len(),
        1
    );
    assert_eq!(queued.pending_events.len(), 1);
}

#[tokio::test]
async fn accepted_sources_cannot_be_evicted_by_any_generic_push_and_capacity_races() {
    let f = fixture("source_capacity_race").await;
    for i in 0..19 {
        let event = source(&f, &format!("q{i}")).await;
        assert!(queue(&f, &event, &format!("question {i}")).await);
    }
    let a = source(&f, "racea").await;
    let b = source(&f, "raceb").await;
    let (a_ok, b_ok) = tokio::join!(queue(&f, &a, "question a"), queue(&f, &b, "question b"));
    assert_ne!(a_ok, b_ok);
    let before = row(&f).await;
    assert_eq!(before.pending_events.len(), 20);
    let result = engine::push_events(
        &f.state.db,
        OWNER,
        &before.id,
        vec![team::event("system", "durable update".into(), None)],
    )
    .await;
    assert!(matches!(result, Err(AppError::Conflict(_))));
    assert_eq!(
        row(&f)
            .await
            .pending_events
            .iter()
            .map(|e| &e.id)
            .collect::<Vec<_>>(),
        before
            .pending_events
            .iter()
            .map(|e| &e.id)
            .collect::<Vec<_>>()
    );
    let rejected = record(&f, if a_ok { &b.id } else { &a.id }).await;
    assert_eq!(rejected.status, "running");
    assert!(rejected.turn_id.is_none());
    // Ordinary updates retain their oldest-first policy without evicting a source.
    let mut mixed = before.pending_events.clone();
    mixed[1] = team::event("system", "old system update".into(), None);
    let replaced = mixed[1].id.clone();
    f.state
        .db
        .collection::<bson::Document>(CONVERSATIONS)
        .update_one(
            doc! {"_id":&before.id},
            doc! {"$set":{"pending_events":bson::to_bson(&mixed).unwrap()}},
        )
        .await
        .unwrap();
    let added = team::event("system", "new system update".into(), None);
    let after = engine::push_events(&f.state.db, OWNER, &before.id, vec![added.clone()])
        .await
        .unwrap()
        .unwrap();
    assert_eq!(after.pending_events.len(), 20);
    assert!(!after.pending_events.iter().any(|e| e.id == replaced));
    assert!(after.pending_events.iter().any(|e| e.id == mixed[0].id));
    assert!(after.pending_events.iter().any(|e| e.id == added.id));
}

#[tokio::test]
async fn queue_transaction_rolls_back_source_acceptance_when_conversation_write_fails() {
    let f = fixture("source_admission_rollback").await;
    let event = source(&f, "rollback").await;
    f.state.db.run_command(doc! {"collMod":CONVERSATIONS,"validator":{"pending_events.0":{"$exists":false}},"validationLevel":"strict"}).await.unwrap();
    assert!(
        delivery::enqueue(
            &f.state.db,
            OWNER,
            f.event.conversation_id.as_deref().unwrap(),
            work(&event, "question")
        )
        .await
        .is_err()
    );
    assert!(row(&f).await.pending_events.is_empty());
    let saved = record(&f, &event.id).await;
    assert_eq!(saved.status, "running");
    assert!(saved.turn_id.is_none());
}

#[tokio::test]
async fn coalesced_sources_are_pruned_independently_and_only_valid_original_is_sent_once() {
    let f = fixture("source_independent_revoke").await;
    let a = source(&f, "revoked").await;
    let b = source(&f, "valid").await;
    assert!(queue(&f, &a, "same question").await);
    assert!(queue(&f, &b, "same question").await);
    assert_eq!(row(&f).await.pending_events.len(), 1);
    assert_eq!(row(&f).await.pending_events[0].source_event_ids.len(), 2);
    f.state
        .db
        .collection::<bson::Document>(EVENTS)
        .update_one(
            doc! {"_id":&a.id},
            doc! {"$set":{"delivery.sender_id":"revoked-owner"}},
        )
        .await
        .unwrap();
    assert_eq!(
        follow::prune_queue(
            &f.state.db,
            OWNER,
            f.event.conversation_id.as_deref().unwrap()
        )
        .await
        .unwrap(),
        1
    );
    let queued = row(&f).await;
    assert_eq!(
        queued.pending_events[0].source_event_ids,
        vec![b.id.clone()]
    );
    assert_eq!(record(&f, &a.id).await.delivery.unwrap().state, "refused");
    // Bind the surviving accepted destination to the answer; no latest-message fallback.
    f.state
        .db
        .collection::<bson::Document>(EVENTS)
        .update_one(
            doc! {"_id":&b.id},
            doc! {"$set":{"turn_id":&f.event.turn_id}},
        )
        .await
        .unwrap();
    settled(&f, false).await;
    late_delivery::process(&f.state, &b.id).await;
    late_delivery::process(&f.state, &b.id).await;
    let attempts = sends(&f).await;
    assert_eq!(attempts.len(), 1);
    assert_eq!(
        attempts[0].url.path(),
        "/reply-targets/cmaeg1.rt.valid/messages"
    );
}

#[tokio::test]
async fn generic_invalid_head_and_legacy_unanchored_messages_drain_durably() {
    let f = fixture("source_poison_drain").await;
    let a = source(&f, "invalid").await;
    assert!(queue(&f, &a, "invalid source").await);
    f.state.db.collection::<bson::Document>(EVENTS).update_one(doc! {"_id":&a.id},doc! {"$set":{"expires_at":bson::DateTime::from_chrono(Utc::now()-ChronoDuration::seconds(1))}}).await.unwrap();
    let mut legacy = work(&a, "legacy question");
    legacy.source_event_ids.clear();
    engine::push_events(
        &f.state.db,
        OWNER,
        f.event.conversation_id.as_deref().unwrap(),
        vec![legacy],
    )
    .await
    .unwrap();
    assert_eq!(
        follow::prune_queue(
            &f.state.db,
            OWNER,
            f.event.conversation_id.as_deref().unwrap()
        )
        .await
        .unwrap(),
        0
    );
    assert!(row(&f).await.pending_events.is_empty());
    assert_eq!(record(&f, &a.id).await.status, "refused");
}

#[tokio::test]
async fn every_coalesced_destination_is_revalidated_and_recipient_limit_is_atomic() {
    let f = fixture("source_duplicate_cap").await;
    for i in 0..=engine::MAX_ALSO_DELIVER {
        let a = source(&f, &format!("repeat{i}")).await;
        assert!(queue(&f, &a, "same question").await);
    }
    let overflow = source(&f, "overflow").await;
    assert!(!queue(&f, &overflow, "same question").await);
    assert_eq!(record(&f, &overflow.id).await.status, "running");
    assert_eq!(
        row(&f).await.pending_events[0].source_event_ids.len(),
        engine::MAX_ALSO_DELIVER + 1
    );
    let invalid = source(&f, "badroute").await;
    f.state
        .db
        .collection::<bson::Document>(crate::models::api_key::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&f.channel.route_api_key_id},
            doc! {"$set":{"user_id":"another-owner"}},
        )
        .await
        .unwrap();
    assert!(
        delivery::enqueue(
            &f.state.db,
            OWNER,
            f.event.conversation_id.as_deref().unwrap(),
            work(&invalid, "another question")
        )
        .await
        .is_err()
    );
    assert_eq!(record(&f, &invalid.id).await.status, "running");
}

#[tokio::test]
async fn successful_blank_stream_and_detached_duplicate_destinations_are_terminal_silence() {
    let f = fixture("source_blank_terminal").await;
    settled(&f, false).await;
    f.state
        .db
        .collection::<bson::Document>(crate::models::assistant_message::COLLECTION_NAME)
        .update_one(
            doc! {"conversation_id":&f.event.conversation_id},
            doc! {"$set":{"text":" \n\t"}},
        )
        .await
        .unwrap();
    let (tx, rx) = broadcast::channel(8);
    tx.send(json!({"event":"turn.completed","status":"completed"}))
        .unwrap();
    let response = late_delivery::provider_stream(
        f.state.clone(),
        f.event.id.clone(),
        "response".into(),
        json!({"type":"response.created"}),
        rx,
        Utc::now(),
    );
    let body = body_text(response).await;
    assert!(body.contains("response.completed"));
    assert!(!body.contains("response.output_text"));
    assert_eq!(status(&f).await, "silent");
    let b = source(&f, "blankduplicate").await;
    f.state
        .db
        .collection::<bson::Document>(EVENTS)
        .update_one(
            doc! {"_id":&b.id},
            doc! {"$set":{"status":"queued","turn_id":&f.event.turn_id}},
        )
        .await
        .unwrap();
    late_delivery::process(&f.state, &b.id).await;
    late_delivery::process(&f.state, &b.id).await;
    assert_eq!(record(&f, &b.id).await.delivery.unwrap().state, "silent");
    assert!(sends(&f).await.is_empty());
    for barrier in ["sending", "streamed", "unknown"] {
        patch(&f, doc! {"delivery.state":barrier}).await;
        assert!(!delivery::silence(&f.state.db, &f.event.id).await.unwrap());
        assert_eq!(status(&f).await, barrier);
    }
}

async fn owner_gateway_home(f: &Fixture) -> AssistantConversation {
    // Owner private ingress selects the agent's real home, which has no
    // channel binding. Other late-delivery fixtures deliberately bind one.
    f.state
        .db
        .collection::<bson::Document>(CONVERSATIONS)
        .update_one(
            doc! {"_id": &f.event.conversation_id},
            doc! {"$set": {"channel": bson::Bson::Null}},
        )
        .await
        .unwrap();
    assert_eq!(
        put_binding(
            State(f.state.clone()),
            Path("source-queue-binding".into()),
            bearer(&f.key),
            Json(binding_body(&f.key, OWNER))
        )
        .await
        .status(),
        StatusCode::OK
    );
    assert_eq!(
        put_conversation(
            State(f.state.clone()),
            Path(("source-queue-binding".into(), PARTITION.into())),
            bearer(&f.key)
        )
        .await
        .status(),
        StatusCode::OK
    );
    row(f).await
}

#[tokio::test]
async fn queued_gateway_handler_is_quiet_and_duplicate_replay_never_adds_busy_output() {
    let f = fixture("source_gateway_queue_handler").await;
    let conversation = owner_gateway_home(&f).await;
    // A real persisted owner turn blocks the new channel question; do not run a model.
    engine::begin_turn(
        &f.state.db,
        OWNER,
        &engine::TurnRequest {
            acknowledgement_id: None,
            retry_of: None,
            attachment_ids: vec![],
            agent_id: None,
            conversation_id: Some(conversation.id.clone()),
            text: "current owner work".into(),
            model: None,
            access_mode: None,
        },
        &f.state.encryption_keys,
    )
    .await
    .unwrap();
    let request = super::super::event("next queued question", "7", "queued-handler");
    for _ in 0..2 {
        let response = respond(&f.state, &f.key, &request, "queued-handler").await;
        assert_eq!(response.status(), StatusCode::OK);
        let body = body_text(response).await;
        assert!(body.contains("response.completed"), "{body}");
        assert!(!body.contains("response.output_text"), "{body}");
        assert!(!body.contains("Got it"));
        assert!(!body.contains("working on"));
    }
    let queued = row(&f).await;
    assert_eq!(queued.pending_events.len(), 1);
    let saved = record(&f, &queued.pending_events[0].source_event_ids[0]).await;
    assert_eq!(saved.status, "queued");
    assert_eq!(
        saved.conversation_id.as_deref(),
        Some(conversation.id.as_str())
    );
    assert!(saved.event_context_ciphertext.is_some());
    assert!(saved.turn_id.is_none());
}

#[tokio::test]
async fn specialist_owner_private_home_keeps_its_original_delivery_authority() {
    let f = fixture("source_specialist_private_home").await;
    let (agent, home) = team::create_specialist(
        &f.state.db,
        &f.state.encryption_keys,
        OWNER,
        team::CreateRequest {
            name: "support".into(),
            description: "Support".into(),
            display_name: None,
            persona: None,
            targets: Default::default(),
            account_read: false,
            specialty: None,
            machines: None,
            logins: None,
            created_by: "user",
        },
    )
    .await
    .unwrap()
    .unwrap();
    let origin = f.event.delivery.as_ref().unwrap().origin.clone();
    f.state
        .db
        .collection::<bson::Document>(THREADS)
        .update_one(
            doc! {"channel_id":&f.channel.id,"partition":PARTITION},
            doc! {"$set":{"conversation_id":&home.id,"agent_id":&agent.id}},
        )
        .await
        .unwrap();
    assert!(
        delivery::target(&f.state.db, &home, &origin)
            .await
            .unwrap()
            .is_some()
    );
    let mut event = source(&f, "specialist").await;
    event.conversation_id = Some(home.id.clone());
    let accepted = delivery::enqueue(
        &f.state.db,
        OWNER,
        &home.id,
        work(&event, "specialist question"),
    )
    .await
    .unwrap();
    assert!(accepted);
    assert_eq!(
        record(&f, &event.id).await.conversation_id.as_deref(),
        Some(home.id.as_str())
    );
}

async fn finish(f: &Fixture, row: &AssistantConversation, text: &str) {
    engine::finish_turn(
        &f.state.db,
        row,
        &row.credential_api_key_id,
        &Uuid::new_v4().to_string(),
        &engine::TurnResult {
            text: text.into(),
            session_id: None,
            response_id: None,
            error: None,
        },
    )
    .await
    .unwrap();
}
fn request(conversation: &str, text: &str) -> engine::TurnRequest {
    engine::TurnRequest {
        acknowledgement_id: None,
        retry_of: None,
        attachment_ids: vec![],
        agent_id: None,
        conversation_id: Some(conversation.into()),
        text: text.into(),
        model: None,
        access_mode: None,
    }
}
#[tokio::test]
async fn queued_event_admission_binds_all_original_sources_and_settles_exactly_once() {
    let f = fixture("source_full_admission_settlement").await;
    let initial = row(&f).await;
    let prior = engine::begin_turn(
        &f.state.db,
        OWNER,
        &request(&initial.id, "ongoing work"),
        &f.state.encryption_keys,
    )
    .await
    .unwrap();
    let a = source(&f, "answer-a").await;
    let b = source(&f, "answer-b").await;
    let unrelated = source(&f, "next-question").await;
    assert!(queue(&f, &a, "one coalesced question").await);
    assert!(queue(&f, &b, "one coalesced question").await);
    assert!(queue(&f, &unrelated, "a different question").await);
    // A later inbox message changes the ordinary thread reply anchor.
    f.state
        .db
        .collection::<bson::Document>(THREADS)
        .update_one(
            doc! {"channel_id":&f.channel.id,"partition":PARTITION},
            doc! {"$set":{"last_message_id":"C-latest","last_relay_message_id":"C-latest"}},
        )
        .await
        .unwrap();
    finish(&f, &prior, "prior answer").await;
    let mut start: engine::TurnStart = (&request(&initial.id, "")).into();
    start.origin = TurnOrigin::Event;
    let admitted = engine::begin_turn(&f.state.db, OWNER, start, &f.state.encryption_keys)
        .await
        .unwrap();
    let turn = admitted.active_turn.as_ref().unwrap();
    assert_ne!(turn.turn_id, prior.active_turn.as_ref().unwrap().turn_id);
    assert_eq!(turn.events.len(), 1);
    assert_eq!(
        turn.events[0].source_event_ids,
        vec![a.id.clone(), b.id.clone()]
    );
    assert_eq!(admitted.pending_events.len(), 1);
    assert_eq!(turn.channel_event_id.as_deref(), Some(a.id.as_str()));
    assert!(matches!(
        engine::answer_routing(&admitted, turn).primary,
        engine::PrimaryDestination::ChannelEvent { .. }
    ));
    for source in [&a, &b] {
        assert_eq!(
            record(&f, &source.id).await.turn_id.as_deref(),
            Some(turn.turn_id.as_str())
        );
    }
    assert!(record(&f, &unrelated.id).await.turn_id.is_none());
    finish(&f, &admitted, "one shared answer").await;
    for _ in 0..2 {
        late_delivery::process(&f.state, &a.id).await;
        late_delivery::process(&f.state, &b.id).await;
    }
    let attempts = sends(&f).await;
    assert_eq!(attempts.len(), 2);
    let paths = attempts.iter().map(|r| r.url.path()).collect::<Vec<_>>();
    assert!(paths.contains(&"/reply-targets/cmaeg1.rt.answer-a/messages"));
    assert!(paths.contains(&"/reply-targets/cmaeg1.rt.answer-b/messages"));
    for source in [&a, &b] {
        let saved = record(&f, &source.id).await;
        let d = saved.delivery.unwrap();
        assert_eq!(d.state, "sent");
        assert!(d.claim_id.is_some());
    }
    assert!(record(&f, &unrelated.id).await.turn_id.is_none());
}

#[tokio::test]
async fn mixed_legacy_and_new_message_queues_isolate_questions_from_ordinary_turns() {
    let f = fixture("source_legacy_selection").await;
    let a = source(&f, "selection").await;
    let mut legacy = work(&a, "legacy native question");
    legacy.source_event_ids.clear();
    legacy.reply_to[0].thread = Some(Box::new(
        crate::models::channel_thread_follow::ThreadTurnBinding {
            sender_kind: None,
            child_id: "original-native-child".into(),
            source_message_id: "original-native-source".into(),
            sender_id: "7".into(),
            guest: false,
            revision: 1,
            generation: 1,
            channel_generation: 1,
            agent_id: "original-agent".into(),
            queued: true,
        },
    ));
    let mut generic = work(&a, "legacy generic channel question");
    generic.source_event_ids.clear();
    // Selection must isolate old channel messages before origin validation;
    // filter_events independently retains validated native origins/drops generic ones.
    let modern = work(&a, "new question");
    let system = team::event("system", "system update".into(), None);
    let internal = team::event("message", "internal team update".into(), None);
    let mut row = row(&f).await;
    row.pending_events = vec![
        legacy.clone(),
        generic.clone(),
        modern.clone(),
        system.clone(),
        internal.clone(),
    ];
    assert_eq!(
        crate::services::async_service_operation::select_events(&mut row, TurnOrigin::Channel)
            .iter()
            .map(|e| &e.id)
            .collect::<Vec<_>>(),
        vec![&system.id, &internal.id]
    );
    assert_eq!(row.pending_events.len(), 3);
    let selected =
        crate::services::async_service_operation::select_events(&mut row, TurnOrigin::Event);
    assert_eq!(selected.len(), 1);
    assert_eq!(selected[0].id, legacy.id);
    assert_eq!(selected[0].reply_to, legacy.reply_to);
    assert_eq!(
        crate::services::async_service_operation::select_events(&mut row, TurnOrigin::Event)[0].id,
        generic.id
    );
    assert_eq!(
        crate::services::async_service_operation::select_events(&mut row, TurnOrigin::Event)[0].id,
        modern.id
    );
    row.pending_events = vec![system.clone(), internal.clone()];
    assert_eq!(
        crate::services::async_service_operation::select_events(&mut row, TurnOrigin::Event)
            .iter()
            .map(|e| &e.id)
            .collect::<Vec<_>>(),
        vec![&system.id, &internal.id]
    );
}

#[tokio::test]
async fn post_admission_reply_preference_failure_preserves_queued_and_bound_sources() {
    for repeat_active in [false, true] {
        let f = fixture("source_preference_failure").await;
        let home = owner_gateway_home(&f).await;
        let prior = engine::begin_turn(
            &f.state.db,
            OWNER,
            &request(&home.id, "current owner question"),
            &f.state.encryption_keys,
        )
        .await
        .unwrap();
        // Reject the real post-admission preference update, without preventing
        // source acceptance or turn settlement. No simulated provider error.
        f.state
            .db
            .run_command(doc! {"collMod": CONVERSATIONS,
            "validator": {"reply_channel.nyxbot_channel_id": {"$ne": &f.channel.id}},
            "validationLevel": "strict"})
            .await
            .unwrap();
        let origin = &f.event.delivery.as_ref().unwrap().origin;
        assert!(
            engine::set_reply_channel(&f.state.db, OWNER, &home.id, origin)
                .await
                .is_err()
        );
        let question = if repeat_active {
            "current owner question"
        } else {
            "next question"
        };
        let mut incoming = super::super::event(question, "7", "preference-failed");
        incoming["event_context"]["event_ref"] = json!("cmaeg1.ev.original");
        for _ in 0..2 {
            let response = respond(&f.state, &f.key, &incoming, "preference-failed").await;
            assert_eq!(response.status(), StatusCode::OK);
            let body = body_text(response).await;
            assert!(body.contains("response.completed"), "{body}");
            assert!(!body.contains("response.output_text"), "{body}");
        }
        let saved = f
            .state
            .db
            .collection::<NyxbotEvent>(EVENTS)
            .find_one(doc! {"channel_id": &f.channel.id, "event_id": "preference-failed"})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(saved.status, "queued");
        assert!(saved.event_context_ciphertext.is_some());
        assert_eq!(saved.turn_id.is_some(), repeat_active);
        assert_eq!(
            row(&f).await.pending_events.len(),
            usize::from(!repeat_active)
        );
        assert_eq!(
            row(&f).await.active_turn.as_ref().unwrap().turn_id,
            prior.active_turn.as_ref().unwrap().turn_id
        );
        // A late provider error must not discard either kind of durable
        // acceptance, regardless of its native-budget bookkeeping lane.
        for budget_claimed in [false, true] {
            f.state
                .db
                .collection::<bson::Document>(EVENTS)
                .update_one(
                    doc! {"_id": &saved.id},
                    doc! {"$set": {"native_bot_budget_claimed": budget_claimed}},
                )
                .await
                .unwrap();
            discard_failed_provider_event(&f.state.db, &saved.id).await;
            let retained = record(&f, &saved.id).await;
            assert_eq!(retained.status, "queued");
            assert_eq!(
                retained.event_context_ciphertext,
                saved.event_context_ciphertext
            );
            assert_eq!(retained.turn_id, saved.turn_id);
        }
        late_delivery::sweep(&f.state).await.unwrap();
        assert!(
            record(&f, &saved.id)
                .await
                .delivery
                .unwrap()
                .target_ciphertext
                .is_some()
        );
        finish(&f, &prior, "original app answer").await;
        if !repeat_active {
            let mut start: engine::TurnStart = (&request(&home.id, "")).into();
            start.origin = TurnOrigin::Event;
            let turn = engine::begin_turn(&f.state.db, OWNER, start, &f.state.encryption_keys)
                .await
                .unwrap();
            assert_eq!(turn.active_turn.as_ref().unwrap().events.len(), 1);
            finish(&f, &turn, "queued answer").await;
        }
        late_delivery::process(&f.state, &saved.id).await;
        late_delivery::sweep(&f.state).await.unwrap();
        assert_eq!(sends(&f).await.len(), 1);
        assert_eq!(record(&f, &saved.id).await.delivery.unwrap().state, "sent");
    }
}

#[tokio::test]
async fn personal_channel_repeat_joins_owner_app_turn_without_another_execution() {
    let f = fixture("source_app_repeat").await;
    let home = owner_gateway_home(&f).await;
    let prior = engine::begin_turn(
        &f.state.db,
        OWNER,
        &request(&home.id, "same app question"),
        &f.state.encryption_keys,
    )
    .await
    .unwrap();
    let a = source(&f, "app-repeat").await;
    assert!(queue(&f, &a, "same app question").await);
    let current = row(&f).await;
    assert!(current.pending_events.is_empty());
    let active = current.active_turn.as_ref().unwrap();
    assert_eq!(active.turn_id, prior.active_turn.as_ref().unwrap().turn_id);
    assert_eq!(active.origin, TurnOrigin::User);
    assert!(active.channel_event_id.is_none());
    assert!(matches!(
        engine::answer_routing(&current, active).primary,
        engine::PrimaryDestination::App
    ));
    assert_eq!(
        record(&f, &a.id).await.turn_id.as_deref(),
        Some(active.turn_id.as_str())
    );
    assert_eq!(
        f.state
            .db
            .collection::<bson::Document>(crate::models::assistant_message::COLLECTION_NAME)
            .count_documents(doc! {"conversation_id": &home.id, "role": "user"})
            .await
            .unwrap(),
        1
    );
    finish(&f, &prior, "one app answer").await;
    late_delivery::sweep(&f.state).await.unwrap();
    late_delivery::process(&f.state, &a.id).await;
    assert_eq!(sends(&f).await.len(), 1);
    assert_eq!(record(&f, &a.id).await.delivery.unwrap().state, "sent");
}

#[tokio::test]
async fn personal_channel_repeat_cannot_join_an_owner_invocation() {
    use crate::services::assistant_invocation_service as invocations;
    let f = fixture("source_invocation_repeat").await;
    let home = owner_gateway_home(&f).await;
    invocations::ensure_indexes(&f.state.db).await.unwrap();
    let start = engine::TurnStart::from(&request(&home.id, "same private question"));
    let admission = Box::pin(invocations::admit(
        &f.state.db,
        OWNER,
        &Uuid::new_v4().to_string(),
        "source-repeat-test",
        &start,
        &f.state.encryption_keys,
    ))
    .await
    .unwrap();
    let turn_id = admission.invocation.turn_id.clone();
    let a = source(&f, "invocation-repeat").await;
    assert!(queue(&f, &a, "same private question").await);
    let current = row(&f).await;
    let active = current.active_turn.as_ref().unwrap();
    assert_eq!(active.turn_id, turn_id);
    assert_eq!(
        active.invocation_id.as_deref(),
        Some(admission.invocation.id.as_str())
    );
    assert!(active.channel_event_id.is_none());
    assert_eq!(current.pending_events.len(), 1);
    assert_eq!(
        current.pending_events[0].source_event_ids,
        vec![a.id.clone()]
    );
    assert!(current.pending_events[0].invocation_id.is_none());
    assert!(record(&f, &a.id).await.turn_id.is_none());
    let origin = &a.delivery.as_ref().unwrap().origin;
    assert!(
        !engine::also_deliver(&f.state.db, OWNER, &home.id, &turn_id, origin)
            .await
            .unwrap()
    );
    let mut continuation = team::event("notice", "private follow-up".into(), None)
        .for_invocation(Some(&admission.invocation.id));
    continuation.question_key = engine::question_key("private follow-up question");
    let key = continuation.question_key.clone().unwrap();
    engine::push_events(&f.state.db, OWNER, &home.id, vec![continuation])
        .await
        .unwrap();
    assert!(
        !engine::also_reply_to_queued(&f.state.db, OWNER, &home.id, &key, origin)
            .await
            .unwrap()
    );
    invocations::cancel(&f.state, OWNER, &admission.invocation.idempotency_key)
        .await
        .unwrap();
    let mut next = engine::TurnStart::from(&request(&home.id, ""));
    next.origin = TurnOrigin::Event;
    let next = Box::pin(engine::begin_turn(
        &f.state.db,
        OWNER,
        &next,
        &f.state.encryption_keys,
    ))
    .await
    .unwrap();
    let next_turn = next.active_turn.as_ref().unwrap();
    assert!(next_turn.invocation_id.is_none());
    assert_ne!(next_turn.turn_id, turn_id);
    assert_eq!(
        record(&f, &a.id).await.turn_id.as_deref(),
        Some(next_turn.turn_id.as_str())
    );
    assert!(sends(&f).await.is_empty());
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn original_route_revocation_prunes_only_that_source_before_event_execution() {
    use crate::models::channel_conversation::COLLECTION_NAME as ROUTES;
    for change in [
        "delete",
        "inactive",
        "key",
        "owner",
        "bot",
        "platform",
        "replacement",
    ] {
        let f = fixture("source_route_revalidation").await;
        let a = source(&f, "revoked-route").await;
        let mut b = source(&f, "valid-route").await;
        let (mut other, _) = super::super::channel(&f.state, "gateway").await;
        other.owner_sender_ids = vec!["7".into()];
        f.state
            .db
            .collection::<NyxbotChannel>(CHANNELS)
            .replace_one(doc! {"_id": &other.id}, &other)
            .await
            .unwrap();
        let mut thread = f
            .state
            .db
            .collection::<NyxbotThread>(THREADS)
            .find_one(doc! {"channel_id": &f.channel.id})
            .await
            .unwrap()
            .unwrap();
        thread.id = Uuid::new_v4().to_string();
        thread.channel_id = other.id.clone();
        f.state
            .db
            .collection::<NyxbotThread>(THREADS)
            .insert_one(thread)
            .await
            .unwrap();
        let d = b.delivery.as_mut().unwrap();
        d.origin.nyxbot_channel_id = other.id.clone();
        d.source_route_id = other.route_id.clone();
        d.source_route_api_key_id = Some(other.route_api_key_id.clone());
        f.state
            .db
            .collection::<NyxbotEvent>(EVENTS)
            .replace_one(doc! {"_id": &b.id}, &b)
            .await
            .unwrap();
        assert!(queue(&f, &a, "same route question").await);
        assert!(queue(&f, &b, "same route question").await);
        let routes = f.state.db.collection::<bson::Document>(ROUTES);
        let route_id = f.channel.route_id.as_deref().unwrap();
        match change {
            "delete" => {
                routes.delete_one(doc! {"_id": route_id}).await.unwrap();
            }
            "replacement" => {
                // A new link route cannot lend authority to an old accepted source.
                let mut replacement = routes
                    .find_one(doc! {"_id": route_id})
                    .await
                    .unwrap()
                    .unwrap();
                let id = Uuid::new_v4().to_string();
                replacement.insert("_id", &id);
                routes.insert_one(replacement).await.unwrap();
                f.state
                    .db
                    .collection::<bson::Document>(CHANNELS)
                    .update_one(doc! {"_id": &f.channel.id}, doc! {"$set": {"route_id": id}})
                    .await
                    .unwrap();
            }
            _ => {
                let field = match change {
                    "inactive" => "is_active",
                    "key" => "agent_api_key_id",
                    "owner" => "user_id",
                    "bot" => "channel_bot_id",
                    "platform" => "platform",
                    _ => unreachable!(),
                };
                let mut set = doc! {};
                set.insert(
                    field,
                    if change == "inactive" {
                        bson::Bson::Boolean(false)
                    } else {
                        bson::Bson::String("changed".into())
                    },
                );
                routes
                    .update_one(doc! {"_id": route_id}, doc! {"$set": set})
                    .await
                    .unwrap();
            }
        }
        let mut start: engine::TurnStart =
            (&request(f.event.conversation_id.as_deref().unwrap(), "")).into();
        start.origin = TurnOrigin::Event;
        let admitted = engine::begin_turn(&f.state.db, OWNER, start, &f.state.encryption_keys)
            .await
            .unwrap();
        let active = admitted.active_turn.as_ref().unwrap();
        assert_eq!(active.events.len(), 1, "{change}");
        assert_eq!(
            active.events[0].source_event_ids,
            vec![b.id.clone()],
            "{change}"
        );
        assert_eq!(
            active.events[0].reply_to[0].nyxbot_channel_id, other.id,
            "{change}"
        );
        assert_eq!(record(&f, &a.id).await.status, "refused", "{change}");
        assert!(record(&f, &a.id).await.turn_id.is_none(), "{change}");
        assert_eq!(
            record(&f, &b.id).await.turn_id.as_deref(),
            Some(active.turn_id.as_str())
        );
        finish(&f, &admitted, "valid route answer").await;
        late_delivery::sweep(&f.state).await.unwrap();
        late_delivery::sweep(&f.state).await.unwrap();
        let attempts = sends(&f).await;
        assert_eq!(attempts.len(), 1, "{change}");
        assert_eq!(
            attempts[0].url.path(),
            "/reply-targets/cmaeg1.rt.valid-route/messages"
        );
    }
}

#[tokio::test]
async fn managed_telegram_raw_route_retains_canonical_channel_queue_authority() {
    let mut f = fixture("source_managed_telegram_route").await;
    f.state
        .db
        .collection::<bson::Document>(crate::models::channel_bot::COLLECTION_NAME)
        .update_one(
            doc! {"_id": &f.channel.channel_bot_id},
            doc! {"$set": {"platform": "telegram-new"}},
        )
        .await
        .unwrap();
    f.state
        .db
        .collection::<bson::Document>(crate::models::api_key::COLLECTION_NAME)
        .update_one(
            doc! {"_id": &f.channel.route_api_key_id},
            doc! {"$set": {"callback_url": "https://example.test/managed-gateway"}},
        )
        .await
        .unwrap();
    f.state
        .db
        .collection::<bson::Document>(crate::models::channel_conversation::COLLECTION_NAME)
        .delete_one(doc! {"_id": &f.channel.route_id})
        .await
        .unwrap();
    // Real provisioning stores the bot's raw platform, not the channel family.
    let route = crate::services::channel_routing_service::create_conversation(
        &f.state.db,
        OWNER,
        Some(&f.channel.channel_bot_id),
        "telegram-new",
        "*",
        "private",
        None,
        &f.channel.route_api_key_id,
        true,
        false,
    )
    .await
    .unwrap();
    assert_eq!(route.platform, "telegram-new");
    assert_eq!(f.channel.platform, "telegram");
    f.channel.route_id = Some(route.id.clone());
    f.state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .replace_one(doc! {"_id": &f.channel.id}, &f.channel)
        .await
        .unwrap();
    f.event.delivery.as_mut().unwrap().source_route_id = Some(route.id);
    let a = source(&f, "managed-telegram").await;
    assert!(queue(&f, &a, "managed Telegram question").await);
    let mut start: engine::TurnStart =
        (&request(f.event.conversation_id.as_deref().unwrap(), "")).into();
    start.origin = TurnOrigin::Event;
    let admitted = engine::begin_turn(&f.state.db, OWNER, start, &f.state.encryption_keys)
        .await
        .unwrap();
    let active = admitted.active_turn.as_ref().unwrap();
    assert_eq!(active.events[0].source_event_ids, vec![a.id.clone()]);
    assert_eq!(
        record(&f, &a.id).await.turn_id.as_deref(),
        Some(active.turn_id.as_str())
    );
    finish(&f, &admitted, "managed answer").await;
    late_delivery::sweep(&f.state).await.unwrap();
    late_delivery::sweep(&f.state).await.unwrap();
    assert_eq!(sends(&f).await.len(), 1);
    assert_eq!(record(&f, &a.id).await.delivery.unwrap().state, "sent");
}

use super::*;
use crate::models::{assistant_message::AssistantMessage, nyxbot_channel::ChannelTurnDelivery};
use crate::services::channel_turn_delivery as delivery;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

#[path = "nyxbot_image_delivery_tests.rs"]
mod image_delivery_tests;

#[path = "nyxbot_source_queue_tests.rs"]
mod source_queue_tests;

struct Fixture {
    thread_id: String,
    calls: Calls,
    state: AppState,
    channel: NyxbotChannel,
    key: String,
    event: NyxbotEvent,
    gateway: MockServer,
    upstream: tokio::task::JoinHandle<()>,
}
impl Fixture {
    async fn seal_reference(
        &self,
        keys: &crate::crypto::aes::EncryptionKeys,
        bytes: &[u8],
    ) -> Vec<u8> {
        seal_thread_reference(keys, &self.thread_id, OWNER, &self.channel.id, bytes).await
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.upstream.abort();
    }
}

async fn fixture(name: &str) -> Fixture {
    Box::pin(fixture_with_record_writes(name, true)).await
}

async fn fixture_with_record_writes(name: &str, contextual: bool) -> Fixture {
    let (mut state, calls, upstream) = setup(name).await;
    let mut keys = crate::test_utils::test_encryption_keys();
    keys.set_contextual_record_writes(contextual);
    state.encryption_keys = Arc::new(keys);
    let (mut channel, key) = Box::pin(channel(&state, "gateway")).await;
    channel.owner_sender_ids = vec!["7".into()];
    state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .replace_one(doc! {"_id": &channel.id}, &channel)
        .await
        .unwrap();
    let mut bot = bot_doc("telegram", "test");
    bot.insert("_id", &channel.channel_bot_id);
    state
        .db
        .collection::<bson::Document>(crate::models::channel_bot::COLLECTION_NAME)
        .replace_one(
            doc! {"_id": bot.get_str("_id").unwrap().to_owned()},
            bot.clone(),
        )
        .upsert(true)
        .await
        .unwrap();
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
    Mock::given(method("POST")).and(path("/events/cmaeg1.ev.original/reply-target"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"reply_target_ref": "cmaeg1.rt.original", "expires_at": (Utc::now() + ChronoDuration::hours(23)).timestamp()})))
        .mount(&gateway).await;
    Mock::given(method("POST"))
        .and(path("/reply-targets/cmaeg1.rt.original/messages"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"delivery": {"outcome": "confirmed"}})),
        )
        .mount(&gateway)
        .await;
    let agent = crate::services::assistant_team_service::ensure_nyxbot(&state.db, OWNER)
        .await
        .unwrap();
    let conversation = Box::pin(crate::services::assistant_team_service::home_thread(
        &state.db,
        &state.encryption_keys,
        &agent,
    ))
    .await
    .unwrap();
    let thread_id = Uuid::new_v4().to_string();
    state.db.collection::<bson::Document>(THREADS).insert_one(doc! {
        "_id": &thread_id, "channel_id": &channel.id, "user_id": OWNER,
        "partition": PARTITION, "conversation_id": &conversation.id, "kind": "private", "owner_chat": true,
        "platform_chat_id": "chat-1", "created_at": bson::DateTime::now(), "updated_at": bson::DateTime::now(),
    }).await.unwrap();
    let now = Utc::now();
    let mut event = admission(&channel.id, "original");
    event.conversation_id = Some(conversation.id.clone());
    event.turn_id = Some(Uuid::new_v4().to_string());
    set_event_context(
        &state.encryption_keys,
        &mut event,
        br#"{"event_ref":"cmaeg1.ev.original"}"#,
    )
    .await;
    event.delivery = Some(ChannelTurnDelivery {
        restart_owner_verified: false,
        notice_language: None,
        execution_source_bound: false,
        execution_owner_verified: false,
        sender_kind: None,
        version: 1,
        state: "waiting".into(),
        origin: ChannelOrigin {
            org_execution: None,
            native_event_id: None,
            nyxbot_channel_id: channel.id.clone(),
            partition: PARTITION.into(),
            platform: "telegram".into(),
            thread: None,
        },
        sender_id: "7".into(),
        guest: false,
        addressed: true,
        transport: "gateway".into(),
        source_route_id: channel.route_id.clone(),
        source_route_api_key_id: Some(channel.route_api_key_id.clone()),
        stream_deadline: now + ChronoDuration::seconds(570),
        checked_at: now,
        claim_id: None,
        target_context_version: None,
        target_ciphertext: None,
        target_expires_at: None,
    });
    state.db.collection::<bson::Document>(CONVERSATIONS).update_one(
        doc! {"_id": &conversation.id}, doc! {"$set": {"channel": bson::to_bson(&event.delivery.as_ref().unwrap().origin).unwrap()}},
    ).await.unwrap();
    state
        .db
        .collection::<NyxbotEvent>(EVENTS)
        .insert_one(&event)
        .await
        .unwrap();
    Fixture {
        thread_id,
        calls,
        state,
        channel,
        key,
        event,
        gateway,
        upstream,
    }
}

async fn settled(f: &Fixture, failed: bool) {
    f.state.db.collection::<bson::Document>(crate::models::assistant_message::COLLECTION_NAME).insert_one(doc! {
        "_id": Uuid::new_v4().to_string(), "user_id": OWNER, "conversation_id": &f.event.conversation_id,
        "turn_id": &f.event.turn_id, "seq": 1, "role": "assistant", "text": "Useful answer", "status": if failed {"failed"} else {"completed"},
        "created_at": bson::DateTime::now(),
    }).await.unwrap();
    f.state
        .db
        .collection::<bson::Document>(CONVERSATIONS)
        .update_one(
            doc! {"_id":&f.event.conversation_id},
            doc! {"$max":{"message_count":1_i64}},
        )
        .await
        .unwrap();
}
async fn status(f: &Fixture) -> String {
    f.state
        .db
        .collection::<NyxbotEvent>(EVENTS)
        .find_one(doc! {"_id": &f.event.id})
        .await
        .unwrap()
        .unwrap()
        .delivery
        .unwrap()
        .state
}
async fn sends(f: &Fixture) -> Vec<wiremock::Request> {
    f.gateway
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .filter(|r| r.url.path().ends_with("/messages"))
        .collect()
}
async fn patch(f: &Fixture, fields: bson::Document) {
    f.state
        .db
        .collection::<bson::Document>(EVENTS)
        .update_one(doc! {"_id": &f.event.id}, doc! {"$set": fields})
        .await
        .unwrap();
}

#[tokio::test]
async fn long_turn_deadline_is_quiet_then_durable_late_answer_once() {
    let f = fixture("channel_late_deadline").await;
    f.state.db.collection::<bson::Document>(CONVERSATIONS).update_one(doc! {"_id": &f.event.conversation_id}, doc! {"$set": {"active_turn": {
        "turn_id": &f.event.turn_id, "origin": "channel", "started_at": bson::DateTime::now(), "stop_requested": false,
    }}}).await.unwrap();
    let (_sender, receiver) = broadcast::channel(8);
    let response = late_delivery::provider_stream(
        f.state.clone(),
        f.event.id.clone(),
        "response".into(),
        json!({"type":"response.created"}),
        InboundTurn {
            receiver,
            hosted_reply: None,
        },
        Utc::now() - ChronoDuration::seconds(late_delivery::STREAM_SECS + 1),
    );
    let text = body_text(response).await;
    assert!(!text.contains("response.output_text"));
    assert!(!text.contains("response.output_item"));
    assert!(text.contains("response.completed"));
    assert!(!text.contains("response.failed"));
    late_delivery::sweep(&f.state).await.unwrap();
    assert!(sends(&f).await.is_empty());
    settled(&f, false).await;
    late_delivery::sweep(&f.state).await.unwrap();
    late_delivery::sweep(&f.state).await.unwrap();
    assert_eq!(sends(&f).await.len(), 1);
    assert_eq!(status(&f).await, "sent");
    let requests = sends(&f).await;
    assert_eq!(
        requests[0].headers.get("idempotency-key").unwrap(),
        f.event.id.as_str()
    );
}

#[tokio::test]
async fn committed_stream_is_never_late_sent_even_after_deadline() {
    let f = fixture("channel_late_stream_wins").await;
    settled(&f, false).await;
    let (sender, receiver) = broadcast::channel(8);
    sender
        .send(json!({"event":"block.completed", "block":{"text":"Useful answer"}}))
        .unwrap();
    sender
        .send(json!({"event":"turn.completed", "status":"completed"}))
        .unwrap();
    let response = late_delivery::provider_stream(
        f.state.clone(),
        f.event.id.clone(),
        "response".into(),
        json!({"type":"response.created"}),
        InboundTurn {
            receiver,
            hosted_reply: None,
        },
        Utc::now(),
    );
    assert!(body_text(response).await.contains("Useful answer"));
    assert_eq!(status(&f).await, "streamed");
    patch(&f, doc! {"delivery.stream_deadline": bson::DateTime::from_chrono(Utc::now() - ChronoDuration::hours(1))}).await;
    delivery::detach(&f.state.db, &f.event.id).await.unwrap();
    late_delivery::sweep(&f.state).await.unwrap();
    assert!(sends(&f).await.is_empty());
}

#[tokio::test]
async fn hosted_reply_delivery_custom_replay_rechecks_the_original_event() {
    use futures::StreamExt;

    let mut f = fixture("hosted_reply_custom_replay").await;
    let ordinary = f.event.conversation_id.clone();
    let hosted = || Some(format!("nyxag-{}", Uuid::new_v4().simple()));
    let events = f.state.db.collection::<NyxbotEvent>(EVENTS);
    for (conversation, replacement, expect_answer) in [
        (hosted(), None, false),
        (hosted(), ordinary.clone(), false),
        (ordinary.clone(), None, true),
    ] {
        f.event.conversation_id = conversation;
        f.event.turn_id = Some(Uuid::new_v4().to_string());
        f.event.custom_binding_id = Some("custom-test".into());
        f.event.status = "completed".into();
        f.event.delivery.as_mut().unwrap().guest = true;
        events
            .replace_one(doc! {"_id": &f.event.id}, &f.event)
            .upsert(true)
            .await
            .unwrap();
        settled(&f, false).await;

        let response = custom_replay(&f.state, &f.event.id, "response").await;
        let mut chunks = response.into_body().into_data_stream();
        let created = chunks.next().await.unwrap().unwrap();
        assert!(String::from_utf8_lossy(&created).contains("response.created"));
        events.delete_one(doc! {"_id": &f.event.id}).await.unwrap();
        if let Some(replacement) = replacement {
            let mut event = f.event.clone();
            event.conversation_id = Some(replacement);
            events.insert_one(event).await.unwrap();
        }
        let mut body = String::new();
        while let Some(chunk) = chunks.next().await {
            body.push_str(&String::from_utf8_lossy(&chunk.unwrap()));
        }

        assert!(body.contains("response.completed"), "{body}");
        assert_eq!(body.contains("Useful answer"), expect_answer, "{body}");
    }
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn hosted_reply_delivery_active_direct_stream_requires_its_original_turn() {
    let f = fixture("hosted_reply_direct_stream").await;
    let events = f.state.db.collection::<NyxbotEvent>(EVENTS);
    let conversation = "nyxag-1234567890abcdef1234567890abcdef";
    let original_turn = Uuid::new_v4().to_string();
    let response = |hosted_reply| {
        let (sender, receiver) = broadcast::channel(8);
        sender
            .send(json!({"event":"block.completed","block":{"text":"Useful answer"}}))
            .unwrap();
        sender
            .send(json!({"event":"turn.completed","status":"completed"}))
            .unwrap();
        late_delivery::provider_stream_direct(
            f.state.clone(),
            f.event.id.clone(),
            "response".into(),
            json!({"type":"response.created"}),
            InboundTurn {
                receiver,
                hosted_reply,
            },
            Utc::now(),
        )
    };

    let missing = response(late_delivery::HostedReply::new(
        conversation,
        Some(&original_turn),
    ));
    events.delete_one(doc! {"_id": &f.event.id}).await.unwrap();
    let missing = body_text(missing).await;
    assert!(missing.contains("response.completed"), "{missing}");
    assert!(!missing.contains("Useful answer"), "{missing}");

    let mut replaced = f.event.clone();
    replaced.conversation_id = Some(conversation.into());
    replaced.delivery.as_mut().unwrap().guest = true;
    events.insert_one(replaced).await.unwrap();
    let stale = body_text(response(late_delivery::HostedReply::new(
        conversation,
        Some(&original_turn),
    )))
    .await;
    assert!(stale.contains("response.completed"), "{stale}");
    assert!(!stale.contains("Useful answer"), "{stale}");
    assert_eq!(status(&f).await, "waiting");

    // Old provider streams can finish a direct result after the event is gone.
    let ordinary = response(None);
    events.delete_one(doc! {"_id": &f.event.id}).await.unwrap();
    assert!(body_text(ordinary).await.contains("Useful answer"));
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn partial_failure_survives_both_stream_and_late_delivery() {
    let f = fixture("channel_late_partial").await;
    settled(&f, true).await;
    let (sender, receiver) = broadcast::channel(8);
    sender
        .send(json!({"event":"block.completed", "block":{"text":"Useful answer"}}))
        .unwrap();
    sender.send(json!({"event":"turn.completed", "status":"failed", "error":{"code":"upstream_failed"}})).unwrap();
    let partial = final_reply(receiver).await.unwrap();
    assert!(partial.contains("Useful answer"));
    assert!(partial.contains("Incomplete"));
    delivery::detach(&f.state.db, &f.event.id).await.unwrap();
    late_delivery::sweep(&f.state).await.unwrap();
    let requests = sends(&f).await;
    assert_eq!(requests.len(), 1);
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(body["text"], partial);
    let long = "界".repeat(9000);
    assert!(late_delivery::answer(&long, true).starts_with(&long));
    assert_eq!(late_delivery::answer(&long, false), long);
}

#[tokio::test]
async fn replica_races_claim_one_late_send_and_stream_cannot_win_after_detach() {
    let f = fixture("channel_late_race").await;
    settled(&f, false).await;
    delivery::detach(&f.state.db, &f.event.id).await.unwrap();
    let workers = (0..16).map(|_| late_delivery::process(&f.state, &f.event.id));
    futures::future::join_all(workers).await;
    assert_eq!(sends(&f).await.len(), 1);
    assert!(
        !delivery::claim_stream(&f.state.db, &f.event.id, "completed")
            .await
            .unwrap()
    );
    assert_eq!(status(&f).await, "sent");
}

#[tokio::test]
async fn guest_access_and_reply_mode_are_live_at_late_delivery() {
    for mode in [
        "private_closed",
        "group_members",
        "group_mentions",
        "owner_revoked",
        "route_revoked",
        "bot_disabled",
        "relinked",
    ] {
        let f = fixture(&format!("channel_late_acl_{mode}")).await;
        settled(&f, false).await;
        delivery::detach(&f.state.db, &f.event.id).await.unwrap();
        match mode {
            "private_closed" | "group_members" => {
                patch(
                    &f,
                    doc! {"delivery.guest": true, "delivery.sender_id": "guest"},
                )
                .await;
                if mode == "group_members" {
                    f.state
                        .db
                        .collection::<bson::Document>(THREADS)
                        .update_one(
                            doc! {"channel_id": &f.channel.id},
                            doc! {"$set": {"kind":"group", "members":"owner"}},
                        )
                        .await
                        .unwrap();
                }
            }
            "group_mentions" => {
                patch(&f, doc! {"delivery.addressed": false}).await;
                f.state
                    .db
                    .collection::<bson::Document>(THREADS)
                    .update_one(
                        doc! {"channel_id": &f.channel.id},
                        doc! {"$set": {"kind":"group", "reply_mode":"mention"}},
                    )
                    .await
                    .unwrap();
            }
            "owner_revoked" => {
                f.state
                    .db
                    .collection::<bson::Document>(CHANNELS)
                    .update_one(
                        doc! {"_id": &f.channel.id},
                        doc! {"$set":{"owner_sender_ids":[]}},
                    )
                    .await
                    .unwrap();
            }
            "route_revoked" => {
                f.state
                    .db
                    .collection::<bson::Document>(crate::models::api_key::COLLECTION_NAME)
                    .update_one(
                        doc! {"_id": &f.channel.route_api_key_id},
                        doc! {"$set":{"is_active":false}},
                    )
                    .await
                    .unwrap();
            }
            "bot_disabled" => {
                f.state
                    .db
                    .collection::<bson::Document>(crate::models::channel_bot::COLLECTION_NAME)
                    .update_one(
                        doc! {"_id": &f.channel.channel_bot_id},
                        doc! {"$set":{"is_active":false}},
                    )
                    .await
                    .unwrap();
            }
            "relinked" => {
                f.state
                    .db
                    .collection::<bson::Document>(THREADS)
                    .update_one(
                        doc! {"channel_id": &f.channel.id},
                        doc! {"$set":{"conversation_id":"different-thread"}},
                    )
                    .await
                    .unwrap();
            }
            _ => unreachable!(),
        }
        late_delivery::sweep(&f.state).await.unwrap();
        assert!(sends(&f).await.is_empty(), "{mode}");
        assert_eq!(status(&f).await, "refused", "{mode}");
    }
}

#[tokio::test]
async fn original_reply_target_survives_event_expiry_and_newer_chat_activity() {
    let f = fixture("channel_late_original_target").await;
    assert_eq!(f.event.event_context_version, Some(3));
    delivery::detach(&f.state.db, &f.event.id).await.unwrap();
    late_delivery::sweep(&f.state).await.unwrap(); // mint before settlement
    let event = f
        .state
        .db
        .collection::<NyxbotEvent>(EVENTS)
        .find_one(doc! {"_id": &f.event.id})
        .await
        .unwrap()
        .unwrap();
    let delivery = event.delivery.as_ref().unwrap();
    assert_eq!(delivery.target_context_version, Some(3));
    patch(
        &f,
        doc! {"created_at": bson::DateTime::from_chrono(Utc::now() - ChronoDuration::hours(1))},
    )
    .await;
    f.state
        .db
        .collection::<bson::Document>(THREADS)
        .update_one(
            doc! {"channel_id": &f.channel.id},
            doc! {"$set":{"last_message_id":"newer-inbound"}},
        )
        .await
        .unwrap();
    settled(&f, false).await;
    delivery::detach(&f.state.db, &f.event.id).await.unwrap();
    late_delivery::sweep(&f.state).await.unwrap();
    assert_eq!(sends(&f).await.len(), 1);
    assert_eq!(status(&f).await, "sent");
}

#[tokio::test]
async fn uncertain_dispatch_and_crashed_claim_are_never_reissued() {
    let f = fixture("channel_late_uncertain").await;
    f.gateway.reset().await;
    Mock::given(method("POST")).and(path("/events/cmaeg1.ev.original/reply-target"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"reply_target_ref":"cmaeg1.rt.original", "expires_at":(Utc::now()+ChronoDuration::hours(23)).timestamp()}))).mount(&f.gateway).await;
    Mock::given(method("POST"))
        .and(path("/reply-targets/cmaeg1.rt.original/messages"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"delivery":{"outcome":"unknown"}})),
        )
        .mount(&f.gateway)
        .await;
    settled(&f, false).await;
    delivery::detach(&f.state.db, &f.event.id).await.unwrap();
    late_delivery::sweep(&f.state).await.unwrap();
    assert_eq!(status(&f).await, "unknown");
    late_delivery::sweep(&f.state).await.unwrap();
    assert_eq!(sends(&f).await.len(), 1);
    patch(&f, doc! {"delivery.state":"sending"}).await;
    late_delivery::sweep(&f.state).await.unwrap();
    assert_eq!(sends(&f).await.len(), 1);
}

#[tokio::test]
async fn legacy_admissions_are_never_claimed_by_new_replicas() {
    let f = fixture("channel_late_legacy").await;
    settled(&f, false).await;
    f.state
        .db
        .collection::<bson::Document>(EVENTS)
        .update_one(doc! {"_id": &f.event.id}, doc! {"$unset":{"delivery":""}})
        .await
        .unwrap();
    late_delivery::sweep(&f.state).await.unwrap();
    assert!(
        !delivery::claim_stream(&f.state.db, &f.event.id, "completed")
            .await
            .unwrap()
    );
    assert!(
        !delivery::claim_send(&f.state.db, &f.event.id, "attempt")
            .await
            .unwrap()
    );
    assert!(sends(&f).await.is_empty());
}

#[tokio::test]
async fn dropped_gateway_response_recovers_the_actual_durable_turn() {
    let f = fixture("channel_late_drop_e2e").await;
    // The manually seeded event is unrelated to the real request below.
    f.state
        .db
        .collection::<NyxbotEvent>(EVENTS)
        .delete_one(doc! {"_id": &f.event.id})
        .await
        .unwrap();
    let mut input = event("Please do the work", "7", "e2e");
    input["event_context"]["event_ref"] = json!("cmaeg1.ev.original");
    let response = respond(&f.state, &f.key, &input, "e2e").await;
    assert_eq!(response.status(), StatusCode::OK);
    drop(response); // even a body that was never polled must detach
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            late_delivery::sweep(&f.state).await.unwrap();
            if !sends(&f).await.is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
    })
    .await
    .expect("the detached answer reached the gateway");
    late_delivery::sweep(&f.state).await.unwrap();
    assert_eq!(sends(&f).await.len(), 1);
    let stored = f
        .state
        .db
        .collection::<NyxbotEvent>(EVENTS)
        .find_one(doc! {"event_id":"e2e"})
        .await
        .unwrap()
        .unwrap();
    assert!(stored.turn_id.is_some());
    assert!(stored.conversation_id.is_some());
    let message = f
        .state
        .db
        .collection::<AssistantMessage>(crate::models::assistant_message::COLLECTION_NAME)
        .find_one(doc! {
            "turn_id": stored.turn_id, "role":"assistant",
        })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(message.text, "Here is your answer");
}

#[tokio::test]
async fn guest_solicited_reply_needs_no_posting_authority() {
    let f = fixture("channel_late_guest_reply").await;
    settled(&f, false).await;
    patch(
        &f,
        doc! {"delivery.guest": true, "delivery.sender_id": "guest"},
    )
    .await;
    f.state
        .db
        .collection::<bson::Document>(CHANNELS)
        .update_one(
            doc! {"_id": &f.channel.id},
            doc! {"$set":{"private_chats":"everyone"}},
        )
        .await
        .unwrap();
    f.state
        .db
        .collection::<bson::Document>(THREADS)
        .update_one(
            doc! {"channel_id": &f.channel.id},
            doc! {"$set":{"owner_chat":false, "allow_posts":false}},
        )
        .await
        .unwrap();
    delivery::detach(&f.state.db, &f.event.id).await.unwrap();
    late_delivery::sweep(&f.state).await.unwrap();
    assert_eq!(sends(&f).await.len(), 1);
    assert_eq!(status(&f).await, "sent");
}

#[tokio::test]
async fn partial_failure_is_a_committed_provider_message_not_generic_failure() {
    let f = fixture("channel_late_partial_stream").await;
    settled(&f, true).await;
    let (sender, receiver) = broadcast::channel(8);
    sender
        .send(json!({"event":"block.completed", "block":{"text":"Useful partial"}}))
        .unwrap();
    sender.send(json!({"event":"turn.completed", "status":"failed", "error":{"code":"upstream_failed"}})).unwrap();
    let response = late_delivery::provider_stream(
        f.state.clone(),
        f.event.id.clone(),
        "response".into(),
        json!({"type":"response.created"}),
        InboundTurn {
            receiver,
            hosted_reply: None,
        },
        Utc::now(),
    );
    let text = body_text(response).await;
    assert!(text.contains("Useful partial"));
    assert!(text.contains("Incomplete"));
    assert!(text.contains("response.output_item.done"));
    assert!(text.contains("response.completed"));
    assert!(!text.contains("response.failed"));
    assert_eq!(status(&f).await, "streamed");
    late_delivery::sweep(&f.state).await.unwrap();
    assert!(sends(&f).await.is_empty());
}

#[tokio::test]
async fn reply_target_unavailable_uses_only_original_unexpired_event_once() {
    let f = fixture("channel_late_legacy_gateway").await;
    f.gateway.reset().await;
    Mock::given(method("POST"))
        .and(path("/events/cmaeg1.ev.original/reply-target"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&f.gateway)
        .await;
    Mock::given(method("POST"))
        .and(path("/events/cmaeg1.ev.original/replies"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"message":{"delivery":{"outcome":"confirmed"}}})),
        )
        .mount(&f.gateway)
        .await;
    settled(&f, false).await;
    delivery::detach(&f.state.db, &f.event.id).await.unwrap();
    late_delivery::sweep(&f.state).await.unwrap();
    late_delivery::sweep(&f.state).await.unwrap();
    assert_eq!(status(&f).await, "sent");
    assert_eq!(
        f.gateway
            .received_requests()
            .await
            .unwrap()
            .iter()
            .filter(|r| r.url.path().ends_with("/replies"))
            .count(),
        1
    );
}

#[tokio::test]
async fn event_binding_rolls_back_with_admission_and_stale_claim_cannot_finish() {
    let f = fixture("channel_late_binding").await;
    patch(
        &f,
        doc! {"status":"running", "conversation_id":bson::Bson::Null, "turn_id":bson::Bson::Null},
    )
    .await;
    let mut session = f.state.db.client().start_session().await.unwrap();
    session.start_transaction().await.unwrap();
    delivery::bind_in_session(
        &f.state.db,
        &mut session,
        &f.event.id,
        OWNER,
        "chat",
        "turn",
    )
    .await
    .unwrap();
    session.abort_transaction().await.unwrap();
    let event = f
        .state
        .db
        .collection::<NyxbotEvent>(EVENTS)
        .find_one(doc! {"_id": &f.event.id})
        .await
        .unwrap()
        .unwrap();
    assert!(event.turn_id.is_none());
    session.start_transaction().await.unwrap();
    delivery::bind_in_session(
        &f.state.db,
        &mut session,
        &f.event.id,
        OWNER,
        "chat",
        "turn",
    )
    .await
    .unwrap();
    session.commit_transaction().await.unwrap();
    session.start_transaction().await.unwrap();
    assert!(
        delivery::bind_in_session(
            &f.state.db,
            &mut session,
            &f.event.id,
            OWNER,
            "other",
            "other"
        )
        .await
        .is_err()
    );
    session.abort_transaction().await.unwrap();
    delivery::detach(&f.state.db, &f.event.id).await.unwrap();
    assert!(
        delivery::claim_send(&f.state.db, &f.event.id, "winner")
            .await
            .unwrap()
    );
    delivery::finish(&f.state.db, &f.event.id, "loser", "sent")
        .await
        .unwrap();
    assert_eq!(status(&f).await, "sending");
    delivery::finish(&f.state.db, &f.event.id, "winner", "sent")
        .await
        .unwrap();
    assert_eq!(status(&f).await, "sent");
}

#[tokio::test]
async fn external_bot_delayed_answer_keeps_guest_authority_despite_owner_id_overlap() {
    let f = fixture("external_bot_delayed_guest").await;
    // This ID is intentionally already present in owner_sender_ids. The
    // authenticated bot actor classification must survive stream detachment.
    f.state.db.collection::<bson::Document>(THREADS).update_one(
        doc! {"channel_id":&f.channel.id},
        doc! {"$set":{"kind":"group","members":"everyone","owner_seen":false,"owner_chat":false}},
    ).await.unwrap();
    patch(
        &f,
        doc! {"delivery.guest":true,"delivery.sender_kind":"bot","delivery.state":"pending"},
    )
    .await;
    f.state
        .db
        .collection::<bson::Document>(CONVERSATIONS)
        .update_one(
            doc! {"_id":&f.event.conversation_id},
            doc! {"$set":{"guest_turn":true}},
        )
        .await
        .unwrap();
    settled(&f, false).await;
    late_delivery::sweep(&f.state).await.unwrap();
    late_delivery::sweep(&f.state).await.unwrap();
    assert_eq!(
        status(&f).await,
        "sent",
        "a bot stays an admitted guest through delayed delivery"
    );
    assert_eq!(
        sends(&f).await.len(),
        1,
        "settlement cannot duplicate an effect"
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn detached_observer_capacity_notice_dispatches_at_settlement_once() {
    let f = fixture("channel_capacity_detached_observer").await;
    settled(&f, true).await;
    f.state
        .db
        .collection::<bson::Document>(crate::models::assistant_message::COLLECTION_NAME)
        .update_one(
            doc! {"conversation_id": &f.event.conversation_id, "turn_id": &f.event.turn_id},
            doc! {"$set": {"text": "", "error_code": "capacity_exceeded", "error_detail": {
                "stage": "upstream_response", "http_status": 503, "attempts": 20,
                "turn_id": &f.event.turn_id, "occurred_at": bson::DateTime::now(),
                "continuations": 0, "capacity_wait_ms": 120_098_i64
            }}},
        )
        .await
        .unwrap();

    // An older replica can disappear between recording observer detach and
    // giving the late sender ownership. The stream deadline is still ahead.
    patch(&f, doc! {"gateway_observer_detached": true}).await;
    late_delivery::settled(&f.state, &f.event.id).await;
    let requests = sends(&f).await;
    assert_eq!(requests.len(), 1);
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(
        body["text"],
        crate::services::assistant_turn_observability::capacity_message(Some(120_098))
    );
    late_delivery::sweep(&f.state).await.unwrap();
    assert_eq!(sends(&f).await.len(), 1);
}

#[tokio::test]
async fn late_capacity_failure_preserves_complete_partial_reply_and_wait_copy() {
    let f = fixture("channel_late_capacity_complete").await;
    settled(&f, true).await;
    let text = "界".repeat(9000);
    f.state
        .db
        .collection::<bson::Document>(crate::models::assistant_message::COLLECTION_NAME)
        .update_one(
            doc! {"conversation_id": &f.event.conversation_id, "turn_id": &f.event.turn_id},
            doc! {"$set": {"text": &text, "error_code": "capacity_exceeded"}},
        )
        .await
        .unwrap();
    delivery::detach(&f.state.db, &f.event.id).await.unwrap();
    late_delivery::sweep(&f.state).await.unwrap();
    late_delivery::sweep(&f.state).await.unwrap();
    let requests = sends(&f).await;
    assert_eq!(requests.len(), 1);
    let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
    assert_eq!(
        body["text"],
        format!(
            "{text}\n\n{}",
            crate::services::assistant_turn_observability::capacity_message(None)
        )
    );
}

/// The real progress adapter sends after the threshold while leaving the
/// event's final-delivery claim available, then the durable answer sends once.
#[tokio::test]
async fn checking_notice_arrives_after_five_seconds_without_consuming_final_answer() {
    use crate::services::assistant_checking_notice as notice;
    let (mut f, platform) =
        Box::pin(image_delivery_tests::image_fixture("telegram", "gateway")).await;
    // This case admits its own live turn; remove the image fixture's settled reply.
    f.state
        .db
        .collection::<bson::Document>(crate::models::assistant_message::COLLECTION_NAME)
        .delete_many(doc! {"conversation_id": &f.event.conversation_id})
        .await
        .unwrap();
    Mock::given(path("/bottoken/sendMessage"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"ok":true,"result":{"message_id":99}})),
        )
        .expect(1)
        .mount(&platform)
        .await;
    Mock::given(method("POST"))
        .and(path("/events/cmaeg1.ev.original/replies"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok": true})))
        .expect(0)
        .mount(&f.gateway)
        .await;
    let request = serde_json::from_value::<engine::TurnRequest>(json!({
        "text": "Check current availability", "conversation_id": &f.event.conversation_id,
    }))
    .unwrap();
    let mut row = Box::pin(engine::begin_turn(
        &f.state.db,
        OWNER,
        &request,
        &f.state.encryption_keys,
    ))
    .await
    .unwrap();
    let origin = f.event.delivery.as_ref().unwrap().origin.clone();
    let turn = row.active_turn.as_mut().unwrap();
    turn.origin = crate::models::assistant_conversation::TurnOrigin::Channel;
    turn.asked_from = Some(origin.clone());
    turn.channel_event_id = Some(f.event.id.clone());
    turn.external_check_started = true;
    let began = std::time::Instant::now();
    turn.started_at = Utc::now();
    f.event.turn_id = Some(turn.turn_id.clone());
    f.state
        .db
        .collection::<bson::Document>(CONVERSATIONS)
        .update_one(
            doc! {"_id": &row.id},
            doc! {"$set": {"active_turn": bson::to_bson(turn).unwrap()}},
        )
        .await
        .unwrap();
    patch(&f, doc! {"turn_id": &f.event.turn_id}).await;
    let reference = f
        .seal_reference(&f.state.encryption_keys, b"cmaeg1.ev.original")
        .await;
    f.state.db.collection::<bson::Document>(THREADS).update_one(
        doc! {"_id": &f.thread_id},
        doc! {"$set": {
            "event_ref_ciphertext": bson::Binary {subtype: bson::spec::BinarySubtype::Generic, bytes: reference},
            "event_ref_context_version": event_record::version(&f.state.encryption_keys).map(i32::from),
            "event_ref_expires_at": bson::DateTime::from_chrono(Utc::now() + ChronoDuration::hours(1)),
        }},
    ).await.unwrap();
    let egress = crate::handlers::assistant_nyxagent::http_egress(
        f.state.clone(),
        crate::test_utils::test_auth_user(OWNER),
    );
    let task = tokio_util::task::AbortOnDropHandle::new(egress.checking_notice(&row).unwrap());
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    assert!(f.gateway.received_requests().await.unwrap().is_empty());
    tokio::time::timeout(std::time::Duration::from_secs(8), task)
        .await
        .unwrap()
        .unwrap();
    assert!(began.elapsed() >= std::time::Duration::from_millis(4900));
    let progress = platform.received_requests().await.unwrap();
    assert_eq!(progress.len(), 1);
    assert_eq!(
        serde_json::from_slice::<Value>(&progress[0].body).unwrap()["text"],
        notice::TEXT
    );
    assert_eq!(status(&f).await, "waiting");
    assert!(!notice::claim(&f.state.db, &row, Utc::now()).await.unwrap());
    assert!(
        Box::pin(engine::finish_turn(
            &f.state.db,
            &row,
            &row.credential_api_key_id,
            &Uuid::new_v4().to_string(),
            &engine::TurnResult {
                text: "Useful answer".into(),
                session_id: None,
                response_id: None,
                error: None,
            },
        ))
        .await
        .unwrap()
        .is_none()
    );
    delivery::detach(&f.state.db, &f.event.id).await.unwrap();
    late_delivery::sweep(&f.state).await.unwrap();
    late_delivery::sweep(&f.state).await.unwrap();
    assert_eq!(sends(&f).await.len(), 1);
    assert_eq!(status(&f).await, "sent");
    let messages = engine::messages(&f.state.db, OWNER, &row.id, 20, None)
        .await
        .unwrap();
    assert!(
        !messages
            .iter()
            .any(|message| message.text.contains(notice::TEXT))
    );
}

#[tokio::test]
async fn guest_1995_route_refusal_is_safe_chat_text_for_stream_and_late_delivery() {
    const SETUP: &str =
        "Visitor setup could not finish. The owner has been notified. Please try again later.";
    const RETRY: &str = "This assistant can't answer right now. Please try again later.";
    for reason in [
        None,
        Some("connection_not_platform"),
        Some("model_connection_unavailable"),
        Some("no_admission"),
        Some("unknown_reason"),
    ] {
        let expected = if reason == Some("model_connection_unavailable") {
            SETUP
        } else {
            RETRY
        };
        for streamed in [true, false] {
            let f = fixture("guest_1995_channel_refusal").await;
            settled(&f, false).await;
            let metadata = crate::models::assistant_message::TurnMetadata {
                continued_after_restart: false,
                voice_link_publication: None,
                links_in_chat: None,
                artifact_recovery_target: None,
                artifact_delivery: None,
                failure_reason: reason.map(|reason| {
                    Box::new(crate::models::assistant_message::TurnFailureReason {
                        guest_setup_step: None,
                        source: "nyxid".into(),
                        stage: "model_proxy".into(),
                        code: "guest_model_route_required".into(),
                        http_status: Some(403),
                        nyxid_error_code: Some(9000),
                        model_route: Some(Box::new(
                            crate::models::assistant_conversation::ModelRouteDiagnostic {
                                reason: reason.into(),
                                unknown_fields: vec![],
                                attempt_id: Some("current".into()),
                                at: Utc::now(),
                            },
                        )),
                    })
                }),
                ..Default::default()
            };
            f.state.db.collection::<AssistantMessage>(crate::models::assistant_message::COLLECTION_NAME)
                .update_one(doc!{"conversation_id":f.event.conversation_id.as_deref(),"turn_id":f.event.turn_id.as_deref(),"role":"assistant"},
                    doc!{"$set":{"text":"","status":"failed","error_code":"guest_model_route_required", "turn_metadata":bson::to_bson(&metadata).unwrap()}}).await.unwrap();
            if streamed {
                let (sender, receiver) = broadcast::channel(8);
                sender.send(json!({"event":"turn.completed","status":"failed","error":{"code":"guest_model_route_required"},"turn_metadata":metadata})).unwrap();
                let response = late_delivery::provider_stream(
                    f.state.clone(),
                    f.event.id.clone(),
                    "response".into(),
                    json!({"type":"response.created"}),
                    InboundTurn {
                        receiver,
                        hosted_reply: None,
                    },
                    Utc::now(),
                );
                let text = body_text(response).await;
                assert!(text.contains(expected), "{reason:?}: {text}");
                assert!(!text.contains("response.failed"));
                assert_eq!(status(&f).await, "streamed");
            } else {
                delivery::detach(&f.state.db, &f.event.id).await.unwrap();
                late_delivery::sweep(&f.state).await.unwrap();
                let requests = sends(&f).await;
                assert_eq!(requests.len(), 1);
                let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
                assert_eq!(body["text"], expected);
            }
            f.state.db.drop().await.unwrap();
        }
    }
}

async fn typing_fixture(
    name: &str,
    contextual: bool,
) -> (
    Fixture,
    crate::models::assistant_conversation::AssistantConversation,
) {
    let mut f = Box::pin(fixture_with_record_writes(name, contextual)).await;
    let request: engine::TurnRequest =
        serde_json::from_value(json!({"text":"Check", "conversation_id":&f.event.conversation_id}))
            .unwrap();
    let mut row = Box::pin(engine::begin_turn(
        &f.state.db,
        OWNER,
        &request,
        &f.state.encryption_keys,
    ))
    .await
    .unwrap();
    let turn = row.active_turn.as_mut().unwrap();
    turn.origin = TurnOrigin::Channel;
    turn.asked_from = Some(f.event.delivery.as_ref().unwrap().origin.clone());
    turn.channel_event_id = Some(f.event.id.clone());
    f.event.turn_id = Some(turn.turn_id.clone());
    f.state
        .db
        .collection::<bson::Document>(CONVERSATIONS)
        .update_one(
            doc! {"_id":&row.id},
            doc! {"$set":{"active_turn":bson::to_bson(turn).unwrap()}},
        )
        .await
        .unwrap();
    set_event_context(
        &f.state.encryption_keys,
        &mut f.event,
        br#"{"event_ref":"cmaeg1.ev.original","capabilities":{"typing":true}}"#,
    )
    .await;
    patch(&f, doc! {
        "turn_id": &f.event.turn_id,
        "event_context_ciphertext": bson::Binary { subtype: bson::spec::BinarySubtype::Generic, bytes: f.event.event_context_ciphertext.clone().unwrap() },
        "event_context_version": f.event.event_context_version.map(i32::from),
    }).await;
    (f, row)
}

#[tokio::test]
async fn channel_typing_refreshes_without_messages_and_stops_with_turn() {
    // The unwrap budget uses the v2_current counter for both stored fields.
    let (f, row) = typing_fixture("channel_typing_lifecycle", false).await;
    Mock::given(method("POST"))
        .and(path("/events/cmaeg1.ev.original/typing"))
        .and(wiremock::matchers::header(
            "content-type",
            "application/json",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"outcome":"sent"})))
        .expect(3)
        .mount(&f.gateway)
        .await;
    let before = f
        .state
        .db
        .collection::<bson::Document>(crate::models::assistant_message::COLLECTION_NAME)
        .count_documents(doc! {})
        .await
        .unwrap();
    let decrypts_before = f.state.encryption_keys.decrypt_stats().v2_current;
    let task = tokio_util::task::AbortOnDropHandle::new(typing::start(&f.state, &row).unwrap());
    tokio::time::timeout(Duration::from_secs(11), async {
        while f.gateway.received_requests().await.unwrap().len() < 3 {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .unwrap();
    // Exactly one context unwrap plus one agent credential unwrap across
    // three platform ticks. No repeated credential resolution/KMS work.
    assert_eq!(
        f.state.encryption_keys.decrypt_stats().v2_current - decrypts_before,
        2
    );
    drop(task);
    let count = f.gateway.received_requests().await.unwrap().len();
    tokio::time::sleep(Duration::from_millis(4300)).await;
    assert_eq!(f.gateway.received_requests().await.unwrap().len(), count);
    assert_eq!(
        f.state
            .db
            .collection::<bson::Document>(crate::models::assistant_message::COLLECTION_NAME)
            .count_documents(doc! {})
            .await
            .unwrap(),
        before
    );
    assert_eq!(status(&f).await, "waiting");
}

async fn pending_recovery(
    f: &mut Fixture,
) -> crate::models::nyxbot_gateway_response::GatewayResponse {
    use crate::{
        models::nyxbot_gateway_response::{COLLECTION_NAME as RESPONSES, GatewayResponse},
        services::nyxbot_response_recovery as recovery,
    };
    f.channel.binding_id = Some("recovery-binding".into());
    f.state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .replace_one(doc! {"_id":&f.channel.id}, &f.channel)
        .await
        .unwrap();
    let payload = recovery::Payload::new("recovery-race", "nyxbot-test", Default::default());
    let stored = GatewayResponse {
        id: Uuid::new_v4().to_string(),
        response_id: "recovery-race".into(),
        user_id: OWNER.into(),
        channel_id: f.channel.id.clone(),
        binding_id: "recovery-binding".into(),
        partition: PARTITION.into(),
        event_key: f.event.id.clone(),
        request_hash: "test".into(),
        conversation_id: f.event.conversation_id.clone(),
        terminal: false,
        answer_delivery: None,
        revision: 0,
        payload_context_version: None,
        payload_ciphertext: payload.seal_legacy(&f.state.encryption_keys).await.unwrap(),
        created_at: Utc::now(),
        expires_at: Utc::now() + ChronoDuration::hours(1),
    };
    f.state
        .db
        .collection::<GatewayResponse>(RESPONSES)
        .insert_one(&stored)
        .await
        .unwrap();
    stored
}

#[tokio::test]
async fn relay_recovery_explains_guest_refusal_and_uncertain_writes() {
    for (code, count, partial, expected) in [
        ("guest_model_route_required", Some(0_i64), "", "No tool ran"),
        (
            "assistant_start_unavailable",
            Some(0),
            "",
            "Nothing was done",
        ),
        ("guest_model_route_required", None, "", "Check its result"),
        (
            "guest_model_route_required",
            Some(1),
            "",
            "Check its result",
        ),
        ("outcome_unknown", Some(0), "", "Check its result"),
        (
            "idle_timeout",
            Some(1),
            "Receipt FG-test",
            "Check its result",
        ),
    ] {
        let mut f = fixture("basket_failure_recovery").await;
        let stored = pending_recovery(&mut f).await;
        patch(&f, doc! {"relay_message_id":"test-inbound"}).await;
        settled(&f, true).await;
        f.state
            .db
            .collection::<bson::Document>(crate::models::assistant_message::COLLECTION_NAME)
            .update_one(
                doc! {"turn_id": &f.event.turn_id},
                doc! {"$set":{"error_code":code, "text":partial, "activity_count":count}},
            )
            .await
            .unwrap();
        let response = Box::pin(native_responses::snapshot(
            State(f.state.clone()),
            Path(stored.response_id.clone()),
            bearer(&f.key),
        ))
        .await;
        assert_eq!(response.status(), StatusCode::OK);
        let snapshot: Value = serde_json::from_str(&body_text(response).await).unwrap();
        assert_eq!(snapshot["status"], "completed", "{code}");
        let output = snapshot["output"][0]["content"][0]["text"]
            .as_str()
            .unwrap();
        assert!(output.contains(expected), "{code}: {output}");
        assert!(output.starts_with(partial));
        late_delivery::process(&f.state, &f.event.id).await;
        assert!(sends(&f).await.is_empty());
        f.state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn relay_recovery_and_late_sender_share_one_irreversible_claim() {
    use crate::services::nyxbot_response_recovery as recovery;
    let mut f = fixture("relay_recovery_delivery_race").await;
    let stored = pending_recovery(&mut f).await;
    for _ in 0..8 {
        patch(
            &f,
            doc! {"delivery.state":"pending","gateway_replay_claimed":false},
        )
        .await;
        let (replay, send) = tokio::join!(
            relay_responses::claim_replay(&f.state, &f.channel, &stored.response_id),
            delivery::claim_send(&f.state.db, &f.event.id, "sender")
        );
        replay.unwrap();
        let state = status(&f).await;
        assert_eq!(send.unwrap(), state == "sending");
        assert!(matches!(state.as_str(), "replay" | "sending"));
        assert!(
            !delivery::claim_send(&f.state.db, &f.event.id, "duplicate")
                .await
                .unwrap()
        );
    }
    // An uncertain outbound send can never be reclaimed by a recovery reader.
    patch(&f, doc! {"delivery.state":"unknown"}).await;
    relay_responses::claim_replay(&f.state, &f.channel, &stored.response_id)
        .await
        .unwrap();
    assert_eq!(status(&f).await, "unknown");
    let mut body = recovery::payload(&f.state.encryption_keys, &stored)
        .await
        .unwrap();
    body.finish(Some("already possibly delivered"), false);
    relay_responses::suppress_output(&mut body);
    assert_eq!(body.snapshot["output"], json!([]));
    assert!(
        !body
            .frames
            .iter()
            .any(|f| f["type"] == "response.output_item.done")
    );
    for (n, frame) in body.frames.iter().enumerate() {
        assert_eq!(frame["sequence_number"], n);
    }
}

#[tokio::test]
async fn channel_typing_stops_on_first_platform_error_or_live_refusal() {
    for cause in ["platform", "stop", "event", "channel"] {
        let (f, row) = typing_fixture("typing_refusal", true).await;
        Mock::given(method("POST"))
            .and(path("/events/cmaeg1.ev.original/typing"))
            .respond_with(if cause == "platform" {
                ResponseTemplate::new(503)
            } else {
                ResponseTemplate::new(200).set_body_json(json!({"outcome":"sent"}))
            })
            .expect(1)
            .mount(&f.gateway)
            .await;
        let task = typing::start(&f.state, &row).unwrap();
        tokio::time::timeout(Duration::from_secs(4), async {
            while f.gateway.received_requests().await.unwrap().is_empty() {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        })
        .await
        .unwrap();
        match cause {
            "stop" => {
                f.state
                    .db
                    .collection::<bson::Document>(CONVERSATIONS)
                    .update_one(
                        doc! {"_id":&row.id},
                        doc! {"$set":{"active_turn.stop_requested":true}},
                    )
                    .await
                    .unwrap();
            }
            "event" => {
                patch(&f, doc! {"turn_id":"different-turn"}).await;
            }
            "channel" => {
                f.state
                    .db
                    .collection::<NyxbotChannel>(CHANNELS)
                    .update_one(
                        doc! {"_id":&f.channel.id},
                        doc! {"$set":{"status":"disconnected"}},
                    )
                    .await
                    .unwrap();
            }
            _ => {}
        }
        tokio::time::timeout(Duration::from_secs(5), task)
            .await
            .unwrap()
            .unwrap();
        f.gateway.verify().await;
        assert_eq!(f.gateway.received_requests().await.unwrap().len(), 1);
    }
}

#[tokio::test]
async fn telegram_partial_failure_notifies_saved_thread_once_without_restarting_execution() {
    let f = fixture("telegram_failure_notice").await;
    let inbound = Uuid::new_v4().to_string();
    f.state
        .db
        .collection::<bson::Document>(EVENTS)
        .update_one(
            doc! {"_id":&f.event.id},
            doc! {"$set":{"relay_message_id":&inbound}},
        )
        .await
        .unwrap();
    let receipt = Uuid::new_v4().to_string();
    let receipts = f
        .state
        .db
        .collection::<bson::Document>(crate::models::channel_reply_delivery::COLLECTION_NAME);
    receipts.insert_one(doc!{"_id":&receipt,"complete":false,"parts":[{"status":"accepted"},{"status":"failed"},{"status":"not_attempted"}]}).await.unwrap();
    for _ in 0..2 {
        crate::services::channel_reply_notice::failed(
            &f.state.db,
            &receipt,
            &f.channel.channel_bot_id,
            &inbound,
        )
        .await
        .unwrap();
    }
    let messages = f
        .state
        .db
        .collection::<AssistantMessage>(crate::models::assistant_message::COLLECTION_NAME);
    assert_eq!(
        messages
            .count_documents(doc! {"conversation_id":&f.event.conversation_id,"role":"event"})
            .await
            .unwrap(),
        1
    );
    let notice = messages
        .find_one(doc! {"conversation_id":&f.event.conversation_id,"role":"event"})
        .await
        .unwrap()
        .unwrap();
    assert!(notice.text.contains("1 of 3 parts were confirmed"));
    assert!(notice.text.contains("Later parts were not sent"));
    assert!(!notice.text.contains("can't parse entities"));
    // A foreign bot cannot choose this owner's thread through an inbound ID.
    let foreign = Uuid::new_v4().to_string();
    receipts
        .insert_one(doc! {"_id":&foreign,"complete":false,"parts":[{"status":"uncertain"}]})
        .await
        .unwrap();
    crate::services::channel_reply_notice::failed(&f.state.db, &foreign, "foreign-bot", &inbound)
        .await
        .unwrap();
    assert_eq!(
        messages
            .count_documents(doc! {"conversation_id":&f.event.conversation_id,"role":"event"})
            .await
            .unwrap(),
        1
    );
    // Reporting a transport failure leaves the send/stream barrier intact.
    assert_eq!(status(&f).await, "waiting");
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn model_approval_channel_stream_delivers_actionable_settings_url() {
    let f = fixture("model_approval_channel_stream").await;
    settled(&f, true).await;
    let message = crate::services::assistant_model_observer::approval_message(
        "model_approval_required",
        &crate::services::assistant_links::AssistantPage::ApprovalSettings
            .url(&f.state.config.frontend_url),
    );
    f.state
        .db
        .collection::<bson::Document>(crate::models::assistant_message::COLLECTION_NAME)
        .update_one(
            doc! {"conversation_id":&f.event.conversation_id,"role":"assistant"},
            doc! {"$set":{"text":&message,"error_code":"model_approval_required"}},
        )
        .await
        .unwrap();
    let (sender, receiver) = broadcast::channel(8);
    sender
        .send(json!({"event":"turn.completed","status":"failed",
        "error":{"code":"model_approval_required","message":message}}))
        .unwrap();
    let response = late_delivery::provider_stream(
        f.state.clone(),
        f.event.id.clone(),
        "response".into(),
        json!({"type":"response.created"}),
        InboundTurn {
            receiver,
            hosted_reply: None,
        },
        Utc::now(),
    );
    let body = body_text(response).await;
    assert!(body.contains("approval settings require approval for this model request"));
    assert!(body.contains("/approvals/settings"));
    assert!(!body.contains("response.failed"));
}

#[tokio::test]
async fn model_approval_channel_partial_reply_keeps_guidance() {
    let (sender, receiver) = broadcast::channel(8);
    let message = crate::services::assistant_model_observer::approval_message(
        "model_approval_required",
        "https://example.test/approvals/settings",
    );
    sender
        .send(json!({"event":"block.completed","block":{"text":"Partial answer"}}))
        .unwrap();
    sender
        .send(json!({"event":"turn.completed","status":"failed",
        "error":{"code":"model_approval_required","message":message}}))
        .unwrap();
    let text = final_reply(receiver).await.unwrap();
    assert!(text.contains("Partial answer"));
    assert!(text.contains("Incomplete"));
    assert!(text.contains("approval settings require approval for this model request"));
    assert!(text.contains("https://example.test/approvals/settings"));
}

// The preparation seam drops the worker after credentials/request preparation,
// before any provider effect. No task is left running after the simulated death.
#[tokio::test]
async fn late_gateway_crash_during_preparation_survivor_sends_once() {
    let f = fixture("late_gateway_preparation").await;
    settled(&f, false).await;
    delivery::detach(&f.state.db, &f.event.id).await.unwrap();
    crate::services::durable_wake_tests::drop_after(
        "late_text_prepared",
        late_delivery::process(&f.state, &f.event.id),
    )
    .await;
    assert!(sends(&f).await.is_empty());
    patch(
        &f,
        doc! {"delivery.preparation.lease_until": bson::DateTime::from_millis(0)},
    )
    .await;
    late_delivery::sweep(&f.state).await.unwrap();
    late_delivery::sweep(&f.state).await.unwrap();
    assert_eq!(
        sends(&f).await.len(),
        1,
        "survivor must recover proven-unsent preparation"
    );
    assert_eq!(status(&f).await, "sent");
}

#[tokio::test]
async fn late_direct_crash_during_preparation_survivor_sends_once() {
    let (f, server) = image_delivery_tests::image_fixture("telegram", "direct").await;
    Mock::given(path("/bottoken/sendMessage"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"ok":true,"result":{"message_id":43}})),
        )
        .mount(&server)
        .await;
    delivery::detach(&f.state.db, &f.event.id).await.unwrap();
    crate::services::durable_wake_tests::drop_after(
        "late_text_prepared",
        late_delivery::process(&f.state, &f.event.id),
    )
    .await;
    patch(
        &f,
        doc! {"delivery.preparation.lease_until": bson::DateTime::from_millis(0)},
    )
    .await;
    late_delivery::sweep(&f.state).await.unwrap();
    late_delivery::sweep(&f.state).await.unwrap();
    let sends = server
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .filter(|r| r.url.path().ends_with("/sendMessage"))
        .count();
    assert_eq!(sends, 1, "survivor must recover proven-unsent preparation");
    assert_eq!(status(&f).await, "sent");
}

#[tokio::test]
async fn late_abandoned_sending_becomes_one_visible_unconfirmed_notice() {
    let f = fixture("late_abandoned_notice").await;
    settled(&f, false).await;
    patch(
        &f,
        doc! {"delivery.state":"sending", "delivery.claim_id":"lost",
        "delivery.checked_at":bson::DateTime::from_millis(0)},
    )
    .await;
    late_delivery::sweep(&f.state).await.unwrap();
    late_delivery::sweep(&f.state).await.unwrap();
    assert!(sends(&f).await.is_empty());
    assert_eq!(status(&f).await, "unknown");
    let notices = f
        .state
        .db
        .collection::<bson::Document>(crate::models::assistant_message::COLLECTION_NAME)
        .count_documents(doc! {"conversation_id":&f.event.conversation_id,"role":"event"})
        .await
        .unwrap();
    assert_eq!(notices, 1);
}

#[tokio::test]
async fn late_normal_send_command_counts() {
    for direct in [false, true] {
        let (mut f, _server) = if direct {
            let (f, server) = image_delivery_tests::image_fixture("telegram", "direct").await;
            Mock::given(path("/bottoken/sendMessage"))
                .respond_with(
                    ResponseTemplate::new(200)
                        .set_body_json(json!({"ok":true,"result":{"message_id":43}})),
                )
                .mount(&server)
                .await;
            (f, Some(server))
        } else {
            let f = fixture("late_command_count").await;
            settled(&f, false).await;
            (f, None)
        };
        delivery::detach(&f.state.db, &f.event.id).await.unwrap();
        let (db, commands) = crate::services::durable_wake_tests::monitored(&f.state.db).await;
        f.state.db = db;
        late_delivery::process(&f.state, &f.event.id).await;
        let counts = crate::services::durable_wake_tests::take(&commands);
        println!(
            "late normal direct={direct}: total={} {counts:?}",
            counts.values().sum::<usize>()
        );
        assert_eq!(status(&f).await, "sent");
    }
}

#[tokio::test]
async fn late_legacy_sending_without_a_date_is_not_abandoned_or_selected_by_the_sweep() {
    let f = fixture("late_legacy_timestamp").await;
    settled(&f, false).await;
    patch(
        &f,
        doc! {"delivery.state":"sending", "delivery.claim_id":"legacy"},
    )
    .await;
    let events = f.state.db.collection::<bson::Document>(EVENTS);
    for value in [
        None,
        Some(bson::Bson::Null),
        Some(bson::Bson::Int64(0)),
        Some(bson::Bson::String("invalid".into())),
    ] {
        let update = match value {
            Some(value) => doc! {"$set":{"delivery.checked_at":value}},
            None => doc! {"$unset":{"delivery.checked_at":""}},
        };
        events
            .update_one(doc! {"_id":&f.event.id}, update)
            .await
            .unwrap();
        assert_eq!(
            events
                .count_documents(crate::services::channel_turn_preparation::due())
                .await
                .unwrap(),
            0
        );
        late_delivery::sweep(&f.state).await.unwrap();
        let row = events
            .find_one(doc! {"_id":&f.event.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            row.get_document("delivery")
                .unwrap()
                .get_str("state")
                .unwrap(),
            "sending"
        );
    }
    patch(
        &f,
        doc! {"delivery.checked_at":bson::DateTime::from_millis(0)},
    )
    .await;
    late_delivery::sweep(&f.state).await.unwrap();
    assert_eq!(status(&f).await, "unknown");
}

async fn expire_text_preparation(f: &Fixture) {
    patch(
        f,
        doc! {"delivery.preparation.lease_until":bson::DateTime::from_millis(0)},
    )
    .await;
}

#[tokio::test]
async fn late_slow_preparer_loses_claim_before_any_provider_send() {
    let f = fixture("late_slow_preparer").await;
    settled(&f, false).await;
    delivery::detach(&f.state.db, &f.event.id).await.unwrap();
    crate::services::durable_wake_tests::resume_after(
        "late_text_prepared",
        late_delivery::process(&f.state, &f.event.id),
        async {
            expire_text_preparation(&f).await;
            let state = f.state.clone();
            let event = f.event.id.clone();
            // A separate task models another replica and does not inherit the
            // deliberately suspended worker's task-local crash injection.
            state
                .shutdown
                .clone()
                .spawn(async move {
                    late_delivery::process(&state, &event).await;
                })
                .await
                .unwrap();
        },
    )
    .await;
    assert_eq!(sends(&f).await.len(), 1);
    assert_eq!(status(&f).await, "sent");
}

#[tokio::test]
async fn late_two_survivors_recover_one_preparation_and_old_claim_cannot_win() {
    let f = fixture("late_two_survivors").await;
    settled(&f, false).await;
    delivery::detach(&f.state.db, &f.event.id).await.unwrap();
    crate::services::durable_wake_tests::drop_after(
        "late_text_prepared",
        late_delivery::process(&f.state, &f.event.id),
    )
    .await;
    assert!(
        !delivery::claim_send(&f.state.db, &f.event.id, "old-worker")
            .await
            .unwrap()
    );
    assert!(
        !delivery::claim_stream(&f.state.db, &f.event.id, "completed")
            .await
            .unwrap()
    );
    expire_text_preparation(&f).await;
    let (a, b) = tokio::join!(
        late_delivery::sweep(&f.state),
        late_delivery::sweep(&f.state)
    );
    a.unwrap();
    b.unwrap();
    assert_eq!(sends(&f).await.len(), 1);
    assert_eq!(status(&f).await, "sent");
}

#[tokio::test]
async fn late_gateway_acceptance_without_receipt_is_terminal_and_visible() {
    let f = fixture("late_acceptance_without_receipt").await;
    settled(&f, false).await;
    delivery::detach(&f.state.db, &f.event.id).await.unwrap();
    crate::services::durable_wake_tests::drop_after(
        "late_text_accepted_before_receipt",
        late_delivery::process(&f.state, &f.event.id),
    )
    .await;
    assert_eq!(sends(&f).await.len(), 1);
    // A pre-#2561 sender did not freeze the reply request.
    patch(
        &f,
        doc! {"delivery.preparation.gateway_request_hash":bson::Bson::Null},
    )
    .await;
    expire_text_preparation(&f).await;
    let (a, b) = tokio::join!(
        late_delivery::sweep(&f.state),
        late_delivery::sweep(&f.state)
    );
    a.unwrap();
    b.unwrap();
    assert_eq!(sends(&f).await.len(), 1);
    assert_eq!(status(&f).await, "unknown");
    let notices = f
        .state
        .db
        .collection::<AssistantMessage>(crate::models::assistant_message::COLLECTION_NAME)
        .find(doc! {"conversation_id":&f.event.conversation_id,"role":"event"})
        .await
        .unwrap()
        .try_collect::<Vec<_>>()
        .await
        .unwrap();
    assert_eq!(notices.len(), 1);
    assert!(notices[0].text.starts_with("Delivery unconfirmed."));
}

#[tokio::test]
async fn late_direct_accepted_prefix_is_not_repeated_after_worker_death() {
    let (f, server) = image_delivery_tests::image_fixture("telegram", "direct").await;
    Mock::given(path("/bottoken/sendMessage"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"ok":true,"result":{"message_id":43}})),
        )
        .mount(&server)
        .await;
    f.state
        .db
        .collection::<bson::Document>(crate::models::assistant_message::COLLECTION_NAME)
        .update_one(
            doc! {"conversation_id":&f.event.conversation_id,"role":"assistant"},
            doc! {"$set":{"text":format!("{} tail", "A".repeat(6000))}},
        )
        .await
        .unwrap();
    delivery::detach(&f.state.db, &f.event.id).await.unwrap();
    crate::services::durable_wake_tests::drop_after(
        "late_text_receipt_committed",
        late_delivery::process(&f.state, &f.event.id),
    )
    .await;
    expire_text_preparation(&f).await;
    late_delivery::sweep(&f.state).await.unwrap();
    late_delivery::sweep(&f.state).await.unwrap();
    let sends: Vec<_> = server
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .filter(|r| r.url.path().ends_with("/sendMessage"))
        .collect();
    assert_eq!(sends.len(), 2);
    let first: Value = serde_json::from_slice(&sends[0].body).unwrap();
    let second: Value = serde_json::from_slice(&sends[1].body).unwrap();
    assert_ne!(first["text"], second["text"]);
    assert_eq!(status(&f).await, "sent");
}

#[tokio::test]
async fn late_recovered_preparation_rechecks_revoked_authority() {
    let f = fixture("late_revoked_preparation").await;
    settled(&f, false).await;
    delivery::detach(&f.state.db, &f.event.id).await.unwrap();
    crate::services::durable_wake_tests::drop_after(
        "late_text_prepared",
        late_delivery::process(&f.state, &f.event.id),
    )
    .await;
    expire_text_preparation(&f).await;
    f.state
        .db
        .collection::<bson::Document>(CHANNELS)
        .update_one(
            doc! {"_id":&f.channel.id},
            doc! {"$set":{"status":"revoked"}},
        )
        .await
        .unwrap();
    late_delivery::sweep(&f.state).await.unwrap();
    assert!(sends(&f).await.is_empty());
    assert_eq!(status(&f).await, "refused");
}

#[tokio::test]
async fn late_preparation_lease_uses_database_time_and_no_content_is_copied() {
    let f = fixture("late_database_clock").await;
    settled(&f, false).await;
    delivery::detach(&f.state.db, &f.event.id).await.unwrap();
    patch(&f,doc! {"delivery.checked_at":bson::DateTime::from_chrono(Utc::now()+ChronoDuration::days(7))}).await;
    crate::services::durable_wake_tests::drop_after(
        "late_text_prepared",
        late_delivery::process(&f.state, &f.event.id),
    )
    .await;
    let row = f
        .state
        .db
        .collection::<bson::Document>(EVENTS)
        .find_one(doc! {"_id":&f.event.id})
        .await
        .unwrap()
        .unwrap();
    let d = row.get_document("delivery").unwrap();
    let p = d.get_document("preparation").unwrap();
    assert_eq!(
        p.get_datetime("lease_until").unwrap().timestamp_millis()
            - d.get_datetime("checked_at").unwrap().timestamp_millis(),
        crate::services::channel_turn_preparation::PREPARATION_SECS * 1000
    );
    let metadata = serde_json::to_string(p).unwrap();
    assert!(!metadata.contains("Useful answer"));
    late_delivery::sweep(&f.state).await.unwrap();
    assert!(sends(&f).await.is_empty());
}

#[tokio::test]
async fn late_guest_uncertainty_does_not_create_owner_transcript_readback() {
    let f = fixture("late_guest_unconfirmed").await;
    settled(&f, false).await;
    patch(&f,doc! {"delivery.guest":true,"delivery.state":"sending","delivery.checked_at":bson::DateTime::from_millis(0)}).await;
    late_delivery::sweep(&f.state).await.unwrap();
    assert_eq!(status(&f).await, "unknown");
    assert_eq!(
        f.state
            .db
            .collection::<bson::Document>(crate::models::assistant_message::COLLECTION_NAME)
            .count_documents(doc! {"conversation_id":&f.event.conversation_id,"role":"event"})
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn late_dispatch_marker_without_receipt_never_reopens_preparation() {
    let f = fixture("late_dispatch_marker").await;
    settled(&f, false).await;
    delivery::detach(&f.state.db, &f.event.id).await.unwrap();
    crate::services::durable_wake_tests::drop_after(
        "late_text_dispatch_committed",
        late_delivery::process(&f.state, &f.event.id),
    )
    .await;
    assert_eq!(status(&f).await, "sending");
    assert!(sends(&f).await.is_empty());
    // A pre-#2561 sender did not freeze the reply request.
    patch(
        &f,
        doc! {"delivery.preparation.gateway_request_hash":bson::Bson::Null},
    )
    .await;
    expire_text_preparation(&f).await;
    late_delivery::sweep(&f.state).await.unwrap();
    late_delivery::sweep(&f.state).await.unwrap();
    assert_eq!(status(&f).await, "unknown");
    assert!(
        sends(&f).await.is_empty(),
        "a missing receipt never proves no send"
    );
}

#[tokio::test]
async fn late_unscoped_direct_reply_uses_only_ordinary_delivery_records() {
    let (mut f, server) = image_delivery_tests::image_fixture("telegram", "direct").await;
    Mock::given(path("/bottoken/sendMessage"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"ok":true,"result":{"message_id":43}})),
        )
        .mount(&server)
        .await;
    let (db, commands) = crate::services::durable_wake_tests::monitored(&f.state.db).await;
    f.state.db = db;
    direct_reply(
        &f.state,
        &f.channel,
        &f.event.event_id,
        "Ordinary reply",
        None,
    )
    .await
    .unwrap();
    let counts = crate::services::durable_wake_tests::take(&commands);
    assert!(!counts.keys().any(|key| key.ends_with(":nyxbot_events")));
    assert_eq!(status(&f).await, "waiting");
}

#[path = "nyxbot_gateway_reply_tests.rs"]
mod gateway_reply_tests;

#[path = "nyxbot_response_read_delivery_tests.rs"]
mod response_read_delivery_tests;

#[path = "nyxbot_channel_capacity_tests.rs"]
mod channel_capacity_tests;

#[tokio::test]
async fn event_record_context_target_and_guest_setup_bind_source_owner_channel_and_purpose() {
    use crate::services::nyxbot_event_record as record;
    let f = fixture("event_record_swaps").await;
    let keys = &f.state.encryption_keys;
    let mut event = f.event.clone();
    event.event_context_version = Some(3);
    let mut delivery = event.delivery.clone().unwrap();
    delivery.target_context_version = Some(3);
    for path in [
        "event_context_ciphertext",
        "delivery.target_ciphertext",
        "guest_setup_payload",
    ] {
        let sealed = bson::Binary {
            subtype: bson::spec::BinarySubtype::Generic,
            bytes: record::seal(
                keys,
                &event.id,
                &event.user_id,
                &[path, &event.channel_id],
                b"fixture bytes",
            )
            .await
            .unwrap(),
        };
        for (identity, value) in [
            ("_id", "swapped"),
            ("user_id", "swapped"),
            ("channel_id", "swapped"),
            ("_id", event.id.as_str()),
        ] {
            let original = value == event.id;
            let mut source = bson::to_document(&event).unwrap();
            source.insert(identity, value);
            source.insert("guest_setup_context_version", 3);
            source.insert("guest_setup_payload", sealed.clone());
            let row = bson::from_document(source.clone()).unwrap();
            for (reader, result) in [
                (
                    "event_context_ciphertext",
                    record::event_context(keys, &row, &sealed.bytes).await,
                ),
                (
                    "delivery.target_ciphertext",
                    record::delivery_target(keys, &row, &delivery, &sealed.bytes).await,
                ),
                (
                    "guest_setup_payload",
                    record::guest_setup(keys, &source).await,
                ),
            ] {
                if original && path == reader {
                    assert_eq!(result.unwrap().as_slice(), b"fixture bytes");
                } else {
                    assert!(
                        result.is_err(),
                        "{reader} accepted {path} after {identity} substitution"
                    );
                }
            }
        }
    }
    let mut compatible = crate::test_utils::test_encryption_keys();
    compatible.set_contextual_record_writes(false);
    let legacy = record::seal(
        &compatible,
        &event.id,
        &event.user_id,
        &[record::GUEST, &event.channel_id],
        b"fixture bytes",
    )
    .await
    .unwrap();
    let mut raw = bson::to_document(&event).unwrap();
    raw.insert(
        "guest_setup_payload",
        bson::Binary {
            subtype: bson::spec::BinarySubtype::Generic,
            bytes: legacy,
        },
    );
    for invalid in [bson::Bson::Int32(8), bson::Bson::String("3".into())] {
        raw.insert("guest_setup_context_version", invalid);
        assert!(record::guest_setup(keys, &raw).await.is_err());
    }
}

use super::*;
use crate::models::{assistant_message::AssistantMessage, nyxbot_channel::ChannelTurnDelivery};
use crate::services::channel_turn_delivery as delivery;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

struct Fixture {
    state: AppState,
    channel: NyxbotChannel,
    key: String,
    event: NyxbotEvent,
    gateway: MockServer,
    upstream: tokio::task::JoinHandle<()>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.upstream.abort();
    }
}

async fn fixture(name: &str) -> Fixture {
    let (state, _, upstream) = setup(name).await;
    let (mut channel, key) = channel(&state, "gateway").await;
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
        .insert_one(bot)
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
    let conversation = crate::services::assistant_team_service::home_thread(
        &state.db,
        &state.encryption_keys,
        &agent,
    )
    .await
    .unwrap();
    state.db.collection::<bson::Document>(THREADS).insert_one(doc! {
        "_id": Uuid::new_v4().to_string(), "channel_id": &channel.id, "user_id": OWNER,
        "partition": PARTITION, "conversation_id": &conversation.id, "kind": "private", "owner_chat": true,
        "platform_chat_id": "chat-1", "created_at": bson::DateTime::now(), "updated_at": bson::DateTime::now(),
    }).await.unwrap();
    let now = Utc::now();
    let mut event = admission(&channel.id, "original");
    event.conversation_id = Some(conversation.id.clone());
    event.turn_id = Some(Uuid::new_v4().to_string());
    event.event_context_ciphertext = Some(
        state
            .encryption_keys
            .encrypt(br#"{"event_ref":"cmaeg1.ev.original"}"#)
            .await
            .unwrap(),
    );
    event.delivery = Some(ChannelTurnDelivery {
        version: 1,
        state: "waiting".into(),
        origin: ChannelOrigin {
            nyxbot_channel_id: channel.id.clone(),
            partition: PARTITION.into(),
            platform: "telegram".into(),
            thread: None,
        },
        sender_id: "7".into(),
        guest: false,
        addressed: true,
        transport: "gateway".into(),
        stream_deadline: now + ChronoDuration::seconds(570),
        checked_at: now,
        claim_id: None,
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
async fn long_turn_deadline_notice_then_durable_late_answer_once() {
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
        receiver,
        Utc::now() - ChronoDuration::seconds(late_delivery::STREAM_SECS + 1),
    );
    let text = body_text(response).await;
    assert!(text.contains("still working"));
    assert!(text.contains("post the answer here"));
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
        receiver,
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
    assert!(
        late_delivery::answer(&"界".repeat(9000), true)
            .chars()
            .count()
            <= MAX_CHANNEL_REPLY_CHARS
    );
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
    assert!(event.delivery.unwrap().target_ciphertext.is_some());
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
        .respond_with(ResponseTemplate::new(503))
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
        receiver,
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

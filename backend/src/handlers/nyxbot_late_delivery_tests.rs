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

async fn image_selection(f: &Fixture, indices: &[usize]) -> Vec<String> {
    use crate::services::assistant_reply_images as images;
    f.state.db.collection::<bson::Document>(CONVERSATIONS).update_one(doc! {"_id":&f.event.conversation_id},doc! {"$set":{"active_turn":{
        "turn_id":&f.event.turn_id,"started_at":bson::DateTime::now(),"origin":"channel","stop_requested":false,
    }}}).await.unwrap();
    let mut refs = Vec::new();
    for bytes in [
        b"\x89PNG\r\n\x1a\nFIRST_IMAGE".as_slice(),
        b"\x89PNG\r\n\x1a\nCHOSEN_IMAGE",
        b"\x89PNG\r\n\x1a\nFAIL_IMAGE",
    ] {
        refs.push(
            engine::attach_image(
                &f.state.db,
                &f.state.encryption_keys,
                OWNER,
                f.event.conversation_id.as_deref().unwrap(),
                "camera__snapshot",
                "image/png",
                bytes,
            )
            .await
            .unwrap()
            .unwrap()
            .id,
        );
    }
    let selected: Vec<_> = indices.iter().map(|i| refs[*i].clone()).collect();
    let result = images::select(
        &f.state,
        OWNER,
        f.event.conversation_id.as_deref().unwrap(),
        f.event.turn_id.as_deref(),
        &json!({"image_refs":selected}),
    )
    .await
    .unwrap();
    assert_eq!(result["status"], "selected_not_sent");
    let live = engine::get(
        &f.state.db,
        OWNER,
        f.event.conversation_id.as_deref().unwrap(),
    )
    .await
    .unwrap();
    settled(f, false).await;
    f.state.db.collection::<bson::Document>(crate::models::assistant_message::COLLECTION_NAME).update_one(doc! {"turn_id":&f.event.turn_id},doc! {"$set":{
        "reply_image_refs": &selected,"attachments":bson::to_bson(&live.active_turn.unwrap().attachments).unwrap(),
    }}).await.unwrap();
    f.state
        .db
        .collection::<bson::Document>(CONVERSATIONS)
        .update_one(
            doc! {"_id":&f.event.conversation_id},
            doc! {"$unset":{"active_turn":""}},
        )
        .await
        .unwrap();
    refs
}

async fn relay_image_fixture(f: &mut Fixture) -> crate::models::channel_message::ChannelMessage {
    let route = Uuid::new_v4().to_string();
    f.channel.route_id = Some(route.clone());
    f.state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .replace_one(doc! {"_id":&f.channel.id}, &f.channel)
        .await
        .unwrap();
    let token = f
        .state
        .encryption_keys
        .encrypt(b"test-token")
        .await
        .unwrap();
    f.state.db.collection::<bson::Document>(crate::models::channel_bot::COLLECTION_NAME).update_one(doc! {"_id":&f.channel.channel_bot_id},doc! {"$set":{"bot_token_encrypted":bson::Binary{subtype:bson::spec::BinarySubtype::Generic,bytes:token}}}).await.unwrap();
    f.state.db.collection::<bson::Document>(crate::models::channel_conversation::COLLECTION_NAME).insert_one(doc! {
        "_id":&route,"user_id":OWNER,"channel_bot_id":&f.channel.channel_bot_id,"platform":"telegram","platform_conversation_id":"*","platform_conversation_type":"private","agent_api_key_id":&f.channel.route_api_key_id,"is_active":true,"created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now(),
    }).await.unwrap();
    f.state
        .db
        .collection::<bson::Document>(THREADS)
        .update_one(
            doc! {"channel_id":&f.channel.id},
            doc! {"$set":{"allow_posts":true}},
        )
        .await
        .unwrap();
    let source: crate::models::channel_message::ChannelMessage = bson::from_document(doc! {
        "_id":&f.event.event_id,"user_id":OWNER,"channel_bot_id":&f.channel.channel_bot_id,"conversation_id":route,"platform":"telegram","platform_conversation_id":"chat-1","direction":"inbound","content_type":"text","platform_message_id":"123","agent_api_key_id":&f.channel.route_api_key_id,"created_at":bson::DateTime::now(),
    }).unwrap();
    f.state
        .db
        .collection::<crate::models::channel_message::ChannelMessage>(
            crate::models::channel_message::COLLECTION_NAME,
        )
        .insert_one(&source)
        .await
        .unwrap();
    source
}

#[tokio::test]
async fn gateway_image_selection_lookup_failure_never_emits_plain_success() {
    for failure in [
        "missing_message",
        "invalid_message",
        "invalid_event",
        "missing_event",
    ] {
        let f = fixture(&format!("gateway_selection_{failure}")).await;
        image_selection(&f, &[1]).await;
        let messages = f
            .state
            .db
            .collection::<bson::Document>(crate::models::assistant_message::COLLECTION_NAME);
        match failure {
            "missing_message" => {
                messages
                    .delete_many(doc! {"turn_id":&f.event.turn_id})
                    .await
                    .unwrap();
            }
            "invalid_message" => {
                messages
                    .update_one(
                        doc! {"turn_id":&f.event.turn_id},
                        doc! {"$set":{"reply_image_refs":42}},
                    )
                    .await
                    .unwrap();
            }
            "missing_event" => {
                f.state
                    .db
                    .collection::<bson::Document>(EVENTS)
                    .delete_one(doc! {"_id":&f.event.id})
                    .await
                    .unwrap();
            }
            _ => {
                patch(&f, doc! {"event_id":42}).await;
            }
        }
        let (sender, receiver) = broadcast::channel(8);
        sender
            .send(json!({"event":"block.completed","block":{"text":"Useful answer"}}))
            .unwrap();
        sender
            .send(json!({"event":"turn.completed","status":"completed"}))
            .unwrap();
        drop(sender);
        let response = late_delivery::provider_stream(
            f.state.clone(),
            f.event.id.clone(),
            "response".into(),
            json!({"type":"response.created"}),
            receiver,
            Utc::now(),
        );
        let body = body_text(response).await;
        assert!(
            body.contains("response.failed"),
            "{failure}: selection unavailable must fail honestly: {body}"
        );
        assert!(!body.contains("Useful answer") && !body.contains("response.completed"));
        assert!(!body.contains("attachments") && !body.contains("input_image"));
        assert!(sends(&f).await.is_empty());
    }
}

#[tokio::test]
async fn nyxbot_gateway_async_missing_selection_does_not_send_plain_text() {
    let f = fixture("gateway_async_selection_missing").await;
    image_selection(&f, &[1]).await;
    let encrypted = f
        .state
        .encryption_keys
        .encrypt(b"cmaeg1.ev.original")
        .await
        .unwrap();
    f.state.db.collection::<bson::Document>(THREADS).update_one(doc! {"channel_id":&f.channel.id},
        doc! {"$set":{"event_ref_ciphertext":bson::to_bson(&encrypted).unwrap(),
        "event_ref_expires_at":bson::DateTime::from_chrono(Utc::now()+ChronoDuration::hours(1))}}).await.unwrap();
    f.state
        .db
        .collection::<bson::Document>(crate::models::assistant_message::COLLECTION_NAME)
        .delete_many(doc! {"turn_id":&f.event.turn_id})
        .await
        .unwrap();
    let row = engine::get(
        &f.state.db,
        OWNER,
        f.event.conversation_id.as_deref().unwrap(),
    )
    .await
    .unwrap();
    deliver_to_selected(
        &f.state,
        &row,
        row.channel.as_ref().unwrap(),
        "Useful answer",
        f.event.turn_id.as_deref(),
    )
    .await;
    assert!(f.gateway.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn nyxbot_image_capability_matches_relay_era_gateway_delivery() {
    use crate::services::assistant_reply_images as images;
    let mut f = fixture("capability_relay_era").await;
    let source = relay_image_fixture(&mut f).await;
    f.state
        .db
        .collection::<bson::Document>(THREADS)
        .update_one(
            doc! {"channel_id":&f.channel.id},
            doc! {"$set":{"last_message_id":&source.id}},
        )
        .await
        .unwrap();
    let (client, requests, server) = crate::test_utils::channel_transport::result_transport().await;
    f.state.http_client = client;
    let refs = image_selection(&f, &[1]).await;
    let message = images::settled(
        &f.state,
        OWNER,
        f.event.conversation_id.as_deref().unwrap(),
        f.event.turn_id.as_deref().unwrap(),
    )
    .await
    .unwrap()
    .unwrap();
    f.state.db.collection::<bson::Document>(CONVERSATIONS).update_one(doc! {"_id":&message.conversation_id},
        doc! {"$set":{"active_turn":{"turn_id":&message.turn_id,"started_at":bson::DateTime::now(),
        "origin":"channel","stop_requested":false,"attachments":bson::to_bson(&message.attachments).unwrap()}}}).await.unwrap();
    let selection = json!({"image_refs":[refs[1]]});
    let select = || {
        images::select(
            &f.state,
            OWNER,
            &message.conversation_id,
            Some(&message.turn_id),
            &selection,
        )
    };
    let selected = select().await.unwrap();
    assert_eq!(selected["destination_capability"], "images");
    let row = engine::get(&f.state.db, OWNER, &message.conversation_id)
        .await
        .unwrap();
    deliver_to_selected(
        &f.state,
        &row,
        row.channel.as_ref().unwrap(),
        "Relay-era reply",
        Some(&message.turn_id),
    )
    .await;
    assert_eq!(requests.lock().await.len(), 2);
    assert!(sends(&f).await.is_empty());
    let encrypted = f
        .state
        .encryption_keys
        .encrypt(b"cmaeg1.ev.original")
        .await
        .unwrap();
    f.state.db.collection::<bson::Document>(THREADS).update_one(doc! {"channel_id":&f.channel.id},
        doc! {"$set":{"event_ref_ciphertext":bson::to_bson(&encrypted).unwrap(),
        "event_ref_expires_at":bson::DateTime::from_chrono(Utc::now()+ChronoDuration::hours(1))}}).await.unwrap();
    assert_eq!(
        select().await.unwrap()["destination_capability"],
        "text_with_image_notice"
    );
    f.state
        .db
        .collection::<bson::Document>(THREADS)
        .update_one(
            doc! {"channel_id":&f.channel.id},
            doc! {"$unset":{"event_ref_ciphertext":"", "last_message_id":""}},
        )
        .await
        .unwrap();
    assert_eq!(select().await.unwrap()["destination_capability"], "unknown");
    server.abort();
}

#[tokio::test]
async fn gateway_selected_images_degrade_in_stream_and_late_reply_without_attachment_blocks() {
    for stream in [false, true] {
        let f = fixture(&format!("gateway_images_{stream}")).await;
        image_selection(&f, &[1]).await;
        if stream {
            let (sender, receiver) = broadcast::channel(8);
            sender
                .send(json!({"event":"block.completed","block":{"text":"Useful answer"}}))
                .unwrap();
            sender
                .send(json!({"event":"turn.completed","status":"completed"}))
                .unwrap();
            drop(sender);
            let response = late_delivery::provider_stream(
                f.state.clone(),
                f.event.id.clone(),
                "response".into(),
                json!({"type":"response.created"}),
                receiver,
                Utc::now(),
            );
            let body = body_text(response).await;
            assert!(
                body.contains("selected images could not be shown"),
                "{body}"
            );
            assert!(body.contains("Owner-only NyxID thread"));
            assert!(!body.contains("input_image") && !body.contains("attachments"));
        } else {
            delivery::detach(&f.state.db, &f.event.id).await.unwrap();
            late_delivery::process(&f.state, &f.event.id).await;
            let requests = sends(&f).await;
            assert_eq!(requests.len(), 1);
            let body: Value = serde_json::from_slice(&requests[0].body).unwrap();
            assert!(
                body["text"]
                    .as_str()
                    .unwrap()
                    .contains("Owner-only NyxID thread")
            );
            assert_eq!(body.as_object().unwrap().len(), 1);
            assert_eq!(status(&f).await, "images_unavailable");
        }
    }
}

#[tokio::test]
async fn selected_subset_posts_through_relay_for_gateway_bots_and_no_selection_sends_no_images() {
    for indices in [vec![], vec![1], vec![1, 2]] {
        let mut f = fixture(&format!("initiated_images_{}", indices.len())).await;
        relay_image_fixture(&mut f).await;
        let (client, requests, server) =
            crate::test_utils::channel_transport::result_transport().await;
        f.state.http_client = client;
        let refs = image_selection(&f, &indices).await;
        let chat: NyxbotThread = f
            .state
            .db
            .collection(THREADS)
            .find_one(doc! {"channel_id":&f.channel.id})
            .await
            .unwrap()
            .unwrap();
        let result = chats::post_automation(
            &f.state,
            OWNER,
            &chat.id,
            "Report",
            None,
            f.event.conversation_id.as_deref().unwrap(),
            f.event.turn_id.as_deref(),
        )
        .await
        .unwrap();
        assert_eq!(
            result["status"],
            if indices.len() == 2 {
                "not_confirmed"
            } else {
                "posted"
            }
        );
        if indices.len() == 2 {
            assert_eq!(result["delivery"]["status"], "not_confirmed");
            assert_eq!(result["delivery"]["images_requested"], 2);
            assert!(
                result["delivery"]["message"]
                    .as_str()
                    .unwrap()
                    .contains("parts may have been delivered")
            );
        }
        let sent = requests.lock().await.clone();
        assert_eq!(sent.len(), 1 + indices.len());
        assert!(sent[0].0.ends_with("sendMessage"));
        if !indices.is_empty() {
            assert!(sent[1].0.ends_with("sendPhoto"));
            let body = sent[1].1["multipart"].as_str().unwrap();
            assert!(body.contains("CHOSEN_IMAGE") && !body.contains("FIRST_IMAGE"));
        }
        // Exercise the native tool: explicit post images do not alter reply selection.
        let message = crate::services::assistant_reply_images::settled(
            &f.state,
            OWNER,
            f.event.conversation_id.as_deref().unwrap(),
            f.event.turn_id.as_deref().unwrap(),
        )
        .await
        .unwrap()
        .unwrap();
        f.state.db.collection::<bson::Document>(CONVERSATIONS).update_one(doc! {"_id":&message.conversation_id},doc! {"$set":{"active_turn":{
            "turn_id":&message.turn_id,"started_at":bson::DateTime::now(),"origin":"channel","stop_requested":false,"attachments":bson::to_bson(&message.attachments).unwrap(),"reply_image_refs":&message.reply_image_refs,
        }}}).await.unwrap();
        let live = engine::get(&f.state.db, OWNER, &message.conversation_id)
            .await
            .unwrap();
        let authority = crate::services::assistant_acknowledgement_service::for_key(
            &f.state.db,
            OWNER,
            Some(&live.credential_api_key_id),
        )
        .await
        .unwrap()
        .unwrap();
        let (result, failed) = crate::handlers::assistant_team::execute_tool(
            &f.state,
            &authority,
            "nyxid__post_to_chat",
            &json!({"chat_id":chat.id,"text":"One-off","image_refs":[refs[0]]}),
        )
        .await;
        assert!(!failed, "{result}");
        assert_eq!(
            engine::get(&f.state.db, OWNER, &message.conversation_id)
                .await
                .unwrap()
                .active_turn
                .unwrap()
                .reply_image_refs,
            message.reply_image_refs
        );
        assert_eq!(result["status"], "posted");
        let sent = requests.lock().await;
        assert!(
            sent.last().unwrap().1["multipart"]
                .as_str()
                .unwrap()
                .contains("FIRST_IMAGE")
        );
        let before_failure = sent.len();
        drop(sent);
        if indices.len() == 2 {
            let (result, failed) = crate::handlers::assistant_team::execute_tool(
                &f.state,
                &authority,
                "nyxid__post_to_chat",
                &json!({"chat_id":chat.id,"text":"Failing one-off","image_refs":[refs[2]]}),
            )
            .await;
            assert!(failed, "An unconfirmed image send must not be tool success");
            assert_eq!(result["status"], "not_confirmed");
            assert_eq!(result["delivery"]["status"], "not_confirmed");
            assert!(
                result["delivery"]["message"]
                    .as_str()
                    .unwrap()
                    .contains("will not retry")
            );
            assert_eq!(requests.lock().await.len(), before_failure + 2);
        }
        assert!(sends(&f).await.is_empty());
        server.abort();
    }
}

#[tokio::test]
async fn direct_selected_reply_records_exact_destination_and_reference_refuses_other_authority() {
    use crate::services::assistant_reply_images as images;
    let mut f = fixture("direct_selected_images").await;
    f.channel.transport = "direct".into();
    let source = relay_image_fixture(&mut f).await;
    let (client, requests, server) = crate::test_utils::channel_transport::result_transport().await;
    f.state.http_client = client;
    let refs = image_selection(&f, &[1]).await;
    let message = images::settled(
        &f.state,
        OWNER,
        f.event.conversation_id.as_deref().unwrap(),
        f.event.turn_id.as_deref().unwrap(),
    )
    .await
    .unwrap()
    .unwrap();
    let report = direct_reply_selected(
        &f.state,
        &f.channel,
        &source.id,
        "Reply",
        None,
        Some(&message),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(report.status(), "confirmed");
    assert_eq!(requests.lock().await.len(), 2);
    let mut auth = crate::handlers::assistant_team::owner_auth(OWNER).unwrap();
    auth.auth_method = crate::mw::auth::AuthMethod::ApiKey;
    auth.api_key_id = Some(f.channel.route_api_key_id.clone());
    auth.scope = "read proxy".into();
    let body:crate::handlers::channel_relay::AsyncReplyRequest<crate::services::channel_platform::RelayMediaSource>=serde_json::from_value(json!({"message_id":source.id,"reply":{"text":"Reserved reference reader", "attachments":[{"kind":"image","source":{"type":"assistant_image","conversation_id":message.conversation_id,"turn_id":message.turn_id,"image_ref":refs[1]}}]}})).unwrap();
    let sent = crate::handlers::channel_relay::async_reply_references(
        State(f.state.clone()),
        HeaderMap::new(),
        crate::mw::auth::OptionalAuthUser(Some(auth)),
        Json(body),
    )
    .await
    .unwrap()
    .0;
    assert!(sent.platform_message_id.is_some());
    let resolved = images::relay_source(
        &f.state,
        &source,
        &f.channel.route_api_key_id,
        &message.conversation_id,
        &message.turn_id,
        &refs[1],
    )
    .await
    .unwrap();
    assert!(matches!(
        resolved.source,
        crate::services::channel_platform::OutboundMediaSource::Base64 { .. }
    ));
    for mode in [
        "destination",
        "thread",
        "bot",
        "route",
        "owner",
        "key",
        "unselected",
        "unknown",
        "turn",
    ] {
        let mut other = source.clone();
        match mode {
            "destination" => other.platform_conversation_id = Some("other".into()),
            "thread" => other.thread_id = Some("other".into()),
            "bot" => other.channel_bot_id = Some("other".into()),
            "route" => other.conversation_id = "other".into(),
            "owner" => other.user_id = "other".into(),
            _ => {}
        }
        let key = if mode == "key" {
            "other"
        } else {
            &f.channel.route_api_key_id
        };
        let image = match mode {
            "unselected" => &refs[0],
            "unknown" => "unknown",
            _ => &refs[1],
        };
        let turn = if mode == "turn" {
            "other"
        } else {
            &message.turn_id
        };
        assert!(
            matches!(
                images::relay_source(&f.state, &other, key, &message.conversation_id, turn, image)
                    .await,
                Err(AppError::NotFound(_))
            ),
            "{mode}"
        );
    }
    // Org routes resolve the linking person's images, never the org wallet's owner.
    let org = Uuid::new_v4().to_string();
    f.state
        .db
        .collection::<crate::models::user::User>(USERS)
        .insert_one(test_user(&org, UserType::Org))
        .await
        .unwrap();
    f.state
        .db
        .collection::<crate::models::org_membership::OrgMembership>(
            crate::models::org_membership::COLLECTION_NAME,
        )
        .insert_one(crate::test_utils::test_membership(
            &org,
            OWNER,
            crate::models::org_membership::OrgRole::Admin,
            None,
        ))
        .await
        .unwrap();
    f.state
        .db
        .collection::<bson::Document>(CHANNELS)
        .update_one(
            doc! {"_id":&f.channel.id},
            doc! {"$set":{"bot_owner_id":&org}},
        )
        .await
        .unwrap();
    let mut org_source = source.clone();
    org_source.user_id = org.clone();
    assert!(
        images::relay_source(
            &f.state,
            &org_source,
            &f.channel.route_api_key_id,
            &message.conversation_id,
            &message.turn_id,
            &refs[1]
        )
        .await
        .is_ok()
    );
    f.state
        .db
        .collection::<bson::Document>(crate::models::org_membership::COLLECTION_NAME)
        .update_one(
            doc! {"org_user_id":&org,"member_user_id":OWNER},
            doc! {"$set":{"role":"member"}},
        )
        .await
        .unwrap();
    assert!(matches!(
        images::relay_source(
            &f.state,
            &org_source,
            &f.channel.route_api_key_id,
            &message.conversation_id,
            &message.turn_id,
            &refs[1]
        )
        .await,
        Err(AppError::NotFound(_))
    ));
    f.state
        .db
        .collection::<bson::Document>(CHANNELS)
        .update_one(
            doc! {"_id":&f.channel.id},
            doc! {"$unset":{"bot_owner_id":""}},
        )
        .await
        .unwrap();
    let settings = crate::models::assistant_upload_retention::Settings {
        policy: Some(crate::models::assistant_upload_retention::Policy {
            tool_image_days: Some(1),
            ..Default::default()
        }),
        ..Default::default()
    };
    f.state
        .db
        .collection::<crate::models::assistant_upload_retention::Settings>("platform_settings")
        .replace_one(doc! {"_id":&settings.id}, &settings)
        .upsert(true)
        .await
        .unwrap();
    f.state.db.collection::<bson::Document>(crate::models::assistant_attachment::COLLECTION_NAME).update_one(doc! {"_id":&refs[1]},doc! {"$set":{"created_at":bson::DateTime::from_chrono(Utc::now()-ChronoDuration::days(2))}}).await.unwrap();
    assert!(matches!(
        images::relay_source(
            &f.state,
            &source,
            &f.channel.route_api_key_id,
            &message.conversation_id,
            &message.turn_id,
            &refs[1]
        )
        .await,
        Err(AppError::NotFound(_))
    ));
    server.abort();
}

#[tokio::test]
async fn direct_reply_with_empty_selection_sends_text_only_and_binds_nothing() {
    use crate::services::assistant_reply_images as images;
    let mut f = fixture("direct_empty_selection").await;
    f.channel.transport = "direct".into();
    let source = relay_image_fixture(&mut f).await;
    let (client, requests, server) = crate::test_utils::channel_transport::result_transport().await;
    f.state.http_client = client;
    // The turn produced images, but the agent selected none of them.
    image_selection(&f, &[]).await;
    let message = images::settled(
        &f.state,
        OWNER,
        f.event.conversation_id.as_deref().unwrap(),
        f.event.turn_id.as_deref().unwrap(),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(message.reply_image_refs.is_empty());
    assert_eq!(message.attachments.len(), 3);
    let report = direct_reply_selected(
        &f.state,
        &f.channel,
        &source.id,
        "Plain answer",
        None,
        Some(&message),
    )
    .await
    .unwrap();
    assert!(report.is_none(), "no image report without a selection");
    let sent = requests.lock().await.clone();
    assert_eq!(sent.len(), 1);
    assert!(sent[0].0.ends_with("sendMessage"));
    assert_eq!(sent[0].1["text"], "Plain answer");
    let after = images::settled(
        &f.state,
        OWNER,
        f.event.conversation_id.as_deref().unwrap(),
        f.event.turn_id.as_deref().unwrap(),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(after.reply_image_destinations.is_empty());
    server.abort();
}

#[tokio::test]
async fn text_only_adapter_degrades_selected_images_without_materializing_or_losing_text() {
    use crate::services::assistant_reply_images as images;
    let f = fixture("text_only_selected_images").await;
    image_selection(&f, &[1]).await;
    let message = images::settled(
        &f.state,
        OWNER,
        f.event.conversation_id.as_deref().unwrap(),
        f.event.turn_id.as_deref().unwrap(),
    )
    .await
    .unwrap()
    .unwrap();
    let adapter = crate::services::channel_adapters::resolve_adapter(
        "aurinko",
        &f.state.token_exchange_cache,
    )
    .unwrap();
    assert!(!images::supports_images(adapter.as_ref()));
    let (text, attachments, omitted) = images::prepare(
        &f.state,
        OWNER,
        "Useful report",
        Some(images::Selection::from_message(&message)),
        images::supports_images(adapter.as_ref()),
    )
    .await;
    assert!(text.starts_with("Useful report"));
    assert!(text.contains("selected images could not be shown") && text.contains("Owner-only"));
    assert!(attachments.is_empty());
    assert_eq!(omitted, 1);
}

#[tokio::test]
async fn late_direct_image_failure_is_not_confirmed_and_not_retried() {
    let mut f = fixture("late_direct_image_not_confirmed").await;
    f.channel.transport = "direct".into();
    relay_image_fixture(&mut f).await;
    patch(
        &f,
        doc! {"delivery.transport":"direct","delivery.state":"pending"},
    )
    .await;
    let (client, requests, server) = crate::test_utils::channel_transport::result_transport().await;
    f.state.http_client = client;
    image_selection(&f, &[1, 2, 0]).await;
    late_delivery::process(&f.state, &f.event.id).await;
    assert_eq!(status(&f).await, "not_confirmed");
    assert_eq!(requests.lock().await.len(), 3);
    let audit: crate::models::audit_log::AuditLog = f
        .state
        .db
        .collection(crate::models::audit_log::COLLECTION_NAME)
        .find_one(
            doc! {"event_type":"nyxbot_channel_answer_delivery","event_data.event_id":&f.event.id},
        )
        .await
        .unwrap()
        .unwrap();
    let data = audit.event_data.unwrap();
    assert_eq!(data["outcome"], "not_confirmed");
    assert_eq!(data["delivery"]["status"], "not_confirmed");
    assert_eq!(data["delivery"]["images_requested"], 3);
    assert!(data["delivery"].get("text").is_none());
    assert!(data["delivery"].get("images_accepted").is_none());
    assert!(
        data["delivery"]["message"]
            .as_str()
            .unwrap()
            .contains("parts may have been delivered")
    );
    late_delivery::process(&f.state, &f.event.id).await;
    assert_eq!(requests.lock().await.len(), 3);
    server.abort();
}

#[tokio::test]
async fn async_selected_images_follow_each_authorized_destination_and_exact_turn() {
    let mut f = fixture("async_selected_images").await;
    f.channel.transport = "direct".into();
    let source = relay_image_fixture(&mut f).await;
    let (client, requests, server) = crate::test_utils::channel_transport::result_transport().await;
    f.state.http_client = client;
    image_selection(&f, &[1]).await;
    f.state
        .db
        .collection::<bson::Document>(THREADS)
        .update_one(
            doc! {"channel_id":&f.channel.id},
            doc! {"$set":{"last_message_id":&source.id}},
        )
        .await
        .unwrap();
    let row = engine::get(
        &f.state.db,
        OWNER,
        f.event.conversation_id.as_deref().unwrap(),
    )
    .await
    .unwrap();
    let own = row.channel.as_ref().unwrap();
    deliver_to_selected(
        &f.state,
        &row,
        own,
        "Async report",
        f.event.turn_id.as_deref(),
    )
    .await;
    assert_eq!(requests.lock().await.len(), 2);

    // The same event reply may already have an additional authorized owner chat.
    let mut extra = own.clone();
    extra.partition = "second-private-chat".into();
    let mut other_source = source.clone();
    other_source.id = Uuid::new_v4().to_string();
    other_source.platform_conversation_id = Some("chat-2".into());
    f.state
        .db
        .collection::<crate::models::channel_message::ChannelMessage>(
            crate::models::channel_message::COLLECTION_NAME,
        )
        .insert_one(&other_source)
        .await
        .unwrap();
    let mut thread: NyxbotThread = f
        .state
        .db
        .collection(THREADS)
        .find_one(doc! {"channel_id":&f.channel.id})
        .await
        .unwrap()
        .unwrap();
    thread.id = Uuid::new_v4().to_string();
    thread.partition = extra.partition.clone();
    thread.last_message_id = Some(other_source.id);
    thread.platform_chat_id = Some("chat-2".into());
    f.state
        .db
        .collection::<NyxbotThread>(THREADS)
        .insert_one(&thread)
        .await
        .unwrap();
    deliver_to_selected(
        &f.state,
        &row,
        &extra,
        "Also deliver",
        f.event.turn_id.as_deref(),
    )
    .await;
    let sent = requests.lock().await.clone();
    assert_eq!(sent.len(), 4);
    for (index, chat) in [(1, "chat-1"), (3, "chat-2")] {
        let body = sent[index].1["multipart"].as_str().unwrap();
        assert!(body.contains(chat) && body.contains("CHOSEN_IMAGE"));
        assert!(!body.contains("FIRST_IMAGE") && !body.contains("FAIL_IMAGE"));
    }
    deliver_to_selected(&f.state, &row, own, "Other turn", Some("other-turn")).await;
    assert_eq!(requests.lock().await.len(), 5);
    // Relinking the additional chat revokes that destination, including images.
    f.state
        .db
        .collection::<bson::Document>(THREADS)
        .update_one(
            doc! {"_id":&thread.id},
            doc! {"$set":{"conversation_id":"another-agent"}},
        )
        .await
        .unwrap();
    deliver_to_selected(
        &f.state,
        &row,
        &extra,
        "Revoked",
        f.event.turn_id.as_deref(),
    )
    .await;
    assert_eq!(requests.lock().await.len(), 5);
    server.abort();
}

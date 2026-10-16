//! Exercise durable settlement paths with real relay adapters and Mongo claims.
use super::*;
use crate::handlers::nyxbot::images::{
    self, ImageAnchor, ImageAudience, ImageSendContext, ImageSendPlan,
};
use crate::models::{
    assistant_attachment::AssistantAttachment, assistant_conversation::TurnAttachment,
    nyxbot_image_delivery::NyxbotImageDelivery,
};
use crate::services::assistant_acknowledgement_service as acks;
use crate::services::channel_platform::PlatformAdapter;
use wiremock::matchers::body_string_contains;

async fn image_fixture(platform: &str, transport: &str) -> (Fixture, MockServer) {
    let mut f = fixture(&format!("nyxbot_images_{platform}_{transport}")).await;
    let server = MockServer::start().await;
    f.channel.platform = platform.into();
    f.channel.transport = transport.into();
    let route = Uuid::new_v4().to_string();
    f.channel.route_id = Some(route.clone());
    f.state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .replace_one(doc! {"_id":&f.channel.id}, &f.channel)
        .await
        .unwrap();
    f.state.db.collection::<bson::Document>(crate::models::channel_conversation::COLLECTION_NAME).insert_one(doc! {
        "_id":&route,"user_id":OWNER,"channel_bot_id":&f.channel.channel_bot_id,"platform":platform,
        "platform_conversation_id":"123","platform_conversation_type":"private","agent_api_key_id":&f.channel.route_api_key_id,
        "is_active":true,"created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now(),
    }).await.unwrap();
    let original = if transport == "direct" {
        f.event.event_id.clone()
    } else {
        Uuid::new_v4().to_string()
    };
    f.state.db.collection::<bson::Document>(crate::models::channel_message::COLLECTION_NAME).insert_one(doc! {
        "_id":&original,"conversation_id":&route,"user_id":OWNER,"channel_bot_id":&f.channel.channel_bot_id,"platform":platform,
        "platform_conversation_id":"123","direction":"inbound","platform_message_id":"17","sender_platform_id":"7",
        "content_type":"text","agent_api_key_id":&f.channel.route_api_key_id,"created_at":bson::DateTime::now(),
    }).await.unwrap();
    f.event.relay_message_id = Some(original.clone());
    let d = f.event.delivery.as_mut().unwrap();
    d.transport = transport.into();
    d.origin.platform = platform.into();
    let origin = bson::to_bson(&d.origin).unwrap();
    f.state
        .db
        .collection::<NyxbotEvent>(EVENTS)
        .replace_one(doc! {"_id":&f.event.id}, &f.event)
        .await
        .unwrap();
    f.state
        .db
        .collection::<bson::Document>(CONVERSATIONS)
        .update_one(
            doc! {"_id":&f.event.conversation_id},
            doc! {"$set":{"channel":origin}},
        )
        .await
        .unwrap();
    f.state.db.collection::<bson::Document>(THREADS).update_one(doc! {"channel_id":&f.channel.id},doc! {"$set":{
        "platform_chat_id":"123","last_relay_message_id":&original,"last_message_id":&original,"allow_posts":true,
    }}).await.unwrap();
    let token = if platform == "lark" {
        "app:secret"
    } else {
        "token"
    };
    let encrypted = f
        .state
        .encryption_keys
        .encrypt(token.as_bytes())
        .await
        .unwrap();
    f.state.db.collection::<bson::Document>(crate::models::channel_bot::COLLECTION_NAME).update_one(doc! {"_id":&f.channel.channel_bot_id},doc! {"$set":{
        "platform":platform,"bot_token_encrypted":bson::Binary {subtype:bson::spec::BinarySubtype::Generic,bytes:encrypted},
    }}).await.unwrap();
    let adapter: std::sync::Arc<dyn PlatformAdapter> = if platform == "lark" {
        Mock::given(path("/open-apis/auth/v3/tenant_access_token/internal"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"code":0,"tenant_access_token":"tenant","expire":7200})),
            )
            .mount(&server)
            .await;
        std::sync::Arc::new(
            crate::services::channel_adapters::lark::LarkFamilyAdapter::media_test_adapter(
                &server.uri(),
                platform,
            ),
        )
    } else if platform == "slack" {
        std::sync::Arc::new(
            crate::services::channel_adapters::slack::SlackAdapter::media_test_adapter(
                &server.uri(),
            ),
        )
    } else {
        std::sync::Arc::new(
            crate::services::channel_adapters::telegram::TelegramAdapter::media_test_adapter(
                &server.uri(),
            ),
        )
    };
    crate::handlers::channel_relay::TEST_REPLY_ADAPTERS
        .lock()
        .unwrap()
        .insert(f.channel.channel_bot_id.clone(), adapter);
    settled(&f, false).await;
    (f, server)
}

async fn add_images(f: &Fixture) -> Vec<TurnAttachment> {
    let mut attachments = Vec::new();
    for index in 0..2 {
        let bytes = format!("pixels-{index}");
        let row = AssistantAttachment {
            id: Uuid::new_v4().to_string(),
            origin: "tool".into(),
            user_id: OWNER.into(),
            conversation_id: f.event.conversation_id.clone().unwrap(),
            turn_id: f.event.turn_id.clone().unwrap(),
            content_type: "image/png".into(),
            size: bytes.len() as i64,
            data_encrypted: f
                .state
                .encryption_keys
                .encrypt(bytes.as_bytes())
                .await
                .unwrap(),
            created_at: Utc::now(),
        };
        f.state
            .db
            .collection::<AssistantAttachment>(crate::models::assistant_attachment::COLLECTION_NAME)
            .insert_one(&row)
            .await
            .unwrap();
        attachments.push(TurnAttachment {
            id: row.id,
            label: "private_tool_label".into(),
            content_type: row.content_type,
            size: row.size,
            origin: "tool".into(),
            pages: None,
            chat_delivery: None,
            image_input: None,
        });
    }
    f.state
        .db
        .collection::<bson::Document>(crate::models::assistant_message::COLLECTION_NAME)
        .update_one(
            doc! {"conversation_id":&f.event.conversation_id,"turn_id":&f.event.turn_id},
            doc! {"$set":{"attachments":bson::to_bson(&attachments).unwrap()}},
        )
        .await
        .unwrap();
    attachments
}

async fn mock_sends(
    server: &MockServer,
    platform: &str,
    second_fails: bool,
    delay: std::time::Duration,
) {
    if platform == "lark" {
        for index in 0..2 {
            Mock::given(path("/open-apis/im/v1/images"))
                .and(body_string_contains(format!("pixels-{index}")))
                .respond_with(ResponseTemplate::new(200).set_delay(delay).set_body_json(
                    if second_fails && index == 1 {
                        json!({"code":999,"msg":"refused"})
                    } else {
                        json!({"code":0,"data":{"image_key":format!("image-{index}")}})
                    },
                ))
                .mount(server)
                .await;
        }
        Mock::given(path("/open-apis/im/v1/messages"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"code":0,"data":{"message_id":"sent"}})),
            )
            .mount(server)
            .await;
    } else {
        Mock::given(path("/bottoken/sendPhoto"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_delay(delay)
                    .set_body_json(json!({"ok":true,"result":{"message_id":42}})),
            )
            .mount(server)
            .await;
        Mock::given(path("/bottoken/sendMessage"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"ok":true,"result":{"message_id":43}})),
            )
            .mount(server)
            .await;
    }
}

struct ImageClaims {
    items: Vec<NyxbotImageDelivery>,
}
async fn claim(f: &Fixture) -> ImageClaims {
    use futures::TryStreamExt;
    let items = f
        .state
        .db
        .collection::<NyxbotImageDelivery>(crate::models::nyxbot_image_delivery::COLLECTION_NAME)
        .find(doc! {"conversation_id":&f.event.conversation_id})
        .sort(doc! {"created_at":1,"_id":1})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    ImageClaims { items }
}
async fn thread(f: &Fixture) -> NyxbotThread {
    f.state
        .db
        .collection::<NyxbotThread>(THREADS)
        .find_one(doc! {"channel_id":&f.channel.id})
        .await
        .unwrap()
        .unwrap()
}
async fn send_plan(
    f: &Fixture,
    attachments: Vec<TurnAttachment>,
    deadline: Option<std::time::Instant>,
) -> images::ImageReport {
    let thread = thread(f).await;

    images::send_turn_images(
        &f.state,
        ImageSendContext {
            owner: OWNER,
            conversation_id: f.event.conversation_id.as_deref().unwrap(),
            turn_id: f.event.turn_id.as_deref().unwrap(),
            channel: &f.channel,
            thread: &thread,
            anchor: ImageAnchor::Inbound {
                message_id: f.event.relay_message_id.clone().unwrap(),
            },
            audience: ImageAudience {
                owner_private: true,
            },
            deadline,

            initiated: None,
            fence: None,
        },
        &ImageSendPlan { attachments },
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn nyxbot_v1_image_barriers_prevent_provider_calls_and_leave_new_images_deliverable() {
    use crate::models::nyxbot_image_delivery::{COLLECTION_NAME, LEGACY_COLLECTION_NAME};
    for (state, item_state, expected) in [
        ("done", "sent", "previously_delivered"),
        ("sending", "sending", "in_progress_or_unknown"),
        ("done", "unknown", "in_progress_or_unknown"),
        ("sending", "pending", "in_progress_or_unknown"),
    ] {
        let (f, server) = image_fixture("telegram", "direct").await;
        let attachments = add_images(&f).await;
        mock_sends(&server, "telegram", false, std::time::Duration::ZERO).await;
        let legacy = doc! {
            "_id": Uuid::new_v4().to_string(), "user_id": OWNER,
            "conversation_id": &f.event.conversation_id, "turn_id": &f.event.turn_id,
            "channel_id": &f.channel.id, "destination_key": "initiated:old-chat",
            "state": state, "claim_id": "old-attempt",
            "items": [{"id": &attachments[0].id, "state": item_state}],
            "created_at": bson::DateTime::now(),
            "expires_at": bson::DateTime::from_chrono(Utc::now() + ChronoDuration::days(7)),
        };
        f.state
            .db
            .collection::<bson::Document>(LEGACY_COLLECTION_NAME)
            .insert_one(&legacy)
            .await
            .unwrap();
        let report = send_plan(&f, attachments.clone(), None).await;
        assert_eq!(report.outcomes[0].status, expected);
        assert_eq!(report.outcomes[1].status, "sent_now");
        assert_eq!((report.claimed, report.sent), (1, 1));
        let repeated = send_plan(&f, attachments, None).await;
        assert_eq!(repeated.outcomes[0].status, expected);
        assert_eq!(repeated.outcomes[1].status, "previously_delivered");
        assert_eq!(repeated.claimed, 0);
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 1);
        assert!(String::from_utf8_lossy(&requests[0].body).contains("pixels-1"));
        assert_eq!(
            f.state
                .db
                .collection::<bson::Document>(COLLECTION_NAME)
                .count_documents(doc! {})
                .await
                .unwrap(),
            1
        );
        assert_eq!(
            f.state
                .db
                .collection::<bson::Document>(LEGACY_COLLECTION_NAME)
                .find_one(doc! {"_id": &legacy["_id"]})
                .await
                .unwrap(),
            Some(legacy)
        );
        f.state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn nyxbot_direct_images_precede_text_preserve_anchor_and_never_resend() {
    for platform in ["lark", "telegram"] {
        let (f, server) = image_fixture(platform, "direct").await;
        add_images(&f).await;
        if platform == "telegram" {
            f.state
                .db
                .collection::<bson::Document>(crate::models::channel_message::COLLECTION_NAME)
                .update_one(
                    doc! {"_id":f.event.relay_message_id.as_deref().unwrap()},
                    doc! {"$set":{"thread_id":"88"}},
                )
                .await
                .unwrap();
        }
        mock_sends(&server, platform, false, std::time::Duration::ZERO).await;
        delivery::detach(&f.state.db, &f.event.id).await.unwrap();
        futures::future::join_all((0..8).map(|_| late_delivery::process(&f.state, &f.event.id)))
            .await;
        late_delivery::process(&f.state, &f.event.id).await;
        assert_eq!(status(&f).await, "sent");
        let requests = server.received_requests().await.unwrap();
        let paths: Vec<_> = requests
            .iter()
            .filter(|r| !r.url.path().contains("tenant_access_token"))
            .map(|r| r.url.path())
            .collect();
        if platform == "lark" {
            assert_eq!(
                paths,
                [
                    "/open-apis/im/v1/images",
                    "/open-apis/im/v1/messages",
                    "/open-apis/im/v1/images",
                    "/open-apis/im/v1/messages",
                    "/open-apis/im/v1/messages"
                ]
            );
        } else {
            assert_eq!(
                paths,
                [
                    "/bottoken/sendPhoto",
                    "/bottoken/sendPhoto",
                    "/bottoken/sendMessage"
                ]
            );
            for request in &requests[..2] {
                assert!(
                    String::from_utf8_lossy(&request.body)
                        .contains("name=\"reply_to_message_id\"\r\n\r\n17")
                );
                assert!(
                    String::from_utf8_lossy(&request.body)
                        .contains("name=\"message_thread_id\"\r\n\r\n88")
                );
            }
        }
        for r in &requests {
            let body = String::from_utf8_lossy(&r.body);
            assert!(!body.contains("private_tool_label"));
            assert!(!body.contains("caption"));
        }
        let claim = claim(&f).await;
        assert!(claim.items.iter().all(|item| !matches!(
            item.state,
            crate::models::nyxbot_image_delivery::ImageDeliveryState::Pending
                | crate::models::nyxbot_image_delivery::ImageDeliveryState::Sending
        )));
        assert!(claim.items.iter().all(|i| i.state.as_str() == "sent"));
        f.state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn nyxbot_gateway_late_images_are_sent_before_reply_target_text_once() {
    let (f, server) = image_fixture("telegram", "gateway").await;
    add_images(&f).await;
    let photos = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let seen = photos.clone();
    Mock::given(path("/bottoken/sendPhoto"))
        .respond_with(move |_: &wiremock::Request| {
            seen.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            ResponseTemplate::new(200).set_body_json(json!({"ok":true,"result":{"message_id":42}}))
        })
        .mount(&server)
        .await;
    let seen = photos.clone();
    Mock::given(path("/reply-targets/cmaeg1.rt.original/messages"))
        .respond_with(move |_: &wiremock::Request| {
            assert_eq!(seen.load(std::sync::atomic::Ordering::SeqCst), 2);
            ResponseTemplate::new(200).set_body_json(json!({"delivery":{"outcome":"confirmed"}}))
        })
        .with_priority(1)
        .expect(1)
        .mount(&f.gateway)
        .await;
    delivery::detach(&f.state.db, &f.event.id).await.unwrap();
    late_delivery::process(&f.state, &f.event.id).await;
    late_delivery::process(&f.state, &f.event.id).await;
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 2);
    assert!(requests.iter().all(|r| r.url.path().ends_with("sendPhoto")));
    let text = sends(&f).await;
    assert_eq!(text.len(), 1);
    f.gateway.verify().await;
    assert_eq!(
        claim(&f)
            .await
            .items
            .iter()
            .filter(|i| i.state.as_str() == "sent")
            .count(),
        2
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn nyxbot_gateway_stream_keeps_alive_while_images_precede_committed_text() {
    let (f, server) = image_fixture("telegram", "gateway").await;
    add_images(&f).await;
    mock_sends(
        &server,
        "telegram",
        false,
        std::time::Duration::from_secs(8),
    )
    .await;
    let (sender, receiver) = broadcast::channel(8);
    sender
        .send(json!({"event":"block.completed","block":{"text":"Useful answer"}}))
        .unwrap();
    sender
        .send(json!({"event":"turn.completed","status":"completed"}))
        .unwrap();
    let response = late_delivery::provider_stream(
        f.state.clone(),
        f.event.id.clone(),
        "response".into(),
        json!({"type":"response.created"}),
        receiver,
        Utc::now(),
    );
    let text = body_text(response).await;
    assert!(text.contains("Useful answer"));
    assert!(!text.contains("could not be sent"));
    let keepalive = text
        .find(":\n\n")
        .expect("keepalive during the two delayed sends");
    assert!(keepalive < text.find("response.output_item.done").unwrap());
    assert_eq!(server.received_requests().await.unwrap().len(), 2);
    assert_eq!(status(&f).await, "streamed");
    assert!(sends(&f).await.is_empty());
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn nyxbot_partial_image_failure_and_retention_have_durable_counts() {
    for expired in [false, true] {
        let (f, server) = image_fixture("lark", "direct").await;
        let images = add_images(&f).await;
        mock_sends(&server, "lark", !expired, std::time::Duration::ZERO).await;
        if expired {
            f.state
                .db
                .collection::<AssistantAttachment>(
                    crate::models::assistant_attachment::COLLECTION_NAME,
                )
                .delete_one(doc! {"_id":&images[1].id})
                .await
                .unwrap();
        }
        delivery::detach(&f.state.db, &f.event.id).await.unwrap();
        late_delivery::process(&f.state, &f.event.id).await;
        let requests = server.received_requests().await.unwrap();
        let text: Value = serde_json::from_slice(&requests.last().unwrap().body).unwrap();
        assert!(text.to_string().contains("1 of 2 image(s)"));
        assert!(text.to_string().contains("NyxID app"));
        let claim = claim(&f).await;
        assert_eq!(claim.items[0].state.as_str(), "sent");
        if expired {
            assert_eq!(claim.items.len(), 1);
        } else {
            assert_eq!(claim.items[1].state.as_str(), "failed");
            assert_eq!(claim.items[1].code.as_deref(), Some("platform_error"));
        }
        f.state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn nyxbot_unknown_image_skips_rest_and_claim_survives_retry() {
    let (f, server) = image_fixture("telegram", "direct").await;
    let images = add_images(&f).await;
    mock_sends(
        &server,
        "telegram",
        false,
        std::time::Duration::from_secs(36),
    )
    .await;
    let first = send_plan(
        &f,
        images.clone(),
        Some(std::time::Instant::now() + std::time::Duration::from_secs(60)),
    )
    .await;
    assert_eq!((first.unknown, first.skipped), (1, 1));
    let before = claim(&f).await;
    assert_eq!(before.items[0].state.as_str(), "unknown");
    assert_eq!(before.items[1].state.as_str(), "pending");
    assert!(before.items[1].code.is_none());
    assert!(before.items[1].attempt.is_none());
    Mock::given(path("/bottoken/sendPhoto"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"ok":true,"result":{"message_id":19}})),
        )
        .with_priority(1)
        .mount(&server)
        .await;
    let again = send_plan(&f, images, None).await;
    assert_eq!(images::image_result(&again)["unknown"], 1);
    assert_eq!(again.sent, 1);
    assert!(images::failure_note(&again).is_empty());
    assert_eq!(server.received_requests().await.unwrap().len(), 2);
    let claim = claim(&f).await;
    assert_eq!(claim.items[0].state.as_str(), "unknown");
    assert_eq!(claim.items[1].state.as_str(), "sent");
    delivery::detach(&f.state.db, &f.event.id).await.unwrap();
    late_delivery::process(&f.state, &f.event.id).await;
    late_delivery::process(&f.state, &f.event.id).await;
    let requests = server.received_requests().await.unwrap();
    assert_eq!(
        requests
            .iter()
            .filter(|r| r.url.path().ends_with("sendPhoto"))
            .count(),
        2
    );
    assert_eq!(
        requests
            .iter()
            .filter(|r| r.url.path().ends_with("sendMessage"))
            .count(),
        1
    );
    assert!(!String::from_utf8_lossy(&requests.last().unwrap().body).contains("The 2 image(s)"));
    assert_eq!(status(&f).await, "sent");
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn nyxbot_gateway_missing_anchor_fails_images_without_changing_text_target() {
    let (f, server) = image_fixture("telegram", "gateway").await;
    add_images(&f).await;
    patch(&f, doc! {"relay_message_id":bson::Bson::Null}).await;
    delivery::detach(&f.state.db, &f.event.id).await.unwrap();
    late_delivery::process(&f.state, &f.event.id).await;
    assert!(server.received_requests().await.unwrap().is_empty());
    let text = sends(&f).await;
    assert_eq!(text.len(), 1);
    assert!(String::from_utf8_lossy(&text[0].body).contains("The 2 image(s)"));
    assert!(
        claim(&f)
            .await
            .items
            .iter()
            .all(|i| i.code.as_deref() == Some("no_anchor"))
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn nyxbot_gateway_expired_event_reference_keeps_images_on_relay_and_text_off_it() {
    let (f, server) = image_fixture("telegram", "gateway").await;
    let attachments = add_images(&f).await;
    mock_sends(&server, "telegram", false, std::time::Duration::ZERO).await;
    let ciphertext = f
        .state
        .encryption_keys
        .encrypt(b"expired-reference")
        .await
        .unwrap();
    f.state.db.collection::<bson::Document>(THREADS).update_one(
        doc! {"channel_id":&f.channel.id},
        doc! {"$set":{
            "last_message_id":bson::Bson::Null,
            "event_ref_ciphertext":bson::Binary {subtype:bson::spec::BinarySubtype::Generic,bytes:ciphertext},
            "event_ref_expires_at":bson::DateTime::from_chrono(Utc::now()-ChronoDuration::seconds(1)),
        }},
    ).await.unwrap();
    f.state.db.collection::<bson::Document>(CONVERSATIONS).update_one(
        doc! {"_id":&f.event.conversation_id},
        doc! {"$set":{"active_turn":{
            "turn_id":&f.event.turn_id,"origin":"event","started_at":bson::DateTime::now(),"stop_requested":false,
        }}},
    ).await.unwrap();
    let row = engine::get(
        &f.state.db,
        OWNER,
        f.event.conversation_id.as_deref().unwrap(),
    )
    .await
    .unwrap();
    crate::handlers::nyxbot::deliver_to(
        &f.state,
        &row,
        row.channel.as_ref().unwrap(),
        "Useful answer",
        Some(&ImageSendPlan { attachments }),
    )
    .await;
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 2);
    assert!(requests.iter().all(|r| r.url.path().ends_with("sendPhoto")));
    assert!(sends(&f).await.is_empty());
    assert!(claim(&f).await.items.iter().all(|item| !matches!(
        item.state,
        crate::models::nyxbot_image_delivery::ImageDeliveryState::Pending
            | crate::models::nyxbot_image_delivery::ImageDeliveryState::Sending
    )));
    assert!(thread(&f).await.last_message_id.is_none());
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn nyxbot_telegram_group_images_are_paced_without_using_private_labels() {
    let (f, server) = image_fixture("telegram", "direct").await;
    let attachments = add_images(&f).await;
    mock_sends(&server, "telegram", false, std::time::Duration::ZERO).await;
    f.state
        .db
        .collection::<bson::Document>(THREADS)
        .update_one(
            doc! {"channel_id":&f.channel.id},
            doc! {"$set":{"kind":"group"}},
        )
        .await
        .unwrap();
    let started = std::time::Instant::now();
    let report = send_plan(&f, attachments, None).await;
    assert_eq!(report.sent, 2);
    assert!(started.elapsed() >= std::time::Duration::from_secs(1));
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 2);
    assert!(
        requests
            .iter()
            .all(|r| !String::from_utf8_lossy(&r.body).contains("private_tool_label"))
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn nyxbot_x_image_billing_refusal_suspends_bot_and_skips_remaining_images() {
    let (mut f, server) = image_fixture("telegram", "direct").await;
    let attachments = add_images(&f).await;
    f.channel.platform = "x".into();
    for collection in [
        crate::models::channel_bot::COLLECTION_NAME,
        crate::models::channel_conversation::COLLECTION_NAME,
        crate::models::channel_message::COLLECTION_NAME,
    ] {
        f.state
            .db
            .collection::<bson::Document>(collection)
            .update_many(
                doc! {},
                doc! {"$set":{"platform":"x","webhook_registered":false}},
            )
            .await
            .unwrap();
    }
    let provider_id = Uuid::new_v4().to_string();
    f.state.db.collection::<bson::Document>(crate::models::provider_config::COLLECTION_NAME).insert_one(doc! {
        "_id": &provider_id, "slug": "twitter", "name": "X", "provider_type": "oauth2", "is_active": true,
        "supports_pkce": true, "credential_mode": "user", "created_by": OWNER,
        "created_at": bson::DateTime::now(), "updated_at": bson::DateTime::now(),
    }).await.unwrap();
    let connection_id = Uuid::new_v4().to_string();
    let access_token = f
        .state
        .encryption_keys
        .encrypt(b"test-access-token")
        .await
        .unwrap();
    f.state.db.collection::<bson::Document>(crate::models::user_api_key::COLLECTION_NAME).insert_one(doc! {
        "_id": &connection_id, "user_id": OWNER, "label": "X account", "credential_type": "oauth2", "credential_source": "platform",
        "provider_config_id": &provider_id, "status": "active",
        "token_scopes": crate::services::channel_adapters::x::REQUIRED_SCOPES.join(" "),
        "access_token_encrypted": bson::Binary { subtype: bson::spec::BinarySubtype::Generic, bytes: access_token },
        "created_at": bson::DateTime::now(), "updated_at": bson::DateTime::now(),
    }).await.unwrap();
    f.state
        .db
        .collection::<bson::Document>(crate::models::channel_bot::COLLECTION_NAME)
        .update_one(
            doc! {"_id": &f.channel.channel_bot_id},
            doc! {"$set": {"credential_source": "connection", "connection_id": &connection_id}},
        )
        .await
        .unwrap();
    f.state.config.billing_enabled = true;
    f.state.billing = std::sync::Arc::new(crate::services::billing::BillingService::new(
        f.state.db.clone(),
        std::sync::Arc::new(f.state.config.clone()),
    ));
    crate::handlers::channel_relay::TEST_REPLY_ADAPTERS
        .lock()
        .unwrap()
        .insert(
            f.channel.channel_bot_id.clone(),
            std::sync::Arc::new(crate::services::channel_adapters::x::XAdapter {
                api_base: Some(server.uri()),
            }),
        );
    let report = send_plan(&f, attachments.clone(), None).await;
    assert_eq!((report.failed, report.skipped), (1, 1));
    let row = claim(&f).await;
    assert!(row.items.iter().all(|item| item.state.as_str() == "pending"
        && item.code.is_none()
        && item.attempt.is_none()));
    let bot = crate::services::channel_bot_service::get_bot(&f.state.db, &f.channel.channel_bot_id)
        .await
        .unwrap();
    assert_eq!(bot.status, "failed");
    let repeated = send_plan(&f, attachments, None).await;
    assert_eq!(repeated.claimed, 2);
    assert_eq!(repeated.sent, 0);
    assert!(server.received_requests().await.unwrap().is_empty());
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn nyxbot_image_anchor_predicates_cover_rolling_and_returned_transports() {
    let (f, _server) = image_fixture("telegram", "gateway").await;
    let mut event = f.event.clone();
    let mut thread = thread(&f).await;
    let mut channel = f.channel.clone();
    event.relay_message_id = None;
    assert!(matches!(
        images::event_anchor(&event, event.delivery.as_ref().unwrap()),
        ImageAnchor::NoAnchor
    ));
    event.delivery.as_mut().unwrap().transport = "direct".into();
    assert!(
        matches!(images::event_anchor(&event,event.delivery.as_ref().unwrap()),ImageAnchor::Inbound{message_id} if message_id==event.event_id)
    );
    thread.last_relay_message_id = None;
    thread.event_ref_ciphertext = Some(vec![1]);
    assert!(matches!(
        images::thread_anchor(&channel, &thread),
        ImageAnchor::NoAnchor
    ));
    channel.transport = "direct".into();
    assert!(
        matches!(images::thread_anchor(&channel,&thread),ImageAnchor::Inbound{message_id} if Some(message_id.as_str())==thread.last_message_id.as_deref())
    );
    channel.transport = "gateway".into();
    thread.event_ref_ciphertext = None;
    assert!(
        matches!(images::thread_anchor(&channel,&thread),ImageAnchor::Inbound{message_id} if Some(message_id.as_str())==thread.last_message_id.as_deref())
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn nyxbot_partial_and_cancelled_turns_deliver_images_only_with_useful_text() {
    for (status_value, text, expected) in [
        ("failed", "Useful answer", true),
        ("cancelled", "Useful answer", true),
        ("failed", "", false),
        ("cancelled", "", false),
    ] {
        let (f, server) = image_fixture("telegram", "direct").await;
        add_images(&f).await;
        mock_sends(&server, "telegram", false, std::time::Duration::ZERO).await;
        f.state
            .db
            .collection::<bson::Document>(crate::models::assistant_message::COLLECTION_NAME)
            .update_one(
                doc! {"conversation_id":&f.event.conversation_id},
                doc! {"$set":{"status":status_value,"text":text}},
            )
            .await
            .unwrap();
        delivery::detach(&f.state.db, &f.event.id).await.unwrap();
        late_delivery::process(&f.state, &f.event.id).await;
        let requests = server.received_requests().await.unwrap();
        assert_eq!(
            requests
                .iter()
                .filter(|r| r.url.path().ends_with("sendPhoto"))
                .count(),
            if expected { 2 } else { 0 }
        );
        if expected {
            assert!(String::from_utf8_lossy(&requests.last().unwrap().body).contains("Incomplete"));
        }
        f.state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn nyxbot_guest_image_failure_hides_app_and_revoked_audience_sends_nothing() {
    for revoked in [false, true] {
        let (f, server) = image_fixture("lark", "direct").await;
        add_images(&f).await;
        mock_sends(&server, "lark", true, std::time::Duration::ZERO).await;
        patch(
            &f,
            doc! {"delivery.guest":true,"delivery.sender_id":"guest"},
        )
        .await;
        f.state
            .db
            .collection::<bson::Document>(THREADS)
            .update_one(
                doc! {"channel_id":&f.channel.id},
                doc! {"$set":{"kind":"group","members":if revoked{"owner"}else{"everyone"}}},
            )
            .await
            .unwrap();
        delivery::detach(&f.state.db, &f.event.id).await.unwrap();
        late_delivery::process(&f.state, &f.event.id).await;
        let requests = server.received_requests().await.unwrap();
        if revoked {
            assert!(requests.is_empty());
            assert_eq!(status(&f).await, "refused");
        } else {
            let text = String::from_utf8_lossy(&requests.last().unwrap().body);
            assert!(text.contains("1 of 2 image(s)"));
            assert!(!text.contains("NyxID app"));
        }
        f.state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn nyxbot_event_turn_and_duplicate_question_destinations_get_images_before_text() {
    let (f, server) = image_fixture("telegram", "direct").await;
    let attachments = add_images(&f).await;
    mock_sends(&server, "telegram", false, std::time::Duration::ZERO).await;
    // after_turn receives the pre-settlement conversation snapshot by design.
    f.state.db.collection::<bson::Document>(CONVERSATIONS).update_one(doc! {"_id":&f.event.conversation_id},doc! {"$set":{"active_turn":{
        "turn_id":&f.event.turn_id,"origin":"event","started_at":bson::DateTime::now(),"stop_requested":false,
    }}}).await.unwrap();
    let row = engine::get(
        &f.state.db,
        OWNER,
        f.event.conversation_id.as_deref().unwrap(),
    )
    .await
    .unwrap();
    // An event turn on a channel thread uses its configured chat. An additional
    // reply destination follows the same image plan and has independent claims.
    crate::handlers::assistant_team::after_turn_boxed(
        f.state.clone(),
        row,
        "Useful answer".into(),
        None,
    )
    .await;
    let requests = server.received_requests().await.unwrap();
    assert_eq!(
        requests
            .iter()
            .filter(|r| r.url.path().ends_with("sendPhoto"))
            .count(),
        attachments.len()
    );
    assert!(requests.last().unwrap().url.path().ends_with("sendMessage"));
    assert!(claim(&f).await.items.iter().all(|item| !matches!(
        item.state,
        crate::models::nyxbot_image_delivery::ImageDeliveryState::Pending
            | crate::models::nyxbot_image_delivery::ImageDeliveryState::Sending
    )));
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn nyxbot_unsupported_chats_record_failure_without_adapter_calls() {
    for platform in ["openclaw", "x"] {
        let (mut f, server) = image_fixture("telegram", "direct").await;
        let attachments = add_images(&f).await;
        f.channel.platform = platform.into();
        if platform == "x" {
            f.state
                .db
                .collection::<bson::Document>(THREADS)
                .update_one(
                    doc! {"channel_id":&f.channel.id},
                    doc! {"$set":{"platform_chat_id":"post:123"}},
                )
                .await
                .unwrap();
        }
        let report = send_plan(&f, attachments, None).await;
        assert_eq!(report.failed, 2);
        assert!(report.unsupported);
        assert!(images::failure_note(&report).contains("cannot receive images"));
        assert!(server.received_requests().await.unwrap().is_empty());
        f.state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn nyxbot_automation_result_images_share_one_limiter_token_and_single_claim() {
    automation_result(false, false, false).await;
}

#[tokio::test]
async fn nyxbot_trigger_scheduler_settles_group_bound_run() {
    automation_result(true, false, false).await;
}

#[tokio::test]
async fn nyxbot_trigger_scheduler_image_metadata_error_still_posts_text_and_finishes() {
    automation_result(false, true, false).await;
}

#[tokio::test]
async fn nyxbot_trigger_scheduler_claim_refreshes_images_from_a_concurrent_settlement() {
    automation_result(false, false, true).await;
}

async fn automation_result(group_bound: bool, metadata_error: bool, stale_run: bool) {
    use crate::models::trigger_run::{RunOutcome, TriggerRun};
    use crate::models::trigger_schedule::DeliverTo;
    let (f, server) = image_fixture("lark", "direct").await;
    let attachments = add_images(&f).await;
    mock_sends(&server, "lark", true, std::time::Duration::ZERO).await;
    let chat = thread(&f).await;
    let run_id = Uuid::new_v4().to_string();
    f.state.db.collection::<bson::Document>(crate::models::trigger_run::COLLECTION_NAME).insert_one(doc! {
        "_id":&run_id,"trigger_id":Uuid::new_v4().to_string(),"user_id":OWNER,"scheduled_at":bson::DateTime::now(),
        "deadline":bson::DateTime::from_chrono(Utc::now()+ChronoDuration::hours(1)),"outcome":"started",
        "thread_id":&f.event.conversation_id,"turn_id":&f.event.turn_id,"label":"Image automation","fence":"test",
        "lease_until":bson::DateTime::now(),"expires_at":bson::DateTime::from_chrono(Utc::now()+ChronoDuration::days(7)),
        "deliver_to":bson::to_bson(&DeliverTo::Chat {chat_id:chat.id.clone()}).unwrap(),
        "image_refs":bson::to_bson(&attachments.iter().map(|attachment|crate::models::trigger_run::RunImageRef {
            conversation_id:f.event.conversation_id.clone().unwrap(),turn_id:f.event.turn_id.clone().unwrap(),attachment:attachment.clone(),
        }).collect::<Vec<_>>()).unwrap(),
    }).await.unwrap();
    f.state.db.collection::<bson::Document>(CONVERSATIONS).update_one(doc! {"_id":&f.event.conversation_id},doc! {"$set":{"active_turn":{
        "turn_id":&f.event.turn_id,"origin":"trigger","trigger_run_id":&run_id,"started_at":bson::DateTime::now(),"stop_requested":false,
    }}}).await.unwrap();
    let mut row = engine::get(
        &f.state.db,
        OWNER,
        f.event.conversation_id.as_deref().unwrap(),
    )
    .await
    .unwrap();
    if group_bound {
        row.group_id = Some(Uuid::new_v4().to_string());
        assert!(matches!(
            engine::answer_routing(&row, row.active_turn.as_ref().unwrap()).primary,
            engine::PrimaryDestination::Group { .. }
        ));
    }
    if metadata_error {
        // Corrupt optional attachment metadata to force a real MongoDB BSON
        // read error without disrupting unrelated settlement/relay collections.
        state_metadata_fault(&f).await;
        assert!(
            engine::turn_tool_attachments(
                &f.state.db,
                OWNER,
                &row.id,
                row.active_turn.as_ref().unwrap().turn_id.as_str()
            )
            .await
            .is_err()
        );
    }
    let mut state = f.state.clone();
    state.per_conversation_initiate_limiter =
        std::sync::Arc::new(crate::mw::rate_limit::PerChannelEventLimiter::new(0, 1));
    if stale_run {
        let mut earlier = state
            .db
            .collection::<TriggerRun>(crate::models::trigger_run::COLLECTION_NAME)
            .find_one(doc! {"_id": &run_id})
            .await
            .unwrap()
            .unwrap();
        earlier.image_refs.clear();
        crate::handlers::trigger_scheduler::settle_result(
            &state,
            &row,
            &earlier,
            "Useful answer",
            None,
        )
        .await
        .unwrap();
    } else {
        crate::handlers::trigger_scheduler::settled(&state, &row, &run_id, "Useful answer", None)
            .await;
    }
    crate::handlers::trigger_scheduler::settled(&state, &row, &run_id, "Useful answer", None).await;
    let run = state
        .db
        .collection::<TriggerRun>(crate::models::trigger_run::COLLECTION_NAME)
        .find_one(doc! {"_id":&run_id})
        .await
        .unwrap()
        .unwrap();
    assert!(run.result_claimed);
    assert_eq!(run.outcome, RunOutcome::Completed);
    let requests = server.received_requests().await.unwrap();
    assert_eq!(
        requests
            .iter()
            .filter(|r| r.url.path().ends_with("/images"))
            .count(),
        if metadata_error { 0 } else { 2 }
    );
    assert_eq!(
        requests
            .iter()
            .filter(|r| r.url.path().ends_with("/messages"))
            .count(),
        if metadata_error { 1 } else { 2 }
    );
    let text = String::from_utf8_lossy(&requests.last().unwrap().body);
    assert!(text.contains("Useful answer"));
    assert_eq!(text.contains("1 of 2 image(s)"), !metadata_error);
    assert!(text.contains("automation's thread"));
    if metadata_error {
        assert!(text.contains("2 image(s)"));
    }
    assert!(matches!(
        crate::handlers::channel_relay::check_initiate_rate_limit(&state, &chat.id).await,
        Err(AppError::RateLimited)
    ));
    if !metadata_error {
        assert!(claim(&f).await.items.iter().all(|item| !matches!(
            item.state,
            crate::models::nyxbot_image_delivery::ImageDeliveryState::Pending
                | crate::models::nyxbot_image_delivery::ImageDeliveryState::Sending
        )));
    }
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn nyxbot_home_event_also_delivers_images_to_each_chat_and_errors_only_apologize() {
    for failed in [false, true] {
        let (f, server) = image_fixture("telegram", "direct").await;
        add_images(&f).await;
        mock_sends(&server, "telegram", false, std::time::Duration::ZERO).await;
        let primary = f.event.delivery.as_ref().unwrap().origin.clone();
        let mut extra = primary.clone();
        extra.partition = "second".into();
        let mut other = thread(&f).await;
        other.id = Uuid::new_v4().to_string();
        other.partition = extra.partition.clone();
        f.state
            .db
            .collection::<NyxbotThread>(THREADS)
            .insert_one(other)
            .await
            .unwrap();
        f.state.db.collection::<bson::Document>(CONVERSATIONS).update_one(doc! {"_id":&f.event.conversation_id},doc! {"$set":{
            "channel":bson::Bson::Null,"reply_channel":bson::to_bson(&primary).unwrap(),"deliver_also":[bson::to_bson(&extra).unwrap()],
            "active_turn":{"turn_id":&f.event.turn_id,"origin":"event","started_at":bson::DateTime::now(),"stop_requested":false},
        }}).await.unwrap();
        let row = engine::get(
            &f.state.db,
            OWNER,
            f.event.conversation_id.as_deref().unwrap(),
        )
        .await
        .unwrap();
        let error = engine::TurnError::internal("upstream_failed", "upstream");
        crate::handlers::assistant_team::after_turn_boxed(
            f.state.clone(),
            row.clone(),
            "Useful answer".into(),
            failed.then_some(error),
        )
        .await;
        let requests = server.received_requests().await.unwrap();
        let paths: Vec<_> = requests.iter().map(|r| r.url.path()).collect();
        if failed {
            assert_eq!(paths, ["/bottoken/sendMessage"]);
            assert!(String::from_utf8_lossy(&requests[0].body).contains("Please ask again"));
        } else {
            assert_eq!(
                paths,
                [
                    "/bottoken/sendPhoto",
                    "/bottoken/sendPhoto",
                    "/bottoken/sendMessage",
                    "/bottoken/sendMessage"
                ]
            );
            let count = f
                .state
                .db
                .collection::<NyxbotImageDelivery>(
                    crate::models::nyxbot_image_delivery::COLLECTION_NAME,
                )
                .count_documents(doc! {"conversation_id":&f.event.conversation_id})
                .await
                .unwrap();
            assert_eq!(count, 2);
        }
        assert!(
            engine::take_deliveries(&f.state.db, OWNER, &row.id)
                .await
                .unwrap()
                .is_empty()
        );
        f.state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn nyxbot_telegram_document_fallback_counts_as_sent_and_failed_documents_get_note() {
    for failed in [false, true] {
        let (f, server) = image_fixture("telegram", "direct").await;
        add_images(&f).await;
        Mock::given(path("/bottoken/sendPhoto"))
            .respond_with(ResponseTemplate::new(400).set_body_json(
                json!({"ok":false,"error_code":400,"description":"PHOTO_INVALID_DIMENSIONS"}),
            ))
            .expect(2)
            .mount(&server)
            .await;
        Mock::given(path("/bottoken/sendDocument"))
            .respond_with(if failed {
                ResponseTemplate::new(400).set_body_json(
                    json!({"ok":false,"error_code":400,"description":"invalid document"}),
                )
            } else {
                ResponseTemplate::new(200)
                    .set_body_json(json!({"ok":true,"result":{"message_id":42}}))
            })
            .expect(2)
            .mount(&server)
            .await;
        Mock::given(path("/bottoken/sendMessage"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"ok":true,"result":{"message_id":43}})),
            )
            .expect(1)
            .mount(&server)
            .await;
        delivery::detach(&f.state.db, &f.event.id).await.unwrap();
        late_delivery::process(&f.state, &f.event.id).await;
        let claim = claim(&f).await;
        assert!(claim.items.iter().all(|item| !matches!(
            item.state,
            crate::models::nyxbot_image_delivery::ImageDeliveryState::Pending
                | crate::models::nyxbot_image_delivery::ImageDeliveryState::Sending
        )));
        assert!(
            claim
                .items
                .iter()
                .all(|i| i.state.as_str() == if failed { "failed" } else { "sent" })
        );
        assert!(claim.items.iter().all(|i| i.code.as_deref()
            == Some(if failed {
                "platform_error"
            } else {
                "as_document"
            })));
        let requests = server.received_requests().await.unwrap();
        let text = String::from_utf8_lossy(&requests.last().unwrap().body);
        assert_eq!(text.contains("The 2 image(s)"), failed);
        if !failed {
            let files = f
                .state
                .db
                .collection::<bson::Document>(crate::models::channel_message::COLLECTION_NAME)
                .count_documents(doc! {"direction":"outbound","content_type":"file"})
                .await
                .unwrap();
            assert_eq!(files, 2);
        }
        server.verify().await;
        f.state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn nyxbot_attach_image_refuses_a_stopped_turn_without_persisting_bytes() {
    let (f, _server) = image_fixture("telegram", "direct").await;
    f.state.db.collection::<bson::Document>(CONVERSATIONS).update_one(doc! {"_id":&f.event.conversation_id},doc! {"$set":{"active_turn":{
        "turn_id":&f.event.turn_id,"origin":"channel","started_at":bson::DateTime::now(),"stop_requested":true,
    }}}).await.unwrap();
    assert!(
        engine::attach_routed_image(
            &f.state.db,
            &f.state.encryption_keys,
            OWNER,
            f.event.conversation_id.as_deref().unwrap(),
            "snapshot",
            "image/png",
            b"\x89PNG\r\n\x1a\npixels",
            true
        )
        .await
        .unwrap()
        .is_none()
    );
    assert_eq!(
        f.state
            .db
            .collection::<AssistantAttachment>(crate::models::assistant_attachment::COLLECTION_NAME)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn nyxbot_image_deadline_requires_a_useful_send_window() {
    let (f, server) = image_fixture("telegram", "direct").await;
    let attachments = add_images(&f).await;
    mock_sends(&server, "telegram", false, std::time::Duration::ZERO).await;
    let report = send_plan(
        &f,
        attachments.clone(),
        Some(std::time::Instant::now() + std::time::Duration::from_secs(24)),
    )
    .await;
    assert_eq!((report.sent, report.unknown, report.skipped), (0, 0, 2));
    assert!(server.received_requests().await.unwrap().is_empty());
    let claim = claim(&f).await;
    assert!(
        claim
            .items
            .iter()
            .all(|item| item.state.as_str() == "pending"
                && item.code.is_none()
                && item.attempt.is_none())
    );
    let later = send_plan(&f, attachments.clone(), None).await;
    assert_eq!(later.sent, 2);
    let repeated = send_plan(&f, attachments, None).await;
    assert_eq!(images::image_result(&repeated)["previously_delivered"], 2);
    assert_eq!(server.received_requests().await.unwrap().len(), 2);
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn nyxbot_unsupported_or_unanchored_images_record_refusals_without_loading_bytes() {
    for unsupported in [false, true] {
        let (mut f, server) = image_fixture("telegram", "direct").await;
        let attachments = add_images(&f).await;
        if unsupported {
            f.channel.platform = "openclaw".into();
        }
        f.state
            .db
            .run_command(doc! {"profile":2,"slowms":0})
            .await
            .unwrap();
        let thread = thread(&f).await;
        let report = images::send_turn_images(
            &f.state,
            ImageSendContext {
                owner: OWNER,
                conversation_id: f.event.conversation_id.as_deref().unwrap(),
                turn_id: f.event.turn_id.as_deref().unwrap(),
                channel: &f.channel,
                thread: &thread,
                anchor: ImageAnchor::NoAnchor,
                audience: ImageAudience {
                    owner_private: true,
                },
                deadline: None,

                initiated: None,
                fence: None,
            },
            &ImageSendPlan { attachments },
        )
        .await
        .unwrap();
        f.state.db.run_command(doc! {"profile":0}).await.unwrap();
        assert_eq!(f.state.db.collection::<bson::Document>("system.profile").count_documents(
            doc! {"command.find":crate::models::assistant_attachment::COLLECTION_NAME,"command.projection":{"$ne":{"_id":1}}}
        ).await.unwrap(),0,"refused destinations may read source identity but never load attachment bytes");
        assert_eq!((report.failed, report.unknown), (2, 0));
        let claim = claim(&f).await;
        assert!(
            claim
                .items
                .iter()
                .all(|item| item.state.as_str() == "failed"
                    && item.code.as_deref()
                        == Some(if unsupported {
                            "unsupported"
                        } else {
                            "no_anchor"
                        }))
        );
        assert!(server.received_requests().await.unwrap().is_empty());
        f.state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn nyxbot_machine_screenshots_stay_in_app_and_explicit_shares_reach_chat() {
    use crate::services::assistant_acknowledgement_service as acks;
    use base64::Engine as _;
    let (f, server) = image_fixture("telegram", "direct").await;
    f.state.db.collection::<bson::Document>(CONVERSATIONS).update_one(
        doc! {"_id":&f.event.conversation_id}, doc! {"$set":{"active_turn":{
            "turn_id":&f.event.turn_id,"origin":"channel","started_at":bson::DateTime::now(),"stop_requested":false,
            "asked_from":bson::to_bson(&f.event.delivery.as_ref().unwrap().origin).unwrap(),
        }}}).await.unwrap();
    let row = engine::get(
        &f.state.db,
        OWNER,
        f.event.conversation_id.as_deref().unwrap(),
    )
    .await
    .unwrap();
    let chat = acks::for_key(&f.state.db, OWNER, Some(&row.credential_api_key_id))
        .await
        .unwrap()
        .unwrap();
    let pixels = b"\x89PNG\r\n\x1a\nobservation";
    let mut result = json!({"content":[{"type":"image","mimeType":"image/png","data":base64::engine::general_purpose::STANDARD.encode(pixels)}]});
    crate::handlers::machine_tools::attach_computer_images(&f.state, &chat, &mut result)
        .await
        .unwrap();
    let note = result["content"][0]["text"].as_str().unwrap();
    assert!(note.contains("NyxID app only; it is not sent to the Telegram chat"));
    assert!(note.contains("Do not describe its contents before you have seen it"));
    let row = engine::get(&f.state.db, OWNER, &row.id).await.unwrap();
    let attachments = row.active_turn.unwrap().attachments;
    assert_eq!(attachments[0].chat_delivery, Some(false));
    assert_eq!(
        send_plan(&f, attachments, None).await,
        images::ImageReport {
            app_visible: true,
            ..Default::default()
        }
    );
    assert!(server.received_requests().await.unwrap().is_empty());
    mock_sends(&server, "telegram", false, std::time::Duration::ZERO).await;
    let shared =
        crate::handlers::machine_tools::attach_shared_image(&f.state, &chat, "image/png", pixels)
            .await
            .unwrap();
    assert_eq!(shared["attached"], true);
    let message = shared["message"].as_str().unwrap();
    assert!(message.contains("Telegram chat") && message.contains("before your reply text"));
    assert!(!message.contains("shown only to the owner"));
    let row = engine::get(&f.state.db, OWNER, &chat.conversation_id)
        .await
        .unwrap();
    let attachments = row.active_turn.unwrap().attachments;
    assert_eq!(attachments[1].chat_delivery, Some(true));
    assert_eq!(send_plan(&f, attachments, None).await.sent, 1);
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn nyxbot_legacy_screenshots_are_excluded_but_legacy_service_images_deliver() {
    let (f, server) = image_fixture("telegram", "direct").await;
    let mut attachments = add_images(&f).await;
    // Deserialize an old writer's metadata, which has no chat_delivery field.
    for attachment in &mut attachments {
        let mut legacy = serde_json::to_value(&*attachment).unwrap();
        legacy.as_object_mut().unwrap().remove("chat_delivery");
        *attachment = serde_json::from_value(legacy).unwrap();
    }
    attachments[0].label = "Machine screenshot".into();
    mock_sends(&server, "telegram", false, std::time::Duration::ZERO).await;
    assert_eq!(send_plan(&f, attachments, None).await.sent, 1);
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
    assert_eq!(claim(&f).await.items.len(), 1);
    f.state.db.drop().await.unwrap();
}

async fn state_metadata_fault(f: &Fixture) {
    f.state
        .db
        .collection::<bson::Document>(crate::models::assistant_message::COLLECTION_NAME)
        .update_one(
            doc! {"conversation_id": &f.event.conversation_id, "turn_id": &f.event.turn_id},
            doc! {"$set":{"attachments":"invalid stored attachment metadata"}},
        )
        .await
        .unwrap();
}

async fn live_image_authority(f: &Fixture, attachments: &[TurnAttachment]) -> acks::ChatAuthority {
    f.state
        .db
        .collection::<bson::Document>(CONVERSATIONS)
        .update_one(
            doc! {"_id":&f.event.conversation_id},
            doc! {"$set":{"guest_turn":false,"active_turn":{
                "turn_id":&f.event.turn_id,"origin":"user","started_at":bson::DateTime::now(),
                "stop_requested":false,"attachments":bson::to_bson(attachments).unwrap(),
            }}},
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
    acks::for_key(&f.state.db, OWNER, Some(&row.credential_api_key_id))
        .await
        .unwrap()
        .unwrap()
}

#[tokio::test]
async fn nyxbot_explicit_images_and_automatic_reply_share_the_resolved_audience() {
    for transport in ["direct", "gateway"] {
        let (f, server) = image_fixture("telegram", transport).await;
        let attachments = add_images(&f).await;
        let chat = live_image_authority(&f, &attachments).await;
        let target = thread(&f).await;
        mock_sends(&server, "telegram", false, std::time::Duration::ZERO).await;
        let first = chats::post_current_turn(&f.state, &chat, &target.id, "First post", true)
            .await
            .unwrap();
        assert_eq!(first["text"], "sent");
        assert_eq!(first["images"]["sent_now"], 2);
        let second = chats::post_current_turn(&f.state, &chat, &target.id, "Second post", true)
            .await
            .unwrap();
        assert_eq!(second["images"]["sent_now"], 0);
        assert_eq!(second["images"]["previously_delivered"], 2);
        let auto = send_plan(&f, attachments, None).await;
        assert_eq!(images::image_result(&auto)["previously_delivered"], 2);
        assert!(images::failure_note(&auto).is_empty());
        assert_eq!(
            server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .filter(|request| request.url.path().ends_with("sendPhoto"))
                .count(),
            2
        );
        f.state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn nyxbot_later_images_remain_deliverable_and_topics_are_separate_audiences() {
    let (f, server) = image_fixture("telegram", "direct").await;
    let first = add_images(&f).await;
    let chat = live_image_authority(&f, &first).await;
    let target = thread(&f).await;
    mock_sends(&server, "telegram", false, std::time::Duration::ZERO).await;
    let posted = chats::post_current_turn(&f.state, &chat, &target.id, "A", true)
        .await
        .unwrap();
    assert_eq!(posted["images"]["sent_now"], 2);
    let later = add_images(&f).await;
    let mut all = first.clone();
    all.extend(later.clone());
    let chat = live_image_authority(&f, &all).await;
    let posted = chats::post_current_turn(&f.state, &chat, &target.id, "B", true)
        .await
        .unwrap();
    assert_eq!(posted["images"]["sent_now"], 2);
    assert_eq!(posted["images"]["previously_delivered"], 2);
    for topic in ["0007", "8"] {
        let mut thread = target.clone();
        thread.id = Uuid::new_v4().to_string();
        thread.partition = thread.id.clone();
        thread.platform_thread_id = Some(topic.into());
        f.state
            .db
            .collection::<NyxbotThread>(THREADS)
            .insert_one(&thread)
            .await
            .unwrap();
        let result = chats::post_current_turn(&f.state, &chat, &thread.id, "Topic update", true)
            .await
            .unwrap();
        assert_eq!(result["images"]["sent_now"], 4);
    }
    let requests = server.received_requests().await.unwrap();
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.url.path().ends_with("sendPhoto"))
            .count(),
        12
    );
    let bodies: Vec<_> = requests
        .iter()
        .map(|request| String::from_utf8_lossy(&request.body))
        .collect();
    assert_eq!(
        bodies
            .iter()
            .filter(|body| body.contains("name=\"message_thread_id\"\r\n\r\n7"))
            .count(),
        4
    );
    assert_eq!(
        bodies
            .iter()
            .filter(|body| body.contains("name=\"message_thread_id\"\r\n\r\n8"))
            .count(),
        4
    );
    assert!(
        bodies
            .iter()
            .any(|body| body.contains("\"message_thread_id\":7"))
    );
    assert!(
        bodies
            .iter()
            .any(|body| body.contains("\"message_thread_id\":8"))
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn nyxbot_invalid_or_disallowed_explicit_posts_have_no_image_effect() {
    let (f, server) = image_fixture("telegram", "direct").await;
    let attachments = add_images(&f).await;
    let chat = live_image_authority(&f, &attachments).await;
    let target = thread(&f).await;
    assert!(
        chats::post_current_turn(&f.state, &chat, &target.id, " ", true)
            .await
            .is_err()
    );
    f.state
        .db
        .collection::<NyxbotThread>(THREADS)
        .update_one(doc! {"_id":&target.id}, doc! {"$set":{"allow_posts":false}})
        .await
        .unwrap();
    assert!(
        chats::post_current_turn(&f.state, &chat, &target.id, "Allowed text", true)
            .await
            .is_err()
    );
    assert!(server.received_requests().await.unwrap().is_empty());
    assert!(claim(&f).await.items.is_empty());
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn nyxbot_stop_during_an_explicit_image_cancels_the_remaining_images_and_text() {
    let (f, server) = image_fixture("telegram", "direct").await;
    let attachments = add_images(&f).await;
    let chat = live_image_authority(&f, &attachments).await;
    let target = thread(&f).await;
    let first = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let signal = first.clone();
    Mock::given(path("/bottoken/sendPhoto"))
        .respond_with(move |_: &wiremock::Request| {
            signal.store(true, std::sync::atomic::Ordering::SeqCst);
            ResponseTemplate::new(200)
                .set_delay(std::time::Duration::from_millis(200))
                .set_body_json(json!({"ok":true,"result":{"message_id":9}}))
        })
        .mount(&server)
        .await;
    let stop = async {
        tokio::time::timeout(std::time::Duration::from_secs(10), async {
            while !first.load(std::sync::atomic::Ordering::SeqCst) {
                tokio::time::sleep(std::time::Duration::from_millis(5)).await;
            }
        })
        .await
        .expect("explicit image dispatch must start");
        f.state
            .db
            .collection::<bson::Document>(CONVERSATIONS)
            .update_one(
                doc! {"_id":&chat.conversation_id},
                doc! {"$set":{"active_turn.stop_requested":true}},
            )
            .await
            .unwrap();
    };
    let post = chats::post_current_turn(&f.state, &chat, &target.id, "Never send this text", true);
    let (result, ()) = tokio::join!(post, stop);
    let result = result.unwrap();
    assert_eq!(result["text"], "cancelled");
    assert_eq!(result["images"]["sent_now"], 1);
    assert_eq!(result["images"]["cancelled"], 1);
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn nyxbot_overlapping_explicit_posts_send_each_source_once_and_keep_text_independent() {
    let (f, server) = image_fixture("lark", "direct").await;
    let attachments = add_images(&f).await;
    let chat = live_image_authority(&f, &attachments).await;
    let target = thread(&f).await;
    mock_sends(
        &server,
        "lark",
        false,
        std::time::Duration::from_millis(100),
    )
    .await;
    let (a, b) = tokio::join!(
        chats::post_current_turn(&f.state, &chat, &target.id, "A", true),
        chats::post_current_turn(&f.state, &chat, &target.id, "B", true),
    );
    let a = a.unwrap();
    let b = b.unwrap();
    assert_eq!(a["text"], "sent");
    assert_eq!(b["text"], "sent");
    assert_eq!(
        a["images"]["sent_now"].as_u64().unwrap() + b["images"]["sent_now"].as_u64().unwrap(),
        2
    );
    let requests = server.received_requests().await.unwrap();
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.url.path().ends_with("/images"))
            .count(),
        2
    );
    assert!(
        claim(&f)
            .await
            .items
            .iter()
            .all(|claim| claim.state
                == crate::models::nyxbot_image_delivery::ImageDeliveryState::Sent)
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn nyxbot_stop_or_replacement_during_last_image_prevents_final_text() {
    for replacement in [false, true] {
        let (f, server) = image_fixture("telegram", "direct").await;
        let attachments = add_images(&f).await;
        let chat = live_image_authority(&f, &attachments[..1]).await;
        let target = thread(&f).await;
        let first = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let signal = first.clone();
        Mock::given(path("/bottoken/sendPhoto"))
            .respond_with(move |_: &wiremock::Request| {
                signal.store(true, std::sync::atomic::Ordering::SeqCst);
                ResponseTemplate::new(200)
                    .set_delay(std::time::Duration::from_millis(200))
                    .set_body_json(json!({"ok":true,"result":{"message_id":9}}))
            })
            .mount(&server)
            .await;
        let stop = async {
            tokio::time::timeout(std::time::Duration::from_secs(10), async {
                while !first.load(std::sync::atomic::Ordering::SeqCst) {
                    tokio::time::sleep(std::time::Duration::from_millis(5)).await;
                }
            })
            .await
            .expect("image dispatch must start");
            let change = if replacement {
                doc! {"active_turn.turn_id":Uuid::new_v4().to_string()}
            } else {
                doc! {"active_turn.stop_requested":true}
            };
            f.state
                .db
                .collection::<bson::Document>(CONVERSATIONS)
                .update_one(doc! {"_id":&chat.conversation_id}, doc! {"$set":change})
                .await
                .unwrap();
        };
        let (result, ()) = tokio::join!(
            chats::post_current_turn(&f.state, &chat, &target.id, "Never sent", true),
            stop
        );
        let result = result.unwrap();
        assert_eq!(result["text"], "cancelled");
        assert_eq!(result["images"]["sent_now"], 1);
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
        f.state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn nyxbot_failed_text_does_not_retry_images_or_report_them_as_failed() {
    let (f, server) = image_fixture("telegram", "direct").await;
    let attachments = add_images(&f).await;
    let chat = live_image_authority(&f, &attachments).await;
    let target = thread(&f).await;
    Mock::given(path("/bottoken/sendPhoto"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"ok":true,"result":{"message_id":9}})),
        )
        .expect(2)
        .mount(&server)
        .await;
    Mock::given(path("/bottoken/sendMessage"))
        .respond_with(
            ResponseTemplate::new(400)
                .set_body_json(json!({"ok":false,"description":"chat not found"})),
        )
        .expect(2)
        .mount(&server)
        .await;
    let result = chats::post_current_turn(&f.state, &chat, &target.id, "Fails", true)
        .await
        .unwrap();
    assert_eq!(result["text"], "failed");
    assert_eq!(result["images"]["sent_now"], 2);
    let result = chats::post_current_turn(&f.state, &chat, &target.id, "Fails again", true)
        .await
        .unwrap();
    assert_eq!(result["text"], "failed");
    assert_eq!(result["images"]["previously_delivered"], 2);
    server.verify().await;
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn nyxbot_automation_terminal_refs_do_not_consume_the_eight_unsent_budget() {
    use crate::models::nyxbot_image_delivery::ImageDeliveryState;
    use crate::models::{
        trigger_run::{RunImageRef, TriggerRun},
        trigger_schedule::DeliverTo,
    };
    use crate::services::channel_image_delivery as claims;
    let (f, server) = image_fixture("lark", "direct").await;
    mock_sends(&server, "lark", false, std::time::Duration::ZERO).await;
    let target = thread(&f).await;
    let mut attachments = Vec::new();
    for _ in 0..8 {
        attachments.extend(add_images(&f).await);
    }
    let first_turn = f.event.turn_id.clone().unwrap();
    let later_turn = Uuid::new_v4().to_string();
    f.state
        .db
        .collection::<bson::Document>(crate::models::assistant_message::COLLECTION_NAME)
        .update_one(
            doc! {"conversation_id":&f.event.conversation_id,"turn_id":&first_turn},
            doc! {"$set":{"attachments":bson::to_bson(&attachments[..8]).unwrap()}},
        )
        .await
        .unwrap();
    f.state.db.collection::<bson::Document>(crate::models::assistant_message::COLLECTION_NAME)
        .insert_one(doc! {"_id":Uuid::new_v4().to_string(),"user_id":OWNER,"conversation_id":&f.event.conversation_id,
            "turn_id":&later_turn,"seq":2,"role":"assistant","text":"Continuation","status":"completed",
            "created_at":bson::DateTime::now(),"attachments":bson::to_bson(&attachments[8..]).unwrap()}).await.unwrap();
    let later_ids: Vec<_> = attachments[8..]
        .iter()
        .map(|item| item.id.as_str())
        .collect();
    f.state
        .db
        .collection::<bson::Document>(crate::models::assistant_attachment::COLLECTION_NAME)
        .update_many(
            doc! {"_id":{"$in":later_ids}},
            doc! {"$set":{"turn_id":&later_turn}},
        )
        .await
        .unwrap();
    let refs: Vec<_> = attachments
        .iter()
        .enumerate()
        .map(|(index, attachment)| RunImageRef {
            conversation_id: f.event.conversation_id.clone().unwrap(),
            turn_id: if index < 8 {
                first_turn.clone()
            } else {
                later_turn.clone()
            },
            attachment: attachment.clone(),
        })
        .collect();
    let run: TriggerRun = bson::from_document(doc! {
        "_id":Uuid::new_v4().to_string(),"trigger_id":"trigger","user_id":OWNER,
        "scheduled_at":bson::DateTime::now(),"deadline":bson::DateTime::now(),"outcome":"started",
        "thread_id":&f.event.conversation_id,"turn_id":&later_turn,
        "deliver_to":bson::to_bson(&DeliverTo::Chat{chat_id:target.id.clone()}).unwrap(),
        "image_refs":bson::to_bson(&refs).unwrap(),"image_refs_truncated":3,
        "fence":"fence","lease_until":bson::DateTime::now(),"expires_at":bson::DateTime::now(),
    })
    .unwrap();
    let audience = format!("{}\u{1f}123\u{1f}", f.channel.channel_bot_id);
    // Newest refs are already terminal; all eight older pending refs still
    // have capacity for their first attempt in this delivery.
    let legacy_items: Vec<_> = attachments[8..12]
        .iter()
        .map(|attachment| {
            doc! {"id": &attachment.id, "state": "sent"}
        })
        .collect();
    f.state
        .db
        .collection::<bson::Document>(crate::models::nyxbot_image_delivery::LEGACY_COLLECTION_NAME)
        .insert_one(doc! {
            "_id": Uuid::new_v4().to_string(), "user_id": OWNER,
            "conversation_id": &run.thread_id, "turn_id": &later_turn,
            "channel_id": &f.channel.id, "destination_key": "initiated:legacy-chat",
            "state": "done", "claim_id": "legacy-attempt", "items": legacy_items,
            "created_at": bson::DateTime::now(),
            "expires_at": bson::DateTime::from_chrono(Utc::now() + ChronoDuration::days(7)),
        })
        .await
        .unwrap();
    for attachment in &attachments[12..] {
        let claims::ImageClaim::Dispatch { id, attempt } = claims::claim(
            &f.state.db,
            OWNER,
            run.thread_id.as_deref().unwrap(),
            &attachment.id,
            &audience,
        )
        .await
        .unwrap() else {
            panic!("new claim")
        };
        claims::finish(&f.state.db, &id, &attempt, ImageDeliveryState::Sent, None)
            .await
            .unwrap();
    }
    let result = chats::post_automation_images(
        &f.state,
        &run,
        &target.id,
        None,
        run.thread_id.as_deref().unwrap(),
        "Automation result",
    )
    .await
    .unwrap();
    assert_eq!(result["images"]["sent_now"], 8);
    assert_eq!(result["images"]["previously_delivered"], 8);
    let requests = server.received_requests().await.unwrap();
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.url.path().ends_with("/images"))
            .count(),
        8
    );
    let text = String::from_utf8_lossy(&requests.last().unwrap().body);
    assert!(text.contains("3 additional images were omitted from this delivery."));
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn nyxbot_deleting_a_conversation_removes_every_image_audience_claim() {
    let (f, server) = image_fixture("telegram", "direct").await;
    let attachments = add_images(&f).await;
    mock_sends(&server, "telegram", false, std::time::Duration::ZERO).await;
    send_plan(&f, attachments, None).await;
    let rows = claim(&f).await.items;
    assert_eq!(rows.len(), 2);
    for row in &rows {
        crate::services::channel_image_delivery::claim(
            &f.state.db,
            OWNER,
            &row.conversation_id,
            &row.attachment_id,
            "second-audience",
        )
        .await
        .unwrap();
    }
    let legacy = f
        .state
        .db
        .collection::<bson::Document>(crate::models::nyxbot_image_delivery::LEGACY_COLLECTION_NAME);
    legacy
        .insert_one(doc! {
            "_id": "legacy", "user_id": OWNER, "conversation_id": &f.event.conversation_id,
            "turn_id": &f.event.turn_id, "channel_id": &f.channel.id,
        })
        .await
        .unwrap();
    engine::delete(
        &f.state.db,
        OWNER,
        f.event.conversation_id.as_deref().unwrap(),
    )
    .await
    .unwrap();
    assert!(claim(&f).await.items.is_empty());
    assert_eq!(legacy.count_documents(doc! {}).await.unwrap(), 0);
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn nyxbot_slack_image_payloads_bind_top_level_and_two_native_roots_across_paths() {
    use crate::models::{
        trigger_run::{RunImageRef, TriggerRun},
        trigger_schedule::DeliverTo,
    };
    let (f, server) = image_fixture("slack", "direct").await;
    let attachments = add_images(&f).await;
    let chat = live_image_authority(&f, &attachments).await;
    Mock::given(path("/files.getUploadURLExternal"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"ok":true,"file_id":"F1","upload_url":format!("{}/upload",server.uri())}),
        ))
        .mount(&server)
        .await;
    Mock::given(path("/upload"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    Mock::given(path("/files.completeUploadExternal"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"ok":true})))
        .mount(&server)
        .await;
    Mock::given(path("/files.info"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"ok":true,"file":{"shares":{"private":{"123":[{"ts":"receipt"}]}}}}),
        ))
        .mount(&server)
        .await;
    Mock::given(path("/chat.postMessage"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"ok":true,"ts":"text-receipt"})),
        )
        .mount(&server)
        .await;
    let target = thread(&f).await;
    let mut root_two = None;
    for root in [None, Some("17"), Some("root-two")] {
        let mut target = target.clone();
        target.id = Uuid::new_v4().to_string();
        target.partition = target.id.clone();
        target.platform_thread_id = root.map(str::to_owned);
        f.state
            .db
            .collection::<NyxbotThread>(THREADS)
            .insert_one(&target)
            .await
            .unwrap();
        let result = chats::post_current_turn(&f.state, &chat, &target.id, "Explicit", true)
            .await
            .unwrap();
        assert_eq!(
            result["images"]["sent_now"], 2,
            "the explicit post sends both images"
        );
        if root == Some("root-two") {
            root_two = Some(target);
        }
    }
    // Slack's automatic reply to message 17 is the same real audience as
    // an initiated post explicitly addressed to root 17.
    let automatic = send_plan(&f, attachments.clone(), None).await;
    assert_eq!(images::image_result(&automatic)["previously_delivered"], 2);
    let target = root_two.unwrap();
    let refs: Vec<_> = attachments
        .iter()
        .map(|attachment| RunImageRef {
            conversation_id: f.event.conversation_id.clone().unwrap(),
            turn_id: f.event.turn_id.clone().unwrap(),
            attachment: attachment.clone(),
        })
        .collect();
    let run:TriggerRun=bson::from_document(doc! {
        "_id":Uuid::new_v4().to_string(),"trigger_id":"trigger","user_id":OWNER,
        "scheduled_at":bson::DateTime::now(),"deadline":bson::DateTime::now(),"outcome":"started",
        "thread_id":&f.event.conversation_id,"turn_id":&f.event.turn_id,
        "deliver_to":bson::to_bson(&DeliverTo::Chat{chat_id:target.id.clone()}).unwrap(),"image_refs":bson::to_bson(&refs).unwrap(),
        "fence":"fence","lease_until":bson::DateTime::now(),"expires_at":bson::DateTime::now(),
    }).unwrap();
    let result = chats::post_automation_images(
        &f.state,
        &run,
        &target.id,
        None,
        run.thread_id.as_deref().unwrap(),
        "Automation",
    )
    .await
    .unwrap();
    assert_eq!(result["images"]["previously_delivered"], 2);
    let requests = server.received_requests().await.unwrap();
    let payloads: Vec<serde_json::Value> = requests
        .iter()
        .filter(|request| request.url.path().ends_with("files.completeUploadExternal"))
        .map(|request| serde_json::from_slice(&request.body).unwrap())
        .collect();
    assert_eq!(payloads.len(), 6);
    for root in [None, Some("17"), Some("root-two")] {
        assert_eq!(
            payloads
                .iter()
                .filter(|body| body["thread_ts"].as_str() == root && body["channel_id"] == "123")
                .count(),
            2
        );
    }
    let text: Vec<serde_json::Value> = requests
        .iter()
        .filter(|request| request.url.path().ends_with("chat.postMessage"))
        .map(|request| serde_json::from_slice(&request.body).unwrap())
        .collect();
    assert_eq!(text.last().unwrap()["thread_ts"], "root-two");
    f.state.db.drop().await.unwrap();
}

/// Telegram sets `message_thread_id` on replies inside plain supergroups too
/// (the reply-chain root). Inbound thread keys stay intact; outbound topic
/// evidence makes an automatic reply and a later explicit post share the chat
/// audience and one claim per image. A real topic has its own audience.
#[tokio::test]
async fn nyxbot_telegram_reply_chain_replies_and_explicit_posts_share_the_chat_audience_while_topics_stay_separate()
 {
    let (f, server) = image_fixture("telegram", "direct").await;
    let attachments = add_images(&f).await;
    mock_sends(&server, "telegram", false, std::time::Duration::ZERO).await;
    let adapter = crate::services::channel_adapters::telegram::TelegramAdapter::new();
    let parse = |message: serde_json::Value| {
        let adapter = &adapter;
        async move {
            adapter
                .parse_inbound(
                    &serde_json::to_vec(&json!({"update_id": 1, "message": message})).unwrap(),
                )
                .await
                .unwrap()
                .remove(0)
        }
    };
    let original = f.event.relay_message_id.clone().unwrap();
    let messages = f
        .state
        .db
        .collection::<bson::Document>(crate::models::channel_message::COLLECTION_NAME);
    // The stored inbound row is what ingestion keeps for a reply in a plain
    // supergroup: the reply-chain root arrives as `message_thread_id` without
    // `is_topic_message`: keep the inbound key and separately record chat-level outbound evidence.
    let chain = parse(json!({
        "message_id": 17, "message_thread_id": 16,
        "from": {"id": 7, "is_bot": false, "first_name": "Owner"},
        "chat": {"id": 123, "type": "supergroup"}, "date": 1700000000, "text": "reply",
        "reply_to_message": {"message_id": 16, "chat": {"id": 123, "type": "supergroup"}, "date": 1699999999, "text": "root"},
    }))
    .await;
    assert_eq!(chain.thread_id.as_deref(), Some("16"));
    let bot = crate::services::channel_bot_service::get_bot(&f.state.db, &f.channel.channel_bot_id)
        .await
        .unwrap();
    let chain_facts = adapter.thread_facts(&chain, &bot, None).unwrap();
    messages
        .update_one(
            doc! {"_id": &original},
            doc! {"$set": {"thread_id": "16", "reply_to_platform_message_id": "16", "thread_context": bson::to_bson(&chain_facts).unwrap()}},
        )
        .await
        .unwrap();
    f.state
        .db
        .collection::<bson::Document>(THREADS)
        .update_one(
            doc! {"channel_id": &f.channel.id},
            doc! {"$set": {
                "platform_thread_id": "16", "telegram_topic": {"kind": "chat"},
            }},
        )
        .await
        .unwrap();
    let automatic = send_plan(&f, attachments.clone(), None).await;
    assert_eq!(images::image_result(&automatic)["sent_now"], 2);
    // The explicit post addresses the chat itself: same audience, nothing resent.
    let chat = live_image_authority(&f, &attachments).await;
    let target = thread(&f).await;
    assert_eq!(target.platform_thread_id.as_deref(), Some("16"));
    let posted = chats::post_current_turn(&f.state, &chat, &target.id, "Posted", true)
        .await
        .unwrap();
    assert_eq!(
        posted["images"]["sent_now"], 0,
        "a post after automatic delivery sends nothing new"
    );
    assert_eq!(posted["images"]["previously_delivered"], 2);
    // The same chain answered automatically again resends nothing either.
    let again = send_plan(&f, attachments.clone(), None).await;
    assert_eq!(images::image_result(&again)["previously_delivered"], 2);
    assert!(images::failure_note(&again).is_empty());
    // An actual forum topic is recorded and is its own audience, both ways.
    let topic = parse(json!({
        "message_id": 18, "message_thread_id": 7, "is_topic_message": true,
        "from": {"id": 7, "is_bot": false, "first_name": "Owner"},
        "chat": {"id": 123, "type": "supergroup", "is_forum": true}, "date": 1700000001, "text": "in topic",
        "reply_to_message": {"message_id": 7, "chat": {"id": 123, "type": "supergroup"}, "date": 1699999998, "text": "topic root"},
    }))
    .await;
    assert_eq!(topic.thread_id.as_deref(), Some("7"));
    let topic_facts = adapter.thread_facts(&topic, &bot, None).unwrap();
    messages
        .update_one(
            doc! {"_id": &original},
            doc! {"$set": {"thread_id": "7", "reply_to_platform_message_id": "7", "thread_context": bson::to_bson(&topic_facts).unwrap()}},
        )
        .await
        .unwrap();
    let in_topic = send_plan(&f, attachments.clone(), None).await;
    assert_eq!(images::image_result(&in_topic)["sent_now"], 2);
    let mut topic_thread = target.clone();
    topic_thread.id = Uuid::new_v4().to_string();
    topic_thread.partition = topic_thread.id.clone();
    topic_thread.platform_thread_id = Some("7".into());
    topic_thread.telegram_topic =
        Some(crate::models::channel_thread::TelegramTopic::Topic { id: "7".into() });
    f.state
        .db
        .collection::<NyxbotThread>(THREADS)
        .insert_one(&topic_thread)
        .await
        .unwrap();
    let posted_in_topic =
        chats::post_current_turn(&f.state, &chat, &topic_thread.id, "Topic post", true)
            .await
            .unwrap();
    assert_eq!(
        posted_in_topic["images"]["sent_now"], 0,
        "a topic post after delivery sends nothing new"
    );
    assert_eq!(posted_in_topic["images"]["previously_delivered"], 2);
    // Provider payloads: two chat-level photos anchored on the reply chain
    // without a topic, two topic photos; four in total for two images.
    let requests = server.received_requests().await.unwrap();
    let photos: Vec<String> = requests
        .iter()
        .filter(|request| request.url.path().ends_with("sendPhoto"))
        .map(|request| String::from_utf8_lossy(&request.body).into_owned())
        .collect();
    assert_eq!(photos.len(), 4);
    let chain_photos = photos
        .iter()
        .filter(|body| {
            body.contains("name=\"reply_to_message_id\"\r\n\r\n17")
                && !body.contains("name=\"message_thread_id\"")
        })
        .count();
    let topic_photos = photos
        .iter()
        .filter(|body| body.contains("name=\"message_thread_id\"\r\n\r\n7"))
        .count();
    assert_eq!((chain_photos, topic_photos), (2, 2), "{photos:?}");
    let claims = claim(&f).await;
    assert_eq!(claims.items.len(), 4);
    assert!(
        claims
            .items
            .iter()
            .all(|item| item.state.as_str() == "sent")
    );
    assert_eq!(
        claims
            .items
            .iter()
            .filter(|item| item.audience_key.ends_with("\u{1f}"))
            .count(),
        2
    );
    assert_eq!(
        claims
            .items
            .iter()
            .filter(|item| item.audience_key.ends_with("\u{1f}7"))
            .count(),
        2
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn nyxbot_automation_delivers_later_turn_images_without_reposting_explicit_images() {
    use crate::models::{
        trigger_run::{RunImageRef, TriggerRun},
        trigger_schedule::DeliverTo,
    };
    let (mut f, server) = image_fixture("lark", "direct").await;
    mock_sends(&server, "lark", false, std::time::Duration::ZERO).await;
    let target = thread(&f).await;
    let first = add_images(&f).await;
    let first_turn = f.event.turn_id.clone().unwrap();
    let chat = live_image_authority(&f, &first).await;
    let posted = chats::post_current_turn(&f.state, &chat, &target.id, "First turn", true)
        .await
        .unwrap();
    assert_eq!(posted["images"]["sent_now"], 2);
    f.event.turn_id = Some(Uuid::new_v4().to_string());
    f.state
        .db
        .collection::<bson::Document>(crate::models::assistant_message::COLLECTION_NAME)
        .insert_one(doc! {
            "_id": Uuid::new_v4().to_string(),
            "user_id": OWNER,
            "conversation_id": &f.event.conversation_id,
            "turn_id": &f.event.turn_id,
            "seq": 2,
            "role": "assistant",
            "text": "Later turn answer",
            "status": "completed",
            "created_at": bson::DateTime::now(),
        })
        .await
        .unwrap();
    let mut later = add_images(&f).await;
    later[1].chat_delivery = Some(false);
    f.state
        .db
        .collection::<bson::Document>(crate::models::assistant_message::COLLECTION_NAME)
        .update_one(
            doc! {"conversation_id":&f.event.conversation_id,"turn_id":&f.event.turn_id},
            doc! {"$set":{"attachments":bson::to_bson(&later).unwrap()}},
        )
        .await
        .unwrap();
    let refs: Vec<_> = first
        .iter()
        .map(|attachment| RunImageRef {
            conversation_id: f.event.conversation_id.clone().unwrap(),
            turn_id: first_turn.clone(),
            attachment: attachment.clone(),
        })
        .chain(
            later
                .iter()
                .filter(|attachment| engine::chat_tool_image(attachment))
                .map(|attachment| RunImageRef {
                    conversation_id: f.event.conversation_id.clone().unwrap(),
                    turn_id: f.event.turn_id.clone().unwrap(),
                    attachment: attachment.clone(),
                }),
        )
        .collect();
    let run: TriggerRun = bson::from_document(doc! {
        "_id":Uuid::new_v4().to_string(),"trigger_id":"trigger","user_id":OWNER,
        "scheduled_at":bson::DateTime::now(),"deadline":bson::DateTime::now(),"outcome":"started",
        "thread_id":&f.event.conversation_id,"turn_id":&f.event.turn_id,
        "deliver_to":bson::to_bson(&DeliverTo::Chat{chat_id:target.id.clone()}).unwrap(),"image_refs":bson::to_bson(&refs).unwrap(),
        "fence":"fence","lease_until":bson::DateTime::now(),"expires_at":bson::DateTime::now(),
    }).unwrap();
    let result = chats::post_automation_images(
        &f.state,
        &run,
        &target.id,
        None,
        run.thread_id.as_deref().unwrap(),
        "Automation result",
    )
    .await
    .unwrap();
    assert_eq!(result["images"]["sent_now"], 1);
    assert_eq!(result["images"]["previously_delivered"], 2);
    let requests = server.received_requests().await.unwrap();
    assert_eq!(
        requests
            .iter()
            .filter(|r| r.url.path().ends_with("/images"))
            .count(),
        3
    );
    let text = String::from_utf8_lossy(&requests.last().unwrap().body);
    assert!(text.contains("Automation result"));
    assert!(!text.contains("could not be sent"));
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn nyxbot_cancelled_image_post_releases_claims_for_automation_result() {
    use crate::models::{
        trigger_run::{RunImageRef, TriggerRun},
        trigger_schedule::DeliverTo,
    };
    let (f, server) = image_fixture("telegram", "direct").await;
    let attachments = add_images(&f).await;
    let target = thread(&f).await;
    let prepared = chats::prepare_post(&f.state, OWNER, &target.id, None)
        .await
        .unwrap();
    let chat = live_image_authority(&f, &attachments).await;
    let first = images::send_turn_images_with_stop(
        &f.state,
        ImageSendContext {
            owner: OWNER,
            conversation_id: f.event.conversation_id.as_deref().unwrap(),
            turn_id: f.event.turn_id.as_deref().unwrap(),
            channel: &f.channel,
            thread: &target,
            anchor: ImageAnchor::Initiated {
                chat_id: target.id.clone(),
            },
            audience: ImageAudience {
                owner_private: true,
            },
            deadline: None,
            initiated: Some(&prepared),
            fence: Some(&chat),
        },
        &ImageSendPlan {
            attachments: attachments.clone(),
        },
        Some("cancelled"),
    )
    .await
    .unwrap();
    assert_eq!(first.claimed, 2);
    assert!(first.outcomes.iter().all(|item| item.status == "cancelled"));
    assert!(server.received_requests().await.unwrap().is_empty());
    assert!(
        claim(&f)
            .await
            .items
            .iter()
            .all(|item| item.state.as_str() == "pending")
    );
    let refs: Vec<_> = attachments
        .into_iter()
        .map(|attachment| RunImageRef {
            conversation_id: f.event.conversation_id.clone().unwrap(),
            turn_id: f.event.turn_id.clone().unwrap(),
            attachment,
        })
        .collect();
    let run: TriggerRun = bson::from_document(doc! {
        "_id": Uuid::new_v4().to_string(), "trigger_id": "trigger", "user_id": OWNER,
        "scheduled_at": bson::DateTime::now(), "deadline": bson::DateTime::now(), "outcome": "started",
        "thread_id": &f.event.conversation_id, "turn_id": &f.event.turn_id,
        "deliver_to": bson::to_bson(&DeliverTo::Chat { chat_id: target.id.clone() }).unwrap(),
        "image_refs": bson::to_bson(&refs).unwrap(), "fence": "fence",
        "lease_until": bson::DateTime::now(), "expires_at": bson::DateTime::now(),
    }).unwrap();
    mock_sends(&server, "telegram", false, std::time::Duration::ZERO).await;
    let result = chats::post_automation_images(
        &f.state,
        &run,
        &target.id,
        None,
        run.thread_id.as_deref().unwrap(),
        "Automation result",
    )
    .await
    .unwrap();
    assert_eq!(result["images"]["sent_now"], 2);
    let repeated = chats::post_automation_images(
        &f.state,
        &run,
        &target.id,
        None,
        run.thread_id.as_deref().unwrap(),
        "Repeated result",
    )
    .await
    .unwrap();
    assert_eq!(repeated["images"]["previously_delivered"], 2);
    assert_eq!(
        server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .filter(|r| r.url.path().ends_with("sendPhoto"))
            .count(),
        2
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn external_bot_delayed_images_keep_guest_authority_and_one_delivery_per_image() {
    let (f, server) = image_fixture("telegram", "direct").await;
    add_images(&f).await;
    mock_sends(&server, "telegram", false, std::time::Duration::ZERO).await;
    f.state.db.collection::<bson::Document>(THREADS).update_one(doc! {"channel_id":&f.channel.id},doc! {"$set":{"kind":"group","members":"everyone","owner_chat":false,"owner_seen":false}}).await.unwrap();
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
    late_delivery::process(&f.state, &f.event.id).await;
    late_delivery::process(&f.state, &f.event.id).await;
    assert_eq!(status(&f).await, "sent");
    assert_eq!(
        server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .filter(|r| r.url.path().ends_with("sendPhoto"))
            .count(),
        2
    );
    assert_eq!(
        claim(&f)
            .await
            .items
            .iter()
            .filter(|c| c.state.as_str() == "sent")
            .count(),
        2
    );
    f.state.db.drop().await.unwrap();
}

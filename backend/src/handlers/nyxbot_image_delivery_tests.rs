//! Exercise durable settlement paths with real relay adapters and Mongo claims.
use super::*;
use crate::handlers::nyxbot::images::{
    self, ImageAnchor, ImageAudience, ImageSendContext, ImageSendPlan,
};
use crate::models::{
    assistant_attachment::AssistantAttachment, assistant_conversation::TurnAttachment,
    nyxbot_image_delivery::NyxbotImageDelivery,
};
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

async fn claim(f: &Fixture) -> NyxbotImageDelivery {
    f.state
        .db
        .collection::<NyxbotImageDelivery>(crate::models::nyxbot_image_delivery::COLLECTION_NAME)
        .find_one(doc! {"turn_id":&f.event.turn_id})
        .await
        .unwrap()
        .unwrap()
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
    let claim_key = format!("inbound:{}", f.event.relay_message_id.as_deref().unwrap());
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
            claim_key: &claim_key,
            initiated: None,
        },
        &ImageSendPlan { attachments },
    )
    .await
    .unwrap()
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
        assert_eq!(claim.state, "done");
        assert!(claim.items.iter().all(|i| i.state == "sent"));
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
            .filter(|i| i.state == "sent")
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
        assert_eq!(claim.items[0].state, "sent");
        assert_eq!(claim.items[1].state, "failed");
        assert_eq!(
            claim.items[1].code.as_deref(),
            Some(if expired {
                "unavailable"
            } else {
                "platform_error"
            })
        );
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
    let again = send_plan(&f, images, None).await;
    assert_eq!((again.unknown, again.skipped), (1, 1));
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
    let claim = claim(&f).await;
    assert_eq!(claim.items[0].state, "unknown");
    assert_eq!(claim.items[1].code.as_deref(), Some("after_unknown"));
    delivery::detach(&f.state.db, &f.event.id).await.unwrap();
    late_delivery::process(&f.state, &f.event.id).await;
    late_delivery::process(&f.state, &f.event.id).await;
    let requests = server.received_requests().await.unwrap();
    assert_eq!(
        requests
            .iter()
            .filter(|r| r.url.path().ends_with("sendPhoto"))
            .count(),
        1
    );
    assert_eq!(
        requests
            .iter()
            .filter(|r| r.url.path().ends_with("sendMessage"))
            .count(),
        1
    );
    assert!(String::from_utf8_lossy(&requests.last().unwrap().body).contains("The 2 image(s)"));
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
    assert_eq!(claim(&f).await.state, "done");
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
    assert_eq!(row.items[0].state, "failed");
    assert_eq!(row.items[1].state, "skipped");
    assert!(
        row.items
            .iter()
            .all(|item| item.code.as_deref() == Some("billing"))
    );
    let bot = crate::services::channel_bot_service::get_bot(&f.state.db, &f.channel.channel_bot_id)
        .await
        .unwrap();
    assert_eq!(bot.status, "failed");
    assert_eq!(send_plan(&f, attachments, None).await, report);
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
    crate::handlers::assistant_team::after_turn(&f.state, &row, "Useful answer", None).await;
    let requests = server.received_requests().await.unwrap();
    assert_eq!(
        requests
            .iter()
            .filter(|r| r.url.path().ends_with("sendPhoto"))
            .count(),
        attachments.len()
    );
    assert!(requests.last().unwrap().url.path().ends_with("sendMessage"));
    assert_eq!(claim(&f).await.state, "done");
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
    automation_result(false, false).await;
}

#[tokio::test]
async fn nyxbot_trigger_scheduler_settles_group_bound_run() {
    automation_result(true, false).await;
}

#[tokio::test]
async fn nyxbot_trigger_scheduler_image_metadata_error_still_posts_text_and_finishes() {
    automation_result(false, true).await;
}

async fn automation_result(group_bound: bool, metadata_error: bool) {
    use crate::models::trigger_run::{RunOutcome, TriggerRun};
    use crate::models::trigger_schedule::DeliverTo;
    let (f, server) = image_fixture("lark", "direct").await;
    add_images(&f).await;
    mock_sends(&server, "lark", true, std::time::Duration::ZERO).await;
    let chat = thread(&f).await;
    let run_id = Uuid::new_v4().to_string();
    f.state.db.collection::<bson::Document>(crate::models::trigger_run::COLLECTION_NAME).insert_one(doc! {
        "_id":&run_id,"trigger_id":Uuid::new_v4().to_string(),"user_id":OWNER,"scheduled_at":bson::DateTime::now(),
        "deadline":bson::DateTime::from_chrono(Utc::now()+ChronoDuration::hours(1)),"outcome":"started",
        "thread_id":&f.event.conversation_id,"turn_id":&f.event.turn_id,"label":"Image automation","fence":"test",
        "lease_until":bson::DateTime::now(),"expires_at":bson::DateTime::from_chrono(Utc::now()+ChronoDuration::days(7)),
        "deliver_to":bson::to_bson(&DeliverTo::Chat {chat_id:chat.id.clone()}).unwrap(),
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
    crate::handlers::trigger_scheduler::settled(&state, &row, &run_id, "Useful answer", None).await;
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
    assert_eq!(text.contains("automation's thread"), !metadata_error);
    assert!(matches!(
        crate::handlers::channel_relay::check_initiate_rate_limit(&state, &chat.id).await,
        Err(AppError::RateLimited)
    ));
    if !metadata_error {
        assert_eq!(claim(&f).await.state, "done");
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
        crate::handlers::assistant_team::after_turn(
            &f.state,
            &row,
            "Useful answer",
            failed.then_some(&error),
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
                    "/bottoken/sendPhoto",
                    "/bottoken/sendPhoto",
                    "/bottoken/sendMessage"
                ]
            );
            let count = f
                .state
                .db
                .collection::<NyxbotImageDelivery>(
                    crate::models::nyxbot_image_delivery::COLLECTION_NAME,
                )
                .count_documents(doc! {"turn_id":&f.event.turn_id})
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
        assert_eq!(claim.state, "done");
        assert!(
            claim
                .items
                .iter()
                .all(|i| i.state == if failed { "failed" } else { "sent" })
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
        attachments,
        Some(std::time::Instant::now() + std::time::Duration::from_secs(24)),
    )
    .await;
    assert_eq!((report.sent, report.unknown, report.skipped), (0, 0, 2));
    assert!(server.received_requests().await.unwrap().is_empty());
    let claim = claim(&f).await;
    assert_eq!(claim.state, "done");
    assert!(
        claim
            .items
            .iter()
            .all(|item| item.state == "skipped" && item.code.as_deref() == Some("deadline"))
    );
    f.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn nyxbot_unsupported_or_unanchored_images_record_refusals_without_loading_bytes() {
    for unsupported in [false, true] {
        let (mut f, server) = image_fixture("telegram", "direct").await;
        let attachments = add_images(&f).await;
        // No readable attachment collection is needed for either refusal.
        f.state
            .db
            .collection::<bson::Document>(crate::models::assistant_attachment::COLLECTION_NAME)
            .drop()
            .await
            .unwrap();
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
                claim_key: "unavailable-destination",
                initiated: None,
            },
            &ImageSendPlan { attachments },
        )
        .await
        .unwrap();
        f.state.db.run_command(doc! {"profile":0}).await.unwrap();
        assert_eq!(f.state.db.collection::<bson::Document>("system.profile").count_documents(
            doc! {"ns":format!("{}.{}",f.state.db.name(),crate::models::assistant_attachment::COLLECTION_NAME),"op":"query"}
        ).await.unwrap(),0,"refused destinations must not load attachment bytes");
        assert_eq!((report.failed, report.unknown), (2, 0));
        let claim = claim(&f).await;
        assert!(claim.items.iter().all(|item| item.state == "failed"
            && item.code.as_deref()
                == Some(if unsupported {
                    "unsupported"
                } else {
                    "no_anchor"
                })));
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

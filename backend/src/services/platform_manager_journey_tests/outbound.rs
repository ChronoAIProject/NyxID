use super::*;
use crate::models::channel_text_delivery::{
    COLLECTION_NAME as TEXT, ChannelTextDelivery, TextPartState,
};

async fn long_reply(j: &Journey, id: i64) -> (ReplyDestination, ReplyRef, AssistantMessage) {
    let (destination, reply, mut saved) = j.prepared_reply(id).await;
    saved.text = (0..600)
        .map(|i| format!("🦉 complete paragraph {i}\n\n"))
        .collect();
    j.state
        .db
        .collection::<AssistantMessage>(MESSAGES)
        .replace_one(doc! {"_id":&saved.id}, &saved)
        .await
        .unwrap();
    (destination, reply, saved)
}

#[tokio::test]
async fn manager_outbound_delivers_full_ordered_text_and_never_replays_a_sent_part() {
    let j = Journey::new().await;
    j.link().await;
    let (destination, reply, saved) = long_reply(&j, 7001).await;
    let before = j.sends().await.len();
    let outcome = j
        .deliver(
            &destination,
            &reply,
            delivery::render_saved(&saved, &j.state.config.frontend_url),
        )
        .await
        .unwrap();
    let DeliveryOutcome::Sent {
        telegram_message_ids,
    } = outcome
    else {
        panic!("complete delivery required");
    };
    let sends = j.sends().await;
    let parts = &sends[before..];
    assert!(parts.len() > 2);
    assert_eq!(telegram_message_ids.len(), parts.len());
    assert_eq!(
        parts
            .iter()
            .map(|p| p["text"].as_str().unwrap())
            .collect::<String>()
            .trim_end(),
        saved.text.trim_end()
    );
    assert!(parts.iter().all(
        |p| p["text"].as_str().unwrap().encode_utf16().count() <= 4096
            && p.get("parse_mode").is_none()
            && p.get("entities").is_some()
    ));
    assert!(!parts.iter().any(|p| {
        p["text"]
            .as_str()
            .unwrap()
            .contains("continue this conversation")
    }));
    assert!(matches!(
        j.deliver(
            &destination,
            &reply,
            delivery::render_saved(&saved, &j.state.config.frontend_url)
        )
        .await
        .unwrap(),
        DeliveryOutcome::Duplicate
    ));
    assert_eq!(j.sends().await.len(), sends.len());
    let journal = j
        .state
        .db
        .collection::<ChannelTextDelivery>(TEXT)
        .find_one(doc! {"source_id":&saved.id})
        .await
        .unwrap()
        .unwrap();
    assert!(journal.parts.iter().all(|p| p.state == TextPartState::Sent));
    j.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn manager_outbound_429_resumes_after_the_prefix_but_ambiguous_parts_never_retry() {
    for uncertain in [false, true] {
        let j = Journey::new().await;
        j.link().await;
        let (destination, reply, saved) = long_reply(&j, 7002).await;
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = calls.clone();
        Mock::given(method("POST"))
            .and(path("/bot100:manager-test-secret/sendMessage"))
            .respond_with(move |_: &wiremock::Request| {
                let n = counter.fetch_add(1, Ordering::SeqCst);
                if n == 1 {
                    if uncertain {
                        ResponseTemplate::new(200).set_body_string("lost envelope")
                    } else {
                        ResponseTemplate::new(429).set_body_json(
                            json!({"ok":false,"error_code":429,"parameters":{"retry_after":1}}),
                        )
                    }
                } else {
                    ResponseTemplate::new(200)
                        .set_body_json(json!({"ok":true,"result":{"message_id":900+n}}))
                }
            })
            .with_priority(1)
            .mount(&j.server)
            .await;
        let body = delivery::render_saved(&saved, &j.state.config.frontend_url);
        let first = j.deliver(&destination, &reply, body.clone()).await.unwrap();
        if uncertain {
            assert!(matches!(first, DeliveryOutcome::Uncertain));
            assert!(matches!(
                j.deliver(&destination, &reply, body).await.unwrap(),
                DeliveryOutcome::Duplicate
            ));
            assert_eq!(calls.load(Ordering::SeqCst), 2);
        } else {
            assert!(matches!(first, DeliveryOutcome::Deferred { .. }));
            tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
            assert!(matches!(
                j.deliver(&destination, &reply, body).await.unwrap(),
                DeliveryOutcome::Sent { .. }
            ));
            let sends = j.sends().await;
            let first_text = &sends[2]["text"];
            assert_eq!(
                sends.iter().filter(|p| &p["text"] == first_text).count(),
                1,
                "accepted prefix is not replayed"
            );
        }
        let row = j
            .state
            .db
            .collection::<ChannelTextDelivery>(TEXT)
            .find_one(doc! {"source_id":&saved.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.parts[0].state, TextPartState::Sent);
        assert_eq!(
            row.parts[1].state,
            if uncertain {
                TextPartState::Sending
            } else {
                TextPartState::Sent
            }
        );
        j.state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn manager_outbound_sends_native_photos_and_files_with_v2_source_claims() {
    let j = Journey::new().await;
    j.link().await;
    let (destination, reply, mut saved) = j.prepared_reply(7003).await;
    for (mime, bytes) in [
        ("image/png", b"\x89PNG\r\n\x1a\nimage".as_slice()),
        ("application/pdf", b"%PDF-1.7 test".as_slice()),
    ] {
        attach(&j, &mut saved, mime, bytes).await;
    }
    for method_name in ["sendPhoto", "sendDocument"] {
        Mock::given(method("POST"))
            .and(path(format!("/bot100:manager-test-secret/{method_name}")))
            .and(|request: &wiremock::Request| {
                let field = b"name=\"protect_content\"\r\n\r\ntrue";
                request.body.windows(field.len()).any(|part| part == field)
            })
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"ok":true,"result":{"message_id":899}})),
            )
            .expect(1)
            .mount(&j.server)
            .await;
    }
    let body = delivery::render_saved(&saved, &j.state.config.frontend_url);
    assert!(matches!(
        j.deliver(&destination, &reply, body.clone()).await.unwrap(),
        DeliveryOutcome::Sent { .. }
    ));
    assert!(matches!(
        j.deliver(&destination, &reply, body).await.unwrap(),
        DeliveryOutcome::Duplicate
    ));
    let claims = j
        .state
        .db
        .collection::<Document>(crate::models::nyxbot_image_delivery::COLLECTION_NAME)
        .count_documents(doc! {"user_id":&j.owner,"state":"sent"})
        .await
        .unwrap();
    assert_eq!(claims, 2);
    assert!(
        !j.sends().await.last().unwrap()["text"]
            .as_str()
            .unwrap()
            .contains("available in your NyxID conversation")
    );
    j.server.verify().await;
    j.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn manager_outbound_recovery_resumes_only_a_proven_unsent_suffix() {
    for ambiguous in [false, true] {
        let j = Journey::new().await;
        j.link().await;
        let (destination, reply, saved) = long_reply(&j, 7004).await;
        let calls = Arc::new(AtomicUsize::new(0));
        let counter = calls.clone();
        Mock::given(method("POST"))
            .and(path("/bot100:manager-test-secret/sendMessage"))
            .respond_with(move |_: &wiremock::Request| {
                let n = counter.fetch_add(1, Ordering::SeqCst);
                if n == 1 {
                    if ambiguous {
                        ResponseTemplate::new(200).set_body_string("lost reply")
                    } else {
                        ResponseTemplate::new(429).set_body_json(
                            json!({"ok":false,"error_code":429,"parameters":{"retry_after":1}}),
                        )
                    }
                } else {
                    ResponseTemplate::new(200)
                        .set_body_json(json!({"ok":true,"result":{"message_id":1000+n}}))
                }
            })
            .with_priority(1)
            .mount(&j.server)
            .await;
        let body = delivery::render_saved(&saved, &j.state.config.frontend_url);
        j.deliver(&destination, &reply, body.clone()).await.unwrap();
        // Model a crash before the parent outcome was recorded. Per-part
        // provider evidence remains the authority for replay safety.
        let parents = j
            .state
            .db
            .collection::<Document>(crate::models::platform_manager_delivery::COLLECTION_NAME);
        parents.update_one(doc! {"reference.reply_id":&saved.id},doc! {"$set":{"state":"dispatching","updated_at":bson::DateTime::from_chrono(Utc::now()-chrono::Duration::minutes(3))}}).await.unwrap();
        delivery::sweep(
            &j.state,
            j.producers.identity.clone(),
            j.producers.public.clone(),
            100,
        )
        .await
        .unwrap();
        let parent = parents
            .find_one(doc! {"reference.reply_id":&saved.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            parent.get_str("state").unwrap(),
            if ambiguous { "uncertain" } else { "pending" }
        );
        tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
        let result = j.deliver(&destination, &reply, body).await.unwrap();
        if ambiguous {
            assert!(matches!(result, DeliveryOutcome::Duplicate));
            assert_eq!(calls.load(Ordering::SeqCst), 2);
        } else {
            assert!(matches!(result, DeliveryOutcome::Sent { .. }));
            let sends = j.sends().await;
            assert_eq!(
                sends
                    .iter()
                    .filter(|s| s["text"] == sends[2]["text"])
                    .count(),
                1
            );
        }
        j.state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn manager_outbound_revalidates_stop_after_an_accepted_prefix() {
    let j = Journey::new().await;
    j.link().await;
    let (destination, reply, saved) = long_reply(&j, 7005).await;
    let entered = Arc::new(tokio::sync::Notify::new());
    let reached = entered.clone();
    Mock::given(method("POST"))
        .and(path("/bot100:manager-test-secret/sendMessage"))
        .respond_with(move |_: &wiremock::Request| {
            reached.notify_one();
            ResponseTemplate::new(200)
                .set_delay(std::time::Duration::from_millis(150))
                .set_body_json(json!({"ok":true,"result":{"message_id":1000}}))
        })
        .expect(1)
        .with_priority(1)
        .mount(&j.server)
        .await;
    let db = j.state.db.clone();
    let id = saved.id.clone();
    let stop = tokio::spawn(async move {
        entered.notified().await;
        db.collection::<Document>(MESSAGES)
            .update_one(
                doc! {"_id":id},
                doc! {"$set":{"status":"stopped","error_code":"cancelled"}},
            )
            .await
            .unwrap();
    });
    let outcome = j
        .deliver(
            &destination,
            &reply,
            delivery::render_saved(&saved, &j.state.config.frontend_url),
        )
        .await
        .unwrap();
    stop.await.unwrap();
    assert!(matches!(outcome, DeliveryOutcome::Refused(_)));
    let parent = j
        .state
        .db
        .collection::<Document>(crate::models::platform_manager_delivery::COLLECTION_NAME)
        .find_one(doc! {"reference.reply_id":&saved.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(parent.get_array("telegram_message_ids").unwrap().len(), 1);
    j.server.verify().await;
    j.state.db.drop().await.unwrap();
}

async fn attach(j: &Journey, saved: &mut AssistantMessage, mime: &str, bytes: &[u8]) -> String {
    let id = uuid::Uuid::new_v4().to_string();
    let mut attachment = crate::models::assistant_attachment::AssistantAttachment {
        secret_context_versions: Default::default(),
        id: id.clone(),
        user_id: j.owner.clone(),
        conversation_id: saved.conversation_id.clone(),
        turn_id: saved.turn_id.clone(),
        origin: "tool".into(),
        content_type: mime.into(),
        size: bytes.len() as i64,
        data_encrypted: Vec::new(),
        created_at: Utc::now(),
    };
    attachment.data_encrypted = crate::services::assistant_record_service::encrypt_image(
        &j.state.encryption_keys,
        &attachment,
        bytes,
    )
    .await
    .unwrap();
    crate::services::credential_record_service::mark_version(
        &mut attachment.secret_context_versions,
        &j.state.encryption_keys,
        "data_encrypted",
    );
    j.state
        .db
        .collection::<crate::models::assistant_attachment::AssistantAttachment>(
            crate::models::assistant_attachment::COLLECTION_NAME,
        )
        .insert_one(attachment)
        .await
        .unwrap();
    saved
        .attachments
        .push(crate::models::assistant_conversation::TurnAttachment {
            id: id.clone(),
            origin: "tool".into(),
            content_type: mime.into(),
            size: bytes.len() as i64,
            label: "generated file".into(),
            chat_delivery: Some(true),
            image_input: None,
            image_inspection: None,
            pages: None,
        });
    j.state
        .db
        .collection::<AssistantMessage>(MESSAGES)
        .replace_one(doc! {"_id":&saved.id}, &*saved)
        .await
        .unwrap();
    id
}

#[tokio::test]
async fn manager_outbound_unavailable_and_ambiguous_media_have_honest_notes_and_never_replay() {
    use crate::models::nyxbot_image_delivery::{ImageDeliveryState, NyxbotImageDelivery};
    for ambiguous in [false, true] {
        let j = Journey::new().await;
        j.link().await;
        let (destination, reply, mut saved) = j.prepared_reply(7006).await;
        let id = attach(
            &j,
            &mut saved,
            "application/pdf",
            b"%PDF-1.7 private document",
        )
        .await;
        if ambiguous {
            Mock::given(method("POST"))
                .and(path("/bot100:manager-test-secret/sendDocument"))
                .respond_with(ResponseTemplate::new(200).set_body_string("lost response"))
                .expect(1)
                .mount(&j.server)
                .await;
        } else {
            // A source belonging to another owner must never be materialized.
            j.state
                .db
                .collection::<Document>(crate::models::assistant_attachment::COLLECTION_NAME)
                .update_one(doc! {"_id":&id}, doc! {"$set":{"user_id":"other-owner"}})
                .await
                .unwrap();
            Mock::given(path("/bot100:manager-test-secret/sendDocument"))
                .respond_with(ResponseTemplate::new(500))
                .expect(0)
                .mount(&j.server)
                .await;
        }
        let body = delivery::render_saved(&saved, &j.state.config.frontend_url);
        assert!(matches!(
            j.deliver(&destination, &reply, body.clone()).await.unwrap(),
            DeliveryOutcome::Sent { .. }
        ));
        assert!(
            j.sends().await.last().unwrap()["text"]
                .as_str()
                .unwrap()
                .contains("1 attachment(s) could not be confirmed delivered")
        );
        let claim = j
            .state
            .db
            .collection::<NyxbotImageDelivery>(
                crate::models::nyxbot_image_delivery::COLLECTION_NAME,
            )
            .find_one(doc! {"attachment_id":&id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            claim.state,
            if ambiguous {
                ImageDeliveryState::Unknown
            } else {
                ImageDeliveryState::Failed
            }
        );
        assert!(matches!(
            j.deliver(&destination, &reply, body).await.unwrap(),
            DeliveryOutcome::Duplicate
        ));
        j.server.verify().await;
        j.state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn manager_outbound_honors_legacy_claims_for_the_owner_channel_alias() {
    use crate::models::nyxbot_image_delivery::{COLLECTION_NAME as IMAGES, LEGACY_COLLECTION_NAME};
    for item_state in ["sent", "sending"] {
        let j = Journey::new().await;
        j.link().await;
        let (destination, reply, mut saved) = j.prepared_reply(7007).await;
        let attachment = attach(&j, &mut saved, "image/png", b"private image").await;
        let alias = uuid::Uuid::new_v4().to_string();
        j.state
            .db
            .collection::<Document>(crate::models::nyxbot_channel::COLLECTION_NAME)
            .insert_one(
                doc! {"_id":&alias,"user_id":&j.owner,"channel_bot_id":&j.profile.channel_bot_id},
            )
            .await
            .unwrap();
        j.state.db.collection::<Document>(LEGACY_COLLECTION_NAME).insert_one(doc! {
            "_id":uuid::Uuid::new_v4().to_string(),"user_id":&j.owner,"conversation_id":&saved.conversation_id,
            "turn_id":&saved.turn_id,"channel_id":&alias,"destination_key":"initiated:legacy",
            "state":if item_state=="sent" {"done"} else {"sending"},"claim_id":"legacy-attempt",
            "items":[{"id":&attachment,"state":item_state}],"created_at":bson::DateTime::now(),
            "expires_at":bson::DateTime::from_chrono(Utc::now()+chrono::Duration::days(7)),
        }).await.unwrap();
        Mock::given(path("/bot100:manager-test-secret/sendPhoto"))
            .respond_with(ResponseTemplate::new(500))
            .expect(0)
            .mount(&j.server)
            .await;
        assert!(matches!(
            j.deliver(
                &destination,
                &reply,
                delivery::render_saved(&saved, &j.state.config.frontend_url)
            )
            .await
            .unwrap(),
            DeliveryOutcome::Sent { .. }
        ));
        let text = j.sends().await.last().unwrap()["text"]
            .as_str()
            .unwrap()
            .to_owned();
        assert_eq!(
            text.contains("could not be confirmed delivered"),
            item_state == "sending"
        );
        assert_eq!(
            j.state
                .db
                .collection::<Document>(IMAGES)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
        j.server.verify().await;
        j.state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn manager_outbound_media_rate_refusal_resumes_only_the_unsent_attachment() {
    let j = Journey::new().await;
    j.link().await;
    let (destination, reply, mut saved) = j.prepared_reply(7010).await;
    attach(&j, &mut saved, "application/pdf", b"first private document").await;
    attach(
        &j,
        &mut saved,
        "application/pdf",
        b"second private document",
    )
    .await;
    let times = Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen = times.clone();
    let count = Arc::new(AtomicUsize::new(0));
    let calls = count.clone();
    Mock::given(path("/bot100:manager-test-secret/sendDocument"))
        .respond_with(move |_: &wiremock::Request| {
            seen.lock().unwrap().push(std::time::Instant::now());
            let n = calls.fetch_add(1, Ordering::SeqCst);
            if n == 1 {
                ResponseTemplate::new(429).set_body_json(
                    json!({"ok":false,"error_code":429,"parameters":{"retry_after":2}}),
                )
            } else {
                ResponseTemplate::new(200)
                    .set_body_json(json!({"ok":true,"result":{"message_id":700+n}}))
            }
        })
        .mount(&j.server)
        .await;
    let body = delivery::render_saved(&saved, &j.state.config.frontend_url);
    let before = j.sends().await.len();
    let first = j.deliver(&destination, &reply, body.clone()).await.unwrap();
    let DeliveryOutcome::Deferred { retry_at } = first else {
        panic!("retryable media refusal");
    };
    assert!(retry_at > Utc::now() + chrono::Duration::milliseconds(1000));
    assert_eq!(count.load(Ordering::SeqCst), 2);
    assert_eq!(
        j.sends().await.len(),
        before,
        "text waits for media outcomes"
    );
    let _ = j.deliver(&destination, &reply, body.clone()).await.unwrap();
    assert_eq!(
        count.load(Ordering::SeqCst),
        2,
        "early retries must not call Telegram"
    );
    tokio::time::sleep(std::time::Duration::from_millis(2100)).await;
    assert!(matches!(
        j.deliver(&destination, &reply, body).await.unwrap(),
        DeliveryOutcome::Sent { .. }
    ));
    assert_eq!(count.load(Ordering::SeqCst), 3);
    let requests = j.server.received_requests().await.unwrap();
    assert_eq!(
        requests
            .iter()
            .filter(|r| String::from_utf8_lossy(&r.body).contains("first private document"))
            .count(),
        1
    );
    {
        let sent_times = times.lock().unwrap();
        assert!(sent_times[2].duration_since(sent_times[1]) >= std::time::Duration::from_secs(2));
    }
    assert!(
        !j.sends().await.last().unwrap()["text"]
            .as_str()
            .unwrap()
            .contains("could not be confirmed")
    );
    j.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn manager_outbound_provider_valid_text_preserves_links_and_whitespace() {
    let j = Journey::new().await;
    j.link().await;
    let (destination, reply, mut saved) = j.prepared_reply(7011).await;
    saved.text = format!(
        "Read [the README](./README.md)\n\n![](https://example.test/a.png)\n\n```\nstart{}🦉end\n```",
        " ".repeat(8200)
    );
    j.state
        .db
        .collection::<AssistantMessage>(MESSAGES)
        .replace_one(doc! {"_id":&saved.id}, &saved)
        .await
        .unwrap();
    let times = Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen = times.clone();
    Mock::given(path("/bot100:manager-test-secret/sendMessage"))
        .respond_with(move |request: &wiremock::Request| {
            let body: serde_json::Value = serde_json::from_slice(&request.body).unwrap();
            let text = body["text"].as_str().unwrap();
            let valid = !text.trim().is_empty()
                && text.encode_utf16().count() <= 4096
                && body["entities"].as_array().unwrap().iter().all(|e| {
                    e["type"] != "text_link" || url::Url::parse(e["url"].as_str().unwrap()).is_ok()
                });
            seen.lock().unwrap().push(std::time::Instant::now());
            if valid {
                ResponseTemplate::new(200)
                    .set_body_json(json!({"ok":true,"result":{"message_id":701}}))
            } else {
                ResponseTemplate::new(400).set_body_json(json!({"ok":false,"error_code":400}))
            }
        })
        .with_priority(1)
        .mount(&j.server)
        .await;
    let before = j.sends().await.len();
    assert!(matches!(
        j.deliver(
            &destination,
            &reply,
            delivery::render_saved(&saved, &j.state.config.frontend_url)
        )
        .await
        .unwrap(),
        DeliveryOutcome::Sent { .. }
    ));
    let sends = j.sends().await;
    let text = sends[before..]
        .iter()
        .map(|v| v["text"].as_str().unwrap())
        .collect::<String>();
    assert!(text.contains("./README.md"));
    assert!(text.contains("https://example.test/a.png"));
    assert!(
        text.replace("Whitespace continuation:\n", "")
            .contains(&format!("start{}🦉end", " ".repeat(8200)))
    );
    {
        let sent_times = times.lock().unwrap();
        // Fixed server-clock windows permit adjacent sends across a boundary.
        // A whole multipart reply must still spend multiple windows.
        assert!(
            sent_times.last().unwrap().duration_since(sent_times[0])
                >= std::time::Duration::from_secs(sent_times.len().saturating_sub(2) as u64)
        );
    }
    j.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn concurrent_saved_reply_notice_snapshots_reuse_the_winner() {
    let j = Journey::new().await;
    j.link().await;
    let (destination, reply, saved) = j.prepared_reply(7801).await;
    let body = delivery::render_saved(&saved, &j.state.config.frontend_url);
    assert!(matches!(
        j.deliver(&destination, &reply, body.clone()).await.unwrap(),
        DeliveryOutcome::Sent { .. }
    ));
    let sent = j.sends().await.len();
    // Repeat the forced interleaving without depending on scheduler luck.
    for iteration in 0..8 {
        j.state
            .db
            .collection::<Document>(crate::models::platform_manager_delivery::COLLECTION_NAME)
            .update_one(
                doc! {"reference.reply_id": &saved.id},
                doc! {"$unset": {"notice_card_ids": ""}},
            )
            .await
            .unwrap();
        let (a, b) = delivery::TEST_BEFORE_NOTICE_SNAPSHOT
            .scope(Arc::new(tokio::sync::Barrier::new(2)), async {
                tokio::join!(
                    Box::pin(j.deliver(&destination, &reply, body.clone())),
                    Box::pin(j.deliver(&destination, &reply, body.clone()))
                )
            })
            .await;
        assert!(
            a.is_ok() && b.is_ok(),
            "iteration={iteration} a={a:?} b={b:?}"
        );
        assert_eq!(j.sends().await.len(), sent, "a={a:?} b={b:?}");
    }
    j.state.db.drop().await.unwrap();
}

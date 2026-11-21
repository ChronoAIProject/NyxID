use super::*;

#[derive(Clone, Copy, PartialEq)]
enum CapacityOutcome {
    Accepted,
    Expired,
    Ambiguous,
    Stopped,
    AuthorityLost,
}

async fn channel_capacity_case(name: &str, outcome: CapacityOutcome) {
    use std::time::Duration;
    let f = fixture(name).await;
    let engine_mock = MockServer::start().await;
    let waits_past_old_deadline = outcome == CapacityOutcome::Accepted;
    let posts = Mock::given(method("POST")).and(path("/v1/responses"));
    posts
        .respond_with(
            ResponseTemplate::new(503)
                .insert_header(
                    "retry-after",
                    if waits_past_old_deadline { "60" } else { "3" },
                )
                .set_body_json(json!({"error":{"code":"capacity_exceeded"}})),
        )
        .up_to_n_times(if waits_past_old_deadline { 2 } else { 1 })
        .mount(&engine_mock)
        .await;
    let completion = json!({"type":"response.completed","sequence_number":1,"response":{
        "id":"resp_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa_bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
        "conversation":{"id":"conv_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"},"status":"completed",
        "output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"One answer"}]}]}});
    Mock::given(method("POST"))
        .and(path("/v1/responses"))
        .respond_with(if outcome == CapacityOutcome::Ambiguous {
            ResponseTemplate::new(504)
        } else {
            ResponseTemplate::new(200)
                .set_body_raw(format!("data: {completion}\n\n"), "text/event-stream")
        })
        .mount(&engine_mock)
        .await;
    f.state
        .db
        .collection::<bson::Document>(SERVICES)
        .update_one(
            doc! {"slug":engine::SERVICE_SLUG},
            doc! {"$set":{"base_url":engine_mock.uri()}},
        )
        .await
        .unwrap();
    let bound = put_binding(
        State(f.state.clone()),
        Path("bnd_capacity".into()),
        bearer(&f.key),
        Json(binding_body(&f.key, OWNER)),
    )
    .await;
    assert_eq!(bound.status(), StatusCode::OK);
    let command = event("What time?", "7", "capacity-question");
    let response = respond(&f.state, &f.key, &command, "evt_capacity").await;
    assert_eq!(response.status(), StatusCode::OK);
    let waiting = tokio::time::timeout(Duration::from_secs(12), async {
        loop {
            if let Some(row) = f
                .state
                .db
                .collection::<AssistantConversation>(CONVERSATIONS)
                .find_one(doc! {"active_turn.waiting_for_capacity":true})
                .await
                .unwrap()
            {
                break row;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("first structured refusal");
    let turn = waiting.active_turn.as_ref().unwrap();
    let window = turn.capacity_wait_deadline.unwrap() - turn.engine_request_started_at.unwrap();
    assert!(
        (598..=600).contains(&window.num_seconds()),
        "window {window:?}"
    );
    let mut other_slots = Vec::new();
    for _ in 0..3 {
        other_slots.push(
            crate::handlers::assistant_team::acquire(
                &f.state,
                crate::handlers::assistant_team::Pool::Channel { owner: OWNER },
            )
            .await
            .unwrap()
            .expect("three free slots"),
        );
    }
    assert!(
        crate::handlers::assistant_team::acquire(
            &f.state,
            crate::handlers::assistant_team::Pool::Channel { owner: OWNER }
        )
        .await
        .unwrap()
        .is_none()
    );
    if outcome == CapacityOutcome::Stopped {
        crate::handlers::assistant_nyxagent::stop(
            State(f.state.clone()),
            crate::test_utils::test_auth_user(OWNER),
            Path(waiting.id.clone()),
        )
        .await
        .unwrap();
    }
    if outcome == CapacityOutcome::AuthorityLost {
        f.state
            .db
            .collection::<bson::Document>(crate::models::channel_conversation::COLLECTION_NAME)
            .update_one(
                doc! {"_id":&f.channel.route_id},
                doc! {"$set":{"is_active":false}},
            )
            .await
            .unwrap();
    }
    if outcome == CapacityOutcome::Expired {
        f.state
            .db
            .collection::<bson::Document>(EVENTS)
            .update_one(
                doc! {"_id":&turn.channel_event_id},
                doc! {"$set":{"delivery.target_expires_at":bson::DateTime::now()}},
            )
            .await
            .unwrap();
    }
    let frames = tokio::time::timeout(
        Duration::from_secs(if waits_past_old_deadline { 150 } else { 15 }),
        body_text(response),
    )
    .await
    .unwrap();
    let requests: Vec<_> = engine_mock
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .filter(|r| r.url.path() == "/v1/responses")
        .collect();
    let expected = match outcome {
        CapacityOutcome::Accepted => 3,
        CapacityOutcome::Ambiguous => 2,
        _ => 1,
    };
    assert_eq!(requests.len(), expected);
    for repeated in requests.iter().skip(1) {
        assert_eq!(
            requests[0].headers["idempotency-key"],
            repeated.headers["idempotency-key"]
        );
        assert_eq!(requests[0].body, repeated.body);
    }
    let messages = f
        .state
        .db
        .collection::<AssistantMessage>(crate::models::assistant_message::COLLECTION_NAME);
    let saved = messages
        .find_one(doc! {"turn_id":&turn.turn_id,"role":"assistant"})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        messages
            .count_documents(doc! {"turn_id":&turn.turn_id,"role":"assistant"})
            .await
            .unwrap(),
        1
    );
    match outcome {
        CapacityOutcome::Accepted => {
            assert_eq!(saved.text, "One answer");
            assert!(!frames.contains("at capacity"));
        }
        CapacityOutcome::Expired => {
            assert_eq!(saved.error_code.as_deref(), Some("capacity_exceeded"));
            assert!(frames.contains("NyxBot is at capacity right now (waited"));
        }
        CapacityOutcome::Ambiguous | CapacityOutcome::AuthorityLost => {
            assert!(saved.error_code.is_some())
        }
        CapacityOutcome::Stopped => assert_eq!(saved.error_code.as_deref(), Some("cancelled")),
    }
    assert!(sends(&f).await.is_empty());
}

#[tokio::test]
async fn channel_capacity_2695_retries_to_one_answer_in_original_slot() {
    Box::pin(channel_capacity_case(
        "capacity_accept",
        CapacityOutcome::Accepted,
    ))
    .await;
}
#[tokio::test]
async fn channel_capacity_2695_expiry_delivers_one_notice() {
    Box::pin(channel_capacity_case(
        "capacity_expire",
        CapacityOutcome::Expired,
    ))
    .await;
}
#[tokio::test]
async fn channel_capacity_2695_ambiguous_retry_is_terminal() {
    Box::pin(channel_capacity_case(
        "capacity_ambiguous",
        CapacityOutcome::Ambiguous,
    ))
    .await;
}
#[tokio::test]
async fn channel_capacity_2695_stop_ends_wait() {
    Box::pin(channel_capacity_case(
        "capacity_stop",
        CapacityOutcome::Stopped,
    ))
    .await;
}

#[tokio::test]
async fn channel_notice_language_2695_manager_patch_controls_group_and_followed_notices() {
    let f = fixture("notice_language_2695").await;
    let rows = f.state.db.collection::<NyxbotThread>(THREADS);
    let chat = rows
        .find_one_and_update(
            doc! {"channel_id": &f.channel.id, "partition":PARTITION},
            doc! {"$set":{"kind":"group","owner_seen":false,"members":"owner","title":"中文群组"}},
        )
        .return_document(mongodb::options::ReturnDocument::After)
        .await
        .unwrap()
        .unwrap();
    let mut child = chat.clone();
    child.id = Uuid::new_v4().to_string();
    child.partition = "followed-child".into();
    child.follow.record_scope = Some("platform_thread".into());
    child.follow.settings_chat_id = Some(chat.id.clone());
    rows.insert_one(&child).await.unwrap();
    let mut origin = f.event.delivery.as_ref().unwrap().origin.clone();
    origin.partition.clone_from(&child.partition);
    let patch = |actor: &str, lang: &str| {
        chats::update_channel_chat(
            State(f.state.clone()),
            crate::test_utils::test_auth_user(actor),
            Path((f.channel.id.clone(), chat.id.clone())),
            Json(serde_json::from_value(json!({"notice_language":lang})).unwrap()),
        )
    };
    for (setting, expected) in [
        (
            None,
            "I'll answer everyone here once my owner has talked to me in this chat.",
        ),
        (
            Some("zh-CN"),
            "我的主人在这里和我说过话后，我就可以回复大家了。",
        ),
        (
            Some("zh-TW"),
            "我的主人在這裡和我說過話後，我就可以回覆大家了。",
        ),
        (
            Some("default"),
            "I'll answer everyone here once my owner has talked to me in this chat.",
        ),
    ] {
        if let Some(setting) = setting {
            let saved = patch(OWNER, setting).await.unwrap().0;
            assert_eq!(
                saved["chat"]["notice_language"].as_str(),
                (setting != "default").then_some(setting)
            );
        }
        let current = rows.find_one(doc! {"_id":&chat.id}).await.unwrap().unwrap();
        rows.update_one(doc! {"_id":&chat.id}, doc! {"$unset":{"guest_hint_at":""}})
            .await
            .unwrap();
        assert_eq!(
            chats::waiting_hint(&f.state, &current)
                .await
                .unwrap()
                .as_deref(),
            Some(expected)
        );
        assert_eq!(
            crate::services::channel_notices::chat_language(
                &f.state.db,
                &origin.nyxbot_channel_id,
                &origin.partition
            )
            .await
            .unwrap(),
            current.notice_language
        );
    }
    assert!(patch(&Uuid::new_v4().to_string(), "zh-TW").await.is_err());
    assert!(patch(OWNER, "fr").await.is_err());
    assert!(
        rows.find_one(doc! {"_id":chat.id})
            .await
            .unwrap()
            .unwrap()
            .notice_language
            .is_none()
    );
}

#[tokio::test]
async fn channel_capacity_2695_route_revoked_during_backoff_ends_retry() {
    Box::pin(channel_capacity_case(
        "capacity_authority_loss",
        CapacityOutcome::AuthorityLost,
    ))
    .await;
}

#[tokio::test]
async fn channel_notices_2695_org_guest_model_route_zh_cn_requires_evidence_for_no_tools() {
    for (count, expected) in [
        (
            Some(0_i64),
            "助手无法使用模型回答这个问题。这次请求没有调用任何工具。",
        ),
        (
            None,
            "助手无法使用模型回答这个问题。请先确认已执行的操作，再试一次。",
        ),
        (
            Some(1),
            "助手无法使用模型回答这个问题。请先确认已执行的操作，再试一次。",
        ),
    ] {
        let mut f = fixture("org_guest_notice_zh_cn").await;
        let stored = pending_recovery(&mut f).await;
        let chat = f
            .state
            .db
            .collection::<NyxbotThread>(THREADS)
            .find_one(doc! {"channel_id":&f.channel.id, "partition":PARTITION})
            .await
            .unwrap()
            .unwrap();
        let settings = chats::ChatSettings {
            notice_language: Some("zh-CN".into()),
            ..Default::default()
        };
        let updated = chats::update_chat(&f.state, OWNER, &chat.id, &settings)
            .await
            .unwrap();
        assert_eq!(updated["chat"]["notice_language"], "zh-CN");
        patch(&f, doc! {"relay_message_id":"test-inbound", "delivery.notice_language":updated["chat"]["notice_language"].as_str().unwrap(),
            "delivery.origin.org_execution":{"version":1,"org_id":"org","revision":"1","agent_id":"agent","actor_id":OWNER,"guest":true,"sender_hash":"sender"}}).await;
        settled(&f, true).await;
        f.state.db.collection::<bson::Document>(crate::models::assistant_message::COLLECTION_NAME)
            .update_one(doc! {"turn_id":&f.event.turn_id},doc! {"$set":{"text":"","error_code":"guest_model_route_required","activity_count":count}}).await.unwrap();
        use crate::services::nyxbot_response_recovery as recovery;
        recovery::refresh(
            &f.state.db,
            &f.state.encryption_keys,
            &stored,
            std::future::ready,
        )
        .await
        .unwrap();
        let saved = f
            .state
            .db
            .collection::<crate::models::nyxbot_gateway_response::GatewayResponse>(
                crate::models::nyxbot_gateway_response::COLLECTION_NAME,
            )
            .find_one(doc! {"_id":&stored.id})
            .await
            .unwrap()
            .unwrap();
        assert!(saved.terminal);
        let snapshot = recovery::payload(&f.state.encryption_keys, &saved)
            .await
            .unwrap()
            .snapshot;
        assert_eq!(snapshot["output"][0]["content"][0]["text"], expected);
    }
}

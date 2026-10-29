use super::*;
use crate::errors::AppError;
use crate::models::telegram_bot_request::{CreationContext, PromptOutcome, PromptPhase};

fn command(text: &str, id: i64) -> Value {
    let mut value = message(json!({"text":text}));
    value["update_id"] = json!(id);
    value
}

async fn start_request(
    state: &crate::AppState,
    actor: &str,
    server: &MockServer,
) -> (TelegramBotRequest, String) {
    let base = server.uri();
    let service = service(state, &base);
    let (request, link) = service
        .begin(actor, actor, "Support & 你好 /? #", true)
        .await
        .unwrap();
    let challenge = reqwest::Url::parse(&link)
        .unwrap()
        .query_pairs()
        .find(|(k, _)| k == "start")
        .unwrap()
        .1
        .to_string();
    (request, format!("/start {challenge}"))
}

#[tokio::test]
async fn telegram_new_bound_keyboard_outcomes_deduplicate_monotonically() {
    for invalid in [false, true] {
        let (state, actor, server) = fixture().await;
        let base = server.uri();
        let service = service(&state, &base);
        let headers = headers(&state).await;
        let (request, start) = start_request(&state, &actor, &server).await;
        Mock::given(method("POST"))
            .and(path(format!("/bot{MANAGER}/sendMessage")))
            .and(|r: &wiremock::Request| {
                r.body_json::<Value>()
                    .unwrap()
                    .get("reply_markup")
                    .is_some()
            })
            .respond_with(if invalid {
                ResponseTemplate::new(200).set_body_json(json!({"ok":true,"result":{}}))
            } else {
                ResponseTemplate::new(400).set_body_json(
                    json!({"ok":false,"error_code":400,"description":"PRIVATE-SENTINEL"}),
                )
            })
            .with_priority(1)
            .mount(&server)
            .await;
        webhook(&service, &headers, command(&start, 10)).await;
        let saved = service.get(&actor, &request.id).await.unwrap();
        let revision = saved.revision;
        let delivery = saved.prompt_delivery.unwrap();
        assert_eq!(delivery.phase, PromptPhase::Keyboard);
        assert_eq!(
            delivery.outcome,
            if invalid {
                PromptOutcome::Unknown
            } else {
                PromptOutcome::Rejected
            }
        );
        assert!(
            !serde_json::to_string(&crate::handlers::telegram_new::RequestResponse::from(
                service.get(&actor, &request.id).await.unwrap()
            ))
            .unwrap()
            .contains("SENTINEL")
        );
        webhook(&service, &headers, command(&start, 10)).await;
        assert_eq!(server.received_requests().await.unwrap().len(), 2);
        webhook(&service, &headers, command("/start", 11)).await;
        webhook(&service, &headers, command(&start, 10)).await;
        assert_eq!(server.received_requests().await.unwrap().len(), 4);
        let saved = service.get(&actor, &request.id).await.unwrap();
        assert_eq!(saved.id, request.id);
        assert_eq!(saved.start_update_id, Some(10));
        assert_eq!(saved.prompt_update_boundary, Some(11));
        assert_eq!(saved.revision, revision);
        assert_eq!(
            state
                .db
                .collection::<bson::Document>(REQUESTS)
                .count_documents(doc! {"active":true})
                .await
                .unwrap(),
            1
        );
    }
}

#[tokio::test]
async fn telegram_new_bound_first_rejection_can_retry_same_update() {
    let (state, actor, server) = fixture().await;
    let base = server.uri();
    let service = service(&state, &base);
    let headers = headers(&state).await;
    let (request, start) = start_request(&state, &actor, &server).await;
    // One provider rejection, then the normal success response. A scoped
    // priority mock is unsuitable here: WireMock 0.6.5 sorts its mock array but
    // retains the guard's insertion index, so dropping it can remove the fallback.
    Mock::given(method("POST"))
        .and(path(format!("/bot{MANAGER}/sendMessage")))
        .respond_with(ResponseTemplate::new(403))
        .with_priority(1)
        .up_to_n_times(1)
        .expect(1)
        .mount(&server)
        .await;
    assert!(
        service
            .webhook(&headers, &serde_json::to_vec(&command(&start, 10)).unwrap())
            .await
            .is_err()
    );
    let saved = service.get(&actor, &request.id).await.unwrap();
    assert_eq!(
        saved.prompt_delivery.unwrap().outcome,
        PromptOutcome::Rejected
    );
    webhook(&service, &headers, command(&start, 10)).await;
    let saved = service.get(&actor, &request.id).await.unwrap();
    assert_eq!(
        saved.prompt_delivery.unwrap().outcome,
        PromptOutcome::Accepted
    );
    assert_eq!(server.received_requests().await.unwrap().len(), 3);
    let requests = server.received_requests().await.unwrap();
    assert!(
        requests[0]
            .body_json::<Value>()
            .unwrap()
            .get("reply_markup")
            .is_none()
    );
    assert!(
        requests[1]
            .body_json::<Value>()
            .unwrap()
            .get("reply_markup")
            .is_none()
    );
    assert!(requests[2].body_json::<Value>().unwrap()["reply_markup"]["keyboard"].is_array());
}

#[tokio::test]
async fn telegram_new_legacy_generation_and_metadata_absence_preserve_start() {
    let (state, actor, server) = fixture().await;
    let base = server.uri();
    let service = service(&state, &base);
    let headers = headers(&state).await;
    let (request, start) = start_request(&state, &actor, &server).await;
    state.db.collection::<bson::Document>(REQUESTS).update_one(doc! {"_id":&request.id},doc! {"$unset":{"creation_context":"","prompt_delivery":"","prompt_update_boundary":"","connection_generation":""}}).await.unwrap();
    webhook(&service, &headers, command(&start, 10)).await;
    state
        .db
        .collection::<bson::Document>(REQUESTS)
        .update_one(
            doc! {"_id":&request.id},
            doc! {"$unset":{"connection_generation":""}},
        )
        .await
        .unwrap();
    webhook(&service, &headers, command("/start", 11)).await;
    assert_eq!(server.received_requests().await.unwrap().len(), 4);
    assert!(service.creation_link(&actor, &request.id).await.is_err());
    assert_eq!(
        service
            .get(&actor, &request.id)
            .await
            .unwrap()
            .prompt_delivery
            .unwrap()
            .outcome,
        PromptOutcome::Accepted
    );
}

#[tokio::test]
async fn telegram_new_nonstandard_http_status_keeps_request_dto_bounded_and_recovery_visible() {
    for status in [600, 700] {
        let (state, actor, server) = fixture().await;
        let base = server.uri();
        let service = service(&state, &base);
        let (request, start) = start_request(&state, &actor, &server).await;
        Mock::given(method("POST"))
            .and(path(format!("/bot{MANAGER}/sendMessage")))
            .and(|r: &wiremock::Request| {
                r.body_json::<Value>()
                    .unwrap()
                    .get("reply_markup")
                    .is_some()
            })
            .respond_with(ResponseTemplate::new(status).set_body_string("PRIVATE-SENTINEL"))
            .with_priority(1)
            .mount(&server)
            .await;
        webhook(&service, &headers(&state).await, command(&start, 10)).await;
        let saved = service.get(&actor, &request.id).await.unwrap();
        let diagnostic = saved
            .prompt_delivery
            .as_ref()
            .unwrap()
            .diagnostic
            .as_ref()
            .unwrap();
        assert_eq!(diagnostic.http_status, None);
        assert_eq!(
            diagnostic.failure,
            crate::models::telegram_bot_request::TelegramFailure::HttpRejected
        );
        let response = crate::handlers::telegram_new::projected_request(&state, saved)
            .await
            .unwrap();
        assert!(response.can_open_creation_form);
        assert_eq!(
            response.prompt_delivery.as_ref().unwrap().outcome,
            PromptOutcome::Rejected
        );
        assert_eq!(response.prompt_delivery.as_ref().unwrap().http_status, None);
        assert!(
            !serde_json::to_string(&response)
                .unwrap()
                .contains("SENTINEL")
        );
        assert!(service.creation_link(&actor, &request.id).await.is_ok());
    }
}

#[tokio::test]
async fn telegram_new_phase_lease_renews_without_reviving_expired_instructions() {
    for expired in [false, true] {
        let (state, actor, server) = fixture().await;
        let base = server.uri();
        let service = service(&state, &base);
        let headers = headers(&state).await;
        let (request, start) = start_request(&state, &actor, &server).await;
        Mock::given(method("POST"))
            .and(path(format!("/bot{MANAGER}/sendMessage")))
            .and(|r: &wiremock::Request| {
                r.body_json::<Value>()
                    .unwrap()
                    .get("reply_markup")
                    .is_none()
            })
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"ok":true,"result":{"message_id":1}}))
                    .set_delay(std::time::Duration::from_millis(600)),
            )
            .with_priority(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path(format!("/bot{MANAGER}/sendMessage")))
            .and(|r: &wiremock::Request| {
                r.body_json::<Value>()
                    .unwrap()
                    .get("reply_markup")
                    .is_some()
            })
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"ok":true,"result":{"message_id":2}}))
                    .set_delay(std::time::Duration::from_millis(2500)),
            )
            .with_priority(1)
            .mount(&server)
            .await;
        let bytes = serde_json::to_vec(&command(&start, 10)).unwrap();
        let (result, ()) = tokio::join!(service.webhook(&headers, &bytes), async {
            for _ in 0..200 {
                if !server.received_requests().await.unwrap().is_empty() {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
            assert_eq!(server.received_requests().await.unwrap().len(), 1);
            let initial = service.get(&actor, &request.id).await.unwrap();
            let attempt_id = initial.prompt_delivery.unwrap().attempt_id;
            let aged = Utc::now() - Duration::seconds(if expired { 46 } else { 44 });
            state.db.collection::<bson::Document>(REQUESTS).update_one(doc! {"_id":&request.id},
                doc! {"$set":{"prompt_delivery.attempted_at":bson::DateTime::from_chrono(aged)}}).await.unwrap();
            if expired {
                return;
            }
            for _ in 0..200 {
                if server.received_requests().await.unwrap().len() == 2 {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
            assert_eq!(server.received_requests().await.unwrap().len(), 2);
            // The original instructions deadline passes while keyboard I/O is
            // still held. The current phase timestamp must keep the lease live.
            tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
            assert!(aged + Duration::seconds(45) < Utc::now());
            let current = service.get(&actor, &request.id).await.unwrap();
            assert_eq!(
                current.prompt_delivery.as_ref().unwrap().phase,
                PromptPhase::Keyboard
            );
            assert!(current.prompt_delivery.as_ref().unwrap().attempted_at > aged);
            let dto = crate::handlers::telegram_new::RequestResponse::from(current);
            assert_eq!(dto.prompt_delivery.unwrap().outcome, PromptOutcome::Sending);
            webhook(&service, &headers, command("/start", 11)).await;
            let current = service.get(&actor, &request.id).await.unwrap();
            assert_eq!(current.prompt_delivery.unwrap().attempt_id, attempt_id);
            assert_eq!(current.prompt_update_boundary, Some(10));
            assert_eq!(current.start_update_id, Some(10));
            assert_eq!(current.revision, initial.revision);
            assert_eq!(server.received_requests().await.unwrap().len(), 2);
        });
        result.unwrap();
        assert_eq!(
            server.received_requests().await.unwrap().len(),
            if expired { 1 } else { 2 }
        );
        if expired {
            let dto = crate::handlers::telegram_new::RequestResponse::from(
                service.get(&actor, &request.id).await.unwrap(),
            );
            assert_eq!(dto.prompt_delivery.unwrap().outcome, PromptOutcome::Unknown);
            webhook(&service, &headers, command("/start", 11)).await;
            assert_eq!(server.received_requests().await.unwrap().len(), 3);
        }
    }
}

#[tokio::test]
async fn telegram_new_recovery_is_same_request_encoded_and_authority_bound() {
    let (state, actor, server) = fixture().await;
    let base = server.uri();
    let service = service(&state, &base);
    let headers = headers(&state).await;
    let (request, start) = start_request(&state, &actor, &server).await;
    assert!(service.creation_link(&actor, &request.id).await.is_err());
    webhook(&service, &headers, command(&start, 10)).await;
    let before = service.get(&actor, &request.id).await.unwrap();
    let (saved, url) = service.creation_link(&actor, &request.id).await.unwrap();
    assert_eq!(saved.id, request.id);
    assert_eq!(saved.revision, before.revision);
    assert_eq!(saved.expires_at, before.expires_at);
    assert_eq!(saved.start_update_id, Some(10));
    let parsed = url::Url::parse(&url).unwrap();
    assert_eq!(parsed.host_str(), Some("t.me"));
    assert_eq!(parsed.path(), "/newbot/NyxSetupBot/support_bot");
    assert_eq!(
        parsed.query_pairs().next().unwrap().1,
        "Support & 你好 /? #"
    );
    assert!(!url.contains(start.trim_start_matches("/start ")));
    assert_eq!(
        service.creation_link(&actor, &request.id).await.unwrap().1,
        url
    );
    assert!(
        service
            .creation_link("wrong-actor", &request.id)
            .await
            .is_err()
    );
    state
        .db
        .collection::<bson::Document>(REQUESTS)
        .update_one(
            doc! {"_id":&request.id},
            doc! {"$unset":{"creation_context":""}},
        )
        .await
        .unwrap();
    assert!(service.creation_link(&actor, &request.id).await.is_err());
    for status in [
        "ready",
        "provisioning",
        "connected",
        "expired",
        "cancelled",
        "suspended",
    ] {
        state
            .db
            .collection::<bson::Document>(REQUESTS)
            .update_one(
                doc! {"_id":&request.id},
                doc! {"$set":{"creation_context":{"kind":"standalone"},"status":status}},
            )
            .await
            .unwrap();
        assert!(service.creation_link(&actor, &request.id).await.is_err());
    }
}

#[tokio::test]
async fn telegram_new_stale_instruction_cannot_dispatch_keyboard_or_regress_lifecycle() {
    for change in ["cancel", "ready", "expire", "revoke", "manager", "launch"] {
        let (state, actor, server) = fixture().await;
        let base = server.uri();
        let service = service(&state, &base);
        let headers = headers(&state).await;
        let (request, start) = start_request(&state, &actor, &server).await;
        Mock::given(method("POST"))
            .and(path(format!("/bot{MANAGER}/sendMessage")))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"ok":true,"result":{"message_id":1}}))
                    .set_delay(std::time::Duration::from_millis(250)),
            )
            .with_priority(1)
            .mount(&server)
            .await;
        let bytes = serde_json::to_vec(&command(&start, 10)).unwrap();
        let (outcome, ()) = tokio::join!(service.webhook(&headers, &bytes), async {
            for _ in 0..100 {
                if !server.received_requests().await.unwrap().is_empty() {
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
            assert!(!server.received_requests().await.unwrap().is_empty());
            match change {
                "cancel" => service.cancel(&actor, &request.id).await.unwrap(),
                "ready" => {
                    state.db.collection::<bson::Document>(REQUESTS).update_one(doc! {"_id":&request.id},doc! {"$set":{"status":"ready","telegram_bot_id":900_i64},"$inc":{"revision":1_i64}}).await.unwrap();
                }
                "expire" => {
                    state.db.collection::<bson::Document>(REQUESTS).update_one(doc! {"_id":&request.id},doc! {"$set":{"expires_at":bson::DateTime::from_chrono(Utc::now()-Duration::seconds(1))}}).await.unwrap();
                }
                "revoke" => {
                    state
                        .db
                        .collection::<bson::Document>(crate::models::user::COLLECTION_NAME)
                        .update_one(doc! {"_id":&actor}, doc! {"$set":{"is_active":false}})
                        .await
                        .unwrap();
                }
                "manager" => {
                    state
                        .db
                        .collection::<bson::Document>(CREDENTIALS)
                        .update_one(
                            doc! {"provider":"telegram-new"},
                            doc! {"$set":{"fields.observation_id":"replacement"}},
                        )
                        .await
                        .unwrap();
                }
                "launch" => {
                    service.launch(&actor, &request.id).await.unwrap();
                }
                _ => unreachable!(),
            }
        });
        outcome.unwrap();
        let sent = server.received_requests().await.unwrap();
        assert!(
            sent.iter()
                .all(|r| r.body_json::<Value>().unwrap()["reply_markup"]
                    .get("keyboard")
                    .is_none()),
            "{change}"
        );
        let saved = state
            .db
            .collection::<TelegramBotRequest>(REQUESTS)
            .find_one(doc! {"_id":&request.id})
            .await
            .unwrap()
            .unwrap();
        assert_ne!(
            saved.prompt_delivery.unwrap().outcome,
            PromptOutcome::Accepted,
            "{change}"
        );
        if change == "ready" {
            assert_eq!(saved.status, Status::Ready);
        }
    }
}

async fn hosted_request(
    state: &crate::AppState,
    actor: &str,
    server: &MockServer,
    group: bool,
) -> (TelegramBotRequest, String, String) {
    hosted_request_timed(state, actor, server, group, None).await
}

async fn hosted_request_timed(
    state: &crate::AppState,
    actor: &str,
    server: &MockServer,
    group: bool,
    deadlines: Option<(i64, i64)>,
) -> (TelegramBotRequest, String, String) {
    use super::super::channel_connect_link_service as links;
    use crate::models::nyxbot_channel::{NyxbotWatch, WATCHES_COLLECTION_NAME};
    let created = links::create(
        &state.db,
        &state.encryption_keys,
        links::CreateInput {
            owner: actor.into(),
            actor: actor.into(),
            platform: "telegram-new".into(),
            label: "Support".into(),
            requested_by: None,
            app_id: None,
            callback_url: None,
            webhook_url: None,
            expires_in: None,
        },
    )
    .await
    .unwrap();
    let watch_id = uuid::Uuid::new_v4().to_string();
    let agent_id = uuid::Uuid::new_v4().to_string();
    let source_id = uuid::Uuid::new_v4().to_string();
    let now = Utc::now();
    let watch = NyxbotWatch {
        id: watch_id.clone(),
        user_id: actor.into(),
        kind: "channel_bot".into(),
        conversation_id: source_id.clone(),
        status: "pending".into(),
        platform: Some("telegram".into()),
        agent_id: Some(agent_id.clone()),
        channel_bot_id: None,
        connect_link_id: Some(created.link.id.clone()),
        last_error: None,
        report_to: Some("original-report-target".into()),
        report_provenance: 1,
        invocation_id: Some("original-invocation".into()),
        trigger_id: None,
        trigger_prefill: None,
        checked_at: None,
        created_at: now,
        expires_at: now + Duration::seconds(deadlines.map_or(1800, |(_, watch)| watch)),
    };
    state
        .db
        .collection::<NyxbotWatch>(WATCHES_COLLECTION_NAME)
        .insert_one(watch)
        .await
        .unwrap();
    state
        .db
        .collection::<bson::Document>(crate::models::assistant_agent::COLLECTION_NAME)
        .insert_one(doc! {"_id":&agent_id,"user_id":actor,"destroyed_at":null})
        .await
        .unwrap();
    let group_id = group.then(|| uuid::Uuid::new_v4().to_string());
    state
        .db
        .collection::<bson::Document>(crate::models::assistant_conversation::COLLECTION_NAME)
        .insert_one(
            doc! {"_id":&source_id,"user_id":actor,"agent_id":&agent_id,"group_id":&group_id},
        )
        .await
        .unwrap();
    if let Some(id) = &group_id {
        state
            .db
            .collection::<bson::Document>(crate::models::assistant_group::COLLECTION_NAME)
            .insert_one(doc! {"_id":id,"user_id":actor,"member_agent_ids":[&agent_id]})
            .await
            .unwrap();
    }
    let mut claim = links::claim(&state.db, actor, &created.token, "telegram")
        .await
        .unwrap();
    if let Some((link_remaining, _)) = deadlines {
        // Simulate elapsed time after the claim pinned this link, without a
        // sleep or changing its creator/source/claim authority.
        claim.link.expires_at = now + Duration::seconds(link_remaining);
        state
            .db
            .collection::<bson::Document>(crate::models::channel_connect_link::COLLECTION_NAME)
            .update_one(
                doc! {"_id": &claim.link.id},
                doc! {"$set": {"expires_at": bson::DateTime::from_chrono(claim.link.expires_at)}},
            )
            .await
            .unwrap();
    }
    let base = server.uri();
    let service = service(state, &base);
    let (request, launch) = service
        .begin_linked(actor, actor, "Support", &claim)
        .await
        .unwrap();
    links::release(&state.db, &claim, false).await.unwrap();
    let challenge = reqwest::Url::parse(&launch)
        .unwrap()
        .query_pairs()
        .find(|(k, _)| k == "start")
        .unwrap()
        .1
        .to_string();
    (request, format!("/start {challenge}"), watch_id)
}

#[tokio::test]
async fn telegram_new_hosted_recovery_preserves_snapshot_and_hidden_group_source() {
    for group in [false, true] {
        let (state, actor, server) = fixture().await;
        let base = server.uri();
        let service = service(&state, &base);
        let headers = headers(&state).await;
        let (request, start, watch_id) = hosted_request(&state, &actor, &server, group).await;
        assert!(matches!(
            request.creation_context,
            Some(CreationContext::Hosted { watch: Some(_), .. })
        ));
        webhook(&service, &headers, command(&start, 10)).await;
        let before = state
            .db
            .collection::<bson::Document>(crate::models::nyxbot_channel::WATCHES_COLLECTION_NAME)
            .find_one(doc! {"_id":&watch_id})
            .await
            .unwrap()
            .unwrap();
        service.creation_link(&actor, &request.id).await.unwrap();
        let mut after = state
            .db
            .collection::<bson::Document>(crate::models::nyxbot_channel::WATCHES_COLLECTION_NAME)
            .find_one(doc! {"_id":&watch_id})
            .await
            .unwrap()
            .unwrap();
        let mut before = before;
        before.remove("telegram_creation_fence");
        after.remove("telegram_creation_fence");
        assert_eq!(before, after);
        assert_eq!(
            state
                .db
                .collection::<bson::Document>(BOTS)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
        assert_eq!(
            state
                .db
                .collection::<bson::Document>(crate::models::assistant_agent::COLLECTION_NAME)
                .count_documents(doc! {})
                .await
                .unwrap(),
            1
        );
    }
}

#[tokio::test]
async fn telegram_new_hosted_recovery_refuses_deleted_replaced_or_retargeted_authority() {
    use crate::models::{channel_connect_link as links, nyxbot_channel as watches};
    for change in [
        "link_deleted",
        "link_cancelled",
        "link_retargeted",
        "watch_deleted",
        "watch_replaced",
        "watch_retargeted",
        "watch_provenance",
        "watch_expired",
        "source_deleted",
        "agent_destroyed",
        "manager_changed",
    ] {
        let (state, actor, server) = fixture().await;
        let base = server.uri();
        let service = service(&state, &base);
        let headers = headers(&state).await;
        let (request, start, watch_id) = hosted_request(&state, &actor, &server, false).await;
        webhook(&service, &headers, command(&start, 10)).await;
        let watch = state
            .db
            .collection::<bson::Document>(watches::WATCHES_COLLECTION_NAME)
            .find_one(doc! {"_id":&watch_id})
            .await
            .unwrap()
            .unwrap();
        match change {
            "link_deleted" => {
                state
                    .db
                    .collection::<bson::Document>(links::COLLECTION_NAME)
                    .delete_one(doc! {"_id":&request.id})
                    .await
                    .unwrap();
            }
            "link_cancelled" => {
                state
                    .db
                    .collection::<bson::Document>(links::COLLECTION_NAME)
                    .update_one(
                        doc! {"_id":&request.id},
                        doc! {"$set":{"status":"cancelled"}},
                    )
                    .await
                    .unwrap();
            }
            "link_retargeted" => {
                state
                    .db
                    .collection::<bson::Document>(links::COLLECTION_NAME)
                    .update_one(
                        doc! {"_id":&request.id},
                        doc! {"$set":{"user_id":"other-owner"}},
                    )
                    .await
                    .unwrap();
            }
            "watch_deleted" => {
                state
                    .db
                    .collection::<bson::Document>(watches::WATCHES_COLLECTION_NAME)
                    .delete_one(doc! {"_id":&watch_id})
                    .await
                    .unwrap();
            }
            "watch_replaced" => {
                state
                    .db
                    .collection::<bson::Document>(watches::WATCHES_COLLECTION_NAME)
                    .delete_one(doc! {"_id":&watch_id})
                    .await
                    .unwrap();
                let mut replacement = watch.clone();
                replacement.insert("_id", uuid::Uuid::new_v4().to_string());
                state
                    .db
                    .collection::<bson::Document>(watches::WATCHES_COLLECTION_NAME)
                    .insert_one(replacement)
                    .await
                    .unwrap();
            }
            "watch_retargeted" => {
                state
                    .db
                    .collection::<bson::Document>(watches::WATCHES_COLLECTION_NAME)
                    .update_one(
                        doc! {"_id":&watch_id},
                        doc! {"$set":{"agent_id":"other-agent"}},
                    )
                    .await
                    .unwrap();
            }
            "watch_provenance" => {
                state
                    .db
                    .collection::<bson::Document>(watches::WATCHES_COLLECTION_NAME)
                    .update_one(
                        doc! {"_id":&watch_id},
                        doc! {"$set":{"report_to":"replacement-turn"}},
                    )
                    .await
                    .unwrap();
            }
            "watch_expired" => {
                state.db.collection::<bson::Document>(watches::WATCHES_COLLECTION_NAME).update_one(doc! {"_id":&watch_id},doc! {"$set":{"expires_at":bson::DateTime::from_chrono(Utc::now()-Duration::seconds(1))}}).await.unwrap();
            }
            "source_deleted" => {
                state
                    .db
                    .collection::<bson::Document>(
                        crate::models::assistant_conversation::COLLECTION_NAME,
                    )
                    .delete_one(doc! {"_id":watch.get_str("conversation_id").unwrap()})
                    .await
                    .unwrap();
            }
            "agent_destroyed" => {
                state
                    .db
                    .collection::<bson::Document>(crate::models::assistant_agent::COLLECTION_NAME)
                    .update_one(
                        doc! {"_id":watch.get_str("agent_id").unwrap()},
                        doc! {"$set":{"destroyed_at":bson::DateTime::now()}},
                    )
                    .await
                    .unwrap();
            }
            "manager_changed" => {
                state
                    .db
                    .collection::<bson::Document>(CREDENTIALS)
                    .update_one(
                        doc! {"provider":"telegram-new"},
                        doc! {"$set":{"fields.observation_id":"new-observation"}},
                    )
                    .await
                    .unwrap();
            }
            _ => unreachable!(),
        }
        assert!(
            service.creation_link(&actor, &request.id).await.is_err(),
            "{change}"
        );
        let before = server.received_requests().await.unwrap().len();
        let body = serde_json::to_vec(&command("/start", 11)).unwrap();
        assert_eq!(
            crate::handlers::telegram_new::webhook_with_service(&state, &service, &headers, &body,)
                .await
                .unwrap(),
            axum::http::StatusCode::OK
        );
        let mut creation = message(json!({"managed_bot_created": {"bot": bot()}}));
        creation["update_id"] = json!(12);
        let body = serde_json::to_vec(&creation).unwrap();
        assert_eq!(
            crate::handlers::telegram_new::webhook_with_service(&state, &service, &headers, &body,)
                .await
                .unwrap(),
            axum::http::StatusCode::OK
        );
        let saved = state
            .db
            .collection::<TelegramBotRequest>(REQUESTS)
            .find_one(doc! {"_id": &request.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(saved.status, Status::WaitingBot, "{change}");
        assert!(saved.telegram_bot_id.is_none(), "{change}");
        assert_eq!(
            state
                .db
                .collection::<bson::Document>(crate::models::telegram_bot_claim::COLLECTION_NAME)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0,
            "{change}"
        );
        let sent = server.received_requests().await.unwrap();
        assert_eq!(sent.len(), before + 1, "{change}");
        let refusal = sent[before].body_json::<Value>().unwrap();
        assert!(
            refusal["text"]
                .as_str()
                .unwrap()
                .contains("setup is no longer available"),
            "{change}"
        );
        assert!(
            !refusal["text"].as_str().unwrap().contains("expired"),
            "{change}"
        );
        assert_eq!(refusal["reply_markup"]["remove_keyboard"], true);
        assert!(refusal["reply_markup"]["keyboard"].is_null());
    }
}

#[tokio::test]
async fn telegram_new_watch_revocation_between_instructions_and_keyboard_stops_dispatch() {
    let (state, actor, server) = fixture().await;
    let base = server.uri();
    let service = service(&state, &base);
    let headers = headers(&state).await;
    let (request, start, watch_id) = hosted_request(&state, &actor, &server, false).await;
    Mock::given(method("POST"))
        .and(path(format!("/bot{MANAGER}/sendMessage")))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"ok":true,"result":{"message_id":1}}))
                .set_delay(std::time::Duration::from_millis(250)),
        )
        .with_priority(1)
        .mount(&server)
        .await;
    let bytes = serde_json::to_vec(&command(&start, 10)).unwrap();
    let (outcome, ()) = tokio::join!(service.webhook(&headers, &bytes), async {
        for _ in 0..100 {
            if !server.received_requests().await.unwrap().is_empty() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        state
            .db
            .collection::<bson::Document>(crate::models::nyxbot_channel::WATCHES_COLLECTION_NAME)
            .delete_one(doc! {"_id":&watch_id})
            .await
            .unwrap();
    });
    outcome.unwrap();
    let sent = server.received_requests().await.unwrap();
    assert_eq!(sent.len(), 2);
    assert!(
        sent[1].body_json::<Value>().unwrap()["text"]
            .as_str()
            .unwrap()
            .contains("setup is no longer available")
    );
    assert!(
        sent.iter()
            .all(|r| r.body_json::<Value>().unwrap()["reply_markup"]["keyboard"].is_null())
    );
    let saved = service.get(&actor, &request.id).await.unwrap();
    assert_eq!(saved.status, Status::WaitingBot);
    assert_ne!(
        saved.prompt_delivery.unwrap().outcome,
        PromptOutcome::Accepted
    );
}

#[tokio::test]
async fn telegram_new_closed_hosted_authority_is_acked_but_database_admission_errors_retry() {
    use axum::http::StatusCode;
    let (state, actor, server) = fixture().await;
    let base = server.uri();
    let service = service(&state, &base);
    let headers = headers(&state).await;
    let (request, start, watch_id) = hosted_request(&state, &actor, &server, false).await;
    webhook(&service, &headers, command(&start, 10)).await;
    let before = server.received_requests().await.unwrap().len();
    state
        .db
        .collection::<bson::Document>(crate::models::nyxbot_channel::WATCHES_COLLECTION_NAME)
        .update_one(
            doc! {"_id":&watch_id},
            doc! {"$set":{"agent_id":"replacement"}},
        )
        .await
        .unwrap();
    for update_id in [11, 11, 12] {
        let body = serde_json::to_vec(&command("/start", update_id)).unwrap();
        assert_eq!(
            crate::handlers::telegram_new::webhook_with_service(&state, &service, &headers, &body)
                .await
                .unwrap(),
            StatusCode::OK
        );
    }
    let sent = server.received_requests().await.unwrap();
    assert_eq!(sent.len(), before + 3);
    assert!(sent[before..].iter().all(|r| {
        let body = r.body_json::<Value>().unwrap();
        body["text"]
            .as_str()
            .unwrap()
            .contains("setup is no longer available")
            && body["reply_markup"]["keyboard"].is_null()
    }));
    // A real collection validator rejects the pre-dispatch metadata write. It
    // must remain retryable, unlike a deliberately closed capability.
    state
        .db
        .collection::<bson::Document>(REQUESTS)
        .update_one(
            doc! {"_id":&request.id},
            doc! {"$set":{"creation_context":{"kind":"standalone"}}},
        )
        .await
        .unwrap();
    state.db.run_command(doc! {"collMod":REQUESTS,"validator":{"$expr":{"$eq":[1,0]}},"validationLevel":"strict"}).await.unwrap();
    let result = crate::handlers::telegram_new::webhook_with_service(
        &state,
        &service,
        &headers,
        &serde_json::to_vec(&command("/start", 13)).unwrap(),
    )
    .await;
    assert!(
        matches!(result, Err(crate::errors::AppError::DatabaseError(_))),
        "{result:?}"
    );
    assert_eq!(server.received_requests().await.unwrap().len(), before + 3);
}

#[tokio::test]
async fn telegram_new_creation_link_handler_is_human_only_empty_rate_limited_and_private() {
    use crate::mw::auth::AuthMethod;
    use axum::{
        body::Bytes,
        extract::{Path, State},
    };
    let (state, actor, server) = fixture().await;
    let base = server.uri();
    let service = service(&state, &base);
    let (request, start) = start_request(&state, &actor, &server).await;
    webhook(&service, &headers(&state).await, command(&start, 10)).await;
    let before = server.received_requests().await.unwrap().len();
    for method in [
        AuthMethod::ApiKey,
        AuthMethod::ServiceAccount,
        AuthMethod::Delegated,
        AuthMethod::Relay,
    ] {
        let mut auth = crate::test_utils::test_auth_user(&actor);
        auth.auth_method = method;
        let result = crate::handlers::telegram_new::creation_link(
            State(state.clone()),
            auth,
            Path(request.id.clone()),
            Bytes::new(),
        )
        .await;
        assert!(matches!(result, Err(AppError::Forbidden(_))));
    }
    let mut delegated_person = crate::test_utils::test_auth_user(&actor);
    delegated_person.auth_method = AuthMethod::AccessToken;
    delegated_person.acting_client_id = Some("unrelated-client".into());
    assert!(matches!(
        crate::handlers::telegram_new::creation_link(
            State(state.clone()),
            delegated_person,
            Path(request.id.clone()),
            Bytes::new(),
        )
        .await,
        Err(AppError::Forbidden(_))
    ));
    assert!(matches!(
        crate::handlers::telegram_new::creation_link(
            State(state.clone()),
            crate::test_utils::test_auth_user(&actor),
            Path(request.id.clone()),
            Bytes::from_static(b"{\"redirect\":\"https://evil.example\"}"),
        )
        .await,
        Err(AppError::BadRequest(_))
    ));
    for _ in 0..4 {
        let (headers, axum::Json(response)) = crate::handlers::telegram_new::creation_link(
            State(state.clone()),
            crate::test_utils::test_auth_user(&actor),
            Path(request.id.clone()),
            Bytes::new(),
        )
        .await
        .unwrap();
        assert_eq!(headers["cache-control"], "no-store");
        assert_eq!(headers["referrer-policy"], "no-referrer");
        assert_eq!(response.request.id, request.id);
        assert!(
            response
                .creation_url
                .starts_with("https://t.me/newbot/NyxSetupBot/")
        );
    }
    assert!(matches!(
        crate::handlers::telegram_new::creation_link(
            State(state.clone()),
            crate::test_utils::test_auth_user(&actor),
            Path(request.id.clone()),
            Bytes::new(),
        )
        .await,
        Err(AppError::RateLimited)
    ));
    assert_eq!(server.received_requests().await.unwrap().len(), before);
}

#[tokio::test]
async fn telegram_new_overlapping_commands_and_outcome_write_failure_never_replay_keyboard() {
    let (state, actor, server) = fixture().await;
    let base = server.uri();
    let service = service(&state, &base);
    let headers = headers(&state).await;
    let (request, start) = start_request(&state, &actor, &server).await;
    // Hold only the keyboard response. A second distinct command cannot replace
    // its live dispatch claim; a failed final metadata write remains unknown.
    Mock::given(method("POST"))
        .and(path(format!("/bot{MANAGER}/sendMessage")))
        .and(|r: &wiremock::Request| {
            r.body_json::<Value>()
                .unwrap()
                .get("reply_markup")
                .is_some()
        })
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"ok":true,"result":{"message_id":2}}))
                .set_delay(std::time::Duration::from_millis(500)),
        )
        .with_priority(1)
        .mount(&server)
        .await;
    let bytes = serde_json::to_vec(&command(&start, 10)).unwrap();
    let (result, ()) = tokio::join!(service.webhook(&headers, &bytes), async {
        for _ in 0..100 {
            if server.received_requests().await.unwrap().len() == 2 {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert_eq!(server.received_requests().await.unwrap().len(), 2);
        webhook(&service, &headers, command("/start", 11)).await;
        state.db.run_command(doc! {"collMod":REQUESTS,"validator":{"$expr":{"$eq":[1,0]}},"validationLevel":"strict"}).await.unwrap();
    });
    result.unwrap();
    let saved = state
        .db
        .collection::<TelegramBotRequest>(REQUESTS)
        .find_one(doc! {"_id": &request.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        saved.prompt_delivery.as_ref().unwrap().phase,
        PromptPhase::Keyboard
    );
    assert_eq!(
        saved.prompt_delivery.as_ref().unwrap().outcome,
        PromptOutcome::Sending
    );
    state
        .db
        .run_command(doc! {"collMod":REQUESTS,"validator":{}})
        .await
        .unwrap();
    webhook(&service, &headers, command(&start, 10)).await;
    assert_eq!(server.received_requests().await.unwrap().len(), 2);
    state.db.collection::<bson::Document>(REQUESTS).update_one(doc! {"_id":&request.id},
        doc! {"$set":{"prompt_delivery.attempted_at":bson::DateTime::from_chrono(Utc::now()-Duration::seconds(46))}}).await.unwrap();
    let stale = service.get(&actor, &request.id).await.unwrap();
    let response = crate::handlers::telegram_new::RequestResponse::from(stale);
    assert_eq!(
        response.prompt_delivery.unwrap().outcome,
        PromptOutcome::Unknown
    );
    webhook(&service, &headers, command(&start, 10)).await;
    assert_eq!(server.received_requests().await.unwrap().len(), 2);
    webhook(&service, &headers, command("/start", 12)).await;
    assert_eq!(server.received_requests().await.unwrap().len(), 4);
}

#[tokio::test]
async fn telegram_new_consumption_contention_retries_and_only_one_fresh_bot_is_consumed() {
    let (state, actor, server) = fixture().await;
    let base = server.uri();
    let service = service(&state, &base);
    let headers = headers(&state).await;
    let (request, start) = start_request(&state, &actor, &server).await;
    webhook(&service, &headers, command(&start, 10)).await;
    service.creation_link(&actor, &request.id).await.unwrap();
    webhook(
        &service,
        &headers,
        json!({"update_id":11,"managed_bot":{"user":{"id":700,"is_bot":false},"bot":bot()}}),
    )
    .await;
    let mut creation = message(json!({"managed_bot_created":{"bot":bot()}}));
    creation["update_id"] = json!(12);
    let body = serde_json::to_vec(&creation).unwrap();
    let mut held = state.db.client().start_session().await.unwrap();
    held.start_transaction().await.unwrap();
    state
        .db
        .collection::<bson::Document>(REQUESTS)
        .update_one(
            doc! {"_id":&request.id},
            doc! {"$inc":{"test_contention":1_i64}},
        )
        .session(&mut held)
        .await
        .unwrap();
    let outcome =
        crate::handlers::telegram_new::webhook_with_service(&state, &service, &headers, &body)
            .await;
    // A transaction conflict is retryable even if the shared transaction mapper
    // presents it as Conflict. It must never become a successful acknowledgement.
    assert!(outcome.is_err(), "{outcome:?}");
    held.abort_transaction().await.unwrap();
    assert_eq!(
        service.get(&actor, &request.id).await.unwrap().status,
        Status::WaitingBot
    );
    assert_eq!(
        crate::handlers::telegram_new::webhook_with_service(&state, &service, &headers, &body)
            .await
            .unwrap(),
        axum::http::StatusCode::OK
    );
    let saved = service.get(&actor, &request.id).await.unwrap();
    assert_eq!(saved.status, Status::Ready);
    let revision = saved.revision;
    webhook(&service, &headers, creation.clone()).await;
    assert_eq!(
        service.get(&actor, &request.id).await.unwrap().revision,
        revision
    );
    provider_connection(&server, &request.id, 200).await;
    let (first, second) = tokio::join!(
        service.complete_pending_creations(),
        service.complete_pending_creations()
    );
    first.unwrap();
    second.unwrap();
    webhook(&service, &headers, creation).await;
    service.complete_pending_creations().await.unwrap();
    assert_eq!(
        state
            .db
            .collection::<ChannelBot>(BOTS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        state
            .db
            .collection::<bson::Document>(REQUESTS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        state
            .db
            .collection::<bson::Document>(crate::models::telegram_bot_claim::COLLECTION_NAME)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    let calls = server.received_requests().await.unwrap();
    for method in ["getManagedBotToken", "setWebhook"] {
        assert_eq!(
            calls
                .iter()
                .filter(|r| r.url.path().ends_with(method))
                .count(),
            1
        );
    }
}

#[tokio::test]
async fn telegram_new_org_hosted_creator_and_initiating_admin_remain_distinct() {
    use super::super::{channel_connect_link_service as links, org_service};
    use crate::models::org_membership::{MemberScopeSource, OrgRole};
    for demoted in [false, true] {
        let (state, creator, server) = fixture().await;
        let starter = uuid::Uuid::new_v4().to_string();
        let third = uuid::Uuid::new_v4().to_string();
        for person in [&starter, &third] {
            state
                .db
                .collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
                .insert_one(test_user(person, crate::models::user::UserType::Person))
                .await
                .unwrap();
        }
        let org = org_service::create_org_user(&state.db, "Shared setup", None, None)
            .await
            .unwrap();
        for person in [&creator, &starter, &third] {
            org_service::create_membership(
                &state.db,
                &org.id,
                person,
                OrgRole::Admin,
                MemberScopeSource::Inherit,
                None,
            )
            .await
            .unwrap();
        }
        let created = links::create(
            &state.db,
            &state.encryption_keys,
            links::CreateInput {
                owner: org.id.clone(),
                actor: creator.clone(),
                platform: "telegram-new".into(),
                label: "Shared support".into(),
                requested_by: None,
                app_id: None,
                callback_url: None,
                webhook_url: None,
                expires_in: None,
            },
        )
        .await
        .unwrap();
        let claim = links::claim(&state.db, &starter, &created.token, "telegram")
            .await
            .unwrap();
        let base = server.uri();
        let service = service(&state, &base);
        let (request, launch) = service
            .begin_linked(&starter, &org.id, "Shared support", &claim)
            .await
            .unwrap();
        links::release(&state.db, &claim, false).await.unwrap();
        match request.creation_context.as_ref().unwrap() {
            CreationContext::Hosted {
                link_created_by_user_id,
                ..
            } => assert_eq!(link_created_by_user_id, &creator),
            _ => panic!("Expected captured hosted origin"),
        }
        let challenge = reqwest::Url::parse(&launch)
            .unwrap()
            .query_pairs()
            .find(|(key, _)| key == "start")
            .unwrap()
            .1
            .to_string();
        let headers = headers(&state).await;
        webhook(
            &service,
            &headers,
            command(&format!("/start {challenge}"), 10),
        )
        .await;
        assert_eq!(server.received_requests().await.unwrap().len(), 2);
        assert!(service.creation_link(&third, &request.id).await.is_err());
        assert!(service.creation_link(&creator, &request.id).await.is_err());
        service.creation_link(&starter, &request.id).await.unwrap();
        if demoted {
            state
                .db
                .collection::<bson::Document>(crate::models::org_membership::COLLECTION_NAME)
                .update_one(
                    doc! {"org_user_id": &org.id, "member_user_id": &starter},
                    doc! {"$set":{"role":"viewer"}},
                )
                .await
                .unwrap();
        }
        let mut creation = message(json!({"managed_bot_created":{"bot":bot()}}));
        creation["update_id"] = json!(12);
        webhook(&service, &headers, creation).await;
        let saved = state
            .db
            .collection::<TelegramBotRequest>(REQUESTS)
            .find_one(doc! {"_id": &request.id})
            .await
            .unwrap()
            .unwrap();
        if demoted {
            assert!(service.creation_link(&starter, &request.id).await.is_err());
        }
        assert_eq!(saved.actor_user_id, starter);
        assert_eq!(saved.owner_user_id, org.id);
        assert_eq!(
            saved.status,
            if demoted {
                Status::Cancelled
            } else {
                Status::Ready
            }
        );
        assert_eq!(
            saved.telegram_bot_id,
            if demoted { None } else { Some(900) }
        );
    }
}

#[tokio::test]
async fn telegram_new_hosted_grace_uses_effective_minimum_for_prompt_recovery_and_creation() {
    for (link_remaining, watch_remaining) in [(-60, 120), (-1700, 600)] {
        let (state, actor, server) = fixture().await;
        let base = server.uri();
        let service = service(&state, &base);
        let headers = headers(&state).await;
        let (request, start, _) = hosted_request_timed(
            &state,
            &actor,
            &server,
            false,
            Some((link_remaining, watch_remaining)),
        )
        .await;
        let link = super::super::channel_connect_link_service::get(&state.db, &request.id)
            .await
            .unwrap();
        let Some(CreationContext::Hosted {
            watch: Some(watch), ..
        }) = &request.creation_context
        else {
            panic!("missing hosted snapshot")
        };
        assert!(link.expires_at < Utc::now());
        assert_eq!(
            request.expires_at,
            super::super::channel_connect_link_service::effective_deadline(&link)
                .min(watch.expires_at)
        );
        webhook(&service, &headers, command(&start, 10)).await;
        let bound = service.get(&actor, &request.id).await.unwrap();
        assert!(
            crate::handlers::telegram_new::projected_request(&state, bound)
                .await
                .unwrap()
                .can_open_creation_form
        );
        service.creation_link(&actor, &request.id).await.unwrap();
        webhook(&service, &headers, command("/start", 11)).await;
        let sent = server.received_requests().await.unwrap();
        assert_eq!(sent.len(), 4);
        assert_eq!(
            sent.iter()
                .filter(|r| r.body_json::<Value>().unwrap()["reply_markup"]["keyboard"].is_array())
                .count(),
            2
        );
        let mut created = message(json!({"managed_bot_created":{"bot":bot()}}));
        created["update_id"] = json!(12);
        webhook(&service, &headers, created).await;
        let saved = service.get(&actor, &request.id).await.unwrap();
        assert_eq!(saved.status, Status::Ready);
        assert_eq!(saved.telegram_bot_id, Some(900));
        assert_eq!(saved.start_update_id, Some(10));
        assert_eq!(saved.expires_at, request.expires_at);
        assert_eq!(
            state
                .db
                .collection::<bson::Document>(crate::models::telegram_bot_claim::COLLECTION_NAME)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
        state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn telegram_new_existing_hosted_request_keeps_earlier_stored_deadline() {
    let (state, actor, server) = fixture().await;
    let base = server.uri();
    let service = service(&state, &base);
    let headers = headers(&state).await;
    let (request, start, _) =
        hosted_request_timed(&state, &actor, &server, false, Some((60, 2400))).await;
    // Model a context-bearing request inserted by the previous binary with its
    // original 15-minute deadline. New link grace must not renew that authority.
    let earlier = bson::DateTime::from_chrono(Utc::now() + Duration::minutes(15));
    assert!(earlier.to_chrono() < request.expires_at);
    state
        .db
        .collection::<bson::Document>(REQUESTS)
        .update_one(
            doc! {"_id": &request.id},
            doc! {"$set": {"expires_at": earlier}},
        )
        .await
        .unwrap();
    webhook(&service, &headers, command(&start, 10)).await;
    service.creation_link(&actor, &request.id).await.unwrap();
    webhook(&service, &headers, command("/start", 11)).await;
    let mut created = message(json!({"managed_bot_created":{"bot":bot()}}));
    created["update_id"] = json!(12);
    webhook(&service, &headers, created).await;
    let saved = service.get(&actor, &request.id).await.unwrap();
    assert_eq!(saved.status, Status::Ready);
    assert_eq!(saved.telegram_bot_id, Some(900));
    assert_eq!(saved.expires_at, earlier.to_chrono());
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn telegram_new_expired_start_replies_and_owner_projection_never_offers_recovery() {
    for bound in [false, true] {
        let (state, actor, server) = fixture().await;
        let base = server.uri();
        let service = service(&state, &base);
        let headers = headers(&state).await;
        let (request, start, _) =
            hosted_request_timed(&state, &actor, &server, false, Some((-60, 120))).await;
        if bound {
            webhook(&service, &headers, command(&start, 10)).await;
        }
        // Move the established effective deadline into the past, as in the
        // existing expiry fixtures; no wall-clock sleeps or authority renewal.
        state.db.collection::<bson::Document>(REQUESTS).update_one(doc! {"_id":&request.id},
            doc! {"$set":{"expires_at":bson::DateTime::from_chrono(Utc::now()-Duration::seconds(1))}}).await.unwrap();
        let before = server.received_requests().await.unwrap().len();
        webhook(&service, &headers, command(&start, 11)).await;
        if bound {
            webhook(&service, &headers, command("/start", 12)).await;
        }
        let saved = service.get(&actor, &request.id).await.unwrap();
        assert_eq!(saved.status, Status::Expired);
        assert!(!saved.active);
        assert!(
            !crate::handlers::telegram_new::projected_request(&state, saved)
                .await
                .unwrap()
                .can_open_creation_form
        );
        assert!(service.creation_link(&actor, &request.id).await.is_err());
        let sent = server.received_requests().await.unwrap();
        assert_eq!(sent.len() - before, if bound { 2 } else { 1 });
        for reply in &sent[before..] {
            let body = reply.body_json::<Value>().unwrap();
            assert!(body["text"].as_str().unwrap().contains("setup has expired"));
            assert_eq!(body["reply_markup"]["remove_keyboard"], true);
            assert!(body["reply_markup"]["keyboard"].is_null());
        }
        assert_eq!(
            state
                .db
                .collection::<bson::Document>(BOTS)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
        state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn telegram_new_recovery_projection_checks_live_context_without_issuance_writes() {
    let (state, actor, server) = fixture().await;
    let base = server.uri();
    let service = service(&state, &base);
    let headers = headers(&state).await;
    let (request, start, watch_id) = hosted_request(&state, &actor, &server, true).await;
    webhook(&service, &headers, command(&start, 10)).await;
    let saved = service.get(&actor, &request.id).await.unwrap();
    let before = state
        .db
        .collection::<bson::Document>(REQUESTS)
        .find_one(doc! {"_id":&request.id})
        .await
        .unwrap()
        .unwrap();
    assert!(
        crate::handlers::telegram_new::projected_request(&state, saved.clone())
            .await
            .unwrap()
            .can_open_creation_form
    );
    assert_eq!(
        state
            .db
            .collection::<bson::Document>(REQUESTS)
            .find_one(doc! {"_id":&request.id})
            .await
            .unwrap()
            .unwrap(),
        before
    );
    for change in ["revision", "ready", "cancelled", "context"] {
        let update = match change {
            "revision" => doc! {"$inc":{"revision":1_i64}},
            "ready" => doc! {"$set":{"status":"ready","telegram_bot_id":900_i64}},
            "cancelled" => doc! {"$set":{"status":"cancelled","active":false}},
            "context" => doc! {"$set":{"creation_context":{"kind":"standalone"}}},
            _ => unreachable!(),
        };
        state
            .db
            .collection::<bson::Document>(REQUESTS)
            .update_one(doc! {"_id":&request.id}, update)
            .await
            .unwrap();
        assert!(
            !crate::handlers::telegram_new::projected_request(&state, saved.clone())
                .await
                .unwrap()
                .can_open_creation_form,
            "{change}"
        );
        state
            .db
            .collection::<bson::Document>(REQUESTS)
            .replace_one(doc! {"_id":&request.id}, before.clone())
            .await
            .unwrap();
    }
    state
        .db
        .collection::<bson::Document>(crate::models::nyxbot_channel::WATCHES_COLLECTION_NAME)
        .delete_one(doc! {"_id":watch_id})
        .await
        .unwrap();
    assert!(
        !crate::handlers::telegram_new::projected_request(&state, saved)
            .await
            .unwrap()
            .can_open_creation_form
    );
    assert!(service.creation_link(&actor, &request.id).await.is_err());
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn telegram_new_concurrent_owners_each_receive_one_creation_keyboard() {
    let (state, first, server) = fixture().await;
    let second = uuid::Uuid::new_v4().to_string();
    state
        .db
        .collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
        .insert_one(test_user(&second, crate::models::user::UserType::Person))
        .await
        .unwrap();
    let base = server.uri();
    let service = service(&state, &base);
    let headers = headers(&state).await;
    let (first_request, first_start) = start_request(&state, &first, &server).await;
    let (second_request, second_start) = start_request(&state, &second, &server).await;
    let first_update = serde_json::to_vec(&command(&first_start, 10)).unwrap();
    let mut second_update = command(&second_start, 11);
    second_update["message"]["from"]["id"] = json!(701);
    second_update["message"]["chat"]["id"] = json!(701);
    let second_update = serde_json::to_vec(&second_update).unwrap();
    let (first_result, second_result) = tokio::join!(
        service.webhook(&headers, &first_update),
        service.webhook(&headers, &second_update)
    );
    first_result.unwrap();
    second_result.unwrap();
    let sent = server.received_requests().await.unwrap();
    assert_eq!(sent.len(), 4);
    for (actor, request, sender, update) in [
        (&first, first_request, 700, 10),
        (&second, second_request, 701, 11),
    ] {
        let messages: Vec<Value> = sent
            .iter()
            .map(|r| r.body_json::<Value>().unwrap())
            .filter(|body| body["chat_id"] == sender)
            .collect();
        assert_eq!(messages.len(), 2);
        assert!(messages[0]["reply_markup"].is_null());
        assert!(messages[1]["reply_markup"]["keyboard"][0][0]["request_managed_bot"].is_object());
        let saved = service.get(actor, &request.id).await.unwrap();
        assert_eq!(saved.start_update_id, Some(update));
        assert_eq!(saved.prompt_update_boundary, Some(update));
        assert_eq!(saved.revision, request.revision + 1);
        assert_eq!(
            saved.prompt_delivery.unwrap().outcome,
            PromptOutcome::Accepted
        );
    }
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn telegram_new_pre_keyboard_manager_write_conflict_retries_without_replaying_instructions() {
    use mongodb::event::{EventHandler, command::CommandEvent};
    use std::sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    };
    let armed = Arc::new(AtomicBool::new(false));
    let injected = Arc::new(AtomicUsize::new(0));
    let failed = Arc::new(AtomicUsize::new(0));
    let target = Arc::new(Mutex::new(None::<(String, String)>));
    let statement = Arc::new(Mutex::new(None::<i32>));
    let observer = EventHandler::callback({
        let (armed, injected, failed, target, statement) = (
            armed.clone(),
            injected.clone(),
            failed.clone(),
            target.clone(),
            statement.clone(),
        );
        move |event| match event {
            CommandEvent::Started(event)
                if event.command_name == "update"
                    && event.command.get_str("update") == Ok(CREDENTIALS)
                    && event.command.get_bool("autocommit") == Ok(false) =>
            {
                let Some((database, manager_id)) = target.lock().unwrap().clone() else {
                    return;
                };
                if event.db != database {
                    return;
                }
                let update = event.command.get_array("updates").unwrap()[0]
                    .as_document()
                    .unwrap();
                let query = update.get_document("q").unwrap();
                if query.get_str("fields.manager_bot_id") != Ok("100")
                    || query.get_str("fields.observation_id") != Ok("test-observation")
                    || !armed.swap(false, Ordering::SeqCst)
                {
                    return;
                }
                *statement.lock().unwrap() = Some(event.request_id);
                // The transaction has already real-write-fenced its actor, so
                // its snapshot predates this independent manager write. Complete
                // it BEFORE dispatching this statement to force MongoDB112.
                let worker = std::thread::spawn(move || {
                    let runtime = tokio::runtime::Builder::new_current_thread()
                        .enable_all()
                        .build()
                        .unwrap();
                    let result = runtime.block_on(async {
                        tokio::time::timeout(std::time::Duration::from_secs(5), async {
                            let uri = std::env::var("NYXID_TEST_DATABASE_URL")
                                .expect("explicit isolated Mongo URI required");
                            let mut options = mongodb::options::ClientOptions::parse(uri).await?;
                            options.server_selection_timeout =
                                Some(std::time::Duration::from_secs(2));
                            options.connect_timeout = Some(std::time::Duration::from_secs(2));
                            let client = mongodb::Client::with_options(options)?;
                            client
                                .database(&database)
                                .collection::<bson::Document>(CREDENTIALS)
                                .update_one(
                                    doc! {"_id":manager_id},
                                    doc! {"$inc":{"telegram_creation_fence":1_i64}},
                                )
                                .await
                        })
                        .await
                    });
                    runtime.shutdown_timeout(std::time::Duration::from_secs(1));
                    let Ok(Ok(write)) = result else {
                        panic!("bounded independent manager collision write failed")
                    };
                    assert_eq!(write.modified_count, 1);
                });
                assert!(worker.join().is_ok(), "manager collision worker failed");
                injected.fetch_add(1, Ordering::SeqCst);
            }
            CommandEvent::Failed(event) if *statement.lock().unwrap() == Some(event.request_id) => {
                assert!(event.failure.contains_label("TransientTransactionError"));
                assert!(
                    matches!(event.failure.kind.as_ref(), mongodb::error::ErrorKind::Command(error) if error.code == 112)
                );
                failed.fetch_add(1, Ordering::SeqCst);
            }
            _ => {}
        }
    });
    let db = crate::test_utils::connect_transaction_test_database_with_command_handler(
        "telegram_keyboard_collision",
        observer,
    )
    .await;
    let (state, actor, server) = fixture_with_database(db).await;
    let manager = state
        .db
        .collection::<bson::Document>(CREDENTIALS)
        .find_one(doc! {"provider":"telegram-new"})
        .await
        .unwrap()
        .unwrap();
    *target.lock().unwrap() = Some((
        state.db.name().to_owned(),
        manager.get_str("_id").unwrap().to_owned(),
    ));
    let base = server.uri();
    let service = service(&state, &base);
    let headers = headers(&state).await;
    let (request, start) = start_request(&state, &actor, &server).await;
    Mock::given(method("POST"))
        .and(path(format!("/bot{MANAGER}/sendMessage")))
        .and(|r: &wiremock::Request| {
            r.body_json::<Value>()
                .unwrap()
                .get("reply_markup")
                .is_none()
        })
        .respond_with(move |_: &wiremock::Request| {
            armed.store(true, Ordering::SeqCst);
            ResponseTemplate::new(200).set_body_json(json!({"ok":true,"result":{"message_id":50}}))
        })
        .with_priority(1)
        .expect(1)
        .mount(&server)
        .await;
    webhook(&service, &headers, command(&start, 10)).await;
    assert_eq!(injected.load(Ordering::SeqCst), 1);
    assert_eq!(failed.load(Ordering::SeqCst), 1);
    let sent = server.received_requests().await.unwrap();
    assert_eq!(sent.len(), 2);
    assert!(sent[0].body_json::<Value>().unwrap()["reply_markup"].is_null());
    assert!(sent[1].body_json::<Value>().unwrap()["reply_markup"]["keyboard"][0][0]["request_managed_bot"].is_object());
    let saved = service.get(&actor, &request.id).await.unwrap();
    assert_eq!(saved.start_update_id, Some(10));
    assert_eq!(saved.prompt_update_boundary, Some(10));
    assert_eq!(saved.revision, request.revision + 1);
    assert_eq!(
        saved.prompt_delivery.unwrap().outcome,
        PromptOutcome::Accepted
    );
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn telegram_new_expired_history_does_not_intercept_newer_setup_or_platform_routing() {
    use crate::models::platform_bot_config::{PlatformBotConfig, PlatformExecutionProfile};

    let (state, actor, server) = fixture().await;
    let base = server.uri();
    let service = service(&state, &base);
    let headers = headers(&state).await;
    let (request, start) = start_request(&state, &actor, &server).await;
    let mut old = request.clone();
    old.id = uuid::Uuid::new_v4().to_string();
    old.created_at -= Duration::hours(1);
    old.expires_at = Utc::now() - Duration::minutes(30);
    old.status = Status::Expired;
    old.active = false;
    old.telegram_user_id = Some(700);
    old.challenge_hash = "old-expired-fixture-hash".into();
    state
        .db
        .collection::<TelegramBotRequest>(REQUESTS)
        .insert_one(&old)
        .await
        .unwrap();
    webhook(&service, &headers, command(&start, 10)).await;
    for (status, active) in [
        (Status::WaitingBot, true),
        (Status::Connected, false),
        (Status::Cancelled, false),
    ] {
        state
            .db
            .collection::<bson::Document>(REQUESTS)
            .update_one(
                doc! {"_id":&request.id},
                doc! {"$set":{"status":bson::to_bson(&status).unwrap(),"active":active}},
            )
            .await
            .unwrap();
        let before = server.received_requests().await.unwrap().len();
        webhook(&service, &headers, command("/start", 11 + before as i64)).await;
        let sent = server.received_requests().await.unwrap();
        assert_eq!(sent.len(), before + 2);
        assert!(sent[before..].iter().all(|r| {
            !r.body_json::<Value>().unwrap()["text"]
                .as_str()
                .unwrap()
                .contains("expired")
        }));
        assert!(
            sent[before + 1].body_json::<Value>().unwrap()["reply_markup"]["keyboard"].is_array()
        );
    }
    // An established opted-in manager retains its platform route even when the
    // latest bound historical setup itself is expired.
    state
        .db
        .collection::<bson::Document>(REQUESTS)
        .delete_one(doc! {"_id":&request.id})
        .await
        .unwrap();
    let manager = register_manager_channel(&state, &actor, &server).await;
    let funding_owner = super::super::org_service::create_org_user(
        &state.db,
        "Platform routing fixture",
        None,
        None,
    )
    .await
    .unwrap();
    let now = Utc::now();
    let profile = PlatformBotConfig {
        id: uuid::Uuid::new_v4().to_string(),
        channel_bot_id: manager.id.clone(),
        manager_bot_id: "100".into(),
        identity_version: None,
        native_entry: None,
        observation_id: "test-observation".into(),
        execution_profile: PlatformExecutionProfile::PlatformOnboarding,
        generation: 1,
        enabled: true,
        guide: None,
        funding_owner_user_id: funding_owner.id,
        configured_by: actor.clone(),
        created_at: now,
        updated_at: now,
        deleted_at: None,
    };
    state
        .db
        .collection::<bson::Document>(BOTS)
        .update_one(
            doc! {"_id":&manager.id},
            doc! {"$set":{"platform_profile_id":&profile.id}},
        )
        .await
        .unwrap();
    state
        .db
        .collection::<PlatformBotConfig>(crate::models::platform_bot_config::COLLECTION_NAME)
        .insert_one(&profile)
        .await
        .unwrap();
    let before = server.received_requests().await.unwrap().len();
    let routed = service
        .webhook(
            &headers,
            &serde_json::to_vec(&command("/start", 100)).unwrap(),
        )
        .await
        .unwrap()
        .expect("existing platform route");
    assert_eq!(routed.0.id, manager.id);
    assert_eq!(server.received_requests().await.unwrap().len(), before);
    state.db.drop().await.unwrap();
}

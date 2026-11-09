use super::*;

async fn last_text(server: &MockServer) -> String {
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .rev()
        .find(|r| r.url.path().ends_with("/sendMessage"))
        .unwrap()
        .body_json::<Value>()
        .unwrap()["text"]
        .as_str()
        .unwrap()
        .to_owned()
}

#[tokio::test]
async fn telegram_new_fresh_first_start_preserves_context_on_default_and_scoped_managers() {
    for scoped in [false, true] {
        let (mut state, actor, server) = fixture().await;
        state.config.base_url = "https://api.nyxid.ai".into();
        state.config.domain_aliases.base_urls = vec!["https://api.nyxid.test".into()];
        state.config.frontend_url = "https://nyxid.ai".into();
        let (request, start, _) = hosted_request(&state, &actor, &server, false).await;
        let headers = headers(&state).await;
        if scoped {
            state
                .db
                .collection::<bson::Document>(CREDENTIALS)
                .update_one(
                    doc! {"provider":"telegram-new"},
                    doc! {"$set":{"provider":"telegram-new/100"}},
                )
                .await
                .unwrap();
        }
        let base = server.uri();
        let service = service(&state, &base).for_manager(100).await.unwrap();
        let addressed = start.replacen("/start", "/start@nYxSeTuPbOt", 1);
        webhook(&service, &headers, command(&addressed, 10)).await;
        let saved = service.get(&actor, &request.id).await.unwrap();
        assert_eq!(saved.status, Status::WaitingBot);
        assert_eq!(saved.telegram_user_id, Some(700));
        assert_eq!(saved.start_update_id, Some(10));
        assert!(saved.expires_at > Utc::now());
        assert!(service.setup_notice(&saved).await.unwrap().is_none());
        assert!(service.can_open_creation_form(&saved).await.unwrap());
        assert_eq!(server.received_requests().await.unwrap().len(), 2);
        webhook(&service, &headers, command("/start", 11)).await;
        assert_eq!(server.received_requests().await.unwrap().len(), 4);
        assert_eq!(
            service
                .get(&actor, &request.id)
                .await
                .unwrap()
                .start_update_id,
            Some(10)
        );
        state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn telegram_new_setup_terminal_reasons_match_web_and_include_recovery() {
    for (status, reason, text) in [
        ("expired", "expired", "setup has expired"),
        ("cancelled", "cancelled", "setup was cancelled"),
        ("connected", "completed", "setup is complete"),
        ("provisioning", "in_progress", "awaiting connection"),
        ("suspended", "suspended", "connection is suspended"),
    ] {
        let (state, actor, server) = fixture().await;
        let (request, start) = start_request(&state, &actor, &server).await;
        let base = server.uri();
        let service = service(&state, &base);
        let headers = headers(&state).await;
        webhook(&service, &headers, command(&start, 10)).await;
        state.db.collection::<bson::Document>(REQUESTS).update_one(doc! {"_id":&request.id},
            doc! {"$set":{"status":status,"active":false,"expires_at":bson::DateTime::from_chrono(Utc::now()-Duration::seconds(1))}}).await.unwrap();
        for command_text in [&start, "/start"] {
            webhook(&service, &headers, command(command_text, 11)).await;
            let body = last_text(&server).await;
            assert!(body.contains(text), "{status}: {body}");
            assert!(body.contains(
                "Check status: https://app.nyxid.test/channel-bots?connect=telegram-new&request_id="
            ));
            assert!(body.contains(
                "Start a new setup: https://app.nyxid.test/channel-bots?connect=telegram-new"
            ));
        }
        let saved = service.get(&actor, &request.id).await.unwrap();
        let response = crate::handlers::telegram_new::projected_request(&state, saved)
            .await
            .unwrap();
        assert_eq!(response.setup_notice.unwrap().reason, reason);
        assert!(!response.can_open_creation_form);
        state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn telegram_new_replaced_missing_and_mismatched_start_never_disclose_destination() {
    let (state, actor, server) = fixture().await;
    let (request, start) = start_request(&state, &actor, &server).await;
    let base = server.uri();
    let service = service(&state, &base);
    let headers = headers(&state).await;
    webhook(&service, &headers, command(&start, 10)).await;
    let new_link = service.launch(&actor, &request.id).await.unwrap();
    webhook(&service, &headers, command(&start, 11)).await;
    assert!(last_text(&server).await.contains("link was replaced"));
    let challenge = url::Url::parse(&new_link)
        .unwrap()
        .query_pairs()
        .find(|(k, _)| k == "start")
        .unwrap()
        .1
        .into_owned();
    let mut other = command(&format!("/start {challenge}"), 12);
    other["message"]["from"]["id"] = json!(701);
    other["message"]["chat"]["id"] = json!(701);
    webhook(&service, &headers, other).await;
    let text = last_text(&server).await;
    assert!(text.contains("different Telegram account"));
    for private in [&request.id, &actor, &request.label, &challenge] {
        assert!(!text.contains(private));
    }
    webhook(
        &service,
        &headers,
        command(&format!("/start {}", "a".repeat(48)), 13),
    )
    .await;
    assert!(last_text(&server).await.contains("could not be found"));
    webhook(&service, &headers, command("/start malformed", 14)).await;
    assert!(last_text(&server).await.contains("could not be found"));
    assert_eq!(
        service
            .get(&actor, &request.id)
            .await
            .unwrap()
            .telegram_user_id,
        Some(700)
    );
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn telegram_new_first_start_reports_hosted_link_lifecycle_without_creating() {
    for (change, reason) in [
        ("cancelled", "cancelled"),
        ("expired", "expired"),
        ("clock", "expired"),
        ("completed", "completed"),
        ("missing", "missing"),
        ("replaced", "superseded"),
    ] {
        let (state, actor, server) = fixture().await;
        let (request, start, watch) = hosted_request(&state, &actor, &server, false).await;
        let links = state
            .db
            .collection::<bson::Document>(crate::models::channel_connect_link::COLLECTION_NAME);
        if change == "missing" {
            links.delete_one(doc! {"_id":&request.id}).await.unwrap();
        } else if change == "clock" {
            links.update_one(doc! {"_id":&request.id}, doc! {"$set":{"expires_at": bson::DateTime::from_chrono(
                Utc::now() - Duration::seconds(crate::services::channel_connect_link_service::GRACE_SECONDS + 1))}}).await.unwrap();
        } else if change == "replaced" {
            state
                .db
                .collection::<bson::Document>(
                    crate::models::nyxbot_channel::WATCHES_COLLECTION_NAME,
                )
                .update_one(
                    doc! {"_id":watch},
                    doc! {"$set":{"connect_link_id":uuid::Uuid::new_v4().to_string()}},
                )
                .await
                .unwrap();
        } else {
            links
                .update_one(doc! {"_id":&request.id}, doc! {"$set":{"status":change}})
                .await
                .unwrap();
        }
        let base = server.uri();
        let service = service(&state, &base);
        webhook(&service, &headers(&state).await, command(&start, 10)).await;
        let saved = service.get(&actor, &request.id).await.unwrap();
        let response = crate::handlers::telegram_new::projected_request(&state, saved)
            .await
            .unwrap();
        let notice = response.setup_notice.unwrap();
        assert_eq!(notice.reason, reason, "{change}");
        assert!(last_text(&server).await.contains(notice.message));
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
async fn telegram_new_first_start_after_request_expiry_or_removal_has_recovery() {
    for removed in [false, true] {
        let (state, actor, server) = fixture().await;
        let (request, start) = start_request(&state, &actor, &server).await;
        let rows = state.db.collection::<bson::Document>(REQUESTS);
        if removed {
            rows.delete_one(doc! {"_id": &request.id}).await.unwrap();
        } else {
            rows.update_one(
                doc! {"_id": &request.id},
                doc! {"$set": {
                    "expires_at": bson::DateTime::from_chrono(Utc::now() - Duration::seconds(1))
                }},
            )
            .await
            .unwrap();
        }
        let base = server.uri();
        let service = service(&state, &base);
        webhook(&service, &headers(&state).await, command(&start, 10)).await;
        let text = last_text(&server).await;
        assert!(text.contains(if removed {
            "could not be found"
        } else {
            "setup has expired"
        }));
        assert!(text.contains(
            "Start a new setup: https://app.nyxid.test/channel-bots?connect=telegram-new"
        ));
        if !removed {
            let saved = service.get(&actor, &request.id).await.unwrap();
            assert_eq!(saved.status, Status::Expired);
            assert!(saved.telegram_user_id.is_none());
            assert_eq!(
                service.setup_notice(&saved).await.unwrap().unwrap().reason,
                "expired"
            );
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
async fn telegram_new_completed_bot_start_returns_saved_context_without_new_creation() {
    let (state, actor, server) = fixture().await;
    let (request, start, _) = hosted_request(&state, &actor, &server, false).await;
    let Some(CreationContext::Hosted {
        watch: Some(watch), ..
    }) = &request.creation_context
    else {
        panic!("Expected captured agent context");
    };
    let agent_url = format!(
        "Open the agent: https://app.nyxid.test/assistant?agent={}",
        watch.agent_id.as_deref().unwrap()
    );
    let base = server.uri();
    let service = service(&state, &base);
    let headers = headers(&state).await;
    webhook(&service, &headers, command(&start, 10)).await;
    webhook(
        &service,
        &headers,
        json!({"update_id":11,"managed_bot":{"user":{"id":700,"is_bot":false},"bot":bot()}}),
    )
    .await;
    let mut created = message(json!({"managed_bot_created":{"bot":bot()}}));
    created["update_id"] = json!(12);
    webhook(&service, &headers, created).await;
    provider_connection(&server, &request.id, 200).await;
    service.complete_pending_creations().await.unwrap();
    assert_eq!(
        service.get(&actor, &request.id).await.unwrap().status,
        Status::Connected
    );
    let before = server.received_requests().await.unwrap().len();
    for (i, text) in [&start, "/start", "/start"].iter().enumerate() {
        webhook(&service, &headers, command(text, 20 + i as i64)).await;
        let text = last_text(&server).await;
        assert!(text.contains("setup is complete"));
        assert!(text.contains(&agent_url));
    }
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), before + 3);
    assert!(
        requests[before..]
            .iter()
            .all(|r| r.body_json::<Value>().unwrap()["reply_markup"]["keyboard"].is_null())
    );
    assert_eq!(
        state
            .db
            .collection::<bson::Document>(BOTS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        bson::to_bson(
            &service
                .get(&actor, &request.id)
                .await
                .unwrap()
                .creation_context
        )
        .unwrap(),
        bson::to_bson(&request.creation_context).unwrap()
    );
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn telegram_new_forwarded_start_cannot_bind_or_expose_setup() {
    let (state, actor, server) = fixture().await;
    let (request, start) = start_request(&state, &actor, &server).await;
    let base = server.uri();
    let service = service(&state, &base);
    let mut forwarded = command(&start, 10);
    forwarded["message"]["forward_from"] = json!({"id":701});
    webhook(&service, &headers(&state).await, forwarded).await;
    assert!(server.received_requests().await.unwrap().is_empty());
    assert!(
        service
            .get(&actor, &request.id)
            .await
            .unwrap()
            .telegram_user_id
            .is_none()
    );
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn telegram_new_replaced_launch_history_is_bounded_and_never_authorizes_start() {
    let (state, actor, server) = fixture().await;
    let (request, oldest) = start_request(&state, &actor, &server).await;
    let base = server.uri();
    let service = service(&state, &base);
    let headers = headers(&state).await;
    let mut previous = oldest.clone();
    for _ in 0..6 {
        let launch = service.launch(&actor, &request.id).await.unwrap();
        webhook(&service, &headers, command(&previous, 10)).await;
        assert!(last_text(&server).await.contains("link was replaced"));
        assert!(!last_text(&server).await.contains(&request.id));
        let saved = service.get(&actor, &request.id).await.unwrap();
        assert!(saved.previous_challenge_hashes.len() <= 4);
        assert!(saved.telegram_user_id.is_none());
        previous = format!(
            "/start {}",
            url::Url::parse(&launch)
                .unwrap()
                .query_pairs()
                .find(|(k, _)| k == "start")
                .unwrap()
                .1
        );
    }
    webhook(&service, &headers, command(&oldest, 11)).await;
    assert!(last_text(&server).await.contains("could not be found"));
    webhook(&service, &headers, command(&previous, 12)).await;
    assert_eq!(
        service.get(&actor, &request.id).await.unwrap().status,
        Status::WaitingBot
    );
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn telegram_new_first_start_reports_manager_change_and_revoked_access_without_private_links()
{
    for change in ["manager", "owner"] {
        let (state, actor, server) = fixture().await;
        let (request, start) = start_request(&state, &actor, &server).await;
        let base = server.uri();
        let service = service(&state, &base);
        let headers = headers(&state).await;
        if change == "manager" {
            state
                .db
                .collection::<bson::Document>(CREDENTIALS)
                .update_one(
                    doc! {"provider":"telegram-new"},
                    doc! {"$set":{"fields.observation_id":"new-observation"}},
                )
                .await
                .unwrap();
        } else {
            state
                .db
                .collection::<bson::Document>(crate::models::user::COLLECTION_NAME)
                .update_one(doc! {"_id":&actor}, doc! {"$set":{"is_active":false}})
                .await
                .unwrap();
        }
        webhook(&service, &headers, command(&start, 10)).await;
        let text = last_text(&server).await;
        if change == "manager" {
            assert!(text.contains("setup manager changed"));
            let response = crate::handlers::telegram_new::projected_request(
                &state,
                service.get(&actor, &request.id).await.unwrap(),
            )
            .await
            .unwrap();
            assert_eq!(response.setup_notice.unwrap().reason, "manager_changed");
        } else {
            assert!(text.contains("Access to this setup has changed"));
            assert!(!text.contains(&request.id));
            assert!(!text.contains(&actor));
            assert!(!text.contains("Open the agent:"));
            assert!(service.get(&actor, &request.id).await.is_err());
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

//! Regression coverage for #2283: rollout availability never revokes a binding.
use super::*;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

async fn preserved(state: &AppState, row: &NyxbotChannel) {
    let current = load_channel(state, OWNER, &row.id).await.unwrap();
    assert_eq!(current.status, "active");
    assert_eq!(current.last_error, None);
    assert_eq!(current.route_id, row.route_id);
    assert_eq!(current.route_api_key_id, row.route_api_key_id);
    for (collection, id) in [
        ("api_keys", row.route_api_key_id.as_str()),
        ("channel_conversations", row.route_id.as_deref().unwrap()),
    ] {
        assert!(
            state
                .db
                .collection::<bson::Document>(collection)
                .find_one(doc! {"_id":id,"is_active":true})
                .await
                .unwrap()
                .is_some()
        );
    }
    assert_eq!(
        state
            .db
            .collection::<bson::Document>(crate::models::audit_log::COLLECTION_NAME)
            .count_documents(doc! {"event_type":"nyxbot_channel_org_access_lost"})
            .await
            .unwrap(),
        0
    );
}

async fn pause(state: &AppState, flag_off: bool) {
    if flag_off {
        flags::set_platform_override(
            &state.db,
            org::FLAG,
            &flags::FlagTarget::Global,
            false,
            OWNER,
        )
        .await
        .unwrap();
    } else {
        crate::services::security_readiness_service::stop_test_replica(&state.db).await;
        state.db.collection::<bson::Document>(crate::models::security_replica::COLLECTION_NAME)
            .update_many(doc! {}, doc! {"$set":{"observed_at":bson::DateTime::from_chrono(Utc::now()-ChronoDuration::seconds(60))}})
            .await.unwrap();
    }
}

async fn resume(state: &AppState) {
    crate::services::security_readiness_service::ready_test_replica(&state.db).await;
    flags::set_platform_override(
        &state.db,
        org::FLAG,
        &flags::FlagTarget::Global,
        true,
        OWNER,
    )
    .await
    .unwrap();
}

async fn verify(state: &AppState, row: &NyxbotChannel) {
    let challenge = org::links::issue(&state.db, OWNER, row).await.unwrap();
    assert!(
        org::links::receive(
            &state.db,
            row,
            "7",
            challenge["code"].as_str().unwrap(),
            None
        )
        .await
        .unwrap()
    );
    org::links::confirm(&state.db, OWNER, row, challenge["id"].as_str().unwrap())
        .await
        .unwrap();
}

#[tokio::test]
async fn org_access_2283_readiness_and_flag_suspend_provider_reply_status_and_late_delivery() {
    for flag_off in [false, true] {
        let (state, calls, server) = setup("org_access_2283_provider").await;
        let _server = tokio_util::task::AbortOnDropHandle::new(server);
        let (row, key, agent, _) = org_channel(&state, false).await;
        verify(&state, &row).await;
        assert_eq!(
            put_binding(
                State(state.clone()),
                Path("bnd_turns".into()),
                bearer(&key),
                Json(binding_body(&key, OWNER))
            )
            .await
            .status(),
            StatusCode::OK
        );
        ensure_org_partition(&state, &key).await;
        let answer = body_text(
            respond(
                &state,
                &key,
                &event("First question", "7", "first"),
                "first",
            )
            .await,
        )
        .await;
        assert!(answer.contains("Here is your answer"), "{answer}");
        let conversation = state.db.collection::<AssistantConversation>(CONVERSATIONS)
            .find_one(doc! {"user_id":OWNER,"agent_id":&agent.id,"reply_channel.org_execution":{"$type":"object"}})
            .await.unwrap().unwrap();
        settle(&state, &conversation.id).await;
        let mut pending = state
            .db
            .collection::<NyxbotEvent>(EVENTS)
            .find_one(doc! {"channel_id":&row.id,"event_id":"first"})
            .await
            .unwrap()
            .unwrap();
        // Simulate a detached stream awaiting a target, then verify that a
        // suspended delivery sends exactly once after readiness recovers.
        pending.delivery.as_mut().unwrap().state = "pending".into();
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
        Mock::given(method("POST"))
            .and(path("/events/cmaeg1.ev.sealed-reference/reply-target"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"reply_target_ref":"cmaeg1.rt.restored",
                "expires_at":(Utc::now()+ChronoDuration::hours(1)).timestamp()})),
            )
            .mount(&gateway)
            .await;
        Mock::given(method("POST"))
            .and(path("/reply-targets/cmaeg1.rt.restored/messages"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"delivery":{"outcome":"confirmed"}})),
            )
            .mount(&gateway)
            .await;
        state
            .db
            .collection::<NyxbotEvent>(EVENTS)
            .replace_one(doc! {"_id":&pending.id}, &pending)
            .await
            .unwrap();
        pause(&state, flag_off).await;
        assert_eq!(
            org_access(&state, &row).await.unwrap(),
            OrgAccess::Unavailable
        );
        let response = respond(
            &state,
            &key,
            &event("Try while paused", "7", "paused"),
            "paused",
        )
        .await;
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert!(body_text(response).await.contains("provider_unavailable"));
        assert_eq!(calls.lock().await.len(), 1);
        assert!(matches!(
            route_key_auth(&state, &row).await,
            Err(AppError::SecurityReadinessUnavailable)
        ));
        for token in [None, Some("unused-reply-token")] {
            assert!(matches!(
                direct_reply(&state, &row, "message", "reply", token).await,
                Err(AppError::SecurityReadinessUnavailable)
            ));
        }
        assert_eq!(
            transport::authority(&state, &row).await,
            Err("org_access_unavailable")
        );
        status::check_deliveries(&state).await.unwrap();
        let listing = channel_listing(&state, OWNER, false).await.unwrap();
        assert_eq!(listing["channel_agents"][0]["delivery_status"], "failing");
        assert_eq!(
            listing["channel_agents"][0]["delivery_error"],
            "org_access_unavailable"
        );
        assert!(
            listing["channel_agents"][0]["delivery_reason"]
                .as_str()
                .unwrap()
                .contains("temporarily unavailable")
        );
        assert_eq!(listing["channel_agents"][0]["status"], "active");
        assert!(matches!(
            late_delivery::eligible(
                &state,
                &pending,
                pending.delivery.as_ref().unwrap(),
                &conversation
            )
            .await,
            Err(AppError::SecurityReadinessUnavailable)
        ));
        late_delivery::process(&state, &pending.id).await;
        let deferred = state
            .db
            .collection::<NyxbotEvent>(EVENTS)
            .find_one(doc! {"_id":&pending.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(deferred.delivery.unwrap().state, "pending");
        assert!(gateway.received_requests().await.unwrap().is_empty());
        preserved(&state, &row).await;
        resume(&state).await;
        assert_eq!(org_access(&state, &row).await.unwrap(), OrgAccess::Holds);
        route_key_auth(&state, &row).await.unwrap();
        transport::authority(&state, &row).await.unwrap();
        assert!(
            late_delivery::eligible(
                &state,
                &pending,
                pending.delivery.as_ref().unwrap(),
                &conversation
            )
            .await
            .unwrap()
            .is_some()
        );
        late_delivery::process(&state, &pending.id).await;
        let delivered = state
            .db
            .collection::<NyxbotEvent>(EVENTS)
            .find_one(doc! {"_id":&pending.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(delivered.delivery.unwrap().state, "sent");
        assert_eq!(
            gateway
                .received_requests()
                .await
                .unwrap()
                .iter()
                .filter(|request| request.url.path().ends_with("/messages"))
                .count(),
            1
        );
        let listing = channel_listing(&state, OWNER, false).await.unwrap();
        assert_ne!(
            listing["channel_agents"][0]["delivery_error"],
            "org_access_unavailable"
        );
        let answer = body_text(
            respond(
                &state,
                &key,
                &event("Try while paused", "7", "paused"),
                "paused",
            )
            .await,
        )
        .await;
        assert!(answer.contains("Here is your answer"), "{answer}");
        assert_eq!(calls.lock().await.len(), 2);
        preserved(&state, &row).await;
        state.db.drop().await.unwrap();
    }
}

async fn relay(state: &AppState, row: &NyxbotChannel, id: &str) -> Response {
    let correlation = format!("relay-{id}");
    let body = serde_json::to_vec(
        &json!({"message_id":id,"correlation_id":correlation,"platform":"lark",
        "agent":{"api_key_id":row.route_api_key_id,"name":"route"},
        "conversation":{"id":row.route_id,"platform_id":"org-private","type":"private"},
        "sender":{"platform_id":"7"},"content":{"type":"text","text":"Relay member request"},
        "raw_platform_data":{"event":{"sender":{"sender_type":"user"}}}}),
    )
    .unwrap();
    let token = crate::crypto::jwt::generate_relay_callback_token(
        &state.jwt_keys,
        &state.config,
        &correlation,
        &row.route_api_key_id,
        id,
        "lark",
        &sha256_hex(&body),
    )
    .unwrap();
    let mut headers = HeaderMap::new();
    headers.insert("x-nyxid-callback-token", token.parse().unwrap());
    relay_callback(
        State(state.clone()),
        Path(row.id.clone()),
        headers,
        Bytes::from(body),
    )
    .await
}

#[tokio::test]
async fn org_access_2283_relay_defers_readiness_and_flag_loss_then_resumes() {
    for flag_off in [false, true] {
        let (state, calls, server) = setup("org_access_2283_relay").await;
        let _server = tokio_util::task::AbortOnDropHandle::new(server);
        let (mut row, _, _, _) = org_channel(&state, false).await;
        row.transport = "direct".into();
        row.platform = "lark".into();
        state
            .db
            .collection::<NyxbotChannel>(CHANNELS)
            .replace_one(doc! {"_id":&row.id}, &row)
            .await
            .unwrap();
        for (collection, id) in [
            ("channel_bots", &row.channel_bot_id),
            ("channel_conversations", row.route_id.as_ref().unwrap()),
        ] {
            state
                .db
                .collection::<bson::Document>(collection)
                .update_one(doc! {"_id":id}, doc! {"$set":{"platform":"lark"}})
                .await
                .unwrap();
        }
        verify(&state, &row).await;
        pause(&state, flag_off).await;
        assert_eq!(
            relay(&state, &row, "paused").await.status(),
            StatusCode::SERVICE_UNAVAILABLE
        );
        assert!(calls.lock().await.is_empty());
        assert_eq!(
            state
                .db
                .collection::<NyxbotEvent>(EVENTS)
                .count_documents(doc! {"channel_id":&row.id})
                .await
                .unwrap(),
            0
        );
        status::check_deliveries(&state).await.unwrap();
        preserved(&state, &row).await;
        resume(&state).await;
        let (_watch, done) = watch_relay_callback(&row.id, "paused");
        assert_eq!(
            relay(&state, &row, "paused").await.status(),
            StatusCode::ACCEPTED
        );
        tokio::time::timeout(Duration::from_secs(20), done)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(calls.lock().await.len(), 1);
        preserved(&state, &row).await;
        state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn org_access_2283_confirmed_losses_still_release_while_flag_is_off() {
    for loss in [
        "demoted",
        "removed",
        "agent_destroyed",
        "agent_moved",
        "bot_moved",
        "bot_inactive",
        "platform_route",
        "binding_version",
        "binding_org",
    ] {
        let (state, _, server) = setup("org_access_2283_revoked").await;
        let _server = tokio_util::task::AbortOnDropHandle::new(server);
        let (mut row, _, agent, org_id) = org_channel(&state, false).await;
        // Use relay cleanup; no test should need a real gateway.
        row.transport = "direct".into();
        state
            .db
            .collection::<NyxbotChannel>(CHANNELS)
            .replace_one(doc! {"_id":&row.id}, &row)
            .await
            .unwrap();
        let (collection, filter, fields) = match loss {
            "demoted" => (
                "org_memberships",
                doc! {"org_user_id":&org_id,"member_user_id":OWNER},
                doc! {"role":"member"},
            ),
            "removed" => (
                "org_memberships",
                doc! {"org_user_id":&org_id,"member_user_id":OWNER},
                doc! {"revoked_at":bson::DateTime::now()},
            ),
            "agent_destroyed" => (
                "assistant_agents",
                doc! {"_id":&agent.id},
                doc! {"destroyed_at":bson::DateTime::now()},
            ),
            "agent_moved" => (
                "assistant_agents",
                doc! {"_id":&agent.id},
                doc! {"user_id":OWNER},
            ),
            "bot_moved" => (
                "channel_bots",
                doc! {"_id":&row.channel_bot_id},
                doc! {"user_id":OWNER},
            ),
            "bot_inactive" => (
                "channel_bots",
                doc! {"_id":&row.channel_bot_id},
                doc! {"is_active":false},
            ),
            "platform_route" => (
                "channel_bots",
                doc! {"_id":&row.channel_bot_id},
                doc! {"platform_entry_pending":true},
            ),
            "binding_version" => {
                row.org_specialist.as_mut().unwrap().version = 2;
                (
                    CHANNELS,
                    doc! {"_id":&row.id},
                    doc! {"org_specialist.version":2},
                )
            }
            "binding_org" => {
                row.org_specialist.as_mut().unwrap().org_id = OWNER.into();
                (
                    CHANNELS,
                    doc! {"_id":&row.id},
                    doc! {"org_specialist.org_id":OWNER},
                )
            }
            _ => unreachable!(),
        };
        state
            .db
            .collection::<bson::Document>(collection)
            .update_one(filter, doc! {"$set":fields})
            .await
            .unwrap();
        pause(&state, true).await;
        assert_eq!(
            org_access(&state, &row).await.unwrap(),
            OrgAccess::Lost,
            "{loss}"
        );
        status::check_deliveries(&state).await.unwrap();
        let retired = load_channel(&state, OWNER, &row.id).await.unwrap();
        assert_eq!(retired.status, "failed", "{loss}");
        assert_eq!(retired.last_error.as_deref(), Some("org_access_lost"));
        for (collection, id) in [
            ("api_keys", &row.route_api_key_id),
            ("channel_conversations", row.route_id.as_ref().unwrap()),
        ] {
            assert!(
                state
                    .db
                    .collection::<bson::Document>(collection)
                    .find_one(doc! {"_id":id,"is_active":true})
                    .await
                    .unwrap()
                    .is_none(),
                "{loss}: {collection}"
            );
        }
        state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn org_access_2283_database_error_never_releases_a_binding() {
    let (state, _, server) = setup("org_access_2283_db_error").await;
    let _server = tokio_util::task::AbortOnDropHandle::new(server);
    let (row, _, _, _) = org_channel(&state, false).await;
    let mut unavailable = state.clone();
    unavailable.db = mongodb::Client::with_uri_str(
        "mongodb://127.0.0.1:1/?serverSelectionTimeoutMS=25&connectTimeoutMS=25",
    )
    .await
    .unwrap()
    .database("unavailable");
    assert!(org_access(&unavailable, &row).await.is_err());
    assert!(route_key_auth(&unavailable, &row).await.is_err());
    assert_eq!(
        transport::authority(&unavailable, &row).await,
        Err("storage_unavailable")
    );
    assert!(status::check_deliveries(&unavailable).await.is_err());
    preserved(&state, &row).await;
    assert_eq!(org_access(&state, &row).await.unwrap(), OrgAccess::Holds);
    state.db.drop().await.unwrap();
}

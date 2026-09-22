use super::*;
use crate::services::channel_connection_webhook_service as webhooks;

pub(crate) async fn credentials(state: &AppState, adapter: &XAdapter, owner: &str) {
    platform_credential_service::update(
        &state.db,
        &state.encryption_keys,
        &adapter.platform_credentials().unwrap(),
        owner,
        &[
            (
                "app_bearer_token".into(),
                Some(zeroize::Zeroizing::new("app-token".into())),
            ),
            (
                "consumer_secret".into(),
                Some(zeroize::Zeroizing::new("api-secret".into())),
            ),
        ]
        .into(),
        false,
    )
    .await
    .unwrap();
}

pub(super) async fn provider_setup(server: &MockServer, bot: &ChannelBot) {
    Mock::given(method("GET"))
        .and(path("/2/webhooks"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"data": [{"id": "100", "valid": true,
            "url": "https://nyx.example/api/v1/webhooks/channel/x/platform"}]})),
        )
        .mount(server)
        .await;
    Mock::given(method("GET")).and(path("/2/activity/subscriptions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": [{"subscription_id": "200", "event_type": "dm.received",
            "filter": {"user_id": "10"}, "webhook_id": "100", "tag": format!("nyxid:{}", bot.id)}]}))).mount(server).await;
}

#[tokio::test]
async fn x_webhook_onboarding_avoids_polling_and_failed_setup_can_initialize_fallback() {
    for setup_ok in [true, false] {
        let (mut state, adapter, server, owner, connection) = fixture().await;
        state.config.base_url = "https://nyx.example".into();
        credentials(&state, &adapter, &owner).await;
        identity(&server, "10").await;
        Mock::given(method("GET")).and(path("/2/webhooks"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": [{
                "id": "100", "valid": true, "url": "https://nyx.example/api/v1/webhooks/channel/x/platform"
            }]}))).mount(&server).await;
        Mock::given(method("GET"))
            .and(path("/2/activity/subscriptions"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": []})))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/2/activity/subscriptions"))
            .respond_with(if setup_ok {
                ResponseTemplate::new(200)
                    .set_body_json(json!({"data": {"subscription": {"subscription_id": "200"}}}))
            } else {
                ResponseTemplate::new(403)
                    .set_body_json(json!({"detail": "private upstream error"}))
            })
            .expect(1)
            .mount(&server)
            .await;
        let bot = create(&state, &adapter, &owner, &connection).await.unwrap();
        assert_eq!(bot.status, "active");
        assert_eq!(bot.webhook_registered, setup_ok);
        assert!(bot.poll_cursor.is_none());
        assert!(bot.last_polled_at.is_none());
        assert!(
            !server
                .received_requests()
                .await
                .unwrap()
                .iter()
                .any(|r| r.url.path() == "/2/dm_events")
        );
        if setup_ok {
            assert!(bot.error.is_none());
        } else {
            assert!(bot.error.as_deref().unwrap().contains("Verify"));
            Mock::given(method("GET"))
                .and(path("/2/dm_events"))
                .respond_with(
                    ResponseTemplate::new(200).set_body_json(json!({"data": [{"id": "300"}]})),
                )
                .expect(1)
                .mount(&server)
                .await;
            channel_poll_service::poll_bot(&state, &adapter, &bot.id, 60)
                .await
                .unwrap();
            let current = channel_bot_service::get_bot(&state.db, &bot.id)
                .await
                .unwrap();
            assert_eq!(current.poll_cursor.as_deref(), Some("300"));
            assert_eq!(current.status, "active");
            assert_eq!(
                state
                    .db
                    .collection::<bson::Document>(crate::models::channel_message::COLLECTION_NAME)
                    .count_documents(doc! {})
                    .await
                    .unwrap(),
                0
            );
        }
    }
}

#[tokio::test]
async fn x_webhook_credentials_are_separate_from_shared_oauth_and_clear_independently() {
    let (state, adapter, _, owner, _) = fixture().await;
    credentials(&state, &adapter, &owner).await;
    let descriptor = adapter.platform_credentials().unwrap();
    let row = state
        .db
        .collection::<crate::models::platform_credential::PlatformCredential>(
            crate::models::platform_credential::COLLECTION_NAME,
        )
        .find_one(doc! {"provider": "x"})
        .await
        .unwrap()
        .unwrap();
    assert!(!row.secrets.contains_key("client_secret"));
    assert_ne!(row.secrets["consumer_secret"].bytes, b"api-secret");
    let loaded =
        platform_credential_service::load_decrypted(&state.db, &state.encryption_keys, &descriptor)
            .await
            .unwrap();
    assert_eq!(loaded.get("client_secret"), Some("platform-secret"));
    assert_eq!(loaded.get("consumer_secret"), Some("api-secret"));
    assert!(adapter.connection_webhook_configured(&loaded));
    platform_credential_service::update(
        &state.db,
        &state.encryption_keys,
        &descriptor,
        &owner,
        &[("consumer_secret".into(), None)].into(),
        false,
    )
    .await
    .unwrap();
    let loaded =
        platform_credential_service::load_decrypted(&state.db, &state.encryption_keys, &descriptor)
            .await
            .unwrap();
    assert_eq!(loaded.get("client_secret"), Some("platform-secret"));
    assert!(!adapter.connection_webhook_configured(&loaded));
}

#[tokio::test]
async fn x_webhook_activation_stops_polling_and_deletion_removes_only_its_subscription() {
    let (state, adapter, server, owner, connection) = fixture().await;
    let bot = insert_bot(&state, &owner, &connection).await;
    credentials(&state, &adapter, &owner).await;
    provider_setup(&server, &bot).await;
    assert!(
        webhooks::configure(
            &state.db,
            &state.billing,
            &state.encryption_keys,
            &state.http_client,
            &adapter,
            &bot,
            "https://nyx.example"
        )
        .await
        .unwrap()
    );
    let current = channel_bot_service::get_bot(&state.db, &bot.id)
        .await
        .unwrap();
    assert!(current.webhook_registered);
    assert_eq!(current.poll_cursor, bot.poll_cursor);
    Mock::given(path("/2/dm_events"))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&server)
        .await;
    channel_poll_service::poll_bot(&state, &adapter, &bot.id, 60)
        .await
        .unwrap();
    Mock::given(method("DELETE"))
        .and(path("/2/activity/subscriptions/200"))
        .and(header("authorization", "Bearer app-token"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    let cleanup = channel_bot_service::delete_bot(
        &state.db,
        &state.config,
        &state.http_client,
        &state.encryption_keys,
        &adapter,
        &bot.id,
        &owner,
    )
    .await
    .unwrap();
    assert_eq!(cleanup, Some("removed"));
    assert!(
        state
            .db
            .collection::<UserApiKey>(KEYS)
            .find_one(doc! {"_id": &connection})
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        !channel_bot_service::get_bot(&state.db, &bot.id)
            .await
            .unwrap()
            .is_active
    );
}

#[tokio::test]
async fn x_deleted_channel_retains_subscription_cleanup_until_retry_succeeds() {
    let (state, adapter, server, owner, connection) = fixture().await;
    let bot = insert_bot(&state, &owner, &connection).await;
    credentials(&state, &adapter, &owner).await;
    provider_setup(&server, &bot).await;
    let attempts = std::sync::atomic::AtomicUsize::new(0);
    Mock::given(method("DELETE"))
        .and(path("/2/activity/subscriptions/200"))
        .respond_with(move |_: &wiremock::Request| {
            ResponseTemplate::new(
                if attempts.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
                    503
                } else {
                    204
                },
            )
        })
        .expect(2)
        .mount(&server)
        .await;
    assert_eq!(
        channel_bot_service::delete_bot(
            &state.db,
            &state.config,
            &state.http_client,
            &state.encryption_keys,
            &adapter,
            &bot.id,
            &owner,
        )
        .await
        .unwrap(),
        Some("failed")
    );
    let deleted = channel_bot_service::get_bot(&state.db, &bot.id)
        .await
        .unwrap();
    assert!(!deleted.is_active);
    assert_eq!(deleted.status, "inactive");
    assert!(deleted.webhook_registered);
    webhooks::remove_stopped(
        &state.db,
        &state.encryption_keys,
        &state.http_client,
        &adapter,
        &deleted,
    )
    .await
    .unwrap();
    let cleaned = channel_bot_service::get_bot(&state.db, &bot.id)
        .await
        .unwrap();
    assert!(!cleaned.is_active);
    assert!(!cleaned.webhook_registered);
    webhooks::remove_stopped(
        &state.db,
        &state.encryption_keys,
        &state.http_client,
        &adapter,
        &deleted,
    )
    .await
    .unwrap();
    server.verify().await;
}

#[tokio::test]
async fn x_webhook_verify_preserves_subscription_when_channel_is_edited_concurrently() {
    let (state, _, _, owner, connection) = fixture().await;
    let mut bot = insert_bot(&state, &owner, &connection).await;
    bot.webhook_registered = true;
    state
        .db
        .collection::<ChannelBot>(BOTS)
        .update_one(
            doc! {"_id": &bot.id},
            doc! {"$set": {"webhook_registered": true}},
        )
        .await
        .unwrap();
    let db = state.db.clone();
    let bot_id = bot.id.clone();
    let edited_at = bson::DateTime::from_chrono(bot.updated_at + chrono::Duration::seconds(1));
    let webhook = axum::routing::get(move || {
        let db = db.clone();
        let bot_id = bot_id.clone();
        async move {
            db.collection::<ChannelBot>(BOTS)
                .update_one(
                    doc! {"_id": bot_id},
                    doc! {"$set": {"label": "Edited during Verify", "updated_at": edited_at}},
                )
                .await
                .unwrap();
            Json(json!({"data": [{"id": "100", "valid": true,
                "url": "https://nyx.example/api/v1/webhooks/channel/x/platform"}]}))
        }
    });
    let tag = format!("nyxid:{}", bot.id);
    let subscriptions = axum::routing::get(move || {
        let tag = tag.clone();
        async move {
            Json(
                json!({"data": [{"subscription_id": "200", "event_type": "dm.received",
            "filter": {"user_id": "10"}, "webhook_id": "100", "tag": tag}]}),
            )
        }
    });
    let deletes = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counter = deletes.clone();
    let remove = axum::routing::delete(move || {
        counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        async { axum::http::StatusCode::NO_CONTENT }
    });
    let router = axum::Router::new()
        .route("/2/webhooks", webhook)
        .route("/2/activity/subscriptions", subscriptions)
        .route("/2/activity/subscriptions/200", remove);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let adapter = XAdapter {
        api_base: Some(format!("http://{}", listener.local_addr().unwrap())),
    };
    let task = tokio::spawn(async move { axum::serve(listener, router).await.unwrap() });
    credentials(&state, &adapter, &owner).await;
    assert!(matches!(
        webhooks::configure(
            &state.db,
            &state.billing,
            &state.encryption_keys,
            &state.http_client,
            &adapter,
            &bot,
            "https://nyx.example"
        )
        .await,
        Err(AppError::Conflict(_))
    ));
    let current = channel_bot_service::get_bot(&state.db, &bot.id)
        .await
        .unwrap();
    assert!(current.webhook_registered);
    assert_eq!(current.label, "Edited during Verify");
    assert_eq!(deletes.load(std::sync::atomic::Ordering::SeqCst), 0);
    task.abort();
}

#[tokio::test]
async fn x_webhook_setup_failures_preserve_polling_and_revoked_connections_cannot_subscribe() {
    let (state, adapter, server, owner, connection) = fixture().await;
    let bot = insert_bot(&state, &owner, &connection).await;
    credentials(&state, &adapter, &owner).await;
    Mock::given(path("/2/webhooks"))
        .respond_with(
            ResponseTemplate::new(403).set_body_json(json!({"detail": "PRIVATE UPSTREAM TEXT"})),
        )
        .mount(&server)
        .await;
    let result = webhooks::configure(
        &state.db,
        &state.billing,
        &state.encryption_keys,
        &state.http_client,
        &adapter,
        &bot,
        "https://nyx.example",
    )
    .await;
    assert!(!format!("{}", result.unwrap_err()).contains("PRIVATE UPSTREAM TEXT"));
    let current = channel_bot_service::get_bot(&state.db, &bot.id)
        .await
        .unwrap();
    assert!(!current.webhook_registered);
    assert_eq!(current.status, "active");
    server.reset().await;
    state
        .db
        .collection::<UserApiKey>(KEYS)
        .update_one(
            doc! {"_id": &connection},
            doc! {"$set": {"status": "revoked"}},
        )
        .await
        .unwrap();
    assert!(
        webhooks::configure(
            &state.db,
            &state.billing,
            &state.encryption_keys,
            &state.http_client,
            &adapter,
            &bot,
            "https://nyx.example"
        )
        .await
        .is_err()
    );
    assert!(server.received_requests().await.unwrap().is_empty());
    assert_eq!(
        channel_bot_service::get_bot(&state.db, &bot.id)
            .await
            .unwrap()
            .status,
        "failed"
    );
}

#[tokio::test]
async fn x_crc_endpoint_returns_json_using_live_platform_secret() {
    use axum::http::{Request, StatusCode};
    use base64::{Engine, engine::general_purpose::STANDARD};
    use hmac::{Hmac, Mac};
    use sha2::Sha256;
    use tower::ServiceExt;
    let (state, adapter, _, owner, _) = fixture().await;
    credentials(&state, &adapter, &owner).await;
    let (_, router) = crate::routes::build_router_with_state(state.clone());
    let response = router
        .with_state(state)
        .oneshot(
            Request::builder()
                .uri("/api/v1/webhooks/channel/x/platform?crc_token=challenge")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["content-type"], "application/json");
    let body = axum::body::to_bytes(response.into_body(), 1024)
        .await
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let mut mac = Hmac::<Sha256>::new_from_slice(b"api-secret").unwrap();
    mac.update(b"challenge");
    assert_eq!(
        body["response_token"],
        format!("sha256={}", STANDARD.encode(mac.finalize().into_bytes()))
    );
}

#[tokio::test]
async fn x_event_settings_require_consent_and_recover_partial_public_setup() {
    use crate::models::channel_bot::XChannelEvent::{Dm, Mentions, Replies};
    use crate::services::channel_bot_service::{SecretPatch, UpdateBotParams};
    let (state, adapter, server, owner, connection) = fixture().await;
    let bot = insert_bot(&state, &owner, &connection).await;
    credentials(&state, &adapter, &owner).await;
    let update = |events| {
        channel_bot_service::update_bot(
            &state.db,
            &state.encryption_keys,
            &state.http_client,
            &adapter,
            &bot.id,
            &owner,
            UpdateBotParams {
                x_events: Some(events),
                bot_token: None,
                label: None,
                verification_token: None,
                encrypt_key: SecretPatch::Unchanged,
                app_id: None,
                app_secret: None,
            },
        )
    };
    assert!(update(&[Dm, Mentions, Replies]).await.is_err());
    assert!(
        channel_bot_service::get_bot(&state.db, &bot.id)
            .await
            .unwrap()
            .x_events
            .is_none()
    );
    for events in [&[][..], &[Dm, Dm][..]] {
        assert!(matches!(
            update(events).await,
            Err(AppError::ValidationError(_))
        ));
    }
    state.db.collection::<bson::Document>(KEYS).update_one(doc! {"_id": &connection},
        doc! {"$set": {"token_scopes": super::super::channel_adapters::x::PUBLIC_SCOPES.join(" ")}}).await.unwrap();
    let current = update(&[Dm, Mentions, Replies]).await.unwrap();
    assert_eq!(current.x_events, Some(vec![Dm, Mentions, Replies]));
    provider_setup(&server, &bot).await;
    let fail = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
    let fail_response = fail.clone();
    Mock::given(method("POST"))
        .and(path("/2/activity/subscriptions"))
        .respond_with(move |request: &wiremock::Request| {
            let body: serde_json::Value = request.body_json().unwrap();
            if body["event_type"] == "post.reply.create"
                && fail_response.load(std::sync::atomic::Ordering::SeqCst)
            {
                ResponseTemplate::new(403)
            } else {
                ResponseTemplate::new(200)
                    .set_body_json(json!({"data": {"subscription": {"subscription_id": "300"}}}))
            }
        })
        .mount(&server)
        .await;
    assert!(
        webhooks::configure(
            &state.db,
            &state.billing,
            &state.encryption_keys,
            &state.http_client,
            &adapter,
            &current,
            "https://nyx.example"
        )
        .await
        .is_err()
    );
    let failed = channel_bot_service::get_bot(&state.db, &bot.id)
        .await
        .unwrap();
    assert_eq!(failed.status, "failed");
    assert!(
        failed.webhook_registered,
        "partial subscriptions remain eligible for cleanup"
    );
    Mock::given(path("/2/dm_events"))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&server)
        .await;
    channel_poll_service::poll_bot(&state, &adapter, &bot.id, 60)
        .await
        .unwrap();
    fail.store(false, std::sync::atomic::Ordering::SeqCst);
    assert!(
        webhooks::configure(
            &state.db,
            &state.billing,
            &state.encryption_keys,
            &state.http_client,
            &adapter,
            &failed,
            "https://nyx.example"
        )
        .await
        .unwrap()
    );
    let recovered = channel_bot_service::get_bot(&state.db, &bot.id)
        .await
        .unwrap();
    assert_eq!(recovered.status, "active");
    assert!(recovered.error.is_none());
    assert_eq!(recovered.x_events, Some(vec![Dm, Mentions, Replies]));
}

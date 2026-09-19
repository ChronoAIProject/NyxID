use super::*;
use crate::errors::AppResult;
use crate::services::channel_adapters::telegram::TelegramAdapter;
use crate::services::channel_adapters::telegram_new::credential_descriptor;
use crate::services::channel_bot_service as bots;
use crate::services::channel_platform::{PlatformAdapter, RegistrationValues};

const UPDATES: [&str; 5] = [
    "message",
    "edited_message",
    "channel_post",
    "callback_query",
    "managed_bot",
];

async fn pending_channel(
    state: &crate::AppState,
    actor: &str,
    server: &MockServer,
) -> bots::CreateBotResult {
    bot_identity_api(server, MANAGER).await;
    bots::create_bot(
        &state.db,
        &state.config,
        &state.encryption_keys,
        &state.http_client,
        &TelegramAdapter::media_test_adapter(&server.uri()),
        actor,
        "Manager channel",
        &RegistrationValues([("bot_token", MANAGER)].into()),
    )
    .await
    .unwrap()
}

async fn bot_identity_api(server: &MockServer, token: &str) {
    Mock::given(method("GET"))
        .and(path(format!("/bot{token}/getMe")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ok": true, "result": {"id": 100, "username": "NyxSetupBot", "is_bot": true}
        })))
        .mount(server)
        .await;
}

async fn verify_channel(
    state: &crate::AppState,
    server: &MockServer,
    channel: &ChannelBot,
) -> AppResult<axum::Json<crate::handlers::channel_bots::VerifyBotResponse>> {
    let base = server.uri();
    crate::handlers::channel_bots::verify_bot_with_adapter(
        state,
        channel.clone(),
        &TelegramAdapter::media_test_adapter(&base),
        &TelegramApi {
            http: &state.http_client,
            base_url: &base,
        },
        None,
    )
    .await
}

async fn check_channel(
    state: &crate::AppState,
    server: &MockServer,
    channel: &ChannelBot,
    token: &str,
) -> AppResult<()> {
    let base = server.uri();
    bots::register_webhook_with_telegram_api(
        &state.db,
        &TelegramApi {
            http: &state.http_client,
            base_url: &base,
        },
        &TelegramAdapter::media_test_adapter(&base),
        &channel.id,
        token,
        &bots::webhook_url(&state.config.base_url, channel),
        "",
    )
    .await
}

async fn webhook_info(server: &MockServer, token: &str, info: Value) {
    Mock::given(method("POST"))
        .and(path(format!("/bot{token}/getWebhookInfo")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "ok": true, "result": info,
        })))
        .mount(server)
        .await;
}

async fn assert_only_telegram_reads(server: &MockServer, webhooks: usize, identities: usize) {
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), webhooks + identities);
    assert_eq!(
        requests
            .iter()
            .filter(|request| request.url.path().ends_with("/getWebhookInfo"))
            .count(),
        webhooks
    );
    for request in requests {
        if request.url.path().ends_with("/getWebhookInfo") {
            assert_eq!(request.body_json::<Value>().unwrap(), json!({}));
        } else {
            assert!(request.url.path().ends_with("/getMe"));
        }
    }
}

async fn assert_invalid_webhook(state: &crate::AppState, server: &MockServer, info: Value) {
    let channel = state
        .db
        .collection::<ChannelBot>(BOTS)
        .find_one(doc! {"credential_source": "telegram_manager"})
        .await
        .unwrap()
        .unwrap();
    let before = credentials::load(&state.db, &credential_descriptor())
        .await
        .unwrap()
        .unwrap();
    for registered in [false, true] {
        state.db.collection::<ChannelBot>(BOTS).update_one(
            doc! {"_id": &channel.id},
            doc! {"$set": {"status": if registered {"active"} else {"pending_webhook"}, "webhook_registered": registered}},
        ).await.unwrap();
        server.reset().await;
        webhook_info(server, MANAGER, info.clone()).await;
        if registered {
            bot_identity_api(server, MANAGER).await;
            assert!(verify_channel(state, server, &channel).await.is_err());
        } else {
            assert!(
                check_channel(state, server, &channel, MANAGER)
                    .await
                    .is_err()
            );
        }
        let saved = bots::get_bot(&state.db, &channel.id).await.unwrap();
        assert_eq!(saved.status, "failed");
        assert!(!saved.webhook_registered);
        assert!(
            saved
                .error
                .as_deref()
                .unwrap()
                .contains("Platform Credentials")
        );
        assert_only_telegram_reads(server, 1, usize::from(registered)).await;
        let after = credentials::load(&state.db, &credential_descriptor())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            bson::to_document(&before).unwrap(),
            bson::to_document(&after).unwrap()
        );
    }
}

#[tokio::test]
async fn telegram_manager_channel_rejects_removed_and_repointed_webhooks() {
    let (state, actor, server) = fixture().await;
    pending_channel(&state, &actor, &server).await;
    for url in [
        "",
        "https://another.example/secret-hook",
        "https://api.nyxid.test/api/v1/webhooks/channel/telegram/per-bot",
    ] {
        assert_invalid_webhook(
            &state,
            &server,
            json!({"url": url, "allowed_updates": UPDATES}),
        )
        .await;
    }
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn telegram_manager_channel_registration_and_verify_require_all_five_updates() {
    let (state, actor, server) = fixture().await;
    let channel = pending_channel(&state, &actor, &server).await.bot;
    let callback = bots::webhook_url(&state.config.base_url, &channel);
    for missing in UPDATES {
        let updates: Vec<_> = UPDATES
            .into_iter()
            .filter(|kind| *kind != missing)
            .collect();
        assert_invalid_webhook(
            &state,
            &server,
            json!({"url": callback, "allowed_updates": updates}),
        )
        .await;
    }
    for updates in [Value::Null, json!([]), json!("message")] {
        assert_invalid_webhook(
            &state,
            &server,
            json!({"url": callback, "allowed_updates": updates}),
        )
        .await;
    }
    assert_invalid_webhook(&state, &server, json!({"url": callback})).await;
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn telegram_manager_channel_checks_complete_webhook_without_mutation_or_secret_exposure() {
    let (state, actor, server) = fixture().await;
    let created = pending_channel(&state, &actor, &server).await;
    let channel = created.bot.clone();
    assert!(created.webhook_secret.is_empty());
    assert!(channel.bot_token_encrypted.is_empty());
    assert!(channel.webhook_secret_hash.is_empty());
    let secret_before = headers(&state).await;
    let callback = bots::webhook_url(&state.config.base_url, &channel);
    let response = crate::handlers::channel_bots::CreateChannelBotResponse::from_bot(
        created.bot,
        TelegramAdapter::default().registration(),
        callback.clone(),
        created.webhook_secret,
    )
    .unwrap();
    let response = serde_json::to_value(response).unwrap();
    assert_eq!(response["credential_source"], "telegram_manager");
    assert_eq!(response["webhook_url"], callback);
    assert!(response.get("webhook_secret").is_none());
    assert_eq!(response["setup_instructions"], json!([]));
    server.reset().await;
    webhook_info(&server, MANAGER, json!({"url": callback, "allowed_updates": ["managed_bot", "callback_query", "channel_post", "edited_message", "message", "poll"]})).await;
    for verify in [false, true] {
        if verify {
            bot_identity_api(&server, MANAGER).await;
            let result = verify_channel(&state, &server, &channel).await.unwrap().0;
            assert_eq!(result.status, "active");
            assert!(result.webhook_registered);
        } else {
            check_channel(&state, &server, &channel, MANAGER)
                .await
                .unwrap();
        }
        let saved = bots::get_bot(&state.db, &channel.id).await.unwrap();
        assert_eq!(saved.status, "active");
        assert!(saved.webhook_registered);
        assert!(saved.error.is_none());
    }
    assert_eq!(headers(&state).await, secret_before);
    assert_only_telegram_reads(&server, 2, 1).await;
    server.reset().await;
    bots::delete_bot(
        &state.db,
        &state.config,
        &state.http_client,
        &state.encryption_keys,
        &TelegramAdapter::media_test_adapter(&server.uri()),
        &channel.id,
        &actor,
    )
    .await
    .unwrap();
    assert!(server.received_requests().await.unwrap().is_empty());
    assert_eq!(headers(&state).await, secret_before);
    state.db.drop().await.unwrap();
}

async fn manager_setup_api(server: &MockServer, token: &str, callback: &str, updates: Vec<&str>) {
    for (endpoint, result) in [
        (
            "getMe",
            json!({"id": 100, "username": "NyxSetupBot", "is_bot": true, "can_manage_bots": true}),
        ),
        ("setWebhook", json!(true)),
    ] {
        Mock::given(method("POST"))
            .and(path(format!("/bot{token}/{endpoint}")))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"ok": true, "result": result})),
            )
            .mount(server)
            .await;
    }
    webhook_info(
        server,
        token,
        json!({"url": callback, "allowed_updates": updates}),
    )
    .await;
}

#[tokio::test]
async fn telegram_manager_save_requires_all_five_confirmed_subscriptions() {
    let (state, actor, server) = fixture().await;
    let base = server.uri();
    let service = service(&state, &base);
    let secret = headers(&state).await;
    for missing in UPDATES {
        server.reset().await;
        manager_setup_api(
            &server,
            MANAGER,
            &service.manager_callback(),
            UPDATES
                .into_iter()
                .filter(|kind| *kind != missing)
                .collect(),
        )
        .await;
        assert!(
            service
                .configure_manager(&actor, &Default::default(), false)
                .await
                .is_err()
        );
        let saved = credentials::load(&state.db, &credential_descriptor())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(saved.fields.get("webhook_ready").unwrap(), "false");
        assert_eq!(headers(&state).await, secret);
        let requests = server.received_requests().await.unwrap();
        let sets: Vec<_> = requests
            .iter()
            .filter(|request| request.url.path().ends_with("/setWebhook"))
            .collect();
        assert_eq!(sets.len(), 1);
        assert_eq!(
            sets[0].body_json::<Value>().unwrap()["allowed_updates"],
            json!(UPDATES)
        );
        assert!(
            !requests
                .iter()
                .any(|request| request.url.path().ends_with("/deleteWebhook"))
        );
    }
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn telegram_manager_channel_configure_first_and_verify_rotated_live_credentials() {
    let (state, actor, server) = fixture().await;
    credentials::delete(&state.db, &credential_descriptor())
        .await
        .unwrap();
    server.reset().await;
    let base = server.uri();
    let service = service(&state, &base);
    manager_setup_api(
        &server,
        MANAGER,
        &service.manager_callback(),
        UPDATES.to_vec(),
    )
    .await;
    service
        .configure_manager(
            &actor,
            &[(
                "manager_bot_token".into(),
                Some(Zeroizing::new(MANAGER.into())),
            )]
            .into(),
            false,
        )
        .await
        .unwrap();
    let channel = pending_channel(&state, &actor, &server).await.bot;
    check_channel(&state, &server, &channel, MANAGER)
        .await
        .unwrap();
    let secret = headers(&state).await;
    let rotated = "100:rotated-test-secret";
    server.reset().await;
    manager_setup_api(
        &server,
        rotated,
        &service.manager_callback(),
        UPDATES.to_vec(),
    )
    .await;
    service
        .configure_manager(
            &actor,
            &[(
                "manager_bot_token".into(),
                Some(Zeroizing::new(rotated.into())),
            )]
            .into(),
            false,
        )
        .await
        .unwrap();
    assert_eq!(headers(&state).await, secret);
    server.reset().await;
    webhook_info(
        &server,
        rotated,
        json!({"url": service.manager_callback(), "allowed_updates": UPDATES}),
    )
    .await;
    let token = crate::services::channel_credentials::resolve_bot_token(
        &state.db,
        &state.encryption_keys,
        &TelegramAdapter::default(),
        &channel,
    )
    .await
    .unwrap();
    assert_eq!(token.as_str(), rotated);
    bot_identity_api(&server, rotated).await;
    let verified = verify_channel(&state, &server, &channel).await.unwrap().0;
    assert_eq!(verified.status, "active");
    assert!(verified.webhook_registered);
    assert_only_telegram_reads(&server, 1, 1).await;
    let saved = bots::get_bot(&state.db, &channel.id).await.unwrap();
    assert!(saved.bot_token_encrypted.is_empty());
    assert!(saved.webhook_secret_hash.is_empty());
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn telegram_manager_channel_unready_configuration_is_projected_without_lifecycle_writes() {
    use crate::handlers::channel_bots;
    use crate::test_utils::test_auth_user;
    use axum::extract::{Path, Query, State};
    let (state, actor, server) = fixture().await;
    let channel = register_manager_channel(&state, &actor, &server).await;
    let original = credentials::load(&state.db, &credential_descriptor())
        .await
        .unwrap()
        .unwrap();
    server.reset().await;
    for change in [
        doc! {"$set": {"fields.webhook_ready": "false"}},
        doc! {"$unset": {"fields.webhook_ready": ""}},
        doc! {"$set": {"fields.manager_bot_id": "200", "fields.webhook_ready": "true"}},
        doc! {"$unset": {"fields.manager_bot_id": ""}},
    ] {
        state
            .db
            .collection::<PlatformCredential>(CREDENTIALS)
            .replace_one(doc! {"_id": &original.id}, &original)
            .await
            .unwrap();
        state
            .db
            .collection::<PlatformCredential>(CREDENTIALS)
            .update_one(doc! {"provider": "telegram-new"}, change)
            .await
            .unwrap();
        let before = bots::get_bot(&state.db, &channel.id).await.unwrap();
        let listed = channel_bots::list_bots(
            State(state.clone()),
            test_auth_user(&actor),
            Query(Default::default()),
        )
        .await
        .unwrap()
        .0;
        assert_eq!(listed.bots.len(), 1);
        assert_eq!(listed.bots[0].status, "failed");
        assert!(!listed.bots[0].webhook_registered);
        let detail = channel_bots::get_bot(
            State(state.clone()),
            test_auth_user(&actor),
            Path(channel.id.clone()),
        )
        .await
        .unwrap()
        .0;
        assert_eq!(detail.status, "failed");
        assert!(!detail.webhook_registered);
        assert!(
            detail
                .connection
                .error
                .unwrap()
                .contains("Platform Credentials")
        );
        let saved = bots::get_bot(&state.db, &channel.id).await.unwrap();
        assert_eq!(
            bson::to_document(&before).unwrap(),
            bson::to_document(&saved).unwrap()
        );
        let update = channel_bots::update_bot(
            State(state.clone()),
            test_auth_user(&actor),
            Path(channel.id.clone()),
            axum::Json(serde_json::from_value(json!({"label": "Renamed manager"})).unwrap()),
        )
        .await
        .unwrap()
        .0;
        assert_eq!(update.status, "failed");
        assert!(!update.webhook_registered);
        assert!(
            update
                .connection
                .error
                .unwrap()
                .contains("Platform Credentials")
        );
        let saved = bots::get_bot(&state.db, &channel.id).await.unwrap();
        assert_eq!(saved.status, "active");
        assert!(saved.webhook_registered);
        assert!(saved.is_active);
        assert!(saved.error.is_none());
        assert_eq!(saved.label, "Renamed manager");
    }
    credentials::delete(&state.db, &credential_descriptor())
        .await
        .unwrap();
    let detail = channel_bots::get_bot(
        State(state.clone()),
        test_auth_user(&actor),
        Path(channel.id.clone()),
    )
    .await
    .unwrap()
    .0;
    assert_eq!(detail.status, "failed");
    assert!(detail.connection.error.is_some());
    assert_eq!(
        bots::get_bot(&state.db, &channel.id).await.unwrap().status,
        "active"
    );
    state
        .db
        .collection::<PlatformCredential>(CREDENTIALS)
        .insert_one(&original)
        .await
        .unwrap();
    let detail = channel_bots::get_bot(
        State(state.clone()),
        test_auth_user(&actor),
        Path(channel.id.clone()),
    )
    .await
    .unwrap()
    .0;
    assert_eq!(detail.status, "active");
    assert!(detail.webhook_registered);
    assert!(detail.connection.error.is_none());
    assert!(server.received_requests().await.unwrap().is_empty());
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn telegram_manager_channel_saved_unready_configuration_blocks_verification() {
    let (state, actor, server) = fixture().await;
    let channel = register_manager_channel(&state, &actor, &server).await;
    server.reset().await;
    for change in [
        doc! {"$set": {"fields.webhook_ready": "false"}},
        doc! {"$unset": {"fields.webhook_ready": ""}},
        doc! {"$set": {"fields.manager_bot_id": "200", "fields.webhook_ready": "true"}},
    ] {
        state
            .db
            .collection::<PlatformCredential>(CREDENTIALS)
            .update_one(doc! {"provider": "telegram-new"}, change)
            .await
            .unwrap();
        state.db.collection::<ChannelBot>(BOTS).update_one(
            doc! {"_id": &channel.id},
            doc! {"$set": {"status": "active", "webhook_registered": true, "error": bson::Bson::Null}},
        ).await.unwrap();
        assert!(verify_channel(&state, &server, &channel).await.is_err());
        let saved = bots::get_bot(&state.db, &channel.id).await.unwrap();
        assert_eq!(saved.status, "failed");
        assert!(!saved.webhook_registered);
        assert!(saved.error.unwrap().contains("Platform Credentials"));
        assert!(
            check_channel(&state, &server, &channel, MANAGER)
                .await
                .is_err()
        );
        assert!(server.received_requests().await.unwrap().is_empty());
    }
    credentials::delete(&state.db, &credential_descriptor())
        .await
        .unwrap();
    assert!(verify_channel(&state, &server, &channel).await.is_err());
    assert!(
        check_channel(&state, &server, &channel, MANAGER)
            .await
            .is_err()
    );
    assert!(server.received_requests().await.unwrap().is_empty());
    state.db.drop().await.unwrap();
}

#[tokio::test]
async fn telegram_manager_channel_get_me_failure_persists_safe_status_and_verify_recovers() {
    use crate::handlers::channel_bots;
    use crate::test_utils::test_auth_user;
    use axum::extract::{Path, Query, State};

    let (state, actor, server) = fixture().await;
    let channel = register_manager_channel(&state, &actor, &server).await;
    let manager_before = credentials::load(&state.db, &credential_descriptor())
        .await
        .unwrap()
        .unwrap();
    let secret_before = headers(&state).await;
    server.reset().await;
    Mock::given(method("GET"))
        .and(path(format!("/bot{MANAGER}/getMe")))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({
            "ok": false, "description": "Unauthorized: revoked provider credential",
        })))
        .mount(&server)
        .await;

    assert!(verify_channel(&state, &server, &channel).await.is_err());
    let saved = bots::get_bot(&state.db, &channel.id).await.unwrap();
    assert_eq!(saved.status, "failed");
    assert!(!saved.webhook_registered);
    assert!(saved.is_active);
    assert_eq!(
        saved.error.as_deref(),
        Some(crate::services::telegram_new_admin::MANAGER_WEBHOOK_ERROR)
    );
    assert_only_telegram_reads(&server, 0, 1).await;

    let listed = channel_bots::list_bots(
        State(state.clone()),
        test_auth_user(&actor),
        Query(Default::default()),
    )
    .await
    .unwrap()
    .0;
    assert_eq!(listed.bots[0].status, "failed");
    assert!(!listed.bots[0].webhook_registered);
    let detail = channel_bots::get_bot(
        State(state.clone()),
        test_auth_user(&actor),
        Path(channel.id.clone()),
    )
    .await
    .unwrap()
    .0;
    assert_eq!(detail.status, "failed");
    assert!(!detail.webhook_registered);
    assert_eq!(detail.connection.error, saved.error);

    server.reset().await;
    bot_identity_api(&server, MANAGER).await;
    webhook_info(
        &server,
        MANAGER,
        json!({
            "url": bots::webhook_url(&state.config.base_url, &channel), "allowed_updates": UPDATES,
        }),
    )
    .await;
    let verified = verify_channel(&state, &server, &saved).await.unwrap().0;
    assert_eq!(verified.status, "active");
    assert!(verified.webhook_registered);
    let recovered = bots::get_bot(&state.db, &channel.id).await.unwrap();
    assert_eq!(recovered.status, "active");
    assert!(recovered.webhook_registered);
    assert!(recovered.error.is_none());
    assert_eq!(headers(&state).await, secret_before);
    let manager_after = credentials::load(&state.db, &credential_descriptor())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        bson::to_document(&manager_before).unwrap(),
        bson::to_document(&manager_after).unwrap()
    );
    assert_only_telegram_reads(&server, 1, 1).await;

    server.reset().await;
    state
        .db
        .collection::<ChannelBot>(BOTS)
        .update_one(
            doc! {"_id": &channel.id},
            doc! {"$set": {
                "credential_source": "user",
                "bot_token_encrypted": bson::Binary {
                    subtype: bson::spec::BinarySubtype::Generic,
                    bytes: state.encryption_keys.encrypt(MANAGER.as_bytes()).await.unwrap(),
                },
            }},
        )
        .await
        .unwrap();
    let ordinary = bots::get_bot(&state.db, &channel.id).await.unwrap();
    Mock::given(method("GET"))
        .and(path(format!("/bot{MANAGER}/getMe")))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({
            "ok": false, "description": "Unauthorized",
        })))
        .mount(&server)
        .await;
    assert!(verify_channel(&state, &server, &ordinary).await.is_err());
    let unchanged = bots::get_bot(&state.db, &channel.id).await.unwrap();
    assert_eq!(
        bson::to_document(&ordinary).unwrap(),
        bson::to_document(&unchanged).unwrap()
    );
    assert_only_telegram_reads(&server, 0, 1).await;
    state.db.drop().await.unwrap();
}

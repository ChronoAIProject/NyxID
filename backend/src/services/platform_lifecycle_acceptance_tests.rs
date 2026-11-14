#![cfg(test)]

use super::{
    channel_routing_service as routes, platform_bot_config_service as profiles,
    platform_onboarding_test_support as support, platform_provisioning_service as provisioning,
    platform_setup_service as setups, telegram_new_tests as telegram,
};
use crate::{
    errors::AppError,
    handlers::nyxbot,
    models::{
        channel_conversation::ChannelConversation,
        nyxbot_channel::{NyxbotChannel, NyxbotThread},
        platform_installation::PlatformInstallation,
    },
};
use axum::{
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
};
use mongodb::bson::{self, Document, doc};
use sha2::{Digest, Sha256};

// Persist hostile legacy routing/settings beside real managed resources. These
// rows deliberately model pre-conversion snapshots, not activation authority.
async fn legacy_rows(
    f: &support::Fixture,
    bot: &str,
    owner: &str,
) -> (routes::AgentRoute, NyxbotChannel, NyxbotThread) {
    let key = super::key_service::create_api_key(
        &f.state.db,
        owner,
        "Legacy callback",
        "read",
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        None,
        Some("https://callback.nyxid.test/legacy"),
    )
    .await
    .unwrap();
    let now = bson::DateTime::now();
    let id = uuid::Uuid::new_v4().to_string();
    let conversation: ChannelConversation = bson::from_document(doc! {
        "_id": &id, "user_id": owner, "channel_bot_id": bot, "platform": "telegram",
        "platform_conversation_id": "-700", "platform_conversation_type": "group",
        "agent_api_key_id": &key.id, "default_agent": true, "allow_agent_initiated": true,
        "is_active": true, "created_at": now, "updated_at": now
    })
    .unwrap();
    f.state
        .db
        .collection::<ChannelConversation>(crate::models::channel_conversation::COLLECTION_NAME)
        .insert_one(&conversation)
        .await
        .unwrap();
    let channel: NyxbotChannel = bson::from_document(doc! {
        "_id": uuid::Uuid::new_v4().to_string(), "user_id": owner, "channel_bot_id": bot,
        "platform": "telegram", "bot_label": "Legacy channel", "transport": "direct",
        "status": "active", "route_api_key_id": &key.id, "route_id": &id,
        "owner_sender_ids": ["700"], "private_chats": "owner",
        "created_at": now, "updated_at": now
    })
    .unwrap();
    f.state
        .db
        .collection::<NyxbotChannel>(crate::models::nyxbot_channel::COLLECTION_NAME)
        .insert_one(&channel)
        .await
        .unwrap();
    let chat: NyxbotThread = bson::from_document(doc! {
        "_id": uuid::Uuid::new_v4().to_string(), "user_id": owner, "channel_id": &channel.id,
        "partition": "legacy-group", "kind": "group", "platform_chat_id": "-700",
        "owner_seen": true, "reply_mode": "all", "threads": "follow",
        "created_at": now, "updated_at": now
    })
    .unwrap();
    f.state
        .db
        .collection::<NyxbotThread>(crate::models::nyxbot_channel::THREADS_COLLECTION_NAME)
        .insert_one(&chat)
        .await
        .unwrap();
    (
        routes::AgentRoute {
            conversation,
            callback_url: "https://callback.nyxid.test/legacy".into(),
            api_key_id: key.id,
        },
        channel,
        chat,
    )
}

async fn assert_closed(
    f: &support::Fixture,
    route: &routes::AgentRoute,
    channel: &NyxbotChannel,
    chat: &NyxbotThread,
) {
    let owner = &channel.user_id;
    let bot = &channel.channel_bot_id;
    // The old ordinary group predicate would admit this guest after owner_seen.
    assert_eq!(
        super::channel_thread_follow_service::eligible(channel, chat, "800"),
        Some(true)
    );
    assert!(matches!(
        super::channel_thread_follow_service::access(&f.state.db, owner, &channel.id).await,
        Err(AppError::Forbidden(_))
    ));
    for sender in ["700", "800"] {
        assert!(
            routes::resolve_agent(&f.state.db, bot, "-700", Some(sender), owner)
                .await
                .unwrap()
                .is_none()
        );
    }
    assert!(matches!(
        routes::load_callback_key(&f.state.db, route, owner).await,
        Err(AppError::Forbidden(_))
    ));
    assert!(matches!(
        routes::update_conversation(
            &f.state.db,
            &route.conversation.id,
            owner,
            Some(&route.api_key_id),
            Some(true),
            Some(true),
            Some(true)
        )
        .await,
        Err(AppError::Forbidden(_))
    ));
    let routes_collection = f
        .state
        .db
        .collection::<ChannelConversation>(crate::models::channel_conversation::COLLECTION_NAME);
    let was_active = routes_collection
        .find_one(doc! {"_id": &route.conversation.id})
        .await
        .unwrap()
        .is_some_and(|row| row.is_active);
    let deleted = routes::delete_conversation(&f.state.db, &route.conversation.id, owner).await;
    if was_active {
        assert!(matches!(deleted, Err(AppError::Forbidden(_))));
    } else {
        // Removing an inactive route is a no-op and never restores routing.
        deleted.unwrap();
        assert!(
            !routes_collection
                .find_one(doc! {"_id": &route.conversation.id})
                .await
                .unwrap()
                .is_some_and(|row| row.is_active)
        );
    }
    assert!(matches!(
        nyxbot::chats::set_private_chats(&f.state, owner, &channel.id, "everyone").await,
        Err(AppError::Forbidden(_))
    ));
    for settings in [
        nyxbot::chats::ChatSettings {
            members: Some("everyone".into()),
            ..Default::default()
        },
        nyxbot::chats::ChatSettings {
            threads: Some("follow".into()),
            ..Default::default()
        },
        nyxbot::chats::ChatSettings {
            agent_id: Some("default".into()),
            allow_posts: Some(true),
            ..Default::default()
        },
    ] {
        assert!(matches!(
            nyxbot::chats::update_chat(&f.state, owner, &chat.id, &settings).await,
            Err(AppError::Forbidden(_))
        ));
    }
    let agent = bson::from_document(
        doc! {"_id": uuid::Uuid::new_v4().to_string(), "user_id": owner,
        "kind": "specialist", "name": "alternate", "created_by": "user", "model": "test-model",
        "created_at": bson::DateTime::now(), "updated_at": bson::DateTime::now()},
    )
    .unwrap();
    assert!(matches!(
        nyxbot::link(&f.state, owner, &channel.id, &agent).await,
        Err(AppError::Forbidden(_))
    ));
    assert!(matches!(
        nyxbot::connect(&f.state, owner, None, bot, &agent).await,
        Err(AppError::Forbidden(_))
    ));
    let body = serde_json::to_vec(&serde_json::json!({"message_id": "legacy-message", "platform": "telegram", "correlation_id": "legacy-correlation", "conversation_id": "-700", "sender_id": "800", "text": "Guest message"})).unwrap();
    let hash = format!("{:x}", Sha256::digest(&body));
    let token = crate::crypto::jwt::generate_relay_callback_token(
        &f.state.jwt_keys,
        &f.state.config,
        "legacy-correlation",
        &route.api_key_id,
        "legacy-message",
        "telegram",
        &hash,
    )
    .unwrap();
    let mut headers = HeaderMap::new();
    headers.insert("x-nyxid-callback-token", token.parse().unwrap());
    let response = nyxbot::relay_callback(
        State(f.state.clone()),
        Path(channel.id.clone()),
        headers,
        Bytes::from(body),
    )
    .await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let stored = f
        .state
        .db
        .collection::<NyxbotChannel>(crate::models::nyxbot_channel::COLLECTION_NAME)
        .find_one(doc! {"_id": &channel.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(stored.private_chats.as_deref(), Some("owner"));
    assert!(stored.agent_id.is_none());
}

#[tokio::test]
async fn platform_onboarding_manager_stale_routes_remain_closed_after_pause_retirement_and_profile_purge()
 {
    let f = Box::pin(support::distinct_fixture()).await;
    let (route, channel, chat) = legacy_rows(&f, &f.profile.channel_bot_id, &f.operator).await;
    assert_closed(&f, &route, &channel, &chat).await;
    let paused = profiles::disable(
        &f.state.db,
        &f.profile.id,
        f.profile.generation,
        false,
        &f.operator,
    )
    .await
    .unwrap();
    assert_closed(&f, &route, &channel, &chat).await;
    profiles::disable(
        &f.state.db,
        &paused.id,
        paused.generation,
        true,
        &f.operator,
    )
    .await
    .unwrap();
    assert_closed(&f, &route, &channel, &chat).await;
    f.state
        .db
        .collection::<Document>(crate::models::platform_bot_config::COLLECTION_NAME)
        .delete_one(doc! {"_id": &f.profile.id})
        .await
        .unwrap();
    assert_closed(&f, &route, &channel, &chat).await;
    assert!(
        !f.server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .any(|r| r.url.path().ends_with("/deleteWebhook"))
    );
}

async fn registered_child(f: &support::Fixture) -> String {
    Box::pin(f.observe()).await;
    let (candidate, native, digest) =
        provisioning::child_candidate(&f.state.db, &f.state.encryption_keys, &f.owner, &f.setup.id)
            .await
            .unwrap();
    let approved = provisioning::approve_child_owned(
        &f.state.db,
        &f.state.encryption_keys,
        &f.owner,
        &f.setup.id,
        candidate.revision - 1,
        native.revision,
        900,
        native.manager_revision.unwrap(),
        &digest,
    )
    .await
    .unwrap();
    let job = f.claim().await;
    Box::pin(provisioning::execute_step(&f.state, &job))
        .await
        .unwrap();
    let bot = approved.resources.channel_bot_id.unwrap();
    telegram::provider_connection(&f.server, &bot, 503).await;
    let base = f.server.uri();
    telegram::service(&f.state, &base)
        .complete_pending_creations()
        .await
        .unwrap();
    let registered = f
        .state
        .db
        .collection::<crate::models::channel_bot::ChannelBot>(
            crate::models::channel_bot::COLLECTION_NAME,
        )
        .find_one(doc! {"_id": &bot})
        .await
        .unwrap()
        .unwrap();
    assert!(registered.is_active);
    assert!(!registered.webhook_registered);
    bot
}

#[tokio::test]
async fn platform_onboarding_child_legacy_group_follow_setters_and_signed_stale_callback_stay_closed()
 {
    let f = Box::pin(support::distinct_fixture()).await;
    let bot = registered_child(&f).await;
    let (route, channel, chat) = legacy_rows(&f, &bot, &f.owner).await;
    assert_closed(&f, &route, &channel, &chat).await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path(
            "/bot900:child-test-secret/setWebhook",
        ))
        .respond_with(
            wiremock::ResponseTemplate::new(200)
                .set_body_json(serde_json::json!({"ok": true, "result": true})),
        )
        .with_priority(1)
        .mount(&f.server)
        .await;
    f.state
        .db
        .collection::<Document>(crate::models::telegram_bot_request::COLLECTION_NAME)
        .update_one(
            doc! {"_id": &bot},
            doc! {"$unset": {"next_connection_attempt_at": ""}},
        )
        .await
        .unwrap();
    let base = f.server.uri();
    telegram::service(&f.state, &base)
        .complete_pending_creations()
        .await
        .unwrap();
    assert_eq!(
        setups::get_owned(&f.state.db, &f.owner, &f.setup.id)
            .await
            .unwrap()
            .state,
        crate::models::platform_setup::SetupState::ReadyForTest
    );
    assert_closed(&f, &route, &channel, &chat).await;
    // State corruption/purge must not remove the durable ordinary-route fence.
    // This does not claim a successful public activation ceremony.
    for stage in ["active", "paused", "cancelled", "unknown_future_state"] {
        f.state
            .db
            .collection::<Document>(crate::models::platform_installation::COLLECTION_NAME)
            .update_one(
                doc! {"channel_bot_id": &bot},
                doc! {"$set": {"state": stage}},
            )
            .await
            .unwrap();
        assert_closed(&f, &route, &channel, &chat).await;
    }
    f.state
        .db
        .collection::<Document>(crate::models::platform_setup::COLLECTION_NAME)
        .delete_one(doc! {"_id": &f.setup.id})
        .await
        .unwrap();
    assert_closed(&f, &route, &channel, &chat).await;
    f.state
        .db
        .collection::<Document>(crate::models::platform_installation::COLLECTION_NAME)
        .delete_one(doc! {"channel_bot_id": &bot})
        .await
        .unwrap();
    assert_closed(&f, &route, &channel, &chat).await;
    assert!(
        !f.server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .any(|r| r.url.path().contains("chat/completions"))
    );
}

#[tokio::test]
async fn platform_onboarding_manager_rotation_and_retirement_preserve_shared_webhook_with_managed_child()
 {
    use wiremock::{
        Mock, ResponseTemplate,
        matchers::{method, path},
    };
    let f = Box::pin(support::distinct_fixture()).await;
    let bot = registered_child(&f).await;
    let descriptor = super::channel_adapters::telegram_new::credential_descriptor();
    let before = super::platform_credential_service::load_decrypted(
        &f.state.db,
        &f.state.encryption_keys,
        &descriptor,
    )
    .await
    .unwrap();
    let secret = before
        .get(super::platform_credential_service::VERIFY_TOKEN_FIELD)
        .unwrap()
        .to_owned();
    let observation = before.get("observation_id").unwrap().to_owned();
    for (endpoint, result) in [
        (
            "getMe",
            serde_json::json!({"id": 100, "is_bot": true, "username": "NyxSetupBot", "can_manage_bots": true}),
        ),
        (
            "getWebhookInfo",
            serde_json::json!({"url": "https://api.nyxid.test/api/v1/webhooks/channel/telegram-new/manager", "allowed_updates": ["message", "edited_message", "channel_post", "callback_query", "managed_bot"]}),
        ),
        ("setWebhook", serde_json::json!(true)),
    ] {
        Mock::given(method("POST"))
            .and(path(format!("/bot100:rotated-test-secret/{endpoint}")))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"ok": true, "result": result})),
            )
            .mount(&f.server)
            .await;
    }
    let base = f.server.uri();
    let service = telegram::service(&f.state, &base);
    assert!(matches!(
        service.clear_manager().await,
        Err(AppError::Conflict(_))
    ));
    // An OAuth-app admin token that raced conversion cannot change the manager.
    assert!(matches!(
        service
            .configure_manager_for(
                &f.operator,
                false,
                &[(
                    "manager_bot_token".into(),
                    Some(zeroize::Zeroizing::new("100:rotated-test-secret".into())),
                )]
                .into(),
                false,
            )
            .await,
        Err(AppError::Forbidden(_))
    ));
    service
        .configure_manager(
            &f.operator,
            &[(
                "manager_bot_token".into(),
                Some(zeroize::Zeroizing::new("100:rotated-test-secret".into())),
            )]
            .into(),
            false,
        )
        .await
        .unwrap();
    let after = super::platform_credential_service::load_decrypted(
        &f.state.db,
        &f.state.encryption_keys,
        &descriptor,
    )
    .await
    .unwrap();
    assert_eq!(
        after.get(super::platform_credential_service::VERIFY_TOKEN_FIELD),
        Some(secret.as_str())
    );
    assert_eq!(after.get("observation_id"), Some(observation.as_str()));
    let rotated = profiles::get(&f.state.db, &f.profile.id).await.unwrap();
    assert!(!rotated.enabled);
    assert!(rotated.generation > f.profile.generation);
    profiles::disable(
        &f.state.db,
        &rotated.id,
        rotated.generation,
        true,
        &f.operator,
    )
    .await
    .unwrap();
    assert!(matches!(
        service.clear_manager().await,
        Err(AppError::Conflict(_))
    ));
    assert!(
        f.state
            .db
            .collection::<crate::models::channel_bot::ChannelBot>(
                crate::models::channel_bot::COLLECTION_NAME
            )
            .find_one(doc! {"_id": &bot, "is_active": true})
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        f.state
            .db
            .collection::<PlatformInstallation>(
                crate::models::platform_installation::COLLECTION_NAME
            )
            .find_one(doc! {"channel_bot_id": &bot})
            .await
            .unwrap()
            .is_some()
    );
    let requests = f.server.received_requests().await.unwrap();
    assert!(
        !requests
            .iter()
            .any(|r| r.url.path().ends_with("/deleteWebhook"))
    );
    let sent = requests
        .iter()
        .find(|r| r.url.path() == "/bot100:rotated-test-secret/setWebhook")
        .unwrap();
    let body: serde_json::Value = serde_json::from_slice(&sent.body).unwrap();
    assert_eq!(body["secret_token"].as_str(), Some(secret.as_str()));
    assert_eq!(body["url"], service.manager_callback());
    assert!(
        super::platform_credential_service::load(&f.state.db, &descriptor)
            .await
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn platform_onboarding_legacy_personal_callback_is_valid_until_real_manager_conversion() {
    let f = Box::pin(support::distinct_fixture()).await;
    let bots = f
        .state
        .db
        .collection::<Document>(crate::models::channel_bot::COLLECTION_NAME);
    // A second, unclaimed manager identity represents an existing personal
    // installation. No marker on an opted-in manager is removed or reused.
    let mut legacy = bots
        .find_one(doc! {"_id": &f.profile.channel_bot_id})
        .await
        .unwrap()
        .unwrap();
    let bot = uuid::Uuid::new_v4().to_string();
    legacy.insert("_id", &bot);
    legacy.insert("platform_bot_id", "101");
    legacy.remove("platform_profile_id");
    legacy.remove("platform_installation_id");
    bots.insert_one(legacy).await.unwrap();
    f.state
        .db
        .collection::<Document>(crate::models::platform_credential::COLLECTION_NAME)
        .update_one(
            doc! {"provider": "telegram-new"},
            doc! {"$set": {"fields.manager_bot_id": "101"}},
        )
        .await
        .unwrap();
    let (route, channel, chat) = legacy_rows(&f, &bot, &f.operator).await;
    let resolved = routes::resolve_agent(&f.state.db, &bot, "-700", Some("800"), &f.operator)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(resolved.api_key_id, route.api_key_id);
    routes::load_callback_key(&f.state.db, &route, &f.operator)
        .await
        .unwrap();
    routes::update_conversation(
        &f.state.db,
        &route.conversation.id,
        &f.operator,
        None,
        Some(true),
        Some(true),
        Some(true),
    )
    .await
    .unwrap();
    super::channel_thread_follow_service::access(&f.state.db, &f.operator, &channel.id)
        .await
        .unwrap();
    let converted = profiles::convert(
        &f.state.db,
        &f.operator,
        &bot,
        &f.profile.funding_owner_user_id,
    )
    .await
    .unwrap();
    assert!(!converted.enabled);
    let retired_link = f
        .state
        .db
        .collection::<NyxbotChannel>(crate::models::nyxbot_channel::COLLECTION_NAME)
        .find_one(doc! {"_id": &channel.id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(retired_link.status, "disconnected");
    // Simulate a legacy writer restoring its captured link. Even this hostile
    // stale snapshot cannot resurrect dispatch through the durable bot marker.
    f.state
        .db
        .collection::<NyxbotChannel>(crate::models::nyxbot_channel::COLLECTION_NAME)
        .replace_one(doc! {"_id": &channel.id}, &channel)
        .await
        .unwrap();
    assert_closed(&f, &route, &channel, &chat).await;
    let key = f
        .state
        .db
        .collection::<crate::models::api_key::ApiKey>(crate::models::api_key::COLLECTION_NAME)
        .find_one(doc! {"_id": &route.api_key_id})
        .await
        .unwrap();
    assert!(key.is_none_or(|key| !key.is_active));
}

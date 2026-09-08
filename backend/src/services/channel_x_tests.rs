use axum::{
    Json,
    extract::{Path, State},
};
use bson::doc;
use serde_json::json;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{body_string_contains, header, method, path},
};

use super::{
    channel_adapters::x::{REQUIRED_SCOPES, XAdapter},
    channel_bot_service, channel_credentials,
    channel_managed::{ManagedOnboardingInput, ManagedProgress, PlatformCredentialBacking},
    channel_platform::{CredentialResolution, Ingestion, PlatformAdapter},
    channel_poll_service, platform_credential_service,
};
use crate::{
    AppState,
    errors::AppError,
    models::{
        channel_bot::{COLLECTION_NAME as BOTS, ChannelBot},
        provider_config::{COLLECTION_NAME as PROVIDERS, ProviderConfig},
        user_api_key::{COLLECTION_NAME as KEYS, UserApiKey},
    },
    test_utils::{connect_test_database, test_app_state, test_auth_user, test_user},
};

async fn fixture() -> (AppState, XAdapter, MockServer, String, String) {
    let db = connect_test_database("x_channel")
        .await
        .expect("test MongoDB");
    let state = test_app_state(db);
    super::audit_service::init_audit_chain_hmac_key(zeroize::Zeroizing::new([2u8; 32]));
    let server = MockServer::start().await;
    let owner = uuid::Uuid::new_v4().to_string();
    state
        .db
        .collection(crate::models::user::COLLECTION_NAME)
        .insert_one(test_user(&owner, crate::models::user::UserType::Person))
        .await
        .unwrap();
    let provider_id = uuid::Uuid::new_v4().to_string();
    let mut provider: ProviderConfig = bson::from_document(doc! {
        "_id": &provider_id, "slug": "twitter", "name": "X", "provider_type": "oauth2", "is_active": true,
        "authorization_url": "https://x.com/i/oauth2/authorize", "token_url": format!("{}/token", server.uri()),
        "supports_pkce": true, "credential_mode": "user", "default_scopes": REQUIRED_SCOPES,
        "created_by": &owner, "created_at": bson::DateTime::now(), "updated_at": bson::DateTime::now(),
    }).unwrap();
    provider.client_id_encrypted = Some(
        state
            .encryption_keys
            .encrypt(b"platform-client")
            .await
            .unwrap(),
    );
    provider.client_secret_encrypted = Some(
        state
            .encryption_keys
            .encrypt(b"platform-secret")
            .await
            .unwrap(),
    );
    state
        .db
        .collection(PROVIDERS)
        .insert_one(provider)
        .await
        .unwrap();
    let id = uuid::Uuid::new_v4().to_string();
    let mut key: UserApiKey = bson::from_document(doc! {
        "_id": &id, "user_id": &owner, "label": "X account", "credential_type": "oauth2", "credential_source": "platform",
        "provider_config_id": provider_id, "connection_id": uuid::Uuid::new_v4().to_string(),
        "status": "active", "token_scopes": REQUIRED_SCOPES.join(" "),
        "created_at": bson::DateTime::now(), "updated_at": bson::DateTime::now(),
    }).unwrap();
    key.access_token_encrypted = Some(
        state
            .encryption_keys
            .encrypt(b"live-access-token")
            .await
            .unwrap(),
    );
    key.refresh_token_encrypted = Some(
        state
            .encryption_keys
            .encrypt(b"refresh-token")
            .await
            .unwrap(),
    );
    state.db.collection(KEYS).insert_one(key).await.unwrap();
    let adapter = XAdapter {
        api_base: Some(server.uri()),
    };
    (state, adapter, server, owner, id)
}

async fn identity(server: &MockServer, id: &str) {
    Mock::given(method("GET"))
        .and(path("/2/users/me"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"data": {"id": id, "username": "support"}})),
        )
        .mount(server)
        .await;
}

async fn create(
    state: &AppState,
    adapter: &XAdapter,
    owner: &str,
    key: &str,
) -> Result<ChannelBot, AppError> {
    channel_bot_service::create_managed_bot(
        &state.db,
        &state.config,
        &state.encryption_keys,
        &state.http_client,
        adapter,
        owner,
        "Support",
        &ManagedOnboardingInput(
            [("connection_id".into(), zeroize::Zeroizing::new(key.into()))].into(),
        ),
        &ManagedProgress::default(),
    )
    .await
    .map(|result| result.bot)
}

async fn insert_bot(state: &AppState, owner: &str, connection: &str) -> ChannelBot {
    let bot: ChannelBot = bson::from_document(doc! {
        "_id": uuid::Uuid::new_v4().to_string(), "user_id": owner, "platform": "x", "label": "Support",
        "credential_source": "connection", "connection_id": connection, "poll_cursor": "100",
        "bot_token_encrypted": bson::Binary {subtype: bson::spec::BinarySubtype::Generic, bytes: vec![]},
        "platform_bot_id": "10", "platform_bot_username": "support", "webhook_registered": false,
        "webhook_secret_hash": "", "status": "active", "is_active": true,
        "created_at": bson::DateTime::now(), "updated_at": bson::DateTime::now(),
    }).unwrap();
    state
        .db
        .collection::<ChannelBot>(BOTS)
        .insert_one(&bot)
        .await
        .unwrap();
    bot
}

async fn due(state: &AppState, bot: &ChannelBot) {
    state
        .db
        .collection::<bson::Document>(BOTS)
        .update_one(
            doc! {"_id": &bot.id},
            doc! {"$set": {"last_polled_at": null, "poll_backoff_until": null}},
        )
        .await
        .unwrap();
}

#[test]
fn descriptors_keep_all_previous_adapters_on_stored_webhook_defaults() {
    let cache =
        std::sync::Arc::new(super::provider_token_exchange_service::TokenExchangeCache::new());
    for adapter in super::channel_adapters::registered_adapters(&cache) {
        if adapter.platform_id() == "x" {
            assert_eq!(
                adapter.ingestion(),
                Ingestion::Poll {
                    min_interval_secs: 60
                }
            );
            assert_eq!(
                adapter.credential_resolution(),
                CredentialResolution::OAuthConnection {
                    provider_slug: "twitter",
                    required_scopes: REQUIRED_SCOPES
                }
            );
            assert!(adapter.registration().managed_only);
            assert!(!adapter.registration().webhook_ingestion);
            assert!(adapter.registration().fields.is_empty());
            assert!(adapter.dedup_inbound_by_platform_message_id());
            assert_eq!(
                adapter.managed_onboarding().unwrap().flow,
                "oauth_connection"
            );
            assert!(matches!(
                adapter.platform_credentials().unwrap().backing,
                PlatformCredentialBacking::ProviderOAuth {
                    provider_slug: "twitter"
                }
            ));
        } else {
            assert_eq!(adapter.ingestion(), Ingestion::Webhook);
            assert_eq!(
                adapter.credential_resolution(),
                CredentialResolution::StoredToken
            );
            assert!(!adapter.registration().managed_only);
            assert!(adapter.registration().webhook_ingestion);
            if let Some(descriptor) = adapter.platform_credentials() {
                assert!(matches!(
                    descriptor.backing,
                    PlatformCredentialBacking::Stored
                ));
            }
        }
    }
}

#[tokio::test]
async fn onboarding_initializes_cursor_checks_scopes_and_rejects_duplicate_account() {
    let (state, adapter, server, owner, key) = fixture().await;
    identity(&server, "10").await;
    Mock::given(method("GET"))
        .and(path("/2/dm_events"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": [{"id": "100"}]})))
        .mount(&server)
        .await;
    state
        .db
        .collection::<bson::Document>(KEYS)
        .update_one(
            doc! {"_id": &key},
            doc! {"$set": {"token_scopes": "tweet.read users.read"}},
        )
        .await
        .unwrap();
    let error = create(&state, &adapter, &owner, &key).await.unwrap_err();
    assert!(error.to_string().contains("consent"));
    assert!(server.received_requests().await.unwrap().is_empty());
    state
        .db
        .collection::<bson::Document>(KEYS)
        .update_one(
            doc! {"_id": &key},
            doc! {"$set": {"token_scopes": REQUIRED_SCOPES.join(" ")}},
        )
        .await
        .unwrap();
    let bot = create(&state, &adapter, &owner, &key).await.unwrap();
    assert_eq!(bot.status, "active");
    assert_eq!(bot.credential_source, "connection");
    assert_eq!(bot.connection_id.as_deref(), Some(key.as_str()));
    assert_eq!(bot.poll_cursor.as_deref(), Some("100"));
    assert!(bot.last_polled_at.is_some());
    assert!(!bot.webhook_registered);
    assert!(bot.bot_token_encrypted.is_empty());
    assert!(bot.webhook_secret_hash.is_empty());
    assert!(matches!(
        create(&state, &adapter, &owner, &key).await,
        Err(AppError::Conflict(_))
    ));
}

#[tokio::test]
async fn polling_sweeps_share_one_lease_and_deduplicate_overlapping_batches() {
    let (state, _, server, owner, key) = fixture().await;
    let bot = insert_bot(&state, &owner, &key).await;
    let agent = uuid::Uuid::new_v4().to_string();
    state.db.collection::<bson::Document>(crate::models::api_key::COLLECTION_NAME).insert_one(doc! {
        "_id": &agent, "user_id": &owner, "name": "agent", "key_prefix": "nyxid_ag", "key_hash": "deadbeef".repeat(8),
        "scopes": "read write", "is_active": true, "callback_url": format!("{}/callback", server.uri()), "created_at": bson::DateTime::now(),
    }).await.unwrap();
    state.db.collection::<bson::Document>(crate::models::channel_conversation::COLLECTION_NAME).insert_one(doc! {
        "_id": uuid::Uuid::new_v4().to_string(), "user_id": &owner, "channel_bot_id": &bot.id, "platform": "x",
        "platform_conversation_id": "10-20", "platform_conversation_type": "private", "agent_api_key_id": agent,
        "default_agent": false, "is_active": true, "created_at": bson::DateTime::now(), "updated_at": bson::DateTime::now(),
    }).await.unwrap();
    Mock::given(method("GET")).and(path("/2/dm_events")).respond_with(ResponseTemplate::new(200)
        .set_delay(std::time::Duration::from_millis(150))
        .set_body_json(json!({"data": [{"id": "101", "sender_id": "20", "dm_conversation_id": "10-20", "text": "private body"}]})))
        .expect(2).mount(&server).await;
    Mock::given(method("POST"))
        .and(path("/callback"))
        .respond_with(ResponseTemplate::new(202))
        .expect(1)
        .mount(&server)
        .await;
    let adapters = || {
        vec![Box::new(XAdapter {
            api_base: Some(server.uri()),
        }) as Box<dyn PlatformAdapter>]
    };
    let (a, b) = tokio::join!(
        channel_poll_service::sweep_with_adapters(&state, adapters()),
        channel_poll_service::sweep_with_adapters(&state, adapters())
    );
    a.unwrap();
    b.unwrap();
    let current = channel_bot_service::get_bot(&state.db, &bot.id)
        .await
        .unwrap();
    assert_eq!(current.poll_cursor.as_deref(), Some("101"));
    assert!(current.poll_lease_until.is_none());
    due(&state, &bot).await;
    state
        .db
        .collection::<bson::Document>(BOTS)
        .update_one(doc! {"_id": &bot.id}, doc! {"$set": {"poll_cursor": "100"}})
        .await
        .unwrap();
    channel_poll_service::sweep_with_adapters(&state, adapters())
        .await
        .unwrap();
    let messages = state
        .db
        .collection::<bson::Document>(crate::models::channel_message::COLLECTION_NAME);
    assert_eq!(
        messages
            .count_documents(doc! {"channel_bot_id": &bot.id})
            .await
            .unwrap(),
        1
    );
    let message = messages
        .find_one(doc! {"channel_bot_id": &bot.id})
        .await
        .unwrap()
        .unwrap();
    assert!(!format!("{message:?}").contains("private body"));
    assert_eq!(message.get_str("callback_status").unwrap(), "delivered");
}

#[tokio::test]
async fn polling_backoff_and_error_threshold_release_lease_and_audit_failure() {
    let (state, adapter, server, owner, key) = fixture().await;
    let bot = insert_bot(&state, &owner, &key).await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(429).insert_header("retry-after", "120"))
        .expect(1)
        .mount(&server)
        .await;
    channel_poll_service::poll_bot(&state, &adapter, &bot.id, 60)
        .await
        .unwrap();
    let current = channel_bot_service::get_bot(&state.db, &bot.id)
        .await
        .unwrap();
    assert!(current.poll_backoff_until.unwrap() > chrono::Utc::now());
    assert_eq!(current.poll_cursor, bot.poll_cursor);
    assert_eq!(current.poll_error_count, 0);
    assert!(current.poll_lease_until.is_none());
    channel_poll_service::poll_bot(&state, &adapter, &bot.id, 60)
        .await
        .unwrap();
    server.reset().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(503).set_body_string("UPSTREAM-SECRET"))
        .expect(5)
        .mount(&server)
        .await;
    for count in 1..=channel_poll_service::ERROR_THRESHOLD {
        due(&state, &bot).await;
        channel_poll_service::poll_bot(&state, &adapter, &bot.id, 60)
            .await
            .unwrap();
        let current = channel_bot_service::get_bot(&state.db, &bot.id)
            .await
            .unwrap();
        assert_eq!(current.poll_error_count, count);
        assert!(current.poll_lease_until.is_none());
        assert_eq!(current.poll_cursor, bot.poll_cursor);
        assert_eq!(current.status, if count == 5 { "failed" } else { "active" });
    }
    let audits = state
        .db
        .collection::<bson::Document>(crate::models::audit_log::COLLECTION_NAME);
    let audit = audits
        .find_one(doc! {"event_type": "channel_bot_failed"})
        .await
        .unwrap()
        .unwrap();
    assert!(!format!("{audit:?}").contains("UPSTREAM-SECRET"));
    assert!(audit.contains_key("seq"));
}

#[tokio::test]
async fn connection_resolver_refreshes_in_place_and_never_uses_stale_tokens() {
    for status in [200, 400, 503] {
        let (state, adapter, server, owner, key) = fixture().await;
        let bot = insert_bot(&state, &owner, &key).await;
        state.db.collection::<bson::Document>(KEYS).update_one(doc! {"_id": &key},
            doc! {"$set": {"expires_at": bson::DateTime::from_chrono(chrono::Utc::now() - chrono::Duration::minutes(1))}}).await.unwrap();
        Mock::given(method("POST")).and(path("/token")).and(body_string_contains("grant_type=refresh_token"))
            .and(body_string_contains("client_id=platform-client"))
            .respond_with(ResponseTemplate::new(status).set_body_json(if status == 200 {
                json!({"access_token": "fresh-access-token", "refresh_token": "fresh-refresh-token", "token_type": "bearer", "expires_in": 7200})
            } else {json!({"error": "invalid_grant", "error_description": "UPSTREAM-SECRET"})})).expect(1).mount(&server).await;
        let result = channel_credentials::resolve_bot_token(
            &state.db,
            &state.encryption_keys,
            &adapter,
            &bot,
        )
        .await;
        if status == 200 {
            assert_eq!(result.unwrap().as_str(), "fresh-access-token");
        } else {
            let error = result.unwrap_err();
            assert!(!error.to_string().contains("UPSTREAM-SECRET"));
        }
        let current = channel_bot_service::get_bot(&state.db, &bot.id)
            .await
            .unwrap();
        assert_eq!(
            current.status,
            if status == 400 { "failed" } else { "active" }
        );
        let key = state
            .db
            .collection::<UserApiKey>(KEYS)
            .find_one(doc! {"_id": key})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(key.credential_epoch, 1);
    }
}

#[tokio::test]
async fn revoked_or_deleted_connection_fails_bot_without_provider_calls() {
    for deleted in [false, true] {
        let (state, adapter, server, owner, key) = fixture().await;
        let bot = insert_bot(&state, &owner, &key).await;
        let keys = state.db.collection::<bson::Document>(KEYS);
        if deleted {
            keys.delete_one(doc! {"_id": key}).await.unwrap();
        } else {
            keys.update_one(doc! {"_id": key}, doc! {"$set": {"status": "revoked"}})
                .await
                .unwrap();
        }
        assert!(
            channel_credentials::resolve_bot_token(
                &state.db,
                &state.encryption_keys,
                &adapter,
                &bot
            )
            .await
            .is_err()
        );
        let current = channel_bot_service::get_bot(&state.db, &bot.id)
            .await
            .unwrap();
        assert_eq!(current.status, "failed");
        assert!(current.error.unwrap().contains("Reconnect"));
        assert!(server.received_requests().await.unwrap().is_empty());
    }
}

#[tokio::test]
async fn organization_connections_require_matching_owner_and_admin_acl() {
    let (state, _, _, actor, key) = fixture().await;
    let org = uuid::Uuid::new_v4().to_string();
    state
        .db
        .collection(crate::models::user::COLLECTION_NAME)
        .insert_one(test_user(&org, crate::models::user::UserType::Org))
        .await
        .unwrap();
    state.db.collection::<bson::Document>(crate::models::org_membership::COLLECTION_NAME).insert_one(doc! {
        "_id": uuid::Uuid::new_v4().to_string(), "org_user_id": &org, "member_user_id": &actor, "role": "admin", "created_at": bson::DateTime::now(),
    }).await.unwrap();
    assert_eq!(
        crate::handlers::channel_bots::resolve_create_owner(&state, &actor, Some(&org))
            .await
            .unwrap(),
        org
    );
    assert!(
        channel_credentials::connection_token(
            &state.db,
            &state.encryption_keys,
            &org,
            &key,
            "twitter",
            REQUIRED_SCOPES
        )
        .await
        .is_err()
    );
    state
        .db
        .collection::<bson::Document>(KEYS)
        .update_one(doc! {"_id": &key}, doc! {"$set": {"user_id": &org}})
        .await
        .unwrap();
    assert!(
        channel_credentials::connection_token(
            &state.db,
            &state.encryption_keys,
            &org,
            &key,
            "twitter",
            REQUIRED_SCOPES
        )
        .await
        .is_ok()
    );
    assert!(
        channel_credentials::connection_token(
            &state.db,
            &state.encryption_keys,
            &actor,
            &key,
            "twitter",
            REQUIRED_SCOPES
        )
        .await
        .is_err()
    );
    state
        .db
        .collection::<bson::Document>(crate::models::org_membership::COLLECTION_NAME)
        .update_one(
            doc! {"org_user_id": &org},
            doc! {"$set": {"role": "member"}},
        )
        .await
        .unwrap();
    assert!(
        crate::handlers::channel_bots::resolve_create_owner(&state, &actor, Some(&org))
            .await
            .is_err()
    );
}

#[tokio::test]
async fn oauth_start_uses_shared_client_pkce_popup_nonce_and_required_scopes() {
    let (state, adapter, _, owner, _) = fixture().await;
    let started = channel_credentials::start_connection(
        &state.db,
        &state.encryption_keys,
        &state.config.base_url,
        &adapter,
        &owner,
        &owner,
        "Support",
    )
    .await
    .unwrap();
    let url = url::Url::parse(&started.authorization_url).unwrap();
    let params = url
        .query_pairs()
        .collect::<std::collections::HashMap<_, _>>();
    assert_eq!(params["client_id"], "platform-client");
    assert_eq!(params["scope"], REQUIRED_SCOPES.join(" "));
    assert_eq!(params["code_challenge_method"], "S256");
    assert_eq!(
        params["state"],
        format!("1cc_{}", started.attempt_nonce.as_ref().unwrap())
    );
    let key = state
        .db
        .collection::<UserApiKey>(KEYS)
        .find_one(doc! {"_id": &started.connection_id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(key.credential_source.as_deref(), Some("platform"));
    assert_eq!(key.oauth_attempt_nonce, started.attempt_nonce);
    assert_eq!(key.status, "pending_auth");
    assert!(!format!("{started:?}").contains("platform-client"));
}

#[tokio::test]
async fn admin_lists_both_providers_and_updates_only_the_shared_provider_config() {
    use crate::handlers::{admin_platform_credentials as admin, channel_managed};
    let (state, adapter, _, owner, _) = fixture().await;
    super::role_service::seed_system_roles(&state.db)
        .await
        .unwrap();
    let admin_role = super::role_service::get_platform_role_ids(&state.db)
        .await
        .unwrap()
        .admin;
    state
        .db
        .collection::<bson::Document>(crate::models::user::COLLECTION_NAME)
        .update_one(
            doc! {"_id": &owner},
            doc! {"$addToSet": {"role_ids": admin_role}},
        )
        .await
        .unwrap();
    let auth = test_auth_user(&owner);
    let (_, Json(list)) = admin::list(State(state.clone()), auth.clone())
        .await
        .unwrap();
    assert_eq!(
        list.iter().map(|p| p.provider).collect::<Vec<_>>(),
        ["meta", "x"]
    );
    let (_, Json(updated)) = admin::update(
        State(state.clone()),
        auth.clone(),
        Path("x".into()),
        Json(
            serde_json::from_value(
                json!({"fields": {"client_id": "new-client", "client_secret": "new-secret"}}),
            )
            .unwrap(),
        ),
    )
    .await
    .unwrap();
    assert!(updated.available);
    assert!(
        updated
            .fields
            .iter()
            .all(|f| f.configured && f.value.is_none())
    );
    assert!(updated.webhook_verify_token.is_none());
    assert!(
        updated
            .callback_url
            .unwrap()
            .ends_with("/api/v1/providers/callback")
    );
    let provider = state
        .db
        .collection::<ProviderConfig>(PROVIDERS)
        .find_one(doc! {"slug": "twitter"})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        state
            .encryption_keys
            .decrypt(provider.client_id_encrypted.as_ref().unwrap())
            .await
            .unwrap(),
        b"new-client"
    );
    assert_eq!(
        state
            .encryption_keys
            .decrypt(provider.client_secret_encrypted.as_ref().unwrap())
            .await
            .unwrap(),
        b"new-secret"
    );
    assert_eq!(
        state
            .db
            .collection::<bson::Document>(crate::models::platform_credential::COLLECTION_NAME)
            .count_documents(doc! {"provider": "x"})
            .await
            .unwrap(),
        0
    );
    let (_, Json(bootstrap)) =
        channel_managed::bootstrap(State(state.clone()), auth.clone(), Path("x".into()))
            .await
            .unwrap();
    assert!(bootstrap.available);
    assert_eq!(bootstrap.provider_slug, Some("twitter"));
    assert_eq!(bootstrap.required_scopes, REQUIRED_SCOPES);
    admin::delete(State(state.clone()), auth, Path("x".into()))
        .await
        .unwrap();
    assert!(!platform_credential_service::configured(
        platform_credential_service::load(&state.db, &adapter.platform_credentials().unwrap())
            .await
            .unwrap()
            .as_ref(),
        &adapter.platform_credentials().unwrap()
    ));
    assert!(
        state
            .db
            .collection::<ProviderConfig>(PROVIDERS)
            .find_one(doc! {"slug": "twitter"})
            .await
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn reconnect_requires_same_identity_and_fences_stale_failure() {
    let (state, adapter, server, owner, key) = fixture().await;
    let bot = insert_bot(&state, &owner, &key).await;
    identity(&server, "11").await;
    assert!(
        channel_bot_service::reconnect_bot(
            &state.db,
            &state.encryption_keys,
            &state.http_client,
            &adapter,
            &bot,
            &key
        )
        .await
        .is_err()
    );
    server.reset().await;
    identity(&server, "10").await;
    Mock::given(method("GET"))
        .and(path("/2/dm_events"))
        .and(header("authorization", "Bearer live-access-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": [{"id": "200"}]})))
        .expect(0)
        .mount(&server)
        .await;
    channel_bot_service::reconnect_bot(
        &state.db,
        &state.encryption_keys,
        &state.http_client,
        &adapter,
        &bot,
        &key,
    )
    .await
    .unwrap();
    channel_credentials::fail_bot(&state.db, &bot, "stale failure")
        .await
        .unwrap();
    let current = channel_bot_service::get_bot(&state.db, &bot.id)
        .await
        .unwrap();
    assert_eq!(current.status, "active");
    assert_eq!(current.poll_cursor.as_deref(), Some("100"));
    channel_bot_service::delete_bot(
        &state.db,
        &state.http_client,
        &state.encryption_keys,
        &adapter,
        &bot.id,
        &owner,
    )
    .await
    .unwrap();
    assert!(
        state
            .db
            .collection::<UserApiKey>(KEYS)
            .find_one(doc! {"_id": key})
            .await
            .unwrap()
            .is_some()
    );
}

#[path = "channel_x_review_tests.rs"]
mod review;

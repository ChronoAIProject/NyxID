use super::*;
use crate::{
    models::channel_meta::SESSIONS,
    services::{
        channel_adapters::meta::tests::fixture,
        feature_flag_admin_service,
        feature_flag_service::{self as flags, FlagTarget},
    },
};
use serde_json::json;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{header, method, path, query_param},
};

async fn configure(state: &crate::AppState, bot: &ChannelBot) {
    ensure_indexes(&state.db).await.unwrap();
    let descriptor = adapter(&bot.platform)
        .unwrap()
        .platform_credentials()
        .unwrap();
    let fields = [
        ("app_id", "111"),
        ("client_secret", "login-secret"),
        ("webhook_signing_secret", "fixture-signing-secret"),
        ("verify_token", "verify-secret"),
    ]
    .into_iter()
    .map(|(k, v)| (k.into(), Some(Zeroizing::new(v.into()))))
    .collect();
    super::super::platform_credential_service::update(
        &state.db,
        &state.encryption_keys,
        &descriptor,
        &bot.user_id,
        &fields,
        false,
    )
    .await
    .unwrap();
    flags::set_platform_override(
        &state.db,
        &format!("channels:{}", bot.platform),
        &FlagTarget::User(bot.user_id.clone()),
        true,
        &bot.user_id,
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn enrollment_opt_out_serializes_with_an_installation_commit() {
    let db = crate::test_utils::connect_transaction_test_database("meta_enrollment_fence").await;
    let actor = uuid::Uuid::new_v4().to_string();
    db.collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
        .insert_one(crate::test_utils::test_user(
            &actor,
            crate::models::user::UserType::Person,
        ))
        .await
        .unwrap();
    flags::set_platform_override(
        &db,
        "channels:messenger",
        &FlagTarget::User(actor.clone()),
        true,
        &actor,
    )
    .await
    .unwrap();

    // Establish the fence in a committed transaction. Concurrent first-time
    // creation can choose either transaction as the winner at commit.
    super::super::transaction_service::run_db_only(&db, &db, |tx, db| {
        Box::pin(async move { flags::fence_meta_enrollment_in_session(db, "messenger", tx).await })
    })
    .await
    .unwrap();
    let mut installation = db.client().start_session().await.unwrap();
    installation.start_transaction().await.unwrap();
    flags::fence_meta_enrollment_in_session(&db, "messenger", &mut installation)
        .await
        .unwrap();
    assert!(
        flags::personal_flag_enabled(&db, &actor, "channels:messenger")
            .await
            .unwrap()
    );

    let writer_db = db.clone();
    let writer_actor = actor.clone();
    let mut disable = tokio::spawn(async move {
        feature_flag_admin_service::mutate(
            &writer_db,
            "channels:messenger",
            feature_flag_admin_service::Mutation::Override {
                kind: crate::models::feature_flag_override::FlagTargetKind::User,
                target_id: Some(writer_actor.clone()),
                enabled: Some(false),
            },
            &crate::services::audit_service::AuditActor {
                user_id: writer_actor,
                api_key_id: None,
                api_key_name: None,
                ip_address: None,
                user_agent: None,
            },
        )
        .await
    });
    let mut retry_after_release = false;
    match tokio::time::timeout(std::time::Duration::from_millis(100), &mut disable).await {
        Err(_) => {}
        Ok(Ok(Ok(_))) => panic!("enrollment opt-out committed under the installation fence"),
        Ok(Ok(Err(crate::errors::AppError::Conflict(_)))) => {
            // Mongo may abort a waiting transaction with a classified
            // transient conflict instead of keeping it blocked for the whole
            // observation window. Retry only after releasing the fence.
            retry_after_release = true;
        }
        Ok(Ok(Err(error))) => panic!("unexpected opt-out result: {error:?}"),
        Ok(Err(error)) => panic!("opt-out task failed: {error:?}"),
    }
    installation.commit_transaction().await.unwrap();
    if retry_after_release {
        feature_flag_admin_service::mutate(
            &db,
            "channels:messenger",
            feature_flag_admin_service::Mutation::Override {
                kind: crate::models::feature_flag_override::FlagTargetKind::User,
                target_id: Some(actor.clone()),
                enabled: Some(false),
            },
            &crate::services::audit_service::AuditActor {
                user_id: actor.clone(),
                api_key_id: None,
                api_key_name: None,
                ip_address: None,
                user_agent: None,
            },
        )
        .await
        .unwrap();
    } else {
        disable.await.unwrap().unwrap();
    }
    assert!(
        !flags::personal_flag_enabled(&db, &actor, "channels:messenger")
            .await
            .unwrap()
    );
    db.drop().await.unwrap();
}

#[tokio::test]
async fn manual_meta_insert_fences_org_admin_and_enrollment_at_commit() {
    use crate::models::{org_membership::OrgRole, user::UserType};
    let db = crate::test_utils::connect_transaction_test_database("meta_manual_commit_fence").await;
    let org = uuid::Uuid::new_v4().to_string();
    let actor = uuid::Uuid::new_v4().to_string();
    db.collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
        .insert_many([
            crate::test_utils::test_user(&org, UserType::Org),
            crate::test_utils::test_user(&actor, UserType::Person),
        ])
        .await
        .unwrap();
    crate::services::org_service::create_membership(
        &db,
        &org,
        &actor,
        OrgRole::Admin,
        crate::models::org_membership::MemberScopeSource::Inherit,
        None,
    )
    .await
    .unwrap();
    flags::set_platform_override(
        &db,
        "channels:messenger",
        &FlagTarget::User(actor.clone()),
        true,
        &actor,
    )
    .await
    .unwrap();
    let mut tx = db.client().start_session().await.unwrap();
    tx.start_transaction().await.unwrap();
    MetaService::fence_installation_authority(&db, &mut tx, &actor, &org, "messenger", true)
        .await
        .unwrap();
    tx.commit_transaction().await.unwrap();
    db.collection::<bson::Document>(crate::models::org_membership::COLLECTION_NAME)
        .update_one(
            doc! {"org_user_id": &org, "member_user_id": &actor},
            doc! {"$set": {"revoked_at": bson::DateTime::now()}},
        )
        .await
        .unwrap();
    let mut tx = db.client().start_session().await.unwrap();
    tx.start_transaction().await.unwrap();
    assert!(
        MetaService::fence_installation_authority(&db, &mut tx, &actor, &org, "messenger", true,)
            .await
            .is_err()
    );
    tx.abort_transaction().await.unwrap();
    db.drop().await.unwrap();
}

#[tokio::test]
async fn ordinary_manual_org_handler_preserves_actor_in_meta_grant_authority() {
    let (state, adapter, server, fixture_bot) =
        crate::services::channel_adapters::meta::tests::fixture("messenger").await;
    configure(&state, &fixture_bot).await;
    let org = uuid::Uuid::new_v4().to_string();
    db_insert_org_admin(&state.db, &org, &fixture_bot.user_id).await;
    Mock::given(method("GET"))
        .and(path("/v25.0/me"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "999",
            "name": "Manual Page",
            "category": "Business",
            "tasks": ["MESSAGING"]
        })))
        .mount(&server)
        .await;
    let body = crate::handlers::channel_bots::CreateChannelBotRequest {
        platform: "messenger".into(),
        bot_token: "manual-page-token".into(),
        label: "Org manual inbox".into(),
        app_id: Some("111".into()),
        app_secret: Some("manual-secret".into()),
        verification_token: Some("exclusive".into()),
        encrypt_key: None,
        public_key: None,
        phone_number_id: None,
        waba_id: None,
        target_org_id: Some(org.clone()),
    };
    let (status, _) = crate::handlers::channel_bots::create_bot_with_adapter(
        &state,
        crate::test_utils::test_auth_user(&fixture_bot.user_id),
        crate::telemetry::TelemetryContext::default(),
        body,
        &adapter,
        &crate::services::telegram_new_api::TelegramApi::new(&state.http_client),
    )
    .await
    .unwrap();
    assert_eq!(status, axum::http::StatusCode::CREATED);
    let created = state
        .db
        .collection::<ChannelBot>(BOTS)
        .find_one(doc! {"platform":"messenger","platform_bot_id":"999"})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(created.user_id, org);
    assert_eq!(
        created.meta_binding.unwrap().grant_actor_id,
        fixture_bot.user_id
    );
}

async fn db_insert_org_admin(db: &mongodb::Database, org: &str, actor: &str) {
    db.collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
        .insert_one(crate::test_utils::test_user(
            org,
            crate::models::user::UserType::Org,
        ))
        .await
        .unwrap();
    crate::services::org_service::create_membership(
        db,
        org,
        actor,
        crate::models::org_membership::OrgRole::Admin,
        crate::models::org_membership::MemberScopeSource::Inherit,
        None,
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn ordinary_manual_org_handler_rejects_admin_demotion_during_provider_verify() {
    let (state, _, server, fixture_bot) =
        crate::services::channel_adapters::meta::tests::fixture("messenger").await;
    configure(&state, &fixture_bot).await;
    let org = uuid::Uuid::new_v4().to_string();
    db_insert_org_admin(&state.db, &org, &fixture_bot.user_id).await;
    Mock::given(method("GET"))
        .and(path("/v25.0/me"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(std::time::Duration::from_millis(250))
                .set_body_json(json!({
                    "id": "998",
                    "name": "Demotion Page",
                    "category": "Business",
                    "tasks": ["MESSAGING"]
                })),
        )
        .mount(&server)
        .await;
    let mut adapter = MetaAdapter::new(Inbox::Messenger);
    adapter.graph_base = server.uri();
    let request = crate::handlers::channel_bots::CreateChannelBotRequest {
        platform: "messenger".into(),
        bot_token: "demotion-page-token".into(),
        label: "Demotion inbox".into(),
        app_id: Some("111".into()),
        app_secret: Some("manual-secret".into()),
        verification_token: Some("exclusive".into()),
        encrypt_key: None,
        public_key: None,
        phone_number_id: None,
        waba_id: None,
        target_org_id: Some(org.clone()),
    };
    let state_for_request = state.clone();
    let actor = fixture_bot.user_id.clone();
    let actor_for_request = actor.clone();
    let task = tokio::spawn(async move {
        crate::handlers::channel_bots::create_bot_with_adapter(
            &state_for_request,
            crate::test_utils::test_auth_user(&actor_for_request),
            crate::telemetry::TelemetryContext::default(),
            request,
            &adapter,
            &crate::services::telegram_new_api::TelegramApi::new(&state_for_request.http_client),
        )
        .await
    });
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while server.received_requests().await.unwrap().is_empty() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    state
        .db
        .collection::<bson::Document>(crate::models::org_membership::COLLECTION_NAME)
        .update_one(
            doc! {"org_user_id": &org, "member_user_id": &actor},
            doc! {"$set": {"role": "member"}},
        )
        .await
        .unwrap();
    let result = task.await.unwrap();
    assert!(result.is_err());
    assert!(
        state
            .db
            .collection::<ChannelBot>(BOTS)
            .find_one(doc! {"platform":"messenger","platform_bot_id":"998"})
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn ordinary_manual_patch_rejects_admin_demotion_during_provider_verify() {
    use crate::services::channel_bot_service::{self, SecretPatch, UpdateBotParams};

    let (state, adapter, server, fixture_bot) =
        crate::services::channel_adapters::meta::tests::fixture("messenger").await;
    configure(&state, &fixture_bot).await;
    let org = uuid::Uuid::new_v4().to_string();
    db_insert_org_admin(&state.db, &org, &fixture_bot.user_id).await;

    Mock::given(method("GET"))
        .and(path("/v25.0/me"))
        .and(header("authorization", "Bearer manual-page-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "id": "998",
            "name": "Manual Page",
            "category": "Business",
            "tasks": ["MESSAGING"]
        })))
        .expect(1)
        .mount(&server)
        .await;
    let create = crate::handlers::channel_bots::CreateChannelBotRequest {
        platform: "messenger".into(),
        bot_token: "manual-page-token".into(),
        label: "Org manual inbox".into(),
        app_id: Some("111".into()),
        app_secret: Some("manual-secret".into()),
        verification_token: Some("exclusive".into()),
        encrypt_key: None,
        public_key: None,
        phone_number_id: None,
        waba_id: None,
        target_org_id: Some(org.clone()),
    };
    let (status, _) = crate::handlers::channel_bots::create_bot_with_adapter(
        &state,
        crate::test_utils::test_auth_user(&fixture_bot.user_id),
        crate::telemetry::TelemetryContext::default(),
        create,
        &adapter,
        &crate::services::telegram_new_api::TelegramApi::new(&state.http_client),
    )
    .await
    .unwrap();
    assert_eq!(status, axum::http::StatusCode::CREATED);
    server.verify().await;
    let created = state
        .db
        .collection::<ChannelBot>(BOTS)
        .find_one(doc! {"platform":"messenger", "platform_bot_id":"998"})
        .await
        .unwrap()
        .unwrap();
    let before_generation = created.meta_binding.as_ref().unwrap().generation;

    Mock::given(method("GET"))
        .and(path("/v25.0/me"))
        .and(header("authorization", "Bearer replacement-page-token"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(std::time::Duration::from_millis(250))
                .set_body_json(json!({
                    "id": "998",
                    "name": "Replacement Page",
                    "category": "Business",
                    "tasks": ["MESSAGING"]
                })),
        )
        .mount(&server)
        .await;
    let mut patch_adapter = MetaAdapter::new(Inbox::Messenger);
    patch_adapter.graph_base = server.uri();
    let patch = channel_bot_service::update_bot_as_actor(
        &state.db,
        &state.encryption_keys,
        &state.http_client,
        &patch_adapter,
        &created.id,
        channel_bot_service::UpdateBotAuthority {
            actor_id: &fixture_bot.user_id,
            owner_id: &org,
        },
        UpdateBotParams {
            x_events: None,
            bot_token: Some("replacement-page-token"),
            label: None,
            verification_token: None,
            encrypt_key: SecretPatch::Unchanged,
            app_id: None,
            app_secret: None,
        },
    );
    let actor = fixture_bot.user_id.clone();
    let demote = async {
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            while server.received_requests().await.unwrap().len() < 2 {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        state
            .db
            .collection::<bson::Document>(crate::models::org_membership::COLLECTION_NAME)
            .update_one(
                doc! {"org_user_id": &org, "member_user_id": &actor},
                doc! {"$set": {"role": "member"}},
            )
            .await
            .unwrap();
    };
    let (result, ()) = tokio::join!(patch, demote);
    assert!(matches!(result, Err(AppError::Forbidden(_))));
    let live = channel_bot_service::get_bot(&state.db, &created.id)
        .await
        .unwrap();
    assert_eq!(
        channel_bot_service::decrypt_bot_token(&state.encryption_keys, &live)
            .await
            .unwrap()
            .as_str(),
        "manual-page-token"
    );
    assert_eq!(
        live.meta_binding.as_ref().unwrap().generation,
        before_generation
    );
}

#[tokio::test]
async fn manual_token_replacement_repairs_grant_but_preserves_operator_hold() {
    use crate::services::channel_bot_service::{self, SecretPatch, UpdateBotParams};

    let (state, adapter, server, bot) =
        crate::services::channel_adapters::meta::tests::fixture("instagram").await;
    configure(&state, &bot).await;
    super::super::channel_registration::bind_fixture_fields(
        &state.db,
        &state.encryption_keys,
        &bot.id,
        &["bot_token_encrypted"],
    )
    .await;
    state
        .db
        .collection::<ChannelBot>(BOTS)
        .update_one(
            doc! {"_id": &bot.id},
            doc! {"$set": {
                "status": "suspended",
                "webhook_registered": false,
                "error": "Reconnect this inbox: its grant expired or was revoked",
                "diagnostics.failure_reason": "reconnect_required",
                "meta_binding.operator_suspended": true,
                "meta_binding.grant_reconnect_required": true,
                "meta_binding.provider_revoked": true,
                "meta_binding.subscription": "suspended",
            }},
        )
        .await
        .unwrap();
    Mock::given(method("GET"))
        .and(path("/v25.0/me"))
        .and(header("authorization", "Bearer replacement-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "user_id": "222",
            "username": "professional",
            "account_type": "Business"
        })))
        .expect(1)
        .mount(&server)
        .await;
    let mut replacement_adapter = adapter;
    replacement_adapter.graph_base = server.uri();
    let before = channel_bot_service::get_bot(&state.db, &bot.id)
        .await
        .unwrap();
    let before_generation = before.meta_binding.as_ref().unwrap().generation;
    let updated = channel_bot_service::update_bot_as_actor(
        &state.db,
        &state.encryption_keys,
        &state.http_client,
        &replacement_adapter,
        &bot.id,
        channel_bot_service::UpdateBotAuthority {
            actor_id: &bot.user_id,
            owner_id: &bot.user_id,
        },
        UpdateBotParams {
            x_events: None,
            bot_token: Some("replacement-token"),
            label: None,
            verification_token: None,
            encrypt_key: SecretPatch::Unchanged,
            app_id: None,
            app_secret: None,
        },
    )
    .await
    .unwrap();
    server.verify().await;
    assert!(
        !updated
            .secret_context_versions
            .contains_key("bot_token_encrypted")
    );
    assert_eq!(
        channel_bot_service::decrypt_bot_token(&state.encryption_keys, &updated)
            .await
            .unwrap()
            .as_str(),
        "replacement-token"
    );
    let binding = updated.meta_binding.as_ref().unwrap();
    assert!(binding.operator_suspended);
    assert!(!binding.grant_reconnect_required);
    assert!(!binding.provider_revoked);
    assert_eq!(binding.subscription, "suspended");
    assert_eq!(updated.status, "suspended");
    assert!(!updated.webhook_registered);
    assert!(binding.events_since.is_some());
    assert!(updated.error.is_none());
    assert_eq!(
        updated.diagnostics.failure_reason.as_deref(),
        Some("meta_delivery_suspended")
    );
    assert_eq!(binding.generation, before_generation + 1);
}

#[tokio::test]
async fn manual_signing_secret_patch_does_not_repair_failed_grant() {
    use crate::services::channel_bot_service::{self, SecretPatch, UpdateBotParams};

    let (state, adapter, server, bot) =
        crate::services::channel_adapters::meta::tests::fixture("instagram").await;
    configure(&state, &bot).await;
    state
        .db
        .collection::<ChannelBot>(BOTS)
        .update_one(
            doc! {"_id": &bot.id},
            doc! {"$set": {
                "status": "failed",
                "webhook_registered": false,
                "error": "Reconnect this inbox: its grant expired or was revoked",
                "diagnostics.failure_reason": "reconnect_required",
                "meta_binding.grant_reconnect_required": true,
                "meta_binding.provider_revoked": true,
                "meta_binding.subscription": "reconnect_required",
            }},
        )
        .await
        .unwrap();
    state.db.collection::<bson::Document>(BOTS).update_one(
        doc! {"_id": &bot.id}, doc! {"$set": {"app_secret_encrypted": binary(state.encryption_keys.encrypt(b"old-signing-secret").await.unwrap())}},
    ).await.unwrap();
    super::super::channel_registration::bind_fixture_fields(
        &state.db,
        &state.encryption_keys,
        &bot.id,
        &["bot_token_encrypted", "app_secret_encrypted"],
    )
    .await;
    let mut signing_adapter = adapter;
    signing_adapter.graph_base = server.uri();
    let before = channel_bot_service::get_bot(&state.db, &bot.id)
        .await
        .unwrap();
    let before_generation = before.meta_binding.as_ref().unwrap().generation;
    let updated = channel_bot_service::update_bot_as_actor(
        &state.db,
        &state.encryption_keys,
        &state.http_client,
        &signing_adapter,
        &bot.id,
        channel_bot_service::UpdateBotAuthority {
            actor_id: &bot.user_id,
            owner_id: &bot.user_id,
        },
        UpdateBotParams {
            x_events: None,
            bot_token: None,
            label: None,
            verification_token: None,
            encrypt_key: SecretPatch::Unchanged,
            app_id: None,
            app_secret: Some("rotated-signing-secret"),
        },
    )
    .await
    .unwrap();
    assert!(
        !updated
            .secret_context_versions
            .contains_key("app_secret_encrypted")
    );
    assert_eq!(
        updated.secret_context_versions.get("bot_token_encrypted"),
        Some(&3)
    );
    assert_eq!(
        signing_adapter
            .build_verify_secrets(&state.encryption_keys, &updated)
            .await
            .unwrap()
            .get("app_secret"),
        Some("rotated-signing-secret")
    );
    assert_eq!(
        channel_bot_service::decrypt_bot_token(&state.encryption_keys, &updated)
            .await
            .unwrap()
            .as_str(),
        "fixture-token"
    );
    let binding = updated.meta_binding.as_ref().unwrap();
    assert!(binding.grant_reconnect_required);
    assert!(binding.provider_revoked);
    assert_eq!(binding.subscription, "reconnect_required");
    assert_eq!(updated.status, "failed");
    assert_eq!(
        updated.error.as_deref(),
        Some("Reconnect this inbox: its grant expired or was revoked")
    );
    assert_eq!(
        updated.diagnostics.failure_reason.as_deref(),
        Some("reconnect_required")
    );
    assert_eq!(binding.generation, before_generation + 1);
}

fn service<'a>(state: &'a crate::AppState, server: &MockServer) -> MetaService<'a> {
    let mut s = MetaService::from(state);
    s.endpoints = MetaEndpoints {
        facebook_graph: server.uri(),
        instagram_graph: server.uri(),
        instagram_oauth: server.uri(),
    };
    s
}
fn state_value(started: &Started) -> String {
    reqwest::Url::parse(&started.authorization_url)
        .unwrap()
        .query_pairs()
        .find(|(k, _)| k == "state")
        .unwrap()
        .1
        .into_owned()
}
async fn instagram(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/oauth/access_token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"data":[{"access_token":"short-token",
            "user_id":900,
            "permissions":"instagram_business_basic,instagram_business_manage_messages"}]}),
        ))
        .expect(1)
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/access_token"))
        .and(query_param("grant_type", "ig_exchange_token"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"access_token":"long-token","expires_in":5184000})),
        )
        .expect(1)
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v25.0/me"))
        .and(header("authorization", "Bearer long-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"data":[{"user_id":"777","username":"professional","account_type":"Business"}]}),
        ))
        .mount(server)
        .await;
    subscriptions(server, "777").await;
}
async fn subscriptions(server: &MockServer, asset: &str) {
    Mock::given(method("POST"))
        .and(path(format!("/v25.0/{asset}/subscribed_apps")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"success":true})))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path(format!("/v25.0/{asset}/subscribed_apps")))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"data":[{"id":"111","subscribed_fields":["messages","messaging_handovers"]}]}),
        ))
        .mount(server)
        .await;
}

#[tokio::test]
async fn instagram_oauth_full_path_is_actor_bound_single_use_and_selects_only_server_assets() {
    let (state, _, server, bot) = fixture("instagram").await;
    configure(&state, &bot).await;
    instagram(&server).await;
    let s = service(&state, &server);
    let start = s
        .start(
            &bot.user_id,
            &bot.user_id,
            "instagram",
            "Professional inbox",
            None,
        )
        .await
        .unwrap();
    let nonce = state_value(&start);
    assert!(
        s.exchange(
            &uuid::Uuid::new_v4().to_string(),
            "instagram",
            &nonce,
            Some("code")
        )
        .await
        .is_err()
    );
    assert!(
        s.exchange(&bot.user_id, "instagram", &nonce, Some(""))
            .await
            .is_err()
    );
    let pending = state
        .db
        .collection::<OAuthSession>(SESSIONS)
        .find_one(doc! {"_id":&start.session_id})
        .await
        .unwrap()
        .unwrap();
    assert_eq!(pending.status, "pending");
    let assets = s
        .exchange(&bot.user_id, "instagram", &nonce, Some("code"))
        .await
        .unwrap();
    assert_eq!(assets.assets.len(), 1);
    assert_eq!(assets.assets[0].id, "777");
    assert!(!serde_json::to_string(&assets).unwrap().contains("token"));
    assert_eq!(
        s.exchange(&bot.user_id, "instagram", &nonce, Some("replayed-code"))
            .await
            .unwrap()
            .status,
        "assets"
    );
    assert!(
        s.finish(&bot.user_id, &start.session_id, "888", true)
            .await
            .is_err()
    );
    let installed = s
        .finish(&bot.user_id, &start.session_id, "777", true)
        .await
        .unwrap();
    assert_eq!(installed.platform_bot_id, "777");
    assert_eq!(installed.credential_source, "platform");
    assert_eq!(installed.status, "active");
    assert_eq!(
        installed.meta_binding.as_ref().unwrap().grant_actor_id,
        bot.user_id
    );
    let repeat = s
        .finish(&bot.user_id, &start.session_id, "777", true)
        .await
        .unwrap();
    assert_eq!(repeat.id, installed.id);
    assert!(
        s.finish(&bot.user_id, &start.session_id, "999", true)
            .await
            .is_err()
    );
    let saved = state
        .db
        .collection::<OAuthSession>(SESSIONS)
        .find_one(doc! {"_id":&start.session_id})
        .await
        .unwrap()
        .unwrap();
    assert!(saved.assets_encrypted.is_none());
    assert_eq!(saved.bot_id.as_deref(), Some(installed.id.as_str()));
    server.verify().await;
}

#[tokio::test]
async fn converting_a_manual_inbox_to_managed_fences_signing_credential_migration() {
    use crate::models::platform_settings::{COLLECTION_NAME as SETTINGS, PLATFORM_SETTINGS_ID};
    let (state, _, server, bot) = fixture("instagram").await;
    configure(&state, &bot).await;
    instagram(&server).await;
    let bots = state.db.collection::<ChannelBot>(BOTS);
    bots.update_one(
        doc! {"_id": &bot.id},
        doc! {"$set": {"platform_bot_id":"777", "meta_binding.grant_subject_id":""}},
    )
    .await
    .unwrap();
    let s = service(&state, &server);
    let start = s
        .start(
            &bot.user_id,
            &bot.user_id,
            "instagram",
            "Inbox",
            Some(&bot.id),
        )
        .await
        .unwrap();
    s.exchange(
        &bot.user_id,
        "instagram",
        &state_value(&start),
        Some("consent"),
    )
    .await
    .unwrap();
    let settings = state.db.collection::<bson::Document>(SETTINGS);
    let id = doc! {"_id": PLATFORM_SETTINGS_ID};
    let before = settings
        .find_one(id.clone())
        .await
        .unwrap()
        .and_then(|row| row.get_i64("channel_bot_registration_revision").ok())
        .unwrap_or(0);
    let managed = s
        .finish(&bot.user_id, &start.session_id, "777", true)
        .await
        .unwrap();
    assert_eq!(managed.id, bot.id);
    assert_eq!(managed.credential_source, "platform");
    assert_eq!(
        settings
            .find_one(id)
            .await
            .unwrap()
            .unwrap()
            .get_i64("channel_bot_registration_revision"),
        Ok(before + 1)
    );
}

#[tokio::test]
async fn interrupted_installation_claim_resumes_same_identity_and_competing_finishes_do_not_orphan_bots()
 {
    let (state, _, server, bot) = fixture("instagram").await;
    configure(&state, &bot).await;
    instagram(&server).await;
    let s = service(&state, &server);
    let start = s
        .start(
            &bot.user_id,
            &bot.user_id,
            "instagram",
            "Recovery inbox",
            None,
        )
        .await
        .unwrap();
    s.exchange(
        &bot.user_id,
        "instagram",
        &state_value(&start),
        Some("code"),
    )
    .await
    .unwrap();
    let session = state
        .db
        .collection::<OAuthSession>(SESSIONS)
        .find_one(doc! {"_id":&start.session_id})
        .await
        .unwrap()
        .unwrap();
    state
        .db
        .collection::<OAuthSession>(SESSIONS)
        .update_one(
            doc! {"_id":&start.session_id},
            doc! {"$set":{"status":"installing","selected_asset_id":"777"}},
        )
        .await
        .unwrap();
    let (a, b) = tokio::join!(
        s.finish(&bot.user_id, &start.session_id, "777", true),
        s.finish(&bot.user_id, &start.session_id, "777", true)
    );
    assert!(a.is_ok() || b.is_ok());
    assert_eq!(
        state
            .db
            .collection::<ChannelBot>(BOTS)
            .count_documents(doc! {"platform":"instagram","platform_bot_id":"777","is_active":true})
            .await
            .unwrap(),
        1
    );
    let installed = s
        .finish(&bot.user_id, &start.session_id, "777", true)
        .await
        .unwrap();
    assert_eq!(Some(installed.id), session.bot_id);
    assert_eq!(
        s.progress(&bot.user_id, &start.session_id)
            .await
            .unwrap()
            .status,
        "completed"
    );
}

#[tokio::test]
async fn enrollment_off_cannot_start_or_complete_but_preserves_existing_management() {
    let (state, _, server, bot) = fixture("instagram").await;
    configure(&state, &bot).await;
    instagram(&server).await;
    let s = service(&state, &server);
    let start = s
        .start(&bot.user_id, &bot.user_id, "instagram", "Inbox", None)
        .await
        .unwrap();
    s.exchange(
        &bot.user_id,
        "instagram",
        &state_value(&start),
        Some("code"),
    )
    .await
    .unwrap();
    flags::set_platform_override(
        &state.db,
        "channels:instagram",
        &FlagTarget::User(bot.user_id.clone()),
        false,
        &bot.user_id,
    )
    .await
    .unwrap();
    assert!(
        s.finish(&bot.user_id, &start.session_id, "777", true)
            .await
            .is_err()
    );
    assert!(
        s.start(&bot.user_id, &bot.user_id, "instagram", "Inbox", None)
            .await
            .is_err()
    );
    assert_eq!(
        s.progress(&bot.user_id, &start.session_id)
            .await
            .unwrap()
            .status,
        "assets"
    );
    assert_eq!(
        super::super::channel_bot_service::get_bot_for_user(&state.db, &bot.id, &bot.user_id)
            .await
            .unwrap()
            .id,
        bot.id
    );
}

#[tokio::test]
async fn old_cleanup_never_unsubscribes_a_successor_and_decrypt_failure_retains_pending_capability()
{
    let (state, _, server, bot) = fixture("messenger").await;
    let s = service(&state, &server);
    let mut successor = bot.clone();
    successor.id = uuid::Uuid::new_v4().to_string();
    successor.user_id = uuid::Uuid::new_v4().to_string();
    state
        .db
        .collection::<ChannelBot>(BOTS)
        .update_one(
            doc! {"_id":&bot.id},
            doc! {"$set":{"is_active":false,"meta_binding.subscription":"removal_pending"}},
        )
        .await
        .unwrap();
    state
        .db
        .collection::<ChannelBot>(BOTS)
        .insert_one(&successor)
        .await
        .unwrap();
    s.disconnect(&bot).await.unwrap();
    assert!(server.received_requests().await.unwrap().is_empty());
    let current = super::super::channel_bot_service::get_bot(&state.db, &bot.id)
        .await
        .unwrap();
    assert_eq!(current.meta_binding.unwrap().subscription, "removed");
    state
        .db
        .collection::<ChannelBot>(BOTS)
        .delete_one(doc! {"_id":&successor.id})
        .await
        .unwrap();
    let mut corrupt = bot.clone();
    corrupt.id = uuid::Uuid::new_v4().to_string();
    corrupt.bot_token_encrypted = vec![1, 2, 3];
    state
        .db
        .collection::<ChannelBot>(BOTS)
        .insert_one(&corrupt)
        .await
        .unwrap();
    assert!(s.disconnect(&corrupt).await.is_err());
    let current = super::super::channel_bot_service::get_bot(&state.db, &corrupt.id)
        .await
        .unwrap();
    assert!(!current.is_active);
    assert_eq!(
        current.meta_binding.unwrap().subscription,
        "removal_pending"
    );
    assert_eq!(current.bot_token_encrypted, vec![1, 2, 3]);
}

#[tokio::test]
async fn due_work_claim_is_atomic_and_does_not_starve_after_a_hundred_not_due_rows() {
    let (state, _, _, bot) = fixture("messenger").await;
    state.db.collection::<ChannelBot>(BOTS).update_one(doc!{"_id":&bot.id},
    doc!{"$set":{"meta_binding.next_reconcile_at":bson::DateTime::from_chrono(Utc::now()+Duration::days(1))}}).await.unwrap();

    for i in 0..100 {
        let mut other = bot.clone();
        other.id = uuid::Uuid::new_v4().to_string();
        other.platform_bot_id = format!("{}", 1000 + i);
        other.meta_binding.as_mut().unwrap().next_reconcile_at =
            Some(Utc::now() + Duration::days(1));
        state
            .db
            .collection::<ChannelBot>(BOTS)
            .insert_one(&other)
            .await
            .unwrap();
    }
    let mut due = bot.clone();
    due.id = uuid::Uuid::new_v4().to_string();
    due.platform_bot_id = "777".into();
    state
        .db
        .collection::<ChannelBot>(BOTS)
        .insert_one(&due)
        .await
        .unwrap();
    let (a, b) = tokio::join!(claim_due(&state.db), claim_due(&state.db));
    let claims: Vec<_> = [a.unwrap(), b.unwrap()].into_iter().flatten().collect();
    assert_eq!(claims.len(), 1);
    assert_eq!(claims[0].0.id, due.id);
    assert!(claim_due(&state.db).await.unwrap().is_none());
}

#[tokio::test]
async fn facebook_page_picker_keeps_siblings_independent_and_records_login_provenance() {
    let (state, _, server, bot) = fixture("messenger").await;
    configure(&state, &bot).await;
    Mock::given(method("POST"))
        .and(path("/v25.0/oauth/access_token"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"access_token":"login-token","expires_in":5184000})),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v25.0/me"))
        .and(header("authorization", "Bearer login-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id":"900"})))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v25.0/me/permissions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"data":[{"permission":"pages_show_list",
            "status":"granted"},
        {"permission":"pages_manage_metadata",
            "status":"granted"},
        {"permission":"pages_messaging",
            "status":"granted"}]}),
        ))
        .mount(&server)
        .await;

    Mock::given(method("GET"))
        .and(path("/v25.0/me/accounts"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"data":[{"id":"777",
            "name":"First Page",
            "tasks":["MESSAGING"],
            "access_token":"first-page-token"},
        {"id":"888",
            "name":"Second Page",
            "tasks":["MANAGE"],
            "access_token":"second-page-token"},
        {"id":"999",
            "name":"Not eligible",
            "tasks":["ADVERTISE"],
            "access_token":"wrong-task-token"}]})),
        )
        .mount(&server)
        .await;

    for (asset, token) in [("777", "first-page-token"), ("888", "second-page-token")] {
        Mock::given(method("GET"))
            .and(path("/v25.0/debug_token"))
            .and(header("authorization", "Bearer 111|login-secret"))
            .and(query_param("input_token", token))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":{
                "app_id":"111","is_valid":true,"type":"PAGE","profile_id":asset,
                "scopes":["pages_messaging","pages_manage_metadata"]}})))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/v25.0/me"))
            .and(header("authorization", format!("Bearer {token}")))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"id":asset,"name":"Page","category":"Business"})),
            )
            .mount(&server)
            .await;
        subscriptions(&server, asset).await;
    }
    Mock::given(method("DELETE"))
        .and(path("/v25.0/777/subscribed_apps"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"success":true})))
        .expect(1)
        .mount(&server)
        .await;
    let s = service(&state, &server);
    let mut installed = Vec::new();
    for asset in ["777", "888"] {
        let start = s
            .start(&bot.user_id, &bot.user_id, "messenger", "Support", None)
            .await
            .unwrap();
        let result = s
            .exchange(
                &bot.user_id,
                "messenger",
                &state_value(&start),
                Some("code"),
            )
            .await
            .unwrap();
        assert_eq!(result.assets.len(), 2);
        installed.push(
            s.finish(&bot.user_id, &start.session_id, asset, true)
                .await
                .unwrap(),
        );
    }
    assert_eq!(
        installed[0].meta_binding.as_ref().unwrap().grant_subject_id,
        "900"
    );
    assert!(
        installed[0]
            .meta_binding
            .as_ref()
            .unwrap()
            .expires_at
            .is_none()
    );
    s.disconnect(&installed[0]).await.unwrap();
    let sibling = super::super::channel_bot_service::get_bot(&state.db, &installed[1].id)
        .await
        .unwrap();
    assert!(sibling.is_active);
    assert_eq!(
        sibling.meta_binding.as_ref().unwrap().subscription,
        "active"
    );
    server.verify().await;
}

fn provider_request(subject: serde_json::Value, issued: i64) -> String {
    use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
    use hmac::{Hmac, Mac};
    let payload = URL_SAFE_NO_PAD.encode(
        serde_json::to_vec(
            &json!({"algorithm":"HMAC-SHA256","user_id":subject,"issued_at":issued}),
        )
        .unwrap(),
    );
    let mut mac = Hmac::<Sha256>::new_from_slice(b"login-secret").unwrap();
    mac.update(payload.as_bytes());
    format!(
        "{}.{}",
        URL_SAFE_NO_PAD.encode(mac.finalize().into_bytes()),
        payload
    )
}

#[tokio::test]
async fn transferred_inbox_account_erasure_preserves_former_history_until_independent_signed_deletion()
 {
    use crate::services::{
        channel_routing_service as routes, key_service, ownership_transfer_service as transfer,
    };
    for callback_first in [true, false] {
        let (state, _, server, bot) = fixture("instagram").await;
        configure(&state, &bot).await;
        state
            .db
            .collection::<bson::Document>(BOTS)
            .update_one(
                doc! {"_id":&bot.id},
                doc! {"$set":{"credential_source":"platform"}},
            )
            .await
            .unwrap();
        let descriptor = adapter("instagram")
            .unwrap()
            .platform_credentials()
            .unwrap();
        let rotated = [(
            "client_secret".into(),
            Some(Zeroizing::new("replacement-login-secret".into())),
        )]
        .into_iter()
        .collect();
        super::super::role_service::seed_system_roles(&state.db)
            .await
            .unwrap();
        let roles = super::super::role_service::get_platform_role_ids(&state.db)
            .await
            .unwrap();
        state
            .db
            .collection::<bson::Document>(crate::models::user::COLLECTION_NAME)
            .update_one(
                doc! {"_id":&bot.user_id},
                doc! {"$set":{"role_ids":[roles.admin]}},
            )
            .await
            .unwrap();
        let recipient = uuid::Uuid::new_v4().to_string();
        state
            .db
            .collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
            .insert_one(crate::test_utils::test_user(
                &recipient,
                crate::models::user::UserType::Person,
            ))
            .await
            .unwrap();
        let key = key_service::create_api_key(
            &state.db,
            &bot.user_id,
            "Historical route",
            "read proxy",
            crate::services::api_key_validation::NewKeyExpiry::Default,
            None,
            Some(&[]),
            Some(&[]),
            Some(false),
            Some(false),
            Some(false),
            None,
            None,
            Some("generic"),
            Some("https://callbacks.example.test/previous"),
        )
        .await
        .unwrap();
        let route = routes::create_conversation(
            &state.db,
            &bot.user_id,
            Some(&bot.id),
            "instagram",
            "*",
            "private",
            None,
            &key.id,
            true,
            false,
        )
        .await
        .unwrap();
        let source = uuid::Uuid::new_v4().to_string();
        let messages = state
            .db
            .collection::<bson::Document>(crate::models::channel_message::COLLECTION_NAME);
        messages
            .insert_one(doc! {"_id":&source,"user_id":&bot.user_id,"channel_bot_id":&bot.id})
            .await
            .unwrap();
        let former = super::super::channel_text_delivery::begin(
            &state.db,
            "legacy-noncanonical-former",
            &bot.user_id,
            &source,
            "former-provider-receipt",
            1,
        )
        .await
        .unwrap();
        let mut sibling = bot.clone();
        sibling.id = uuid::Uuid::new_v4().to_string();
        sibling.platform_bot_id = "444".into();
        sibling.app_id = Some("112".into());
        sibling.meta_binding.as_mut().unwrap().app_id = "112".into();
        sibling.meta_binding.as_mut().unwrap().grant_id = uuid::Uuid::new_v4().to_string();
        sibling.meta_binding.as_mut().unwrap().grant_subject_id = "901".into();
        state
            .db
            .collection::<ChannelBot>(BOTS)
            .insert_one(&sibling)
            .await
            .unwrap();
        messages
            .insert_one(doc! {"_id":"sibling-source","user_id":&bot.user_id,
            "channel_bot_id":&sibling.id})
            .await
            .unwrap();
        let sibling_plan = super::super::channel_text_delivery::begin(
            &state.db,
            "retained-sibling",
            &sibling.user_id,
            "sibling-source",
            "sibling-receipt",
            1,
        )
        .await
        .unwrap();
        let s = service(&state, &server);
        let signed = provider_request(json!(900), Utc::now().timestamp());
        // Deauthorization has no authority to erase any former-owner history.
        accept_provider_request(&s, "instagram", &signed, false)
            .await
            .unwrap();
        let preview = transfer::preview(
            &state.db,
            &bot.user_id,
            transfer::ResourceKind::ChannelBot,
            &bot.id,
            &recipient,
            5,
        )
        .await
        .unwrap();
        transfer::transfer_with_keys(
            &state.db,
            Some(&state.encryption_keys),
            transfer::TransferCommand {
                actor: &bot.user_id,
                api_key_id: None,
                first_party_human: true,
                kind: transfer::ResourceKind::ChannelBot,
                resource_id: &bot.id,
                destination: &recipient,
                request_id: &uuid::Uuid::new_v4().to_string(),
                expected_version: &preview.version,
                capacity: 5,
            },
        )
        .await
        .unwrap();
        // Missing attribution must not block deletion of this journal's own account.
        let personal_key = format!("saved-result:{}:{}", uuid::Uuid::new_v4(), "a".repeat(64));
        let own = super::super::channel_text_delivery::begin(
            &state.db,
            &personal_key,
            &recipient,
            "expired-personal-source",
            "personal-receipt",
            1,
        )
        .await
        .unwrap();
        let mut accepted = None;
        if callback_first {
            accepted = Some(
                accept_provider_request(&s, "instagram", &signed, true)
                    .await
                    .unwrap(),
            );
        }
        super::super::durable_wake_tests::resume_after(
            "meta_account_before_retirement",
            async {
                erase_personal_owner(&state, &recipient).await;
            },
            async {
                assert!(matches!(
                    super::super::platform_credential_service::update(
                        &state.db,
                        &state.encryption_keys,
                        &descriptor,
                        &bot.user_id,
                        &rotated,
                        false
                    )
                    .await,
                    Err(AppError::Conflict(_))
                ));
            },
        )
        .await;
        assert!(
            messages
                .find_one(doc! {"_id":&source,"user_id":&bot.user_id})
                .await
                .unwrap()
                .is_some()
        );
        let retained_route = state
            .db
            .collection::<bson::Document>(crate::models::channel_conversation::COLLECTION_NAME)
            .find_one(doc! {"_id":&route.id,"user_id":&bot.user_id,"retired_by_transfer":true})
            .await
            .unwrap();
        assert!(retained_route.is_some());
        let plans = state
            .db
            .collection::<bson::Document>(crate::models::channel_text_delivery::COLLECTION_NAME);
        assert!(
            plans
                .find_one(doc! {"_id":&own.id})
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            plans
                .find_one(doc! {"_id":&former.id,"channel_bot_id":&bot.id})
                .await
                .unwrap()
                .is_some()
        );
        assert!(
            plans
                .find_one(doc! {"_id":&sibling_plan.id})
                .await
                .unwrap()
                .is_some()
        );
        assert!(
            state
                .db
                .collection::<bson::Document>("channel_meta_erasure_fences")
                .find_one(doc! {"_id":&bot.id,"local_data_erased_at":{"$exists":true}})
                .await
                .unwrap()
                .is_none()
        );
        // Exact sibling proof must survive its own source's TTL too.
        assert!(
            plans
                .find_one(doc! {"_id":&sibling_plan.id,"channel_bot_id":&sibling.id})
                .await
                .unwrap()
                .is_some()
        );
        messages
            .delete_one(doc! {"_id":"sibling-source"})
            .await
            .unwrap();
        // Exact attribution now lives on the original receipt and survives TTL.
        messages.delete_one(doc! {"_id":&source}).await.unwrap();
        if !callback_first {
            provider::reconcile_with_service(&state, &s).await.unwrap();
            assert!(
                plans
                    .find_one(doc! {"_id":&former.id})
                    .await
                    .unwrap()
                    .is_some()
            );
            // The revocation is fully processed. The first deletion has not
            // arrived, but its retained verifier still cannot be rotated.
            assert!(matches!(
                super::super::platform_credential_service::update(
                    &state.db,
                    &state.encryption_keys,
                    &descriptor,
                    &bot.user_id,
                    &rotated,
                    false
                )
                .await,
                Err(AppError::Conflict(_))
            ));
            accepted = Some(
                accept_provider_request(&s, "instagram", &signed, true)
                    .await
                    .unwrap(),
            );
        }
        let accepted = accepted.unwrap();
        let code = accepted["confirmation_code"].as_str().unwrap();
        provider::reconcile_with_service(&state, &s).await.unwrap();
        assert_eq!(
            request_status(&state.db, "instagram", code).await.unwrap()["status"],
            "completed"
        );
        assert!(
            plans
                .find_one(doc! {"_id":&former.id})
                .await
                .unwrap()
                .is_none()
        );
        assert!(
            plans
                .find_one(doc! {"_id":&sibling_plan.id})
                .await
                .unwrap()
                .is_some()
        );
        assert!(
            state
                .db
                .collection::<ChannelBot>(BOTS)
                .find_one(doc! {"_id":&sibling.id})
                .await
                .unwrap()
                .is_some()
        );
        let replay = accept_provider_request(&s, "instagram", &signed, true)
            .await
            .unwrap();
        assert_eq!(replay["confirmation_code"], accepted["confirmation_code"]);
        state.db.collection::<bson::Document>(provider::REQUESTS).update_one(doc! {"_id":code},doc! {"$set":{
            "status":"pending","next_attempt_at":bson::DateTime::now(),"targets":[{"bot_id":&bot.id,"grant_id":&bot.meta_binding.as_ref().unwrap().grant_id}]}}).await.unwrap();
        provider::reconcile_with_service(&state, &s).await.unwrap();
        assert_eq!(
            request_status(&state.db, "instagram", code).await.unwrap()["status"],
            "completed"
        );
        assert!(server.received_requests().await.unwrap().is_empty());
        super::super::platform_credential_service::update(
            &state.db,
            &state.encryption_keys,
            &descriptor,
            &bot.user_id,
            &rotated,
            false,
        )
        .await
        .unwrap();
    }
}

async fn erase_personal_owner(state: &crate::AppState, owner: &str) {
    use crate::services::personal_erasure_service as personal;
    let mut accepted_personal = false;
    for _ in 0..64 {
        match personal::accept(&state.db, owner, owner, None).await {
            Ok(_) => {
                accepted_personal = true;
                break;
            }
            Err(AppError::ErasurePreparing) => {
                personal::prepare_one_for_test(&state.db, &state.encryption_keys)
                    .await
                    .unwrap();
            }
            Err(error) => panic!("account acceptance failed: {error:?}"),
        }
    }
    assert!(accepted_personal);
    // Fully run ordinary personal erasure before processing any provider job.
    for _ in 0..640 {
        if state
            .db
            .collection::<bson::Document>(crate::models::user::COLLECTION_NAME)
            .find_one(doc! {"_id":owner})
            .await
            .unwrap()
            .is_none()
        {
            break;
        }
        state
            .db
            .collection::<bson::Document>(crate::models::personal_erasure::COLLECTION_NAME)
            .update_many(doc! {}, doc! {"$set":{"retry_at":bson::DateTime::now()}})
            .await
            .unwrap();
        personal::process_one(state).await.unwrap();
    }
    assert!(
        state
            .db
            .collection::<bson::Document>(crate::models::user::COLLECTION_NAME)
            .find_one(doc! {"_id":owner})
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn manual_account_erasure_never_invents_an_empty_managed_subject_or_blocks_app_rotation() {
    let (state, _, server, mut bot) = fixture("instagram").await;
    configure(&state, &bot).await;
    bot.meta_binding.as_mut().unwrap().grant_subject_id.clear();
    bot.credential_source = "user".into();
    state
        .db
        .collection::<ChannelBot>(BOTS)
        .replace_one(doc! {"_id":&bot.id}, &bot)
        .await
        .unwrap();
    Mock::given(method("DELETE"))
        .and(path("/v25.0/222/subscribed_apps"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"success":true})))
        .expect(1)
        .mount(&server)
        .await;
    service(&state, &server).disconnect(&bot).await.unwrap();
    let unknown = format!("saved-result:{}:{}", uuid::Uuid::new_v4(), "a".repeat(64));
    let receipt = super::super::channel_text_delivery::begin(
        &state.db,
        &unknown,
        &bot.user_id,
        "expired-manual-source",
        "personal-owned-receipt",
        1,
    )
    .await
    .unwrap();
    erase_personal_owner(&state, &bot.user_id).await;
    assert_eq!(
        state
            .db
            .collection::<bson::Document>("channel_meta_grant_authority")
            .count_documents(doc! {"platform":"instagram"})
            .await
            .unwrap(),
        0
    );
    assert!(
        state
            .db
            .collection::<bson::Document>("channel_meta_erasure_fences")
            .find_one(doc! {"_id":&bot.id,"retired_grant":{"$exists":true}})
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        state
            .db
            .collection::<bson::Document>(crate::models::channel_text_delivery::COLLECTION_NAME)
            .find_one(doc! {"_id":&receipt.id})
            .await
            .unwrap()
            .is_none()
    );
    let descriptor = adapter("instagram")
        .unwrap()
        .platform_credentials()
        .unwrap();
    let fields = [(
        "client_secret".into(),
        Some(Zeroizing::new("replacement-login-secret".into())),
    )]
    .into_iter()
    .collect();
    super::super::platform_credential_service::update(
        &state.db,
        &state.encryption_keys,
        &descriptor,
        "operator",
        &fields,
        false,
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn signed_deletion_finishes_after_account_erasure_purges_the_inbox_first_and_replays() {
    let (state, _, server, bot) = fixture("instagram").await;
    configure(&state, &bot).await;
    let s = service(&state, &server);
    let signed = provider_request(json!(900), Utc::now().timestamp());
    let accepted = accept_provider_request(&s, "instagram", &signed, true)
        .await
        .unwrap();
    let code = accepted["confirmation_code"].as_str().unwrap();
    // Provider revocation independently makes remote unsubscription unnecessary.
    // The deletion target is still pending: its worker has never run.
    accept_provider_request(&s, "instagram", &signed, false)
        .await
        .unwrap();
    assert_eq!(
        request_status(&state.db, "instagram", code).await.unwrap()["status"],
        "pending"
    );
    state
        .db
        .collection::<bson::Document>(crate::models::channel_message::COLLECTION_NAME)
        .insert_one(
            doc! {"_id":"account-first-source", "user_id":&bot.user_id, "channel_bot_id":&bot.id},
        )
        .await
        .unwrap();
    let journal = super::super::channel_text_delivery::begin(
        &state.db,
        "legacy-result",
        &bot.user_id,
        "account-first-source",
        "retained-provider-message",
        1,
    )
    .await
    .unwrap();
    erase_personal_owner(&state, &bot.user_id).await;
    assert!(
        state
            .db
            .collection::<ChannelBot>(BOTS)
            .find_one(doc! {"_id":&bot.id})
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        state
            .db
            .collection::<bson::Document>("channel_meta_erasure_fences")
            .find_one(doc! {"_id":&bot.id,"retired_grant.grant_id":&bot.meta_binding.as_ref().unwrap().grant_id,"local_data_erased_at":{"$exists":false}})
            .await
            .unwrap()
            .is_some()
    );
    assert!(
        state
            .db
            .collection::<bson::Document>(crate::models::channel_text_delivery::COLLECTION_NAME)
            .find_one(doc! {"_id":&journal.id})
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        request_status(&state.db, "instagram", code).await.unwrap()["status"],
        "pending"
    );
    provider::reconcile_with_service(&state, &s).await.unwrap();
    assert_eq!(
        request_status(&state.db, "instagram", code).await.unwrap()["status"],
        "completed"
    );
    // Replaying the durable target after bot and account removal uses only the
    // scoped completion proof, with no retained provider identity required.
    state.db.collection::<bson::Document>(provider::REQUESTS).update_one(doc! {"_id":code},
        doc! {"$set":{"status":"pending","next_attempt_at":bson::DateTime::now(),
            "targets":[{"bot_id":&bot.id,"grant_id":&bot.meta_binding.as_ref().unwrap().grant_id}]}})
        .await.unwrap();
    provider::reconcile_with_service(&state, &s).await.unwrap();
    assert_eq!(
        request_status(&state.db, "instagram", code).await.unwrap()["status"],
        "completed"
    );
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn erasure_then_deauthorization_preserves_the_erasure_and_scrubs_all_provider_identity() {
    let (state, _, server, bot) = fixture("instagram").await;
    configure(&state, &bot).await;
    for (collection, field) in [
        (
            crate::models::channel_message::COLLECTION_NAME,
            "channel_bot_id",
        ),
        (
            crate::models::channel_reply_delivery::COLLECTION_NAME,
            "channel_bot_id",
        ),
        (crate::models::channel_delivery::COLLECTION_NAME, "bot_id"),
    ] {
        state
            .db
            .collection::<bson::Document>(collection)
            .insert_one(
                doc! {"_id":uuid::Uuid::new_v4().to_string(),field:&bot.id,"provider_id":"333"},
            )
            .await
            .unwrap();
    }
    let s = service(&state, &server);
    let request = provider_request(json!(900), Utc::now().timestamp());
    let erased = accept_provider_request(&s, "instagram", &request, true)
        .await
        .unwrap();
    let after_erase = super::super::channel_bot_service::get_bot(&state.db, &bot.id)
        .await
        .unwrap();
    assert!(after_erase.meta_binding.as_ref().unwrap().erasure_requested);
    accept_provider_request(&s, "instagram", &request, false)
        .await
        .unwrap();
    let after_deauth = super::super::channel_bot_service::get_bot(&state.db, &bot.id)
        .await
        .unwrap();
    assert_eq!(
        after_erase.meta_binding.as_ref().unwrap().generation,
        after_deauth.meta_binding.as_ref().unwrap().generation
    );
    provider::reconcile_with_service(&state, &s).await.unwrap();
    assert_eq!(
        request_status(
            &state.db,
            "instagram",
            erased["confirmation_code"].as_str().unwrap()
        )
        .await
        .unwrap()["status"],
        "completed"
    );
    assert!(
        state
            .db
            .collection::<ChannelBot>(BOTS)
            .find_one(doc! {"_id":&bot.id})
            .await
            .unwrap()
            .is_none()
    );
    for collection in [
        crate::models::channel_message::COLLECTION_NAME,
        crate::models::channel_reply_delivery::COLLECTION_NAME,
        crate::models::channel_delivery::COLLECTION_NAME,
    ] {
        assert_eq!(
            state
                .db
                .collection::<bson::Document>(collection)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
    }
    assert!(server.received_requests().await.unwrap().is_empty());
    // A crash/replay after the retired row has been deleted remains idempotent.
    state
        .db
        .collection::<bson::Document>(provider::REQUESTS)
        .update_one(
            doc! {"_id":erased["confirmation_code"].as_str().unwrap()},
            doc! {"$set":{"status":"pending",
            "next_attempt_at":bson::DateTime::now(),
            "targets":[{"bot_id":&bot.id,
            "grant_id":&bot.meta_binding.as_ref().unwrap().grant_id,
            "generation":2_i64}]}},
        )
        .await
        .unwrap();

    provider::reconcile_with_service(&state, &s).await.unwrap();
    assert_eq!(
        request_status(
            &state.db,
            "instagram",
            erased["confirmation_code"].as_str().unwrap()
        )
        .await
        .unwrap()["status"],
        "completed"
    );
}

#[tokio::test]
async fn deauthorization_then_deletion_works_after_the_token_was_retired_in_the_same_second() {
    let (state, _, server, bot) = fixture("instagram").await;
    configure(&state, &bot).await;
    super::super::channel_registration::bind_fixture_fields(
        &state.db,
        &state.encryption_keys,
        &bot.id,
        &["bot_token_encrypted"],
    )
    .await;
    let second = Utc::now().timestamp();
    state.db.collection::<ChannelBot>(BOTS).update_one(doc!{"_id":&bot.id},
    doc!{"$set":{"meta_binding.authorized_at":bson::DateTime::from_millis(second*1000+999)}}).await.unwrap();

    let s = service(&state, &server);
    let request = provider_request(json!("900"), second);
    accept_provider_request(&s, "instagram", &request, false)
        .await
        .unwrap();
    provider::reconcile_with_service(&state, &s).await.unwrap();
    let retired = super::super::channel_bot_service::get_bot(&state.db, &bot.id)
        .await
        .unwrap();
    assert!(retired.is_active);
    assert_eq!(retired.status, "failed");
    assert!(retired.bot_token_encrypted.is_empty());
    assert!(
        !retired
            .secret_context_versions
            .contains_key("bot_token_encrypted")
    );
    let erased = accept_provider_request(&s, "instagram", &request, true)
        .await
        .unwrap();
    provider::reconcile_with_service(&state, &s).await.unwrap();
    assert!(
        state
            .db
            .collection::<ChannelBot>(BOTS)
            .find_one(doc! {"_id":&bot.id})
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(
        request_status(
            &state.db,
            "instagram",
            erased["confirmation_code"].as_str().unwrap()
        )
        .await
        .unwrap()["status"],
        "completed"
    );
}

#[tokio::test]
async fn prepared_meta_credentials_transfer_before_and_after_signed_deauthorization() {
    use crate::services::{
        channel_credential_continuity::{self as continuity, TOKEN},
        ownership_transfer_service::{ResourceKind, TransferCommand, preview, transfer_with_keys},
    };
    for revoked in [false, true] {
        let (state, _, server, bot) = fixture("instagram").await;
        configure(&state, &bot).await;
        let mut keys = crate::test_utils::test_encryption_keys();
        keys.set_contextual_record_writes(true);
        let db = &state.db;
        let bots = db.collection::<bson::Document>(BOTS);
        let secret = "webhook_secret_encrypted";
        bots.update_one(
            doc! {"_id": &bot.id},
            doc! {"$set": {
                "credential_source":"platform",
                secret: binary(keys.encrypt(b"webhook-secret").await.unwrap()),
            }},
        )
        .await
        .unwrap();
        let prepared = continuity::rewrap_fixture(db, &keys, &bot, &[TOKEN]).await;
        assert!(continuity::verified(&prepared).unwrap().is_some());
        if revoked {
            let s = service(&state, &server);
            let request = provider_request(json!("900"), Utc::now().timestamp());
            accept_provider_request(&s, "instagram", &request, false)
                .await
                .unwrap();
            provider::reconcile_with_service(&state, &s).await.unwrap();
            let retired = continuity::rewrap_fixture(db, &keys, &bot, &[TOKEN, secret]).await;
            assert!(continuity::has_empty_token(&retired));
            assert!(continuity::verified(&retired).unwrap().is_some());
        }
        let destination = uuid::Uuid::new_v4().to_string();
        db_insert_org_admin(db, &destination, &bot.user_id).await;
        let kind = ResourceKind::ChannelBot;
        let reviewed = preview(db, &bot.user_id, kind, &bot.id, &destination, 5)
            .await
            .unwrap();
        let request = uuid::Uuid::new_v4().to_string();
        transfer_with_keys(
            db,
            Some(&keys),
            TransferCommand {
                actor: &bot.user_id,
                api_key_id: None,
                first_party_human: true,
                kind,
                resource_id: &bot.id,
                destination: &destination,
                request_id: &request,
                expected_version: &reviewed.version,
                capacity: 5,
            },
        )
        .await
        .unwrap();
        let moved = super::super::channel_bot_service::get_bot(db, &bot.id)
            .await
            .unwrap();
        assert_eq!(moved.user_id, destination);
        assert!(continuity::verified(&moved).unwrap().is_some());
        assert_eq!(moved.bot_token_encrypted.is_empty(), revoked);
        assert_eq!(moved.secret_context_versions.contains_key(TOKEN), !revoked);
        assert!(
            moved
                .meta_binding
                .as_ref()
                .unwrap()
                .grant_reconnect_required
        );
        if !revoked {
            assert_eq!(
                super::super::channel_bot_service::decrypt_bot_token(&keys, &moved)
                    .await
                    .unwrap()
                    .as_str(),
                "fixture-token"
            );
        }
    }
}

#[tokio::test]
async fn revocation_invalidates_fetched_candidates_and_fences_an_exchange_without_a_known_subject()
{
    for revoke_before_exchange in [false, true] {
        let (state, _, server, bot) = fixture("instagram").await;
        configure(&state, &bot).await;
        instagram(&server).await;
        let s = service(&state, &server);
        let start = s
            .start(&bot.user_id, &bot.user_id, "instagram", "Support", None)
            .await
            .unwrap();
        if !revoke_before_exchange {
            s.exchange(
                &bot.user_id,
                "instagram",
                &state_value(&start),
                Some("code"),
            )
            .await
            .unwrap();
        }
        accept_provider_request(
            &s,
            "instagram",
            &provider_request(json!(900), Utc::now().timestamp()),
            false,
        )
        .await
        .unwrap();
        if revoke_before_exchange {
            assert!(
                s.exchange(
                    &bot.user_id,
                    "instagram",
                    &state_value(&start),
                    Some("code")
                )
                .await
                .is_err()
            );
        }
        assert!(
            s.finish(&bot.user_id, &start.session_id, "777", true)
                .await
                .is_err()
        );
        let stored = state
            .db
            .collection::<OAuthSession>(SESSIONS)
            .find_one(doc! {"_id":&start.session_id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.status, "revoked");
        assert!(stored.assets_encrypted.is_none());
        assert_eq!(
            state
                .db
                .collection::<ChannelBot>(BOTS)
                .count_documents(doc! {"platform_bot_id":"777"})
                .await
                .unwrap(),
            0
        );
    }
}

#[tokio::test]
async fn incomplete_subscription_recovers_completed_session_and_completes_only_its_pinned_link() {
    let (state, _, server, bot) = fixture("instagram").await;
    configure(&state, &bot).await;
    instagram(&server).await;
    let link = super::super::channel_connect_link_service::create(
        &state.db,
        &state.encryption_keys,
        super::super::channel_connect_link_service::CreateInput {
            owner: bot.user_id.clone(),
            actor: bot.user_id.clone(),
            platform: "instagram".into(),
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
    let s = service(&state, &server);
    let started = s
        .start(&bot.user_id, &bot.user_id, "instagram", "Support", None)
        .await
        .unwrap();
    let claim = super::super::channel_connect_link_service::claim(
        &state.db,
        &bot.user_id,
        &link.token,
        "meta_oauth",
    )
    .await
    .unwrap();
    super::super::channel_connect_link_service::pin_connection(
        &state.db,
        &claim,
        &started.session_id,
    )
    .await
    .unwrap();
    super::super::channel_connect_link_service::release(&state.db, &claim, false)
        .await
        .unwrap();
    state
        .db
        .collection::<OAuthSession>(SESSIONS)
        .update_one(
            doc! {"_id":&started.session_id},
            doc! {"$set":{"connect_link_id":&link.link.id}},
        )
        .await
        .unwrap();
    s.exchange(
        &bot.user_id,
        "instagram",
        &state_value(&started),
        Some("code"),
    )
    .await
    .unwrap();
    Mock::given(method("POST"))
        .and(path("/v25.0/777/subscribed_apps"))
        .respond_with(ResponseTemplate::new(503).set_body_json(json!({"error":{"code":2}})))
        .with_priority(1)
        .mount(&server)
        .await;
    assert!(
        s.finish(&bot.user_id, &started.session_id, "777", true)
            .await
            .is_err()
    );
    let installed = s.progress(&bot.user_id, &started.session_id).await.unwrap();
    assert_eq!(installed.status, "completed");
    let pending =
        super::super::channel_bot_service::get_bot(&state.db, installed.bot_id.as_deref().unwrap())
            .await
            .unwrap();
    set_bot_delivery(&state.db, &pending, &pending.user_id, false)
        .await
        .unwrap();
    let held = s
        .finish(&bot.user_id, &started.session_id, "777", true)
        .await
        .unwrap();
    assert_eq!(held.status, "suspended");
    assert_eq!(
        super::super::channel_connect_link_service::get(&state.db, &link.link.id)
            .await
            .unwrap()
            .status,
        crate::models::channel_connect_link::LinkStatus::Pending
    );
    set_bot_delivery(&state.db, &held, &held.user_id, true)
        .await
        .unwrap();
    server.reset().await;
    Mock::given(method("GET"))
        .and(path("/v25.0/me"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(json!({"data":[{"user_id":"777",
            "username":"professional",
            "account_type":"Media_Creator"}]})),
        )
        .mount(&server)
        .await;

    subscriptions(&server, "777").await;
    let recovered = s
        .finish(&bot.user_id, &started.session_id, "777", true)
        .await
        .unwrap();
    assert_eq!(recovered.status, "active");
    assert_eq!(Some(recovered.id), installed.bot_id);
    let link = super::super::channel_connect_link_service::get(&state.db, &link.link.id)
        .await
        .unwrap();
    assert_eq!(
        link.status,
        crate::models::channel_connect_link::LinkStatus::Completed
    );
}

#[tokio::test]
async fn cancelled_expired_and_replaced_setup_links_cannot_install_a_formerly_pinned_session() {
    let (state, _, server, bot) = fixture("instagram").await;
    configure(&state, &bot).await;
    instagram(&server).await;
    let link = super::super::channel_connect_link_service::create(
        &state.db,
        &state.encryption_keys,
        super::super::channel_connect_link_service::CreateInput {
            owner: bot.user_id.clone(),
            actor: bot.user_id.clone(),
            platform: "instagram".into(),
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
    let s = service(&state, &server);
    let start = s
        .start(&bot.user_id, &bot.user_id, "instagram", "Support", None)
        .await
        .unwrap();
    state
        .db
        .collection::<OAuthSession>(SESSIONS)
        .update_one(
            doc! {"_id":&start.session_id},
            doc! {"$set":{"connect_link_id":&link.link.id}},
        )
        .await
        .unwrap();
    state
        .db
        .collection::<bson::Document>(crate::models::channel_connect_link::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&link.link.id},
            doc! {"$set":{"flow":"meta_oauth","connection_id":&start.session_id}},
        )
        .await
        .unwrap();
    s.exchange(
        &bot.user_id,
        "instagram",
        &state_value(&start),
        Some("code"),
    )
    .await
    .unwrap();
    for (status, pin, expires) in [
        (
            "cancelled",
            start.session_id.clone(),
            Utc::now() + Duration::minutes(15),
        ),
        (
            "pending",
            uuid::Uuid::new_v4().to_string(),
            Utc::now() + Duration::minutes(15),
        ),
        (
            "pending",
            start.session_id.clone(),
            Utc::now() - Duration::hours(1),
        ),
    ] {
        state
            .db
            .collection::<bson::Document>(crate::models::channel_connect_link::COLLECTION_NAME)
            .update_one(
                doc! {"_id":&link.link.id},
                doc! {"$set":{"status":status,
                "connection_id":pin,
                "expires_at":bson::DateTime::from_chrono(expires)}},
            )
            .await
            .unwrap();

        assert!(
            s.finish(&bot.user_id, &start.session_id, "777", true)
                .await
                .is_err()
        );
    }
    assert_eq!(
        state
            .db
            .collection::<ChannelBot>(BOTS)
            .count_documents(doc! {"platform_bot_id":"777"})
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn instagram_refresh_extends_before_expiry_and_never_renews_a_healthy_or_expired_token() {
    let (state, _, server, bot) = fixture("instagram").await;
    let s = service(&state, &server);
    s.refresh(&bot).await.unwrap();
    assert!(server.received_requests().await.unwrap().is_empty());
    state.db.collection::<ChannelBot>(BOTS).update_one(
        doc! {"_id": &bot.id},
        doc! {"$set": {"meta_binding.expires_at": bson::DateTime::from_chrono(Utc::now() + Duration::days(9))}},
    ).await.unwrap();
    let mut rewrap_keys = crate::test_utils::test_encryption_keys();
    rewrap_keys.set_contextual_record_writes(true);
    let due = super::super::channel_bot_service::get_bot(&state.db, &bot.id)
        .await
        .unwrap();
    let before =
        super::super::channel_credential_continuity::field_pin(&due, "bot_token_encrypted")
            .unwrap();
    Mock::given(method("GET"))
        .and(path("/refresh_access_token"))
        .and(query_param("grant_type", "ig_refresh_token"))
        .and(query_param("access_token", "fixture-token"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"access_token":"refreshed-token","expires_in":5184000})),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET")).and(path("/v25.0/me"))
        .and(header("authorization", "Bearer refreshed-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data":[{"user_id":"222","username":"professional","account_type":"Media_Creator"}]})))
        .expect(1).mount(&server).await;
    super::super::durable_wake_tests::resume_after(
        "meta_refresh_provider_confirmed",
        async {
            s.refresh(&due).await.unwrap();
        },
        async {
            let rewrapped = super::super::channel_credential_continuity::rewrap_fixture(
                &state.db,
                &rewrap_keys,
                &due,
                &["bot_token_encrypted"],
            )
            .await;
            assert_eq!(
                super::super::channel_credential_continuity::field_pin(
                    &rewrapped,
                    "bot_token_encrypted"
                )
                .unwrap(),
                before
            );
        },
    )
    .await;
    let fresh = super::super::channel_bot_service::get_bot(&state.db, &bot.id)
        .await
        .unwrap();
    assert_ne!(
        super::super::channel_credential_continuity::field_pin(&fresh, "bot_token_encrypted")
            .unwrap(),
        before
    );
    assert!(
        !fresh
            .secret_context_versions
            .contains_key("bot_token_encrypted")
    );
    assert!(
        fresh.meta_binding.as_ref().unwrap().expires_at.unwrap() > Utc::now() + Duration::days(59)
    );
    assert_eq!(
        fresh.meta_binding.as_ref().unwrap().grant_id,
        bot.meta_binding.as_ref().unwrap().grant_id
    );
    assert_eq!(
        super::super::channel_bot_service::decrypt_bot_token(&state.encryption_keys, &fresh)
            .await
            .unwrap()
            .as_str(),
        "refreshed-token"
    );
    s.refresh(&fresh).await.unwrap();
    server.verify().await;
    server.reset().await;
    state.db.collection::<ChannelBot>(BOTS).update_one(
        doc! {"_id": &bot.id},
        doc! {"$set": {"meta_binding.expires_at": bson::DateTime::from_chrono(Utc::now() - Duration::seconds(1))}},
    ).await.unwrap();
    let expired = super::super::channel_bot_service::get_bot(&state.db, &bot.id)
        .await
        .unwrap();
    s.refresh(&expired).await.unwrap();
    let failed = super::super::channel_bot_service::get_bot(&state.db, &bot.id)
        .await
        .unwrap();
    assert_eq!(
        failed.meta_binding.as_ref().unwrap().subscription,
        "reconnect_required"
    );
    assert!(!failed.webhook_registered);
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn repeated_sweeps_preserve_the_generation_of_a_held_expired_inbox() {
    let (state, _, server, bot) = fixture("instagram").await;
    let service = service(&state, &server);
    set_bot_delivery(&state.db, &bot, &bot.user_id, false)
        .await
        .unwrap();
    let bots = state.db.collection::<ChannelBot>(BOTS);
    bots.update_one(
        doc! {"_id": &bot.id},
        doc! {"$set": {
            "meta_binding.expires_at": bson::DateTime::from_chrono(Utc::now() - Duration::days(1))
        }},
    )
    .await
    .unwrap();
    sweep_service(&state, &service).await.unwrap();
    let first = bots
        .find_one(doc! {"_id": &bot.id})
        .await
        .unwrap()
        .unwrap()
        .meta_binding
        .unwrap();
    assert!(first.operator_suspended && first.grant_reconnect_required);
    bots.update_one(
        doc! {"_id": &bot.id},
        doc! {"$unset": {"meta_binding.next_reconcile_at": ""}},
    )
    .await
    .unwrap();
    sweep_service(&state, &service).await.unwrap();
    let second = bots
        .find_one(doc! {"_id": &bot.id})
        .await
        .unwrap()
        .unwrap()
        .meta_binding
        .unwrap();
    assert_eq!(first.generation, second.generation);
    assert!(second.operator_suspended && second.grant_reconnect_required);
}

#[tokio::test]
async fn refresh_cannot_overwrite_a_grant_suspended_during_provider_io() {
    let (state, _, server, bot) = fixture("instagram").await;
    state.db.collection::<ChannelBot>(BOTS).update_one(
        doc! {"_id": &bot.id},
        doc! {"$set": {"meta_binding.expires_at": bson::DateTime::from_chrono(Utc::now() + Duration::days(2))}},
    ).await.unwrap();
    let due = super::super::channel_bot_service::get_bot(&state.db, &bot.id)
        .await
        .unwrap();
    Mock::given(method("GET"))
        .and(path("/refresh_access_token"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(std::time::Duration::from_millis(250))
                .set_body_json(json!({"access_token":"must-not-commit","expires_in":5184000})),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v25.0/me"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"data":[{"user_id":"222","account_type":"Business"}]})),
        )
        .mount(&server)
        .await;
    let s = service(&state, &server);
    let suspend = async {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while server.received_requests().await.unwrap().is_empty() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        let _ = crate::handlers::channel_meta::bot_delivery(
            axum::extract::State(state.clone()),
            crate::test_utils::test_auth_user(&bot.user_id),
            axum::extract::Path(bot.id.clone()),
            axum::Json(crate::handlers::channel_meta::Delivery { enabled: false }),
        )
        .await
        .unwrap();
    };
    let (result, ()) = tokio::join!(s.refresh(&due), suspend);
    assert!(matches!(result, Err(AppError::Conflict(_))));
    let live = super::super::channel_bot_service::get_bot(&state.db, &bot.id)
        .await
        .unwrap();
    assert_eq!(live.status, "suspended");
    assert_eq!(
        super::super::channel_bot_service::decrypt_bot_token(&state.encryption_keys, &live)
            .await
            .unwrap()
            .as_str(),
        "fixture-token"
    );
}

#[tokio::test]
async fn instagram_flat_exchange_and_identity_preserve_scope_and_subject_validation() {
    let (state, _, server, bot) = fixture("instagram").await;
    configure(&state, &bot).await;
    Mock::given(method("POST"))
        .and(path("/oauth/access_token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"access_token":"short-token",
        "user_id":900,
        "permissions":"instagram_business_basic,instagram_business_manage_messages"}),
        ))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/access_token"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(json!({"access_token":"long-token","expires_in":5184000})),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v25.0/me"))
        .respond_with(ResponseTemplate::new(200).set_body_json(
            json!({"user_id":777,"username":"professional","account_type":"Business"}),
        ))
        .mount(&server)
        .await;
    subscriptions(&server, "777").await;
    let s = service(&state, &server);
    let start = s
        .start(&bot.user_id, &bot.user_id, "instagram", "Support", None)
        .await
        .unwrap();
    s.exchange(
        &bot.user_id,
        "instagram",
        &state_value(&start),
        Some("code"),
    )
    .await
    .unwrap();
    let installed = s
        .finish(&bot.user_id, &start.session_id, "777", true)
        .await
        .unwrap();
    assert_eq!(installed.platform_bot_id, "777");
    assert_eq!(
        installed.meta_binding.as_ref().unwrap().grant_subject_id,
        "900"
    );
}

#[tokio::test]
async fn instagram_token_shapes_do_not_bypass_required_messaging_consent() {
    for envelope in [false, true] {
        let (state, _, server, bot) = fixture("instagram").await;
        configure(&state, &bot).await;
        let record = json!({"access_token":"short-token", "user_id":900, "permissions":"instagram_business_basic"});
        Mock::given(method("POST"))
            .and(path("/oauth/access_token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(if envelope {
                json!({"data":[record]})
            } else {
                record
            }))
            .expect(1)
            .mount(&server)
            .await;
        let s = service(&state, &server);
        let start = s
            .start(&bot.user_id, &bot.user_id, "instagram", "Support", None)
            .await
            .unwrap();
        assert!(
            s.exchange(
                &bot.user_id,
                "instagram",
                &state_value(&start),
                Some("code")
            )
            .await
            .is_err()
        );
        assert_eq!(
            s.progress(&bot.user_id, &start.session_id)
                .await
                .unwrap()
                .status,
            "failed"
        );
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }
}

#[tokio::test]
async fn meta_credential_patch_returns_conflict_when_the_live_generation_changes_during_verification()
 {
    use crate::services::channel_bot_service::{self, SecretPatch, UpdateBotParams};
    let (state, a, server, bot) = fixture("instagram").await;
    state
        .db
        .collection::<ChannelBot>(BOTS)
        .update_one(
            doc! {"_id":&bot.id},
            doc! {"$set":{"ownership_version":0_i64}},
        )
        .await
        .unwrap();
    Mock::given(method("GET"))
        .and(path("/v25.0/me"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(std::time::Duration::from_millis(250))
                .set_body_json(json!({"user_id":"222","account_type":"Business"})),
        )
        .expect(1)
        .mount(&server)
        .await;
    let patch = channel_bot_service::update_bot(
        &state.db,
        &state.encryption_keys,
        &state.http_client,
        &a,
        &bot.id,
        &bot.user_id,
        UpdateBotParams {
            x_events: None,
            bot_token: Some("replacement-token"),
            label: None,
            verification_token: None,
            encrypt_key: SecretPatch::Unchanged,
            app_id: None,
            app_secret: None,
        },
    );
    let replace = async {
        tokio::time::timeout(std::time::Duration::from_secs(5), async {
            while server.received_requests().await.unwrap().is_empty() {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        state
            .db
            .collection::<ChannelBot>(BOTS)
            .update_one(
                doc! {"_id":&bot.id},
                doc! {"$inc":{"meta_binding.generation":1_i64}},
            )
            .await
            .unwrap();
    };
    let (result, ()) = tokio::join!(patch, replace);
    assert!(matches!(result, Err(AppError::Conflict(_))));
    let live = channel_bot_service::get_bot(&state.db, &bot.id)
        .await
        .unwrap();
    assert_eq!(
        channel_bot_service::decrypt_bot_token(&state.encryption_keys, &live)
            .await
            .unwrap()
            .as_str(),
        "fixture-token"
    );
    server.verify().await;
}

#[tokio::test]
async fn meta_delete_propagates_an_asset_lock_failure_without_retiring_live_routes() {
    use crate::services::{channel_bot_service, channel_connection_webhook_service};
    let (state, a, _, bot) = fixture("instagram").await;
    let route = uuid::Uuid::new_v4().to_string();
    state
        .db
        .collection::<bson::Document>(crate::models::channel_conversation::COLLECTION_NAME)
        .insert_one(doc! {"_id":&route,"channel_bot_id":&bot.id,"is_active":true})
        .await
        .unwrap();
    let acquired = tokio::sync::Notify::new();
    let release = tokio::sync::Notify::new();
    let lock_key = format!("meta-asset:instagram:{}", bot.platform_bot_id);
    let hold = channel_connection_webhook_service::serialized(&state.db, &lock_key, async {
        acquired.notify_one();
        release.notified().await;
        Ok(())
    });
    let remove = async {
        acquired.notified().await;
        let result = channel_bot_service::delete_bot(
            &state.db,
            &state.config,
            &state.http_client,
            &state.encryption_keys,
            &a,
            &bot.id,
            &bot.user_id,
        )
        .await;
        release.notify_one();
        result
    };
    let (_, result) = tokio::join!(hold, remove);
    assert!(matches!(result, Err(AppError::Conflict(_))));
    assert!(
        channel_bot_service::get_bot(&state.db, &bot.id)
            .await
            .unwrap()
            .is_active
    );
    assert!(
        state
            .db
            .collection::<bson::Document>(crate::models::channel_conversation::COLLECTION_NAME)
            .find_one(doc! {"_id":&route,"is_active":true})
            .await
            .unwrap()
            .is_some()
    );
}

#[tokio::test]
async fn final_ingress_fence_rejects_changed_generation_asset_owner_and_managed_application() {
    let (state, _, _, bot) = fixture("instagram").await;
    configure(&state, &bot).await;
    let chat = meta::conversation("instagram", &bot.id, &bot.platform_bot_id, "333");
    state
        .db
        .collection::<bson::Document>(crate::models::platform_credential::COLLECTION_NAME)
        .update_one(
            doc! {"provider":"instagram"},
            doc! {"$set":{"fields.app_id":"999"}},
        )
        .await
        .unwrap();
    for change in [
        doc! {"meta_binding.generation":2_i64},
        doc! {"platform_bot_id":"999"},
        doc! {"user_id":uuid::Uuid::new_v4().to_string()},
        doc! {"credential_source":"platform"},
    ] {
        state
            .db
            .collection::<ChannelBot>(BOTS)
            .replace_one(doc! {"_id":&bot.id}, &bot)
            .await
            .unwrap();
        state
            .db
            .collection::<ChannelBot>(BOTS)
            .update_one(doc! {"_id":&bot.id}, doc! {"$set":change})
            .await
            .unwrap();
        assert!(meta::require_ingress(&state.db, &bot, &chat).await.is_err());
    }
}

#[tokio::test]
async fn deletion_without_deauthorization_retries_each_page_and_reaches_later_siblings() {
    let (state, _, server, first) = fixture("messenger").await;
    configure(&state, &first).await;
    let bots = state.db.collection::<ChannelBot>(BOTS);
    let jobs = state.db.collection::<bson::Document>(provider::REQUESTS);
    for page in 1..=8 {
        let mut sibling = first.clone();
        sibling.id = uuid::Uuid::new_v4().to_string();
        sibling.platform_bot_id = format!("300{page}");
        bots.insert_one(sibling).await.unwrap();
    }
    let service = service(&state, &server);
    let signed = provider_request(json!("900"), Utc::now().timestamp());
    let accepted = accept_provider_request(&service, "messenger", &signed, true)
        .await
        .unwrap();
    let job_id = accepted["confirmation_code"].as_str().unwrap();
    let job = jobs.find_one(doc! {"_id": job_id}).await.unwrap().unwrap();
    let targets = job.get_array("targets").unwrap();
    assert_eq!(targets.len(), 9);
    let last_bot_id = targets[8].as_document().unwrap().get_str("bot_id").unwrap();
    let last = bots
        .find_one(doc! {"_id": last_bot_id})
        .await
        .unwrap()
        .unwrap();
    Mock::given(method("DELETE"))
        .and(path(format!(
            "/v25.0/{}/subscribed_apps",
            last.platform_bot_id
        )))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"success":true})))
        .expect(1)
        .mount(&server)
        .await;
    // The first eight subscriptions receive an unconfirmed removal. The ninth
    // Page can finish in the next bounded batch before any failed Page retries.
    provider::reconcile_with_service(&state, &service)
        .await
        .unwrap();
    let pending = jobs.find_one(doc! {"_id": job_id}).await.unwrap().unwrap();
    assert_eq!(pending.get_str("status").unwrap(), "pending");
    assert!(!pending.contains_key("expires_at"));
    let remaining = pending.get_array("targets").unwrap();
    assert_eq!(remaining.len(), 8);
    assert!(remaining.iter().all(|target| {
        target
            .as_document()
            .unwrap()
            .get_datetime("retry_at")
            .is_ok()
    }));
    assert!(
        bots.find_one(doc! {"_id":last_bot_id})
            .await
            .unwrap()
            .is_none()
    );
    assert_eq!(bots.count_documents(doc! {"is_active":false, "meta_binding.provider_revoked":false, "meta_binding.erasure_requested":true}).await.unwrap(), 8);
    assert_eq!(server.received_requests().await.unwrap().len(), 9);

    // Simulate the retry deadline and confirm the remaining remote removals.
    server.verify().await;
    server.reset().await;
    Mock::given(method("DELETE"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"success":true})))
        .expect(8)
        .mount(&server)
        .await;
    jobs.update_one(doc! {"_id":job_id}, doc! {"$set":{"next_attempt_at":bson::DateTime::now(), "targets.$[].retry_at":bson::DateTime::now()}}).await.unwrap();
    provider::reconcile_with_service(&state, &service)
        .await
        .unwrap();
    assert_eq!(
        request_status(&state.db, "messenger", job_id)
            .await
            .unwrap()["status"],
        "completed"
    );
    let done = jobs.find_one(doc! {"_id":job_id}).await.unwrap().unwrap();
    assert!(done.get_datetime("expires_at").is_ok());
    assert!(!done.contains_key("targets"));
    assert_eq!(bots.count_documents(doc! {}).await.unwrap(), 0);
    server.verify().await;
}

#[tokio::test]
async fn meta_login_credentials_remain_available_until_each_apps_data_is_drained() {
    use crate::services::platform_credential_service as credentials;
    let (state, _, server, mut bot) = fixture("messenger").await;
    configure(&state, &bot).await;
    bot.platform = "instagram".into();
    configure(&state, &bot).await;
    let messenger = adapter("messenger")
        .unwrap()
        .platform_credentials()
        .unwrap();
    let instagram = adapter("instagram")
        .unwrap()
        .platform_credentials()
        .unwrap();
    let s = service(&state, &server);
    let started = s
        .start(
            &bot.user_id,
            &bot.user_id,
            "messenger",
            "Customer support",
            None,
        )
        .await
        .unwrap();
    let field = |name: &str, value: Option<&str>| {
        std::collections::BTreeMap::from([(
            name.to_owned(),
            value.map(|v| Zeroizing::new(v.to_owned())),
        )])
    };
    for fields in [
        field("client_secret", None),
        field("client_secret", Some("rotated")),
        field("app_id", None),
        field("app_id", Some("333")),
    ] {
        assert!(matches!(
            credentials::update(
                &state.db,
                &state.encryption_keys,
                &messenger,
                &bot.user_id,
                &fields,
                false
            )
            .await,
            Err(AppError::Conflict(_))
        ));
    }
    assert!(matches!(
        credentials::delete(&state.db, &messenger).await,
        Err(AppError::Conflict(_))
    ));
    for fields in [
        field("client_secret", Some("login-secret")),
        field("app_id", Some("111")),
        field("verify_token", None),
        field("webhook_signing_secret", None),
    ] {
        credentials::update(
            &state.db,
            &state.encryption_keys,
            &messenger,
            &bot.user_id,
            &fields,
            false,
        )
        .await
        .unwrap();
    }
    let request = provider_request(json!("555"), Utc::now().timestamp());
    accept_provider_request(&s, "messenger", &request, true)
        .await
        .unwrap();
    state
        .db
        .collection::<OAuthSession>(SESSIONS)
        .delete_one(doc! {"_id":started.session_id})
        .await
        .unwrap();

    // An open Messenger grant must protect only its own descriptor, even
    // when both products have the same numeric app ID.
    super::super::transaction_service::run_db_only(&state.db, &state.db, |tx, db| {
        Box::pin(async move {
            fence_grant_in_session(db, tx, "messenger", "111", "666", Utc::now()).await
        })
    })
    .await
    .unwrap();
    credentials::delete(&state.db, &instagram).await.unwrap();
    assert!(matches!(
        credentials::delete(&state.db, &messenger).await,
        Err(AppError::Conflict(_))
    ));
    let revoke = provider_request(json!("666"), Utc::now().timestamp());
    accept_provider_request(&s, "messenger", &revoke, false)
        .await
        .unwrap();
    credentials::delete(&state.db, &messenger).await.unwrap();
}

#[tokio::test]
async fn platform_suspension_and_transfer_keep_their_recovery_states_on_resume() {
    let (state, _, _, bot) = fixture("messenger").await;
    let live = || super::super::channel_bot_service::get_bot(&state.db, &bot.id);
    set_platform_delivery(&state.db, "messenger", false)
        .await
        .unwrap();
    let suspended = live().await.unwrap();
    let before = suspended.meta_binding.as_ref().unwrap();
    assert_eq!(before.subscription, "suspended");
    assert!(
        matches!(set_bot_delivery(&state.db, &suspended, &suspended.user_id, true).await,
        Err(AppError::ChannelConversationNotReachable(reason)) if reason == "meta:platform_suspended")
    );
    let current = live().await.unwrap();
    assert_eq!(
        current.meta_binding.as_ref().unwrap().generation,
        before.generation
    );
    set_platform_delivery(&state.db, "messenger", true)
        .await
        .unwrap();
    set_bot_delivery(&state.db, &current, &current.user_id, true)
        .await
        .unwrap();
    assert_eq!(
        live().await.unwrap().meta_binding.unwrap().subscription,
        "pending"
    );

    state.db.collection::<ChannelBot>(BOTS).update_one(doc! {"_id": &bot.id}, doc! {
        "$set": {"meta_binding.subscription": "reconnect_required",
        "meta_binding.exclusive_routing_verified": false, "error": "Reauthorize for the new owner"},
    }).await.unwrap();
    let transferred = live().await.unwrap();
    set_bot_delivery(&state.db, &transferred, &transferred.user_id, false)
        .await
        .unwrap();
    let held = live().await.unwrap();
    assert!(
        matches!(set_bot_delivery(&state.db, &held, &held.user_id, true).await,
        Err(AppError::ChannelConversationNotReachable(reason)) if reason == "meta:reconnect_required")
    );
    let current = live().await.unwrap();
    assert_eq!(
        current.meta_binding.unwrap().subscription,
        "reconnect_required"
    );
    assert_eq!(
        current.error.as_deref(),
        Some("Reauthorize for the new owner")
    );
}

#[tokio::test]
async fn operator_hold_and_deauthorization_preserve_both_repair_states_in_either_order() {
    for suspend_first in [true, false] {
        let (state, _, server, bot) = fixture("instagram").await;
        configure(&state, &bot).await;
        instagram(&server).await;
        let service = service(&state, &server);
        super::super::channel_registration::bind_fixture_fields(
            &state.db,
            &state.encryption_keys,
            &bot.id,
            &["bot_token_encrypted"],
        )
        .await;
        let live = || super::super::channel_bot_service::get_bot(&state.db, &bot.id);
        let granted = Utc::now() - Duration::minutes(2);
        state
            .db
            .collection::<ChannelBot>(BOTS)
            .update_one(
                doc! {"_id": &bot.id},
                doc! {"$set": {
                    "credential_source":"platform", "platform_bot_id":"777",
                    "meta_binding.authorized_at": bson::DateTime::from_chrono(granted),
                }},
            )
            .await
            .unwrap();
        if suspend_first {
            set_platform_delivery(&state.db, "instagram", false)
                .await
                .unwrap();
        }
        let request = provider_request(json!("900"), (granted + Duration::minutes(1)).timestamp());
        accept_provider_request(&service, "instagram", &request, false)
            .await
            .unwrap();
        if !suspend_first {
            set_bot_delivery(&state.db, &live().await.unwrap(), &bot.user_id, false)
                .await
                .unwrap();
        }
        let held = live().await.unwrap();
        let binding = held.meta_binding.as_ref().unwrap();
        assert!(binding.operator_suspended && binding.grant_reconnect_required);
        assert!(meta::require_resumable(&held).is_err());
        // A revoked capability must never reach token decryption or the provider.
        let mut due = held.clone();
        due.meta_binding.as_mut().unwrap().expires_at = Some(Utc::now() + Duration::days(1));
        service.refresh(&due).await.unwrap();

        let started = service
            .start(
                &bot.user_id,
                &bot.user_id,
                "instagram",
                "Support",
                Some(&bot.id),
            )
            .await
            .unwrap();
        service
            .exchange(
                &bot.user_id,
                "instagram",
                &state_value(&started),
                Some("code"),
            )
            .await
            .unwrap();
        let repaired = service
            .finish(&bot.user_id, &started.session_id, "777", true)
            .await
            .unwrap();
        assert!(
            !repaired
                .secret_context_versions
                .contains_key("bot_token_encrypted")
        );
        assert_eq!(super::super::channel_bot_service::decrypt_bot_token(
            &state.encryption_keys, &repaired,
        ).await.unwrap().as_str(), "long-token");
        let binding = repaired.meta_binding.as_ref().unwrap();
        assert!(binding.operator_suspended && !binding.grant_reconnect_required);
        assert_eq!(repaired.status, "suspended");
        assert!(repaired.error.is_none());
        assert_eq!(
            repaired.diagnostics.failure_reason.as_deref(),
            Some("meta_delivery_suspended")
        );
        let chat = meta::conversation("instagram", &bot.id, "777", "333");
        assert!(
            meta::require_ingress(&state.db, &repaired, &chat)
                .await
                .is_err()
        );
        if suspend_first {
            assert!(
                set_bot_delivery(&state.db, &repaired, &repaired.user_id, true)
                    .await
                    .is_err()
            );
            set_platform_delivery(&state.db, "instagram", true)
                .await
                .unwrap();
        }
        set_bot_delivery(&state.db, &repaired, &repaired.user_id, true)
            .await
            .unwrap();
        service.verify(&live().await.unwrap()).await.unwrap();
        meta::require_ingress(&state.db, &live().await.unwrap(), &chat)
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn messenger_verify_records_missing_page_authority_and_subscription_drift() {
    for (scopes, expected) in [
        (json!(["pages_manage_metadata"]), "reconnect_required"),
        (
            json!(["pages_messaging", "pages_manage_metadata"]),
            "pending",
        ),
    ] {
        let (state, _, server, bot) = fixture("messenger").await;
        configure(&state, &bot).await;
        state
            .db
            .collection::<ChannelBot>(BOTS)
            .update_one(
                doc! {"_id": &bot.id},
                doc! {"$set":{"credential_source":"platform"}},
            )
            .await
            .unwrap();
        Mock::given(method("GET"))
            .and(path("/v25.0/debug_token"))
            .and(query_param("input_token", "fixture-token"))
            .and(header("authorization", "Bearer 111|login-secret"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(json!({"data":{"app_id":"111",
                "type":"PAGE","is_valid":true,"profile_id":"222","scopes":scopes}})),
            )
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/v25.0/me"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "id":"222","name":"Support","category":"Business",
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/v25.0/222/subscribed_apps"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data":[{"id":"111","subscribed_fields":["messages"]}]
            })))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/v25.0/222/subscribed_apps"))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;
        let error = service(&state, &server).verify(&bot).await.unwrap_err();
        let current = super::super::channel_bot_service::get_bot(&state.db, &bot.id)
            .await
            .unwrap();
        assert!(
            !current.webhook_registered,
            "expected {expected} after verification: {error:?}"
        );
        assert_eq!(current.meta_binding.unwrap().subscription, expected);
    }
}

#[tokio::test]
async fn ordinary_delivery_handler_fences_org_admin_demotion_for_suspend_and_resume() {
    use crate::services::{channel_bot_service as bots, org_service};
    for enabled in [false, true] {
        let (state, _, _, mut bot) = fixture("instagram").await;
        let actor = bot.user_id.clone();
        // Personal and organization writes share the generic authorization service.
        assert!(
            bots::resolve_bot_owner_for_write(&state.db, &actor, &bot.id)
                .await
                .is_ok()
        );
        let org = uuid::Uuid::new_v4().to_string();
        db_insert_org_admin(&state.db, &org, &actor).await;
        let other_admin = uuid::Uuid::new_v4().to_string();
        state
            .db
            .collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
            .insert_one(crate::test_utils::test_user(
                &other_admin,
                crate::models::user::UserType::Person,
            ))
            .await
            .unwrap();
        org_service::create_membership(
            &state.db,
            &org,
            &other_admin,
            crate::models::org_membership::OrgRole::Admin,
            crate::models::org_membership::MemberScopeSource::Inherit,
            None,
        )
        .await
        .unwrap();
        bot.user_id = org.clone();
        if enabled {
            let binding = bot.meta_binding.as_mut().unwrap();
            binding.operator_suspended = true;
            binding.subscription = "suspended".into();
            bot.status = "suspended".into();
        }
        state
            .db
            .collection::<ChannelBot>(BOTS)
            .replace_one(doc! {"_id": &bot.id}, &bot)
            .await
            .unwrap();
        assert!(
            bots::resolve_bot_owner_for_write(&state.db, &actor, &bot.id)
                .await
                .is_ok()
        );
        assert!(
            crate::handlers::channel_bots::resolve_bot_owner_for_write(&state, &actor, &bot.id)
                .await
                .is_ok()
        );
        let before = state
            .db
            .collection::<bson::Document>(BOTS)
            .find_one(doc! {"_id": &bot.id})
            .await
            .unwrap()
            .unwrap();
        let membership = state
            .db
            .collection::<crate::models::org_membership::OrgMembership>(
                crate::models::org_membership::COLLECTION_NAME,
            )
            .find_one(doc! {"org_user_id": &org, "member_user_id": &actor})
            .await
            .unwrap()
            .unwrap();
        super::super::durable_wake_tests::resume_after(
            "meta_delivery_before_commit",
            async {
                let result = crate::handlers::channel_meta::bot_delivery(
                    axum::extract::State(state.clone()),
                    crate::test_utils::test_auth_user(&actor),
                    axum::extract::Path(bot.id.clone()),
                    axum::Json(crate::handlers::channel_meta::Delivery { enabled }),
                )
                .await;
                assert!(matches!(result, Err(AppError::Forbidden(_))));
            },
            async {
                org_service::update_membership(
                    &state.db,
                    &membership.id,
                    Some(crate::models::org_membership::OrgRole::Member),
                    None,
                    None,
                )
                .await
                .unwrap();
            },
        )
        .await;
        let after = state
            .db
            .collection::<bson::Document>(BOTS)
            .find_one(doc! {"_id": &bot.id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            after.get_document("meta_binding"),
            before.get_document("meta_binding")
        );
        assert_eq!(after.get_str("status"), before.get_str("status"));
        assert!(matches!(
            bots::resolve_bot_owner_for_write(&state.db, &actor, &bot.id).await,
            Err(AppError::OrgRoleInsufficient(_))
        ));
        assert!(matches!(
            crate::handlers::channel_bots::resolve_bot_owner_for_write(&state, &actor, &bot.id)
                .await,
            Err(AppError::OrgRoleInsufficient(_))
        ));
        let outsider = uuid::Uuid::new_v4().to_string();
        assert!(
            bots::resolve_bot_owner_for_write(&state.db, &other_admin, &bot.id)
                .await
                .is_ok()
        );
        assert!(matches!(
            bots::resolve_bot_owner_for_write(&state.db, &outsider, &bot.id).await,
            Err(AppError::ChannelBotNotFound(_))
        ));
    }
}

#[tokio::test]
async fn ownerless_scoped_metadata_erases_only_without_an_unattributed_saved_reply_and_never_acknowledges_a_blocked_request()
 {
    use crate::services::{channel_bot_service, channel_text_delivery as journal};
    for has_parentless_receipt in [false, true] {
        let (state, _, server, bot) = fixture("instagram").await;
        configure(&state, &bot).await;
        let source = uuid::Uuid::new_v4().to_string();
        state
            .db
            .collection::<bson::Document>(crate::models::channel_message::COLLECTION_NAME)
            .insert_one(doc! {"_id": &source, "channel_bot_id": &bot.id,
            "platform_message_id": "legacy-scoped-provider-id"})
            .await
            .unwrap();
        let logical_key = format!("saved-result:{}:{}", uuid::Uuid::new_v4(), "a".repeat(64));
        if has_parentless_receipt {
            let plan = journal::begin(
                &state.db,
                &logical_key,
                &bot.user_id,
                &source,
                "legacy-hash",
                1,
            )
            .await
            .unwrap();
            state.db.collection::<bson::Document>(crate::models::channel_text_delivery::COLLECTION_NAME)
                .update_one(doc! {"_id": &plan.id}, doc! {"$set": {
                    "parts.0.platform_message_id": "legacy-saved-provider-id", "parts.0.state": "sent",
                }}).await.unwrap();
        }
        let service = service(&state, &server);
        let signed = provider_request(json!(900), Utc::now().timestamp());
        let request = accept_provider_request(&service, "instagram", &signed, true)
            .await
            .unwrap();
        // Signed revocation independently proves remote grant retirement, so
        // this regression exercises local erasure without a provider DELETE.
        accept_provider_request(&service, "instagram", &signed, false)
            .await
            .unwrap();
        provider::reconcile_with_service(&state, &service)
            .await
            .unwrap();
        let id = request["confirmation_code"].as_str().unwrap();
        let status = request_status(&state.db, "instagram", id).await.unwrap();
        if has_parentless_receipt {
            assert_eq!(status["status"], "pending");
            let retained = journal::load(&state.db, &logical_key)
                .await
                .unwrap()
                .unwrap();
            assert!(retained.channel_bot_id.is_none());
            assert_eq!(
                retained.parts[0].platform_message_id.as_deref(),
                Some("legacy-saved-provider-id")
            );
            assert!(
                !channel_bot_service::get_bot(&state.db, &bot.id)
                    .await
                    .unwrap()
                    .is_active
            );
            assert_eq!(
                state
                    .db
                    .collection::<bson::Document>(crate::models::channel_message::COLLECTION_NAME)
                    .count_documents(doc! {"_id": &source})
                    .await
                    .unwrap(),
                1
            );
            let pending = state
                .db
                .collection::<bson::Document>(provider::REQUESTS)
                .find_one(doc! {"_id": id})
                .await
                .unwrap()
                .unwrap();
            assert!(!pending.contains_key("completed_at") && !pending.contains_key("expires_at"));
            assert!(
                state
                    .db
                    .collection::<bson::Document>("channel_meta_erasure_fences")
                    .find_one(doc! {"_id": &bot.id, "local_data_erased_at": {"$exists": true}})
                    .await
                    .unwrap()
                    .is_none()
            );
        } else {
            assert_eq!(status["status"], "completed");
            assert_eq!(
                state
                    .db
                    .collection::<bson::Document>(crate::models::channel_message::COLLECTION_NAME)
                    .count_documents(doc! {"channel_bot_id": &bot.id})
                    .await
                    .unwrap(),
                0
            );
            assert!(
                state
                    .db
                    .collection::<ChannelBot>(BOTS)
                    .find_one(doc! {"_id": &bot.id})
                    .await
                    .unwrap()
                    .is_none()
            );
        }
        assert!(server.received_requests().await.unwrap().is_empty());
    }
}

#[tokio::test]
async fn disconnect_retires_contextual_token_and_decoder_marker_together() {
    let (state, _, server, bot) = fixture("instagram").await;
    configure(&state, &bot).await;
    let mut rewrap_keys = crate::test_utils::test_encryption_keys();
    rewrap_keys.set_contextual_record_writes(true);
    Mock::given(method("DELETE"))
        .and(path("/v25.0/222/subscribed_apps"))
        .and(header("authorization", "Bearer fixture-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"success": true})))
        .expect(1)
        .mount(&server)
        .await;
    let current = super::super::channel_bot_service::get_bot(&state.db, &bot.id)
        .await
        .unwrap();
    let s = service(&state, &server);
    super::super::durable_wake_tests::resume_after(
        "meta_disconnect_provider_confirmed",
        async {
            s.disconnect(&current).await.unwrap();
        },
        async {
            super::super::channel_credential_continuity::rewrap_fixture(
                &state.db,
                &rewrap_keys,
                &current,
                &["bot_token_encrypted"],
            )
            .await;
        },
    )
    .await;
    let retired = super::super::channel_bot_service::get_bot(&state.db, &bot.id)
        .await
        .unwrap();
    assert!(!retired.is_active);
    assert!(retired.bot_token_encrypted.is_empty());
    assert!(
        !retired
            .secret_context_versions
            .contains_key("bot_token_encrypted")
    );
    server.verify().await;
}

#[tokio::test]
async fn manual_account_retirement_sweeper_preserves_transferred_history_without_explicit_delete() {
    for explicit_delete in [false, true] {
        use crate::services::ownership_transfer_service as transfer;
        let (state, _, server, mut bot) = fixture("instagram").await;
        configure(&state, &bot).await;
        bot.meta_binding.as_mut().unwrap().grant_subject_id.clear();
        bot.credential_source = "user".into();
        state
            .db
            .collection::<ChannelBot>(BOTS)
            .replace_one(doc! {"_id":&bot.id}, &bot)
            .await
            .unwrap();
        let recipient = uuid::Uuid::new_v4().to_string();
        state
            .db
            .collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
            .insert_one(crate::test_utils::test_user(
                &recipient,
                crate::models::user::UserType::Person,
            ))
            .await
            .unwrap();
        let source = uuid::Uuid::new_v4().to_string();
        state
            .db
            .collection::<bson::Document>(crate::models::channel_message::COLLECTION_NAME)
            .insert_one(doc! {"_id":&source,"user_id":&bot.user_id,"channel_bot_id":&bot.id})
            .await
            .unwrap();
        let receipt = super::super::channel_text_delivery::begin(
            &state.db,
            "manual-former-history",
            &bot.user_id,
            &source,
            "receipt",
            1,
        )
        .await
        .unwrap();
        let preview = transfer::preview(
            &state.db,
            &bot.user_id,
            transfer::ResourceKind::ChannelBot,
            &bot.id,
            &recipient,
            5,
        )
        .await
        .unwrap();
        transfer::transfer_with_keys(
            &state.db,
            Some(&state.encryption_keys),
            transfer::TransferCommand {
                actor: &bot.user_id,
                api_key_id: None,
                first_party_human: true,
                kind: transfer::ResourceKind::ChannelBot,
                resource_id: &bot.id,
                destination: &recipient,
                request_id: &uuid::Uuid::new_v4().to_string(),
                expected_version: &preview.version,
                capacity: 5,
            },
        )
        .await
        .unwrap();
        // Confirm remote removal using the existing test provider before account cleanup.
        Mock::given(method("DELETE"))
            .and(path("/v25.0/222/subscribed_apps"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"success":true})))
            .expect(1)
            .mount(&server)
            .await;
        let live = super::super::channel_bot_service::get_bot(&state.db, &bot.id)
            .await
            .unwrap();
        service(&state, &server).disconnect(&live).await.unwrap();
        if explicit_delete {
            let stopped = super::super::channel_bot_service::get_bot(&state.db, &bot.id)
                .await
                .unwrap();
            super::super::channel_bot_service::delete_meta_bot(
                &state.db,
                &state.config,
                &state.http_client,
                &state.encryption_keys,
                &stopped,
                &recipient,
            )
            .await
            .unwrap();
        }
        super::super::durable_wake_tests::resume_after(
            "meta_account_after_disconnect",
            erase_personal_owner(&state, &recipient),
            async {
                let s = service(&state, &server);
                state
                    .db
                    .collection::<bson::Document>(BOTS)
                    .update_one(
                        doc! {"_id":&bot.id},
                        doc! {"$set":{"meta_binding.next_reconcile_at":bson::DateTime::now()}},
                    )
                    .await
                    .unwrap();
                sweep_service(&state, &s).await.unwrap();
                assert_eq!(
                    super::super::channel_text_delivery::load(&state.db, "manual-former-history")
                        .await
                        .unwrap()
                        .is_none(),
                    explicit_delete
                );
            },
        )
        .await;
        assert_eq!(
            state
                .db
                .collection::<bson::Document>(crate::models::channel_text_delivery::COLLECTION_NAME)
                .find_one(doc! {"_id":receipt.id,"channel_bot_id":&bot.id})
                .await
                .unwrap()
                .is_none(),
            explicit_delete
        );
        server.verify().await;
    }
}

#[tokio::test]
async fn unknown_managed_grant_chronology_and_malformed_completion_never_acknowledge_deletion() {
    let (state, _, server, bot) = fixture("instagram").await;
    configure(&state, &bot).await;
    state.db.collection::<bson::Document>(BOTS).update_one(doc! {"_id":&bot.id},
        doc! {"$set":{"credential_source":"platform"},"$unset":{"meta_binding.authorized_at":""}}).await.unwrap();
    let s = service(&state, &server);
    let accepted = accept_provider_request(
        &s,
        "instagram",
        &provider_request(json!(900), Utc::now().timestamp()),
        true,
    )
    .await
    .unwrap();
    provider::reconcile_with_service(&state, &s).await.unwrap();
    assert_eq!(
        request_status(
            &state.db,
            "instagram",
            accepted["confirmation_code"].as_str().unwrap()
        )
        .await
        .unwrap()["status"],
        "pending"
    );
    // A present malformed field cannot use the absent-field legacy compatibility rule.
    state
        .db
        .collection::<bson::Document>(BOTS)
        .delete_one(doc! {"_id":&bot.id})
        .await
        .unwrap();
    state
        .db
        .collection::<bson::Document>("channel_meta_erasure_fences")
        .update_one(
            doc! {"_id":&bot.id},
            doc! {"$set":{"local_data_erased_at":bson::DateTime::now(),"completed_grant_id":42}},
        )
        .upsert(true)
        .await
        .unwrap();
    assert!(matches!(
        super::super::personal_erasure_service::erase_provider_inbox(
            &state,
            &bot.id,
            &bot.meta_binding.as_ref().unwrap().grant_id
        )
        .await,
        Err(AppError::Conflict(_))
    ));
}

#[tokio::test]
async fn signed_callback_chronology_repair_rechecks_both_sides_before_effects() {
    for erase in [false, true] {
        for later in [false, true] {
            let (state, _, server, bot) = fixture("instagram").await;
            configure(&state, &bot).await;
            let bots = state.db.collection::<bson::Document>(BOTS);
            // Both missing and wrong-type retained dates are unresolved, never empty success.
            if later {
                bots.update_one(
                    doc! {"_id":&bot.id},
                    doc! {"$set":{"meta_binding.authorized_at":"unknown"}},
                )
                .await
                .unwrap();
            } else {
                bots.update_one(
                    doc! {"_id":&bot.id},
                    doc! {"$unset":{"meta_binding.authorized_at":""}},
                )
                .await
                .unwrap();
            }
            let s = service(&state, &server);
            let issued = Utc::now().timestamp();
            let accepted = accept_provider_request(
                &s,
                "instagram",
                &provider_request(json!(900), issued),
                erase,
            )
            .await
            .unwrap();
            let code = accepted["confirmation_code"].as_str().unwrap();
            provider::reconcile_with_service(&state, &s).await.unwrap();
            // The ordinary sweep cannot receive a callback-induced removal/revocation state.
            let _ = sweep_service(&state, &s).await;
            let pending = bots.find_one(doc! {"_id":&bot.id}).await.unwrap().unwrap();
            assert_eq!(pending.get_bool("is_active"), Ok(true));
            assert_eq!(
                pending
                    .get_document("meta_binding")
                    .unwrap()
                    .get_bool("provider_revoked"),
                Ok(false)
            );
            assert_eq!(
                pending.get_binary_generic("bot_token_encrypted").unwrap(),
                &bot.bot_token_encrypted
            );
            assert!(
                server
                    .received_requests()
                    .await
                    .unwrap()
                    .iter()
                    .all(|r| r.method.as_str() != "DELETE")
            );
            let request = state
                .db
                .collection::<bson::Document>(provider::REQUESTS)
                .find_one(doc! {"_id":code})
                .await
                .unwrap()
                .unwrap();
            assert_eq!(request.get_str("status"), Ok("pending"));
            let cutoff = request.get_datetime("signed_cutoff").unwrap().to_chrono();
            let repaired = if later {
                cutoff + Duration::seconds(1)
            } else {
                cutoff - Duration::seconds(1)
            };
            bots.update_one(
                doc! {"_id":&bot.id},
                doc! {"$set":{"meta_binding.authorized_at":bson::DateTime::from_chrono(repaired)}},
            )
            .await
            .unwrap();
            if erase && !later {
                Mock::given(method("DELETE"))
                    .and(path("/v25.0/222/subscribed_apps"))
                    .respond_with(ResponseTemplate::new(200).set_body_json(json!({"success":true})))
                    .expect(1)
                    .mount(&server)
                    .await;
            }
            state.db.collection::<bson::Document>(provider::REQUESTS).update_one(doc! {"_id":code},
                doc! {"$set":{"next_attempt_at":bson::DateTime::now(),"targets.0.retry_at":bson::DateTime::now()}}).await.unwrap();
            provider::reconcile_with_service(&state, &s).await.unwrap();
            assert_eq!(
                state
                    .db
                    .collection::<bson::Document>(provider::REQUESTS)
                    .find_one(doc! {"_id":code})
                    .await
                    .unwrap()
                    .unwrap()
                    .get_str("status"),
                Ok("completed")
            );
            let remaining = bots.find_one(doc! {"_id":&bot.id}).await.unwrap();
            if later {
                let remaining = remaining.unwrap();
                assert_eq!(remaining.get_bool("is_active"), Ok(true));
                assert_eq!(
                    remaining.get_binary_generic("bot_token_encrypted").unwrap(),
                    &bot.bot_token_encrypted
                );
            } else if erase {
                assert!(remaining.is_none());
            } else {
                assert!(
                    remaining
                        .unwrap()
                        .get_binary_generic("bot_token_encrypted")
                        .unwrap()
                        .is_empty()
                );
            }
            server.verify().await;
        }
    }
}

#[tokio::test]
async fn provider_completion_requires_a_matched_typed_exact_fence_and_replay_clears_stale_scope() {
    let (state, _, _server, mut bot) = fixture("instagram").await;
    bot.is_active = false;
    bot.meta_binding.as_mut().unwrap().subscription = "removed".into();
    state
        .db
        .collection::<ChannelBot>(BOTS)
        .replace_one(doc! {"_id":&bot.id}, &bot)
        .await
        .unwrap();
    let grant = &bot.meta_binding.as_ref().unwrap().grant_id;
    let fences = state
        .db
        .collection::<bson::Document>("channel_meta_erasure_fences");
    super::super::durable_wake_tests::resume_after(
        "meta_provider_before_completion",
        async {
            assert!(matches!(
                super::super::personal_erasure_service::erase_provider_inbox(
                    &state, &bot.id, grant
                )
                .await,
                Err(AppError::Conflict(_))
            ));
        },
        async {
            fences.delete_one(doc! {"_id":&bot.id}).await.unwrap();
        },
    )
    .await;
    assert!(
        state
            .db
            .collection::<ChannelBot>(BOTS)
            .find_one(doc! {"_id":&bot.id})
            .await
            .unwrap()
            .is_some()
    );
    super::super::personal_erasure_service::erase_provider_inbox(&state, &bot.id, grant)
        .await
        .unwrap();
    fences
        .update_one(
            doc! {"_id":&bot.id},
            doc! {"$set":{"retired_grant":{"grant_id":grant}}},
        )
        .await
        .unwrap();
    super::super::personal_erasure_service::erase_provider_inbox(&state, &bot.id, grant)
        .await
        .unwrap();
    assert!(
        !fences
            .find_one(doc! {"_id":&bot.id})
            .await
            .unwrap()
            .unwrap()
            .contains_key("retired_grant")
    );
    for malformed in [
        bson::Bson::Null,
        bson::Bson::Int32(42),
        bson::Bson::String("other-grant".into()),
    ] {
        fences
            .update_one(
                doc! {"_id":&bot.id},
                doc! {"$set":{"completed_grant_id":malformed}},
            )
            .await
            .unwrap();
        assert!(matches!(
            super::super::personal_erasure_service::erase_provider_inbox(&state, &bot.id, grant)
                .await,
            Err(AppError::Conflict(_))
        ));
    }
}

#[tokio::test]
async fn transferred_manual_inbox_requires_verified_token_and_explicit_routing_confirmation() {
    use crate::services::{channel_bot_service as bots, ownership_transfer_service as transfer};
    for held in [false, true] {
        let (state, adapter, server, mut bot) = fixture("instagram").await;
        configure(&state, &bot).await;
        bot.meta_binding.as_mut().unwrap().grant_subject_id.clear();
        state
            .db
            .collection::<ChannelBot>(BOTS)
            .replace_one(doc! {"_id":&bot.id}, &bot)
            .await
            .unwrap();
        if held {
            set_bot_delivery(&state.db, &bot, &bot.user_id, false)
                .await
                .unwrap();
        }
        let recipient = uuid::Uuid::new_v4().to_string();
        state
            .db
            .collection::<crate::models::user::User>(crate::models::user::COLLECTION_NAME)
            .insert_one(crate::test_utils::test_user(
                &recipient,
                crate::models::user::UserType::Person,
            ))
            .await
            .unwrap();
        flags::set_platform_override(
            &state.db,
            "channels:instagram",
            &FlagTarget::User(recipient.clone()),
            true,
            &recipient,
        )
        .await
        .unwrap();
        let preview = transfer::preview(
            &state.db,
            &bot.user_id,
            transfer::ResourceKind::ChannelBot,
            &bot.id,
            &recipient,
            5,
        )
        .await
        .unwrap();
        transfer::transfer_with_keys(
            &state.db,
            Some(&state.encryption_keys),
            transfer::TransferCommand {
                actor: &bot.user_id,
                api_key_id: None,
                first_party_human: true,
                kind: transfer::ResourceKind::ChannelBot,
                resource_id: &bot.id,
                destination: &recipient,
                request_id: &uuid::Uuid::new_v4().to_string(),
                expected_version: &preview.version,
                capacity: 5,
            },
        )
        .await
        .unwrap();
        Mock::given(method("GET"))
            .and(path("/v25.0/me"))
            .and(header("authorization", "Bearer replacement-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                json!({"user_id":"222","username":"professional","account_type":"Business"}),
            ))
            .expect(3)
            .mount(&server)
            .await;
        subscriptions(&server, "222").await;
        let before = bots::get_bot(&state.db, &bot.id).await.unwrap();
        assert!(
            !before
                .meta_binding
                .as_ref()
                .unwrap()
                .exclusive_routing_verified
        );
        for confirm in [false, true] {
            let updated = bots::update_bot_as_actor(
                &state.db,
                &state.encryption_keys,
                &state.http_client,
                &adapter,
                &bot.id,
                bots::UpdateBotAuthority {
                    actor_id: &recipient,
                    owner_id: &recipient,
                },
                bots::UpdateBotParams {
                    x_events: None,
                    bot_token: Some("replacement-token"),
                    label: None,
                    verification_token: confirm.then_some("exclusive"),
                    encrypt_key: bots::SecretPatch::Unchanged,
                    app_id: None,
                    app_secret: None,
                },
            )
            .await;
            if !confirm {
                assert!(matches!(updated, Err(AppError::ValidationError(_))));
                let retained = bots::get_bot(&state.db, &bot.id).await.unwrap();
                assert_eq!(
                    retained.meta_binding.as_ref().unwrap().generation,
                    before.meta_binding.as_ref().unwrap().generation
                );
                assert!(
                    retained
                        .meta_binding
                        .as_ref()
                        .unwrap()
                        .grant_reconnect_required
                );
                continue;
            }
            let mut repaired = updated.unwrap();
            let binding = repaired.meta_binding.as_ref().unwrap();
            assert_eq!(repaired.id, bot.id);
            assert!(binding.exclusive_routing_verified && !binding.grant_reconnect_required);
            assert_eq!(binding.operator_suspended, held);
            assert!(binding.events_since.unwrap() >= before.updated_at);
            let s = service(&state, &server);
            if held {
                assert!(s.verify(&repaired).await.is_err());
                set_bot_delivery(&state.db, &repaired, &recipient, true)
                    .await
                    .unwrap();
                repaired = bots::get_bot(&state.db, &bot.id).await.unwrap();
            }
            s.verify(&repaired).await.unwrap();
            assert_eq!(
                bots::get_bot(&state.db, &bot.id)
                    .await
                    .unwrap()
                    .meta_binding
                    .unwrap()
                    .subscription,
                "active"
            );
        }
        server.verify().await;
    }
}

#[tokio::test]
async fn removed_manual_retry_requires_positive_delete_evidence_for_the_same_installation() {
    let (state, _, server, mut bot) = fixture("instagram").await;
    configure(&state, &bot).await;
    bot.is_active = false;
    bot.meta_binding.as_mut().unwrap().subscription = "removed".into();
    state
        .db
        .collection::<ChannelBot>(BOTS)
        .replace_one(doc! {"_id":&bot.id}, &bot)
        .await
        .unwrap();
    assert!(
        !grants::manual_purge_authorized(&state.db, &bot)
            .await
            .unwrap()
    );
    super::super::audit_service::log_actor_event(
        state.db.clone(),
        &super::super::audit_service::AuditActor {
            user_id: bot.user_id.clone(),
            ip_address: None,
            user_agent: None,
            api_key_id: None,
            api_key_name: None,
        },
        "channel_bot_deleted",
        Some(json!({"bot_id":bot.id,"platform":bot.platform,
        "owner_user_id":bot.user_id,"managed_webhook_cleanup":"removed"})),
    )
    .await
    .unwrap();
    assert!(
        grants::manual_purge_authorized(&state.db, &bot)
            .await
            .unwrap()
    );
    sweep_service(&state, &service(&state, &server))
        .await
        .unwrap();
    assert!(
        state
            .db
            .collection::<ChannelBot>(BOTS)
            .find_one(doc! {"_id":&bot.id})
            .await
            .unwrap()
            .is_none()
    );
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn app_drain_negative_lookup_is_indexed_after_bounded_authority_state_recovery() {
    use futures::TryStreamExt;
    let (state, _, _, bot) = fixture("instagram").await;
    configure(&state, &bot).await;
    let authority = state
        .db
        .collection::<bson::Document>("channel_meta_grant_authority");
    let now = bson::DateTime::now();
    let rows: Vec<_> = (0..1200)
        .map(|i| {
            doc! {"_id":uuid::Uuid::new_v4().to_string(),
            "identity":format!("retained-{i}"),"platform":"instagram","app_id":"111",
            "revoked_at":now,"last_authorized_at":now,"outstanding":false}
        })
        .collect();
    authority.insert_many(rows).await.unwrap();
    // Earlier feature records are reconciled in bounded pages, on their original rows.
    for i in 0..125 {
        authority
            .insert_one(doc! {"_id":uuid::Uuid::new_v4().to_string(),
            "identity":format!("legacy-derived-{i}"),"platform":"instagram","app_id":"111",
            "revoked_at":now,"last_authorized_at":now})
            .await
            .unwrap();
    }
    grants::reconcile_authority_state(&state.db).await.unwrap();
    assert_eq!(
        authority
            .count_documents(doc! {"outstanding":{"$exists":false}})
            .await
            .unwrap(),
        25
    );
    grants::reconcile_authority_state(&state.db).await.unwrap();
    let filter =
        doc! {"platform":"instagram","app_id":"111","outstanding":{"$in":[true,bson::Bson::Null]}};
    assert!(authority.find_one(filter.clone()).await.unwrap().is_none());
    let explain = state
        .db
        .run_command(doc! {"explain":{"find":"channel_meta_grant_authority",
        "filter":filter,"projection":{"_id":1},"limit":1},"verbosity":"executionStats"})
        .await
        .unwrap();
    let stats = explain.get_document("executionStats").unwrap();
    let count = |field| {
        stats
            .get_i64(field)
            .unwrap_or_else(|_| i64::from(stats.get_i32(field).unwrap()))
    };
    assert_eq!(count("nReturned"), 0);
    assert!(
        count("totalDocsExamined") <= 2 && count("totalKeysExamined") <= 4,
        "{stats:?}"
    );
    assert_eq!(
        authority
            .find(doc! {"outstanding":false})
            .await
            .unwrap()
            .try_collect::<Vec<_>>()
            .await
            .unwrap()
            .len(),
        1325
    );
}

#[tokio::test]
async fn manual_delete_snapshot_cannot_stop_or_purge_a_reconnected_managed_grant() {
    let (state, adapter, server, bot) = fixture("instagram").await;
    configure(&state, &bot).await;
    let successor_grant = uuid::Uuid::new_v4().to_string();
    super::super::durable_wake_tests::resume_after(
        "meta_delete_before_stop",
        async {
            assert!(matches!(
                super::super::channel_bot_service::delete_bot_with_state_as_actor(
                    &state,
                    &adapter,
                    &bot.id,
                    &bot.user_id,
                    &bot.user_id
                )
                .await,
                Err(AppError::Conflict(_))
            ));
        },
        async {
            state.db.collection::<bson::Document>(BOTS).update_one(doc! {"_id":&bot.id},doc! {
            "$set":{"credential_source":"platform","meta_binding.grant_id":&successor_grant},
            "$inc":{"meta_binding.generation":1_i64}}).await.unwrap();
        },
    )
    .await;
    let retained = state
        .db
        .collection::<ChannelBot>(BOTS)
        .find_one(doc! {"_id":&bot.id})
        .await
        .unwrap()
        .unwrap();
    assert!(retained.is_active);
    assert_eq!(retained.meta_binding.unwrap().grant_id, successor_grant);
    assert!(
        state
            .db
            .collection::<bson::Document>("channel_meta_erasure_fences")
            .find_one(doc! {"_id":&bot.id})
            .await
            .unwrap()
            .is_none()
    );
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn verify_and_refresh_share_the_asset_lease_and_preserve_same_token_expiry() {
    for same_token in [false, true] {
        let (state, _, server, bot) = fixture("instagram").await;
        configure(&state, &bot).await;
        state.db.collection::<bson::Document>(BOTS).update_one(doc! {"_id":&bot.id},doc! {"$set":{
            "meta_binding.subscription":"pending","meta_binding.expires_at":bson::DateTime::from_chrono(Utc::now()+Duration::days(2))}}).await.unwrap();
        let due = super::super::channel_bot_service::get_bot(&state.db, &bot.id)
            .await
            .unwrap();
        let refreshed_token = if same_token {
            "fixture-token"
        } else {
            "refreshed-token"
        };
        Mock::given(method("GET"))
            .and(path("/refresh_access_token"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"access_token":refreshed_token,"expires_in":5184000})),
            )
            .expect(1)
            .mount(&server)
            .await;
        for token in ["fixture-token", "refreshed-token"] {
            Mock::given(method("GET"))
                .and(path("/v25.0/me"))
                .and(header("authorization", format!("Bearer {token}")))
                .respond_with(ResponseTemplate::new(200).set_body_json(
                    json!({"user_id":"222","username":"professional","account_type":"Business"}),
                ))
                .mount(&server)
                .await;
        }
        subscriptions(&server, "222").await;
        let s = service(&state, &server);
        super::super::durable_wake_tests::resume_after(
            "meta_refresh_provider_confirmed",
            async {
                s.refresh(&due).await.unwrap();
            },
            async {
                // A normal Verify must lose the busy asset lease at this exact boundary.
                assert!(matches!(s.verify(&due).await, Err(AppError::Conflict(_))));
            },
        )
        .await;
        let fresh = super::super::channel_bot_service::get_bot(&state.db, &bot.id)
            .await
            .unwrap();
        let extended = fresh.meta_binding.as_ref().unwrap().expires_at.unwrap();
        assert!(extended > Utc::now() + Duration::days(59));
        s.verify(&due).await.unwrap();
        let active = super::super::channel_bot_service::get_bot(&state.db, &bot.id)
            .await
            .unwrap();
        assert_eq!(active.meta_binding.as_ref().unwrap().subscription, "active");
        assert_eq!(
            active.meta_binding.as_ref().unwrap().expires_at,
            Some(extended)
        );
        // A delayed failure for the old logical token cannot stop the successor.
        if !same_token {
            s.reconnect_required(&due).await.unwrap();
            assert!(
                !super::super::channel_bot_service::get_bot(&state.db, &bot.id)
                    .await
                    .unwrap()
                    .meta_binding
                    .unwrap()
                    .grant_reconnect_required
            );
        }
        server.verify().await;
    }
}

#[tokio::test]
async fn invalid_token_verify_holds_the_same_asset_fence_as_refresh() {
    let (state, _, server, bot) = fixture("instagram").await;
    configure(&state, &bot).await;
    state.db.collection::<bson::Document>(BOTS).update_one(doc! {"_id":&bot.id},doc! {
        "$set":{"meta_binding.expires_at":bson::DateTime::from_chrono(Utc::now()+Duration::days(2))}}).await.unwrap();
    let due = super::super::channel_bot_service::get_bot(&state.db, &bot.id)
        .await
        .unwrap();
    Mock::given(method("GET"))
        .and(path("/v25.0/me"))
        .respond_with(
            ResponseTemplate::new(400)
                .set_body_json(json!({"error":{"code":190,"message":"expired"}})),
        )
        .expect(1)
        .mount(&server)
        .await;
    let s = service(&state, &server);
    super::super::durable_wake_tests::resume_after(
        "meta_verify_token_rejected",
        async {
            assert!(s.verify(&due).await.is_err());
        },
        async {
            assert!(matches!(s.refresh(&due).await, Err(AppError::Conflict(_))));
        },
    )
    .await;
    let stopped = super::super::channel_bot_service::get_bot(&state.db, &bot.id)
        .await
        .unwrap();
    assert!(stopped.meta_binding.unwrap().grant_reconnect_required);
    assert!(
        server
            .received_requests()
            .await
            .unwrap()
            .iter()
            .all(|request| request.url.path() != "/refresh_access_token")
    );
}

#[tokio::test]
async fn legacy_absent_grant_can_refresh_and_disconnect_but_malformed_grant_cannot_publish() {
    for malformed in [false, true] {
        let (state, _, server, bot) = fixture("instagram").await;
        configure(&state, &bot).await;
        let bots = state.db.collection::<bson::Document>(BOTS);
        bots.update_one(doc! {"_id":&bot.id},doc! {"$unset":{"meta_binding.grant_id":""},
            "$set":{"meta_binding.expires_at":bson::DateTime::from_chrono(Utc::now()+Duration::days(2))}}).await.unwrap();
        let due = super::super::channel_bot_service::get_bot(&state.db, &bot.id)
            .await
            .unwrap();
        assert!(due.meta_binding.as_ref().unwrap().grant_id.is_empty());
        Mock::given(method("GET"))
            .and(path("/refresh_access_token"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"access_token":"fresh-token","expires_in":5184000})),
            )
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/v25.0/me"))
            .and(header("authorization", "Bearer fresh-token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                json!({"user_id":"222","username":"professional","account_type":"Business"}),
            ))
            .expect(1)
            .mount(&server)
            .await;
        let s = service(&state, &server);
        if malformed {
            super::super::durable_wake_tests::resume_after(
                "meta_refresh_provider_confirmed",
                async {
                    assert!(matches!(s.refresh(&due).await, Err(AppError::Conflict(_))));
                },
                async {
                    bots.update_one(
                        doc! {"_id":&bot.id},
                        doc! {"$set":{"meta_binding.grant_id":bson::Bson::Null}},
                    )
                    .await
                    .unwrap();
                },
            )
            .await;
            assert_eq!(
                bots.find_one(doc! {"_id":&bot.id})
                    .await
                    .unwrap()
                    .unwrap()
                    .get_binary_generic("bot_token_encrypted")
                    .unwrap(),
                &bot.bot_token_encrypted
            );
        } else {
            s.refresh(&due).await.unwrap();
            let fresh = super::super::channel_bot_service::get_bot(&state.db, &bot.id)
                .await
                .unwrap();
            Mock::given(method("DELETE"))
                .and(path("/v25.0/222/subscribed_apps"))
                .respond_with(ResponseTemplate::new(200).set_body_json(json!({"success":true})))
                .expect(1)
                .mount(&server)
                .await;
            s.disconnect(&fresh).await.unwrap();
            let removed = bots.find_one(doc! {"_id":&bot.id}).await.unwrap().unwrap();
            assert_eq!(
                removed
                    .get_document("meta_binding")
                    .unwrap()
                    .get_str("subscription"),
                Ok("removed")
            );
            assert!(
                !removed
                    .get_document("meta_binding")
                    .unwrap()
                    .contains_key("grant_id")
            );
        }
        server.verify().await;
    }
}

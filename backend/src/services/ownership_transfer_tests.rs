use super::ownership_transfer_service::*;
use crate::{
    errors::AppError,
    models::{
        agent_service_binding::COLLECTION_NAME as AGENT_BINDINGS,
        channel_bot::{COLLECTION_NAME as BOTS, ChannelBot},
        channel_conversation::COLLECTION_NAME as CONVERSATIONS,
        downstream_service::{COLLECTION_NAME as SERVICES, DownstreamService},
        oauth_state::COLLECTION_NAME as OAUTH_STATES,
        org_membership::OrgRole,
        ownership_transfer::COLLECTION_NAME as TRANSFERS,
        provider_config::{COLLECTION_NAME as PROVIDERS, ProviderConfig},
        user::{COLLECTION_NAME as USERS, User, UserType},
        user_api_key::{COLLECTION_NAME as USER_API_KEYS, UserApiKey},
        user_provider_token::COLLECTION_NAME as USER_PROVIDER_TOKENS,
        user_service::{COLLECTION_NAME as USER_SERVICES, UserService},
    },
    services::{
        catalog_service, channel_adapters::x::REQUIRED_SCOPES as X_REQUIRED_SCOPES,
        channel_credentials, channel_routing_service, role_service,
    },
    test_utils,
};
use bson::{Document, doc};
use chrono::Utc;
use mongodb::Database;
use uuid::Uuid;

async fn owner_transfer(
    f: &Fixture,
    (actor, key): (&str, Option<&str>),
    kind: ResourceKind,
    id: &str,
    destination: &str,
    version: &str,
    request: &str,
) -> crate::errors::AppResult<crate::models::ownership_transfer::OwnershipTransfer> {
    transfer(
        &f.db,
        TransferCommand {
            actor,
            api_key_id: key,
            kind,
            resource_id: id,
            destination,
            request_id: request,
            expected_version: version,
            capacity: 5,
        },
    )
    .await
}

async fn management_key(
    f: &Fixture,
    owner: &str,
    scopes: &str,
) -> crate::services::key_service::CreatedApiKey {
    crate::services::key_service::create_api_key(
        &f.db,
        owner,
        "Asset manager",
        scopes,
        None,
        None,
        None,
        None,
        Some(true),
        Some(false),
        Some(true),
        None,
        None,
        Some("codex"),
        None,
    )
    .await
    .unwrap()
}

#[tokio::test]
async fn owners_transfer_bots_and_catalog_and_replay_after_losing_access() {
    let f = fixture("ownership_owner_access").await;
    let bot = insert_bot(&f).await;
    for (kind, id) in [
        (ResourceKind::Service, &f.service.id),
        (ResourceKind::ChannelBot, &bot.id),
    ] {
        let reviewed = preview(&f.db, &f.owner, kind, id, &f.destination, 5)
            .await
            .unwrap();
        let request = Uuid::new_v4().to_string();
        let moved = owner_transfer(
            &f,
            (&f.owner, None),
            kind,
            id,
            &f.destination,
            &reviewed.version,
            &request,
        )
        .await
        .unwrap();
        assert_eq!(moved.previous_owner_user_id, f.owner);
        let replay = owner_transfer(
            &f,
            (&f.owner, None),
            kind,
            id,
            &f.destination,
            &reviewed.version,
            &request,
        )
        .await
        .unwrap();
        assert_eq!(moved.id, replay.id);
        assert!(
            preview(&f.db, &f.owner, kind, id, &f.owner, 5)
                .await
                .is_err()
        );
    }
}

#[tokio::test]
async fn org_transfer_rechecks_membership_and_denies_members_viewers_and_restricted_admins() {
    use crate::models::org_membership::{COLLECTION_NAME as MEMBERSHIPS, OrgMembership};
    let f = fixture("ownership_org_authority").await;
    let bot = insert_bot(&f).await;
    f.db.collection::<Document>(BOTS)
        .update_one(
            doc! {"_id": &bot.id},
            doc! {"$set": {"user_id": &f.destination}},
        )
        .await
        .unwrap();
    let membership = test_utils::test_membership(&f.destination, &f.owner, OrgRole::Admin, None);
    f.db.collection::<OrgMembership>(MEMBERSHIPS)
        .insert_one(&membership)
        .await
        .unwrap();
    let reviewed = preview(
        &f.db,
        &f.owner,
        ResourceKind::ChannelBot,
        &bot.id,
        &f.owner,
        5,
    )
    .await
    .unwrap();
    for change in [
        doc! {"role": "member"},
        doc! {"role": "viewer"},
        doc! {"role": "admin", "allowed_service_ids": []},
        doc! {"allowed_service_ids": null, "revoked_at": bson::DateTime::now()},
    ] {
        f.db.collection::<Document>(MEMBERSHIPS)
            .update_one(doc! {"_id": &membership.id}, doc! {"$set": change})
            .await
            .unwrap();
        assert!(matches!(
            owner_transfer(
                &f,
                (&f.owner, None),
                ResourceKind::ChannelBot,
                &bot.id,
                &f.owner,
                &reviewed.version,
                &Uuid::new_v4().to_string()
            )
            .await,
            Err(AppError::Forbidden(_))
        ));
    }
    f.db.collection::<Document>(MEMBERSHIPS)
        .update_one(
            doc! {"_id": &membership.id},
            doc! {"$set": {"revoked_at": null}},
        )
        .await
        .unwrap();
    let moved = owner_transfer(
        &f,
        (&f.owner, None),
        ResourceKind::ChannelBot,
        &bot.id,
        &f.owner,
        &reviewed.version,
        &Uuid::new_v4().to_string(),
    )
    .await
    .unwrap();
    assert_eq!(moved.previous_owner_user_id, f.destination);
    assert_eq!(moved.new_owner_user_id, f.owner);
}

#[tokio::test]
async fn agent_transfer_checks_live_management_scope_and_binds_receipt_to_key() {
    use crate::models::api_key::COLLECTION_NAME as KEYS;
    let f = fixture("ownership_agent_authority").await;
    let bot = insert_bot(&f).await;
    let key = management_key(&f, &f.owner, "write").await;
    let other_key = management_key(&f, &f.owner, "admin").await;
    let reviewed = preview_with_agent(
        &f.db,
        &f.owner,
        Some(&key.id),
        ResourceKind::ChannelBot,
        &bot.id,
        &f.destination,
        5,
    )
    .await
    .unwrap();
    for change in [
        doc! {"scopes": "proxy"},
        doc! {"scopes": "write", "allow_all_services": false},
        doc! {"allow_all_services": true, "is_active": false},
        doc! {"is_active": true, "expires_at": bson::DateTime::from_millis(1)},
        doc! {"expires_at": null, "purpose": "scheduled_invocation"},
    ] {
        f.db.collection::<Document>(KEYS)
            .update_one(doc! {"_id": &key.id}, doc! {"$set": change})
            .await
            .unwrap();
        assert!(matches!(
            owner_transfer(
                &f,
                (&f.owner, Some(&key.id)),
                ResourceKind::ChannelBot,
                &bot.id,
                &f.destination,
                &reviewed.version,
                &Uuid::new_v4().to_string()
            )
            .await,
            Err(AppError::Forbidden(_))
        ));
    }
    f.db.collection::<Document>(KEYS)
        .update_one(doc! {"_id": &key.id}, doc! {"$set": {"purpose": "general"}})
        .await
        .unwrap();
    let request = Uuid::new_v4().to_string();
    let moved = owner_transfer(
        &f,
        (&f.owner, Some(&key.id)),
        ResourceKind::ChannelBot,
        &bot.id,
        &f.destination,
        &reviewed.version,
        &request,
    )
    .await
    .unwrap();
    assert_eq!(moved.actor_api_key_id.as_deref(), Some(key.id.as_str()));
    owner_transfer(
        &f,
        (&f.owner, Some(&key.id)),
        ResourceKind::ChannelBot,
        &bot.id,
        &f.destination,
        &reviewed.version,
        &request,
    )
    .await
    .unwrap();
    assert!(
        owner_transfer(
            &f,
            (&f.owner, Some(&other_key.id)),
            ResourceKind::ChannelBot,
            &bot.id,
            &f.destination,
            &reviewed.version,
            &request
        )
        .await
        .is_err()
    );
    let admin_key = management_key(&f, &f.admin, "admin").await;
    assert!(
        preview_with_agent(
            &f.db,
            &f.admin,
            Some(&admin_key.id),
            ResourceKind::Service,
            &f.service.id,
            &f.destination,
            5
        )
        .await
        .is_err()
    );
    let org_key = management_key(&f, &f.destination, "write").await;
    let ready = preview_with_agent(
        &f.db,
        &f.destination,
        Some(&org_key.id),
        ResourceKind::ChannelBot,
        &bot.id,
        &f.owner,
        5,
    )
    .await
    .unwrap();
    let returned = owner_transfer(
        &f,
        (&f.destination, Some(&org_key.id)),
        ResourceKind::ChannelBot,
        &bot.id,
        &f.owner,
        &ready.version,
        &Uuid::new_v4().to_string(),
    )
    .await
    .unwrap();
    assert_eq!(returned.previous_owner_user_id, f.destination);
    assert_eq!(returned.new_owner_user_id, f.owner);
}

#[tokio::test]
async fn owner_destination_search_does_not_expose_platform_directory() {
    use crate::services::ownership_transfer_access as access;
    let f = fixture("ownership_destination_privacy").await;
    let (known, _) =
        access::destination_candidates(&f.db, &f.owner, &f.owner, false, "person", "", 0)
            .await
            .unwrap();
    assert_eq!(known.len(), 1);
    assert_eq!(known[0].get_str("_id").unwrap(), f.owner);
    let (exact, _) =
        access::destination_candidates(&f.db, &f.owner, &f.owner, false, "person", &f.admin, 0)
            .await
            .unwrap();
    assert_eq!(exact.len(), 1);
    let (all, _) = access::destination_candidates(&f.db, &f.admin, &f.owner, true, "person", "", 0)
        .await
        .unwrap();
    assert_eq!(all.len(), 2);
}

#[tokio::test]
async fn agent_can_transfer_through_public_asset_routes_without_mobile_approval() {
    use axum::{
        body::{Body, to_bytes},
        http::{Request, StatusCode},
    };
    use tower::ServiceExt;
    let f = fixture("ownership_agent_router").await;
    let key = management_key(&f, &f.owner, "write").await;
    let read_key = management_key(&f, &f.owner, "proxy").await;
    let state = test_utils::test_app_state(f.db.clone());
    let (_, router) = crate::routes::build_router_with_state(state.clone());
    let app = router.with_state(state);
    let base = format!("/api/v1/ownership/service/{}", f.service.id);
    let body = serde_json::json!({"new_owner_user_id": f.destination});
    let call = |path: String, token: &str, body: serde_json::Value| {
        Request::builder()
            .method("POST")
            .uri(path)
            .header("x-api-key", token)
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    };
    let rejected = app
        .clone()
        .oneshot(call(
            format!("{base}/preview"),
            &read_key.full_key,
            body.clone(),
        ))
        .await
        .unwrap();
    assert_eq!(rejected.status(), StatusCode::FORBIDDEN);
    let response = app
        .clone()
        .oneshot(call(format!("{base}/preview"), &key.full_key, body))
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let reviewed: serde_json::Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 100_000).await.unwrap()).unwrap();
    let moved = app.clone().oneshot(call(format!("{base}/transfer"), &key.full_key, serde_json::json!({
        "new_owner_user_id": f.destination, "expected_version": reviewed["version"], "request_id": Uuid::new_v4().to_string()
    }))).await.unwrap();
    assert_eq!(moved.status(), StatusCode::OK);
    let audit =
        f.db.collection::<Document>(crate::models::audit_log::COLLECTION_NAME)
            .find_one(doc! {"event_type": "admin_ownership_transferred", "api_key_id": &key.id})
            .await
            .unwrap();
    assert!(audit.is_some());
}

struct Fixture {
    db: Database,
    admin: String,
    owner: String,
    destination: String,
    service: DownstreamService,
}

async fn fixture(label: &str) -> Fixture {
    let db = test_utils::connect_transaction_test_database(label).await;
    role_service::seed_system_roles(&db).await.unwrap();
    let roles = role_service::get_platform_role_ids(&db).await.unwrap();
    let admin = Uuid::new_v4().to_string();
    let owner = Uuid::new_v4().to_string();
    let destination = Uuid::new_v4().to_string();
    let mut user = test_utils::test_user(&admin, UserType::Person);
    user.role_ids.push(roles.admin);
    db.collection::<User>(USERS)
        .insert_many([
            user,
            test_utils::test_user(&owner, UserType::Person),
            test_utils::test_user(&destination, UserType::Org),
        ])
        .await
        .unwrap();
    let mut service = crate::models::downstream_service::test_helpers::dummy_service();
    service.id = Uuid::new_v4().to_string();
    service.slug = format!("transfer-{}", service.id);
    service.created_by = owner.clone();
    service.visibility = "private".into();
    service.service_category = "connection".into();
    service.requires_user_credential = true;
    db.collection::<DownstreamService>(SERVICES)
        .insert_one(&service)
        .await
        .unwrap();
    Fixture {
        db,
        admin,
        owner,
        destination,
        service,
    }
}

async fn move_resource(
    f: &Fixture,
    kind: ResourceKind,
    id: &str,
    destination: &str,
    version: &str,
    request: &str,
) -> crate::errors::AppResult<crate::models::ownership_transfer::OwnershipTransfer> {
    transfer(
        &f.db,
        TransferCommand {
            actor: &f.admin,
            api_key_id: None,
            kind,
            resource_id: id,
            destination,
            request_id: request,
            expected_version: version,
            capacity: 5,
        },
    )
    .await
}

async fn insert_bot(f: &Fixture) -> ChannelBot {
    let id = Uuid::new_v4().to_string();
    let doc = doc! { "_id": id, "user_id": &f.owner, "platform": "telegram", "label": "Team bot",
    "credential_source": "user", "bot_token_encrypted": bson::Binary { subtype: bson::spec::BinarySubtype::Generic, bytes: vec![1,2,3] },
    "platform_bot_id": "remote-bot", "platform_bot_username": "team_bot", "webhook_registered": true,
    "webhook_secret_hash": "original-secret-hash", "status": "active", "is_active": true,
    "created_at": bson::DateTime::now(), "updated_at": bson::DateTime::now() };
    f.db.collection::<Document>(BOTS)
        .insert_one(&doc)
        .await
        .unwrap();
    bson::from_document(doc).unwrap()
}

async fn insert_x_bot(f: &Fixture) -> (ChannelBot, UserApiKey) {
    let state = test_utils::test_app_state(f.db.clone());
    let provider_id = Uuid::new_v4().to_string();
    let mut provider: ProviderConfig = bson::from_document(doc! {
        "_id": &provider_id,
        "slug": "twitter",
        "name": "X",
        "provider_type": "oauth2",
        "is_active": true,
        "authorization_url": "https://x.com/i/oauth2/authorize",
        "token_url": "https://api.x.com/2/oauth2/token",
        "supports_pkce": true,
        "credential_mode": "user",
        "default_scopes": X_REQUIRED_SCOPES,
        "created_by": &f.admin,
        "created_at": bson::DateTime::now(),
        "updated_at": bson::DateTime::now(),
    })
    .unwrap();
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
    f.db.collection(PROVIDERS)
        .insert_one(provider)
        .await
        .unwrap();

    let internal_connection_id = Uuid::new_v4().to_string();
    let key_id = Uuid::new_v4().to_string();
    let mut key: UserApiKey = bson::from_document(doc! {
        "_id": &key_id,
        "user_id": &f.owner,
        "label": "X account",
        "credential_type": "oauth2",
        "credential_source": "platform",
        "provider_config_id": &provider_id,
        "connection_id": &internal_connection_id,
        "source": "channel_onboarding",
        "source_id": &internal_connection_id,
        "status": "active",
        "token_scopes": X_REQUIRED_SCOPES.join(" "),
        "created_at": bson::DateTime::now(),
        "updated_at": bson::DateTime::now(),
    })
    .unwrap();
    key.access_token_encrypted = Some(
        state
            .encryption_keys
            .encrypt(b"live-x-access-token")
            .await
            .unwrap(),
    );
    key.refresh_token_encrypted = Some(
        state
            .encryption_keys
            .encrypt(b"rotating-x-refresh-token")
            .await
            .unwrap(),
    );
    f.db.collection::<UserApiKey>(USER_API_KEYS)
        .insert_one(&key)
        .await
        .unwrap();

    let bot: ChannelBot = bson::from_document(doc! {
        "_id": Uuid::new_v4().to_string(),
        "user_id": &f.owner,
        "platform": "x",
        "label": "Support on X",
        "credential_source": "connection",
        "connection_id": &key.id,
        "poll_cursor": "100",
        "bot_token_encrypted": bson::Binary {
            subtype: bson::spec::BinarySubtype::Generic,
            bytes: vec![],
        },
        "platform_bot_id": "10",
        "platform_bot_username": "support",
        "webhook_registered": false,
        "webhook_secret_hash": "",
        "status": "active",
        "is_active": true,
        "created_at": bson::DateTime::now(),
        "updated_at": bson::DateTime::now(),
    })
    .unwrap();
    f.db.collection::<ChannelBot>(BOTS)
        .insert_one(&bot)
        .await
        .unwrap();
    (bot, key)
}

#[tokio::test]
async fn x_channel_onboarding_transfer_moves_one_live_credential_and_rotates_callback_handle() {
    let f = fixture("ownership_x_oauth").await;
    f.db.collection::<Document>(USERS)
        .update_one(
            doc! { "_id": &f.owner },
            doc! { "$set": { "user_type": "org" } },
        )
        .await
        .unwrap();
    f.db.collection::<Document>(USERS)
        .update_one(
            doc! { "_id": &f.destination },
            doc! { "$set": { "user_type": "person" } },
        )
        .await
        .unwrap();
    let (bot, key) = insert_x_bot(&f).await;
    let state = test_utils::test_app_state(f.db.clone());
    let preview = preview(
        &f.db,
        &f.admin,
        ResourceKind::ChannelBot,
        &bot.id,
        &f.destination,
        5,
    )
    .await
    .unwrap();
    assert!(preview.blockers.is_empty(), "{:?}", preview.blockers);
    assert!(preview.moves_oauth_credential);
    assert_eq!(preview.destination_type, "person");

    let axum::Json(http_preview) = crate::handlers::admin_ownership::preview(
        axum::extract::State(state.clone()),
        test_utils::test_auth_user(&f.admin),
        axum::extract::Path((ResourceKind::ChannelBot, bot.id.clone())),
        axum::Json(crate::handlers::admin_ownership::PreviewRequest {
            new_owner_user_id: f.destination.clone(),
        }),
    )
    .await
    .unwrap();
    assert!(http_preview.effects.iter().any(|effect| {
        effect
            == "The bot's dedicated X connection moves to the new owner. The connected X account and existing token copies are unchanged."
    }));

    let request_id = Uuid::new_v4().to_string();
    move_resource(
        &f,
        ResourceKind::ChannelBot,
        &bot.id,
        &f.destination,
        &preview.version,
        &request_id,
    )
    .await
    .unwrap();

    let moved_bot =
        f.db.collection::<ChannelBot>(BOTS)
            .find_one(doc! { "_id": &bot.id })
            .await
            .unwrap()
            .unwrap();
    let moved_key =
        f.db.collection::<UserApiKey>(USER_API_KEYS)
            .find_one(doc! { "_id": &key.id })
            .await
            .unwrap()
            .unwrap();
    assert_eq!(moved_bot.user_id, f.destination);
    assert_eq!(moved_bot.connection_id.as_deref(), Some(key.id.as_str()));
    assert_eq!(moved_key.id, key.id);
    assert_eq!(moved_key.user_id, f.destination);
    assert_eq!(moved_key.access_token_encrypted, key.access_token_encrypted);
    assert_eq!(
        moved_key.refresh_token_encrypted,
        key.refresh_token_encrypted
    );
    assert_ne!(moved_key.connection_id, key.connection_id);
    assert_eq!(moved_key.connection_id, moved_key.source_id);
    assert_eq!(moved_key.credential_epoch, key.credential_epoch);

    let token = channel_credentials::connection_token(
        &f.db,
        &state.encryption_keys,
        &f.destination,
        &key.id,
        "twitter",
        X_REQUIRED_SCOPES,
    )
    .await
    .unwrap();
    assert_eq!(token.as_str(), "live-x-access-token");
    assert!(
        channel_credentials::connection_token(
            &f.db,
            &state.encryption_keys,
            &f.owner,
            &key.id,
            "twitter",
            X_REQUIRED_SCOPES,
        )
        .await
        .is_err()
    );

    assert!(
        crate::services::user_api_key_service::write_oauth_tokens_to_key(
            &f.db,
            &state.encryption_keys,
            key.connection_id.as_deref().unwrap(),
            "stale-callback-access",
            Some("stale-callback-refresh"),
            Some(&X_REQUIRED_SCOPES.join(" ")),
            None,
        )
        .await
        .is_err()
    );
    let stale_refresh = crate::services::user_token_service::refresh_user_api_key_in_place(
        &f.db,
        &state.encryption_keys,
        &key,
        None,
    )
    .await
    .unwrap();
    assert_eq!(stale_refresh.user_id, f.destination);
    assert_eq!(
        stale_refresh.refresh_token_encrypted,
        key.refresh_token_encrypted
    );

    let oauth = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path("/token"))
        .respond_with(
            wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token": "destination-access-token",
                "refresh_token": "destination-refresh-token",
                "token_type": "Bearer",
                "expires_in": 3600,
            })),
        )
        .expect(1)
        .mount(&oauth)
        .await;
    f.db.collection::<Document>(PROVIDERS)
        .update_one(
            doc! {"_id": &key.provider_config_id},
            doc! {"$set": {"token_url": format!("{}/token", oauth.uri())}},
        )
        .await
        .unwrap();
    let refreshed = crate::services::user_token_service::refresh_user_api_key_in_place(
        &f.db,
        &state.encryption_keys,
        &moved_key,
        None,
    )
    .await
    .unwrap();
    assert_eq!(refreshed.user_id, f.destination);
    assert_eq!(refreshed.credential_epoch, key.credential_epoch);
    assert_eq!(refreshed.connection_id, moved_key.connection_id);
    assert_eq!(
        channel_credentials::connection_token(
            &f.db,
            &state.encryption_keys,
            &f.destination,
            &key.id,
            "twitter",
            X_REQUIRED_SCOPES
        )
        .await
        .unwrap()
        .as_str(),
        "destination-access-token"
    );
    assert_eq!(
        f.db.collection::<Document>(USER_API_KEYS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );

    let replay = move_resource(
        &f,
        ResourceKind::ChannelBot,
        &bot.id,
        &f.destination,
        &preview.version,
        &request_id,
    )
    .await
    .unwrap();
    assert_eq!(replay.id, request_id);
    let replayed_key =
        f.db.collection::<UserApiKey>(USER_API_KEYS)
            .find_one(doc! { "_id": &key.id })
            .await
            .unwrap()
            .unwrap();
    assert_eq!(replayed_key.connection_id, moved_key.connection_id);
}

async fn assert_x_transfer_blocked(f: &Fixture, bot: &ChannelBot, reason: &str) {
    let reviewed = preview(
        &f.db,
        &f.admin,
        ResourceKind::ChannelBot,
        &bot.id,
        &f.destination,
        5,
    )
    .await
    .unwrap();
    assert!(
        reviewed
            .blockers
            .iter()
            .any(|blocker| blocker.contains(reason)),
        "expected {reason}: {:?}",
        reviewed.blockers
    );
    assert!(matches!(
        move_resource(
            f,
            ResourceKind::ChannelBot,
            &bot.id,
            &f.destination,
            &reviewed.version,
            &Uuid::new_v4().to_string()
        )
        .await,
        Err(AppError::Conflict(_))
    ));
    let current =
        f.db.collection::<ChannelBot>(BOTS)
            .find_one(doc! {"_id": &bot.id})
            .await
            .unwrap()
            .unwrap();
    assert_eq!(current.user_id, bot.user_id);
    assert_eq!(
        f.db.collection::<Document>(TRANSFERS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn x_transfer_rejects_invalid_or_pending_credentials_without_partial_moves() {
    let f = fixture("ownership_x_invalid").await;
    let (bot, key) = insert_x_bot(&f).await;
    let original =
        f.db.collection::<Document>(USER_API_KEYS)
            .find_one(doc! {"_id": &key.id})
            .await
            .unwrap()
            .unwrap();
    for (change, reason) in [
        (doc! {"status": "pending_auth"}, "not active"),
        (doc! {"user_id": &f.destination}, "different owner"),
        (doc! {"source": "provider_token"}, "dedicated credential"),
        (doc! {"credential_source": "byo"}, "not a NyxID-managed"),
        (doc! {"refresh_token_encrypted": null}, "refresh access"),
        (
            doc! {"token_scopes": "users.read"},
            "required channel permissions",
        ),
        (
            doc! {"provider_config_id": Uuid::new_v4().to_string()},
            "provider is missing",
        ),
        (
            doc! {"source_id": Uuid::new_v4().to_string()},
            "inconsistent onboarding",
        ),
    ] {
        f.db.collection::<Document>(USER_API_KEYS)
            .update_one(doc! {"_id": &key.id}, doc! {"$set": change})
            .await
            .unwrap();
        let changed =
            f.db.collection::<Document>(USER_API_KEYS)
                .find_one(doc! {"_id": &key.id})
                .await
                .unwrap()
                .unwrap();
        assert_x_transfer_blocked(&f, &bot, reason).await;
        assert_eq!(
            f.db.collection::<Document>(USER_API_KEYS)
                .find_one(doc! {"_id": &key.id})
                .await
                .unwrap()
                .unwrap(),
            changed
        );
        f.db.collection::<Document>(USER_API_KEYS)
            .replace_one(doc! {"_id": &key.id}, &original)
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn x_public_event_transfer_requires_public_scopes_and_preserves_selected_events() {
    use crate::models::channel_bot::XChannelEvent;
    use crate::services::channel_adapters::x::PUBLIC_SCOPES;

    let f = fixture("ownership_x_public_events").await;
    let (bot, key) = insert_x_bot(&f).await;
    f.db.collection::<Document>(BOTS)
        .update_one(
            doc! { "_id": &bot.id },
            doc! { "$set": { "x_events": ["dm", "mentions", "replies"] } },
        )
        .await
        .unwrap();
    assert_x_transfer_blocked(&f, &bot, "required channel permissions").await;
    f.db.collection::<Document>(USER_API_KEYS)
        .update_one(
            doc! { "_id": &key.id },
            doc! { "$set": { "token_scopes": PUBLIC_SCOPES.join(" ") } },
        )
        .await
        .unwrap();
    let reviewed = preview(
        &f.db,
        &f.admin,
        ResourceKind::ChannelBot,
        &bot.id,
        &f.destination,
        5,
    )
    .await
    .unwrap();
    assert!(reviewed.blockers.is_empty(), "{:?}", reviewed.blockers);
    move_resource(
        &f,
        ResourceKind::ChannelBot,
        &bot.id,
        &f.destination,
        &reviewed.version,
        &Uuid::new_v4().to_string(),
    )
    .await
    .unwrap();
    let moved =
        f.db.collection::<ChannelBot>(BOTS)
            .find_one(doc! { "_id": &bot.id })
            .await
            .unwrap()
            .unwrap();
    assert_eq!(moved.user_id, f.destination);
    assert_eq!(
        moved.x_events,
        Some(vec![
            XChannelEvent::Dm,
            XChannelEvent::Mentions,
            XChannelEvent::Replies
        ])
    );
}

#[tokio::test]
async fn x_transfer_versions_shared_and_inflight_dependencies_including_inactive_consumers() {
    let f = fixture("ownership_x_dependencies").await;
    let (bot, key) = insert_x_bot(&f).await;
    let clean = preview(
        &f.db,
        &f.admin,
        ResourceKind::ChannelBot,
        &bot.id,
        &f.destination,
        5,
    )
    .await
    .unwrap();
    for (collection, mut dependency, reason) in [
        (
            USER_SERVICES,
            doc! {"api_key_id": &key.id, "is_active": false},
            "AI service",
        ),
        (
            AGENT_BINDINGS,
            doc! {"user_api_key_id": &key.id},
            "agent service binding",
        ),
        (
            BOTS,
            doc! {"connection_id": &key.id, "is_active": false},
            "another channel bot",
        ),
        (
            USER_PROVIDER_TOKENS,
            doc! {"connection_id": &key.connection_id},
            "legacy provider connection",
        ),
        (
            OAUTH_STATES,
            doc! {"connection_id": &key.connection_id, "consumed": false,
            "expires_at": bson::DateTime::from_chrono(Utc::now() + chrono::Duration::minutes(10))},
            "authorization is still",
        ),
        (
            OAUTH_STATES,
            doc! {"connection_id": &key.connection_id, "consumed": true,
            "expires_at": bson::DateTime::from_chrono(Utc::now() + chrono::Duration::minutes(10))},
            "authorization is still",
        ),
    ] {
        let id = Uuid::new_v4().to_string();
        dependency.insert("_id", &id);
        dependency.insert("user_id", &f.owner);
        f.db.collection::<Document>(collection)
            .insert_one(dependency)
            .await
            .unwrap();
        let changed = preview(
            &f.db,
            &f.admin,
            ResourceKind::ChannelBot,
            &bot.id,
            &f.destination,
            5,
        )
        .await
        .unwrap();
        assert_ne!(clean.version, changed.version);
        assert!(matches!(
            move_resource(
                &f,
                ResourceKind::ChannelBot,
                &bot.id,
                &f.destination,
                &clean.version,
                &Uuid::new_v4().to_string()
            )
            .await,
            Err(AppError::Conflict(_))
        ));
        assert_x_transfer_blocked(&f, &bot, reason).await;
        f.db.collection::<Document>(collection)
            .delete_one(doc! {"_id": id})
            .await
            .unwrap();
    }
    let runtime = crate::services::coordination_service::cluster_lease_runtime();
    let lease = runtime
        .acquire(
            &f.db,
            &crate::services::user_token_service::user_api_key_refresh_lease_name(&key.id),
        )
        .await
        .unwrap()
        .unwrap();
    assert_x_transfer_blocked(&f, &bot, "refreshing").await;
    crate::services::coordination_service::LeaseStore::release(&f.db, &lease)
        .await
        .unwrap();
    let ready = preview(
        &f.db,
        &f.admin,
        ResourceKind::ChannelBot,
        &bot.id,
        &f.destination,
        5,
    )
    .await
    .unwrap();
    assert!(ready.blockers.is_empty());
    move_resource(
        &f,
        ResourceKind::ChannelBot,
        &bot.id,
        &f.destination,
        &ready.version,
        &Uuid::new_v4().to_string(),
    )
    .await
    .unwrap();
}

#[tokio::test]
async fn x_transfer_rejects_stale_owner_service_references_and_preserves_missing_legacy_backing() {
    let f = fixture("ownership_x_stale_reference").await;
    let (bot, key) = insert_x_bot(&f).await;
    let reviewed = preview(
        &f.db,
        &f.admin,
        ResourceKind::ChannelBot,
        &bot.id,
        &f.destination,
        5,
    )
    .await
    .unwrap();
    move_resource(
        &f,
        ResourceKind::ChannelBot,
        &bot.id,
        &f.destination,
        &reviewed.version,
        &Uuid::new_v4().to_string(),
    )
    .await
    .unwrap();
    let mut service = test_utils::test_user_service(
        &Uuid::new_v4().to_string(),
        &f.owner,
        "stale-owner",
        &Uuid::new_v4().to_string(),
        None,
        None,
    );
    service.api_key_id = Some(key.id.clone());
    let result = crate::services::service_history::collection::<UserService>(&f.db, USER_SERVICES)
        .insert_one(&service)
        .await;
    assert!(result.is_err());
    assert!(
        f.db.collection::<Document>(USER_SERVICES)
            .find_one(doc! {"_id": &service.id})
            .await
            .unwrap()
            .is_none()
    );
    service.api_key_id = Some(Uuid::new_v4().to_string());
    crate::services::service_history::collection::<UserService>(&f.db, USER_SERVICES)
        .insert_one(&service)
        .await
        .unwrap();
}

#[tokio::test]
async fn x_transfer_and_new_bot_reference_cannot_both_commit() {
    let f = fixture("ownership_x_concurrent_reference").await;
    let (bot, _) = insert_x_bot(&f).await;
    let reviewed = preview(
        &f.db,
        &f.admin,
        ResourceKind::ChannelBot,
        &bot.id,
        &f.destination,
        5,
    )
    .await
    .unwrap();
    let mut other = bot.clone();
    other.id = Uuid::new_v4().to_string();
    other.platform_bot_id = "another-account".into();
    let request = Uuid::new_v4().to_string();
    let (moved, attached) = tokio::join!(
        move_resource(
            &f,
            ResourceKind::ChannelBot,
            &bot.id,
            &f.destination,
            &reviewed.version,
            &request
        ),
        crate::services::channel_bot_service::insert_registered_bot(&f.db, &other, 5, None),
    );
    assert_ne!(
        moved.is_ok(),
        attached.is_ok(),
        "move={moved:?}; attach={attached:?}"
    );
    let dangling =
        f.db.collection::<Document>(BOTS)
            .find_one(doc! {"_id": &other.id})
            .await
            .unwrap();
    assert_eq!(dangling.is_some(), attached.is_ok());
}

#[tokio::test]
async fn owner_can_preview_but_admin_override_requires_live_platform_role() {
    let f = fixture("ownership_admin_only").await;
    f.db.collection::<crate::models::org_membership::OrgMembership>(
        crate::models::org_membership::COLLECTION_NAME,
    )
    .insert_one(test_utils::test_membership(
        &f.destination,
        &f.owner,
        OrgRole::Admin,
        None,
    ))
    .await
    .unwrap();
    preview(
        &f.db,
        &f.owner,
        ResourceKind::Service,
        &f.service.id,
        &f.destination,
        5,
    )
    .await
    .unwrap();
    for actor in [&f.destination] {
        assert!(matches!(
            preview(
                &f.db,
                actor,
                ResourceKind::Service,
                &f.service.id,
                &f.destination,
                5
            )
            .await,
            Err(crate::errors::AppError::Forbidden(_))
        ));
    }
    let operator = role_service::get_platform_role_ids(&f.db)
        .await
        .unwrap()
        .operator;
    f.db.collection::<Document>(USERS)
        .update_one(
            doc! { "_id": &f.owner },
            doc! { "$set": { "role_ids": [operator] } },
        )
        .await
        .unwrap();
    assert!(require_platform_admin(&f.db, &f.owner).await.is_err());
    let p = preview(
        &f.db,
        &f.admin,
        ResourceKind::Service,
        &f.service.id,
        &f.destination,
        5,
    )
    .await
    .unwrap();
    f.db.collection::<Document>(USERS)
        .update_one(
            doc! { "_id": &f.admin },
            doc! { "$set": { "role_ids": [] } },
        )
        .await
        .unwrap();
    assert!(
        move_resource(
            &f,
            ResourceKind::Service,
            &f.service.id,
            &f.destination,
            &p.version,
            &Uuid::new_v4().to_string()
        )
        .await
        .is_err()
    );
    assert_eq!(
        f.db.collection::<DownstreamService>(SERVICES)
            .find_one(doc! { "_id": &f.service.id })
            .await
            .unwrap()
            .unwrap()
            .owner_user_id,
        None
    );
}

#[tokio::test]
async fn catalog_transfer_preserves_creator_consumers_and_configuration_and_updates_access() {
    let f = fixture("ownership_catalog").await;
    let before =
        f.db.collection::<Document>(SERVICES)
            .find_one(doc! { "_id": &f.service.id })
            .await
            .unwrap()
            .unwrap();
    let connection = doc! { "_id": Uuid::new_v4().to_string(), "user_id": &f.owner, "catalog_service_id": &f.service.id, "is_active": false };
    f.db.collection::<Document>("user_services")
        .insert_one(&connection)
        .await
        .unwrap();
    let p = preview(
        &f.db,
        &f.admin,
        ResourceKind::Service,
        &f.service.id,
        &f.destination,
        5,
    )
    .await
    .unwrap();
    assert!(p.blockers.is_empty(), "{:?}", p.blockers);
    let request = Uuid::new_v4().to_string();
    let receipt = move_resource(
        &f,
        ResourceKind::Service,
        &f.service.id,
        &f.destination,
        &p.version,
        &request,
    )
    .await
    .unwrap();
    assert_eq!(receipt.previous_owner_user_id, f.owner);
    let mut after =
        f.db.collection::<Document>(SERVICES)
            .find_one(doc! { "_id": &f.service.id })
            .await
            .unwrap()
            .unwrap();
    assert_eq!(
        after.remove("owner_user_id").unwrap().as_str(),
        Some(f.destination.as_str())
    );
    after.insert("updated_at", before.get("updated_at").unwrap().clone());
    assert_eq!(before, after);
    assert_eq!(
        f.db.collection::<Document>("user_services")
            .find_one(doc! { "_id": connection.get_str("_id").unwrap() })
            .await
            .unwrap()
            .unwrap(),
        connection
    );
    assert!(
        catalog_service::get_downstream_service_by_slug(&f.db, &f.service.slug, &f.owner)
            .await
            .is_err()
    );
    assert!(
        catalog_service::get_downstream_service_by_slug(&f.db, &f.service.slug, &f.destination)
            .await
            .is_ok()
    );
    let saved =
        f.db.collection::<DownstreamService>(SERVICES)
            .find_one(doc! { "_id": &f.service.id })
            .await
            .unwrap()
            .unwrap();
    let state = test_utils::test_app_state(f.db.clone());
    assert!(
        crate::handlers::services_helpers::require_admin_or_creator(
            &state,
            &test_utils::test_auth_user(&f.owner),
            &saved
        )
        .await
        .is_err()
    );
    assert!(
        crate::handlers::services_helpers::require_admin_or_creator(
            &state,
            &test_utils::test_auth_user(&f.admin),
            &saved
        )
        .await
        .is_ok()
    );
    let replay = move_resource(
        &f,
        ResourceKind::Service,
        &f.service.id,
        &f.destination,
        &p.version,
        &request,
    )
    .await
    .unwrap();
    assert_eq!(receipt.id, replay.id);
    assert_eq!(
        f.db.collection::<Document>(TRANSFERS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
}

#[tokio::test]
async fn stale_preview_and_inactive_destination_leave_every_record_unchanged() {
    let f = fixture("ownership_conflicts").await;
    let p = preview(
        &f.db,
        &f.admin,
        ResourceKind::Service,
        &f.service.id,
        &f.destination,
        5,
    )
    .await
    .unwrap();
    f.db.collection::<Document>(SERVICES)
        .update_one(
            doc! { "_id": &f.service.id },
            doc! { "$set": { "name": "Edited concurrently" } },
        )
        .await
        .unwrap();
    assert!(matches!(
        move_resource(
            &f,
            ResourceKind::Service,
            &f.service.id,
            &f.destination,
            &p.version,
            &Uuid::new_v4().to_string()
        )
        .await,
        Err(crate::errors::AppError::Conflict(_))
    ));
    let p = preview(
        &f.db,
        &f.admin,
        ResourceKind::Service,
        &f.service.id,
        &f.destination,
        5,
    )
    .await
    .unwrap();
    f.db.collection::<Document>(USERS)
        .update_one(
            doc! { "_id": &f.destination },
            doc! { "$set": { "is_active": false } },
        )
        .await
        .unwrap();
    assert!(
        move_resource(
            &f,
            ResourceKind::Service,
            &f.service.id,
            &f.destination,
            &p.version,
            &Uuid::new_v4().to_string()
        )
        .await
        .is_err()
    );
    assert_eq!(
        f.db.collection::<Document>(TRANSFERS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    assert_eq!(
        f.db.collection::<DownstreamService>(SERVICES)
            .find_one(doc! { "_id": &f.service.id })
            .await
            .unwrap()
            .unwrap()
            .owner_user_id,
        None
    );
}

#[tokio::test]
async fn bot_transfer_retires_routes_without_moving_history_or_agent_keys() {
    let f = fixture("ownership_bot").await;
    let bot = insert_bot(&f).await;
    f.db.collection::<Document>(BOTS).update_one(doc! { "_id": &bot.id },
        doc! { "$set": { "updated_at": bson::DateTime::from_millis(bot.updated_at.timestamp_millis() + 1), "status": "active" } })
        .await.unwrap();
    assert!(require_current_bot(&f.db, &bot).await.is_ok());
    let conversation = Uuid::new_v4().to_string();
    let agent = Uuid::new_v4().to_string();
    let now = bson::DateTime::now();
    f.db.collection::<Document>(CONVERSATIONS).insert_one(doc! { "_id": &conversation, "channel_bot_id": &bot.id,
        "user_id": &f.owner, "platform": "telegram", "platform_conversation_id": "chat", "platform_conversation_type": "private",
        "agent_api_key_id": &agent, "is_active": true, "default_agent": true, "allow_agent_initiated": true, "created_at": now, "updated_at": now }).await.unwrap();
    let message = doc! { "_id": Uuid::new_v4().to_string(), "user_id": &f.owner, "conversation_id": &conversation, "channel_bot_id": &bot.id };
    f.db.collection::<Document>("channel_messages")
        .insert_one(&message)
        .await
        .unwrap();
    let p = preview(
        &f.db,
        &f.admin,
        ResourceKind::ChannelBot,
        &bot.id,
        &f.destination,
        5,
    )
    .await
    .unwrap();
    assert_eq!(p.routes_to_retire, 1);
    let receipt = move_resource(
        &f,
        ResourceKind::ChannelBot,
        &bot.id,
        &f.destination,
        &p.version,
        &Uuid::new_v4().to_string(),
    )
    .await
    .unwrap();
    assert_eq!(receipt.retired_routes, 1);
    let saved =
        f.db.collection::<ChannelBot>(BOTS)
            .find_one(doc! { "_id": &bot.id })
            .await
            .unwrap()
            .unwrap();
    assert_eq!(saved.user_id, f.destination);
    assert_eq!(saved.ownership_version, 1);
    assert_eq!(saved.bot_token_encrypted, bot.bot_token_encrypted);
    assert_eq!(saved.webhook_secret_hash, bot.webhook_secret_hash);
    assert!(require_current_bot(&f.db, &bot).await.is_err());
    assert!(
        channel_routing_service::resolve_agent(&f.db, &bot.id, "chat", None, &f.destination)
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        channel_routing_service::update_conversation(
            &f.db,
            &conversation,
            &f.owner,
            None,
            None,
            Some(true),
            None
        )
        .await
        .is_err()
    );
    let route =
        f.db.collection::<Document>(CONVERSATIONS)
            .find_one(doc! { "_id": &conversation })
            .await
            .unwrap()
            .unwrap();
    assert_eq!(route.get_str("user_id").unwrap(), f.owner);
    assert!(!route.get_bool("is_active").unwrap());
    assert_eq!(route.get_str("agent_api_key_id").unwrap(), agent);
    assert_eq!(
        f.db.collection::<Document>("channel_messages")
            .find_one(doc! { "conversation_id": &conversation })
            .await
            .unwrap()
            .unwrap(),
        message
    );
    let destination_agent = Uuid::new_v4().to_string();
    f.db.collection::<Document>("api_keys")
        .insert_one(doc! {
            "_id": &destination_agent, "user_id": &f.destination, "name": "Destination agent",
            "key_prefix": "nyxid_ag", "key_hash": "test", "scopes": "read write",
            "is_active": true, "created_at": now, "callback_url": "https://agent.example/callback",
        })
        .await
        .unwrap();
    let new_route = channel_routing_service::create_conversation(
        &f.db,
        &f.destination,
        Some(&bot.id),
        "telegram",
        "chat",
        "private",
        None,
        &destination_agent,
        false,
        false,
    )
    .await
    .unwrap();
    let resolved =
        channel_routing_service::resolve_agent(&f.db, &bot.id, "chat", None, &f.destination)
            .await
            .unwrap()
            .unwrap();
    assert_eq!(resolved.conversation.id, new_route.id);
    assert_eq!(resolved.api_key_id, destination_agent);
    assert!(
        channel_routing_service::resolve_agent(&f.db, &bot.id, "chat", None, &f.owner)
            .await
            .unwrap()
            .is_none()
    );
    // Moving back cannot revive any retired route or old reply authorization.
    let p = preview(
        &f.db,
        &f.admin,
        ResourceKind::ChannelBot,
        &bot.id,
        &f.owner,
        5,
    )
    .await
    .unwrap();
    move_resource(
        &f,
        ResourceKind::ChannelBot,
        &bot.id,
        &f.owner,
        &p.version,
        &Uuid::new_v4().to_string(),
    )
    .await
    .unwrap();
    assert!(require_current_bot(&f.db, &bot).await.is_err());
    assert!(require_current_bot(&f.db, &saved).await.is_err());
    assert!(
        channel_routing_service::update_conversation(
            &f.db,
            &conversation,
            &f.owner,
            None,
            None,
            Some(true),
            None
        )
        .await
        .is_err()
    );
}

#[tokio::test]
async fn unsupported_dependencies_and_capacity_block_before_mutation() {
    let f = fixture("ownership_blockers").await;
    let bot = insert_bot(&f).await;
    for set in [
        doc! { "connection_id": Uuid::new_v4().to_string() },
        doc! { "platform": "aurinko" },
        doc! { "platform": "telegram-new" },
    ] {
        f.db.collection::<Document>(BOTS)
            .update_one(doc! { "_id": &bot.id }, doc! { "$set": set })
            .await
            .unwrap();
        let p = preview(
            &f.db,
            &f.admin,
            ResourceKind::ChannelBot,
            &bot.id,
            &f.destination,
            5,
        )
        .await
        .unwrap();
        assert!(!p.blockers.is_empty());
        assert!(
            move_resource(
                &f,
                ResourceKind::ChannelBot,
                &bot.id,
                &f.destination,
                &p.version,
                &Uuid::new_v4().to_string()
            )
            .await
            .is_err()
        );
    }
    for set in [
        doc! { "created_by": "system" },
        doc! { "created_by": &f.owner, "oauth_client_id": "client" },
        doc! { "oauth_client_id": null, "service_type": "ssh" },
    ] {
        f.db.collection::<Document>(SERVICES)
            .update_one(doc! { "_id": &f.service.id }, doc! { "$set": set })
            .await
            .unwrap();
        let p = preview(
            &f.db,
            &f.admin,
            ResourceKind::Service,
            &f.service.id,
            &f.destination,
            5,
        )
        .await
        .unwrap();
        assert!(!p.blockers.is_empty());
        assert!(
            move_resource(
                &f,
                ResourceKind::Service,
                &f.service.id,
                &f.destination,
                &p.version,
                &Uuid::new_v4().to_string()
            )
            .await
            .is_err()
        );
    }
    let p = preview(
        &f.db,
        &f.admin,
        ResourceKind::ChannelBot,
        &bot.id,
        &f.destination,
        0,
    )
    .await
    .unwrap();
    assert!(p.blockers.iter().any(|b| b.contains("limit")));
    assert_eq!(
        f.db.collection::<Document>(TRANSFERS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
}

#[tokio::test]
async fn concurrent_transfers_have_one_winner_and_receipts_cannot_be_repurposed() {
    let f = fixture("ownership_concurrent").await;
    let p = preview(
        &f.db,
        &f.admin,
        ResourceKind::Service,
        &f.service.id,
        &f.destination,
        5,
    )
    .await
    .unwrap();
    let first = Uuid::new_v4().to_string();
    let second = Uuid::new_v4().to_string();
    let (a, b) = tokio::join!(
        move_resource(
            &f,
            ResourceKind::Service,
            &f.service.id,
            &f.destination,
            &p.version,
            &first
        ),
        move_resource(
            &f,
            ResourceKind::Service,
            &f.service.id,
            &f.destination,
            &p.version,
            &second
        )
    );
    assert_ne!(a.is_ok(), b.is_ok());
    let winner = a.or(b).unwrap();
    assert!(
        move_resource(
            &f,
            ResourceKind::Service,
            &f.service.id,
            &f.owner,
            &p.version,
            &winner.id
        )
        .await
        .is_err()
    );
    assert_eq!(
        f.db.collection::<Document>(TRANSFERS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
}

#[tokio::test]
async fn private_catalog_discovery_follows_destination_org_membership() {
    let f = fixture("ownership_discovery").await;
    let member = Uuid::new_v4().to_string();
    f.db.collection::<User>(USERS)
        .insert_one(test_utils::test_user(&member, UserType::Person))
        .await
        .unwrap();
    let membership = test_utils::test_membership(&f.destination, &member, OrgRole::Viewer, None);
    f.db.collection::<crate::models::org_membership::OrgMembership>("org_memberships")
        .insert_one(&membership)
        .await
        .unwrap();
    let p = preview(
        &f.db,
        &f.admin,
        ResourceKind::Service,
        &f.service.id,
        &f.destination,
        5,
    )
    .await
    .unwrap();
    move_resource(
        &f,
        ResourceKind::Service,
        &f.service.id,
        &f.destination,
        &p.version,
        &Uuid::new_v4().to_string(),
    )
    .await
    .unwrap();
    let state = test_utils::test_app_state(f.db.clone());
    let entries = catalog_service::list_catalog_all(&f.db, &state.encryption_keys, &member)
        .await
        .unwrap();
    assert!(entries.iter().any(|e| e.slug == f.service.slug));
    assert!(
        catalog_service::get_downstream_service_by_slug(&f.db, &f.service.slug, &member)
            .await
            .is_ok()
    );
    f.db.collection::<Document>("org_memberships")
        .update_one(
            doc! { "_id": &membership.id },
            doc! { "$set": { "revoked_at": bson::DateTime::from_chrono(Utc::now()) } },
        )
        .await
        .unwrap();
    assert!(
        catalog_service::list_catalog_all(&f.db, &state.encryption_keys, &member)
            .await
            .unwrap()
            .iter()
            .all(|e| e.slug != f.service.slug)
    );
    assert!(
        catalog_service::get_downstream_service_by_slug(&f.db, &f.service.slug, &member)
            .await
            .is_err()
    );
}

#[tokio::test]
async fn admin_handlers_preview_without_writes_and_never_serialize_credentials() {
    use crate::handlers::admin_ownership as handlers;
    use axum::{
        Json,
        extract::{Path, Query, State},
    };
    let f = fixture("ownership_handlers").await;
    let bot = insert_bot(&f).await;
    let state = test_utils::test_app_state(f.db.clone());
    let Json(authorization) = handlers::authorization(
        State(state.clone()),
        test_utils::test_auth_user(&f.owner),
        Path((ResourceKind::ChannelBot, bot.id.clone())),
    )
    .await
    .unwrap();
    assert!(authorization.can_transfer);
    let owner_metadata = serde_json::to_value(authorization).unwrap();
    assert_eq!(owner_metadata["resource"]["owner_user_id"], f.owner);
    assert!(!owner_metadata.to_string().contains("encrypted"));
    assert!(!owner_metadata.to_string().contains("secret"));
    let response = handlers::list_resources(
        State(state.clone()),
        test_utils::test_auth_user(&f.admin),
        Path(ResourceKind::ChannelBot),
        Query(handlers::ResourceQuery {
            owner_user_id: None,
            search: None,
            offset: 0,
        }),
    )
    .await
    .unwrap();
    let value = serde_json::to_value(response.0).unwrap();
    assert_eq!(value["items"][0]["id"], bot.id);
    let serialized = value.to_string();
    assert!(!serialized.contains("encrypted"));
    assert!(!serialized.contains("secret"));
    let before =
        f.db.collection::<Document>(BOTS)
            .find_one(doc! { "_id": &bot.id })
            .await
            .unwrap()
            .unwrap();
    let Json(p) = handlers::preview(
        State(state.clone()),
        test_utils::test_auth_user(&f.admin),
        Path((ResourceKind::ChannelBot, bot.id.clone())),
        Json(handlers::PreviewRequest {
            new_owner_user_id: f.destination.clone(),
        }),
    )
    .await
    .unwrap();
    assert_eq!(
        f.db.collection::<Document>(BOTS)
            .find_one(doc! { "_id": &bot.id })
            .await
            .unwrap()
            .unwrap(),
        before
    );
    assert_eq!(
        f.db.collection::<Document>(TRANSFERS)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    let Json(result) = handlers::transfer(
        State(state.clone()),
        test_utils::test_auth_user(&f.admin),
        Path((ResourceKind::ChannelBot, bot.id.clone())),
        Json(handlers::TransferRequest {
            new_owner_user_id: f.destination.clone(),
            request_id: Uuid::new_v4().to_string(),
            expected_version: p.version,
        }),
    )
    .await
    .unwrap();
    assert_eq!(result.new_owner_user_id, f.destination);
    let audit =
        f.db.collection::<Document>(crate::models::audit_log::COLLECTION_NAME)
            .find_one(doc! { "event_type": "admin_ownership_transferred" })
            .await
            .unwrap();
    assert!(audit.is_some());
    assert!(
        handlers::list_resources(
            State(state),
            test_utils::test_auth_user(&f.owner),
            Path(ResourceKind::ChannelBot),
            Query(handlers::ResourceQuery {
                owner_user_id: None,
                search: None,
                offset: 0
            })
        )
        .await
        .is_err()
    );
}

#[tokio::test]
async fn missing_or_same_destination_and_invalid_requests_cannot_transfer() {
    let f = fixture("ownership_validation").await;
    assert!(
        preview(
            &f.db,
            &f.admin,
            ResourceKind::Service,
            &f.service.id,
            "invalid",
            5
        )
        .await
        .is_err()
    );
    assert!(
        preview(
            &f.db,
            &f.admin,
            ResourceKind::Service,
            &f.service.id,
            &Uuid::new_v4().to_string(),
            5
        )
        .await
        .is_err()
    );
    let p = preview(
        &f.db,
        &f.admin,
        ResourceKind::Service,
        &f.service.id,
        &f.owner,
        5,
    )
    .await
    .unwrap();
    assert!(p.blockers.iter().any(|b| b.contains("already belongs")));
    assert!(
        move_resource(
            &f,
            ResourceKind::Service,
            &f.service.id,
            &f.owner,
            &p.version,
            &Uuid::new_v4().to_string()
        )
        .await
        .is_err()
    );
    assert!(
        move_resource(
            &f,
            ResourceKind::Service,
            &f.service.id,
            &f.destination,
            "",
            &Uuid::new_v4().to_string()
        )
        .await
        .is_err()
    );
    assert!(serde_json::from_value::<crate::handlers::admin_ownership::TransferRequest>(serde_json::json!({
        "new_owner_user_id": f.destination, "request_id": Uuid::new_v4(), "expected_version": p.version,
        "force": true,
    })).is_err());
}

#[tokio::test]
async fn concurrent_registration_and_transfer_share_destination_capacity() {
    let f = fixture("ownership_capacity").await;
    let bot = insert_bot(&f).await;
    let mut registration = bot.clone();
    registration.id = Uuid::new_v4().to_string();
    registration.platform = "discord".into();
    registration.platform_bot_id = "new-remote-bot".into();
    registration.user_id = f.destination.clone();
    let p = preview(
        &f.db,
        &f.admin,
        ResourceKind::ChannelBot,
        &bot.id,
        &f.destination,
        1,
    )
    .await
    .unwrap();
    let request = Uuid::new_v4().to_string();
    let (moved, registered) = tokio::join!(
        transfer(
            &f.db,
            TransferCommand {
                actor: &f.admin,
                api_key_id: None,
                kind: ResourceKind::ChannelBot,
                resource_id: &bot.id,
                destination: &f.destination,
                request_id: &request,
                expected_version: &p.version,
                capacity: 1,
            }
        ),
        crate::services::channel_bot_service::insert_registered_bot(&f.db, &registration, 1, None),
    );
    assert_ne!(
        moved.is_ok(),
        registered.is_ok(),
        "exactly one operation should fit: {moved:?}, {registered:?}"
    );
    assert_eq!(
        f.db.collection::<Document>(BOTS)
            .count_documents(doc! {"user_id": &f.destination, "is_active": true})
            .await
            .unwrap(),
        1
    );
}

#[tokio::test]
async fn org_deletion_follows_current_catalog_owner_and_preserves_transferred_rows() {
    let f = fixture("ownership_org_cleanup").await;
    f.db.collection::<Document>(USERS)
        .update_one(doc! {"_id": &f.owner}, doc! {"$set": {"user_type": "org"}})
        .await
        .unwrap();
    let p = preview(
        &f.db,
        &f.admin,
        ResourceKind::Service,
        &f.service.id,
        &f.destination,
        5,
    )
    .await
    .unwrap();
    move_resource(
        &f,
        ResourceKind::Service,
        &f.service.id,
        &f.destination,
        &p.version,
        &Uuid::new_v4().to_string(),
    )
    .await
    .unwrap();
    assert!(
        crate::services::org_service::delete_org_user(&f.db, &f.destination)
            .await
            .is_err()
    );
    f.db.collection::<Document>(SERVICES)
        .update_one(
            doc! {"_id": &f.service.id},
            doc! {"$set": {"is_active": false}},
        )
        .await
        .unwrap();
    crate::services::org_service::delete_org_user(&f.db, &f.owner)
        .await
        .unwrap();
    assert!(
        f.db.collection::<Document>(SERVICES)
            .find_one(doc! {"_id": &f.service.id})
            .await
            .unwrap()
            .is_some()
    );
    crate::services::org_service::delete_org_user(&f.db, &f.destination)
        .await
        .unwrap();
    assert!(
        f.db.collection::<Document>(SERVICES)
            .find_one(doc! {"_id": &f.service.id})
            .await
            .unwrap()
            .is_none()
    );
}

#[tokio::test]
async fn route_creation_and_transfer_cannot_leave_an_active_previous_owner_route() {
    let f = fixture("ownership_route_race").await;
    let bot = insert_bot(&f).await;
    let agent = Uuid::new_v4().to_string();
    f.db.collection::<Document>("api_keys")
        .insert_one(doc! {
            "_id": &agent, "user_id": &f.owner, "name": "Source agent", "key_prefix": "nyxid_ag",
            "key_hash": "test", "scopes": "read write", "is_active": true,
            "created_at": bson::DateTime::now(), "callback_url": "https://agent.example/callback",
        })
        .await
        .unwrap();
    let p = preview(
        &f.db,
        &f.admin,
        ResourceKind::ChannelBot,
        &bot.id,
        &f.destination,
        5,
    )
    .await
    .unwrap();
    let request = Uuid::new_v4().to_string();
    let (moved, routed) = tokio::join!(
        move_resource(
            &f,
            ResourceKind::ChannelBot,
            &bot.id,
            &f.destination,
            &p.version,
            &request
        ),
        channel_routing_service::create_conversation(
            &f.db,
            &f.owner,
            Some(&bot.id),
            "telegram",
            "chat",
            "private",
            None,
            &agent,
            false,
            false
        ),
    );
    assert_ne!(
        moved.is_ok(),
        routed.is_ok(),
        "a new route must invalidate the preview: {moved:?}, {routed:?}"
    );
    if moved.is_err() {
        let p = preview(
            &f.db,
            &f.admin,
            ResourceKind::ChannelBot,
            &bot.id,
            &f.destination,
            5,
        )
        .await
        .unwrap();
        move_resource(
            &f,
            ResourceKind::ChannelBot,
            &bot.id,
            &f.destination,
            &p.version,
            &Uuid::new_v4().to_string(),
        )
        .await
        .unwrap();
    }
    assert_eq!(
        f.db.collection::<Document>(CONVERSATIONS)
            .count_documents(
                doc! {"channel_bot_id": &bot.id, "user_id": &f.owner, "is_active": true}
            )
            .await
            .unwrap(),
        0
    );
    assert!(
        channel_routing_service::create_conversation(
            &f.db,
            &f.owner,
            Some(&bot.id),
            "telegram",
            "late",
            "private",
            None,
            &agent,
            false,
            false
        )
        .await
        .is_err()
    );
}

use super::ownership_transfer_service::*;
use crate::{
    models::{
        channel_bot::{COLLECTION_NAME as BOTS, ChannelBot},
        channel_conversation::COLLECTION_NAME as CONVERSATIONS,
        downstream_service::{COLLECTION_NAME as SERVICES, DownstreamService},
        org_membership::OrgRole,
        ownership_transfer::COLLECTION_NAME as TRANSFERS,
        user::{COLLECTION_NAME as USERS, User, UserType},
    },
    services::{catalog_service, channel_routing_service, role_service},
    test_utils,
};
use bson::{Document, doc};
use chrono::Utc;
use mongodb::Database;
use uuid::Uuid;

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

#[tokio::test]
async fn transfers_require_live_platform_admin_not_owner_or_org_admin_or_operator() {
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
    for actor in [&f.owner, &f.destination] {
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
        channel_routing_service::resolve_agent(&f.db, &bot.id, &f.destination, "chat", None)
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
        channel_routing_service::resolve_agent(&f.db, &bot.id, &f.destination, "chat", None)
            .await
            .unwrap()
            .unwrap();
    assert_eq!(resolved.conversation.id, new_route.id);
    assert_eq!(resolved.api_key_id, destination_agent);
    assert!(
        channel_routing_service::resolve_agent(&f.db, &bot.id, &f.owner, "chat", None)
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

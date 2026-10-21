use super::*;
use crate::models::{
    assistant_agent_credential::COLLECTION_NAME as CHAT_CREDENTIALS,
    assistant_conversation::{AssistantConversation, COLLECTION_NAME as CONVERSATIONS},
    consent::{COLLECTION_NAME as CONSENTS, Consent},
    notification_channel::{COLLECTION_NAME as CHANNELS, NotificationChannel},
    oauth_client::COLLECTION_NAME as CLIENTS,
};
use mongodb::bson::{Document, from_document};

impl Fixture {
    async fn allow_service_without_approval(&self) {
        let now = mongodb::bson::DateTime::now();
        let config: ServiceApprovalConfig = from_document(doc! {
            "_id": Uuid::new_v4().to_string(), "user_id": &self.owner,
            "service_id": &self.connection, "service_name": "resource",
            "approval_required": false, "created_at": now, "updated_at": now,
        })
        .unwrap();
        self.db
            .collection::<ServiceApprovalConfig>(APPROVAL_CONFIGS)
            .insert_one(config)
            .await
            .unwrap();
    }

    async fn catalog_change(&self, fields: Document) {
        self.db
            .collection::<DownstreamService>(SERVICES)
            .update_one(doc! {"_id": &self.service}, doc! {"$set": fields})
            .await
            .unwrap();
    }

    async fn connection_change(&self, fields: Document) {
        self.db
            .collection::<UserService>(CONNECTIONS)
            .update_one(doc! {"_id": &self.connection}, doc! {"$set": fields})
            .await
            .unwrap();
    }

    async fn refused(&self) {
        assert!(matches!(
            self.evidence().await,
            Err(AppError::Unauthorized(_))
        ));
    }
}

#[tokio::test]
async fn effective_catalog_grant_requires_exact_catalog_and_live_owned_connection() {
    let f = Fixture::new().await.unwrap();
    f.key_change(doc! {"allowed_service_ids": [], "allowed_platform_service_ids": [&f.service]})
        .await;
    assert_eq!(f.evidence().await.unwrap().service_id, f.service);
    f.key_change(doc! {"allowed_platform_service_ids": [Uuid::new_v4().to_string()]})
        .await;
    f.refused().await;
    f.key_change(doc! {"allowed_platform_service_ids": [&f.service]})
        .await;
    for patch in [
        doc! {"catalog_service_id": Uuid::new_v4().to_string()},
        doc! {"user_id": Uuid::new_v4().to_string()},
        doc! {"is_active": false},
        doc! {"deleted_at": mongodb::bson::DateTime::now()},
    ] {
        f.connection_change(patch).await;
        f.refused().await;
        f.connection_change(doc! {"catalog_service_id": &f.service, "user_id": &f.owner, "is_active": true, "deleted_at": null}).await;
        assert!(f.evidence().await.is_ok());
    }
    f.db.collection::<UserService>(CONNECTIONS)
        .delete_one(doc! {"_id": &f.connection})
        .await
        .unwrap();
    f.refused().await;
}

#[tokio::test]
async fn operation_owner_and_group_restrictions_never_become_owner_wide_evidence() {
    let f = Fixture::new().await.unwrap();
    for operations in [
        Vec::<Document>::new(),
        vec![doc! {"endpoint_id": null, "rule": {"method": "GET", "path_template": "/agents"}}],
    ] {
        let mut scopes = Document::new();
        scopes.insert(
            &f.connection,
            doc! {"revision": 1_i64, "catalog_service_id": &f.service, "operations": operations},
        );
        f.key_change(doc! {"assistant_operation_scopes": scopes})
            .await;
        f.refused().await;
        f.key_change(doc! {"assistant_operation_scopes": {}}).await;
        assert!(f.evidence().await.is_ok());
    }
    for patch in [
        doc! {"assistant_agent_owner_id": &f.owner},
        doc! {"assistant_group_id": Uuid::new_v4().to_string()},
    ] {
        f.key_change(patch).await;
        f.refused().await;
        f.key_change(doc! {"assistant_agent_owner_id": null, "assistant_group_id": null})
            .await;
        assert!(f.evidence().await.is_ok());
    }
}

#[tokio::test]
async fn legacy_conversation_key_retains_guest_and_turn_authority() {
    let f = Fixture::new().await.unwrap();
    let agent = crate::services::assistant_team_service::ensure_nyxbot(&f.db, &f.owner)
        .await
        .unwrap();
    let conversation = format!("nyxa-{}", Uuid::new_v4().simple());
    let now = mongodb::bson::DateTime::now();
    // A valid legacy conversation row exercises the shared live authority loader,
    // including credentials predating the explicit assistant owner marker.
    let row: AssistantConversation = from_document(doc! {
        "_id": &conversation, "user_id": &f.owner, "agent_id": &agent.id,
        "title": "introspection isolation", "model": &agent.model,
        "credential_api_key_id": &f.key.id, "message_count": 0_i64,
        "guest_turn": true, "created_at": now, "updated_at": now,
    })
    .unwrap();
    f.db.collection::<AssistantConversation>(CONVERSATIONS)
        .insert_one(row)
        .await
        .unwrap();
    f.db.collection::<Document>(CHAT_CREDENTIALS)
        .insert_one(doc! {"_id": Uuid::new_v4().to_string(), "user_id": &f.owner, "api_key_id": &f.key.id, "conversation_id": &conversation})
        .await
        .unwrap();
    f.key_change(doc! {
        "platform": crate::services::assistant_agent_credential_service::ASSISTANT_PLATFORM,
        "allow_auto_connected_services": true,
    })
    .await;
    let (_, key, child) = key_service::validate_api_key(&f.db, &f.key.full_key)
        .await
        .unwrap();
    let auth = crate::mw::auth::api_key_auth_user(&f.db, &key, child, None, None)
        .await
        .unwrap();
    assert!(auth.assistant_chat.as_ref().unwrap().guest);
    assert!(auth.assistant_turn_fence.is_some());
    f.refused().await;
}

#[tokio::test]
async fn constrained_catalogs_and_node_transports_are_not_service_wide_authority() {
    let f = Fixture::new().await.unwrap();
    for patch in [
        doc! {"proxy_operation_policy": {"rules": []}},
        doc! {"proxy_operation_policy": {"rules": [{"method": "GET", "path_template": "/agents"}]}},
        doc! {"offering_kind": "tool"},
        doc! {"service_type": "ssh"},
        doc! {"service_category": "provider"},
        doc! {"service_category": crate::services::retired_service_service::RETIRED_CATEGORY},
        doc! {"is_active": false},
    ] {
        f.catalog_change(patch).await;
        f.refused().await;
        f.catalog_change(doc! {"proxy_operation_policy": null, "offering_kind": "ai_service", "service_type": "http", "service_category": "connection", "is_active": true}).await;
        assert!(f.evidence().await.is_ok());
    }
    for patch in [
        doc! {"node_id": Uuid::new_v4().to_string()},
        doc! {"service_type": "ssh"},
    ] {
        f.connection_change(patch).await;
        f.refused().await;
        f.connection_change(doc! {"node_id": null, "service_type": "http"})
            .await;
        assert!(f.evidence().await.is_ok());
    }
}

#[tokio::test]
async fn credential_admission_cannot_be_bypassed_with_service_wide_evidence() {
    let f = Fixture::new().await.unwrap();
    f.allow_service_without_approval().await;
    // Manual connections must remain subject to this boundary too; the
    // auto-provision eligibility check deliberately skips those connections.
    f.connection_change(doc! {"source": null}).await;
    for patch in [
        doc! {"auth_method": "bearer", "service_category": "internal"},
        doc! {"requires_user_credential": true},
        doc! {"provider_config_id": Uuid::new_v4().to_string()},
    ] {
        f.catalog_change(patch).await;
        f.refused().await;
        f.catalog_change(doc! {"auth_method": "none", "service_category": "connection", "requires_user_credential": false, "provider_config_id": null}).await;
        assert!(f.evidence().await.is_ok());
    }
    for patch in [
        doc! {"auth_method": "bearer"},
        doc! {"api_key_id": Uuid::new_v4().to_string()},
    ] {
        f.connection_change(patch).await;
        f.refused().await;
        f.connection_change(doc! {"auth_method": "none", "api_key_id": null})
            .await;
        assert!(f.evidence().await.is_ok());
    }
    // A provider requirement can carry credentials even when older catalog
    // metadata has no provider_config_id and still declares auth_method=none.
    let requirements =
        f.db.collection::<Document>(crate::models::service_provider_requirement::COLLECTION_NAME);
    requirements
        .insert_one(doc! {"_id": Uuid::new_v4().to_string(), "service_id": &f.service})
        .await
        .unwrap();
    f.refused().await;
    requirements
        .delete_many(doc! {"service_id": &f.service})
        .await
        .unwrap();
    assert!(f.evidence().await.is_ok());
}

#[tokio::test]
async fn policy_free_master_credentials_never_receive_introspection_evidence() {
    let f = Fixture::new().await.unwrap();
    f.catalog_change(doc! {
        "auth_method": "bearer", "service_category": "internal",
        "credential_encrypted": mongodb::bson::Binary {
            subtype: mongodb::bson::spec::BinarySubtype::Generic,
            bytes: vec![1, 2, 3],
        },
        "platform_key": {"enabled": true, "audience": "public", "allowed_owner_ids": []},
        "proxy_operation_policy": null,
    })
    .await;
    f.connection_change(doc! {
        "auth_method": "bearer", "credential_binding": "platform", "source": null,
    })
    .await;
    let catalog =
        f.db.collection::<DownstreamService>(SERVICES)
            .find_one(doc! {"_id": &f.service})
            .await
            .unwrap()
            .unwrap();
    // The old ACL-only implementation admitted this shape. It is refused now
    // regardless of PLATFORM_REQUIRE_OPERATION_POLICY, without mutating env.
    assert!(
        platform_key_service::require(&f.db, &catalog, &f.owner)
            .await
            .is_ok()
    );
    f.refused().await;
}

#[tokio::test]
async fn private_auto_connection_rechecks_consent_expiry_and_client_status() {
    let f = Fixture::new().await.unwrap();
    f.db.collection::<OauthClient>(CLIENTS)
        .insert_one(&f.client)
        .await
        .unwrap();
    f.catalog_change(doc! {"visibility": "private", "developer_app_ids": [&f.client.id]})
        .await;
    f.refused().await;
    let consent = Consent {
        id: Uuid::new_v4().to_string(),
        user_id: f.owner.clone(),
        client_id: f.client.id.clone(),
        scopes: "openid".into(),
        revision: None,
        issuance_fence: None,
        allow_all_services: false,
        allowed_service_ids: None,
        granted_at: Utc::now(),
        expires_at: None,
    };
    f.db.collection::<Consent>(CONSENTS)
        .insert_one(&consent)
        .await
        .unwrap();
    assert!(f.evidence().await.is_ok());
    f.db.collection::<Consent>(CONSENTS)
        .update_one(doc! {"_id": &consent.id}, doc! {"$set": {"expires_at": mongodb::bson::DateTime::from_chrono(Utc::now() - chrono::Duration::seconds(1))}})
        .await
        .unwrap();
    f.refused().await;
    f.db.collection::<Consent>(CONSENTS)
        .update_one(
            doc! {"_id": &consent.id},
            doc! {"$set": {"expires_at": null}},
        )
        .await
        .unwrap();
    assert!(f.evidence().await.is_ok());
    f.db.collection::<OauthClient>(CLIENTS)
        .update_one(
            doc! {"_id": &f.client.id},
            doc! {"$set": {"is_active": false}},
        )
        .await
        .unwrap();
    f.refused().await;
    f.db.collection::<OauthClient>(CLIENTS)
        .update_one(
            doc! {"_id": &f.client.id},
            doc! {"$set": {"is_active": true}},
        )
        .await
        .unwrap();
    assert!(f.evidence().await.is_ok());
    f.db.collection::<Consent>(CONSENTS)
        .delete_one(doc! {"_id": &consent.id})
        .await
        .unwrap();
    f.refused().await;
}

#[tokio::test]
async fn platform_acl_is_live_even_for_non_auto_connection() {
    let f = Fixture::new().await.unwrap();
    f.allow_service_without_approval().await;
    f.connection_change(doc! {"source": null}).await;
    f.catalog_change(doc! {
        "service_category": "internal",
        "platform_key": {"enabled": true, "audience": "restricted", "allowed_owner_ids": [&f.owner]},
    }).await;
    assert!(f.evidence().await.is_ok());
    for patch in [
        doc! {"platform_key.allowed_owner_ids": []},
        doc! {"platform_key.enabled": false},
    ] {
        f.catalog_change(patch).await;
        f.refused().await;
        f.catalog_change(
            doc! {"platform_key.enabled": true, "platform_key.allowed_owner_ids": [&f.owner]},
        )
        .await;
        assert!(f.evidence().await.is_ok());
    }
}

#[tokio::test]
async fn unset_global_preference_refuses_manual_service_wide_evidence() {
    let f = Fixture::new().await.unwrap();
    f.connection_change(doc! {"source": null}).await;
    // No channel or local policy: reads pass but writes require approval, so
    // an audience-only result cannot vouch for the entire service.
    f.refused().await;
    crate::services::notification_service::get_or_create_channel(&f.db, &f.owner)
        .await
        .unwrap();
    f.refused().await;
    f.allow_service_without_approval().await;
    assert!(f.evidence().await.is_ok());
}

#[tokio::test]
async fn audience_only_evidence_never_bypasses_service_approval_rules() {
    let f = Fixture::new().await.unwrap();
    let now = mongodb::bson::DateTime::now();
    for service_id in [&f.connection, &f.service] {
        let config: ServiceApprovalConfig = from_document(doc! {
            "_id": Uuid::new_v4().to_string(), "user_id": &f.owner,
            "service_id": service_id, "service_name": "resource",
            "approval_required": true, "created_at": now, "updated_at": now,
        })
        .unwrap();
        f.db.collection::<ServiceApprovalConfig>(APPROVAL_CONFIGS)
            .insert_one(&config)
            .await
            .unwrap();
        f.refused().await;
        for policy in [
            doc! {"approval_required": false, "default_effect": "deny"},
            doc! {"approval_required": false, "default_effect": "auto_allow", "rules": [{"methods": ["DELETE"], "resource_pattern": "*", "effect": "require_approval"}]},
        ] {
            f.db.collection::<ServiceApprovalConfig>(APPROVAL_CONFIGS)
                .update_one(doc! {"_id": &config.id}, doc! {"$set": policy})
                .await
                .unwrap();
            f.refused().await;
        }
        f.db.collection::<ServiceApprovalConfig>(APPROVAL_CONFIGS)
            .update_one(
                doc! {"_id": &config.id},
                doc! {"$set": {"approval_required": false, "default_effect": null, "rules": []}},
            )
            .await
            .unwrap();
        assert!(f.evidence().await.is_ok());
        f.db.collection::<ServiceApprovalConfig>(APPROVAL_CONFIGS)
            .delete_one(doc! {"_id": &config.id})
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn global_approval_is_live_and_auto_connection_exemption_is_preserved() {
    let f = Fixture::new().await.unwrap();
    let now = mongodb::bson::DateTime::now();
    let channel: NotificationChannel = from_document(doc! {
        "_id": Uuid::new_v4().to_string(), "user_id": &f.owner,
        "approval_required": true, "telegram_enabled": true, "telegram_chat_id": 42_i64,
        "created_at": now, "updated_at": now,
    })
    .unwrap();
    f.db.collection::<NotificationChannel>(CHANNELS)
        .insert_one(&channel)
        .await
        .unwrap();
    assert!(f.evidence().await.is_ok());
    f.connection_change(doc! {"source": null}).await;
    f.refused().await;
    let config: ServiceApprovalConfig = from_document(doc! {
        "_id": Uuid::new_v4().to_string(), "user_id": &f.owner,
        "service_id": &f.connection, "service_name": "resource",
        "approval_required": false, "created_at": now, "updated_at": now,
    })
    .unwrap();
    f.db.collection::<ServiceApprovalConfig>(APPROVAL_CONFIGS)
        .insert_one(&config)
        .await
        .unwrap();
    assert!(f.evidence().await.is_ok());
    f.db.collection::<ServiceApprovalConfig>(APPROVAL_CONFIGS)
        .delete_one(doc! {"_id": &config.id})
        .await
        .unwrap();
    f.refused().await;
    f.db.collection::<NotificationChannel>(CHANNELS)
        .update_one(
            doc! {"_id": &channel.id},
            doc! {"$set": {"approval_required": false}},
        )
        .await
        .unwrap();
    assert!(f.evidence().await.is_ok());
}

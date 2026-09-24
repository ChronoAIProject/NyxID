use super::*;
use crate::{
    crypto::token::hash_token,
    models::{
        api_key::{ApiKey, COLLECTION_NAME as KEYS},
        api_key_credential::ApiKeyCredentialKind,
        downstream_service::test_helpers::dummy_service,
        role::{COLLECTION_NAME as ROLES, Role},
        user::UserType,
    },
    test_utils::{connect_test_database, test_user, test_user_service},
};
use uuid::Uuid;

#[tokio::test]
async fn public_service_introspection_registration_requires_admin_and_confidential_client() {
    use crate::{
        handlers::services::{UpdateServiceRequest, update_service},
        services::role_service,
        test_utils::{test_app_state, test_auth_user},
    };
    use axum::{
        Json,
        extract::{Path, State},
    };
    let f = Fixture::new().await.unwrap();
    f.db.collection::<OauthClient>(crate::models::oauth_client::COLLECTION_NAME)
        .insert_one(&f.client)
        .await
        .unwrap();
    f.db.collection::<DownstreamService>(SERVICES)
        .update_one(doc! {"_id": &f.service}, doc! {"$set": {"created_by": &f.owner, "visibility": "public", "introspection_client_ids": []}})
        .await.unwrap();
    let state = test_app_state(f.db.clone());
    let request = |ids: Vec<String>| {
        serde_json::from_value::<UpdateServiceRequest>(
            serde_json::json!({"introspection_client_ids": ids}),
        )
        .unwrap()
    };
    // Being the public service creator does not grant resource-server trust administration.
    for ids in [vec![f.client.id.clone()], vec![]] {
        let result = update_service(
            State(state.clone()),
            test_auth_user(&f.owner),
            Default::default(),
            Path(f.service.clone()),
            Json(request(ids)),
        )
        .await;
        assert!(matches!(result, Err(AppError::Forbidden(_))));
    }
    assert!(f.evidence().await.is_err());
    role_service::seed_system_roles(&f.db).await.unwrap();
    let admin = role_service::get_platform_role_ids(&f.db)
        .await
        .unwrap()
        .admin;
    f.db.collection::<User>(USERS)
        .update_one(
            doc! {"_id": &f.owner},
            doc! {"$addToSet": {"role_ids": admin}},
        )
        .await
        .unwrap();
    for kind in ["public", "confidential"] {
        f.db.collection::<OauthClient>(crate::models::oauth_client::COLLECTION_NAME)
            .update_one(
                doc! {"_id": &f.client.id},
                doc! {"$set": {"client_type": kind}},
            )
            .await
            .unwrap();
        let result = update_service(
            State(state.clone()),
            test_auth_user(&f.owner),
            Default::default(),
            Path(f.service.clone()),
            Json(request(vec![f.client.id.clone()])),
        )
        .await;
        if kind == "public" {
            assert!(matches!(result, Err(AppError::ValidationError(_))));
        } else {
            let result = result.unwrap().0;
            assert_eq!(result.visibility, "public");
            assert_eq!(
                result.introspection_client_ids,
                Some(vec![f.client.id.clone()])
            );
            assert_eq!(result.developer_app_ids, None);
            assert!(f.evidence().await.is_ok());
        }
    }
    update_service(
        State(state),
        test_auth_user(&f.owner),
        Default::default(),
        Path(f.service.clone()),
        Json(request(vec![])),
    )
    .await
    .unwrap();
    assert!(f.evidence().await.is_err());
}

#[tokio::test]
async fn oauth_endpoint_authenticates_client_and_returns_service_bound_evidence() {
    use crate::handlers::oauth::{self, IntrospectRequest};
    use axum::{body::to_bytes, extract::State};
    use axum_extra::extract::Form;
    let Some(mut f) = Fixture::new().await else {
        return;
    };
    let secret = Uuid::new_v4().to_string();
    f.client.client_secret_hash = hash_token(&secret);
    f.db.collection::<OauthClient>(crate::models::oauth_client::COLLECTION_NAME)
        .insert_one(&f.client)
        .await
        .unwrap();
    let state = crate::test_utils::test_app_state(f.db.clone());
    for (credential, service, expected) in [
        (secret.clone(), Some(f.service.clone()), true),
        (Uuid::new_v4().to_string(), Some(f.service.clone()), false),
        (secret.clone(), None, false),
        (secret.clone(), Some(Uuid::new_v4().to_string()), false),
    ] {
        let response = oauth::introspect(
            State(state.clone()),
            Form(IntrospectRequest {
                token: f.key.full_key.clone(),
                token_type_hint: Some("api_key".into()),
                client_id: Some(f.client.id.clone()),
                client_secret: Some(credential),
                service_id: service,
            }),
        )
        .await;
        assert_eq!(response.status(), 200);
        let body: serde_json::Value =
            serde_json::from_slice(&to_bytes(response.into_body(), 16384).await.unwrap()).unwrap();
        assert_eq!(body["active"], expected);
        if expected {
            assert_eq!(body["aud"], f.service);
            assert_eq!(body["sub"], f.owner);
            assert_eq!(body["token_type"], "api_key");
            assert!(body.get("exp").is_none());
        } else {
            assert_eq!(body, serde_json::json!({"active":false}));
        }
    }
}

struct Fixture {
    db: Database,
    client: OauthClient,
    service: String,
    connection: String,
    owner: String,
    role: String,
    key: key_service::CreatedApiKey,
}

impl Fixture {
    async fn new() -> Option<Self> {
        let db = connect_test_database("service_key_introspection").await?;
        let owner = Uuid::new_v4().to_string();
        let now = Utc::now();
        let client = OauthClient {
            id: Uuid::new_v4().to_string(),
            client_name: "resource server".into(),
            client_secret_hash: hash_token(&Uuid::new_v4().to_string()),
            redirect_uris: vec![],
            allowed_scopes: "openid".into(),
            scope_provenance: Default::default(),
            grant_types: "authorization_code".into(),
            client_type: "confidential".into(),
            is_active: true,
            delegation_scopes: String::new(),
            default_service_catalog_slugs: vec![],
            broker_capability_enabled: false,
            revocation_webhook_url: None,
            revocation_webhook_secret_encrypted: None,
            connection_webhook_url: None,
            connection_webhook_secret_encrypted: None,
            connection_webhook_key_id: None,
            connection_webhook_enabled: false,
            created_by: Some(owner.clone()),
            created_at: now,
            updated_at: now,
        };
        let role = Role {
            id: Uuid::new_v4().to_string(),
            name: "User".into(),
            slug: "user".into(),
            description: None,
            permissions: vec!["cma:sandboxes:use-own".into()],
            is_default: false,
            is_system: false,
            client_id: Some(client.id.clone()),
            created_at: now,
            updated_at: now,
        };
        db.collection::<Role>(ROLES)
            .insert_one(&role)
            .await
            .unwrap();
        let mut user = test_user(&owner, UserType::Person);
        user.role_ids.push(role.id.clone());
        db.collection::<User>(USERS).insert_one(user).await.unwrap();
        let mut service = dummy_service();
        service.id = Uuid::new_v4().to_string();
        service.is_active = true;
        service.introspection_client_ids = Some(vec![client.id.clone()]);
        db.collection::<DownstreamService>(SERVICES)
            .insert_one(&service)
            .await
            .unwrap();
        let connection = Uuid::new_v4().to_string();
        let mut binding = test_user_service(
            &connection,
            &owner,
            "resource",
            &Uuid::new_v4().to_string(),
            Some(&service.id),
            None,
        );
        binding.source = Some("auto_provision".into());
        db.collection::<UserService>(CONNECTIONS)
            .insert_one(binding)
            .await
            .unwrap();
        let key = key_service::create_api_key(
            &db,
            &owner,
            "service caller",
            "proxy",
            None,
            None,
            Some(std::slice::from_ref(&connection)),
            None,
            Some(false),
            Some(false),
            Some(false),
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();
        Some(Self {
            db,
            client,
            service: service.id,
            connection,
            owner,
            role: role.id,
            key,
        })
    }

    async fn evidence(&self) -> AppResult<Evidence> {
        introspect(&self.db, &self.client, &self.service, &self.key.full_key).await
    }

    async fn key_change(&self, fields: mongodb::bson::Document) {
        self.db
            .collection::<ApiKey>(KEYS)
            .update_one(doc! {"_id": &self.key.id}, doc! {"$set": fields})
            .await
            .unwrap();
    }
}

#[tokio::test]
async fn resource_server_receives_live_permissions_and_exact_owner() {
    let Some(f) = Fixture::new().await else {
        return;
    };
    let evidence = f.evidence().await.unwrap();
    assert_eq!(evidence.subject, f.owner);
    assert_eq!(evidence.service_id, f.service);
    assert_eq!(evidence.permissions, ["cma:sandboxes:use-own"]);
    assert_eq!(evidence.expires_at, None);
    f.db.collection::<Role>(ROLES)
        .update_one(doc! {"_id": &f.role}, doc! {"$set": {"permissions": []}})
        .await
        .unwrap();
    assert!(f.evidence().await.unwrap().permissions.is_empty());
}

#[tokio::test]
async fn only_registered_confidential_resource_server_can_inspect() {
    let Some(mut f) = Fixture::new().await else {
        return;
    };
    f.client.client_type = "public".into();
    assert!(f.evidence().await.is_err());
    f.client.client_type = "confidential".into();
    f.client.id = Uuid::new_v4().to_string();
    assert!(f.evidence().await.is_err());
    assert!(
        introspect(
            &f.db,
            &f.client,
            &Uuid::new_v4().to_string(),
            &f.key.full_key
        )
        .await
        .is_err()
    );
}

#[tokio::test]
async fn scoped_connection_requires_current_owner_and_live_grant() {
    let Some(f) = Fixture::new().await else {
        return;
    };
    f.key_change(doc! {"allowed_service_ids": []}).await;
    assert!(f.evidence().await.is_err());
    f.key_change(doc! {"allow_auto_connected_services": true})
        .await;
    assert!(f.evidence().await.is_ok());
    f.db.collection::<UserService>(CONNECTIONS)
        .update_one(
            doc! {"_id": &f.connection},
            doc! {"$set": {"user_id": Uuid::new_v4().to_string()}},
        )
        .await
        .unwrap();
    assert!(f.evidence().await.is_err());
    f.key_change(doc! {"allow_all_services": true}).await;
    assert!(f.evidence().await.is_err());
}

#[tokio::test]
async fn revoked_expired_scheduled_and_non_proxy_keys_are_inactive() {
    let Some(f) = Fixture::new().await else {
        return;
    };
    for patch in [
        doc! {"is_active": false},
        doc! {"expires_at": mongodb::bson::DateTime::from_chrono(Utc::now() - chrono::Duration::seconds(1))},
        doc! {"purpose": "scheduled_invocation"},
        doc! {"scopes": "read"},
    ] {
        f.key_change(patch).await;
        assert!(f.evidence().await.is_err());
        f.key_change(
            doc! {"is_active": true, "expires_at": null, "purpose": "general", "scopes": "proxy"},
        )
        .await;
        assert!(f.evidence().await.is_ok());
    }
    f.db.collection::<User>(USERS)
        .update_one(doc! {"_id": &f.owner}, doc! {"$set": {"is_active": false}})
        .await
        .unwrap();
    assert!(f.evidence().await.is_err());
}

#[tokio::test]
async fn child_credential_expiry_and_revocation_are_enforced() {
    let Some(f) = Fixture::new().await else {
        return;
    };
    let secret = crate::services::api_key_credential_service::generate_secret();
    let expiry = Utc::now() + chrono::Duration::minutes(5);
    let child = ApiKeyCredential {
        id: Uuid::new_v4().to_string(),
        api_key_id: f.key.id.clone(),
        user_id: f.owner.clone(),
        secret_hash: hash_token(&secret),
        secret_prefix: secret[..16].into(),
        kind: ApiKeyCredentialKind::AgentKeyLogin,
        label: "test".into(),
        login_request_id: Uuid::new_v4().to_string(),
        is_active: true,
        revoked_at: None,
        revoked_reason: None,
        expires_at: Some(expiry),
        last_used_at: None,
        created_at: Utc::now(),
    };
    f.db.collection::<ApiKeyCredential>(CREDENTIALS)
        .insert_one(&child)
        .await
        .unwrap();
    let evidence = introspect(&f.db, &f.client, &f.service, &secret)
        .await
        .unwrap();
    assert_eq!(
        evidence.expires_at.unwrap().timestamp_millis(),
        expiry.timestamp_millis()
    );
    f.db.collection::<ApiKeyCredential>(CREDENTIALS)
        .update_one(doc! {"_id": &child.id}, doc! {"$set": {"is_active": false}})
        .await
        .unwrap();
    assert!(
        introspect(&f.db, &f.client, &f.service, &secret)
            .await
            .is_err()
    );
}

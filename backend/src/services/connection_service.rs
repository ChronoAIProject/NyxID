use chrono::{DateTime, Utc};
use mongodb::bson::doc;
use uuid::Uuid;

use crate::crypto::aes::EncryptionKeys;
use crate::errors::{AppError, AppResult};
use crate::models::downstream_service::{
    COLLECTION_NAME as DOWNSTREAM_SERVICES, DownstreamService,
};
use crate::models::user_service_connection::{
    COLLECTION_NAME as CONNECTIONS, UserServiceConnection,
};
use crate::services::node_routing_service;
use crate::services::node_ws_manager::NodeWsManager;

/// Maximum credential length in bytes to prevent abuse.
const MAX_CREDENTIAL_LENGTH: usize = 8192;

#[derive(Debug)]
pub struct ConnectionResult {
    pub service_name: String,
    pub connected_at: DateTime<Utc>,
}

/// Connect a user to a service with credential validation.
///
/// For "connection" category services: `credential` is required.
/// For "internal" category services: `credential` must be None.
/// For "provider" category services: returns error (not connectable).
pub async fn connect_user(
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
    node_ws_manager: &NodeWsManager,
    user_id: &str,
    service_id: &str,
    credential: Option<&str>,
    credential_label: Option<&str>,
) -> AppResult<ConnectionResult> {
    // Fetch service
    let service = db
        .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .find_one(doc! { "_id": service_id, "is_active": true })
        .await?
        .ok_or_else(|| AppError::NotFound("Service not found".to_string()))?;

    if service.service_type != "http" {
        return Err(AppError::BadRequest(
            "SSH services do not support credential connections".to_string(),
        ));
    }

    // Validate by category
    match service.service_category.as_str() {
        "provider" => {
            return Err(AppError::BadRequest(
                "Provider services are not connectable".to_string(),
            ));
        }
        "connection" => {
            if credential.is_none() {
                let has_node_route = node_routing_service::has_routable_node_bindings(
                    db,
                    user_id,
                    service_id,
                    node_ws_manager,
                )
                .await?;
                if !has_node_route {
                    return Err(AppError::BadRequest(
                        "Credential is required for this service type unless an online node route is available".to_string(),
                    ));
                }
            }
        }
        "internal" => {
            if credential.is_some() {
                return Err(AppError::BadRequest(
                    "Internal services do not accept user credentials".to_string(),
                ));
            }
        }
        _ => {
            return Err(AppError::Internal(format!(
                "Unknown service category: {}",
                service.service_category
            )));
        }
    }

    // Validate credential length
    if let Some(cred) = credential {
        if cred.is_empty() {
            return Err(AppError::ValidationError(
                "Credential must not be empty".to_string(),
            ));
        }
        if cred.len() > MAX_CREDENTIAL_LENGTH {
            return Err(AppError::ValidationError(format!(
                "Credential exceeds maximum length of {MAX_CREDENTIAL_LENGTH} bytes"
            )));
        }
    }

    let raw_source = db
        .collection::<bson::Document>(CONNECTIONS)
        .find_one(doc! { "user_id":user_id,"service_id":service_id })
        .await?;
    let previous = raw_source
        .clone()
        .map(bson::from_document::<UserServiceConnection>)
        .transpose()
        .map_err(|_| AppError::Internal("Invalid service connection storage".into()))?;
    if previous.as_ref().is_some_and(|row| row.is_active) {
        return Err(AppError::Conflict(
            "Already connected to this service".into(),
        ));
    }

    // Validate credential_label length
    if let Some(label) = credential_label
        && label.len() > 200
    {
        return Err(AppError::ValidationError(
            "Credential label must not exceed 200 characters".to_string(),
        ));
    }

    let now = Utc::now();
    let mut conn = UserServiceConnection {
        id: previous
            .as_ref()
            .map(|row| row.id.clone())
            .unwrap_or_else(|| Uuid::new_v4().to_string()),
        user_id: user_id.to_owned(),
        service_id: service_id.to_owned(),
        credential_encrypted: None,
        secret_context_versions: Default::default(),
        credential_type: credential.and(service.auth_type.clone()),
        credential_label: credential_label.map(str::to_owned).or_else(|| {
            previous
                .as_ref()
                .and_then(|row| row.credential_label.clone())
        }),
        metadata: previous.as_ref().and_then(|row| row.metadata.clone()),
        is_active: true,
        state_version: previous
            .as_ref()
            .map_or(Some(1), |row| row.state_version.checked_add(1))
            .ok_or_else(|| AppError::Conflict("Service connection revision exhausted".into()))?,
        created_at: previous.as_ref().map_or(now, |row| row.created_at),
        updated_at: now,
    };
    if let Some(credential) = credential {
        super::connection_record_service::encrypt(
            encryption_keys,
            &mut conn,
            credential.as_bytes(),
        )
        .await?;
    }
    super::connection_record_service::commit(db, raw_source.as_ref(), &conn).await?;

    Ok(ConnectionResult {
        service_name: service.name,
        connected_at: now,
    })
}

/// Update the credential on an existing connection.
pub async fn update_credential(
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
    user_id: &str,
    service_id: &str,
    credential: &str,
    credential_label: Option<&str>,
) -> AppResult<()> {
    if credential.is_empty() {
        return Err(AppError::ValidationError(
            "Credential must not be empty".to_string(),
        ));
    }
    if credential.len() > MAX_CREDENTIAL_LENGTH {
        return Err(AppError::ValidationError(format!(
            "Credential exceeds maximum length of {MAX_CREDENTIAL_LENGTH} bytes"
        )));
    }

    // Validate credential_label length
    if let Some(label) = credential_label
        && label.len() > 200
    {
        return Err(AppError::ValidationError(
            "Credential label must not exceed 200 characters".to_string(),
        ));
    }

    // Verify the service exists and requires credentials
    let service = db
        .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .find_one(doc! { "_id": service_id, "is_active": true })
        .await?
        .ok_or_else(|| AppError::NotFound("Service not found".to_string()))?;

    if service.service_type != "http" {
        return Err(AppError::BadRequest(
            "SSH services do not support credential updates".to_string(),
        ));
    }

    if !service.requires_user_credential {
        return Err(AppError::BadRequest(
            "This service does not use per-user credentials".to_string(),
        ));
    }

    let raw_source = db
        .collection::<bson::Document>(CONNECTIONS)
        .find_one(doc! {"user_id":user_id,"service_id":service_id,"is_active":true})
        .await?
        .ok_or_else(|| AppError::NotFound("No active connection found for this service".into()))?;
    let mut row: UserServiceConnection = bson::from_document(raw_source.clone())
        .map_err(|_| AppError::Internal("Invalid service connection storage".into()))?;
    super::connection_record_service::encrypt(encryption_keys, &mut row, credential.as_bytes())
        .await?;
    row.credential_type = Some(service.auth_type.unwrap_or_else(|| "api_key".into()));
    if let Some(label) = credential_label {
        row.credential_label = Some(label.to_owned());
    }
    row.state_version = row
        .state_version
        .checked_add(1)
        .ok_or_else(|| AppError::Conflict("Service connection revision exhausted".into()))?;
    row.updated_at = Utc::now();
    super::connection_record_service::commit(db, Some(&raw_source), &row).await?;

    Ok(())
}

/// Disconnect a user from a service.
/// Clears the credential_encrypted field before deactivating.
pub async fn disconnect_user(
    db: &mongodb::Database,
    user_id: &str,
    service_id: &str,
) -> AppResult<()> {
    disconnect_user_with_expected_state_version(db, user_id, service_id, None)
        .await
        .map(|_| ())
}

/// Disconnect one active legacy connection, optionally fencing the mutation
/// on the authorization evidence observed by the browser.
pub async fn disconnect_user_with_expected_state_version(
    db: &mongodb::Database,
    user_id: &str,
    service_id: &str,
    expected_state_version: Option<i64>,
) -> AppResult<UserServiceConnection> {
    let raw_source = db
        .collection::<bson::Document>(CONNECTIONS)
        .find_one(doc! {"user_id":user_id,"service_id":service_id,"is_active":true})
        .await?
        .ok_or_else(|| AppError::NotFound("Connection not found".into()))?;
    let mut row: UserServiceConnection = bson::from_document(raw_source.clone())
        .map_err(|_| AppError::Internal("Invalid service connection storage".into()))?;
    if expected_state_version.is_some_and(|expected| expected != row.state_version) {
        return Err(AppError::Conflict(
            "the connection changed since this action was prepared".into(),
        ));
    }
    row.is_active = false;
    row.credential_encrypted = None;
    row.secret_context_versions.clear();
    row.credential_type = None;
    row.credential_label = None;
    row.updated_at = Utc::now();
    row.state_version = row
        .state_version
        .checked_add(1)
        .ok_or_else(|| AppError::Conflict("Service connection revision exhausted".into()))?;
    super::connection_record_service::commit(db, Some(&raw_source), &row).await?;
    Ok(row)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::downstream_service::test_helpers::dummy_service;
    use crate::test_utils::*;

    async fn insert_connection_service(
        db: &mongodb::Database,
        service_id: &str,
        category: &str,
        requires_cred: bool,
        service_type: &str,
    ) {
        let mut svc = dummy_service();
        svc.id = service_id.to_string();
        svc.service_category = category.to_string();
        svc.requires_user_credential = requires_cred;
        svc.service_type = service_type.to_string();
        svc.auth_type = Some("api_key".to_string());
        db.collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .insert_one(&svc)
            .await
            .expect("insert test service");
    }

    #[tokio::test]
    async fn test_connect_user_connection_service() {
        let Some(db) = connect_test_database("conn_svc_connect").await else {
            return;
        };
        let enc = test_encryption_keys();
        let ws = NodeWsManager::new(30, 100);
        let user_id = Uuid::new_v4().to_string();
        let service_id = Uuid::new_v4().to_string();
        insert_connection_service(&db, &service_id, "connection", true, "http").await;

        let result = connect_user(
            &db,
            &enc,
            &ws,
            &user_id,
            &service_id,
            Some("my-api-key"),
            Some("prod"),
        )
        .await;
        assert!(result.is_ok());
        let cr = result.unwrap();
        assert!(!cr.service_name.is_empty());

        let conn = db
            .collection::<UserServiceConnection>(CONNECTIONS)
            .find_one(doc! { "user_id": &user_id, "service_id": &service_id })
            .await
            .unwrap()
            .expect("connection should exist");
        assert!(conn.is_active);
        assert!(conn.credential_encrypted.is_some());
        assert_eq!(conn.credential_label.as_deref(), Some("prod"));
    }

    #[tokio::test]
    async fn test_connect_user_internal_service_rejects_credential() {
        let Some(db) = connect_test_database("conn_svc_internal").await else {
            return;
        };
        let enc = test_encryption_keys();
        let ws = NodeWsManager::new(30, 100);
        let user_id = Uuid::new_v4().to_string();
        let service_id = Uuid::new_v4().to_string();
        insert_connection_service(&db, &service_id, "internal", false, "http").await;

        let err = connect_user(&db, &enc, &ws, &user_id, &service_id, Some("secret"), None)
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)));
    }

    #[tokio::test]
    async fn test_connect_user_provider_service_rejected() {
        let Some(db) = connect_test_database("conn_svc_provider").await else {
            return;
        };
        let enc = test_encryption_keys();
        let ws = NodeWsManager::new(30, 100);
        let user_id = Uuid::new_v4().to_string();
        let service_id = Uuid::new_v4().to_string();
        insert_connection_service(&db, &service_id, "provider", false, "http").await;

        let err = connect_user(&db, &enc, &ws, &user_id, &service_id, None, None)
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::BadRequest(_)));
    }

    #[tokio::test]
    async fn test_connect_user_duplicate_rejected() {
        let Some(db) = connect_test_database("conn_svc_dup").await else {
            return;
        };
        let enc = test_encryption_keys();
        let ws = NodeWsManager::new(30, 100);
        let user_id = Uuid::new_v4().to_string();
        let service_id = Uuid::new_v4().to_string();
        insert_connection_service(&db, &service_id, "connection", true, "http").await;

        connect_user(&db, &enc, &ws, &user_id, &service_id, Some("key1"), None)
            .await
            .unwrap();
        let err = connect_user(&db, &enc, &ws, &user_id, &service_id, Some("key2"), None)
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::Conflict(_)));
    }

    #[tokio::test]
    async fn test_update_credential() {
        let Some(db) = connect_test_database("conn_svc_update").await else {
            return;
        };
        let enc = test_encryption_keys();
        let ws = NodeWsManager::new(30, 100);
        let user_id = Uuid::new_v4().to_string();
        let service_id = Uuid::new_v4().to_string();
        insert_connection_service(&db, &service_id, "connection", true, "http").await;

        connect_user(&db, &enc, &ws, &user_id, &service_id, Some("old-key"), None)
            .await
            .unwrap();

        let result = update_credential(
            &db,
            &enc,
            &user_id,
            &service_id,
            "new-key",
            Some("updated-label"),
        )
        .await;
        assert!(result.is_ok());

        let conn = db
            .collection::<UserServiceConnection>(CONNECTIONS)
            .find_one(doc! { "user_id": &user_id, "service_id": &service_id, "is_active": true })
            .await
            .unwrap()
            .expect("connection should exist");
        assert_eq!(conn.credential_label.as_deref(), Some("updated-label"));
    }

    #[tokio::test]
    async fn test_disconnect_user() {
        let Some(db) = connect_test_database("conn_svc_disc").await else {
            return;
        };
        let enc = test_encryption_keys();
        let ws = NodeWsManager::new(30, 100);
        let user_id = Uuid::new_v4().to_string();
        let service_id = Uuid::new_v4().to_string();
        insert_connection_service(&db, &service_id, "connection", true, "http").await;

        connect_user(&db, &enc, &ws, &user_id, &service_id, Some("key"), None)
            .await
            .unwrap();

        disconnect_user(&db, &user_id, &service_id).await.unwrap();

        let conn = db
            .collection::<UserServiceConnection>(CONNECTIONS)
            .find_one(doc! { "user_id": &user_id, "service_id": &service_id })
            .await
            .unwrap()
            .expect("connection should still exist (soft delete)");
        assert!(!conn.is_active);
        assert!(conn.credential_encrypted.is_none());

        let err = disconnect_user(&db, &user_id, &service_id)
            .await
            .unwrap_err();
        assert!(matches!(err, AppError::NotFound(_)));
    }
    #[tokio::test]
    async fn staged_and_strict_connection_lifecycle_authenticates_service_and_invalidates_dependencies()
     {
        let Some(db) = connect_test_database("connection_context_lifecycle").await else {
            return;
        };
        let owner = Uuid::new_v4().to_string();
        let service_id = Uuid::new_v4().to_string();
        insert_connection_service(&db, &service_id, "connection", true, "http").await;
        // Ensure this exercises the production credential branch of the proxy resolver.
        db.collection::<bson::Document>(DOWNSTREAM_SERVICES)
            .update_one(
                doc! {"_id":&service_id},
                doc! {"$set":{"auth_method":"header"}},
            )
            .await
            .unwrap();
        let ws = NodeWsManager::new(30, 100);
        let mut stage = test_encryption_keys();
        stage.set_contextual_record_writes(false);
        connect_user(
            &db,
            &stage,
            &ws,
            &owner,
            &service_id,
            Some("staged-secret"),
            None,
        )
        .await
        .unwrap();
        let source = db
            .collection::<bson::Document>(CONNECTIONS)
            .find_one(doc! {"user_id":&owner})
            .await
            .unwrap()
            .unwrap();
        let staged: UserServiceConnection = bson::from_document(source.clone()).unwrap();
        assert!(staged.secret_context_versions.is_empty());
        assert_eq!(
            stage
                .decrypt(staged.credential_encrypted.as_deref().unwrap())
                .await
                .unwrap(),
            b"staged-secret"
        );
        let mut migration = test_encryption_keys();
        migration.set_contextual_record_writes(true);
        assert!(
            super::super::connection_record_service::migrate_snapshot(&db, &migration, &source)
                .await
                .unwrap()
        );
        let current = db
            .collection::<bson::Document>(CONNECTIONS)
            .find_one(doc! {"user_id":&owner})
            .await
            .unwrap()
            .unwrap();
        let before = db
            .collection::<bson::Document>(super::super::erasure_admission::COLLECTION)
            .find_one(doc! {"_id":&owner})
            .await
            .unwrap();
        assert!(
            !super::super::connection_record_service::migrate_snapshot(&db, &migration, &current)
                .await
                .unwrap()
        );
        assert_eq!(
            db.collection::<bson::Document>(super::super::erasure_admission::COLLECTION)
                .find_one(doc! {"_id":&owner})
                .await
                .unwrap(),
            before
        );
        let mut strict = test_encryption_keys();
        strict.set_contextual_record_writes(true);
        strict.set_allow_legacy_record_reads(false);
        super::super::connection_record_service::verify_current_snapshot(&strict, &current)
            .await
            .unwrap();
        let target = super::super::proxy_service::resolve_proxy_target(
            &db,
            &strict,
            &owner,
            &service_id,
            crate::mw::rate_limit::PlatformUserRateLimitPolicy::disabled(),
        )
        .await
        .unwrap();
        assert_eq!(target.credential.as_str(), "staged-secret");
        let mut swapped: UserServiceConnection = bson::from_document(current.clone()).unwrap();
        swapped.service_id = Uuid::new_v4().to_string();
        assert!(
            super::super::connection_record_service::text(&strict, &swapped)
                .await
                .is_err()
        );
        swapped = bson::from_document(current.clone()).unwrap();
        swapped.user_id = Uuid::new_v4().to_string();
        assert!(
            super::super::connection_record_service::text(&strict, &swapped)
                .await
                .is_err()
        );
        update_credential(
            &db,
            &strict,
            &owner,
            &service_id,
            "replacement-secret",
            Some("new"),
        )
        .await
        .unwrap();
        // The earlier snapshot cannot replace a successor, including marker changes.
        let old: UserServiceConnection = bson::from_document(current).unwrap();
        assert!(matches!(
            super::super::connection_record_service::commit(&db, Some(&source), &old).await,
            Err(AppError::Conflict(_))
        ));
        let target = super::super::proxy_service::resolve_proxy_target(
            &db,
            &strict,
            &owner,
            &service_id,
            crate::mw::rate_limit::PlatformUserRateLimitPolicy::disabled(),
        )
        .await
        .unwrap();
        assert_eq!(target.credential.as_str(), "replacement-secret");
        disconnect_user_with_expected_state_version(&db, &owner, &service_id, Some(1))
            .await
            .unwrap_err();
        disconnect_user(&db, &owner, &service_id).await.unwrap();
        let disconnected = db
            .collection::<bson::Document>(CONNECTIONS)
            .find_one(doc! {"user_id":&owner})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(disconnected.get("credential_type"), Some(&bson::Bson::Null));
        assert_eq!(
            disconnected.get("credential_label"),
            Some(&bson::Bson::Null)
        );
        assert_eq!(
            disconnected.get("credential_encrypted"),
            Some(&bson::Bson::Null)
        );

        connect_user(
            &db,
            &strict,
            &ws,
            &owner,
            &service_id,
            Some("reconnected-secret"),
            None,
        )
        .await
        .unwrap();
        let reconnected = db
            .collection::<bson::Document>(CONNECTIONS)
            .find_one(doc! {"user_id":&owner})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(reconnected.get("credential_label"), Some(&bson::Bson::Null));
        super::super::connection_record_service::verify_current_snapshot(&strict, &reconnected)
            .await
            .unwrap();
        // A durable tombstone blocks the same delayed production writer even
        // after its source owner document has been removed.
        db.collection::<bson::Document>(super::super::erasure_admission::COLLECTION)
            .update_one(doc! {"_id":&owner}, doc! {"$set":{"erasure_accepted":true}})
            .await
            .unwrap();
        assert!(matches!(
            update_credential(
                &db,
                &strict,
                &owner,
                &service_id,
                "forbidden-late-secret",
                None
            )
            .await,
            Err(AppError::Conflict(_))
        ));
        assert_eq!(
            db.collection::<bson::Document>(CONNECTIONS)
                .find_one(doc! {"user_id":&owner})
                .await
                .unwrap()
                .unwrap(),
            reconnected
        );
    }
}

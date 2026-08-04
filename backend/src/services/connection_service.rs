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

    // Check for existing active connection
    let existing = db
        .collection::<UserServiceConnection>(CONNECTIONS)
        .find_one(doc! {
            "user_id": user_id,
            "service_id": service_id,
            "is_active": true,
        })
        .await?;

    if existing.is_some() {
        return Err(AppError::Conflict(
            "Already connected to this service".to_string(),
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

    // Encrypt credential if provided
    let credential_encrypted = match credential {
        Some(cred) => Some(encryption_keys.encrypt(cred.as_bytes()).await?),
        None => None,
    };

    let credential_type = if credential.is_some() {
        service.auth_type.clone()
    } else {
        None
    };

    let now = Utc::now();

    // Check for an inactive (soft-deleted) connection and reactivate it
    let inactive = db
        .collection::<UserServiceConnection>(CONNECTIONS)
        .find_one(doc! {
            "user_id": user_id,
            "service_id": service_id,
            "is_active": false,
        })
        .await?;

    if inactive.is_some() {
        let mut set_doc = doc! {
            "is_active": true,
            "updated_at": mongodb::bson::DateTime::from_chrono(now),
        };
        if let Some(enc) = &credential_encrypted {
            set_doc.insert(
                "credential_encrypted",
                mongodb::bson::Binary {
                    subtype: mongodb::bson::spec::BinarySubtype::Generic,
                    bytes: enc.clone(),
                },
            );
        }
        if let Some(ct) = &credential_type {
            set_doc.insert("credential_type", ct.as_str());
        }
        if let Some(label) = credential_label {
            set_doc.insert("credential_label", label);
        }

        db.collection::<UserServiceConnection>(CONNECTIONS)
            .update_one(
                doc! { "user_id": user_id, "service_id": service_id, "is_active": false },
                doc! { "$set": set_doc },
            )
            .await?;

        return Ok(ConnectionResult {
            service_name: service.name,
            connected_at: now,
        });
    }

    // Truly new connection: insert
    let conn_id = Uuid::new_v4().to_string();

    let conn = UserServiceConnection {
        id: conn_id.clone(),
        user_id: user_id.to_string(),
        service_id: service_id.to_string(),
        credential_encrypted,
        credential_type,
        credential_label: credential_label.map(|s| s.to_string()),
        metadata: None,
        is_active: true,
        created_at: now,
        updated_at: now,
    };

    db.collection::<UserServiceConnection>(CONNECTIONS)
        .insert_one(&conn)
        .await?;

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

    let encrypted = encryption_keys.encrypt(credential.as_bytes()).await?;
    let now = Utc::now();

    let mut set_doc = doc! {
        "credential_encrypted": mongodb::bson::Binary {
            subtype: mongodb::bson::spec::BinarySubtype::Generic,
            bytes: encrypted,
        },
        "credential_type": service.auth_type.as_deref().unwrap_or("api_key"),
        "updated_at": mongodb::bson::DateTime::from_chrono(now),
    };

    // Note: When credential_label is None, the existing label is preserved.
    // To clear a label, pass an explicit empty string.
    if let Some(label) = credential_label {
        set_doc.insert("credential_label", label);
    }

    let result = db
        .collection::<UserServiceConnection>(CONNECTIONS)
        .update_one(
            doc! {
                "user_id": user_id,
                "service_id": service_id,
                "is_active": true,
            },
            doc! { "$set": set_doc },
        )
        .await?;

    if result.matched_count == 0 {
        return Err(AppError::NotFound(
            "No active connection found for this service".to_string(),
        ));
    }

    Ok(())
}

/// Disconnect a user from a service.
/// Clears the credential_encrypted field before deactivating.
pub async fn disconnect_user(
    db: &mongodb::Database,
    user_id: &str,
    service_id: &str,
) -> AppResult<()> {
    let now = Utc::now();

    let result = db
        .collection::<UserServiceConnection>(CONNECTIONS)
        .update_one(
            doc! {
                "user_id": user_id,
                "service_id": service_id,
                "is_active": true,
            },
            doc! { "$set": {
                "is_active": false,
                "credential_encrypted": mongodb::bson::Bson::Null,
                "credential_type": mongodb::bson::Bson::Null,
                "credential_label": mongodb::bson::Bson::Null,
                "updated_at": mongodb::bson::DateTime::from_chrono(now),
            }},
        )
        .await?;

    if result.matched_count == 0 {
        return Err(AppError::NotFound("Connection not found".to_string()));
    }

    Ok(())
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
}

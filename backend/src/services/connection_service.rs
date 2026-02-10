use chrono::{DateTime, Utc};
use mongodb::bson::doc;
use uuid::Uuid;

use crate::crypto::aes;
use crate::errors::{AppError, AppResult};
use crate::models::downstream_service::{DownstreamService, COLLECTION_NAME as DOWNSTREAM_SERVICES};
use crate::models::user_service_connection::{
    UserServiceConnection, COLLECTION_NAME as CONNECTIONS,
};

/// Maximum credential length in bytes to prevent abuse.
const MAX_CREDENTIAL_LENGTH: usize = 8192;

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
    encryption_key: &[u8],
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

    // Validate by category
    match service.service_category.as_str() {
        "provider" => {
            return Err(AppError::BadRequest(
                "Provider services are not connectable".to_string(),
            ));
        }
        "connection" => {
            if credential.is_none() {
                return Err(AppError::BadRequest(
                    "Credential is required for this service type".to_string(),
                ));
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
        Some(cred) => Some(aes::encrypt(cred.as_bytes(), encryption_key)?),
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
            set_doc.insert("credential_encrypted", mongodb::bson::Binary {
                subtype: mongodb::bson::spec::BinarySubtype::Generic,
                bytes: enc.clone(),
            });
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
    encryption_key: &[u8],
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

    if !service.requires_user_credential {
        return Err(AppError::BadRequest(
            "This service does not use per-user credentials".to_string(),
        ));
    }

    let encrypted = aes::encrypt(credential.as_bytes(), encryption_key)?;
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


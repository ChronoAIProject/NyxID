use chrono::Utc;
use futures::TryStreamExt;
use mongodb::bson::doc;
use uuid::Uuid;

use crate::errors::{AppError, AppResult};
use crate::models::consent::{COLLECTION_NAME as CONSENTS, Consent};

/// Grant consent for a user to a client with specific scopes.
/// Upserts: if consent exists for (user_id, client_id), replaces scopes.
pub async fn grant_consent(
    db: &mongodb::Database,
    user_id: &str,
    client_id: &str,
    scopes: &str,
) -> AppResult<Consent> {
    let now = Utc::now();

    let consent = Consent {
        id: Uuid::new_v4().to_string(),
        user_id: user_id.to_string(),
        client_id: client_id.to_string(),
        scopes: scopes.to_string(),
        granted_at: now,
        expires_at: None,
    };

    // Try to find existing consent for this user+client
    let existing = db
        .collection::<Consent>(CONSENTS)
        .find_one(doc! { "user_id": user_id, "client_id": client_id })
        .await?;

    match existing {
        Some(ex) => {
            // Update existing consent
            let updated = Consent {
                id: ex.id,
                user_id: user_id.to_string(),
                client_id: client_id.to_string(),
                scopes: scopes.to_string(),
                granted_at: now,
                expires_at: None,
            };

            db.collection::<Consent>(CONSENTS)
                .replace_one(doc! { "_id": &updated.id }, &updated)
                .await?;

            Ok(updated)
        }
        None => {
            db.collection::<Consent>(CONSENTS)
                .insert_one(&consent)
                .await?;
            Ok(consent)
        }
    }
}

/// Check if a user has granted consent for the requested scopes to a client.
/// Returns Some(Consent) if all requested scopes are covered.
pub async fn check_consent(
    db: &mongodb::Database,
    user_id: &str,
    client_id: &str,
    requested_scopes: &str,
) -> AppResult<Option<Consent>> {
    let consent = db
        .collection::<Consent>(CONSENTS)
        .find_one(doc! { "user_id": user_id, "client_id": client_id })
        .await?;

    match consent {
        Some(c) => {
            // Check if the consent has expired
            if let Some(expires_at) = c.expires_at
                && expires_at < Utc::now()
            {
                return Ok(None);
            }

            let granted: std::collections::HashSet<&str> = c.scopes.split_whitespace().collect();
            let requested: Vec<&str> = requested_scopes.split_whitespace().collect();

            let all_covered = requested.iter().all(|s| granted.contains(s));
            if all_covered { Ok(Some(c)) } else { Ok(None) }
        }
        None => Ok(None),
    }
}

/// Revoke consent for a specific client.
pub async fn revoke_consent(
    db: &mongodb::Database,
    user_id: &str,
    client_id: &str,
) -> AppResult<()> {
    let result = db
        .collection::<Consent>(CONSENTS)
        .delete_one(doc! { "user_id": user_id, "client_id": client_id })
        .await?;

    if result.deleted_count == 0 {
        return Err(AppError::ConsentNotFound);
    }

    Ok(())
}

/// List all consents for a user.
pub async fn list_user_consents(db: &mongodb::Database, user_id: &str) -> AppResult<Vec<Consent>> {
    let consents: Vec<Consent> = db
        .collection::<Consent>(CONSENTS)
        .find(doc! { "user_id": user_id })
        .await?
        .try_collect()
        .await?;

    Ok(consents)
}

/// List all consents for a client.
pub async fn list_client_consents(
    db: &mongodb::Database,
    client_id: &str,
) -> AppResult<Vec<Consent>> {
    let consents: Vec<Consent> = db
        .collection::<Consent>(CONSENTS)
        .find(doc! { "client_id": client_id })
        .await?
        .try_collect()
        .await?;

    Ok(consents)
}

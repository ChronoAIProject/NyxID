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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::*;

    #[tokio::test]
    async fn test_grant_consent_creates_new() {
        let Some(db) = connect_test_database("consent").await else {
            return;
        };
        let user_id = Uuid::new_v4().to_string();
        let client_id = Uuid::new_v4().to_string();

        let consent = grant_consent(&db, &user_id, &client_id, "openid profile")
            .await
            .unwrap();

        assert_eq!(consent.user_id, user_id);
        assert_eq!(consent.client_id, client_id);
        assert_eq!(consent.scopes, "openid profile");
        assert!(consent.expires_at.is_none());

        let stored = db
            .collection::<Consent>(CONSENTS)
            .find_one(doc! { "_id": &consent.id })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(stored.scopes, "openid profile");
    }

    #[tokio::test]
    async fn test_grant_consent_upserts_existing() {
        let Some(db) = connect_test_database("consent").await else {
            return;
        };
        let user_id = Uuid::new_v4().to_string();
        let client_id = Uuid::new_v4().to_string();

        let first = grant_consent(&db, &user_id, &client_id, "openid")
            .await
            .unwrap();
        let second = grant_consent(&db, &user_id, &client_id, "openid profile email")
            .await
            .unwrap();

        assert_eq!(first.id, second.id);
        assert_eq!(second.scopes, "openid profile email");

        let count = db
            .collection::<Consent>(CONSENTS)
            .count_documents(doc! { "user_id": &user_id, "client_id": &client_id })
            .await
            .unwrap();
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn test_check_consent_covers_all_scopes() {
        let Some(db) = connect_test_database("consent").await else {
            return;
        };
        let user_id = Uuid::new_v4().to_string();
        let client_id = Uuid::new_v4().to_string();

        grant_consent(&db, &user_id, &client_id, "openid profile email")
            .await
            .unwrap();

        let found = check_consent(&db, &user_id, &client_id, "openid profile")
            .await
            .unwrap();
        assert!(found.is_some());

        let missing = check_consent(&db, &user_id, &client_id, "openid admin")
            .await
            .unwrap();
        assert!(missing.is_none());

        let no_consent = check_consent(&db, &user_id, &Uuid::new_v4().to_string(), "openid")
            .await
            .unwrap();
        assert!(no_consent.is_none());
    }

    #[tokio::test]
    async fn test_revoke_consent_deletes_and_errors_on_missing() {
        let Some(db) = connect_test_database("consent").await else {
            return;
        };
        let user_id = Uuid::new_v4().to_string();
        let client_id = Uuid::new_v4().to_string();

        grant_consent(&db, &user_id, &client_id, "openid")
            .await
            .unwrap();
        revoke_consent(&db, &user_id, &client_id).await.unwrap();

        let after = check_consent(&db, &user_id, &client_id, "openid")
            .await
            .unwrap();
        assert!(after.is_none());

        let err = revoke_consent(&db, &user_id, &client_id).await;
        assert!(err.is_err());
    }

    #[tokio::test]
    async fn test_list_user_and_client_consents() {
        let Some(db) = connect_test_database("consent").await else {
            return;
        };
        let user_id = Uuid::new_v4().to_string();
        let client_a = Uuid::new_v4().to_string();
        let client_b = Uuid::new_v4().to_string();

        grant_consent(&db, &user_id, &client_a, "openid")
            .await
            .unwrap();
        grant_consent(&db, &user_id, &client_b, "profile")
            .await
            .unwrap();

        let user_consents = list_user_consents(&db, &user_id).await.unwrap();
        assert_eq!(user_consents.len(), 2);

        let client_consents = list_client_consents(&db, &client_a).await.unwrap();
        assert_eq!(client_consents.len(), 1);
        assert_eq!(client_consents[0].scopes, "openid");

        let empty = list_user_consents(&db, &Uuid::new_v4().to_string())
            .await
            .unwrap();
        assert!(empty.is_empty());
    }
}

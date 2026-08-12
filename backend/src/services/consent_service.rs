use chrono::Utc;
use futures::TryStreamExt;
use mongodb::bson::{self, doc};
use uuid::Uuid;

use crate::errors::{AppError, AppResult};
use crate::models::consent::{COLLECTION_NAME as CONSENTS, Consent};
use crate::models::refresh_token::{COLLECTION_NAME as REFRESH_TOKENS, RefreshToken};

/// Grant consent for a user to a client with specific scopes.
/// Upserts: if consent exists for (user_id, client_id), replaces scopes.
///
/// Test-only convenience wrapper: production code uses
/// [`grant_consent_with_services`]; the plain `grant_consent` is retained only
/// for existing tests, so it is scoped to test builds to avoid a bin dead-code error.
#[cfg(test)]
pub async fn grant_consent(
    db: &mongodb::Database,
    user_id: &str,
    client_id: &str,
    scopes: &str,
) -> AppResult<Consent> {
    grant_consent_internal(db, user_id, client_id, scopes, false, Some(Vec::new())).await
}

/// Grant consent for a user to a client with optional service restriction.
///
/// `Some(ids)`, including an empty list, is an explicit per-service consent
/// grant. `None` is an explicit unrestricted service grant.
pub async fn grant_consent_with_services(
    db: &mongodb::Database,
    user_id: &str,
    client_id: &str,
    scopes: &str,
    allowed_service_ids: Option<Vec<String>>,
) -> AppResult<Consent> {
    let allow_all_services = allowed_service_ids.is_none();
    let explicit_allowed_service_ids = allowed_service_ids.or_else(|| Some(Vec::new()));
    grant_consent_internal(
        db,
        user_id,
        client_id,
        scopes,
        allow_all_services,
        explicit_allowed_service_ids,
    )
    .await
}

async fn grant_consent_internal(
    db: &mongodb::Database,
    user_id: &str,
    client_id: &str,
    scopes: &str,
    allow_all_services: bool,
    allowed_service_ids: Option<Vec<String>>,
) -> AppResult<Consent> {
    let now = Utc::now();

    let consent = Consent {
        id: Uuid::new_v4().to_string(),
        user_id: user_id.to_string(),
        client_id: client_id.to_string(),
        scopes: scopes.to_string(),
        allow_all_services,
        allowed_service_ids: allowed_service_ids.clone(),
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
                allow_all_services,
                allowed_service_ids,
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
) -> AppResult<ConsentRevocationResult> {
    let existing = db
        .collection::<Consent>(CONSENTS)
        .find_one(doc! { "user_id": user_id, "client_id": client_id })
        .await?;
    if existing.is_none() {
        return Err(AppError::ConsentNotFound);
    }

    let now = Utc::now();
    let revoked_refresh_tokens = if client_id == Uuid::nil().to_string() {
        0
    } else {
        db.collection::<RefreshToken>(REFRESH_TOKENS)
            .update_many(
                doc! {
                    "user_id": user_id,
                    "client_id": client_id,
                    "revoked": false,
                },
                doc! { "$set": {
                    "revoked": true,
                    "revoked_at": bson::DateTime::from_chrono(now),
                }},
            )
            .await?
            .modified_count
    };

    let result = db
        .collection::<Consent>(CONSENTS)
        .delete_one(doc! { "user_id": user_id, "client_id": client_id })
        .await?;

    if result.deleted_count == 0 {
        return Err(AppError::ConsentNotFound);
    }

    Ok(ConsentRevocationResult {
        revoked_refresh_tokens,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConsentRevocationResult {
    pub revoked_refresh_tokens: u64,
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
    use crate::models::oauth_client::{COLLECTION_NAME as OAUTH_CLIENTS, OauthClient};
    use crate::test_utils::*;
    use chrono::Duration;

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
        assert!(!consent.allow_all_services);
        assert_eq!(consent.allowed_service_ids, Some(vec![]));
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
        let second = grant_consent_with_services(
            &db,
            &user_id,
            &client_id,
            "openid profile email",
            Some(vec!["svc-1".to_string()]),
        )
        .await
        .unwrap();

        assert_eq!(first.id, second.id);
        assert_eq!(second.scopes, "openid profile email");
        assert!(!second.allow_all_services);
        assert_eq!(second.allowed_service_ids, Some(vec!["svc-1".to_string()]));

        let count = db
            .collection::<Consent>(CONSENTS)
            .count_documents(doc! { "user_id": &user_id, "client_id": &client_id })
            .await
            .unwrap();
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn test_grant_consent_with_services_persists_allowlist() {
        let Some(db) = connect_test_database("consent").await else {
            return;
        };
        let user_id = Uuid::new_v4().to_string();
        let client_id = Uuid::new_v4().to_string();

        let consent = grant_consent_with_services(
            &db,
            &user_id,
            &client_id,
            "openid",
            Some(vec!["svc-1".to_string(), "svc-2".to_string()]),
        )
        .await
        .unwrap();

        assert_eq!(
            consent.allowed_service_ids,
            Some(vec!["svc-1".to_string(), "svc-2".to_string()])
        );
        assert!(!consent.allow_all_services);

        let updated =
            grant_consent_with_services(&db, &user_id, &client_id, "openid", Some(vec![]))
                .await
                .unwrap();
        assert_eq!(updated.id, consent.id);
        assert!(!updated.allow_all_services);
        assert_eq!(updated.allowed_service_ids, Some(vec![]));

        let unrestricted = grant_consent_with_services(&db, &user_id, &client_id, "openid", None)
            .await
            .unwrap();
        assert_eq!(unrestricted.id, consent.id);
        assert!(unrestricted.allow_all_services);
        assert_eq!(unrestricted.allowed_service_ids, Some(vec![]));
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
        let result = revoke_consent(&db, &user_id, &client_id).await.unwrap();
        assert_eq!(result.revoked_refresh_tokens, 0);

        let after = check_consent(&db, &user_id, &client_id, "openid")
            .await
            .unwrap();
        assert!(after.is_none());

        let err = revoke_consent(&db, &user_id, &client_id).await;
        assert!(err.is_err());
    }

    #[tokio::test]
    async fn test_revoke_consent_revokes_refresh_token_chain_for_client_only() {
        let Some(db) = connect_test_database("consent_revoke_refresh").await else {
            return;
        };
        let config = test_app_config();
        let jwt_keys = cached_test_jwt_keys();
        let user_id = Uuid::new_v4().to_string();
        let client_id = Uuid::new_v4().to_string();
        let other_client_id = Uuid::new_v4().to_string();
        let now = Utc::now();

        db.collection::<OauthClient>(OAUTH_CLIENTS)
            .insert_one(OauthClient {
                id: client_id.clone(),
                client_name: "Revoked Client".to_string(),
                client_secret_hash: String::new(),
                redirect_uris: vec!["https://app.example/callback".to_string()],
                allowed_scopes: "openid profile".to_string(),
                scope_provenance: Default::default(),
                grant_types: "authorization_code refresh_token".to_string(),
                client_type: "public".to_string(),
                is_active: true,
                delegation_scopes: String::new(),
                broker_capability_enabled: false,
                revocation_webhook_url: None,
                revocation_webhook_secret_encrypted: None,
                connection_webhook_url: None,
                connection_webhook_secret_encrypted: None,
                connection_webhook_key_id: None,
                connection_webhook_enabled: false,
                created_by: Some(user_id.clone()),
                default_service_catalog_slugs: Vec::new(),
                created_at: now,
                updated_at: now,
            })
            .await
            .unwrap();

        grant_consent(&db, &user_id, &client_id, "openid")
            .await
            .unwrap();
        let user_uuid = Uuid::parse_str(&user_id).unwrap();
        let (refresh_jwt, refresh_jti) =
            crate::crypto::jwt::generate_refresh_token(&jwt_keys, &config, &user_uuid).unwrap();
        let first_party_jti = Uuid::new_v4().to_string();
        let other_client_jti = Uuid::new_v4().to_string();

        db.collection::<RefreshToken>(REFRESH_TOKENS)
            .insert_many([
                RefreshToken {
                    id: Uuid::new_v4().to_string(),
                    jti: refresh_jti.clone(),
                    client_id: client_id.clone(),
                    user_id: user_id.clone(),
                    session_id: None,
                    scope: Some("openid".to_string()),
                    expires_at: now + Duration::days(7),
                    revoked: false,
                    replaced_by: None,
                    revoked_at: None,
                    resource_uris: Vec::new(),
                    allowed_service_ids: Vec::new(),
                    allow_all_services: true,
                    created_at: now,
                },
                RefreshToken {
                    id: Uuid::new_v4().to_string(),
                    jti: first_party_jti.clone(),
                    client_id: Uuid::nil().to_string(),
                    user_id: user_id.clone(),
                    session_id: None,
                    scope: None,
                    expires_at: now + Duration::days(7),
                    revoked: false,
                    replaced_by: None,
                    revoked_at: None,
                    resource_uris: Vec::new(),
                    allowed_service_ids: Vec::new(),
                    allow_all_services: true,
                    created_at: now,
                },
                RefreshToken {
                    id: Uuid::new_v4().to_string(),
                    jti: other_client_jti.clone(),
                    client_id: other_client_id,
                    user_id: user_id.clone(),
                    session_id: None,
                    scope: Some("openid".to_string()),
                    expires_at: now + Duration::days(7),
                    revoked: false,
                    replaced_by: None,
                    revoked_at: None,
                    resource_uris: Vec::new(),
                    allowed_service_ids: Vec::new(),
                    allow_all_services: true,
                    created_at: now,
                },
            ])
            .await
            .unwrap();

        let result = revoke_consent(&db, &user_id, &client_id).await.unwrap();
        assert_eq!(result.revoked_refresh_tokens, 1);

        let revoked = db
            .collection::<RefreshToken>(REFRESH_TOKENS)
            .find_one(doc! { "jti": &refresh_jti })
            .await
            .unwrap()
            .unwrap();
        assert!(revoked.revoked);
        assert!(revoked.revoked_at.is_some());

        let first_party = db
            .collection::<RefreshToken>(REFRESH_TOKENS)
            .find_one(doc! { "jti": &first_party_jti })
            .await
            .unwrap()
            .unwrap();
        assert!(!first_party.revoked);
        let other_client = db
            .collection::<RefreshToken>(REFRESH_TOKENS)
            .find_one(doc! { "jti": &other_client_jti })
            .await
            .unwrap()
            .unwrap();
        assert!(!other_client.revoked);

        let refresh_result = crate::services::token_service::refresh_tokens(
            &db,
            &config,
            &jwt_keys,
            &refresh_jwt,
            None,
            None,
        )
        .await;
        assert!(matches!(refresh_result, Err(AppError::Unauthorized(_))));
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

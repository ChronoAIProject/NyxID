use chrono::Utc;
use futures::TryStreamExt;
use mongodb::bson::{self, doc};
use uuid::Uuid;

use crate::errors::{AppError, AppResult};
use crate::models::consent::{COLLECTION_NAME as CONSENTS, Consent};
use crate::models::refresh_token::{COLLECTION_NAME as REFRESH_TOKENS, RefreshToken};
use crate::services::catalog_delegation_service;

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

fn duplicate_key(error: &mongodb::error::Error) -> bool {
    matches!(
        error.kind.as_ref(),
        mongodb::error::ErrorKind::Write(mongodb::error::WriteFailure::WriteError(write_error))
            if write_error.code == 11000
    )
}

fn merged_scope_expression(required_scopes: &[&str]) -> bson::Bson {
    let mut expression = bson::Bson::Document(doc! { "$ifNull": ["$scopes", ""] });
    for scope in required_scopes {
        expression = bson::Bson::Document(doc! {
            "$cond": [
                { "$in": [scope, { "$split": [expression.clone(), " "] }] },
                expression.clone(),
                { "$trim": { "input": { "$concat": [expression, " ", scope] } } },
            ],
        });
    }
    expression
}

async fn merge_existing_consent(
    db: &mongodb::Database,
    user_id: &str,
    client_id: &str,
    required_scopes: &[&str],
    service_id: &str,
) -> AppResult<Option<Consent>> {
    let existing_services = doc! { "$ifNull": ["$allowed_service_ids", []] };
    let result = db
        .collection::<Consent>(CONSENTS)
        .update_one(
            doc! { "user_id": user_id, "client_id": client_id },
            vec![doc! { "$set": {
                "scopes": merged_scope_expression(required_scopes),
                "allowed_service_ids": {
                    "$cond": [
                        { "$in": [service_id, existing_services.clone()] },
                        existing_services.clone(),
                        { "$concatArrays": [existing_services, [service_id]] },
                    ],
                },
            }}],
        )
        .await?;
    if result.matched_count == 0 {
        return Ok(None);
    }
    Ok(db
        .collection::<Consent>(CONSENTS)
        .find_one(doc! { "user_id": user_id, "client_id": client_id })
        .await?)
}

pub async fn merge_consent_services_atomic(
    db: &mongodb::Database,
    user_id: &str,
    client_id: &str,
    required_scopes: &str,
    service_id: &str,
) -> AppResult<Consent> {
    let mut unique_scopes = Vec::new();
    for scope in required_scopes.split_whitespace() {
        if !unique_scopes.contains(&scope) {
            unique_scopes.push(scope);
        }
    }
    if unique_scopes.is_empty() {
        return Err(AppError::ValidationError(
            "at least one consent scope is required".to_string(),
        ));
    }

    if let Some(consent) =
        merge_existing_consent(db, user_id, client_id, &unique_scopes, service_id).await?
    {
        return Ok(consent);
    }

    let consent = Consent {
        id: Uuid::new_v4().to_string(),
        user_id: user_id.to_string(),
        client_id: client_id.to_string(),
        scopes: unique_scopes.join(" "),
        allow_all_services: false,
        allowed_service_ids: Some(vec![service_id.to_string()]),
        granted_at: Utc::now(),
        expires_at: None,
    };
    match db.collection::<Consent>(CONSENTS).insert_one(&consent).await {
        Ok(_) => Ok(consent),
        Err(error) if duplicate_key(&error) => {
            merge_existing_consent(db, user_id, client_id, &unique_scopes, service_id)
                .await?
                .ok_or_else(|| {
                    AppError::Conflict("consent merge raced without a winning row".to_string())
                })
        }
        Err(error) => Err(error.into()),
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
    let revoked_catalog_grants = catalog_delegation_service::revoke_for_user_client_roles(
        db, user_id, client_id,
    )
    .await?;
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
        revoked_catalog_grants,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConsentRevocationResult {
    pub revoked_refresh_tokens: u64,
    pub revoked_catalog_grants: u64,
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
    use crate::models::catalog_delegation_grant::{
        COLLECTION_NAME as CATALOG_DELEGATION_GRANTS, CatalogDelegationGrant,
    };
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
    async fn merge_consent_services_atomic_inserts_explicit_restricted_consent() {
        let Some(db) = connect_test_database("consent_merge_insert").await else {
            return;
        };
        let user_id = Uuid::new_v4().to_string();
        let client_id = Uuid::new_v4().to_string();
        let service_id = Uuid::new_v4().to_string();

        let consent = merge_consent_services_atomic(
            &db,
            &user_id,
            &client_id,
            "openid profile",
            &service_id,
        )
        .await
        .unwrap();

        assert_eq!(consent.user_id, user_id);
        assert_eq!(consent.client_id, client_id);
        assert_eq!(consent.scopes, "openid profile");
        assert!(!consent.allow_all_services);
        assert_eq!(consent.allowed_service_ids, Some(vec![service_id]));
        assert!(Uuid::parse_str(&consent.id).is_ok());
    }

    #[tokio::test]
    async fn merge_consent_services_atomic_preserves_and_unions_existing_consent() {
        let Some(db) = connect_test_database("consent_merge_existing").await else {
            return;
        };
        let user_id = Uuid::new_v4().to_string();
        let client_id = Uuid::new_v4().to_string();
        let first_service_id = Uuid::new_v4().to_string();
        let second_service_id = Uuid::new_v4().to_string();
        let consent_id = Uuid::new_v4().to_string();
        let granted_at = Utc::now() - Duration::hours(2);
        let expires_at = Utc::now() + Duration::hours(2);

        db.collection::<mongodb::bson::Document>(CONSENTS)
            .insert_one(doc! {
                "_id": &consent_id,
                "user_id": &user_id,
                "client_id": &client_id,
                "scopes": "openid email",
                "allow_all_services": false,
                "allowed_service_ids": [&first_service_id],
                "granted_at": bson::DateTime::from_chrono(granted_at),
                "expires_at": bson::DateTime::from_chrono(expires_at),
                "future_policy": { "mode": "preserve" },
            })
            .await
            .unwrap();

        let consent = merge_consent_services_atomic(
            &db,
            &user_id,
            &client_id,
            "openid profile",
            &second_service_id,
        )
        .await
        .unwrap();

        assert_eq!(consent.id, consent_id);
        assert_eq!(consent.scopes, "openid email profile");
        assert!(!consent.allow_all_services);
        assert_eq!(
            consent.allowed_service_ids,
            Some(vec![first_service_id, second_service_id])
        );
        assert_eq!(
            consent.granted_at.timestamp_millis(),
            granted_at.timestamp_millis()
        );
        assert_eq!(
            consent.expires_at.unwrap().timestamp_millis(),
            expires_at.timestamp_millis()
        );

        let stored = db
            .collection::<mongodb::bson::Document>(CONSENTS)
            .find_one(doc! { "_id": &consent.id })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            stored.get_document("future_policy").unwrap().get_str("mode"),
            Ok("preserve")
        );
    }

    #[tokio::test]
    async fn merge_consent_services_atomic_preserves_unrestricted_consent() {
        let Some(db) = connect_test_database("consent_merge_unrestricted").await else {
            return;
        };
        let user_id = Uuid::new_v4().to_string();
        let client_id = Uuid::new_v4().to_string();
        let service_id = Uuid::new_v4().to_string();
        let existing = grant_consent_with_services(
            &db,
            &user_id,
            &client_id,
            "openid email",
            None,
        )
        .await
        .unwrap();

        let merged = merge_consent_services_atomic(
            &db,
            &user_id,
            &client_id,
            "openid profile",
            &service_id,
        )
        .await
        .unwrap();

        assert_eq!(merged.id, existing.id);
        assert!(merged.allow_all_services);
        assert_eq!(merged.scopes, "openid email profile");
        assert_eq!(merged.allowed_service_ids, Some(vec![service_id]));
    }

    #[tokio::test]
    async fn concurrent_consent_merges_keep_both_service_ids() {
        let Some(db) = connect_test_database("consent_merge_concurrent").await else {
            return;
        };
        let user_id = Uuid::new_v4().to_string();
        let client_id = Uuid::new_v4().to_string();
        let first_service_id = Uuid::new_v4().to_string();
        let second_service_id = Uuid::new_v4().to_string();

        let (first, second) = tokio::join!(
            merge_consent_services_atomic(
                &db,
                &user_id,
                &client_id,
                "openid",
                &first_service_id,
            ),
            merge_consent_services_atomic(
                &db,
                &user_id,
                &client_id,
                "openid",
                &second_service_id,
            ),
        );
        first.unwrap();
        second.unwrap();

        let stored = check_consent(&db, &user_id, &client_id, "openid")
            .await
            .unwrap()
            .unwrap();
        let mut actual = stored.allowed_service_ids.unwrap();
        actual.sort();
        let mut expected = vec![first_service_id, second_service_id];
        expected.sort();
        assert_eq!(actual, expected);
        assert_eq!(
            db.collection::<Consent>(CONSENTS)
                .count_documents(doc! { "user_id": &user_id, "client_id": &client_id })
                .await
                .unwrap(),
            1
        );
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
        assert_eq!(result.revoked_catalog_grants, 0);

        let after = check_consent(&db, &user_id, &client_id, "openid")
            .await
            .unwrap();
        assert!(after.is_none());

        let err = revoke_consent(&db, &user_id, &client_id).await;
        assert!(err.is_err());
    }

    #[tokio::test]
    async fn revoke_consent_revokes_catalog_grants_for_either_client_role_first() {
        let Some(db) = connect_test_database("consent_revoke_catalog").await else {
            return;
        };
        let user_id = Uuid::new_v4().to_string();
        let other_user_id = Uuid::new_v4().to_string();
        let client_id = Uuid::new_v4().to_string();
        let other_client_id = Uuid::new_v4().to_string();
        grant_consent(&db, &user_id, &client_id, "openid")
            .await
            .unwrap();
        let now = Utc::now();

        let grant = |user_id: &str, actor: &str, receiver: &str| CatalogDelegationGrant {
            id: Uuid::new_v4().to_string(),
            user_id: user_id.to_string(),
            actor_client_id: actor.to_string(),
            receiving_client_id: receiver.to_string(),
            scope: "proxy:* mcp:catalog:read".to_string(),
            resources: Vec::new(),
            allow_all_services: false,
            allowed_service_ids: Vec::new(),
            allow_all_nodes: false,
            allowed_node_ids: Vec::new(),
            revoked: false,
            expires_at: now + Duration::minutes(5),
            created_at: now,
        };
        let actor_match = grant(&user_id, &client_id, &other_client_id);
        let receiver_match = grant(&user_id, &other_client_id, &client_id);
        let wrong_user = grant(&other_user_id, &client_id, &client_id);
        let wrong_client = grant(&user_id, &other_client_id, &other_client_id);
        db.collection::<CatalogDelegationGrant>(CATALOG_DELEGATION_GRANTS)
            .insert_many([
                actor_match.clone(),
                receiver_match.clone(),
                wrong_user.clone(),
                wrong_client.clone(),
            ])
            .await
            .unwrap();

        let result = revoke_consent(&db, &user_id, &client_id).await.unwrap();
        assert_eq!(result.revoked_catalog_grants, 2);
        for id in [&actor_match.id, &receiver_match.id] {
            let stored = db
                .collection::<CatalogDelegationGrant>(CATALOG_DELEGATION_GRANTS)
                .find_one(doc! { "_id": id })
                .await
                .unwrap()
                .unwrap();
            assert!(stored.revoked);
        }
        for id in [&wrong_user.id, &wrong_client.id] {
            let stored = db
                .collection::<CatalogDelegationGrant>(CATALOG_DELEGATION_GRANTS)
                .find_one(doc! { "_id": id })
                .await
                .unwrap()
                .unwrap();
            assert!(!stored.revoked);
        }
        assert!(
            db.collection::<Consent>(CONSENTS)
                .find_one(doc! { "user_id": &user_id, "client_id": &client_id })
                .await
                .unwrap()
                .is_none()
        );
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

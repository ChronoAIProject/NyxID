use chrono::{Duration, Utc};
use futures::TryStreamExt;
use mongodb::Database;
use mongodb::bson::{self, doc};
use rand::RngCore;
use serde::Serialize;
use uuid::Uuid;

use crate::config::AppConfig;
use crate::crypto::jwt::{self, JwtKeys};
use crate::crypto::token::{constant_time_eq, hash_token};
use crate::errors::{AppError, AppResult};
use crate::models::service_account::{COLLECTION_NAME as SERVICE_ACCOUNTS, ServiceAccount};
use crate::models::service_account_token::{COLLECTION_NAME as SA_TOKENS, ServiceAccountToken};

#[cfg(test)]
tokio::task_local! { pub(crate) static ISSUANCE_BARRIER: (std::sync::Arc<tokio::sync::Barrier>, std::sync::Arc<tokio::sync::Barrier>); }

#[derive(Debug, Serialize)]
pub struct ClientCredentialsResponse {
    pub access_token: String,
    pub token_type: String,
    pub expires_in: i64,
    pub scope: String,
}

/// Generate a client_id: "sa_" + 24 hex chars (12 random bytes).
fn generate_client_id() -> String {
    let mut bytes = [0u8; 12];
    rand::thread_rng().fill_bytes(&mut bytes);
    format!("sa_{}", hex::encode(bytes))
}

/// Generate a client_secret: "sas_" + 64 hex chars (32 random bytes).
fn generate_client_secret() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    format!("sas_{}", hex::encode(bytes))
}

/// Create a new service account. Returns (ServiceAccount, raw_client_secret).
///
/// Note: Duplicate names are intentionally allowed. The `client_id` is the
/// unique identifier; names are for human display only.
pub async fn create_service_account(
    db: &Database,
    name: &str,
    description: Option<&str>,
    allowed_scopes: &str,
    role_ids: &[String],
    rate_limit_override: Option<u64>,
    created_by: &str,
) -> AppResult<(ServiceAccount, String)> {
    let id = Uuid::new_v4().to_string();
    create_service_account_with_id(
        db,
        &id,
        name,
        description,
        allowed_scopes,
        role_ids,
        rate_limit_override,
        created_by,
    )
    .await
}

/// Create a service account with a caller-reserved UUID.
#[allow(clippy::too_many_arguments)]
pub async fn create_service_account_with_id(
    db: &Database,
    id: &str,
    name: &str,
    description: Option<&str>,
    allowed_scopes: &str,
    role_ids: &[String],
    rate_limit_override: Option<u64>,
    created_by: &str,
) -> AppResult<(ServiceAccount, String)> {
    if name.is_empty() || name.len() > 100 {
        return Err(AppError::ValidationError(
            "Service account name must be between 1 and 100 characters".to_string(),
        ));
    }

    if let Some(d) = description
        && d.len() > 500
    {
        return Err(AppError::ValidationError(
            "Description must be 500 characters or less".to_string(),
        ));
    }

    // Scopes are free-form strings. Validation against a known scope vocabulary
    // is not enforced here; unrecognized scopes will simply not match any
    // access control rules at request time.
    if allowed_scopes.is_empty() {
        return Err(AppError::ValidationError(
            "At least one scope is required".to_string(),
        ));
    }

    if let Some(rl) = rate_limit_override
        && rl == 0
    {
        return Err(AppError::ValidationError(
            "Rate limit override must be greater than 0".to_string(),
        ));
    }

    if !role_ids.is_empty() {
        let existing_count = db
            .collection::<crate::models::role::Role>(crate::models::role::COLLECTION_NAME)
            .count_documents(doc! { "_id": { "$in": role_ids } })
            .await?;
        if existing_count != role_ids.len() as u64 {
            return Err(AppError::ValidationError(
                "One or more role IDs do not exist".to_string(),
            ));
        }
    }

    let catalog_editor =
        super::catalog_editor_service::role_has_editor_permissions(db, role_ids).await?;
    if catalog_editor {
        super::catalog_editor_service::validate_scopes(allowed_scopes)?;
    }
    let client_id = generate_client_id();
    let raw_secret = generate_client_secret();
    let secret_hash = hash_token(&raw_secret);
    let secret_prefix = raw_secret[..8].to_string();
    let now = Utc::now();

    let sa = ServiceAccount {
        id: id.to_string(),
        name: name.to_string(),
        description: description.map(String::from),
        client_id,
        client_secret_hash: secret_hash,
        platform_protected: catalog_editor,
        purpose: if catalog_editor {
            crate::models::service_account::ServiceAccountPurpose::CatalogEditor
        } else {
            crate::models::service_account::ServiceAccountPurpose::General
        },
        curation_grant: None,
        credential_generation: 0,
        secret_prefix,
        role_ids: role_ids.to_vec(),
        allowed_scopes: allowed_scopes.to_string(),
        is_active: true,
        rate_limit_override,
        created_by: created_by.to_string(),
        owner_user_id: Some(created_by.to_string()),
        created_at: now,
        updated_at: now,
        last_authenticated_at: None,
    };

    db.collection::<ServiceAccount>(SERVICE_ACCOUNTS)
        .insert_one(&sa)
        .await?;

    Ok((sa, raw_secret))
}

/// List service accounts (paginated). When `owner_user_id` is `Some`,
/// scopes the result to SAs owned by that user (used for org-scoped
/// listing). Without an owner filter the function returns every SA in
/// the system; the caller is responsible for the global-admin gate.
pub async fn list_service_accounts(
    db: &Database,
    page: u64,
    per_page: u64,
    search: Option<&str>,
    owner_user_id: Option<&str>,
) -> AppResult<(Vec<ServiceAccount>, u64)> {
    let offset = (page - 1) * per_page;

    let mut filter = match search {
        Some(s) if !s.is_empty() => {
            let escaped = regex::escape(s);
            doc! { "name": { "$regex": &escaped, "$options": "i" } }
        }
        _ => doc! {},
    };

    // Owner filter (used for org-scoped listing). Match either
    // `owner_user_id` directly or `created_by` for pre-owner-field
    // records that never got the field populated.
    if let Some(owner) = owner_user_id {
        filter.insert(
            "$or",
            vec![
                doc! { "owner_user_id": owner },
                doc! { "owner_user_id": { "$exists": false }, "created_by": owner },
                doc! { "owner_user_id": bson::Bson::Null, "created_by": owner },
            ],
        );
    }

    let total = db
        .collection::<ServiceAccount>(SERVICE_ACCOUNTS)
        .count_documents(filter.clone())
        .await?;

    let accounts: Vec<ServiceAccount> = db
        .collection::<ServiceAccount>(SERVICE_ACCOUNTS)
        .find(filter)
        .sort(doc! { "created_at": -1 })
        .skip(offset)
        .limit(per_page as i64)
        .await?
        .try_collect()
        .await?;

    Ok((accounts, total))
}

/// Get a service account by ID.
pub async fn get_service_account(db: &Database, sa_id: &str) -> AppResult<ServiceAccount> {
    db.collection::<ServiceAccount>(SERVICE_ACCOUNTS)
        .find_one(doc! { "_id": sa_id })
        .await?
        .ok_or_else(|| AppError::ServiceAccountNotFound(sa_id.to_string()))
}

#[derive(Debug, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExpectedAccessState {
    pub role_ids: Vec<String>,
    pub allowed_scopes: String,
    pub purpose: crate::models::service_account::ServiceAccountPurpose,
    pub platform_protected: bool,
    pub is_active: bool,
}

#[allow(clippy::too_many_arguments)]
pub async fn update_service_account(
    db: &Database,
    sa_id: &str,
    name: Option<&str>,
    description: Option<&str>,
    allowed_scopes: Option<&str>,
    role_ids: Option<&[String]>,
    rate_limit_override: Option<Option<u64>>,
    is_active: Option<bool>,
    platform_admin: bool,
    expected_access: Option<&ExpectedAccessState>,
) -> AppResult<ServiceAccount> {
    // Verify it exists first
    let existing = get_service_account(db, sa_id).await?;
    let activate_editor = if let Some(roles) = role_ids {
        platform_admin
            && super::catalog_editor_service::role_has_editor_permissions(db, roles).await?
    } else {
        false
    };
    if activate_editor
        || existing.purpose == crate::models::service_account::ServiceAccountPurpose::CatalogEditor
    {
        super::catalog_editor_service::validate_scopes(
            allowed_scopes.unwrap_or(&existing.allowed_scopes),
        )?;
    }
    if existing.purpose == crate::models::service_account::ServiceAccountPurpose::Curation
        && !activate_editor
        && let Some(scopes) = allowed_scopes
    {
        super::curation_grant_service::validate_scopes(
            scopes,
            existing
                .curation_grant
                .as_ref()
                .and_then(|g| g.ornn_proxy_service_id.as_deref()),
        )?;
    }

    let mut set_doc = doc! {
        "updated_at": bson::DateTime::from_chrono(Utc::now()),
    };

    if let Some(n) = name {
        if n.is_empty() || n.len() > 100 {
            return Err(AppError::ValidationError(
                "Service account name must be between 1 and 100 characters".to_string(),
            ));
        }
        set_doc.insert("name", n);
    }

    if let Some(d) = description {
        if d.len() > 500 {
            return Err(AppError::ValidationError(
                "Description must be 500 characters or less".to_string(),
            ));
        }
        if d.is_empty() {
            set_doc.insert("description", bson::Bson::Null);
        } else {
            set_doc.insert("description", d);
        }
    }

    if let Some(s) = allowed_scopes {
        if s.is_empty() {
            return Err(AppError::ValidationError(
                "At least one scope is required".to_string(),
            ));
        }
        set_doc.insert("allowed_scopes", s);
    }

    if let Some(roles) = role_ids {
        if !platform_admin {
            return Err(AppError::Forbidden(
                "Role assignment requires platform admin".into(),
            ));
        }
        if activate_editor {
            set_doc.insert("platform_protected", true);
            set_doc.insert("purpose", "catalog_editor");
        }
        if !roles.is_empty() {
            let existing_count = db
                .collection::<crate::models::role::Role>(crate::models::role::COLLECTION_NAME)
                .count_documents(doc! { "_id": { "$in": roles.iter().map(|r| r.as_str()).collect::<Vec<&str>>() } })
                .await?;
            if existing_count != roles.len() as u64 {
                return Err(AppError::ValidationError(
                    "One or more role IDs do not exist".to_string(),
                ));
            }
        }
        set_doc.insert(
            "role_ids",
            roles.iter().map(|r| r.as_str()).collect::<Vec<&str>>(),
        );
    }

    if let Some(rl) = rate_limit_override {
        match rl {
            Some(val) => {
                if val == 0 {
                    return Err(AppError::ValidationError(
                        "Rate limit override must be greater than 0".to_string(),
                    ));
                }
                set_doc.insert("rate_limit_override", val as i64);
            }
            None => {
                set_doc.insert("rate_limit_override", bson::Bson::Null);
            }
        }
    }

    if let Some(active) = is_active {
        set_doc.insert("is_active", active);
    }

    let mut filter = management_filter(sa_id, platform_admin);
    filter.insert(
        "purpose",
        match existing.purpose {
            crate::models::service_account::ServiceAccountPurpose::Curation => {
                bson::Bson::String("curation".into())
            }
            crate::models::service_account::ServiceAccountPurpose::CatalogEditor => {
                bson::Bson::String("catalog_editor".into())
            }
            crate::models::service_account::ServiceAccountPurpose::General => {
                bson::Bson::Document(doc! {"$nin": ["curation", "catalog_editor"]})
            }
        },
    );
    if let Some(expected) = expected_access {
        // Missing fields on legacy accounts have the same defaults as serde.
        filter.insert("$expr", doc! {"$and": [
            {"$eq": [{"$ifNull": ["$role_ids", []]}, {"$literal": &expected.role_ids}]},
            {"$eq": ["$allowed_scopes", {"$literal": &expected.allowed_scopes}]},
            {"$eq": [{"$ifNull": ["$purpose", "general"]}, bson::to_bson(&expected.purpose).map_err(|e| AppError::Internal(e.to_string()))?]},
            {"$eq": [{"$ifNull": ["$platform_protected", false]}, expected.platform_protected]},
            {"$eq": ["$is_active", expected.is_active]},
        ]});
    }
    let mut update = doc! {"$set": set_doc};
    if is_active == Some(false) {
        update.insert("$inc", doc! {"credential_generation": 1_i64});
    }
    let result = db
        .collection::<ServiceAccount>(SERVICE_ACCOUNTS)
        .update_one(filter, update)
        .await?;
    if expected_access.is_some() && result.matched_count == 0 {
        return Err(AppError::Conflict(
            "Service account access changed; reload before applying catalog access".into(),
        ));
    }
    require_managed_match(result.matched_count)?;

    get_service_account(db, sa_id).await
}

/// Rotate the client secret. Revokes all outstanding tokens.
/// Returns (updated ServiceAccount, new raw_client_secret).
pub async fn rotate_secret(
    db: &Database,
    sa_id: &str,
    platform_admin: bool,
) -> AppResult<(ServiceAccount, String)> {
    let _existing = get_service_account(db, sa_id).await?;

    let raw_secret = generate_client_secret();
    let secret_hash = hash_token(&raw_secret);
    let secret_prefix = raw_secret[..8].to_string();

    let result = db
        .collection::<ServiceAccount>(SERVICE_ACCOUNTS)
        .update_one(
            management_filter(sa_id, platform_admin),
            doc! {
                "$inc": { "credential_generation": 1_i64 },
                "$set": {
                    "client_secret_hash": &secret_hash,
                    "secret_prefix": &secret_prefix,
                    "updated_at": bson::DateTime::from_chrono(Utc::now()),
                }
            },
        )
        .await?;

    require_managed_match(result.matched_count)?;
    revoke_token_rows(db, sa_id).await?;

    let updated = get_service_account(db, sa_id).await?;
    Ok((updated, raw_secret))
}

/// Soft-delete (deactivate) a service account and revoke all tokens.
pub async fn delete_service_account(
    db: &Database,
    sa_id: &str,
    platform_admin: bool,
) -> AppResult<()> {
    let _existing = get_service_account(db, sa_id).await?;

    let result = db
        .collection::<ServiceAccount>(SERVICE_ACCOUNTS)
        .update_one(
            management_filter(sa_id, platform_admin),
            doc! {
                "$inc": {"credential_generation": 1_i64},
                "$set": {
                    "is_active": false,
                    "updated_at": bson::DateTime::from_chrono(Utc::now()),
                }
            },
        )
        .await?;

    require_managed_match(result.matched_count)?;
    revoke_token_rows(db, sa_id).await?;

    Ok(())
}

/// Revoke all active tokens for a service account.
fn management_filter(sa_id: &str, platform_admin: bool) -> bson::Document {
    let mut filter = doc! {"_id": sa_id};
    if !platform_admin {
        filter.insert("platform_protected", doc! {"$ne": true});
        filter.insert("purpose", doc! {"$nin": ["curation", "catalog_editor"]});
    }
    filter
}

fn require_managed_match(count: u64) -> AppResult<()> {
    if count == 1 {
        Ok(())
    } else {
        Err(AppError::Forbidden(
            "Service account changed or requires platform administration".into(),
        ))
    }
}

pub async fn revoke_all_tokens(db: &Database, sa_id: &str, platform_admin: bool) -> AppResult<u64> {
    let result = db.collection::<ServiceAccount>(SERVICE_ACCOUNTS).update_one(management_filter(sa_id, platform_admin),
        doc! {"$inc": {"credential_generation": 1_i64}, "$set": {"updated_at": bson::DateTime::from_chrono(Utc::now())}}).await?;
    require_managed_match(result.matched_count)?;
    revoke_token_rows(db, sa_id).await
}

async fn revoke_token_rows(db: &Database, sa_id: &str) -> AppResult<u64> {
    let result = db
        .collection::<ServiceAccountToken>(SA_TOKENS)
        .update_many(
            doc! {"service_account_id": sa_id, "revoked": false},
            doc! {"$set": {"revoked": true}},
        )
        .await?;
    Ok(result.modified_count)
}

/// Authenticate via client credentials: validate client_id + client_secret,
/// issue a JWT, and persist a token record.
pub async fn authenticate_client_credentials(
    db: &Database,
    config: &AppConfig,
    jwt_keys: &JwtKeys,
    client_id: &str,
    client_secret: &str,
    requested_scope: Option<&str>,
) -> AppResult<ClientCredentialsResponse> {
    let secret_hash = hash_token(client_secret);

    let sa = db
        .collection::<ServiceAccount>(SERVICE_ACCOUNTS)
        .find_one(doc! { "client_id": client_id })
        .await?
        .ok_or_else(|| AppError::AuthenticationFailed("Invalid client credentials".to_string()))?;

    if !sa.is_active {
        return Err(AppError::AuthenticationFailed(
            "Invalid client credentials".to_string(),
        ));
    }

    if !constant_time_eq(sa.client_secret_hash.as_bytes(), secret_hash.as_bytes()) {
        return Err(AppError::AuthenticationFailed(
            "Invalid client credentials".to_string(),
        ));
    }

    #[cfg(test)]
    if let Ok((validated, resume)) = ISSUANCE_BARRIER.try_with(Clone::clone) {
        validated.wait().await;
        resume.wait().await;
    }

    // Validate requested scopes are a subset of allowed_scopes
    let granted_scope = match requested_scope {
        Some(req) if !req.is_empty() => {
            let allowed: std::collections::HashSet<&str> =
                sa.allowed_scopes.split_whitespace().collect();
            for s in req.split_whitespace() {
                if !allowed.contains(s) {
                    return Err(AppError::InvalidScope(format!(
                        "Scope '{}' is not allowed for this service account",
                        s
                    )));
                }
            }
            req.to_string()
        }
        _ => sa.allowed_scopes.clone(),
    };

    let ttl = config.sa_token_ttl_secs;

    let (token, jti) = jwt::generate_service_account_token(
        jwt_keys,
        config,
        &sa.id,
        &granted_scope,
        ttl,
        sa.credential_generation,
    )?;

    // Persist token record for revocation support
    let token_record = ServiceAccountToken {
        id: Uuid::new_v4().to_string(),
        jti,
        service_account_id: sa.id.clone(),
        scope: granted_scope.clone(),
        expires_at: Utc::now() + Duration::seconds(ttl),
        revoked: false,
        credential_generation: sa.credential_generation,
        created_at: Utc::now(),
    };

    db.collection::<ServiceAccountToken>(SA_TOKENS)
        .insert_one(&token_record)
        .await?;

    // Update last_authenticated_at
    db.collection::<ServiceAccount>(SERVICE_ACCOUNTS)
        .update_one(
            doc! { "_id": &sa.id },
            doc! { "$set": { "last_authenticated_at": bson::DateTime::from_chrono(Utc::now()) } },
        )
        .await?;

    Ok(ClientCredentialsResponse {
        access_token: token,
        token_type: "Bearer".to_string(),
        expires_in: ttl,
        scope: granted_scope,
    })
}

pub(crate) fn validate_token_record(
    sa: &ServiceAccount,
    claims: &crate::crypto::jwt::Claims,
    record: &ServiceAccountToken,
) -> Result<(), AppError> {
    if !sa.is_active
        || record.revoked
        || record.expires_at <= chrono::Utc::now()
        || record.service_account_id != sa.id
        || claims.sub != sa.id
        || record.jti != claims.jti
        || record.scope != claims.scope
        || claims.sgen.unwrap_or(0) != sa.credential_generation
        || record.credential_generation != sa.credential_generation
    {
        return Err(AppError::Unauthorized(
            "Invalid service account token".into(),
        ));
    }
    Ok(())
}

pub async fn validate_access_token(
    db: &Database,
    claims: &jwt::Claims,
) -> AppResult<ServiceAccount> {
    let sa = get_service_account(db, &claims.sub).await?;
    let record = db
        .collection::<ServiceAccountToken>(SA_TOKENS)
        .find_one(doc! {"jti": &claims.jti})
        .await?
        .ok_or_else(|| AppError::Unauthorized("Service account token not found".into()))?;
    validate_token_record(&sa, claims, &record)?;
    Ok(sa)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_id_format() {
        let id = generate_client_id();
        assert!(id.starts_with("sa_"));
        assert_eq!(id.len(), 3 + 24); // "sa_" + 24 hex chars
    }

    #[test]
    fn client_secret_format() {
        let secret = generate_client_secret();
        assert!(secret.starts_with("sas_"));
        assert_eq!(secret.len(), 4 + 64); // "sas_" + 64 hex chars
    }

    #[test]
    fn client_ids_are_unique() {
        let id1 = generate_client_id();
        let id2 = generate_client_id();
        assert_ne!(id1, id2);
    }

    #[test]
    fn client_secrets_are_unique() {
        let s1 = generate_client_secret();
        let s2 = generate_client_secret();
        assert_ne!(s1, s2);
    }

    #[test]
    fn secret_hash_matches() {
        let secret = generate_client_secret();
        let hash1 = hash_token(&secret);
        let hash2 = hash_token(&secret);
        assert_eq!(hash1, hash2);
    }

    #[test]
    fn client_id_hex_chars_only() {
        let id = generate_client_id();
        let hex_part = &id[3..];
        assert!(hex_part.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn client_secret_hex_chars_only() {
        let secret = generate_client_secret();
        let hex_part = &secret[4..];
        assert!(hex_part.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn secret_prefix_matches_first_eight_chars() {
        let secret = generate_client_secret();
        let prefix = &secret[..8];
        assert!(prefix.starts_with("sas_"));
    }

    #[tokio::test]
    async fn create_service_account_happy_path() {
        let Some(db) = crate::test_utils::connect_test_database("sa_create_ok").await else {
            eprintln!("skipping: no local MongoDB available");
            return;
        };

        let creator_id = Uuid::new_v4().to_string();
        let (sa, raw_secret) = create_service_account(
            &db,
            "Test SA",
            Some("A test account"),
            "read write",
            &[],
            None,
            &creator_id,
        )
        .await
        .expect("create service account");

        assert_eq!(sa.name, "Test SA");
        assert_eq!(sa.description.as_deref(), Some("A test account"));
        assert_eq!(sa.allowed_scopes, "read write");
        assert!(sa.is_active);
        assert!(sa.client_id.starts_with("sa_"));
        assert!(raw_secret.starts_with("sas_"));
        assert_eq!(sa.created_by, creator_id);
        assert_eq!(sa.owner_user_id.as_deref(), Some(creator_id.as_str()));

        let stored = get_service_account(&db, &sa.id).await.expect("get sa");
        assert_eq!(stored.name, "Test SA");
        let stored_hash = hash_token(&raw_secret);
        assert_eq!(stored.client_secret_hash, stored_hash);
    }

    #[tokio::test]
    async fn create_service_account_empty_name_error() {
        let Some(db) = crate::test_utils::connect_test_database("sa_empty_name").await else {
            eprintln!("skipping: no local MongoDB available");
            return;
        };

        let err = create_service_account(&db, "", None, "read", &[], None, "creator")
            .await
            .expect_err("empty name");
        assert!(matches!(err, AppError::ValidationError(_)));
    }

    #[tokio::test]
    async fn create_service_account_long_name_error() {
        let Some(db) = crate::test_utils::connect_test_database("sa_long_name").await else {
            eprintln!("skipping: no local MongoDB available");
            return;
        };

        let long_name = "x".repeat(101);
        let err = create_service_account(&db, &long_name, None, "read", &[], None, "creator")
            .await
            .expect_err("long name");
        assert!(matches!(err, AppError::ValidationError(_)));
    }

    #[tokio::test]
    async fn create_service_account_empty_scopes_error() {
        let Some(db) = crate::test_utils::connect_test_database("sa_no_scope").await else {
            eprintln!("skipping: no local MongoDB available");
            return;
        };

        let err = create_service_account(&db, "SA", None, "", &[], None, "creator")
            .await
            .expect_err("empty scopes");
        assert!(matches!(err, AppError::ValidationError(_)));
    }

    #[tokio::test]
    async fn create_service_account_zero_rate_limit_error() {
        let Some(db) = crate::test_utils::connect_test_database("sa_zero_rl").await else {
            eprintln!("skipping: no local MongoDB available");
            return;
        };

        let err = create_service_account(&db, "SA", None, "read", &[], Some(0), "creator")
            .await
            .expect_err("zero rate limit");
        assert!(matches!(err, AppError::ValidationError(_)));
    }

    #[tokio::test]
    async fn create_service_account_description_too_long_error() {
        let Some(db) = crate::test_utils::connect_test_database("sa_long_desc").await else {
            eprintln!("skipping: no local MongoDB available");
            return;
        };

        let long_desc = "d".repeat(501);
        let err = create_service_account(&db, "SA", Some(&long_desc), "read", &[], None, "creator")
            .await
            .expect_err("long description");
        assert!(matches!(err, AppError::ValidationError(_)));
    }

    #[tokio::test]
    async fn create_service_account_with_rate_limit() {
        let Some(db) = crate::test_utils::connect_test_database("sa_with_rl").await else {
            eprintln!("skipping: no local MongoDB available");
            return;
        };

        let (sa, _) = create_service_account(&db, "RL SA", None, "read", &[], Some(50), "creator")
            .await
            .expect("create");
        assert_eq!(sa.rate_limit_override, Some(50));
    }

    #[tokio::test]
    async fn get_service_account_not_found() {
        let Some(db) = crate::test_utils::connect_test_database("sa_get_nf").await else {
            eprintln!("skipping: no local MongoDB available");
            return;
        };

        let err = get_service_account(&db, "nonexistent")
            .await
            .expect_err("not found");
        assert!(matches!(err, AppError::ServiceAccountNotFound(_)));
    }

    #[tokio::test]
    async fn create_service_account_invalid_role_ids_error() {
        let Some(db) = crate::test_utils::connect_test_database("sa_bad_roles").await else {
            eprintln!("skipping: no local MongoDB available");
            return;
        };

        let err = create_service_account(
            &db,
            "SA",
            None,
            "read",
            &["fake-role-id".to_string()],
            None,
            "creator",
        )
        .await
        .expect_err("bad role ids");
        assert!(matches!(err, AppError::ValidationError(_)));
    }
}

#[cfg(test)]
mod custom_scope_regression_tests {
    use super::*;
    use crate::test_utils::{connect_test_database, test_app_state};

    #[tokio::test]
    async fn custom_scopes_remain_permissive_and_token_subsets_are_exact() {
        let db = connect_test_database("sa_custom_scope_compat")
            .await
            .expect("MongoDB required");
        let state = test_app_state(db.clone());
        let owner = Uuid::new_v4().to_string();
        let original = "custom:read 未知:scope proxy:* custom:read";
        let (sa, secret) =
            create_service_account(&db, "Custom bot", None, original, &[], None, &owner)
                .await
                .unwrap();
        assert_eq!(sa.allowed_scopes, original);
        let token = authenticate_client_credentials(
            &db,
            &state.config,
            &state.jwt_keys,
            &sa.client_id,
            &secret,
            Some("custom:read"),
        )
        .await
        .unwrap();
        assert_eq!(token.scope, "custom:read");
        let claims =
            jwt::verify_token(&state.jwt_keys, &state.config, &token.access_token).unwrap();
        assert!(
            authenticate_client_credentials(
                &db,
                &state.config,
                &state.jwt_keys,
                &sa.client_id,
                &secret,
                Some("proxy")
            )
            .await
            .is_err()
        );
        let updated = "new:custom proxy:service:not-a-grant";
        let sa = update_service_account(
            &db,
            &sa.id,
            None,
            None,
            Some(updated),
            None,
            None,
            None,
            true,
            None,
        )
        .await
        .unwrap();
        assert_eq!(sa.allowed_scopes, updated);
        let record = db
            .collection::<ServiceAccountToken>(SA_TOKENS)
            .find_one(doc! { "jti": &claims.jti })
            .await
            .unwrap()
            .unwrap();
        assert!(
            !record.revoked,
            "Editing suggestions must not change existing token semantics"
        );
        assert_eq!(record.scope, "custom:read");
        let current = authenticate_client_credentials(
            &db,
            &state.config,
            &state.jwt_keys,
            &sa.client_id,
            &secret,
            None,
        )
        .await
        .unwrap();
        assert_eq!(current.scope, updated);
        assert!(!crate::mw::auth::scope_allows_rest_proxy(&current.scope));
    }
}

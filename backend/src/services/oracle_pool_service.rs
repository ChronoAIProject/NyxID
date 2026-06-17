//! Oracle pool management: capacity pools of browser worker tabs.
//!
//! A pool is owned by a person or org (`user_id`, polymorphic like
//! `Node` / `UserService`) and authenticates its workers with a single
//! rotatable pool worker token (`nyx_owk_...`, SHA-256 hash stored).
//! Consumers are gated by `visibility`; workers only ever authenticate
//! with the token, never with a NyxID account.

use chrono::Utc;
use futures::TryStreamExt;
use mongodb::bson::doc;

use crate::crypto::token::hash_token;
use crate::errors::{AppError, AppResult};
use crate::models::oracle_pool::{
    COLLECTION_NAME as ORACLE_POOLS, DEFAULT_MAX_QUEUE_LENGTH, DEFAULT_MAX_WORKERS,
    DEFAULT_PER_USER_MAX_INFLIGHT, DEFAULT_TASK_TIMEOUT_SECS, OraclePool, OraclePoolVisibility,
};
use crate::models::org_membership::{COLLECTION_NAME as ORG_MEMBERSHIPS, OrgMembership};
use crate::models::user::{COLLECTION_NAME as USERS, User, UserType};
use crate::services::org_service::{self, OwnerAccess};

const WORKER_TOKEN_PREFIX: &str = "nyx_owk_";

pub const MAX_POOLS_PER_OWNER: u64 = 10;

const MAX_NAME_LEN: usize = 128;
const MAX_DESCRIPTION_LEN: usize = 1024;
const MAX_URL_LEN: usize = 2048;
const MAX_MODEL_LABEL_LEN: usize = 128;

const MAX_WORKERS_CAP: u32 = 20;
const MAX_QUEUE_LENGTH_CAP: u32 = 1000;
const PER_USER_MAX_INFLIGHT_CAP: u32 = 100;
const TASK_TIMEOUT_SECS_MIN: u64 = 60;
const TASK_TIMEOUT_SECS_MAX: u64 = 86_400;

#[derive(Debug, Default)]
pub struct CreatePoolInput {
    pub slug: String,
    pub name: String,
    pub description: Option<String>,
    pub visibility: Option<OraclePoolVisibility>,
    pub chatgpt_project_url: Option<String>,
    pub default_model_label: Option<String>,
    pub allow_extract: Option<bool>,
    pub max_workers: Option<u32>,
    pub max_queue_length: Option<u32>,
    pub per_user_max_inflight: Option<u32>,
    pub task_timeout_secs: Option<u64>,
}

/// Fields a pool owner may update. `None` = leave unchanged.
#[derive(Debug, Default)]
pub struct UpdatePoolInput {
    pub name: Option<String>,
    pub description: Option<String>,
    pub visibility: Option<OraclePoolVisibility>,
    pub chatgpt_project_url: Option<String>,
    pub default_model_label: Option<String>,
    pub allow_extract: Option<bool>,
    pub max_workers: Option<u32>,
    pub max_queue_length: Option<u32>,
    pub per_user_max_inflight: Option<u32>,
    pub task_timeout_secs: Option<u64>,
    pub is_active: Option<bool>,
}

fn validate_slug(slug: &str) -> AppResult<()> {
    let ok = !slug.is_empty()
        && slug.len() <= 64
        && slug
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        && !slug.starts_with('-')
        && !slug.ends_with('-');
    if !ok {
        return Err(AppError::ValidationError(
            "Pool slug must be 1-64 chars of lowercase letters, digits, and inner hyphens"
                .to_string(),
        ));
    }
    Ok(())
}

fn validate_limits(
    max_workers: u32,
    max_queue_length: u32,
    per_user_max_inflight: u32,
    task_timeout_secs: u64,
) -> AppResult<()> {
    if max_workers == 0 || max_workers > MAX_WORKERS_CAP {
        return Err(AppError::ValidationError(format!(
            "max_workers must be 1-{MAX_WORKERS_CAP}"
        )));
    }
    if max_queue_length == 0 || max_queue_length > MAX_QUEUE_LENGTH_CAP {
        return Err(AppError::ValidationError(format!(
            "max_queue_length must be 1-{MAX_QUEUE_LENGTH_CAP}"
        )));
    }
    if per_user_max_inflight == 0 || per_user_max_inflight > PER_USER_MAX_INFLIGHT_CAP {
        return Err(AppError::ValidationError(format!(
            "per_user_max_inflight must be 1-{PER_USER_MAX_INFLIGHT_CAP}"
        )));
    }
    if !(TASK_TIMEOUT_SECS_MIN..=TASK_TIMEOUT_SECS_MAX).contains(&task_timeout_secs) {
        return Err(AppError::ValidationError(format!(
            "task_timeout_secs must be {TASK_TIMEOUT_SECS_MIN}-{TASK_TIMEOUT_SECS_MAX}"
        )));
    }
    Ok(())
}

fn validate_text_fields(
    name: &str,
    description: Option<&str>,
    project_url: Option<&str>,
    model_label: Option<&str>,
) -> AppResult<()> {
    if name.trim().is_empty() || name.len() > MAX_NAME_LEN {
        return Err(AppError::ValidationError(format!(
            "Pool name must be 1-{MAX_NAME_LEN} chars"
        )));
    }
    if description.is_some_and(|d| d.len() > MAX_DESCRIPTION_LEN) {
        return Err(AppError::ValidationError(format!(
            "description exceeds {MAX_DESCRIPTION_LEN} chars"
        )));
    }
    if let Some(url) = project_url {
        if url.len() > MAX_URL_LEN {
            return Err(AppError::ValidationError(format!(
                "chatgpt_project_url exceeds {MAX_URL_LEN} chars"
            )));
        }
        if !url.is_empty() && !url.starts_with("https://") {
            return Err(AppError::ValidationError(
                "chatgpt_project_url must be an https:// URL".to_string(),
            ));
        }
    }
    if model_label.is_some_and(|m| m.len() > MAX_MODEL_LABEL_LEN) {
        return Err(AppError::ValidationError(format!(
            "default_model_label exceeds {MAX_MODEL_LABEL_LEN} chars"
        )));
    }
    Ok(())
}

fn mint_worker_token() -> (String, String) {
    let raw = format!(
        "{WORKER_TOKEN_PREFIX}{}",
        hex::encode(rand::random::<[u8; 32]>())
    );
    let hash = hash_token(&raw);
    (raw, hash)
}

/// Create a pool owned by `owner_user_id` (already resolved: the caller's
/// own id, or an org id the caller administers — handlers verify org
/// admin rights before passing an org owner here).
///
/// Returns the pool and the raw worker token (shown exactly once).
pub async fn create_pool(
    db: &mongodb::Database,
    owner_user_id: &str,
    input: CreatePoolInput,
) -> AppResult<(OraclePool, String)> {
    validate_slug(&input.slug)?;
    let visibility = input.visibility.unwrap_or(OraclePoolVisibility::Private);
    let max_workers = input.max_workers.unwrap_or(DEFAULT_MAX_WORKERS);
    let max_queue_length = input.max_queue_length.unwrap_or(DEFAULT_MAX_QUEUE_LENGTH);
    let per_user_max_inflight = input
        .per_user_max_inflight
        .unwrap_or(DEFAULT_PER_USER_MAX_INFLIGHT);
    let task_timeout_secs = input.task_timeout_secs.unwrap_or(DEFAULT_TASK_TIMEOUT_SECS);
    validate_limits(
        max_workers,
        max_queue_length,
        per_user_max_inflight,
        task_timeout_secs,
    )?;
    validate_text_fields(
        &input.name,
        input.description.as_deref(),
        input.chatgpt_project_url.as_deref(),
        input.default_model_label.as_deref(),
    )?;

    // `org` visibility only makes sense for org-owned pools.
    if visibility == OraclePoolVisibility::Org {
        let owner = db
            .collection::<User>(USERS)
            .find_one(doc! { "_id": owner_user_id })
            .await?;
        let is_org = owner.is_some_and(|u| u.user_type == UserType::Org);
        if !is_org {
            return Err(AppError::ValidationError(
                "visibility=org requires an org-owned pool (use --org)".to_string(),
            ));
        }
    }

    let pool_count = db
        .collection::<OraclePool>(ORACLE_POOLS)
        .count_documents(doc! { "user_id": owner_user_id })
        .await?;
    if pool_count >= MAX_POOLS_PER_OWNER {
        return Err(AppError::Conflict(format!(
            "Maximum of {MAX_POOLS_PER_OWNER} oracle pools per owner reached"
        )));
    }

    let existing = db
        .collection::<OraclePool>(ORACLE_POOLS)
        .find_one(doc! { "slug": &input.slug })
        .await?;
    if existing.is_some() {
        return Err(AppError::OraclePoolSlugTaken(input.slug));
    }

    let (raw_token, token_hash) = mint_worker_token();
    let now = Utc::now();
    let pool = OraclePool {
        id: uuid::Uuid::new_v4().to_string(),
        user_id: owner_user_id.to_string(),
        slug: input.slug,
        name: input.name,
        description: input.description,
        visibility,
        worker_token_hash: token_hash,
        chatgpt_project_url: input.chatgpt_project_url.filter(|u| !u.is_empty()),
        default_model_label: input.default_model_label.filter(|m| !m.is_empty()),
        allow_extract: input.allow_extract.unwrap_or(false),
        max_workers,
        max_queue_length,
        per_user_max_inflight,
        task_timeout_secs,
        is_active: true,
        created_at: now,
        updated_at: now,
    };

    // The unique slug index backstops the pre-check above under races.
    db.collection::<OraclePool>(ORACLE_POOLS)
        .insert_one(&pool)
        .await
        .map_err(|e| {
            if is_duplicate_key(&e) {
                AppError::OraclePoolSlugTaken(pool.slug.clone())
            } else {
                AppError::DatabaseError(e)
            }
        })?;

    Ok((pool, raw_token))
}

pub(crate) fn is_duplicate_key(err: &mongodb::error::Error) -> bool {
    matches!(
        err.kind.as_ref(),
        mongodb::error::ErrorKind::Write(mongodb::error::WriteFailure::WriteError(we))
            if we.code == 11000
    )
}

/// Look up a pool by id or slug.
pub async fn get_pool(db: &mongodb::Database, id_or_slug: &str) -> AppResult<OraclePool> {
    db.collection::<OraclePool>(ORACLE_POOLS)
        .find_one(doc! { "$or": [ { "_id": id_or_slug }, { "slug": id_or_slug } ] })
        .await?
        .ok_or_else(|| AppError::OraclePoolNotFound(id_or_slug.to_string()))
}

/// Authenticate a worker request: hash the bearer token and find its pool.
/// Inactive pools reject workers — turning a pool off detaches its tabs.
pub async fn validate_worker_token(
    db: &mongodb::Database,
    raw_token: &str,
) -> AppResult<OraclePool> {
    if !raw_token.starts_with(WORKER_TOKEN_PREFIX) {
        return Err(AppError::OracleWorkerTokenInvalid);
    }
    let token_hash = hash_token(raw_token);
    db.collection::<OraclePool>(ORACLE_POOLS)
        .find_one(doc! { "worker_token_hash": &token_hash, "is_active": true })
        .await?
        .ok_or(AppError::OracleWorkerTokenInvalid)
}

/// Rotate the pool worker token. Returns the updated pool and the new raw
/// token (shown once). All currently-configured tabs must be re-paired.
pub async fn rotate_worker_token(
    db: &mongodb::Database,
    actor_user_id: &str,
    id_or_slug: &str,
) -> AppResult<(OraclePool, String)> {
    let pool = get_pool(db, id_or_slug).await?;
    ensure_can_manage(db, actor_user_id, &pool).await?;

    let (raw_token, token_hash) = mint_worker_token();
    db.collection::<OraclePool>(ORACLE_POOLS)
        .update_one(
            doc! { "_id": &pool.id },
            doc! { "$set": {
                "worker_token_hash": &token_hash,
                "updated_at": bson::DateTime::from_chrono(Utc::now()),
            } },
        )
        .await?;

    let updated = get_pool(db, &pool.id).await?;
    Ok((updated, raw_token))
}

/// Apply owner updates to a pool.
pub async fn update_pool(
    db: &mongodb::Database,
    actor_user_id: &str,
    id_or_slug: &str,
    input: UpdatePoolInput,
) -> AppResult<OraclePool> {
    let pool = get_pool(db, id_or_slug).await?;
    ensure_can_manage(db, actor_user_id, &pool).await?;

    let max_workers = input.max_workers.unwrap_or(pool.max_workers);
    let max_queue_length = input.max_queue_length.unwrap_or(pool.max_queue_length);
    let per_user_max_inflight = input
        .per_user_max_inflight
        .unwrap_or(pool.per_user_max_inflight);
    let task_timeout_secs = input.task_timeout_secs.unwrap_or(pool.task_timeout_secs);
    validate_limits(
        max_workers,
        max_queue_length,
        per_user_max_inflight,
        task_timeout_secs,
    )?;
    let name = input.name.unwrap_or_else(|| pool.name.clone());
    validate_text_fields(
        &name,
        input.description.as_deref(),
        input.chatgpt_project_url.as_deref(),
        input.default_model_label.as_deref(),
    )?;
    if input.visibility == Some(OraclePoolVisibility::Org) {
        let owner = db
            .collection::<User>(USERS)
            .find_one(doc! { "_id": &pool.user_id })
            .await?;
        if !owner.is_some_and(|u| u.user_type == UserType::Org) {
            return Err(AppError::ValidationError(
                "visibility=org requires an org-owned pool".to_string(),
            ));
        }
    }

    let mut set = doc! {
        "name": &name,
        "max_workers": max_workers,
        "max_queue_length": max_queue_length,
        "per_user_max_inflight": per_user_max_inflight,
        "task_timeout_secs": task_timeout_secs as i64,
        "updated_at": bson::DateTime::from_chrono(Utc::now()),
    };
    if let Some(v) = input.visibility {
        set.insert("visibility", v.as_str());
    }
    if let Some(d) = input.description {
        set.insert("description", d);
    }
    if let Some(u) = input.chatgpt_project_url {
        set.insert("chatgpt_project_url", u);
    }
    if let Some(m) = input.default_model_label {
        set.insert("default_model_label", m);
    }
    if let Some(allow_extract) = input.allow_extract {
        set.insert("allow_extract", allow_extract);
    }
    if let Some(active) = input.is_active {
        set.insert("is_active", active);
    }

    db.collection::<OraclePool>(ORACLE_POOLS)
        .update_one(doc! { "_id": &pool.id }, doc! { "$set": set })
        .await?;

    get_pool(db, &pool.id).await
}

/// Pools the actor can see: platform-visible ones, their own, and pools
/// owned by orgs they belong to.
pub async fn list_visible_pools(
    db: &mongodb::Database,
    actor_user_id: &str,
) -> AppResult<Vec<OraclePool>> {
    let org_ids: Vec<String> = db
        .collection::<OrgMembership>(ORG_MEMBERSHIPS)
        .find(doc! { "member_user_id": actor_user_id, "revoked_at": null })
        .await?
        .try_collect::<Vec<_>>()
        .await?
        .into_iter()
        .map(|m| m.org_user_id)
        .collect();

    let mut owner_ids = vec![actor_user_id.to_string()];
    owner_ids.extend(org_ids);

    let pools = db
        .collection::<OraclePool>(ORACLE_POOLS)
        .find(doc! { "$or": [
            { "visibility": "platform" },
            { "user_id": { "$in": owner_ids } },
        ] })
        .await?
        .try_collect::<Vec<_>>()
        .await?;
    Ok(pools)
}

/// Gate a metadata read by visibility alone. Unlike `ensure_can_submit`
/// this lets consumers (and owners) see inactive pools — `is_active:
/// false` is information, not a secret.
pub async fn ensure_can_view(
    db: &mongodb::Database,
    actor_user_id: &str,
    pool: &OraclePool,
) -> AppResult<()> {
    match pool.visibility {
        OraclePoolVisibility::Platform => Ok(()),
        OraclePoolVisibility::Org => {
            let access = org_service::resolve_owner_access(db, actor_user_id, &pool.user_id)
                .await
                .unwrap_or(OwnerAccess::Forbidden);
            if access.can_read() {
                Ok(())
            } else {
                Err(AppError::OraclePoolNotFound(pool.slug.clone()))
            }
        }
        OraclePoolVisibility::Private => {
            let access = org_service::resolve_owner_access(db, actor_user_id, &pool.user_id)
                .await
                .unwrap_or(OwnerAccess::Forbidden);
            if access.can_write() {
                Ok(())
            } else {
                Err(AppError::OraclePoolNotFound(pool.slug.clone()))
            }
        }
    }
}

/// Gate a submit (or other consumer read) by pool visibility.
pub async fn ensure_can_submit(
    db: &mongodb::Database,
    actor_user_id: &str,
    pool: &OraclePool,
) -> AppResult<()> {
    if !pool.is_active {
        return Err(AppError::OraclePoolInactive(pool.slug.clone()));
    }
    match pool.visibility {
        OraclePoolVisibility::Platform => Ok(()),
        OraclePoolVisibility::Org => {
            let access = org_service::resolve_owner_access(db, actor_user_id, &pool.user_id)
                .await
                .unwrap_or(OwnerAccess::Forbidden);
            if access.can_read() {
                Ok(())
            } else {
                Err(AppError::Forbidden(
                    "This oracle pool is restricted to members of its org".to_string(),
                ))
            }
        }
        OraclePoolVisibility::Private => {
            let access = org_service::resolve_owner_access(db, actor_user_id, &pool.user_id)
                .await
                .unwrap_or(OwnerAccess::Forbidden);
            if access.can_write() {
                Ok(())
            } else {
                Err(AppError::Forbidden(
                    "This oracle pool is private".to_string(),
                ))
            }
        }
    }
}

/// Gate management operations (update, rotate token): owner or org admin.
pub async fn ensure_can_manage(
    db: &mongodb::Database,
    actor_user_id: &str,
    pool: &OraclePool,
) -> AppResult<()> {
    let access = org_service::resolve_owner_access(db, actor_user_id, &pool.user_id)
        .await
        .unwrap_or(OwnerAccess::Forbidden);
    if access.can_write() {
        Ok(())
    } else {
        Err(AppError::Forbidden(
            "Only the pool owner (or an org admin) can manage this pool".to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::org_membership::OrgRole;
    use crate::models::user::UserType;
    use crate::test_utils::{connect_test_database, test_membership, test_user};

    #[test]
    fn slug_validation() {
        assert!(validate_slug("chatgpt-pro").is_ok());
        assert!(validate_slug("a").is_ok());
        assert!(validate_slug("p0ol-2").is_ok());
        assert!(validate_slug("").is_err());
        assert!(validate_slug("-leading").is_err());
        assert!(validate_slug("trailing-").is_err());
        assert!(validate_slug("UpperCase").is_err());
        assert!(validate_slug("under_score").is_err());
        assert!(validate_slug(&"x".repeat(65)).is_err());
    }

    #[test]
    fn limits_validation() {
        assert!(validate_limits(3, 50, 2, 14_400).is_ok());
        assert!(validate_limits(0, 50, 2, 14_400).is_err());
        assert!(validate_limits(21, 50, 2, 14_400).is_err());
        assert!(validate_limits(3, 0, 2, 14_400).is_err());
        assert!(validate_limits(3, 1001, 2, 14_400).is_err());
        assert!(validate_limits(3, 50, 0, 14_400).is_err());
        assert!(validate_limits(3, 50, 101, 14_400).is_err());
        assert!(validate_limits(3, 50, 2, 59).is_err());
        assert!(validate_limits(3, 50, 2, 86_401).is_err());
    }

    #[test]
    fn text_field_validation() {
        assert!(validate_text_fields("Pool", None, None, None).is_ok());
        assert!(validate_text_fields("", None, None, None).is_err());
        assert!(validate_text_fields("   ", None, None, None).is_err());
        assert!(validate_text_fields("Pool", Some(&"d".repeat(1025)), None, None).is_err());
        assert!(validate_text_fields("Pool", None, Some("http://insecure"), None).is_err());
        assert!(
            validate_text_fields(
                "Pool",
                None,
                Some("https://chatgpt.com/g/g-p-x/project"),
                None
            )
            .is_ok()
        );
        assert!(validate_text_fields("Pool", None, None, Some(&"m".repeat(129))).is_err());
    }

    #[test]
    fn worker_token_shape() {
        let (raw, hash) = mint_worker_token();
        assert!(raw.starts_with("nyx_owk_"));
        assert_eq!(raw.len(), "nyx_owk_".len() + 64);
        assert_eq!(hash.len(), 64);
        let (raw2, _) = mint_worker_token();
        assert_ne!(raw, raw2, "tokens must be random");
    }

    fn pool_input(slug: &str) -> CreatePoolInput {
        CreatePoolInput {
            slug: slug.to_string(),
            name: "Test Pool".to_string(),
            visibility: Some(OraclePoolVisibility::Platform),
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn create_get_and_token_validation() {
        let Some(db) = connect_test_database("oracle_pool_create").await else {
            return;
        };
        let owner = uuid::Uuid::new_v4().to_string();

        let (pool, raw_token) = create_pool(&db, &owner, pool_input("pool-a"))
            .await
            .expect("create pool");
        assert_eq!(pool.slug, "pool-a");
        assert!(pool.is_active);

        // Slug uniqueness.
        let dup = create_pool(&db, &owner, pool_input("pool-a")).await;
        assert!(matches!(dup, Err(AppError::OraclePoolSlugTaken(_))));

        // Lookup by id and slug.
        assert_eq!(get_pool(&db, &pool.id).await.unwrap().id, pool.id);
        assert_eq!(get_pool(&db, "pool-a").await.unwrap().id, pool.id);

        // Worker token authenticates to the right pool.
        let by_token = validate_worker_token(&db, &raw_token).await.unwrap();
        assert_eq!(by_token.id, pool.id);
        assert!(matches!(
            validate_worker_token(&db, "nyx_owk_wrong").await,
            Err(AppError::OracleWorkerTokenInvalid)
        ));
        assert!(matches!(
            validate_worker_token(&db, "not-a-token").await,
            Err(AppError::OracleWorkerTokenInvalid)
        ));

        // Rotation invalidates the old token.
        let (_, new_token) = rotate_worker_token(&db, &owner, "pool-a").await.unwrap();
        assert!(matches!(
            validate_worker_token(&db, &raw_token).await,
            Err(AppError::OracleWorkerTokenInvalid)
        ));
        assert_eq!(
            validate_worker_token(&db, &new_token).await.unwrap().id,
            pool.id
        );

        // Deactivating the pool detaches workers.
        update_pool(
            &db,
            &owner,
            "pool-a",
            UpdatePoolInput {
                is_active: Some(false),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert!(matches!(
            validate_worker_token(&db, &new_token).await,
            Err(AppError::OracleWorkerTokenInvalid)
        ));

        db.drop().await.ok();
    }

    #[tokio::test]
    async fn visibility_acl() {
        let Some(db) = connect_test_database("oracle_pool_acl").await else {
            return;
        };
        let owner = uuid::Uuid::new_v4().to_string();
        let org_id = uuid::Uuid::new_v4().to_string();
        let member = uuid::Uuid::new_v4().to_string();
        let stranger = uuid::Uuid::new_v4().to_string();

        let users = db.collection::<crate::models::user::User>("users");
        users
            .insert_one(test_user(&owner, UserType::Person))
            .await
            .unwrap();
        users
            .insert_one(test_user(&org_id, UserType::Org))
            .await
            .unwrap();
        users
            .insert_one(test_user(&member, UserType::Person))
            .await
            .unwrap();
        users
            .insert_one(test_user(&stranger, UserType::Person))
            .await
            .unwrap();
        db.collection::<OrgMembership>(ORG_MEMBERSHIPS)
            .insert_one(test_membership(&org_id, &member, OrgRole::Member, None))
            .await
            .unwrap();

        // visibility=org on a person-owned pool is rejected.
        let bad = create_pool(
            &db,
            &owner,
            CreatePoolInput {
                visibility: Some(OraclePoolVisibility::Org),
                ..pool_input("person-org")
            },
        )
        .await;
        assert!(matches!(bad, Err(AppError::ValidationError(_))));

        // Platform pool: anyone can submit.
        let (platform_pool, _) = create_pool(&db, &owner, pool_input("platform-pool"))
            .await
            .unwrap();
        ensure_can_submit(&db, &stranger, &platform_pool)
            .await
            .unwrap();

        // Org pool: members yes, strangers no.
        let (org_pool, _) = create_pool(
            &db,
            &org_id,
            CreatePoolInput {
                visibility: Some(OraclePoolVisibility::Org),
                ..pool_input("org-pool")
            },
        )
        .await
        .unwrap();
        ensure_can_submit(&db, &member, &org_pool).await.unwrap();
        assert!(matches!(
            ensure_can_submit(&db, &stranger, &org_pool).await,
            Err(AppError::Forbidden(_))
        ));

        // Private person pool: only the owner.
        let (private_pool, _) = create_pool(
            &db,
            &owner,
            CreatePoolInput {
                visibility: Some(OraclePoolVisibility::Private),
                ..pool_input("private-pool")
            },
        )
        .await
        .unwrap();
        ensure_can_submit(&db, &owner, &private_pool).await.unwrap();
        assert!(matches!(
            ensure_can_submit(&db, &stranger, &private_pool).await,
            Err(AppError::Forbidden(_))
        ));

        // Inactive pool rejects submits regardless of visibility.
        update_pool(
            &db,
            &owner,
            "platform-pool",
            UpdatePoolInput {
                is_active: Some(false),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        let inactive = get_pool(&db, "platform-pool").await.unwrap();
        assert!(matches!(
            ensure_can_submit(&db, &stranger, &inactive).await,
            Err(AppError::OraclePoolInactive(_))
        ));

        // Manage: org member (non-admin) cannot, stranger cannot.
        assert!(matches!(
            ensure_can_manage(&db, &member, &org_pool).await,
            Err(AppError::Forbidden(_))
        ));
        assert!(matches!(
            ensure_can_manage(&db, &stranger, &org_pool).await,
            Err(AppError::Forbidden(_))
        ));

        // Visible pools per actor.
        let member_view = list_visible_pools(&db, &member).await.unwrap();
        let member_slugs: Vec<&str> = member_view.iter().map(|p| p.slug.as_str()).collect();
        assert!(member_slugs.contains(&"platform-pool"));
        assert!(member_slugs.contains(&"org-pool"));
        assert!(!member_slugs.contains(&"private-pool"));

        let stranger_view = list_visible_pools(&db, &stranger).await.unwrap();
        let stranger_slugs: Vec<&str> = stranger_view.iter().map(|p| p.slug.as_str()).collect();
        assert!(stranger_slugs.contains(&"platform-pool"));
        assert!(!stranger_slugs.contains(&"org-pool"));

        db.drop().await.ok();
    }
}

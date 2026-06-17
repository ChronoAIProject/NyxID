//! Compute pool management for shared GPU / local-compute workers.

use chrono::Utc;
use mongodb::bson::doc;

use crate::errors::{AppError, AppResult};
use crate::models::compute_pool::{
    COLLECTION_NAME as COMPUTE_POOLS, ComputePool, ComputePoolVisibility, ComputeSchedulingPolicy,
    DEFAULT_MAX_QUEUE_LENGTH, DEFAULT_MAX_WORKERS, DEFAULT_PER_USER_MAX_INFLIGHT,
    DEFAULT_TASK_TIMEOUT_SECS,
};
use crate::models::org_membership::{COLLECTION_NAME as ORG_MEMBERSHIPS, OrgMembership};
use crate::models::user::{COLLECTION_NAME as USERS, User, UserType};
use crate::services::org_service;

const WORKER_TOKEN_PREFIX: &str = "nyx_cwk_";
const MAX_POOLS_PER_OWNER: u64 = 20;
const MAX_NAME_LEN: usize = 120;
const MAX_DESCRIPTION_LEN: usize = 1024;
const MAX_WORKERS_CAP: u32 = 512;
const MAX_QUEUE_CAP: u32 = 20_000;
const MAX_PER_USER_INFLIGHT_CAP: u32 = 256;
const TASK_TIMEOUT_SECS_MIN: u64 = 30;
const TASK_TIMEOUT_SECS_MAX: u64 = 86_400;

#[derive(Debug, Clone)]
pub struct CreateComputePoolInput {
    pub slug: String,
    pub name: String,
    pub description: Option<String>,
    pub visibility: Option<ComputePoolVisibility>,
    pub scheduling_policy: Option<ComputeSchedulingPolicy>,
    pub max_workers: Option<u32>,
    pub max_queue_length: Option<u32>,
    pub per_user_max_inflight: Option<u32>,
    pub task_timeout_secs: Option<u64>,
}

#[derive(Debug, Clone)]
pub struct UpdateComputePoolInput {
    pub name: Option<String>,
    pub description: Option<String>,
    pub visibility: Option<ComputePoolVisibility>,
    pub scheduling_policy: Option<ComputeSchedulingPolicy>,
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
        && !slug.ends_with('-')
        && !slug.contains("--");
    if !ok {
        return Err(AppError::ValidationError(
            "pool slug must be 1-64 chars of lowercase letters, digits, and inner hyphens"
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
    if max_queue_length == 0 || max_queue_length > MAX_QUEUE_CAP {
        return Err(AppError::ValidationError(format!(
            "max_queue_length must be 1-{MAX_QUEUE_CAP}"
        )));
    }
    if per_user_max_inflight == 0 || per_user_max_inflight > MAX_PER_USER_INFLIGHT_CAP {
        return Err(AppError::ValidationError(format!(
            "per_user_max_inflight must be 1-{MAX_PER_USER_INFLIGHT_CAP}"
        )));
    }
    if !(TASK_TIMEOUT_SECS_MIN..=TASK_TIMEOUT_SECS_MAX).contains(&task_timeout_secs) {
        return Err(AppError::ValidationError(format!(
            "task_timeout_secs must be {TASK_TIMEOUT_SECS_MIN}-{TASK_TIMEOUT_SECS_MAX}"
        )));
    }
    Ok(())
}

fn validate_text_fields(name: &str, description: Option<&str>) -> AppResult<()> {
    if name.trim().is_empty() || name.len() > MAX_NAME_LEN {
        return Err(AppError::ValidationError(format!(
            "pool name must be 1-{MAX_NAME_LEN} chars"
        )));
    }
    if description.is_some_and(|d| d.len() > MAX_DESCRIPTION_LEN) {
        return Err(AppError::ValidationError(format!(
            "description exceeds {MAX_DESCRIPTION_LEN} chars"
        )));
    }
    Ok(())
}

fn hash_token(raw: &str) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(raw.as_bytes()))
}

fn mint_worker_token() -> (String, String) {
    let raw = format!(
        "{WORKER_TOKEN_PREFIX}{}",
        hex::encode(rand::random::<[u8; 32]>())
    );
    let hash = hash_token(&raw);
    (raw, hash)
}

pub(crate) fn is_duplicate_key(err: &mongodb::error::Error) -> bool {
    matches!(*err.kind, mongodb::error::ErrorKind::Write(_)) && err.to_string().contains("E11000")
}

pub async fn create_pool(
    db: &mongodb::Database,
    owner_user_id: &str,
    input: CreateComputePoolInput,
) -> AppResult<(ComputePool, String)> {
    validate_slug(&input.slug)?;
    validate_text_fields(&input.name, input.description.as_deref())?;

    let visibility = input.visibility.unwrap_or(ComputePoolVisibility::Private);
    let scheduling_policy = input
        .scheduling_policy
        .unwrap_or(ComputeSchedulingPolicy::ModelFit);
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

    if visibility == ComputePoolVisibility::Org {
        let owner = db
            .collection::<User>(USERS)
            .find_one(doc! { "_id": owner_user_id })
            .await?;
        let is_org = owner.is_some_and(|u| u.user_type == UserType::Org);
        if !is_org {
            return Err(AppError::BadRequest(
                "visibility=org requires an org-owned pool (use --org)".to_string(),
            ));
        }
    }

    let pool_count = db
        .collection::<ComputePool>(COMPUTE_POOLS)
        .count_documents(doc! { "user_id": owner_user_id })
        .await?;
    if pool_count >= MAX_POOLS_PER_OWNER {
        return Err(AppError::BadRequest(format!(
            "maximum of {MAX_POOLS_PER_OWNER} compute pools per owner reached"
        )));
    }

    let (raw_token, token_hash) = mint_worker_token();
    let now = Utc::now();
    let pool = ComputePool {
        id: uuid::Uuid::new_v4().to_string(),
        user_id: owner_user_id.to_string(),
        slug: input.slug,
        name: input.name,
        description: input.description,
        visibility,
        scheduling_policy,
        worker_token_hash: token_hash,
        max_workers,
        max_queue_length,
        per_user_max_inflight,
        task_timeout_secs,
        is_active: true,
        created_at: now,
        updated_at: now,
    };

    if let Err(e) = db
        .collection::<ComputePool>(COMPUTE_POOLS)
        .insert_one(&pool)
        .await
    {
        if is_duplicate_key(&e) {
            return Err(AppError::ComputePoolSlugTaken(pool.slug));
        }
        return Err(e.into());
    }

    Ok((pool, raw_token))
}

pub async fn get_pool(db: &mongodb::Database, id_or_slug: &str) -> AppResult<ComputePool> {
    db.collection::<ComputePool>(COMPUTE_POOLS)
        .find_one(doc! { "$or": [ { "_id": id_or_slug }, { "slug": id_or_slug } ] })
        .await?
        .ok_or_else(|| AppError::ComputePoolNotFound(id_or_slug.to_string()))
}

pub async fn validate_worker_token(
    db: &mongodb::Database,
    raw_token: &str,
) -> AppResult<ComputePool> {
    if !raw_token.starts_with(WORKER_TOKEN_PREFIX) {
        return Err(AppError::ComputeWorkerTokenInvalid);
    }
    let token_hash = hash_token(raw_token);
    db.collection::<ComputePool>(COMPUTE_POOLS)
        .find_one(doc! { "worker_token_hash": token_hash, "is_active": true })
        .await?
        .ok_or(AppError::ComputeWorkerTokenInvalid)
}

pub async fn rotate_worker_token(
    db: &mongodb::Database,
    actor_user_id: &str,
    id_or_slug: &str,
) -> AppResult<(ComputePool, String)> {
    let pool = get_pool(db, id_or_slug).await?;
    ensure_can_manage(db, actor_user_id, &pool).await?;
    let (raw_token, token_hash) = mint_worker_token();
    db.collection::<ComputePool>(COMPUTE_POOLS)
        .update_one(
            doc! { "_id": &pool.id },
            doc! { "$set": {
                "worker_token_hash": token_hash,
                "updated_at": bson::DateTime::from_chrono(Utc::now()),
            } },
        )
        .await?;
    Ok((get_pool(db, &pool.id).await?, raw_token))
}

pub async fn update_pool(
    db: &mongodb::Database,
    actor_user_id: &str,
    id_or_slug: &str,
    input: UpdateComputePoolInput,
) -> AppResult<ComputePool> {
    let pool = get_pool(db, id_or_slug).await?;
    ensure_can_manage(db, actor_user_id, &pool).await?;

    let name = input.name.unwrap_or_else(|| pool.name.clone());
    let description = input.description.or(pool.description.clone());
    validate_text_fields(&name, description.as_deref())?;
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

    if input.visibility == Some(ComputePoolVisibility::Org) {
        let owner = db
            .collection::<User>(USERS)
            .find_one(doc! { "_id": &pool.user_id })
            .await?;
        let is_org = owner.is_some_and(|u| u.user_type == UserType::Org);
        if !is_org {
            return Err(AppError::BadRequest(
                "visibility=org requires an org-owned pool".to_string(),
            ));
        }
    }

    let mut set = doc! {
        "name": name,
        "max_workers": max_workers as i64,
        "max_queue_length": max_queue_length as i64,
        "per_user_max_inflight": per_user_max_inflight as i64,
        "task_timeout_secs": task_timeout_secs as i64,
        "updated_at": bson::DateTime::from_chrono(Utc::now()),
    };
    match description {
        Some(d) => set.insert("description", d),
        None => set.insert("description", bson::Bson::Null),
    };
    if let Some(v) = input.visibility {
        set.insert("visibility", v.as_str());
    }
    if let Some(p) = input.scheduling_policy {
        set.insert("scheduling_policy", p.as_str());
    }
    if let Some(active) = input.is_active {
        set.insert("is_active", active);
    }

    db.collection::<ComputePool>(COMPUTE_POOLS)
        .update_one(doc! { "_id": &pool.id }, doc! { "$set": set })
        .await?;
    get_pool(db, &pool.id).await
}

pub async fn list_visible_pools(
    db: &mongodb::Database,
    actor_user_id: &str,
) -> AppResult<Vec<ComputePool>> {
    use futures::TryStreamExt;
    let org_ids: Vec<String> = db
        .collection::<OrgMembership>(ORG_MEMBERSHIPS)
        .find(doc! { "user_id": actor_user_id, "is_active": true })
        .await?
        .try_collect::<Vec<_>>()
        .await?
        .into_iter()
        .map(|m| m.org_user_id)
        .collect();

    let mut owner_ids = vec![actor_user_id.to_string()];
    owner_ids.extend(org_ids);

    Ok(db
        .collection::<ComputePool>(COMPUTE_POOLS)
        .find(doc! {
            "$or": [
                { "visibility": "platform" },
                { "user_id": { "$in": owner_ids } },
            ]
        })
        .await?
        .try_collect()
        .await?)
}

pub async fn ensure_can_view(
    db: &mongodb::Database,
    actor_user_id: &str,
    pool: &ComputePool,
) -> AppResult<()> {
    match pool.visibility {
        ComputePoolVisibility::Platform => Ok(()),
        ComputePoolVisibility::Org => {
            let access = org_service::resolve_owner_access(db, actor_user_id, &pool.user_id)
                .await
                .map_err(|_| AppError::ComputePoolNotFound(pool.slug.clone()))?;
            if access.can_read() || pool.user_id == actor_user_id {
                Ok(())
            } else {
                Err(AppError::ComputePoolNotFound(pool.slug.clone()))
            }
        }
        ComputePoolVisibility::Private => {
            let access = org_service::resolve_owner_access(db, actor_user_id, &pool.user_id)
                .await
                .map_err(|_| AppError::ComputePoolNotFound(pool.slug.clone()))?;
            if access.can_write() || pool.user_id == actor_user_id {
                Ok(())
            } else {
                Err(AppError::ComputePoolNotFound(pool.slug.clone()))
            }
        }
    }
}

pub async fn ensure_can_submit(
    db: &mongodb::Database,
    actor_user_id: &str,
    pool: &ComputePool,
) -> AppResult<()> {
    if !pool.is_active {
        return Err(AppError::ComputePoolInactive(pool.slug.clone()));
    }
    match pool.visibility {
        ComputePoolVisibility::Platform => Ok(()),
        ComputePoolVisibility::Org => {
            let access =
                org_service::resolve_owner_access(db, actor_user_id, &pool.user_id).await?;
            if access.can_read() || pool.user_id == actor_user_id {
                Ok(())
            } else {
                Err(AppError::Forbidden(
                    "This compute pool is restricted to members of its org".to_string(),
                ))
            }
        }
        ComputePoolVisibility::Private => {
            let access =
                org_service::resolve_owner_access(db, actor_user_id, &pool.user_id).await?;
            if access.can_write() || pool.user_id == actor_user_id {
                Ok(())
            } else {
                Err(AppError::Forbidden(
                    "This compute pool is private".to_string(),
                ))
            }
        }
    }
}

pub async fn ensure_can_manage(
    db: &mongodb::Database,
    actor_user_id: &str,
    pool: &ComputePool,
) -> AppResult<()> {
    let access = org_service::resolve_owner_access(db, actor_user_id, &pool.user_id).await?;
    if access.can_write() || pool.user_id == actor_user_id {
        Ok(())
    } else {
        Err(AppError::Forbidden(
            "Only the pool owner (or an org admin) can manage this compute pool".to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::connect_test_database;

    fn pool_input(slug: &str) -> CreateComputePoolInput {
        CreateComputePoolInput {
            slug: slug.to_string(),
            name: "Test Compute Pool".to_string(),
            description: None,
            visibility: Some(ComputePoolVisibility::Private),
            scheduling_policy: Some(ComputeSchedulingPolicy::ModelFit),
            max_workers: Some(2),
            max_queue_length: Some(8),
            per_user_max_inflight: Some(2),
            task_timeout_secs: Some(120),
        }
    }

    fn update_input(is_active: Option<bool>) -> UpdateComputePoolInput {
        UpdateComputePoolInput {
            name: None,
            description: None,
            visibility: None,
            scheduling_policy: None,
            max_workers: None,
            max_queue_length: None,
            per_user_max_inflight: None,
            task_timeout_secs: None,
            is_active,
        }
    }

    #[test]
    fn slug_validation() {
        assert!(validate_slug("home-4060").is_ok());
        assert!(validate_slug("a").is_ok());
        assert!(validate_slug("pool2").is_ok());
        assert!(validate_slug("").is_err());
        assert!(validate_slug("-leading").is_err());
        assert!(validate_slug("trailing-").is_err());
        assert!(validate_slug("two--hyphens").is_err());
        assert!(validate_slug("UpperCase").is_err());
        assert!(validate_slug("under_score").is_err());
        assert!(validate_slug(&"x".repeat(65)).is_err());
    }

    #[test]
    fn limits_validation() {
        assert!(validate_limits(1, 1, 1, 30).is_ok());
        assert!(
            validate_limits(
                MAX_WORKERS_CAP,
                MAX_QUEUE_CAP,
                MAX_PER_USER_INFLIGHT_CAP,
                86_400
            )
            .is_ok()
        );
        assert!(validate_limits(0, 10, 1, 120).is_err());
        assert!(validate_limits(MAX_WORKERS_CAP + 1, 10, 1, 120).is_err());
        assert!(validate_limits(1, 0, 1, 120).is_err());
        assert!(validate_limits(1, MAX_QUEUE_CAP + 1, 1, 120).is_err());
        assert!(validate_limits(1, 10, 0, 120).is_err());
        assert!(validate_limits(1, 10, MAX_PER_USER_INFLIGHT_CAP + 1, 120).is_err());
        assert!(validate_limits(1, 10, 1, 29).is_err());
        assert!(validate_limits(1, 10, 1, 86_401).is_err());
    }

    #[test]
    fn text_field_validation() {
        assert!(validate_text_fields("Pool", None).is_ok());
        assert!(validate_text_fields("", None).is_err());
        assert!(validate_text_fields("   ", None).is_err());
        assert!(validate_text_fields(&"n".repeat(MAX_NAME_LEN + 1), None).is_err());
        assert!(validate_text_fields("Pool", Some(&"d".repeat(MAX_DESCRIPTION_LEN + 1))).is_err());
    }

    #[test]
    fn worker_token_shape() {
        let (raw, hash) = mint_worker_token();
        assert!(raw.starts_with(WORKER_TOKEN_PREFIX));
        assert_eq!(raw.len(), WORKER_TOKEN_PREFIX.len() + 64);
        assert_eq!(hash.len(), 64);

        let (raw2, hash2) = mint_worker_token();
        assert_ne!(raw, raw2, "worker tokens must be random");
        assert_ne!(hash, hash2, "worker token hashes must be random");
    }

    #[tokio::test]
    async fn create_get_rotate_and_deactivate_worker_token() {
        let Some(db) = connect_test_database("compute_pool_token").await else {
            return;
        };
        crate::db::ensure_indexes(&db)
            .await
            .expect("ensure indexes");

        let owner = uuid::Uuid::new_v4().to_string();
        let (pool, raw_token) = create_pool(&db, &owner, pool_input("compute-pool-a"))
            .await
            .expect("create pool");
        assert_eq!(pool.slug, "compute-pool-a");
        assert!(pool.is_active);
        assert_ne!(pool.worker_token_hash, raw_token);
        assert_eq!(pool.worker_token_hash.len(), 64);

        let duplicate = create_pool(&db, &owner, pool_input("compute-pool-a")).await;
        assert!(matches!(duplicate, Err(AppError::ComputePoolSlugTaken(_))));

        assert_eq!(get_pool(&db, &pool.id).await.unwrap().id, pool.id);
        assert_eq!(get_pool(&db, "compute-pool-a").await.unwrap().id, pool.id);

        let token_pool = validate_worker_token(&db, &raw_token).await.unwrap();
        assert_eq!(token_pool.id, pool.id);
        assert!(matches!(
            validate_worker_token(&db, "nyx_cwk_wrong").await,
            Err(AppError::ComputeWorkerTokenInvalid)
        ));
        assert!(matches!(
            validate_worker_token(&db, "not-a-token").await,
            Err(AppError::ComputeWorkerTokenInvalid)
        ));

        let (_, new_token) = rotate_worker_token(&db, &owner, "compute-pool-a")
            .await
            .unwrap();
        assert!(matches!(
            validate_worker_token(&db, &raw_token).await,
            Err(AppError::ComputeWorkerTokenInvalid)
        ));
        assert_eq!(
            validate_worker_token(&db, &new_token).await.unwrap().id,
            pool.id
        );

        update_pool(&db, &owner, "compute-pool-a", update_input(Some(false)))
            .await
            .unwrap();
        assert!(matches!(
            validate_worker_token(&db, &new_token).await,
            Err(AppError::ComputeWorkerTokenInvalid)
        ));

        db.drop().await.ok();
    }
}

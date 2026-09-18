use chrono::Utc;
use futures::TryStreamExt;
use mongodb::bson::{self, doc};
use uuid::Uuid;

use crate::errors::{AppError, AppResult};
use crate::models::service_pool::{
    COLLECTION_NAME as SERVICE_POOLS, PoolStrategy, ServicePool, ServicePoolMember,
};
use crate::models::user_service::{COLLECTION_NAME as USER_SERVICES, UserService};
use crate::services::user_service_service;

pub const MAX_POOL_MEMBERS: usize = 50;

const MAX_NAME_LEN: usize = 128;
const MAX_DESCRIPTION_LEN: usize = 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PoolSelection {
    pub pool_id: String,
    pub pool_slug: String,
    pub strategy: PoolStrategy,
    pub selected_member_id: String,
    pub tick: i64,
}

#[derive(Debug)]
pub struct CreatePoolInput {
    pub slug: String,
    pub name: String,
    pub description: Option<String>,
    pub strategy: PoolStrategy,
    pub members: Vec<ServicePoolMember>,
    pub is_active: Option<bool>,
}

#[derive(Debug, Default)]
pub struct UpdatePoolInput {
    pub slug: Option<String>,
    pub name: Option<String>,
    pub description: Option<String>,
    pub strategy: Option<PoolStrategy>,
    pub members: Option<Vec<ServicePoolMember>>,
    pub is_active: Option<bool>,
}

fn validate_text_fields(name: &str, description: Option<&str>) -> AppResult<()> {
    if name.trim().is_empty() || name.len() > MAX_NAME_LEN {
        return Err(AppError::ValidationError(format!(
            "Pool name must be 1-{MAX_NAME_LEN} characters"
        )));
    }
    if description.is_some_and(|d| d.len() > MAX_DESCRIPTION_LEN) {
        return Err(AppError::ValidationError(format!(
            "description must not exceed {MAX_DESCRIPTION_LEN} characters"
        )));
    }
    Ok(())
}

fn normalize_members(members: Vec<ServicePoolMember>) -> AppResult<Vec<ServicePoolMember>> {
    if members.len() > MAX_POOL_MEMBERS {
        return Err(AppError::ServicePoolMemberInvalid(format!(
            "Service pools may contain at most {MAX_POOL_MEMBERS} members"
        )));
    }

    let mut seen = std::collections::HashSet::new();
    let mut out = Vec::with_capacity(members.len());
    for mut member in members {
        if member.user_service_id.trim().is_empty() {
            return Err(AppError::ServicePoolMemberInvalid(
                "Pool member user_service_id must not be empty".to_string(),
            ));
        }
        if !seen.insert(member.user_service_id.clone()) {
            return Err(AppError::ServicePoolMemberInvalid(format!(
                "Duplicate pool member '{}'",
                member.user_service_id
            )));
        }
        member.weight = member.weight.max(1);
        out.push(member);
    }
    Ok(out)
}

async fn validate_members_owned_and_active(
    db: &mongodb::Database,
    owner_id: &str,
    members: &[ServicePoolMember],
) -> AppResult<()> {
    for member in members {
        let service =
            crate::services::service_history::collection::<UserService>(db, USER_SERVICES)
                .find_one(doc! {
                    "_id": &member.user_service_id,
                    "user_id": owner_id,
                    "is_active": true,
                })
                .await?;
        if service.is_none() {
            return Err(AppError::ServicePoolMemberInvalid(format!(
                "Pool member '{}' must be an active UserService owned by the same owner",
                member.user_service_id
            )));
        }
    }
    Ok(())
}

async fn ensure_slug_available(
    db: &mongodb::Database,
    owner_id: &str,
    slug: &str,
    exclude_pool_id: Option<&str>,
) -> AppResult<()> {
    user_service_service::validate_slug(slug)?;

    if crate::services::service_history::collection::<UserService>(db, USER_SERVICES)
        .find_one(doc! { "user_id": owner_id, "slug": slug })
        .await?
        .is_some()
    {
        return Err(AppError::ServicePoolSlugTaken(slug.to_string()));
    }

    let mut filter = doc! {
        "user_id": owner_id,
        "slug": slug,
    };
    if let Some(pool_id) = exclude_pool_id {
        filter.insert("_id", doc! { "$ne": pool_id });
    }

    if db
        .collection::<ServicePool>(SERVICE_POOLS)
        .find_one(filter)
        .await?
        .is_some()
    {
        return Err(AppError::ServicePoolSlugTaken(slug.to_string()));
    }

    Ok(())
}

pub async fn list_pools(db: &mongodb::Database, owner_id: &str) -> AppResult<Vec<ServicePool>> {
    Ok(db
        .collection::<ServicePool>(SERVICE_POOLS)
        .find(doc! { "user_id": owner_id })
        .sort(doc! { "created_at": -1 })
        .await?
        .try_collect()
        .await?)
}

pub async fn get_pool(
    db: &mongodb::Database,
    owner_id: &str,
    pool_id: &str,
) -> AppResult<ServicePool> {
    db.collection::<ServicePool>(SERVICE_POOLS)
        .find_one(doc! { "_id": pool_id, "user_id": owner_id })
        .await?
        .ok_or_else(|| AppError::ServicePoolNotFound(pool_id.to_string()))
}

pub async fn find_pool_by_slug(
    db: &mongodb::Database,
    owner_id: &str,
    slug: &str,
) -> AppResult<Option<ServicePool>> {
    Ok(db
        .collection::<ServicePool>(SERVICE_POOLS)
        .find_one(doc! { "user_id": owner_id, "slug": slug, "is_active": true })
        .await?)
}

pub async fn create_pool(
    db: &mongodb::Database,
    owner_id: &str,
    input: CreatePoolInput,
) -> AppResult<ServicePool> {
    ensure_slug_available(db, owner_id, &input.slug, None).await?;
    validate_text_fields(&input.name, input.description.as_deref())?;
    let members = normalize_members(input.members)?;
    validate_members_owned_and_active(db, owner_id, &members).await?;

    let now = Utc::now();
    let pool = ServicePool {
        id: Uuid::new_v4().to_string(),
        user_id: owner_id.to_string(),
        slug: input.slug,
        name: input.name,
        description: input.description,
        strategy: input.strategy,
        members,
        rr_counter: 0,
        is_active: input.is_active.unwrap_or(true),
        created_at: now,
        updated_at: now,
    };

    db.collection::<ServicePool>(SERVICE_POOLS)
        .insert_one(&pool)
        .await
        .map_err(|e| {
            if is_duplicate_key(&e) {
                AppError::ServicePoolSlugTaken(pool.slug.clone())
            } else {
                AppError::DatabaseError(e)
            }
        })?;

    Ok(pool)
}

pub async fn update_pool(
    db: &mongodb::Database,
    owner_id: &str,
    pool_id: &str,
    input: UpdatePoolInput,
) -> AppResult<ServicePool> {
    let current = get_pool(db, owner_id, pool_id).await?;
    let slug = input.slug.unwrap_or_else(|| current.slug.clone());
    if slug != current.slug {
        ensure_slug_available(db, owner_id, &slug, Some(&current.id)).await?;
    } else {
        user_service_service::validate_slug(&slug)?;
    }

    let name = input.name.unwrap_or_else(|| current.name.clone());
    let description = input.description.or(current.description.clone());
    validate_text_fields(&name, description.as_deref())?;
    let members = match input.members {
        Some(members) => {
            let members = normalize_members(members)?;
            validate_members_owned_and_active(db, owner_id, &members).await?;
            Some(members)
        }
        None => None,
    };

    let mut set = doc! {
        "slug": &slug,
        "name": &name,
        "updated_at": bson::DateTime::from_chrono(Utc::now()),
    };
    match description {
        Some(value) => {
            set.insert("description", value);
        }
        None => {
            set.insert("description", bson::Bson::Null);
        }
    }
    if let Some(strategy) = input.strategy {
        set.insert("strategy", strategy.as_str());
    }
    if let Some(members) = members {
        set.insert(
            "members",
            bson::to_bson(&members)
                .map_err(|e| AppError::Internal(format!("BSON serialization error: {e}")))?,
        );
    }
    if let Some(is_active) = input.is_active {
        set.insert("is_active", is_active);
    }

    let result = db
        .collection::<ServicePool>(SERVICE_POOLS)
        .update_one(
            doc! { "_id": pool_id, "user_id": owner_id },
            doc! { "$set": set },
        )
        .await
        .map_err(|e| {
            if is_duplicate_key(&e) {
                AppError::ServicePoolSlugTaken(slug.clone())
            } else {
                AppError::DatabaseError(e)
            }
        })?;
    if result.matched_count == 0 {
        return Err(AppError::ServicePoolNotFound(pool_id.to_string()));
    }

    get_pool(db, owner_id, pool_id).await
}

pub async fn delete_pool(db: &mongodb::Database, owner_id: &str, pool_id: &str) -> AppResult<()> {
    let result = db
        .collection::<ServicePool>(SERVICE_POOLS)
        .delete_one(doc! { "_id": pool_id, "user_id": owner_id })
        .await?;
    if result.deleted_count == 0 {
        return Err(AppError::ServicePoolNotFound(pool_id.to_string()));
    }
    Ok(())
}

pub async fn set_members(
    db: &mongodb::Database,
    owner_id: &str,
    pool_id: &str,
    members: Vec<ServicePoolMember>,
) -> AppResult<ServicePool> {
    update_pool(
        db,
        owner_id,
        pool_id,
        UpdatePoolInput {
            members: Some(members),
            ..Default::default()
        },
    )
    .await
}

pub async fn add_member(
    db: &mongodb::Database,
    owner_id: &str,
    pool_id: &str,
    member: ServicePoolMember,
) -> AppResult<ServicePool> {
    let pool = get_pool(db, owner_id, pool_id).await?;
    let mut members = pool.members;
    if let Some(existing) = members
        .iter_mut()
        .find(|m| m.user_service_id == member.user_service_id)
    {
        existing.weight = member.weight.max(1);
        existing.enabled = member.enabled;
    } else {
        members.push(member);
    }
    set_members(db, owner_id, pool_id, members).await
}

pub async fn remove_member(
    db: &mongodb::Database,
    owner_id: &str,
    pool_id: &str,
    user_service_id: &str,
) -> AppResult<ServicePool> {
    let pool = get_pool(db, owner_id, pool_id).await?;
    let original_len = pool.members.len();
    let members: Vec<ServicePoolMember> = pool
        .members
        .into_iter()
        .filter(|m| m.user_service_id != user_service_id)
        .collect();
    if members.len() == original_len {
        return Err(AppError::ServicePoolMemberInvalid(format!(
            "Pool member '{user_service_id}' not found"
        )));
    }
    set_members(db, owner_id, pool_id, members).await
}

pub async fn resolve_member(
    db: &mongodb::Database,
    owner_id: &str,
    slug: &str,
) -> AppResult<Option<(UserService, PoolSelection)>> {
    let Some(pool) = find_pool_by_slug(db, owner_id, slug).await? else {
        return Ok(None);
    };

    let mut viable_services = Vec::new();
    let mut viable_weights = Vec::new();
    for member in &pool.members {
        if !member.enabled {
            continue;
        }
        let Some(service) =
            crate::services::service_history::collection::<UserService>(db, USER_SERVICES)
                .find_one(doc! {
                    "_id": &member.user_service_id,
                    "user_id": owner_id,
                    "is_active": true,
                })
                .await?
        else {
            continue;
        };
        viable_weights.push(member.weight.max(1));
        viable_services.push(service);
    }

    if viable_services.is_empty() {
        return Err(AppError::ServicePoolNoViableMember(pool.slug));
    }

    let Some(updated) = db
        .collection::<ServicePool>(SERVICE_POOLS)
        .find_one_and_update(
            doc! { "_id": &pool.id, "user_id": owner_id, "is_active": true },
            doc! {
                "$inc": { "rr_counter": 1 },
                "$set": { "updated_at": bson::DateTime::from_chrono(Utc::now()) },
            },
        )
        .return_document(mongodb::options::ReturnDocument::After)
        .await?
    else {
        return Ok(None);
    };
    let tick = updated.rr_counter - 1;
    let selected_index = choose_member_index(pool.strategy, &viable_weights, tick)
        .ok_or_else(|| AppError::ServicePoolNoViableMember(pool.slug.clone()))?;
    let service = viable_services
        .get(selected_index)
        .cloned()
        .ok_or_else(|| AppError::ServicePoolNoViableMember(pool.slug.clone()))?;
    let selection = PoolSelection {
        pool_id: pool.id,
        pool_slug: pool.slug,
        strategy: pool.strategy,
        selected_member_id: service.id.clone(),
        tick,
    };
    Ok(Some((service, selection)))
}

pub async fn find_first_viable_member(
    db: &mongodb::Database,
    owner_id: &str,
    slug: &str,
) -> AppResult<Option<UserService>> {
    let Some(pool) = find_pool_by_slug(db, owner_id, slug).await? else {
        return Ok(None);
    };

    for member in &pool.members {
        if !member.enabled {
            continue;
        }
        if let Some(service) =
            crate::services::service_history::collection::<UserService>(db, USER_SERVICES)
                .find_one(doc! {
                    "_id": &member.user_service_id,
                    "user_id": owner_id,
                    "is_active": true,
                })
                .await?
        {
            return Ok(Some(service));
        }
    }

    Err(AppError::ServicePoolNoViableMember(pool.slug))
}

pub fn choose_member_index(strategy: PoolStrategy, weights: &[u32], tick: i64) -> Option<usize> {
    if weights.is_empty() {
        return None;
    }
    match strategy {
        PoolStrategy::RoundRobin => {
            let tick = tick.rem_euclid(weights.len() as i64) as usize;
            Some(tick)
        }
        PoolStrategy::Weighted => {
            let total: u64 = weights.iter().map(|w| u64::from((*w).max(1))).sum();
            if total == 0 {
                return None;
            }
            let mut slot = tick.rem_euclid(total as i64) as u64;
            for (idx, weight) in weights.iter().enumerate() {
                let width = u64::from((*weight).max(1));
                if slot < width {
                    return Some(idx);
                }
                slot -= width;
            }
            Some(weights.len() - 1)
        }
    }
}

pub(crate) fn is_duplicate_key(err: &mongodb::error::Error) -> bool {
    matches!(
        err.kind.as_ref(),
        mongodb::error::ErrorKind::Write(mongodb::error::WriteFailure::WriteError(we))
            if we.code == 11000
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::{connect_test_database, test_user_service};

    fn member(user_service_id: &str, weight: u32, enabled: bool) -> ServicePoolMember {
        ServicePoolMember {
            user_service_id: user_service_id.to_string(),
            weight,
            enabled,
        }
    }

    fn create_input(slug: &str, members: Vec<ServicePoolMember>) -> CreatePoolInput {
        CreatePoolInput {
            slug: slug.to_string(),
            name: format!("{slug} pool"),
            description: Some("test pool".to_string()),
            strategy: PoolStrategy::RoundRobin,
            members,
            is_active: Some(true),
        }
    }

    async fn insert_service(
        db: &mongodb::Database,
        owner_id: &str,
        slug: &str,
        is_active: bool,
    ) -> String {
        let service_id = Uuid::new_v4().to_string();
        let mut service = test_user_service(
            &service_id,
            owner_id,
            slug,
            &Uuid::new_v4().to_string(),
            None,
            None,
        );
        service.is_active = is_active;
        db.collection::<UserService>(USER_SERVICES)
            .insert_one(&service)
            .await
            .unwrap();
        service_id
    }

    #[test]
    fn choose_member_index_round_robin_rotates_evenly() {
        let picked: Vec<usize> = (0..6)
            .map(|tick| choose_member_index(PoolStrategy::RoundRobin, &[1, 10, 1], tick).unwrap())
            .collect();
        assert_eq!(picked, vec![0, 1, 2, 0, 1, 2]);
    }

    #[test]
    fn choose_member_index_weighted_gives_weight_two_member_twice_share() {
        let picked: Vec<usize> = (0..6)
            .map(|tick| choose_member_index(PoolStrategy::Weighted, &[2, 1], tick).unwrap())
            .collect();
        assert_eq!(picked, vec![0, 0, 1, 0, 0, 1]);
    }

    #[test]
    fn normalize_members_clamps_weight() {
        let normalized = normalize_members(vec![member("svc-1", 0, true)]).unwrap();
        assert_eq!(normalized[0].weight, 1);
    }

    #[tokio::test]
    async fn service_pool_crud_and_slug_conflict() {
        let Some(db) = connect_test_database("service_pool_crud").await else {
            eprintln!("skipping service_pool_service integration test: no local MongoDB available");
            return;
        };
        let owner_id = Uuid::new_v4().to_string();
        let service_id = insert_service(&db, &owner_id, "direct-service", true).await;

        let created = create_pool(
            &db,
            &owner_id,
            create_input("service-pool", vec![member(&service_id, 0, true)]),
        )
        .await
        .unwrap();
        assert_eq!(created.slug, "service-pool");
        assert_eq!(created.members[0].weight, 1);

        let listed = list_pools(&db, &owner_id).await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, created.id);

        let by_slug = find_pool_by_slug(&db, &owner_id, "service-pool")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(by_slug.id, created.id);

        let updated = update_pool(
            &db,
            &owner_id,
            &created.id,
            UpdatePoolInput {
                name: Some("Updated Pool".to_string()),
                strategy: Some(PoolStrategy::Weighted),
                ..Default::default()
            },
        )
        .await
        .unwrap();
        assert_eq!(updated.name, "Updated Pool");
        assert_eq!(updated.strategy, PoolStrategy::Weighted);

        let direct_slug_conflict = create_pool(
            &db,
            &owner_id,
            create_input("direct-service", vec![member(&service_id, 1, true)]),
        )
        .await;
        assert!(matches!(
            direct_slug_conflict,
            Err(AppError::ServicePoolSlugTaken(_))
        ));

        let pool_slug_conflict = create_pool(
            &db,
            &owner_id,
            create_input("service-pool", vec![member(&service_id, 1, true)]),
        )
        .await;
        assert!(matches!(
            pool_slug_conflict,
            Err(AppError::ServicePoolSlugTaken(_))
        ));

        delete_pool(&db, &owner_id, &created.id).await.unwrap();
        assert!(matches!(
            get_pool(&db, &owner_id, &created.id).await,
            Err(AppError::ServicePoolNotFound(_))
        ));
    }

    #[tokio::test]
    async fn service_pool_resolve_filters_nonviable_members() {
        let Some(db) = connect_test_database("service_pool_viable").await else {
            eprintln!("skipping service_pool_service integration test: no local MongoDB available");
            return;
        };
        let owner_id = Uuid::new_v4().to_string();
        let soon_inactive_id = insert_service(&db, &owner_id, "soon-inactive-member", true).await;
        let disabled_id = insert_service(&db, &owner_id, "disabled-member", true).await;
        let active_id = insert_service(&db, &owner_id, "active-member", true).await;

        let pool = create_pool(
            &db,
            &owner_id,
            CreatePoolInput {
                strategy: PoolStrategy::Weighted,
                ..create_input(
                    "routed-pool",
                    vec![
                        member(&soon_inactive_id, 10, true),
                        member(&disabled_id, 10, false),
                        member(&active_id, 1, true),
                    ],
                )
            },
        )
        .await
        .unwrap();

        db.collection::<UserService>(USER_SERVICES)
            .update_one(
                doc! { "_id": &soon_inactive_id },
                doc! { "$set": { "is_active": false } },
            )
            .await
            .unwrap();

        let (selected, metadata) = resolve_member(&db, &owner_id, "routed-pool")
            .await
            .unwrap()
            .expect("pool should resolve");
        assert_eq!(selected.id, active_id);
        assert_eq!(metadata.pool_id, pool.id);
        assert_eq!(metadata.selected_member_id, active_id);
        assert_eq!(metadata.strategy, PoolStrategy::Weighted);

        db.collection::<UserService>(USER_SERVICES)
            .update_one(
                doc! { "_id": &active_id },
                doc! { "$set": { "is_active": false } },
            )
            .await
            .unwrap();
        assert!(matches!(
            resolve_member(&db, &owner_id, "routed-pool").await,
            Err(AppError::ServicePoolNoViableMember(_))
        ));
    }

    #[tokio::test]
    async fn service_pool_rejects_cross_owner_and_inactive_members() {
        let Some(db) = connect_test_database("service_pool_member_validation").await else {
            eprintln!("skipping service_pool_service integration test: no local MongoDB available");
            return;
        };
        let owner_id = Uuid::new_v4().to_string();
        let other_owner = Uuid::new_v4().to_string();
        let inactive_id = insert_service(&db, &owner_id, "inactive-service", false).await;
        let other_id = insert_service(&db, &other_owner, "other-service", true).await;

        let inactive = create_pool(
            &db,
            &owner_id,
            create_input("inactive-member-pool", vec![member(&inactive_id, 1, true)]),
        )
        .await;
        assert!(matches!(
            inactive,
            Err(AppError::ServicePoolMemberInvalid(_))
        ));

        let cross_owner = create_pool(
            &db,
            &owner_id,
            create_input("cross-owner-pool", vec![member(&other_id, 1, true)]),
        )
        .await;
        assert!(matches!(
            cross_owner,
            Err(AppError::ServicePoolMemberInvalid(_))
        ));
    }
}

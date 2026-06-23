use chrono::Utc;
use futures::TryStreamExt;
use mongodb::bson::{self, doc};
use mongodb::options::ReturnDocument;
use uuid::Uuid;

use crate::errors::{AppError, AppResult};
use crate::models::service_pool::{COLLECTION_NAME, PoolStrategy, ServicePool, ServicePoolMember};
use crate::models::user_service::{COLLECTION_NAME as USER_SERVICES, UserService};
use crate::services::user_service_service;

const MAX_POOL_MEMBERS: usize = 50;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PoolSelection {
    pub pool_id: String,
    pub pool_slug: String,
    pub strategy: PoolStrategy,
    pub member_user_service_id: String,
    pub tick: i64,
}

#[derive(Clone, Debug)]
pub struct CreatePoolInput {
    pub slug: String,
    pub name: String,
    pub description: Option<String>,
    pub strategy: PoolStrategy,
    pub members: Vec<ServicePoolMember>,
}

#[derive(Clone, Debug, Default)]
pub struct UpdatePoolInput {
    pub slug: Option<String>,
    pub name: Option<String>,
    pub description: Option<String>,
    pub strategy: Option<PoolStrategy>,
    pub is_active: Option<bool>,
}

fn validate_name(name: &str) -> AppResult<()> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Err(AppError::ValidationError(
            "Service pool name is required".to_string(),
        ));
    }
    if trimmed.len() > 128 {
        return Err(AppError::ValidationError(
            "Service pool name must be at most 128 characters".to_string(),
        ));
    }
    Ok(())
}

fn validate_description(description: Option<&str>) -> AppResult<()> {
    if let Some(description) = description
        && description.len() > 1024
    {
        return Err(AppError::ValidationError(
            "Service pool description must be at most 1024 characters".to_string(),
        ));
    }
    Ok(())
}

fn normalize_member(mut member: ServicePoolMember) -> ServicePoolMember {
    if member.weight == 0 {
        member.weight = 1;
    }
    member
}

async fn ensure_slug_available(
    db: &mongodb::Database,
    user_id: &str,
    slug: &str,
    exclude_pool_id: Option<&str>,
) -> AppResult<()> {
    user_service_service::validate_slug(slug)?;

    if user_service_service::find_by_slug(db, user_id, slug)
        .await?
        .is_some()
    {
        return Err(AppError::ServicePoolSlugTaken(slug.to_string()));
    }

    let mut pool_filter = doc! {
        "user_id": user_id,
        "slug": slug,
        "is_active": true,
    };
    if let Some(exclude_pool_id) = exclude_pool_id {
        pool_filter.insert("_id", doc! { "$ne": exclude_pool_id });
    }
    if db
        .collection::<ServicePool>(COLLECTION_NAME)
        .find_one(pool_filter)
        .await?
        .is_some()
    {
        return Err(AppError::ServicePoolSlugTaken(slug.to_string()));
    }

    Ok(())
}

async fn validate_members(
    db: &mongodb::Database,
    user_id: &str,
    members: Vec<ServicePoolMember>,
) -> AppResult<Vec<ServicePoolMember>> {
    if members.len() > MAX_POOL_MEMBERS {
        return Err(AppError::ServicePoolMemberInvalid(format!(
            "service pools support at most {MAX_POOL_MEMBERS} members"
        )));
    }

    let mut seen = std::collections::HashSet::with_capacity(members.len());
    let mut out = Vec::with_capacity(members.len());
    for member in members {
        let member = normalize_member(member);
        if member.user_service_id.trim().is_empty() {
            return Err(AppError::ServicePoolMemberInvalid(
                "member user_service_id must not be empty".to_string(),
            ));
        }
        if !seen.insert(member.user_service_id.clone()) {
            return Err(AppError::ServicePoolMemberInvalid(format!(
                "duplicate service pool member '{}'",
                member.user_service_id
            )));
        }

        let count = db
            .collection::<UserService>(USER_SERVICES)
            .count_documents(doc! {
                "_id": &member.user_service_id,
                "user_id": user_id,
                "is_active": true,
            })
            .await?;
        if count == 0 {
            return Err(AppError::ServicePoolMemberInvalid(format!(
                "member '{}' must reference an active service owned by the same owner",
                member.user_service_id
            )));
        }
        out.push(member);
    }

    Ok(out)
}

pub fn choose_member_index(strategy: &PoolStrategy, weights: &[u32], tick: i64) -> Option<usize> {
    if weights.is_empty() {
        return None;
    }

    let tick = tick.max(0) as u64;
    match strategy {
        PoolStrategy::RoundRobin => Some((tick % weights.len() as u64) as usize),
        PoolStrategy::Weighted => {
            let total: u64 = weights.iter().map(|w| u64::from((*w).max(1))).sum();
            if total == 0 {
                return None;
            }
            let mut cursor = tick % total;
            for (idx, weight) in weights.iter().enumerate() {
                let weight = u64::from((*weight).max(1));
                if cursor < weight {
                    return Some(idx);
                }
                cursor -= weight;
            }
            Some(weights.len() - 1)
        }
    }
}

pub async fn list_pools(db: &mongodb::Database, user_id: &str) -> AppResult<Vec<ServicePool>> {
    Ok(db
        .collection::<ServicePool>(COLLECTION_NAME)
        .find(doc! { "user_id": user_id, "is_active": true })
        .sort(doc! { "created_at": -1 })
        .await?
        .try_collect()
        .await?)
}

pub async fn get_pool(
    db: &mongodb::Database,
    user_id: &str,
    pool_id: &str,
) -> AppResult<ServicePool> {
    db.collection::<ServicePool>(COLLECTION_NAME)
        .find_one(doc! { "_id": pool_id, "user_id": user_id })
        .await?
        .ok_or_else(|| AppError::ServicePoolNotFound(pool_id.to_string()))
}

pub async fn find_pool_by_slug(
    db: &mongodb::Database,
    user_id: &str,
    slug: &str,
) -> AppResult<Option<ServicePool>> {
    Ok(db
        .collection::<ServicePool>(COLLECTION_NAME)
        .find_one(doc! { "user_id": user_id, "slug": slug, "is_active": true })
        .await?)
}

pub async fn create_pool(
    db: &mongodb::Database,
    user_id: &str,
    input: CreatePoolInput,
) -> AppResult<ServicePool> {
    validate_name(&input.name)?;
    validate_description(input.description.as_deref())?;
    ensure_slug_available(db, user_id, &input.slug, None).await?;
    let members = validate_members(db, user_id, input.members).await?;

    let now = Utc::now();
    let pool = ServicePool {
        id: Uuid::new_v4().to_string(),
        user_id: user_id.to_string(),
        slug: input.slug,
        name: input.name.trim().to_string(),
        description: input.description,
        strategy: input.strategy,
        members,
        rr_counter: 0,
        is_active: true,
        created_at: now,
        updated_at: now,
    };

    db.collection::<ServicePool>(COLLECTION_NAME)
        .insert_one(&pool)
        .await?;

    Ok(pool)
}

pub async fn update_pool(
    db: &mongodb::Database,
    user_id: &str,
    pool_id: &str,
    input: UpdatePoolInput,
) -> AppResult<ServicePool> {
    let current = get_pool(db, user_id, pool_id).await?;
    let mut set_doc = doc! {
        "updated_at": bson::DateTime::from_chrono(Utc::now()),
    };

    if let Some(slug) = input.slug {
        if slug != current.slug {
            ensure_slug_available(db, user_id, &slug, Some(pool_id)).await?;
            set_doc.insert("slug", slug);
        }
    }
    if let Some(name) = input.name {
        validate_name(&name)?;
        set_doc.insert("name", name.trim());
    }
    if let Some(description) = input.description {
        validate_description(Some(&description))?;
        if description.trim().is_empty() {
            set_doc.insert("description", bson::Bson::Null);
        } else {
            set_doc.insert("description", description);
        }
    }
    if let Some(strategy) = input.strategy {
        set_doc.insert("strategy", strategy.as_str());
    }
    if let Some(is_active) = input.is_active {
        set_doc.insert("is_active", is_active);
    }

    let updated = db
        .collection::<ServicePool>(COLLECTION_NAME)
        .find_one_and_update(
            doc! { "_id": pool_id, "user_id": user_id },
            doc! { "$set": set_doc },
        )
        .return_document(ReturnDocument::After)
        .await?
        .ok_or_else(|| AppError::ServicePoolNotFound(pool_id.to_string()))?;

    Ok(updated)
}

pub async fn delete_pool(db: &mongodb::Database, user_id: &str, pool_id: &str) -> AppResult<()> {
    let result = db
        .collection::<ServicePool>(COLLECTION_NAME)
        .update_one(
            doc! { "_id": pool_id, "user_id": user_id },
            doc! {
                "$set": {
                    "is_active": false,
                    "updated_at": bson::DateTime::from_chrono(Utc::now()),
                }
            },
        )
        .await?;
    if result.matched_count == 0 {
        return Err(AppError::ServicePoolNotFound(pool_id.to_string()));
    }
    Ok(())
}

pub async fn set_members(
    db: &mongodb::Database,
    user_id: &str,
    pool_id: &str,
    members: Vec<ServicePoolMember>,
) -> AppResult<ServicePool> {
    let _ = get_pool(db, user_id, pool_id).await?;
    let members = validate_members(db, user_id, members).await?;
    let members_bson = bson::to_bson(&members)
        .map_err(|e| AppError::Internal(format!("BSON serialization error: {e}")))?;

    db.collection::<ServicePool>(COLLECTION_NAME)
        .find_one_and_update(
            doc! { "_id": pool_id, "user_id": user_id },
            doc! {
                "$set": {
                    "members": members_bson,
                    "updated_at": bson::DateTime::from_chrono(Utc::now()),
                }
            },
        )
        .return_document(ReturnDocument::After)
        .await?
        .ok_or_else(|| AppError::ServicePoolNotFound(pool_id.to_string()))
}

pub async fn add_member(
    db: &mongodb::Database,
    user_id: &str,
    pool_id: &str,
    member: ServicePoolMember,
) -> AppResult<ServicePool> {
    let mut pool = get_pool(db, user_id, pool_id).await?;
    let member = normalize_member(member);
    if pool
        .members
        .iter()
        .any(|m| m.user_service_id == member.user_service_id)
    {
        return Err(AppError::ServicePoolMemberInvalid(format!(
            "service '{}' is already a member of this pool",
            member.user_service_id
        )));
    }
    pool.members.push(member);
    set_members(db, user_id, pool_id, pool.members).await
}

pub async fn remove_member(
    db: &mongodb::Database,
    user_id: &str,
    pool_id: &str,
    user_service_id: &str,
) -> AppResult<ServicePool> {
    let mut pool = get_pool(db, user_id, pool_id).await?;
    let before = pool.members.len();
    pool.members
        .retain(|m| m.user_service_id != user_service_id);
    if pool.members.len() == before {
        return Err(AppError::ServicePoolMemberInvalid(format!(
            "service '{user_service_id}' is not a member of this pool"
        )));
    }
    set_members(db, user_id, pool_id, pool.members).await
}

pub async fn resolve_member(
    db: &mongodb::Database,
    owner_id: &str,
    slug: &str,
) -> AppResult<Option<(UserService, PoolSelection)>> {
    let Some(pool) = find_pool_by_slug(db, owner_id, slug).await? else {
        return Ok(None);
    };
    if !pool.is_active {
        return Ok(None);
    }

    let mut viable = Vec::new();
    for member in pool.members.iter().filter(|m| m.enabled) {
        if let Some(service) =
            user_service_service::find_user_service_by_id(db, &member.user_service_id).await?
            && service.user_id == owner_id
            && service.is_active
        {
            viable.push((member.clone(), service));
        }
    }

    if viable.is_empty() {
        return Err(AppError::ServicePoolNoViableMember(pool.slug));
    }

    let advanced = db
        .collection::<ServicePool>(COLLECTION_NAME)
        .find_one_and_update(
            doc! { "_id": &pool.id, "user_id": owner_id, "is_active": true },
            doc! {
                "$inc": { "rr_counter": 1_i64 },
                "$set": { "updated_at": bson::DateTime::from_chrono(Utc::now()) },
            },
        )
        .return_document(ReturnDocument::After)
        .await?
        .ok_or_else(|| AppError::ServicePoolNotFound(pool.id.clone()))?;

    let tick = advanced.rr_counter.saturating_sub(1);
    let weights: Vec<u32> = viable
        .iter()
        .map(|(member, _)| member.weight.max(1))
        .collect();
    let idx = choose_member_index(&pool.strategy, &weights, tick)
        .ok_or_else(|| AppError::ServicePoolNoViableMember(pool.slug.clone()))?;
    let (member, service) = viable
        .into_iter()
        .nth(idx)
        .ok_or_else(|| AppError::ServicePoolNoViableMember(pool.slug.clone()))?;

    Ok(Some((
        service,
        PoolSelection {
            pool_id: pool.id,
            pool_slug: pool.slug,
            strategy: pool.strategy,
            member_user_service_id: member.user_service_id,
            tick,
        },
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::user_endpoint::COLLECTION_NAME as USER_ENDPOINTS;
    use crate::test_utils::{connect_test_database, test_user_endpoint, test_user_service};

    fn member(id: &str, weight: u32, enabled: bool) -> ServicePoolMember {
        ServicePoolMember {
            user_service_id: id.to_string(),
            weight,
            enabled,
        }
    }

    fn create_input(slug: &str, members: Vec<ServicePoolMember>) -> CreatePoolInput {
        CreatePoolInput {
            slug: slug.to_string(),
            name: "Service Pool".to_string(),
            description: Some("Test pool".to_string()),
            strategy: PoolStrategy::RoundRobin,
            members,
        }
    }

    #[test]
    fn choose_member_index_round_robin_rotates_evenly() {
        let weights = [1, 1, 1];
        let chosen: Vec<usize> = (0..6)
            .map(|tick| choose_member_index(&PoolStrategy::RoundRobin, &weights, tick).unwrap())
            .collect();
        assert_eq!(chosen, vec![0, 1, 2, 0, 1, 2]);
    }

    #[test]
    fn choose_member_index_weighted_honors_weight_two_share() {
        let weights = [2, 1];
        let chosen: Vec<usize> = (0..6)
            .map(|tick| choose_member_index(&PoolStrategy::Weighted, &weights, tick).unwrap())
            .collect();
        assert_eq!(chosen, vec![0, 0, 1, 0, 0, 1]);
    }

    #[tokio::test]
    async fn create_update_members_and_slug_conflicts() {
        let Some(db) = connect_test_database("service_pool_crud").await else {
            return;
        };
        let owner = uuid::Uuid::new_v4().to_string();
        let endpoint_id = uuid::Uuid::new_v4().to_string();
        let svc_id = uuid::Uuid::new_v4().to_string();
        let svc_2_id = uuid::Uuid::new_v4().to_string();

        db.collection::<crate::models::user_endpoint::UserEndpoint>(USER_ENDPOINTS)
            .insert_one(test_user_endpoint(
                &endpoint_id,
                &owner,
                "Endpoint",
                "https://example.test",
                None,
                None,
            ))
            .await
            .unwrap();
        db.collection::<UserService>(USER_SERVICES)
            .insert_many([
                test_user_service(&svc_id, &owner, "direct-svc", &endpoint_id, None, None),
                test_user_service(&svc_2_id, &owner, "direct-svc-2", &endpoint_id, None, None),
            ])
            .await
            .unwrap();

        let conflict = create_pool(&db, &owner, create_input("direct-svc", vec![])).await;
        assert!(matches!(conflict, Err(AppError::ServicePoolSlugTaken(_))));

        let pool = create_pool(
            &db,
            &owner,
            create_input("pool-a", vec![member(&svc_id, 0, true)]),
        )
        .await
        .expect("create pool");
        assert_eq!(pool.slug, "pool-a");
        assert_eq!(pool.members[0].weight, 1);

        let duplicate = create_pool(&db, &owner, create_input("pool-a", vec![])).await;
        assert!(matches!(duplicate, Err(AppError::ServicePoolSlugTaken(_))));

        let updated = update_pool(
            &db,
            &owner,
            &pool.id,
            UpdatePoolInput {
                name: Some("Updated Pool".to_string()),
                strategy: Some(PoolStrategy::Weighted),
                ..Default::default()
            },
        )
        .await
        .expect("update pool");
        assert_eq!(updated.name, "Updated Pool");
        assert_eq!(updated.strategy, PoolStrategy::Weighted);

        let updated = add_member(&db, &owner, &pool.id, member(&svc_2_id, 2, true))
            .await
            .expect("add member");
        assert_eq!(updated.members.len(), 2);

        let updated = remove_member(&db, &owner, &pool.id, &svc_id)
            .await
            .expect("remove member");
        assert_eq!(updated.members.len(), 1);
        assert_eq!(updated.members[0].user_service_id, svc_2_id);

        delete_pool(&db, &owner, &pool.id)
            .await
            .expect("delete pool");
        assert!(
            find_pool_by_slug(&db, &owner, "pool-a")
                .await
                .unwrap()
                .is_none()
        );

        db.drop().await.ok();
    }

    #[tokio::test]
    async fn resolve_member_filters_inactive_and_disabled_members() {
        let Some(db) = connect_test_database("service_pool_resolve").await else {
            return;
        };
        let owner = uuid::Uuid::new_v4().to_string();
        let endpoint_id = uuid::Uuid::new_v4().to_string();
        let inactive_svc_id = uuid::Uuid::new_v4().to_string();
        let disabled_svc_id = uuid::Uuid::new_v4().to_string();
        let viable_svc_id = uuid::Uuid::new_v4().to_string();

        db.collection::<crate::models::user_endpoint::UserEndpoint>(USER_ENDPOINTS)
            .insert_one(test_user_endpoint(
                &endpoint_id,
                &owner,
                "Endpoint",
                "https://example.test",
                None,
                None,
            ))
            .await
            .unwrap();
        db.collection::<UserService>(USER_SERVICES)
            .insert_many([
                test_user_service(
                    &inactive_svc_id,
                    &owner,
                    "inactive",
                    &endpoint_id,
                    None,
                    None,
                ),
                test_user_service(
                    &disabled_svc_id,
                    &owner,
                    "disabled",
                    &endpoint_id,
                    None,
                    None,
                ),
                test_user_service(&viable_svc_id, &owner, "viable", &endpoint_id, None, None),
            ])
            .await
            .unwrap();

        let pool = create_pool(
            &db,
            &owner,
            CreatePoolInput {
                strategy: PoolStrategy::RoundRobin,
                ..create_input(
                    "pool-a",
                    vec![
                        member(&inactive_svc_id, 1, true),
                        member(&disabled_svc_id, 1, false),
                        member(&viable_svc_id, 1, true),
                    ],
                )
            },
        )
        .await
        .expect("create pool");
        db.collection::<UserService>(USER_SERVICES)
            .update_one(
                doc! { "_id": &inactive_svc_id },
                doc! { "$set": { "is_active": false } },
            )
            .await
            .unwrap();

        let (selected, selection) = resolve_member(&db, &owner, "pool-a")
            .await
            .expect("resolve should not fail")
            .expect("pool should exist");
        assert_eq!(selected.id, viable_svc_id);
        assert_eq!(selection.pool_id, pool.id);
        assert_eq!(selection.member_user_service_id, viable_svc_id);

        db.collection::<ServicePool>(COLLECTION_NAME)
            .update_one(
                doc! { "_id": &pool.id },
                doc! {
                    "$set": {
                        "members": bson::to_bson(&vec![
                            member(&inactive_svc_id, 1, true),
                            member(&disabled_svc_id, 1, false),
                        ]).unwrap(),
                    }
                },
            )
            .await
            .unwrap();
        let err = resolve_member(&db, &owner, "pool-a")
            .await
            .expect_err("no viable member");
        assert!(matches!(err, AppError::ServicePoolNoViableMember(_)));

        db.drop().await.ok();
    }
}

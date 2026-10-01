use chrono::Utc;
use futures::TryStreamExt;
use mongodb::bson::{self, doc};
use std::collections::HashSet;
use uuid::Uuid;

use crate::errors::{AppError, AppResult};
use crate::models::service_pool::{
    COLLECTION_NAME as SERVICE_POOLS, PoolStrategy, ServicePool, ServicePoolMember, TierBalance,
};
use crate::models::user_service::{COLLECTION_NAME as USER_SERVICES, UserService};
use crate::services::user_service_service;

pub const MAX_POOL_MEMBERS: usize = 50;

const MAX_NAME_LEN: usize = 128;
const MAX_DESCRIPTION_LEN: usize = 1024;
const MAX_MODEL_LEN: usize = 256;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PoolSelection {
    pub pool_id: String,
    pub pool_slug: String,
    pub strategy: PoolStrategy,
    pub selected_member_id: String,
    pub tick: i64,
    pub tier: u32,
    pub attempt: u32,
    pub failover_enabled: bool,
}

#[derive(Clone)]
pub struct PoolCandidate {
    pub chat_plan: Option<super::pool_ai_service::ChatPlan>,
    pub contract_metadata: super::service_pool_contract::ContractMember,
    pub member: ServicePoolMember,
    pub service: UserService,
    pub tier: u32,
    pub cooldown_until: Option<chrono::DateTime<Utc>>,
    pub health_scope: crate::services::service_pool_health_service::HealthScope,
}

#[derive(Clone)]
pub struct PoolCandidatePlan {
    pub chat_request: Option<super::pool_ai_service::ChatRequest>,
    pub pool: ServicePool,
    pub candidates: Vec<PoolCandidate>,
    pub max_attempts: usize,
    pub all_cooled_until: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug)]
pub struct CreatePoolInput {
    pub slug: String,
    pub name: String,
    pub description: Option<String>,
    pub strategy: PoolStrategy,
    pub tier_balance: TierBalance,
    pub member_contract: crate::models::service_pool::PoolMemberContract,
    pub failover: Option<crate::models::service_pool::FailoverPolicy>,
    pub members: Vec<ServicePoolMember>,
    pub is_active: Option<bool>,
}

#[derive(Debug, Default)]
pub struct UpdatePoolInput {
    pub slug: Option<String>,
    pub name: Option<String>,
    pub description: Option<Option<String>>,
    pub strategy: Option<PoolStrategy>,
    pub tier_balance: Option<TierBalance>,
    pub member_contract: Option<crate::models::service_pool::PoolMemberContract>,
    pub failover: Option<Option<crate::models::service_pool::FailoverPolicy>>,
    pub members: Option<Vec<ServicePoolMember>>,
    pub is_active: Option<bool>,
    pub expected_revision: Option<i64>,
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
        if member.weight > 1000 {
            return Err(AppError::ServicePoolMemberInvalid(
                "Member weight must not exceed 1000".into(),
            ));
        }
        member.weight = member.weight.max(1);
        if let Some(model) = member.model.as_mut() {
            let trimmed = model.trim();
            if trimmed.is_empty() {
                member.model = None;
            } else if trimmed.len() > MAX_MODEL_LEN {
                return Err(AppError::ServicePoolMemberInvalid(format!(
                    "member model must not exceed {MAX_MODEL_LEN} characters"
                )));
            } else {
                *model = trimmed.to_string();
            }
        }
        out.push(member);
    }
    Ok(out)
}

fn validate_pool_policy(
    strategy: PoolStrategy,
    tier_balance: TierBalance,
    contract: crate::models::service_pool::PoolMemberContract,
    failover: Option<&crate::models::service_pool::FailoverPolicy>,
    members: &[ServicePoolMember],
) -> AppResult<()> {
    if strategy != PoolStrategy::Priority && tier_balance != TierBalance::RoundRobin {
        return Err(AppError::ServicePoolMemberInvalid(
            "tier balancing requires the priority strategy".to_string(),
        ));
    }
    if failover.is_some() && strategy != PoolStrategy::Priority {
        return Err(AppError::ServicePoolMemberInvalid(
            "failover requires the priority strategy; older strategies remain single-attempt"
                .to_string(),
        ));
    }
    if contract == crate::models::service_pool::PoolMemberContract::SameApi
        && members.iter().any(|member| member.model.is_some())
    {
        return Err(AppError::ServicePoolMemberInvalid(
            "Model mapping requires the ai_chat contract".into(),
        ));
    }
    if contract == crate::models::service_pool::PoolMemberContract::AiChat {
        if strategy != PoolStrategy::Priority {
            return Err(AppError::ServicePoolMemberInvalid(
                "the ai_chat contract requires the priority strategy".to_string(),
            ));
        }
        if members.iter().any(|member| member.model.is_none()) {
            return Err(AppError::ServicePoolMemberInvalid(
                "every ai_chat member must define a model mapping".to_string(),
            ));
        }
    }
    if strategy != PoolStrategy::Priority
        && members
            .iter()
            .any(|member| member.priority != 0 || member.model.is_some())
    {
        return Err(AppError::ServicePoolMemberInvalid(
            "priority and model member settings require the priority strategy".to_string(),
        ));
    }
    if let Some(policy) = failover
        && (!(1..=5).contains(&policy.max_attempts)
            || !(1_000..=300_000).contains(&policy.per_attempt_timeout_ms)
            || !(1_000..=600_000).contains(&policy.overall_deadline_ms)
            || policy.cooldown.base_ms == 0
            || policy.cooldown.max_ms < policy.cooldown.base_ms
            || policy.cooldown.failures_to_open == 0
            || policy.max_replay_body_bytes == 0
            || u64::from(policy.cooldown.max_ms)
                > crate::services::service_pool_health_service::HEALTH_RETENTION_MS)
    {
        return Err(AppError::ServicePoolMemberInvalid(
            "failover limits are outside the supported bounds".to_string(),
        ));
    }

    Ok(())
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
    let field = if Uuid::parse_str(pool_id).is_ok() {
        "_id"
    } else {
        "slug"
    };
    db.collection::<ServicePool>(SERVICE_POOLS)
        .find_one(doc! { field: pool_id, "user_id": owner_id })
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

/// Resolve within the pool owner; UUIDs never fall through to another owner's
/// service or to a slug that happens to resemble a UUID.
pub async fn resolve_member_identifier(
    db: &mongodb::Database,
    owner: &str,
    identifier: &str,
) -> AppResult<String> {
    let field = if Uuid::parse_str(identifier).is_ok() {
        "_id"
    } else {
        "slug"
    };
    crate::services::service_history::collection::<UserService>(db, USER_SERVICES)
        .find_one(doc! { field: identifier, "user_id": owner })
        .await?
        .map(|service| service.id)
        .ok_or_else(|| {
            AppError::ValidationError("Pool member must belong to the pool owner".into())
        })
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
    validate_pool_policy(
        input.strategy,
        input.tier_balance,
        input.member_contract,
        input.failover.as_ref(),
        &members,
    )?;

    super::service_pool_contract::validate_configuration(
        db,
        owner_id,
        input.strategy,
        input.member_contract,
        &members,
    )
    .await?;

    let now = Utc::now();
    let pool = ServicePool {
        id: Uuid::new_v4().to_string(),
        user_id: owner_id.to_string(),
        slug: input.slug,
        name: input.name,
        description: input.description,
        strategy: input.strategy,
        tier_balance: input.tier_balance,
        member_contract: input.member_contract,
        failover: input.failover,
        members,
        rr_counter: 0,
        tier_counters: Default::default(),
        config_revision: 0,
        health_reset_generation: 0,
        health_observation_sequence: 0,
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
    let disable_only = input.is_active == Some(false)
        && input.members.is_none()
        && input.strategy.is_none()
        && input.member_contract.is_none()
        && input.tier_balance.is_none()
        && input.failover.is_none();
    let current = get_pool(db, owner_id, pool_id).await?;
    let pool_id = current.id.as_str();
    let slug = input.slug.unwrap_or_else(|| current.slug.clone());
    if slug != current.slug {
        ensure_slug_available(db, owner_id, &slug, Some(&current.id)).await?;
    } else {
        user_service_service::validate_slug(&slug)?;
    }

    let name = input.name.unwrap_or_else(|| current.name.clone());
    let description = input
        .description
        .unwrap_or_else(|| current.description.clone());
    validate_text_fields(&name, description.as_deref())?;
    let members = match input.members {
        Some(members) => {
            let members = normalize_members(members)?;
            validate_members_owned_and_active(db, owner_id, &members).await?;
            Some(members)
        }
        None => None,
    };
    let strategy = input.strategy.unwrap_or(current.strategy);
    let tier_balance = input.tier_balance.unwrap_or(current.tier_balance);
    let member_contract = input.member_contract.unwrap_or(current.member_contract);
    let failover_was_present = input.failover.is_some();
    let failover = input
        .failover
        .clone()
        .unwrap_or_else(|| current.failover.clone());
    let effective_members = members.clone().unwrap_or_else(|| current.members.clone());
    validate_pool_policy(
        strategy,
        tier_balance,
        member_contract,
        failover.as_ref(),
        &effective_members,
    )?;

    if !disable_only {
        super::service_pool_contract::validate_configuration(
            db,
            owner_id,
            strategy,
            member_contract,
            &effective_members,
        )
        .await?;
    }

    let mut set = doc! {
        "slug": &slug,
        "name": &name,
        "tier_counters": doc! {},
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
    if input.strategy.is_some() {
        set.insert("strategy", strategy.as_str());
    }
    if input.tier_balance.is_some() {
        set.insert(
            "tier_balance",
            bson::to_bson(&tier_balance)
                .map_err(|e| AppError::Internal(format!("BSON serialization error: {e}")))?,
        );
    }
    if input.member_contract.is_some() {
        set.insert("member_contract", member_contract.as_str());
    }
    if failover_was_present {
        set.insert(
            "failover",
            bson::to_bson(&failover)
                .map_err(|e| AppError::Internal(format!("BSON serialization error: {e}")))?,
        );
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

    let expected_revision = input.expected_revision.unwrap_or(current.config_revision);
    let revision_filter = if expected_revision == 0 {
        doc! {
            "$or": [
                { "config_revision": 0_i64 },
                { "config_revision": { "$exists": false } },
            ]
        }
    } else {
        doc! { "config_revision": expected_revision }
    };
    let update_filter = doc! {
        "_id": pool_id,
        "user_id": owner_id,
        "$and": [revision_filter],
    };
    let result = db
        .collection::<ServicePool>(SERVICE_POOLS)
        .update_one(
            update_filter,
            doc! { "$set": set, "$inc": { "config_revision": 1_i64 } },
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
        let exists = db
            .collection::<ServicePool>(SERVICE_POOLS)
            .find_one(doc! { "_id": pool_id, "user_id": owner_id })
            .await?;
        return Err(if exists.is_some() {
            AppError::Conflict("service pool changed while it was being edited".to_string())
        } else {
            AppError::ServicePoolNotFound(pool_id.to_string())
        });
    }

    get_pool(db, owner_id, pool_id).await
}

pub async fn delete_pool(db: &mongodb::Database, owner_id: &str, pool_id: &str) -> AppResult<()> {
    let pool = get_pool(db, owner_id, pool_id).await?;
    let pool_id = pool.id.as_str();
    let result = db
        .collection::<ServicePool>(SERVICE_POOLS)
        .delete_one(doc! { "_id": pool_id, "user_id": owner_id })
        .await?;
    if result.deleted_count == 0 {
        return Err(AppError::ServicePoolNotFound(pool_id.to_string()));
    }
    Ok(())
}

pub async fn set_members_with_revision(
    db: &mongodb::Database,
    owner_id: &str,
    pool_id: &str,
    members: Vec<ServicePoolMember>,
    expected_revision: Option<i64>,
) -> AppResult<ServicePool> {
    update_pool(
        db,
        owner_id,
        pool_id,
        UpdatePoolInput {
            members: Some(members),
            expected_revision,
            ..Default::default()
        },
    )
    .await
}

pub async fn add_member_with_revision(
    db: &mongodb::Database,
    owner_id: &str,
    pool_id: &str,
    member: ServicePoolMember,
    expected_revision: Option<i64>,
) -> AppResult<ServicePool> {
    let pool = get_pool(db, owner_id, pool_id).await?;
    let mut members = pool.members;
    if let Some(existing) = members
        .iter_mut()
        .find(|m| m.user_service_id == member.user_service_id)
    {
        *existing = member;
    } else {
        members.push(member);
    }
    set_members_with_revision(
        db,
        owner_id,
        pool_id,
        members,
        expected_revision.or(Some(pool.config_revision)),
    )
    .await
}

pub async fn remove_member_with_revision(
    db: &mongodb::Database,
    owner_id: &str,
    pool_id: &str,
    user_service_id: &str,
    expected_revision: Option<i64>,
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
    set_members_with_revision(
        db,
        owner_id,
        pool_id,
        members,
        expected_revision.or(Some(pool.config_revision)),
    )
    .await
}

/// Only read-only member resolution uses this classification. Approval outcomes
/// and request-wide authentication never enter candidate filtering.
pub fn member_unavailable(error: &AppError) -> bool {
    matches!(
        error,
        AppError::NotFound(_)
            | AppError::OrgRoleInsufficient(_)
            | AppError::ApiKeyScopeForbidden(_)
            | AppError::Forbidden(_)
            | AppError::RequiredServiceNotConnected { .. }
    )
}

pub async fn resolve_member(
    db: &mongodb::Database,
    owner_id: &str,
    slug: &str,
) -> AppResult<Option<(UserService, PoolSelection)>> {
    let Some(pool) = find_pool_by_slug(db, owner_id, slug).await? else {
        return Ok(None);
    };
    if pool.strategy == PoolStrategy::Priority {
        return Err(AppError::BadRequest(
            "Priority pools require the HTTP pool executor".into(),
        ));
    }

    let mut viable_services = Vec::new();
    let mut viable_weights = Vec::new();
    let mut viable_members = Vec::new();
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
        viable_members.push(member.clone());
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
    let selected_member = viable_members
        .get(selected_index)
        .ok_or_else(|| AppError::ServicePoolNoViableMember(pool.slug.clone()))?;
    let selection = PoolSelection {
        pool_id: pool.id,
        pool_slug: pool.slug,
        strategy: pool.strategy,
        selected_member_id: service.id.clone(),
        tick,
        tier: selected_member.priority,
        attempt: 1,
        failover_enabled: pool.failover.is_some() && pool.strategy == PoolStrategy::Priority,
    };
    Ok(Some((service, selection)))
}

/// Plan concrete members without materializing credentials. The resolver still
/// rechecks the exact member before every attempt; this function only orders
/// and statically filters durable membership state.
#[cfg(test)]
#[allow(clippy::too_many_arguments)]
pub async fn plan_candidates(
    db: &mongodb::Database,
    encryption_keys: &crate::crypto::aes::EncryptionKeys,
    actor_user_id: &str,
    actor_api_key_id: Option<&str>,
    owner_id: &str,
    slug: &str,
    method: &http::Method,
    body_len: usize,
) -> AppResult<PoolCandidatePlan> {
    plan_candidates_with_allowlist(
        db,
        encryption_keys,
        actor_user_id,
        actor_api_key_id,
        owner_id,
        slug,
        method,
        None,
        body_len,
        None,
        None,
        None,
    )
    .await
}

/// Read live node prerequisites before selection; execution rechecks its exact route.
pub async fn node_eligibility_reason(
    db: &mongodb::Database,
    owner: &str,
    resolution: &super::proxy_service::UserServiceResolution,
    allowed_nodes: Option<&HashSet<String>>,
) -> AppResult<Option<&'static str>> {
    if resolution.master_credential || resolution.node_id.is_none() {
        return Ok(None);
    }
    let mut nodes = super::node_routing_service::list_configured_binding_node_ids(
        db,
        owner,
        &resolution.target.service.id,
    )
    .await?;
    if let Some(id) = &resolution.node_id {
        nodes.push(id.clone());
    }
    nodes.sort();
    nodes.dedup();
    let mut live = false;
    for id in nodes {
        if allowed_nodes.is_some_and(|allowed| !allowed.contains(&id)) {
            continue;
        }
        let node = db
            .collection::<crate::models::node::Node>("nodes")
            .find_one(doc! {"_id":id,"user_id":owner})
            .await?;
        if let Some(node) = node {
            if !node.is_active || node.status != crate::models::node::NodeStatus::Online {
                continue;
            }
            if let Some(connection) = node.connection_owner.filter(|c| c.expires_at > Utc::now()) {
                live = true;
                if !connection.http_cancellation {
                    return Ok(Some("node_upgrade_required"));
                }
            }
        }
    }
    Ok((!live).then_some("node_offline"))
}

#[allow(clippy::too_many_arguments)]
pub async fn plan_candidates_with_allowlist(
    db: &mongodb::Database,
    encryption_keys: &crate::crypto::aes::EncryptionKeys,
    actor_user_id: &str,
    actor_api_key_id: Option<&str>,
    owner_id: &str,
    slug: &str,
    method: &http::Method,
    operation_path: Option<&str>,
    body_len: usize,
    request_body: Option<&[u8]>,
    allowed_service_ids: Option<&HashSet<String>>,
    allowed_node_ids: Option<&HashSet<String>>,
) -> AppResult<PoolCandidatePlan> {
    let pool = find_pool_by_slug(db, owner_id, slug)
        .await?
        .ok_or_else(|| AppError::ServicePoolNotFound(slug.to_string()))?;
    let chat_request =
        if pool.member_contract == crate::models::service_pool::PoolMemberContract::AiChat {
            Some(super::pool_ai_service::ChatRequest::parse(
                method,
                operation_path.unwrap_or("/"),
                request_body.unwrap_or_default(),
            )?)
        } else {
            None
        };
    let mut candidates = Vec::new();
    let mut compatibility_error = None;
    for member in &pool.members {
        if !member.enabled {
            continue;
        }
        if let Some(allowed) = allowed_service_ids
            && !allowed.contains(&member.user_service_id)
        {
            continue;
        }
        let service =
            crate::services::service_history::collection::<UserService>(db, USER_SERVICES)
                .find_one(doc! { "_id": &member.user_service_id, "user_id": owner_id })
                .await?;
        let Some(service) = service else {
            continue;
        };
        if !service.is_active {
            continue;
        }
        if service.service_type == "ssh" || service.endpoint_id.is_empty() {
            continue;
        }
        if let Some(catalog_id) = &service.catalog_service_id
            && db
                .collection::<mongodb::bson::Document>("downstream_services")
                .find_one(doc! {"_id":catalog_id})
                .projection(doc! {"_id":1})
                .await?
                .is_none()
        {
            continue;
        }
        let resolution =
            match crate::services::proxy_service::read_proxy_authority_snapshot_by_user_service_id(
                db,
                encryption_keys,
                actor_user_id,
                &service.id,
                None,
            )
            .await
            {
                Ok(Some(resolution)) => resolution,
                Ok(None) => {
                    continue;
                }
                Err(error) if member_unavailable(&error) => {
                    continue;
                }
                Err(error) => return Err(error),
            };
        if let Some(node_id) = resolution.node_id.as_deref()
            && let Some(allowed_nodes) = allowed_node_ids
            && !allowed_nodes.contains(node_id)
        {
            continue;
        }
        if pool.strategy == PoolStrategy::Priority
            && node_eligibility_reason(db, owner_id, &resolution, allowed_node_ids)
                .await?
                .is_some()
        {
            continue;
        }
        let chat_plan = if let Some(request) = &chat_request {
            match super::pool_ai_service::plan(
                &resolution.target.service,
                resolution.catalog_service_slug.as_deref(),
                member.model.as_deref().ok_or_else(|| {
                    AppError::BadRequest("AI member requires a model mapping".into())
                })?,
                request,
            ) {
                Ok(plan) => Some(plan),
                Err(error) => {
                    compatibility_error = Some(error);
                    continue;
                }
            }
        } else {
            None
        };
        let operation_path = chat_plan
            .as_ref()
            .map(|plan| plan.path.as_str())
            .or(operation_path);
        if let Some(operation_path) = operation_path
            && (resolution.target.service.proxy_operation_policy.is_some()
                || !resolution.target.service.destination_targets.is_empty())
        {
            let canonical = crate::services::proxy_authorization::CanonicalPath::from_rest_decoded(
                operation_path,
            )?;
            if crate::services::destination_routing::select_target(
                &resolution.target.service,
                method.as_str(),
                &canonical,
            )
            .is_err()
            {
                continue;
            }
        }
        let override_identity = match actor_api_key_id {
            Some(agent_key_id) => {
                crate::services::proxy_service::read_agent_credential_override_identity(
                    db,
                    actor_user_id,
                    agent_key_id,
                    &service.id,
                    &resolution.target,
                )
                .await?
            }
            None => None,
        };
        let health_scope = crate::services::service_pool_health_service::scope_from_resolution(
            db,
            &pool.id,
            &pool.user_id,
            pool.config_revision,
            &resolution,
            member.model.clone(),
            override_identity.as_ref(),
            method.as_str(),
            operation_path.unwrap_or("/"),
        )
        .await?;
        candidates.push(PoolCandidate {
            contract_metadata: super::service_pool_contract::ContractMember {
                catalog_id: service.catalog_service_id.clone(),
                protocol: resolution
                    .target
                    .service
                    .inference
                    .as_ref()
                    .map(|metadata| metadata.wire_protocol),
                declared: member.same_api_compatible,
            },
            chat_plan,
            member: member.clone(),
            service,
            tier: member.priority,
            cooldown_until: None,
            health_scope,
        });
    }
    if candidates.is_empty() {
        return Err(compatibility_error.unwrap_or(AppError::ServicePoolNoViableMember(pool.slug)));
    }

    let priority = pool.strategy == PoolStrategy::Priority;
    if priority {
        super::service_pool_contract::validate(
            pool.member_contract,
            &candidates
                .iter()
                .map(|candidate| candidate.contract_metadata.clone())
                .collect::<Vec<_>>(),
        )?;
    }
    let policy = pool.failover.clone().unwrap_or_default();
    let max_attempts = if !priority || body_len > policy.max_replay_body_bytes as usize {
        1
    } else {
        policy.max_attempts as usize
    };
    let tick = if priority {
        0
    } else {
        increment_counter(db, &pool).await?
    };
    let now = Utc::now();
    let mut filtered = Vec::with_capacity(candidates.len());
    let mut all_cooled_until = None;
    for mut candidate in candidates {
        let scope = &candidate.health_scope;
        candidate.cooldown_until =
            crate::services::service_pool_health_service::load_for_scope(db, scope)
                .await?
                .and_then(
                    |row: crate::models::service_pool_member_health::ServicePoolMemberHealth| {
                        row.cooldown_until
                    },
                );
        if let Some(until) = candidate.cooldown_until.filter(|until| *until > now) {
            all_cooled_until = Some(
                all_cooled_until.map_or(until, |current: chrono::DateTime<Utc>| current.min(until)),
            );
            continue;
        }
        filtered.push(candidate);
    }
    if filtered.is_empty() {
        return Ok(PoolCandidatePlan {
            chat_request,
            pool,
            candidates: Vec::new(),
            max_attempts,
            all_cooled_until,
        });
    }

    let mut ordered = filtered;
    if priority {
        // Preserve every eligible candidate until its tier is actually visited.
        // The executor reserves that tier's balancing counter at selection time.
        ordered.sort_by_key(|candidate| candidate.tier);
    } else {
        let weights: Vec<u32> = ordered.iter().map(|c| c.member.weight.max(1)).collect();
        if let Some(first) = choose_member_index(pool.strategy, &weights, tick) {
            ordered.rotate_left(first);
        }
        ordered.truncate(1);
    }
    if ordered.is_empty() {
        return Err(AppError::ServicePoolNoViableMember(slug.to_string()));
    }
    Ok(PoolCandidatePlan {
        chat_request,
        pool,
        candidates: ordered,
        max_attempts,
        all_cooled_until: None,
    })
}

/// Reserve balancing only when a request reaches this tier. Counter keys are
/// bounded by the configured member tiers, and all config writes clear the map.
pub async fn order_visited_tier(
    db: &mongodb::Database,
    pool: &ServicePool,
    candidates: &mut [PoolCandidate],
) -> AppResult<i64> {
    let Some(first) = candidates.first() else {
        return Ok(0);
    };
    let tier = first.tier;
    let count = candidates
        .iter()
        .take_while(|candidate| candidate.tier == tier)
        .count();
    let key = tier.to_string();
    let counter_path = format!("tier_counters.{key}");
    let updated = db
        .collection::<ServicePool>(SERVICE_POOLS)
        .find_one_and_update(
            doc! { "_id": &pool.id, "config_revision": pool.config_revision, "is_active": true },
            doc! { "$inc": { counter_path: 1_i64 } },
        )
        .return_document(mongodb::options::ReturnDocument::After)
        .await?
        .ok_or_else(|| AppError::Conflict("Pool configuration changed during selection".into()))?;
    let tick = updated.tier_counters.get(&key).copied().unwrap_or(1) - 1;
    let weights: Vec<u32> = candidates[..count]
        .iter()
        .map(|c| c.member.weight.max(1))
        .collect();
    let strategy = match pool.tier_balance {
        TierBalance::RoundRobin => PoolStrategy::RoundRobin,
        TierBalance::Weighted => PoolStrategy::Weighted,
    };
    if let Some(first) = choose_member_index(strategy, &weights, tick) {
        candidates[..count].rotate_left(first);
    }
    Ok(tick)
}

async fn increment_counter(db: &mongodb::Database, pool: &ServicePool) -> AppResult<i64> {
    let updated = db
        .collection::<ServicePool>(SERVICE_POOLS)
        .find_one_and_update(
            doc! { "_id": &pool.id, "user_id": &pool.user_id, "is_active": true },
            doc! { "$inc": { "rr_counter": 1 } },
        )
        .with_options(
            mongodb::options::FindOneAndUpdateOptions::builder()
                .return_document(mongodb::options::ReturnDocument::After)
                .build(),
        )
        .await?
        .ok_or_else(|| AppError::ServicePoolNoViableMember(pool.slug.clone()))?;
    Ok(updated.rr_counter - 1)
}

pub async fn find_first_viable_member(
    db: &mongodb::Database,
    owner_id: &str,
    slug: &str,
) -> AppResult<Option<UserService>> {
    let Some(pool) = find_pool_by_slug(db, owner_id, slug).await? else {
        return Ok(None);
    };
    if pool.strategy == PoolStrategy::Priority {
        return Err(AppError::BadRequest(
            "Priority pools require the HTTP pool executor".into(),
        ));
    }

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
        PoolStrategy::Priority => None,
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
            priority: 0,
            model: None,
            same_api_compatible: false,
            health_reset_generation: 0,
        }
    }

    fn create_input(slug: &str, members: Vec<ServicePoolMember>) -> CreatePoolInput {
        CreatePoolInput {
            slug: slug.to_string(),
            name: format!("{slug} pool"),
            description: Some("test pool".to_string()),
            strategy: PoolStrategy::RoundRobin,
            tier_balance: TierBalance::default(),
            member_contract: crate::models::service_pool::PoolMemberContract::SameApi,
            failover: None,
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

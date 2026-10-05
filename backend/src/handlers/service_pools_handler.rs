use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use mongodb::bson::doc;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::models::service_pool::{
    COLLECTION_NAME as SERVICE_POOLS, FailoverPolicy, PoolMemberContract, PoolStrategy,
    ServicePool, ServicePoolMember, TierBalance,
};
use crate::mw::auth::AuthUser;
use crate::services::{org_service, service_pool_service};

#[derive(Debug, Deserialize, IntoParams)]
pub struct PoolListQuery {
    /// Optional org owner. Omit for personal pools.
    pub org_id: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct PoolMemberRequest {
    pub user_service_id: String,
    #[serde(default)]
    pub weight: Option<u32>,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub priority: Option<u32>,
    #[serde(
        default,
        deserialize_with = "crate::models::nullable_field::deserialize"
    )]
    pub same_api_compatible: Option<Option<bool>>,
    #[serde(
        default,
        deserialize_with = "crate::models::nullable_field::deserialize"
    )]
    pub model: Option<Option<String>>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateServicePoolRequest {
    pub slug: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub strategy: PoolStrategy,
    #[serde(default)]
    pub tier_balance: TierBalance,
    #[serde(default)]
    pub member_contract: PoolMemberContract,
    #[serde(default)]
    pub failover: Option<FailoverPolicy>,
    #[serde(default)]
    pub members: Vec<PoolMemberRequest>,
    #[serde(default)]
    pub is_active: Option<bool>,
    /// Optional org owner. Omit for a personal pool.
    #[serde(default)]
    pub org_id: Option<String>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateServicePoolRequest {
    /// Optional revision last observed by the editor. Stale saves return 409.
    pub expected_revision: Option<i64>,
    pub slug: Option<String>,
    pub name: Option<String>,
    #[serde(
        default,
        deserialize_with = "crate::models::nullable_field::deserialize"
    )]
    pub description: Option<Option<String>>,
    pub strategy: Option<PoolStrategy>,
    pub tier_balance: Option<TierBalance>,
    pub member_contract: Option<PoolMemberContract>,
    #[serde(
        default,
        deserialize_with = "crate::models::nullable_field::deserialize"
    )]
    pub failover: Option<Option<FailoverPolicy>>,
    #[serde(default)]
    pub members: Option<Vec<PoolMemberRequest>>,
    pub is_active: Option<bool>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct SetPoolMembersRequest {
    #[serde(default)]
    pub members: Vec<PoolMemberRequest>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct PoolMemberResponse {
    pub user_service_id: String,
    pub weight: u32,
    pub enabled: bool,
    pub priority: u32,
    pub same_api_compatible: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ServicePoolResponse {
    pub id: String,
    pub user_id: String,
    pub slug: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub strategy: String,
    pub tier_balance: TierBalance,
    pub config_revision: i64,
    pub member_contract: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failover: Option<FailoverPolicy>,
    pub members: Vec<PoolMemberResponse>,
    pub rr_counter: i64,
    pub is_active: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ServicePoolListResponse {
    pub pools: Vec<ServicePoolResponse>,
}

async fn resolve_requested_owner(
    state: &AppState,
    actor: &str,
    org_id: Option<&str>,
    action: &str,
) -> AppResult<String> {
    if let Some(org_id) = org_id.filter(|id| !id.is_empty()) {
        let access = org_service::resolve_owner_access(&state.db, actor, org_id).await?;
        if !access.can_write() {
            return Err(AppError::OrgRoleInsufficient(format!(
                "admin access to the target org is required to {action} service pools"
            )));
        }
        Ok(org_id.to_string())
    } else {
        Ok(actor.to_string())
    }
}

async fn resolve_pool_write_owner(
    state: &AppState,
    actor: &str,
    pool_id: &str,
) -> AppResult<String> {
    let filter = if uuid::Uuid::parse_str(pool_id).is_ok() {
        doc! { "_id": pool_id }
    } else {
        doc! { "slug": pool_id, "user_id": actor }
    };
    let pool = state
        .db
        .collection::<ServicePool>(SERVICE_POOLS)
        .find_one(filter)
        .await?
        .ok_or_else(|| AppError::ServicePoolNotFound(pool_id.to_string()))?;

    let access = org_service::resolve_owner_access(&state.db, actor, &pool.user_id).await?;
    if !access.can_read() {
        return Err(AppError::ServicePoolNotFound(pool_id.to_string()));
    }
    if !access.can_write() {
        return Err(AppError::OrgRoleInsufficient(
            "you do not have permission to modify this service pool".to_string(),
        ));
    }
    Ok(pool.user_id)
}

#[utoipa::path(
    get,
    path = "/api/v1/service-pools",
    params(PoolListQuery),
    responses(
        (status = 200, description = "List of service pools", body = ServicePoolListResponse),
        (status = 401, description = "Unauthorized", body = crate::errors::ErrorResponse)
    ),
    tag = "Service Pools"
)]
pub async fn list_pools(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Query(query): Query<PoolListQuery>,
) -> AppResult<Json<ServicePoolListResponse>> {
    let actor = auth_user.user_id.to_string();
    let owner_id = resolve_requested_owner(&state, &actor, query.org_id.as_deref(), "list").await?;
    let pools = service_pool_service::list_pools(&state.db, &owner_id).await?;
    Ok(Json(ServicePoolListResponse {
        pools: pools.into_iter().map(pool_response).collect(),
    }))
}

#[utoipa::path(
    post,
    path = "/api/v1/service-pools",
    request_body = CreateServicePoolRequest,
    responses(
        (status = 201, description = "Created service pool", body = ServicePoolResponse),
        (status = 400, description = "Invalid pool", body = crate::errors::ErrorResponse),
        (status = 401, description = "Unauthorized", body = crate::errors::ErrorResponse),
        (status = 409, description = "Slug taken", body = crate::errors::ErrorResponse)
    ),
    tag = "Service Pools"
)]
pub async fn create_pool(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<CreateServicePoolRequest>,
) -> AppResult<(StatusCode, Json<ServicePoolResponse>)> {
    let actor = auth_user.user_id.to_string();
    let owner_id =
        resolve_requested_owner(&state, &actor, body.org_id.as_deref(), "create").await?;
    let pool = service_pool_service::create_pool(
        &state.db,
        &owner_id,
        service_pool_service::CreatePoolInput {
            slug: body.slug,
            name: body.name,
            description: body.description,
            strategy: body.strategy,
            tier_balance: body.tier_balance,
            member_contract: body.member_contract,
            failover: body.failover,
            members: resolve_member_requests(&state, &owner_id, body.members)
                .await?
                .into_iter()
                .map(member_from_request)
                .collect(),
            is_active: body.is_active,
        },
    )
    .await?;

    Ok((StatusCode::CREATED, Json(pool_response(pool))))
}

#[utoipa::path(
    get,
    path = "/api/v1/service-pools/{pool_id}",
    params(("pool_id" = String, Path, description = "Service pool ID")),
    responses(
        (status = 200, description = "Service pool", body = ServicePoolResponse),
        (status = 401, description = "Unauthorized", body = crate::errors::ErrorResponse),
        (status = 404, description = "Pool not found", body = crate::errors::ErrorResponse)
    ),
    tag = "Service Pools"
)]
pub async fn get_pool(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(pool_id): Path<String>,
) -> AppResult<Json<ServicePoolResponse>> {
    let actor = auth_user.user_id.to_string();
    let owner_id = resolve_pool_write_owner(&state, &actor, &pool_id).await?;
    let pool = service_pool_service::get_pool(&state.db, &owner_id, &pool_id).await?;
    Ok(Json(pool_response(pool)))
}

#[utoipa::path(
    put,
    path = "/api/v1/service-pools/{pool_id}",
    params(("pool_id" = String, Path, description = "Service pool ID")),
    request_body = UpdateServicePoolRequest,
    responses(
        (status = 200, description = "Updated service pool", body = ServicePoolResponse),
        (status = 400, description = "Invalid pool", body = crate::errors::ErrorResponse),
        (status = 401, description = "Unauthorized", body = crate::errors::ErrorResponse),
        (status = 404, description = "Pool not found", body = crate::errors::ErrorResponse)
    ),
    tag = "Service Pools"
)]
pub async fn update_pool(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(pool_id): Path<String>,
    Json(body): Json<UpdateServicePoolRequest>,
) -> AppResult<Json<ServicePoolResponse>> {
    let actor = auth_user.user_id.to_string();
    let owner_id = resolve_pool_write_owner(&state, &actor, &pool_id).await?;
    let current = service_pool_service::get_pool(&state.db, &owner_id, &pool_id).await?;
    let members = match body.members {
        Some(members) => Some(resolve_member_requests(&state, &owner_id, members).await?),
        None => None,
    };
    let members = members.map(|members| {
        members
            .into_iter()
            .map(|member| {
                let existing = current
                    .members
                    .iter()
                    .find(|row| row.user_service_id == member.user_service_id);
                member_from_request_preserving(existing, member)
            })
            .collect()
    });
    let pool = service_pool_service::update_pool(
        &state.db,
        &owner_id,
        &pool_id,
        service_pool_service::UpdatePoolInput {
            slug: body.slug,
            name: body.name,
            description: body.description,
            strategy: body.strategy,
            tier_balance: body.tier_balance,
            member_contract: body.member_contract,
            failover: body.failover,
            members,
            is_active: body.is_active,
            expected_revision: body.expected_revision.or(Some(current.config_revision)),
        },
    )
    .await?;

    Ok(Json(pool_response(pool)))
}

#[utoipa::path(
    delete,
    path = "/api/v1/service-pools/{pool_id}",
    params(("pool_id" = String, Path, description = "Service pool ID")),
    responses(
        (status = 204, description = "Service pool deleted"),
        (status = 401, description = "Unauthorized", body = crate::errors::ErrorResponse),
        (status = 404, description = "Pool not found", body = crate::errors::ErrorResponse)
    ),
    tag = "Service Pools"
)]
pub async fn delete_pool(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(pool_id): Path<String>,
) -> AppResult<StatusCode> {
    let actor = auth_user.user_id.to_string();
    let owner_id = resolve_pool_write_owner(&state, &actor, &pool_id).await?;
    service_pool_service::delete_pool(&state.db, &owner_id, &pool_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    put,
    path = "/api/v1/service-pools/{pool_id}/members",
    params(("pool_id" = String, Path, description = "Service pool ID")),
    request_body = SetPoolMembersRequest,
    responses(
        (status = 200, description = "Updated service pool members", body = ServicePoolResponse),
        (status = 400, description = "Invalid members", body = crate::errors::ErrorResponse),
        (status = 401, description = "Unauthorized", body = crate::errors::ErrorResponse),
        (status = 404, description = "Pool not found", body = crate::errors::ErrorResponse)
    ),
    tag = "Service Pools"
)]
pub async fn set_members(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(pool_id): Path<String>,
    Json(body): Json<SetPoolMembersRequest>,
) -> AppResult<Json<ServicePoolResponse>> {
    let actor = auth_user.user_id.to_string();
    let owner_id = resolve_pool_write_owner(&state, &actor, &pool_id).await?;
    let current = service_pool_service::get_pool(&state.db, &owner_id, &pool_id).await?;
    let members = resolve_member_requests(&state, &owner_id, body.members)
        .await?
        .into_iter()
        .map(|member| {
            let existing = current
                .members
                .iter()
                .find(|row| row.user_service_id == member.user_service_id);
            member_from_request_preserving(existing, member)
        })
        .collect();
    let pool = service_pool_service::set_members_with_revision(
        &state.db,
        &owner_id,
        &pool_id,
        members,
        Some(current.config_revision),
    )
    .await?;
    Ok(Json(pool_response(pool)))
}

#[utoipa::path(
    post,
    path = "/api/v1/service-pools/{pool_id}/members",
    params(("pool_id" = String, Path, description = "Service pool ID")),
    request_body = PoolMemberRequest,
    responses(
        (status = 200, description = "Added or updated service pool member", body = ServicePoolResponse),
        (status = 400, description = "Invalid member", body = crate::errors::ErrorResponse),
        (status = 401, description = "Unauthorized", body = crate::errors::ErrorResponse),
        (status = 404, description = "Pool not found", body = crate::errors::ErrorResponse)
    ),
    tag = "Service Pools"
)]
pub async fn add_member(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(pool_id): Path<String>,
    Json(body): Json<PoolMemberRequest>,
) -> AppResult<Json<ServicePoolResponse>> {
    let actor = auth_user.user_id.to_string();
    let owner_id = resolve_pool_write_owner(&state, &actor, &pool_id).await?;
    let current = service_pool_service::get_pool(&state.db, &owner_id, &pool_id).await?;
    let mut body = body;
    body.user_service_id = service_pool_service::resolve_member_identifier(
        &state.db,
        &owner_id,
        &body.user_service_id,
    )
    .await?;
    let existing = current
        .members
        .iter()
        .find(|member| member.user_service_id == body.user_service_id);
    let pool = service_pool_service::add_member_with_revision(
        &state.db,
        &owner_id,
        &pool_id,
        member_from_request_preserving(existing, body),
        Some(current.config_revision),
    )
    .await?;
    Ok(Json(pool_response(pool)))
}

#[utoipa::path(
    delete,
    path = "/api/v1/service-pools/{pool_id}/members/{user_service_id}",
    params(
        ("pool_id" = String, Path, description = "Service pool ID"),
        ("user_service_id" = String, Path, description = "UserService ID")
    ),
    responses(
        (status = 200, description = "Removed service pool member", body = ServicePoolResponse),
        (status = 400, description = "Invalid member", body = crate::errors::ErrorResponse),
        (status = 401, description = "Unauthorized", body = crate::errors::ErrorResponse),
        (status = 404, description = "Pool not found", body = crate::errors::ErrorResponse)
    ),
    tag = "Service Pools"
)]
pub async fn remove_member(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path((pool_id, user_service_id)): Path<(String, String)>,
) -> AppResult<Json<ServicePoolResponse>> {
    let actor = auth_user.user_id.to_string();
    let owner_id = resolve_pool_write_owner(&state, &actor, &pool_id).await?;
    let current = service_pool_service::get_pool(&state.db, &owner_id, &pool_id).await?;
    let user_service_id = if current
        .members
        .iter()
        .any(|m| m.user_service_id == user_service_id)
    {
        user_service_id
    } else {
        service_pool_service::resolve_member_identifier(&state.db, &owner_id, &user_service_id)
            .await?
    };
    let pool = service_pool_service::remove_member_with_revision(
        &state.db,
        &owner_id,
        &pool_id,
        &user_service_id,
        Some(current.config_revision),
    )
    .await?;
    Ok(Json(pool_response(pool)))
}

async fn resolve_member_requests(
    state: &AppState,
    owner: &str,
    mut members: Vec<PoolMemberRequest>,
) -> AppResult<Vec<PoolMemberRequest>> {
    for member in &mut members {
        member.user_service_id = service_pool_service::resolve_member_identifier(
            &state.db,
            owner,
            &member.user_service_id,
        )
        .await?;
    }
    Ok(members)
}

fn member_from_request(member: PoolMemberRequest) -> ServicePoolMember {
    member_from_request_preserving(None, member)
}

fn member_from_request_preserving(
    existing: Option<&ServicePoolMember>,
    member: PoolMemberRequest,
) -> ServicePoolMember {
    ServicePoolMember {
        user_service_id: member.user_service_id,
        weight: member
            .weight
            .or_else(|| existing.map(|row| row.weight))
            .unwrap_or(1),
        enabled: member
            .enabled
            .or_else(|| existing.map(|row| row.enabled))
            .unwrap_or(true),
        priority: member
            .priority
            .or_else(|| existing.map(|row| row.priority))
            .unwrap_or(0),
        model: match member.model {
            Some(value) => value,
            None => existing.and_then(|row| row.model.clone()),
        },
        same_api_compatible: match member.same_api_compatible {
            Some(value) => value.unwrap_or(false),
            None => existing.map(|row| row.same_api_compatible).unwrap_or(false),
        },
        health_reset_generation: existing.map(|row| row.health_reset_generation).unwrap_or(0),
    }
}

fn pool_response(pool: ServicePool) -> ServicePoolResponse {
    let strategy = PoolStrategy::parse(pool.strategy.as_str()).unwrap_or(pool.strategy);
    ServicePoolResponse {
        id: pool.id,
        user_id: pool.user_id,
        slug: pool.slug,
        name: pool.name,
        description: pool.description,
        strategy: strategy.as_str().to_string(),
        tier_balance: pool.tier_balance,
        config_revision: pool.config_revision,
        member_contract: pool.member_contract.as_str().to_string(),
        failover: pool.failover,
        members: pool
            .members
            .into_iter()
            .map(|member| PoolMemberResponse {
                user_service_id: member.user_service_id,
                weight: member.weight,
                enabled: member.enabled,
                priority: member.priority,
                same_api_compatible: member.same_api_compatible,
                model: member.model,
            })
            .collect(),
        rr_counter: pool.rr_counter,
        is_active: pool.is_active,
        created_at: pool.created_at.to_rfc3339(),
        updated_at: pool.updated_at.to_rfc3339(),
    }
}

#[derive(Debug, Deserialize, IntoParams)]
pub struct PoolCandidatesQuery {
    pub after: Option<String>,
    pub search: Option<String>,
    pub limit: Option<u32>,
    pub strategy: Option<PoolStrategy>,
    pub declared_peer_ids: Option<String>,
    pub org_id: Option<String>,
    #[serde(default)]
    pub member_contract: Option<PoolMemberContract>,
    /// Comma-separated selected UserService IDs in the draft (maximum 50).
    pub peer_ids: Option<String>,
    /// Return only draft peer IDs, independently of inventory search/pagination.
    #[serde(default)]
    pub selected_only: bool,
    /// Inspect a concrete operation (default true for compatibility).
    /// False browses connections without operation checks or cooldown health.
    pub check_operation: Option<bool>,
    pub method: Option<String>,
    pub path: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub struct PoolCandidateResponse {
    pub user_service_id: String,
    pub name: String,
    pub slug: String,
    pub is_active: bool,
    pub eligible: bool,
    pub reason: Option<String>,
    pub credential_binding: String,
    pub protocol: Option<crate::models::downstream_service::InferenceWireProtocol>,
    pub catalog_service_id: Option<String>,
    /// Authoritative original catalog grouping metadata. Custom connections
    /// use the stable "Custom connections" group and a null catalog ID.
    pub group_name: String,
    pub group_slug: Option<String>,
    pub requires_compatibility_declaration: bool,
    pub cooldown_until: Option<String>,
    pub consecutive_failures: i64,
    pub last_status: Option<i32>,
}

#[derive(Serialize, ToSchema)]
pub struct PoolCandidatesResponse {
    pub operation_checked: bool,
    pub method: Option<String>,
    pub path: Option<String>,
    pub next_cursor: Option<String>,
    pub has_more: bool,
    pub candidates: Vec<PoolCandidateResponse>,
}

async fn inspect_pool_candidates(
    state: &AppState,
    auth: &AuthUser,
    owner: &str,
    pool: Option<&ServicePool>,
    query: PoolCandidatesQuery,
    members_only: bool,
) -> AppResult<Json<PoolCandidatesResponse>> {
    let allowed =
        (!auth.allow_all_services).then(|| auth.allowed_service_ids.iter().cloned().collect());
    let nodes = (!auth.allow_all_nodes).then(|| auth.allowed_node_ids.iter().cloned().collect());
    let check_operation = query.check_operation.unwrap_or(true);
    let method = query.method.as_deref().unwrap_or("POST");
    http::Method::from_bytes(method.as_bytes())
        .map_err(|_| AppError::BadRequest("Invalid method".into()))?;
    let strategy = if members_only {
        pool.map_or(PoolStrategy::Priority, |pool| pool.strategy)
    } else {
        query
            .strategy
            .or_else(|| pool.map(|p| p.strategy))
            .unwrap_or(PoolStrategy::Priority)
    };
    let contract = if members_only {
        pool.map(|p| p.member_contract).unwrap_or_default()
    } else {
        query
            .member_contract
            .or_else(|| pool.map(|p| p.member_contract))
            .unwrap_or_default()
    };
    let path = query
        .path
        .as_deref()
        .unwrap_or(if contract == PoolMemberContract::AiChat {
            "chat/completions"
        } else {
            "/"
        });
    if check_operation {
        crate::services::proxy_authorization::CanonicalPath::from_rest_decoded(path)?;
    }
    let peers = query.peer_ids.as_ref().map(|value| {
        value
            .split(',')
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect::<Vec<_>>()
    });
    if peers.as_ref().is_some_and(|peers| {
        peers.len() > 50 || peers.iter().any(|id| uuid::Uuid::parse_str(id).is_err())
    }) {
        return Err(AppError::BadRequest("Invalid draft members".into()));
    }
    if let Some(declarations) = &query.declared_peer_ids {
        let ids: Vec<_> = declarations
            .split(',')
            .filter(|id| !id.is_empty())
            .collect();
        if ids.len() > 50 || ids.iter().any(|id| uuid::Uuid::parse_str(id).is_err()) {
            return Err(AppError::BadRequest("Invalid draft declarations".into()));
        }
    }
    let candidates = crate::services::service_pool_inspection::inspect(
        &state.db,
        &state.encryption_keys,
        &auth.user_id.to_string(),
        auth.api_key_id.as_deref(),
        owner,
        pool,
        strategy,
        contract,
        method,
        path,
        allowed.as_ref(),
        nodes.as_ref(),
        crate::services::service_pool_inspection::InspectionQuery {
            after: query.after.as_deref(),
            search: query.search.as_deref(),
            limit: query.limit.unwrap_or(100),
            members_only,
            selected_only: query.selected_only && !members_only,
            inventory_only: !check_operation,
            peer_ids: peers.as_deref(),
            declared_peer_ids: query.declared_peer_ids.as_deref(),
        },
    )
    .await?;
    Ok(Json(PoolCandidatesResponse {
        operation_checked: check_operation,
        method: check_operation.then(|| method.into()),
        path: check_operation.then(|| path.into()),
        has_more: candidates.next_cursor.is_some(),
        next_cursor: candidates.next_cursor,
        candidates: candidates
            .candidates
            .into_iter()
            .map(|row| PoolCandidateResponse {
                user_service_id: row.user_service_id,
                name: row.name,
                slug: row.slug,
                is_active: row.is_active,
                eligible: row.eligible,
                reason: row.reason,
                credential_binding: row.credential_binding,
                protocol: row.protocol,
                catalog_service_id: row.catalog_service_id,
                group_name: row.group_name,
                group_slug: row.group_slug,
                requires_compatibility_declaration: row.requires_compatibility_declaration,
                cooldown_until: row.cooldown_until.map(|t| t.to_rfc3339()),
                consecutive_failures: row.consecutive_failures,
                last_status: row.last_status,
            })
            .collect(),
    }))
}

#[utoipa::path(get, path="/api/v1/service-pools/candidates", params(PoolCandidatesQuery), responses((status=200, body=PoolCandidatesResponse)), tag="Service Pools")]
pub async fn candidates(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(query): Query<PoolCandidatesQuery>,
) -> AppResult<Json<PoolCandidatesResponse>> {
    let owner = resolve_requested_owner(
        &state,
        &auth.user_id.to_string(),
        query.org_id.as_deref(),
        "inspect",
    )
    .await?;
    inspect_pool_candidates(&state, &auth, &owner, None, query, false).await
}

#[utoipa::path(get, path="/api/v1/service-pools/{pool_id}/candidates", params(("pool_id"=String, Path), PoolCandidatesQuery), responses((status=200, body=PoolCandidatesResponse)), tag="Service Pools")]
pub async fn pool_candidates(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Query(query): Query<PoolCandidatesQuery>,
) -> AppResult<Json<PoolCandidatesResponse>> {
    let owner = resolve_pool_write_owner(&state, &auth.user_id.to_string(), &id).await?;
    let pool = service_pool_service::get_pool(&state.db, &owner, &id).await?;
    inspect_pool_candidates(&state, &auth, &owner, Some(&pool), query, false).await
}

#[utoipa::path(get, path="/api/v1/service-pools/{pool_id}/health", params(("pool_id"=String, Path), PoolCandidatesQuery), responses((status=200, body=PoolCandidatesResponse)), tag="Service Pools")]
pub async fn health(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Query(query): Query<PoolCandidatesQuery>,
) -> AppResult<Json<PoolCandidatesResponse>> {
    let owner = resolve_pool_write_owner(&state, &auth.user_id.to_string(), &id).await?;
    let pool = service_pool_service::get_pool(&state.db, &owner, &id).await?;
    let Json(mut inspected) =
        inspect_pool_candidates(&state, &auth, &owner, Some(&pool), query, true).await?;
    inspected.candidates.retain(|row| {
        pool.members
            .iter()
            .any(|member| member.user_service_id == row.user_service_id)
    });
    Ok(Json(inspected))
}

#[derive(Deserialize, ToSchema)]
pub struct ResetPoolHealthRequest {
    pub user_service_id: Option<String>,
}
#[derive(Serialize, ToSchema)]
pub struct ResetPoolHealthResponse {
    pub reset: bool,
}

#[utoipa::path(post, path="/api/v1/service-pools/{pool_id}/health/reset", params(("pool_id"=String, Path)), request_body=ResetPoolHealthRequest, responses((status=200, body=ResetPoolHealthResponse)), tag="Service Pools")]
pub async fn reset_health(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(body): Json<ResetPoolHealthRequest>,
) -> AppResult<Json<ResetPoolHealthResponse>> {
    let owner = resolve_pool_write_owner(&state, &auth.user_id.to_string(), &id).await?;
    let pool = service_pool_service::get_pool(&state.db, &owner, &id).await?;
    let member = if let Some(id) = body.user_service_id {
        Some(if pool.members.iter().any(|m| m.user_service_id == id) {
            id
        } else {
            service_pool_service::resolve_member_identifier(&state.db, &owner, &id).await?
        })
    } else {
        None
    };
    crate::services::service_pool_health_service::reset_pool(
        &state.db,
        &pool.id,
        &owner,
        member.as_deref(),
    )
    .await?;
    Ok(Json(ResetPoolHealthResponse { reset: true }))
}

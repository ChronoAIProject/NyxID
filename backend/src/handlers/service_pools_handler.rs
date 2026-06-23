use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use mongodb::bson::doc;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::models::service_pool::{
    COLLECTION_NAME as SERVICE_POOLS, PoolStrategy, ServicePool, ServicePoolMember,
};
use crate::mw::auth::AuthUser;
use crate::services::{org_service, service_pool_service};

#[derive(Deserialize, ToSchema)]
pub struct PoolOwnerQuery {
    #[serde(default)]
    pub org_id: Option<String>,
}

#[derive(Deserialize, ToSchema)]
pub struct CreateServicePoolRequest {
    pub slug: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    /// "round_robin" | "weighted"
    pub strategy: String,
    #[serde(default)]
    pub members: Vec<ServicePoolMemberRequest>,
    #[serde(default)]
    pub org_id: Option<String>,
}

#[derive(Deserialize, ToSchema)]
pub struct UpdateServicePoolRequest {
    #[serde(default)]
    pub slug: Option<String>,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    /// "round_robin" | "weighted"
    pub strategy: Option<String>,
    #[serde(default)]
    pub is_active: Option<bool>,
}

#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
pub struct ServicePoolMemberRequest {
    pub user_service_id: String,
    #[serde(default = "default_member_weight")]
    pub weight: u32,
    #[serde(default = "default_member_enabled")]
    pub enabled: bool,
}

fn default_member_weight() -> u32 {
    1
}

fn default_member_enabled() -> bool {
    true
}

#[derive(Deserialize, ToSchema)]
pub struct SetServicePoolMembersRequest {
    pub members: Vec<ServicePoolMemberRequest>,
}

#[derive(Deserialize, ToSchema)]
pub struct AddServicePoolMemberRequest {
    pub user_service_id: String,
    #[serde(default = "default_member_weight")]
    pub weight: u32,
    #[serde(default = "default_member_enabled")]
    pub enabled: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ServicePoolMemberResponse {
    pub user_service_id: String,
    pub weight: u32,
    pub enabled: bool,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ServicePoolResponse {
    pub id: String,
    pub owner_user_id: String,
    pub slug: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub strategy: String,
    pub members: Vec<ServicePoolMemberResponse>,
    pub rr_counter: i64,
    pub is_active: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ServicePoolListResponse {
    pub pools: Vec<ServicePoolResponse>,
}

fn member_request(member: ServicePoolMemberRequest) -> ServicePoolMember {
    ServicePoolMember {
        user_service_id: member.user_service_id,
        weight: member.weight,
        enabled: member.enabled,
    }
}

fn parse_strategy(value: &str) -> AppResult<PoolStrategy> {
    PoolStrategy::parse(value).ok_or_else(|| {
        AppError::ValidationError(format!(
            "strategy must be round_robin|weighted, got '{value}'"
        ))
    })
}

fn service_pool_response(pool: ServicePool) -> ServicePoolResponse {
    ServicePoolResponse {
        id: pool.id,
        owner_user_id: pool.user_id,
        slug: pool.slug,
        name: pool.name,
        description: pool.description,
        strategy: pool.strategy.as_str().to_string(),
        members: pool
            .members
            .into_iter()
            .map(|m| ServicePoolMemberResponse {
                user_service_id: m.user_service_id,
                weight: m.weight,
                enabled: m.enabled,
            })
            .collect(),
        rr_counter: pool.rr_counter,
        is_active: pool.is_active,
        created_at: pool.created_at.to_rfc3339(),
        updated_at: pool.updated_at.to_rfc3339(),
    }
}

async fn resolve_requested_owner(
    state: &AppState,
    actor: &str,
    org_id: Option<&str>,
    write: bool,
) -> AppResult<String> {
    let Some(org_id) = org_id else {
        return Ok(actor.to_string());
    };

    let access = org_service::resolve_owner_access(&state.db, actor, org_id).await?;
    if write {
        if access.can_write() {
            Ok(org_id.to_string())
        } else {
            Err(AppError::OrgRoleInsufficient(
                "you must be an admin of the target org to manage service pools".to_string(),
            ))
        }
    } else if access.can_write() {
        Ok(org_id.to_string())
    } else {
        Err(AppError::OrgRoleInsufficient(
            "admin access to the target org is required to list its service pools".to_string(),
        ))
    }
}

async fn resolve_pool_write_owner(
    state: &AppState,
    actor: &str,
    pool_id: &str,
) -> AppResult<String> {
    let pool = state
        .db
        .collection::<ServicePool>(SERVICE_POOLS)
        .find_one(doc! { "_id": pool_id })
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

async fn resolve_pool_read_owner(
    state: &AppState,
    actor: &str,
    pool_id: &str,
) -> AppResult<String> {
    let pool = state
        .db
        .collection::<ServicePool>(SERVICE_POOLS)
        .find_one(doc! { "_id": pool_id })
        .await?
        .ok_or_else(|| AppError::ServicePoolNotFound(pool_id.to_string()))?;
    let access = org_service::resolve_owner_access(&state.db, actor, &pool.user_id).await?;
    if access.can_write() {
        Ok(pool.user_id)
    } else {
        Err(AppError::ServicePoolNotFound(pool_id.to_string()))
    }
}

#[utoipa::path(
    get,
    path = "/api/v1/service-pools",
    responses(
        (status = 200, description = "List of service pools", body = ServicePoolListResponse),
        (status = 401, description = "Unauthorized", body = crate::errors::ErrorResponse)
    ),
    tag = "Service Pools"
)]
pub async fn list_pools(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Query(query): Query<PoolOwnerQuery>,
) -> AppResult<Json<ServicePoolListResponse>> {
    let actor = auth_user.user_id.to_string();
    let owner = resolve_requested_owner(&state, &actor, query.org_id.as_deref(), false).await?;
    let pools = service_pool_service::list_pools(&state.db, &owner).await?;
    Ok(Json(ServicePoolListResponse {
        pools: pools.into_iter().map(service_pool_response).collect(),
    }))
}

#[utoipa::path(
    post,
    path = "/api/v1/service-pools",
    request_body = CreateServicePoolRequest,
    responses(
        (status = 201, description = "Created service pool", body = ServicePoolResponse),
        (status = 400, description = "Validation error", body = crate::errors::ErrorResponse),
        (status = 401, description = "Unauthorized", body = crate::errors::ErrorResponse),
        (status = 409, description = "Slug already taken", body = crate::errors::ErrorResponse)
    ),
    tag = "Service Pools"
)]
pub async fn create_pool(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<CreateServicePoolRequest>,
) -> AppResult<impl IntoResponse> {
    let actor = auth_user.user_id.to_string();
    let owner = resolve_requested_owner(&state, &actor, body.org_id.as_deref(), true).await?;
    let pool = service_pool_service::create_pool(
        &state.db,
        &owner,
        service_pool_service::CreatePoolInput {
            slug: body.slug,
            name: body.name,
            description: body.description,
            strategy: parse_strategy(&body.strategy)?,
            members: body.members.into_iter().map(member_request).collect(),
        },
    )
    .await?;
    Ok((StatusCode::CREATED, Json(service_pool_response(pool))))
}

#[utoipa::path(
    get,
    path = "/api/v1/service-pools/{pool_id}",
    params(("pool_id" = String, Path, description = "Service pool ID")),
    responses(
        (status = 200, description = "Service pool", body = ServicePoolResponse),
        (status = 401, description = "Unauthorized", body = crate::errors::ErrorResponse),
        (status = 404, description = "Service pool not found", body = crate::errors::ErrorResponse)
    ),
    tag = "Service Pools"
)]
pub async fn get_pool(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(pool_id): Path<String>,
) -> AppResult<Json<ServicePoolResponse>> {
    let actor = auth_user.user_id.to_string();
    let owner = resolve_pool_read_owner(&state, &actor, &pool_id).await?;
    let pool = service_pool_service::get_pool(&state.db, &owner, &pool_id).await?;
    Ok(Json(service_pool_response(pool)))
}

#[utoipa::path(
    put,
    path = "/api/v1/service-pools/{pool_id}",
    params(("pool_id" = String, Path, description = "Service pool ID")),
    request_body = UpdateServicePoolRequest,
    responses(
        (status = 200, description = "Updated service pool", body = ServicePoolResponse),
        (status = 400, description = "Validation error", body = crate::errors::ErrorResponse),
        (status = 401, description = "Unauthorized", body = crate::errors::ErrorResponse),
        (status = 404, description = "Service pool not found", body = crate::errors::ErrorResponse)
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
    let owner = resolve_pool_write_owner(&state, &actor, &pool_id).await?;
    let pool = service_pool_service::update_pool(
        &state.db,
        &owner,
        &pool_id,
        service_pool_service::UpdatePoolInput {
            slug: body.slug,
            name: body.name,
            description: body.description,
            strategy: body.strategy.as_deref().map(parse_strategy).transpose()?,
            is_active: body.is_active,
        },
    )
    .await?;
    Ok(Json(service_pool_response(pool)))
}

#[utoipa::path(
    delete,
    path = "/api/v1/service-pools/{pool_id}",
    params(("pool_id" = String, Path, description = "Service pool ID")),
    responses(
        (status = 204, description = "Service pool deactivated"),
        (status = 401, description = "Unauthorized", body = crate::errors::ErrorResponse),
        (status = 404, description = "Service pool not found", body = crate::errors::ErrorResponse)
    ),
    tag = "Service Pools"
)]
pub async fn delete_pool(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(pool_id): Path<String>,
) -> AppResult<impl IntoResponse> {
    let actor = auth_user.user_id.to_string();
    let owner = resolve_pool_write_owner(&state, &actor, &pool_id).await?;
    service_pool_service::delete_pool(&state.db, &owner, &pool_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    put,
    path = "/api/v1/service-pools/{pool_id}/members",
    params(("pool_id" = String, Path, description = "Service pool ID")),
    request_body = SetServicePoolMembersRequest,
    responses(
        (status = 200, description = "Updated service pool members", body = ServicePoolResponse),
        (status = 400, description = "Invalid member", body = crate::errors::ErrorResponse),
        (status = 401, description = "Unauthorized", body = crate::errors::ErrorResponse),
        (status = 404, description = "Service pool not found", body = crate::errors::ErrorResponse)
    ),
    tag = "Service Pools"
)]
pub async fn set_members(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(pool_id): Path<String>,
    Json(body): Json<SetServicePoolMembersRequest>,
) -> AppResult<Json<ServicePoolResponse>> {
    let actor = auth_user.user_id.to_string();
    let owner = resolve_pool_write_owner(&state, &actor, &pool_id).await?;
    let pool = service_pool_service::set_members(
        &state.db,
        &owner,
        &pool_id,
        body.members.into_iter().map(member_request).collect(),
    )
    .await?;
    Ok(Json(service_pool_response(pool)))
}

#[utoipa::path(
    post,
    path = "/api/v1/service-pools/{pool_id}/members",
    params(("pool_id" = String, Path, description = "Service pool ID")),
    request_body = AddServicePoolMemberRequest,
    responses(
        (status = 200, description = "Added service pool member", body = ServicePoolResponse),
        (status = 400, description = "Invalid member", body = crate::errors::ErrorResponse),
        (status = 401, description = "Unauthorized", body = crate::errors::ErrorResponse),
        (status = 404, description = "Service pool not found", body = crate::errors::ErrorResponse)
    ),
    tag = "Service Pools"
)]
pub async fn add_member(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(pool_id): Path<String>,
    Json(body): Json<AddServicePoolMemberRequest>,
) -> AppResult<Json<ServicePoolResponse>> {
    let actor = auth_user.user_id.to_string();
    let owner = resolve_pool_write_owner(&state, &actor, &pool_id).await?;
    let pool = service_pool_service::add_member(
        &state.db,
        &owner,
        &pool_id,
        ServicePoolMember {
            user_service_id: body.user_service_id,
            weight: body.weight,
            enabled: body.enabled,
        },
    )
    .await?;
    Ok(Json(service_pool_response(pool)))
}

#[utoipa::path(
    delete,
    path = "/api/v1/service-pools/{pool_id}/members/{user_service_id}",
    params(
        ("pool_id" = String, Path, description = "Service pool ID"),
        ("user_service_id" = String, Path, description = "User service member ID")
    ),
    responses(
        (status = 200, description = "Removed service pool member", body = ServicePoolResponse),
        (status = 400, description = "Invalid member", body = crate::errors::ErrorResponse),
        (status = 401, description = "Unauthorized", body = crate::errors::ErrorResponse),
        (status = 404, description = "Service pool not found", body = crate::errors::ErrorResponse)
    ),
    tag = "Service Pools"
)]
pub async fn remove_member(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path((pool_id, user_service_id)): Path<(String, String)>,
) -> AppResult<Json<ServicePoolResponse>> {
    let actor = auth_user.user_id.to_string();
    let owner = resolve_pool_write_owner(&state, &actor, &pool_id).await?;
    let pool =
        service_pool_service::remove_member(&state.db, &owner, &pool_id, &user_service_id).await?;
    Ok(Json(service_pool_response(pool)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn response_maps_model_without_serializing_model() {
        let now = chrono::Utc::now();
        let pool = ServicePool {
            id: "pool-1".to_string(),
            user_id: "owner-1".to_string(),
            slug: "pool-a".to_string(),
            name: "Pool A".to_string(),
            description: Some("description".to_string()),
            strategy: PoolStrategy::Weighted,
            members: vec![ServicePoolMember {
                user_service_id: "svc-1".to_string(),
                weight: 2,
                enabled: true,
            }],
            rr_counter: 3,
            is_active: true,
            created_at: now,
            updated_at: now,
        };

        let response = service_pool_response(pool);
        assert_eq!(response.id, "pool-1");
        assert_eq!(response.owner_user_id, "owner-1");
        assert_eq!(response.strategy, "weighted");
        assert_eq!(response.members[0].weight, 2);
        assert_eq!(response.created_at, now.to_rfc3339());
    }
}

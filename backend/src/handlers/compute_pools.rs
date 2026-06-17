//! Compute pool management endpoints (`/api/v1/compute/pools`).

use axum::{
    Json,
    extract::{Path, State},
};
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::models::compute_pool::{ComputePool, ComputePoolVisibility, ComputeSchedulingPolicy};
use crate::mw::auth::AuthUser;
use crate::services::{audit_service, compute_pool_service, org_service};

#[derive(Deserialize)]
pub struct CreateComputePoolRequest {
    pub slug: String,
    pub name: String,
    pub description: Option<String>,
    pub visibility: Option<String>,
    pub scheduling_policy: Option<String>,
    pub max_workers: Option<u32>,
    pub max_queue_length: Option<u32>,
    pub per_user_max_inflight: Option<u32>,
    pub task_timeout_secs: Option<u64>,
    pub target_org_id: Option<String>,
}

#[derive(Deserialize)]
pub struct UpdateComputePoolRequest {
    pub name: Option<String>,
    pub description: Option<String>,
    pub visibility: Option<String>,
    pub scheduling_policy: Option<String>,
    pub max_workers: Option<u32>,
    pub max_queue_length: Option<u32>,
    pub per_user_max_inflight: Option<u32>,
    pub task_timeout_secs: Option<u64>,
    pub is_active: Option<bool>,
}

#[derive(Serialize)]
pub struct ComputePoolInfo {
    pub id: String,
    pub slug: String,
    pub name: String,
    pub description: Option<String>,
    pub visibility: String,
    pub scheduling_policy: String,
    pub owner_user_id: String,
    pub can_manage: bool,
    pub max_workers: u32,
    pub max_queue_length: u32,
    pub per_user_max_inflight: u32,
    pub task_timeout_secs: u64,
    pub is_active: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Serialize)]
pub struct CreateComputePoolResponse {
    #[serde(flatten)]
    pub pool: ComputePoolInfo,
    pub worker_token: String,
}

#[derive(Serialize)]
pub struct RotateComputePoolTokenResponse {
    pub id: String,
    pub slug: String,
    pub worker_token: String,
}

#[derive(Serialize)]
pub struct ListComputePoolsResponse {
    pub pools: Vec<ComputePoolInfo>,
}

fn parse_visibility(value: &str) -> AppResult<ComputePoolVisibility> {
    match value {
        "private" => Ok(ComputePoolVisibility::Private),
        "org" => Ok(ComputePoolVisibility::Org),
        "platform" => Ok(ComputePoolVisibility::Platform),
        other => Err(AppError::ValidationError(format!(
            "visibility must be private|org|platform, got '{other}'"
        ))),
    }
}

fn parse_policy(value: &str) -> AppResult<ComputeSchedulingPolicy> {
    match value {
        "fifo" => Ok(ComputeSchedulingPolicy::Fifo),
        "least_busy" => Ok(ComputeSchedulingPolicy::LeastBusy),
        "model_fit" => Ok(ComputeSchedulingPolicy::ModelFit),
        other => Err(AppError::ValidationError(format!(
            "scheduling_policy must be fifo|least_busy|model_fit, got '{other}'"
        ))),
    }
}

fn pool_info(pool: &ComputePool, can_manage: bool) -> ComputePoolInfo {
    ComputePoolInfo {
        id: pool.id.clone(),
        slug: pool.slug.clone(),
        name: pool.name.clone(),
        description: pool.description.clone(),
        visibility: pool.visibility.as_str().to_string(),
        scheduling_policy: pool.scheduling_policy.as_str().to_string(),
        owner_user_id: pool.user_id.clone(),
        can_manage,
        max_workers: pool.max_workers,
        max_queue_length: pool.max_queue_length,
        per_user_max_inflight: pool.per_user_max_inflight,
        task_timeout_secs: pool.task_timeout_secs,
        is_active: pool.is_active,
        created_at: pool.created_at.to_rfc3339(),
        updated_at: pool.updated_at.to_rfc3339(),
    }
}

async fn can_manage(state: &AppState, actor: &str, pool: &ComputePool) -> bool {
    compute_pool_service::ensure_can_manage(&state.db, actor, pool)
        .await
        .is_ok()
}

pub async fn create_pool(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<CreateComputePoolRequest>,
) -> AppResult<Json<CreateComputePoolResponse>> {
    let actor = auth_user.user_id.to_string();
    let owner = match body.target_org_id.as_deref() {
        Some(org_id) => {
            let access = org_service::resolve_owner_access(&state.db, &actor, org_id).await?;
            if !access.can_write() {
                return Err(AppError::Forbidden(
                    "you must be an admin of the target org to create a compute pool under it"
                        .to_string(),
                ));
            }
            org_id.to_string()
        }
        None => actor.clone(),
    };

    let visibility = body
        .visibility
        .as_deref()
        .map(parse_visibility)
        .transpose()?;
    let scheduling_policy = body
        .scheduling_policy
        .as_deref()
        .map(parse_policy)
        .transpose()?;
    let (pool, worker_token) = compute_pool_service::create_pool(
        &state.db,
        &owner,
        compute_pool_service::CreateComputePoolInput {
            slug: body.slug,
            name: body.name,
            description: body.description,
            visibility,
            scheduling_policy,
            max_workers: body.max_workers,
            max_queue_length: body.max_queue_length,
            per_user_max_inflight: body.per_user_max_inflight,
            task_timeout_secs: body.task_timeout_secs,
        },
    )
    .await?;

    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "compute_pool_created",
        Some(serde_json::json!({
            "pool_id": &pool.id,
            "slug": &pool.slug,
            "visibility": pool.visibility.as_str(),
            "owner_user_id": &pool.user_id,
        })),
    );

    Ok(Json(CreateComputePoolResponse {
        pool: pool_info(&pool, true),
        worker_token,
    }))
}

pub async fn list_pools(
    State(state): State<AppState>,
    auth_user: AuthUser,
) -> AppResult<Json<ListComputePoolsResponse>> {
    let actor = auth_user.user_id.to_string();
    let pools = compute_pool_service::list_visible_pools(&state.db, &actor).await?;
    let mut infos = Vec::with_capacity(pools.len());
    for pool in &pools {
        infos.push(pool_info(pool, can_manage(&state, &actor, pool).await));
    }
    Ok(Json(ListComputePoolsResponse { pools: infos }))
}

pub async fn get_pool(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(id_or_slug): Path<String>,
) -> AppResult<Json<ComputePoolInfo>> {
    let actor = auth_user.user_id.to_string();
    let pool = compute_pool_service::get_pool(&state.db, &id_or_slug).await?;
    compute_pool_service::ensure_can_view(&state.db, &actor, &pool).await?;
    let manage = can_manage(&state, &actor, &pool).await;
    Ok(Json(pool_info(&pool, manage)))
}

pub async fn update_pool(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(id_or_slug): Path<String>,
    Json(body): Json<UpdateComputePoolRequest>,
) -> AppResult<Json<ComputePoolInfo>> {
    let actor = auth_user.user_id.to_string();
    let visibility = body
        .visibility
        .as_deref()
        .map(parse_visibility)
        .transpose()?;
    let scheduling_policy = body
        .scheduling_policy
        .as_deref()
        .map(parse_policy)
        .transpose()?;
    let pool = compute_pool_service::update_pool(
        &state.db,
        &actor,
        &id_or_slug,
        compute_pool_service::UpdateComputePoolInput {
            name: body.name,
            description: body.description,
            visibility,
            scheduling_policy,
            max_workers: body.max_workers,
            max_queue_length: body.max_queue_length,
            per_user_max_inflight: body.per_user_max_inflight,
            task_timeout_secs: body.task_timeout_secs,
            is_active: body.is_active,
        },
    )
    .await?;

    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "compute_pool_updated",
        Some(serde_json::json!({
            "pool_id": &pool.id,
            "slug": &pool.slug,
            "is_active": pool.is_active,
            "visibility": pool.visibility.as_str(),
        })),
    );

    Ok(Json(pool_info(&pool, true)))
}

pub async fn rotate_token(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(id_or_slug): Path<String>,
) -> AppResult<Json<RotateComputePoolTokenResponse>> {
    let actor = auth_user.user_id.to_string();
    let (pool, worker_token) =
        compute_pool_service::rotate_worker_token(&state.db, &actor, &id_or_slug).await?;

    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "compute_pool_token_rotated",
        Some(serde_json::json!({
            "pool_id": &pool.id,
            "slug": &pool.slug,
        })),
    );

    Ok(Json(RotateComputePoolTokenResponse {
        id: pool.id,
        slug: pool.slug,
        worker_token,
    }))
}

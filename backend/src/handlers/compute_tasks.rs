//! Compute consumer endpoints: submit tasks, poll results, cancel, status.

use axum::{
    Json,
    extract::{Path, State},
};
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::errors::AppResult;
use crate::models::compute_task::ComputeTask;
use crate::mw::auth::AuthUser;
use crate::services::{audit_service, compute_pool_service, compute_task_service};

#[derive(Deserialize)]
pub struct SubmitComputeTaskRequest {
    pub kind: Option<String>,
    pub model: String,
    #[serde(default)]
    pub input: serde_json::Value,
    #[serde(default)]
    pub priority: i32,
    #[serde(default)]
    pub client_ref: Option<String>,
}

#[derive(Serialize)]
pub struct SubmitComputeTaskResponse {
    pub task_id: String,
    pub pool_id: String,
    pub status: String,
    pub queue_position: u64,
    pub deduplicated: bool,
}

#[derive(Serialize)]
pub struct ComputeTaskInfo {
    pub task_id: String,
    pub pool_id: String,
    pub submitter_user_id: String,
    pub api_key_id: Option<String>,
    pub api_key_name: Option<String>,
    pub kind: String,
    pub model: String,
    pub priority: i32,
    pub status: String,
    pub phase: Option<String>,
    pub phase_detail: Option<String>,
    pub assigned_worker_id: Option<String>,
    pub queue_position: u64,
    pub output: Option<serde_json::Value>,
    pub failure_reason: Option<String>,
    pub created_at: String,
    pub updated_at: String,
    pub completed_at: Option<String>,
}

#[derive(Serialize)]
pub struct ComputeWorkerInfo {
    pub worker_label: String,
    pub node_id: Option<String>,
    pub host_kind: Option<String>,
    pub gpu_name: Option<String>,
    pub backend: Option<String>,
    pub models: Vec<String>,
    pub vram_total_mb: Option<u64>,
    pub vram_free_mb: Option<u64>,
    pub max_concurrency: u32,
    pub current_inflight: u32,
    pub avg_tokens_per_sec: Option<f64>,
    pub current_task_id: Option<String>,
    pub worker_version: Option<String>,
    pub last_seen_at: String,
}

#[derive(Serialize)]
pub struct ComputePoolStatusResponse {
    pub pool_id: String,
    pub slug: String,
    pub queued: u64,
    pub dispatched: u64,
    pub max_workers: u32,
    pub active_workers: Vec<ComputeWorkerInfo>,
}

fn submitter_identity(auth_user: &AuthUser) -> compute_task_service::SubmitterIdentity {
    compute_task_service::SubmitterIdentity {
        user_id: auth_user.user_id.to_string(),
        api_key_id: auth_user.api_key_id.clone(),
        api_key_name: auth_user.api_key_name.clone(),
    }
}

fn task_info(task: &ComputeTask, queue_position: u64) -> ComputeTaskInfo {
    ComputeTaskInfo {
        task_id: task.id.clone(),
        pool_id: task.pool_id.clone(),
        submitter_user_id: task.submitter_user_id.clone(),
        api_key_id: task.api_key_id.clone(),
        api_key_name: task.api_key_name.clone(),
        kind: task.kind.clone(),
        model: task.model.clone(),
        priority: task.priority,
        status: task.status.as_str().to_string(),
        phase: task.phase.clone(),
        phase_detail: task.phase_detail.clone(),
        assigned_worker_id: task.assigned_worker_id.clone(),
        queue_position,
        output: task.output.clone(),
        failure_reason: task.failure_reason.clone(),
        created_at: task.created_at.to_rfc3339(),
        updated_at: task.updated_at.to_rfc3339(),
        completed_at: task.completed_at.map(|t| t.to_rfc3339()),
    }
}

fn pool_status_response(status: compute_task_service::PoolStatus) -> ComputePoolStatusResponse {
    ComputePoolStatusResponse {
        pool_id: status.pool_id,
        slug: status.slug,
        queued: status.queued,
        dispatched: status.dispatched,
        max_workers: status.max_workers,
        active_workers: status
            .active_workers
            .into_iter()
            .map(|worker| ComputeWorkerInfo {
                worker_label: worker.worker_label,
                node_id: worker.node_id,
                host_kind: worker.host_kind,
                gpu_name: worker.gpu_name,
                backend: worker.backend,
                models: worker.models,
                vram_total_mb: worker.vram_total_mb,
                vram_free_mb: worker.vram_free_mb,
                max_concurrency: worker.max_concurrency,
                current_inflight: worker.current_inflight,
                avg_tokens_per_sec: worker.avg_tokens_per_sec,
                current_task_id: worker.current_task_id,
                worker_version: worker.worker_version,
                last_seen_at: worker.last_seen_at.to_rfc3339(),
            })
            .collect(),
    }
}

pub async fn submit_task(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(pool_id_or_slug): Path<String>,
    Json(body): Json<SubmitComputeTaskRequest>,
) -> AppResult<Json<SubmitComputeTaskResponse>> {
    let actor = auth_user.user_id.to_string();
    let pool = compute_pool_service::get_pool(&state.db, &pool_id_or_slug).await?;
    compute_pool_service::ensure_can_submit(&state.db, &actor, &pool).await?;

    let outcome = compute_task_service::submit_task(
        &state.db,
        &pool,
        &submitter_identity(&auth_user),
        compute_task_service::SubmitComputeTaskInput {
            kind: body.kind.unwrap_or_else(|| "chat_completion".to_string()),
            model: body.model,
            input: body.input,
            priority: body.priority,
            client_ref: body.client_ref,
        },
    )
    .await?;

    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "compute_task_submitted",
        Some(serde_json::json!({
            "task_id": &outcome.task.id,
            "pool_id": &pool.id,
            "pool_slug": &pool.slug,
            "model": &outcome.task.model,
            "kind": &outcome.task.kind,
            "deduplicated": outcome.deduplicated,
        })),
    );

    Ok(Json(SubmitComputeTaskResponse {
        task_id: outcome.task.id,
        pool_id: pool.id,
        status: "queued".to_string(),
        queue_position: outcome.queue_position,
        deduplicated: outcome.deduplicated,
    }))
}

pub async fn get_task(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(task_id): Path<String>,
) -> AppResult<Json<ComputeTaskInfo>> {
    let actor = auth_user.user_id.to_string();
    let (task, queue_position) =
        compute_task_service::get_task_for_consumer(&state.db, &actor, &task_id).await?;
    Ok(Json(task_info(&task, queue_position)))
}

pub async fn cancel_task(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(task_id): Path<String>,
) -> AppResult<Json<ComputeTaskInfo>> {
    let actor = auth_user.user_id.to_string();
    let task = compute_task_service::cancel_task(
        &state.db,
        &actor,
        &task_id,
        state.config.compute_task_retention_days,
    )
    .await?;

    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "compute_task_cancelled",
        Some(serde_json::json!({
            "task_id": &task.id,
            "pool_id": &task.pool_id,
        })),
    );

    Ok(Json(task_info(&task, 0)))
}

pub async fn pool_status(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(pool_id_or_slug): Path<String>,
) -> AppResult<Json<ComputePoolStatusResponse>> {
    let actor = auth_user.user_id.to_string();
    let pool = compute_pool_service::get_pool(&state.db, &pool_id_or_slug).await?;
    compute_pool_service::ensure_can_view(&state.db, &actor, &pool).await?;
    let status = compute_task_service::pool_status(&state.db, &pool).await?;
    Ok(Json(pool_status_response(status)))
}

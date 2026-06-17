//! Compute worker endpoints (`/api/v1/compute/worker/*`).
//!
//! Worker calls authenticate with a pool worker token (`nyx_cwk_...`) in
//! the handler, outside the user JWT middleware. Workers pull bounded
//! tasks and report results; NyxID does not run arbitrary SSH commands.

use axum::{
    Json,
    extract::{Query, State},
    http::{HeaderMap, header},
};
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::models::compute_pool::ComputePool;
use crate::services::{compute_pool_service, compute_task_service};

async fn authenticate_worker(state: &AppState, headers: &HeaderMap) -> AppResult<ComputePool> {
    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .ok_or(AppError::ComputeWorkerTokenInvalid)?;
    compute_pool_service::validate_worker_token(&state.db, token).await
}

#[derive(Deserialize)]
pub struct PollTaskQuery {
    pub worker: String,
}

#[derive(Deserialize)]
pub struct PollTaskBody {
    #[serde(default)]
    pub capabilities: compute_task_service::WorkerCapabilities,
}

#[derive(Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum PollTaskResponse {
    Idle,
    Task {
        #[serde(flatten)]
        task: compute_task_service::WorkerTaskPayload,
    },
}

pub async fn poll_task(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<PollTaskQuery>,
    Json(body): Json<PollTaskBody>,
) -> AppResult<Json<PollTaskResponse>> {
    let pool = authenticate_worker(&state, &headers).await?;
    let claimed =
        compute_task_service::claim_task(&state.db, &pool, &query.worker, body.capabilities)
            .await?;
    Ok(Json(match claimed {
        Some(task) => PollTaskResponse::Task { task },
        None => PollTaskResponse::Idle,
    }))
}

#[derive(Deserialize)]
pub struct WorkerAckRequest {
    pub task_id: String,
    pub worker: String,
    #[serde(default)]
    pub phase: Option<String>,
    #[serde(default)]
    pub phase_detail: Option<String>,
    #[serde(default)]
    pub capabilities: Option<compute_task_service::WorkerCapabilities>,
}

#[derive(Serialize)]
pub struct WorkerAckResponse {
    pub status: String,
}

pub async fn ack(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<WorkerAckRequest>,
) -> AppResult<Json<WorkerAckResponse>> {
    let pool = authenticate_worker(&state, &headers).await?;
    let outcome = compute_task_service::worker_ack(
        &state.db,
        &pool,
        &body.worker,
        &body.task_id,
        body.phase.as_deref(),
        body.phase_detail.as_deref(),
        body.capabilities,
    )
    .await?;
    Ok(Json(WorkerAckResponse {
        status: match outcome {
            compute_task_service::AckOutcome::Ok => "ok".to_string(),
            compute_task_service::AckOutcome::Cancelled => "cancelled".to_string(),
        },
    }))
}

#[derive(Deserialize)]
pub struct WorkerResultRequest {
    pub task_id: String,
    pub worker: String,
    #[serde(default)]
    pub output: Option<serde_json::Value>,
    #[serde(default)]
    pub failure_reason: Option<String>,
}

#[derive(Serialize)]
pub struct WorkerResultResponse {
    pub status: String,
}

pub async fn submit_result(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<WorkerResultRequest>,
) -> AppResult<Json<WorkerResultResponse>> {
    let pool = authenticate_worker(&state, &headers).await?;
    let outcome = compute_task_service::worker_submit_result(
        &state.db,
        &pool,
        &body.worker,
        &body.task_id,
        body.output,
        body.failure_reason.as_deref(),
        state.config.compute_task_retention_days,
    )
    .await?;

    tracing::info!(
        task_id = %body.task_id,
        pool_id = %pool.id,
        outcome = ?outcome,
        "Compute worker result received"
    );

    Ok(Json(WorkerResultResponse {
        status: match outcome {
            compute_task_service::ResultOutcome::Completed => "saved".to_string(),
            compute_task_service::ResultOutcome::Failed => "saved_failed".to_string(),
            compute_task_service::ResultOutcome::Ignored => "ignored".to_string(),
        },
    }))
}

//! Oracle worker endpoints (`/api/v1/oracle/worker/*`).
//!
//! Mounted OUTSIDE the JWT auth middleware (like `/api/v1/node-agent`):
//! every request authenticates with the pool worker token in the
//! `Authorization: Bearer nyx_owk_...` header. The wire format mirrors
//! the local oracle servers (`/task`, `/ack`, `/result`, `/pin-conv-url`)
//! so the userscript port is a thin diff: same field names, plus the
//! auth header.
//!
//! Responses never include other submitters' data beyond the claimed
//! task itself; prompts/responses are never logged here.

use axum::{
    Json,
    extract::{Query, State},
    http::{HeaderMap, header},
    response::IntoResponse,
};
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::models::oracle_pool::OraclePool;
use crate::services::{oracle_pool_service, oracle_task_service};

async fn authenticate_worker(state: &AppState, headers: &HeaderMap) -> AppResult<OraclePool> {
    let token = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .ok_or(AppError::OracleWorkerTokenInvalid)?;
    oracle_pool_service::validate_worker_token(&state.db, token).await
}

#[derive(Deserialize)]
pub struct PollTaskQuery {
    /// Tab-chosen worker label (e.g. "tab_1"), unique per tab.
    pub worker: String,
    #[serde(default)]
    pub script_version: Option<String>,
    #[serde(default)]
    pub page_url: Option<String>,
}

#[derive(Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
// The `Task` variant carries the full task payload and dwarfs `Idle`; this
// enum is constructed at most once per poll and serialized straight to the
// response, so the size delta is not load-bearing. Boxing would only add an
// allocation on the hot path.
#[allow(clippy::large_enum_variant)]
pub enum PollTaskResponse {
    Idle {
        #[serde(skip_serializing_if = "Option::is_none")]
        required_project_url: Option<String>,
    },
    Task {
        #[serde(flatten)]
        task: oracle_task_service::WorkerTaskPayload,
    },
}

pub async fn poll_task(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<PollTaskQuery>,
) -> AppResult<Json<PollTaskResponse>> {
    let pool = authenticate_worker(&state, &headers).await?;
    let claimed = oracle_task_service::claim_task(
        &state.db,
        &pool,
        &query.worker,
        query.script_version.as_deref(),
        query.page_url.as_deref(),
    )
    .await?;
    Ok(Json(match claimed {
        Some(task) => PollTaskResponse::Task { task },
        None => PollTaskResponse::Idle {
            required_project_url: pool.chatgpt_project_url.clone(),
        },
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
    pub script_version: Option<String>,
    #[serde(default)]
    pub page_url: Option<String>,
}

#[derive(Serialize)]
pub struct WorkerAckResponse {
    /// "ok" — keep going; "cancelled" — abandon the task and re-poll.
    pub status: String,
}

pub async fn ack(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<WorkerAckRequest>,
) -> AppResult<Json<WorkerAckResponse>> {
    let pool = authenticate_worker(&state, &headers).await?;
    let outcome = oracle_task_service::worker_ack(
        &state.db,
        &pool,
        &body.worker,
        &body.task_id,
        body.phase.as_deref(),
        body.phase_detail.as_deref(),
        body.script_version.as_deref(),
        body.page_url.as_deref(),
    )
    .await?;
    Ok(Json(WorkerAckResponse {
        status: match outcome {
            oracle_task_service::AckOutcome::Ok => "ok".to_string(),
            oracle_task_service::AckOutcome::Cancelled => "cancelled".to_string(),
        },
    }))
}

#[derive(Deserialize)]
pub struct WorkerResultRequest {
    pub task_id: String,
    pub worker: String,
    pub response: String,
    #[serde(default)]
    pub chatgpt_url: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub script_version: Option<String>,
}

#[derive(Serialize)]
pub struct WorkerResultResponse {
    /// "saved" | "saved_failed" | "ignored"
    pub status: String,
}

pub async fn submit_result(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<WorkerResultRequest>,
) -> AppResult<Json<WorkerResultResponse>> {
    let pool = authenticate_worker(&state, &headers).await?;
    let outcome = oracle_task_service::worker_submit_result(
        &state.db,
        &pool,
        &body.worker,
        &body.task_id,
        &body.response,
        body.chatgpt_url.as_deref(),
        body.model.as_deref(),
        body.script_version.as_deref(),
        state.config.oracle_task_retention_days,
    )
    .await?;

    // Metadata-only trace: task id + outcome + size, never the body.
    tracing::info!(
        task_id = %body.task_id,
        pool_id = %pool.id,
        outcome = ?outcome,
        response_chars = body.response.chars().count(),
        "Oracle worker result received"
    );

    Ok(Json(WorkerResultResponse {
        status: match outcome {
            oracle_task_service::ResultOutcome::Completed => "saved".to_string(),
            oracle_task_service::ResultOutcome::Failed => "saved_failed".to_string(),
            oracle_task_service::ResultOutcome::Ignored => "ignored".to_string(),
        },
    }))
}

#[derive(Deserialize)]
pub struct TranscriptTurnDto {
    pub role: String,
    pub text: String,
}

#[derive(Deserialize)]
pub struct WorkerTranscriptRequest {
    pub task_id: String,
    pub worker: String,
    pub turns: Vec<TranscriptTurnDto>,
    #[serde(default)]
    pub chatgpt_url: Option<String>,
}

#[derive(Serialize)]
pub struct WorkerTranscriptResponse {
    /// "imported" | "ignored"
    pub status: String,
    pub imported_pairs: usize,
}

pub async fn submit_transcript(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<WorkerTranscriptRequest>,
) -> AppResult<Json<WorkerTranscriptResponse>> {
    let pool = authenticate_worker(&state, &headers).await?;
    let turns: Vec<oracle_task_service::TranscriptTurn> = body
        .turns
        .into_iter()
        .map(|turn| oracle_task_service::TranscriptTurn {
            role: turn.role,
            text: turn.text,
        })
        .collect();
    let outcome = oracle_task_service::worker_submit_transcript(
        &state.db,
        &pool,
        &body.worker,
        &body.task_id,
        &turns,
        body.chatgpt_url.as_deref(),
        state.config.oracle_task_retention_days,
    )
    .await?;

    let (status, imported_pairs) = match outcome {
        oracle_task_service::TranscriptOutcome::Imported { pairs } => ("imported", pairs),
        oracle_task_service::TranscriptOutcome::Ignored => ("ignored", 0),
    };
    tracing::info!(
        task_id = %body.task_id,
        pool_id = %pool.id,
        imported_pairs,
        "Oracle worker transcript received"
    );

    Ok(Json(WorkerTranscriptResponse {
        status: status.to_string(),
        imported_pairs,
    }))
}

#[derive(Deserialize)]
pub struct PinConvUrlRequest {
    pub task_id: String,
    pub worker: String,
    pub chatgpt_url: String,
}

pub async fn pin_conv_url(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<PinConvUrlRequest>,
) -> AppResult<impl IntoResponse> {
    let pool = authenticate_worker(&state, &headers).await?;
    oracle_task_service::pin_conversation_url(
        &state.db,
        &pool,
        &body.worker,
        &body.task_id,
        &body.chatgpt_url,
    )
    .await?;
    Ok(Json(serde_json::json!({ "status": "pinned" })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn poll_response_wire_format() {
        // The userscript switches on `status`; idle carries the project
        // pin so a drifted tab can navigate home even with an empty queue.
        let idle = serde_json::to_value(PollTaskResponse::Idle {
            required_project_url: Some("https://chatgpt.com/g/g-p-x/project".to_string()),
        })
        .unwrap();
        assert_eq!(idle["status"], "idle");
        assert_eq!(
            idle["required_project_url"],
            "https://chatgpt.com/g/g-p-x/project"
        );

        let task = PollTaskResponse::Task {
            task: oracle_task_service::WorkerTaskPayload {
                task_id: "t1".to_string(),
                kind: "prompt".to_string(),
                prompt: "p".to_string(),
                target_url: None,
                conversation_id: Some("conv_1".to_string()),
                conversation_url: None,
                is_followup: false,
                model: Some("chatgpt-5.5-pro".to_string()),
                tag: None,
                pdf_base64: None,
                pdf_name: None,
                required_project_url: None,
                assigned_worker: "tab_1".to_string(),
                submitted_at: "2026-06-11T00:00:00Z".to_string(),
            },
        };
        let task = serde_json::to_value(task).unwrap();
        assert_eq!(task["status"], "task");
        assert_eq!(task["task_id"], "t1");
        assert_eq!(task["assigned_worker"], "tab_1");
        assert_eq!(task["is_followup"], false);
    }
}

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
    extract::{Path, Query, State},
    http::{HeaderMap, header},
    response::IntoResponse,
};
use base64::Engine;
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::models::oracle_pool::OraclePool;
use crate::models::oracle_worker_command::OracleWorkerCommand;
use crate::services::{
    oracle_login_snapshot_service, oracle_pool_service, oracle_task_service, oracle_worker_service,
};

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
    #[serde(default)]
    pub instance_id: Option<String>,
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
    oracle_worker_service::ensure_instance_matches(
        &state.db,
        &pool,
        &query.worker,
        query.instance_id.as_deref(),
    )
    .await?;
    let claimed = oracle_task_service::claim_task_with_retention(
        &state.db,
        &pool,
        &query.worker,
        query.script_version.as_deref(),
        query.page_url.as_deref(),
        state.config.oracle_task_retention_days,
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
    #[serde(default)]
    pub dispatch_attempt_id: Option<String>,
    #[serde(default)]
    pub instance_id: Option<String>,
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
    oracle_worker_service::ensure_instance_matches(
        &state.db,
        &pool,
        &body.worker,
        body.instance_id.as_deref(),
    )
    .await?;
    let outcome = oracle_task_service::worker_ack_fenced(
        &state.db,
        &pool,
        &body.worker,
        &body.task_id,
        oracle_task_service::WorkerAckInput {
            phase: body.phase.as_deref(),
            phase_detail: body.phase_detail.as_deref(),
            script_version: body.script_version.as_deref(),
            page_url: body.page_url.as_deref(),
            dispatch_attempt_id: body.dispatch_attempt_id.as_deref(),
        },
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
pub struct WorkerImageDto {
    pub mime: String,
    pub data_base64: String,
    #[serde(default)]
    pub name: Option<String>,
}

#[derive(Deserialize)]
pub struct WorkerFileDto {
    pub name: String,
    pub mime: String,
    pub data_base64: String,
}

#[derive(Deserialize)]
pub struct WorkerResultRequest {
    pub task_id: String,
    pub worker: String,
    pub response: String,
    /// Images produced on an image-generation turn (base64). Optional and
    /// validated/capped server-side; an image-only turn sends an empty
    /// `response` and a non-empty `images` list.
    #[serde(default)]
    pub images: Option<Vec<WorkerImageDto>>,
    /// Generic files linked from the assistant's final turn. Optional so old
    /// workers and the deployed userscript retain their existing wire shape.
    #[serde(default)]
    pub files: Option<Vec<WorkerFileDto>>,
    #[serde(default)]
    pub chatgpt_url: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub script_version: Option<String>,
    #[serde(default)]
    pub dispatch_attempt_id: Option<String>,
    #[serde(default)]
    pub instance_id: Option<String>,
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
    oracle_worker_service::ensure_instance_matches(
        &state.db,
        &pool,
        &body.worker,
        body.instance_id.as_deref(),
    )
    .await?;
    let req_images = body.images.unwrap_or_default();
    let image_count = req_images.len();
    let image_base64_chars: usize = req_images.iter().map(|i| i.data_base64.len()).sum();
    let images: Vec<oracle_task_service::ResultImage> = req_images
        .into_iter()
        .map(|i| oracle_task_service::ResultImage {
            mime: i.mime,
            data_base64: i.data_base64,
            name: i.name,
        })
        .collect();
    let req_files = body.files.unwrap_or_default();
    let file_count = req_files.len();
    let file_base64_chars: usize = req_files.iter().map(|file| file.data_base64.len()).sum();
    let files = req_files
        .into_iter()
        .map(|file| oracle_task_service::ResultFile {
            name: file.name,
            mime: file.mime,
            data_base64: file.data_base64,
        })
        .collect();
    let outcome = oracle_task_service::worker_submit_result_fenced(
        &state.db,
        &pool,
        &body.worker,
        &body.task_id,
        oracle_task_service::WorkerResultInput {
            response: &body.response,
            images,
            files,
            chatgpt_url: body.chatgpt_url.as_deref(),
            model: body.model.as_deref(),
            script_version: body.script_version.as_deref(),
            retention_days: state.config.oracle_task_retention_days,
            dispatch_attempt_id: body.dispatch_attempt_id.as_deref(),
        },
    )
    .await?;

    // Metadata-only trace: task id + outcome + sizes, never the body or bytes.
    tracing::info!(
        task_id = %body.task_id,
        pool_id = %pool.id,
        outcome = ?outcome,
        response_chars = body.response.chars().count(),
        image_count,
        image_base64_chars,
        file_count,
        file_base64_chars,
        "Oracle worker result received"
    );

    Ok(Json(WorkerResultResponse {
        status: match outcome {
            oracle_task_service::ResultOutcome::Completed => "saved".to_string(),
            oracle_task_service::ResultOutcome::Failed => "saved_failed".to_string(),
            oracle_task_service::ResultOutcome::Requeued => "requeued".to_string(),
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
    #[serde(default)]
    pub instance_id: Option<String>,
    #[serde(default)]
    pub dispatch_attempt_id: Option<String>,
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
    oracle_worker_service::ensure_instance_matches(
        &state.db,
        &pool,
        &body.worker,
        body.instance_id.as_deref(),
    )
    .await?;
    let turns: Vec<oracle_task_service::TranscriptTurn> = body
        .turns
        .into_iter()
        .map(|turn| oracle_task_service::TranscriptTurn {
            role: turn.role,
            text: turn.text,
        })
        .collect();
    let outcome = oracle_task_service::worker_submit_transcript_fenced(
        &state.db,
        &pool,
        &body.worker,
        &body.task_id,
        oracle_task_service::WorkerTranscriptInput {
            turns: &turns,
            chatgpt_url: body.chatgpt_url.as_deref(),
            retention_days: state.config.oracle_task_retention_days,
            dispatch_attempt_id: body.dispatch_attempt_id.as_deref(),
        },
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
    #[serde(default)]
    pub instance_id: Option<String>,
    #[serde(default)]
    pub dispatch_attempt_id: Option<String>,
}

pub async fn pin_conv_url(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<PinConvUrlRequest>,
) -> AppResult<impl IntoResponse> {
    let pool = authenticate_worker(&state, &headers).await?;
    oracle_worker_service::ensure_instance_matches(
        &state.db,
        &pool,
        &body.worker,
        body.instance_id.as_deref(),
    )
    .await?;
    oracle_task_service::pin_conversation_url_fenced(
        &state.db,
        &pool,
        &body.worker,
        &body.task_id,
        &body.chatgpt_url,
        body.dispatch_attempt_id.as_deref(),
    )
    .await?;
    Ok(Json(serde_json::json!({ "status": "pinned" })))
}

#[derive(Deserialize)]
pub struct WorkerCommandReportDto {
    pub command_id: String,
    pub succeeded: bool,
    #[serde(default)]
    pub result_code: Option<String>,
}

#[derive(Deserialize)]
pub struct WorkerHeartbeatRequest {
    pub worker: String,
    pub instance_id: String,
    #[serde(default)]
    pub script_version: Option<String>,
    #[serde(default)]
    pub platform: Option<String>,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub logged_in: Option<bool>,
    #[serde(default)]
    pub current_task_id: Option<String>,
    #[serde(default)]
    pub chrome_alive: Option<bool>,
    #[serde(default)]
    pub last_error: Option<String>,
    #[serde(default)]
    pub command_reports: Vec<WorkerCommandReportDto>,
}

#[derive(Serialize)]
pub struct WorkerCommandDto {
    pub id: String,
    pub command: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bundle_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bundle_sha256: Option<String>,
    pub deadline_at: String,
}

impl From<OracleWorkerCommand> for WorkerCommandDto {
    fn from(command: OracleWorkerCommand) -> Self {
        Self {
            id: command.id,
            command: command.kind.as_str().to_string(),
            snapshot_id: command.snapshot_id,
            bundle_version: command.bundle_version,
            bundle_sha256: command.bundle_sha256,
            deadline_at: command.deadline_at.to_rfc3339(),
        }
    }
}

#[derive(Serialize)]
pub struct WorkerHeartbeatResponse {
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<WorkerCommandDto>,
    /// Delivered commands a manager withdrew; a worker still holding one
    /// drops it. Absent for legacy workers, which never receive commands.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cancelled_command_ids: Vec<String>,
}

pub async fn heartbeat(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<WorkerHeartbeatRequest>,
) -> AppResult<Json<WorkerHeartbeatResponse>> {
    let pool = authenticate_worker(&state, &headers).await?;
    let capabilities = body.capabilities.clone();
    oracle_worker_service::report_presence(
        &state.db,
        &pool,
        oracle_worker_service::WorkerPresenceInput {
            worker_label: body.worker.clone(),
            current_task_id: body.current_task_id,
            script_version: body.script_version,
            instance_id: Some(body.instance_id),
            platform: body.platform,
            capabilities: capabilities.clone(),
            logged_in: body.logged_in,
            chrome_alive: body.chrome_alive,
            last_error: body.last_error,
        },
    )
    .await?;
    oracle_worker_service::apply_command_reports(
        &state.db,
        &pool.id,
        &body.worker,
        body.command_reports
            .into_iter()
            .map(|report| oracle_worker_service::CommandReport {
                command_id: report.command_id,
                succeeded: report.succeeded,
                result_code: report.result_code,
            })
            .collect(),
    )
    .await?;
    let command = oracle_worker_service::deliver_next_command(
        &state.db,
        &pool.id,
        &body.worker,
        &capabilities,
    )
    .await?
    .map(WorkerCommandDto::from);
    let cancelled_command_ids = if capabilities.iter().any(|value| value == "commands_v1") {
        oracle_worker_service::recently_cancelled_delivered(&state.db, &pool.id, &body.worker)
            .await?
    } else {
        Vec::new()
    };
    Ok(Json(WorkerHeartbeatResponse {
        status: "ok".to_string(),
        command,
        cancelled_command_ids,
    }))
}

#[derive(Serialize)]
pub struct WorkerLoginSnapshotResponse {
    pub format_version: u32,
    pub sealed_blob_base64: String,
}

pub async fn fetch_login_snapshot(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(snapshot_id): Path<String>,
) -> AppResult<Json<WorkerLoginSnapshotResponse>> {
    let pool = authenticate_worker(&state, &headers).await?;
    let payload = oracle_login_snapshot_service::fetch_for_worker(
        &state.db,
        &state.encryption_keys,
        &pool.id,
        &snapshot_id,
    )
    .await?;
    Ok(Json(WorkerLoginSnapshotResponse {
        format_version: payload.format_version,
        sealed_blob_base64: base64::engine::general_purpose::STANDARD
            .encode(payload.sealed_envelope.as_slice()),
    }))
}

pub async fn fetch_bundle(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> AppResult<Json<super::oracle_worker_bundle::OracleWorkerBundleResponse>> {
    authenticate_worker(&state, &headers).await?;
    Ok(Json(super::oracle_worker_bundle::bundle_response()))
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
                attempts: 1,
                retry_count: 0,
                max_retries: 3,
                dispatch_attempt_id: Some("attempt-1".to_string()),
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
                attachment_base64: None,
                attachment_name: None,
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

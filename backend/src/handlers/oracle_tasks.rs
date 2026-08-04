//! Oracle consumer endpoints: submit tasks, poll results, manage
//! sessions (`/api/v1/oracle/{pools/{slug}/tasks, tasks, sessions}`).
//!
//! The polling contract mirrors the local oracle servers these replace:
//! `POST .../tasks` returns immediately with a task id; consumers poll
//! `GET /oracle/tasks/{id}` (cheap, seconds-scale) until a terminal
//! status. Long waits live in the polling loop, never in a request.

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::errors::AppResult;
use crate::models::oracle_session::OracleSession;
use crate::models::oracle_task::OracleTask;
use crate::mw::auth::AuthUser;
use crate::services::{
    audit_service, oracle_pool_service, oracle_session_service, oracle_task_service,
};

#[derive(Deserialize)]
pub struct SubmitOracleTaskRequest {
    pub prompt: String,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub project_url: Option<String>,
    #[serde(default)]
    pub tag: Option<String>,
    /// Omitted = single-shot. `""` = open a new session. An existing id =
    /// continue that session (owner only).
    #[serde(default)]
    pub conversation_id: Option<String>,
    #[serde(default)]
    pub pdf_base64: Option<String>,
    #[serde(default)]
    pub pdf_name: Option<String>,
    /// Optional submitter-scoped idempotency key: retried submits with
    /// the same value return the original task.
    #[serde(default)]
    pub client_ref: Option<String>,
}

#[derive(Serialize)]
pub struct SubmitOracleTaskResponse {
    pub task_id: String,
    pub status: String,
    pub queue_position: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
    pub deduplicated: bool,
}

#[derive(Deserialize)]
pub struct AttachConversationRequest {
    pub chatgpt_url: String,
    #[serde(default)]
    pub tag: Option<String>,
}

#[derive(Serialize)]
pub struct AttachConversationResponse {
    pub conversation_id: String,
    pub task_id: String,
    pub status: String,
}

#[derive(Deserialize)]
pub struct ExtractRequest {
    pub url: String,
    #[serde(default)]
    pub model: Option<String>,
}

#[derive(Serialize)]
pub struct ExtractResponse {
    pub task_id: String,
    pub status: String,
}

#[derive(Serialize)]
pub struct OracleTaskInfo {
    pub task_id: String,
    pub pool_id: String,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phase: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phase_detail: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub conversation_id: Option<String>,
    pub is_followup: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    /// 1-based position while queued; 0 otherwise.
    pub queue_position: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub assigned_worker: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response_chars: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chatgpt_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub failure_reason: Option<String>,
    pub created_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dispatched_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
}

/// Transcript turns include the prompt; the poll response does not (the
/// submitter already has it, and prompts can be hundreds of KB).
#[derive(Serialize)]
pub struct OracleTurnInfo {
    pub task_id: String,
    pub status: String,
    pub prompt: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub response: Option<String>,
    pub created_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
}

#[derive(Serialize)]
pub struct OracleSessionInfo {
    pub conversation_id: String,
    pub pool_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chatgpt_url: Option<String>,
    pub turn_count: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_task_id: Option<String>,
    pub closed: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Serialize)]
pub struct OracleSessionDetailResponse {
    #[serde(flatten)]
    pub session: OracleSessionInfo,
    pub turns: Vec<OracleTurnInfo>,
}

#[derive(Serialize)]
pub struct ListOracleSessionsResponse {
    pub sessions: Vec<OracleSessionInfo>,
}

#[derive(Deserialize)]
pub struct ListSessionsQuery {
    #[serde(default)]
    pub pool: Option<String>,
    #[serde(default)]
    pub limit: Option<i64>,
}

fn task_info(task: &OracleTask, queue_position: u64) -> OracleTaskInfo {
    OracleTaskInfo {
        task_id: task.id.clone(),
        pool_id: task.pool_id.clone(),
        status: task.status.as_str().to_string(),
        phase: task.phase.clone(),
        phase_detail: task.phase_detail.clone(),
        conversation_id: task.conversation_id.clone(),
        is_followup: task.is_followup,
        model: task.model_label.clone(),
        tag: task.tag.clone(),
        queue_position,
        assigned_worker: task.assigned_worker_id.clone(),
        response: task.response.clone(),
        response_chars: task.response_chars,
        chatgpt_url: task.chatgpt_url.clone(),
        failure_reason: task.failure_reason.clone(),
        created_at: task.created_at.to_rfc3339(),
        dispatched_at: task.dispatched_at.map(|t| t.to_rfc3339()),
        completed_at: task.completed_at.map(|t| t.to_rfc3339()),
    }
}

fn session_info(session: &OracleSession) -> OracleSessionInfo {
    OracleSessionInfo {
        conversation_id: session.id.clone(),
        pool_id: session.pool_id.clone(),
        tag: session.tag.clone(),
        chatgpt_url: session.chatgpt_url.clone(),
        turn_count: session.turn_count,
        last_task_id: session.last_task_id.clone(),
        closed: session.closed_at.is_some(),
        created_at: session.created_at.to_rfc3339(),
        updated_at: session.updated_at.to_rfc3339(),
    }
}

fn submitter_identity(auth_user: &AuthUser) -> oracle_task_service::SubmitterIdentity {
    oracle_task_service::SubmitterIdentity {
        user_id: auth_user.user_id.to_string(),
        api_key_id: auth_user.api_key_id.clone(),
        api_key_name: auth_user.api_key_name.clone(),
    }
}

pub async fn submit_task(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(pool_id_or_slug): Path<String>,
    Json(body): Json<SubmitOracleTaskRequest>,
) -> AppResult<impl IntoResponse> {
    let actor = auth_user.user_id.to_string();
    let pool = oracle_pool_service::get_pool(&state.db, &pool_id_or_slug).await?;
    oracle_pool_service::ensure_can_submit(&state.db, &actor, &pool).await?;

    let outcome = oracle_task_service::submit_task(
        &state.db,
        &pool,
        &submitter_identity(&auth_user),
        oracle_task_service::SubmitTaskInput {
            prompt: body.prompt,
            model_label: body.model,
            project_url: body.project_url,
            tag: body.tag,
            conversation_id: body.conversation_id,
            pdf_base64: body.pdf_base64,
            pdf_name: body.pdf_name,
            client_ref: body.client_ref,
        },
    )
    .await?;

    // Metadata only — never the prompt body.
    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "oracle_task_submitted",
        Some(serde_json::json!({
            "task_id": &outcome.task.id,
            "pool_id": &pool.id,
            "pool_slug": &pool.slug,
            "conversation_id": &outcome.task.conversation_id,
            "is_followup": outcome.task.is_followup,
            "prompt_chars": outcome.task.prompt.chars().count(),
            "has_pdf": outcome.task.pdf_base64.is_some(),
            "deduplicated": outcome.deduplicated,
        })),
    );

    Ok((
        StatusCode::ACCEPTED,
        Json(SubmitOracleTaskResponse {
            task_id: outcome.task.id.clone(),
            status: outcome.task.status.as_str().to_string(),
            queue_position: outcome.queue_position,
            conversation_id: outcome.task.conversation_id.clone(),
            deduplicated: outcome.deduplicated,
        }),
    ))
}

pub async fn attach_conversation(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(pool_id_or_slug): Path<String>,
    Json(body): Json<AttachConversationRequest>,
) -> AppResult<impl IntoResponse> {
    let actor = auth_user.user_id.to_string();
    let pool = oracle_pool_service::get_pool(&state.db, &pool_id_or_slug).await?;
    oracle_pool_service::ensure_can_submit(&state.db, &actor, &pool).await?;

    let (session, task) = oracle_task_service::attach_conversation(
        &state.db,
        &pool,
        &submitter_identity(&auth_user),
        &body.chatgpt_url,
        body.tag,
    )
    .await?;

    // Metadata only — never transcript text.
    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "oracle_conversation_attached",
        Some(serde_json::json!({
            "conversation_id": &session.id,
            "pool_id": &pool.id,
            "pool_slug": &pool.slug,
        })),
    );

    Ok((
        StatusCode::ACCEPTED,
        Json(AttachConversationResponse {
            conversation_id: session.id,
            task_id: task.id.clone(),
            status: task.status.as_str().to_string(),
        }),
    ))
}

pub async fn extract_url(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(pool_id_or_slug): Path<String>,
    Json(body): Json<ExtractRequest>,
) -> AppResult<impl IntoResponse> {
    let actor = auth_user.user_id.to_string();
    let pool = oracle_pool_service::get_pool(&state.db, &pool_id_or_slug).await?;
    oracle_pool_service::ensure_can_submit(&state.db, &actor, &pool).await?;

    let host = url::Url::parse(&body.url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_string));
    let task = oracle_task_service::extract_url(
        &state.db,
        &pool,
        &submitter_identity(&auth_user),
        &body.url,
        body.model,
    )
    .await?;

    // Metadata only — never extracted content or full URL path/query.
    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "oracle_extract_submitted",
        Some(serde_json::json!({
            "task_id": &task.id,
            "pool_id": &pool.id,
            "pool_slug": &pool.slug,
            "url_host": host,
        })),
    );

    Ok((
        StatusCode::ACCEPTED,
        Json(ExtractResponse {
            task_id: task.id.clone(),
            status: task.status.as_str().to_string(),
        }),
    ))
}

pub async fn get_task(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(task_id): Path<String>,
) -> AppResult<Json<OracleTaskInfo>> {
    let actor = auth_user.user_id.to_string();
    let (task, position) =
        oracle_task_service::get_task_for_consumer(&state.db, &actor, &task_id).await?;
    Ok(Json(task_info(&task, position)))
}

pub async fn cancel_task(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(task_id): Path<String>,
) -> AppResult<Json<OracleTaskInfo>> {
    let actor = auth_user.user_id.to_string();
    let task = oracle_task_service::cancel_task(
        &state.db,
        &actor,
        &task_id,
        state.config.oracle_task_retention_days,
    )
    .await?;

    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "oracle_task_cancelled",
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
) -> AppResult<Json<oracle_task_service::PoolStatus>> {
    let actor = auth_user.user_id.to_string();
    let pool = oracle_pool_service::get_pool(&state.db, &pool_id_or_slug).await?;
    oracle_pool_service::ensure_can_view(&state.db, &actor, &pool).await?;
    let status = oracle_task_service::pool_status(&state.db, &pool).await?;
    Ok(Json(status))
}

pub async fn list_sessions(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Query(query): Query<ListSessionsQuery>,
) -> AppResult<Json<ListOracleSessionsResponse>> {
    let actor = auth_user.user_id.to_string();
    let pool_id = match query.pool.as_deref() {
        Some(id_or_slug) => Some(
            oracle_pool_service::get_pool(&state.db, id_or_slug)
                .await?
                .id,
        ),
        None => None,
    };
    let sessions = oracle_session_service::list_own_sessions(
        &state.db,
        &actor,
        pool_id.as_deref(),
        query.limit.unwrap_or(50),
    )
    .await?;
    Ok(Json(ListOracleSessionsResponse {
        sessions: sessions.iter().map(session_info).collect(),
    }))
}

pub async fn get_session(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(conversation_id): Path<String>,
) -> AppResult<Json<OracleSessionDetailResponse>> {
    let actor = auth_user.user_id.to_string();
    let (session, tasks) =
        oracle_session_service::list_session_tasks(&state.db, &actor, &conversation_id).await?;
    let turns = tasks
        .iter()
        .map(|t| OracleTurnInfo {
            task_id: t.id.clone(),
            status: t.status.as_str().to_string(),
            prompt: t.prompt.clone(),
            response: t.response.clone(),
            created_at: t.created_at.to_rfc3339(),
            completed_at: t.completed_at.map(|ts| ts.to_rfc3339()),
        })
        .collect();
    Ok(Json(OracleSessionDetailResponse {
        session: session_info(&session),
        turns,
    }))
}

pub async fn close_session(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(conversation_id): Path<String>,
) -> AppResult<Json<OracleSessionInfo>> {
    let actor = auth_user.user_id.to_string();
    let session =
        oracle_session_service::close_session(&state.db, &actor, &conversation_id).await?;

    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "oracle_session_closed",
        Some(serde_json::json!({
            "conversation_id": &session.id,
            "pool_id": &session.pool_id,
            "turn_count": session.turn_count,
        })),
    );

    Ok(Json(session_info(&session)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::oracle_task::OracleTaskStatus;
    use chrono::Utc;

    fn sample_task() -> OracleTask {
        OracleTask {
            id: "t1".to_string(),
            pool_id: "p1".to_string(),
            submitter_user_id: "u1".to_string(),
            kind: "prompt".to_string(),
            target_url: None,
            api_key_id: None,
            api_key_name: None,
            prompt: "the prompt".to_string(),
            model_label: Some("chatgpt-5.5-pro".to_string()),
            project_url: None,
            tag: None,
            pdf_base64: Some("cGRm".to_string()),
            pdf_name: Some("a.pdf".to_string()),
            conversation_id: Some("conv_1".to_string()),
            is_followup: false,
            client_ref: None,
            status: OracleTaskStatus::Queued,
            phase: None,
            phase_detail: None,
            phase_at: None,
            assigned_worker_id: None,
            dispatched_at: None,
            lease_expires_at: None,
            response: None,
            response_chars: None,
            chatgpt_url: None,
            failure_reason: None,
            worker_script_version: None,
            completed_at: None,
            expires_at: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn poll_response_omits_prompt_and_pdf() {
        // The poll response must stay small: no prompt echo, no pdf body,
        // no submitter identifiers.
        let json = serde_json::to_string(&task_info(&sample_task(), 1)).unwrap();
        assert!(!json.contains("the prompt"));
        assert!(!json.contains("cGRm"));
        assert!(!json.contains("submitter"));
        assert!(json.contains("\"queue_position\":1"));
        assert!(json.contains("\"status\":\"queued\""));
    }

    #[test]
    fn transcript_includes_prompt() {
        let task = sample_task();
        let turn = OracleTurnInfo {
            task_id: task.id.clone(),
            status: task.status.as_str().to_string(),
            prompt: task.prompt.clone(),
            response: Some("answer".to_string()),
            created_at: task.created_at.to_rfc3339(),
            completed_at: None,
        };
        let json = serde_json::to_string(&turn).unwrap();
        assert!(json.contains("the prompt"));
        assert!(json.contains("answer"));
    }
}

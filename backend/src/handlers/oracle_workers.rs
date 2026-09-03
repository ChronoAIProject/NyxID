use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use base64::Engine;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::models::oracle_pool::OraclePool;
use crate::models::oracle_worker::{OracleWorker, OracleWorkerDesiredState};
use crate::models::oracle_worker_command::{
    OracleWorkerCommand, OracleWorkerCommandKind, OracleWorkerCommandStatus,
};
use crate::mw::auth::AuthUser;
use crate::services::{
    audit_service, oracle_login_snapshot_service, oracle_pool_service,
    oracle_worker_bundle_service, oracle_worker_service,
};

const ONLINE_WINDOW_SECS: i64 = 90;

#[derive(Serialize)]
pub struct OracleWorkerInfo {
    pub label: String,
    pub online: bool,
    pub last_seen_at: String,
    pub last_seen_secs_ago: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub platform: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub logged_in: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub current_task_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chrome_alive: Option<bool>,
    pub desired_state: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    pub capabilities: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub provisioned_at: Option<String>,
}

#[derive(Serialize)]
pub struct ListOracleWorkersResponse {
    pub workers: Vec<OracleWorkerInfo>,
}

#[derive(Deserialize, Default)]
pub struct AllocateOracleWorkerRequest {
    /// Optional operator-chosen label; omitted = server-generated.
    #[serde(default)]
    pub label: Option<String>,
}

#[derive(Serialize)]
pub struct AllocateOracleWorkerResponse {
    pub label: String,
    /// True when an existing unbound (legacy) worker row was taken over.
    pub adopted: bool,
}

#[derive(Deserialize, Default)]
pub struct ForgetWorkerQuery {
    #[serde(default)]
    pub force: bool,
}

#[derive(Serialize)]
pub struct ForgetWorkerResponse {
    pub label: String,
    pub commands_removed: u64,
    pub sessions_released: u64,
    pub tasks_released: u64,
}

#[derive(Deserialize)]
pub struct EnqueueWorkerCommandRequest {
    pub command: String,
}

#[derive(Serialize)]
pub struct OracleWorkerCommandInfo {
    pub id: String,
    pub worker_label: String,
    pub command: String,
    pub status: String,
    pub delivery_count: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub result_code: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub snapshot_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bundle_version: Option<String>,
    pub created_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub completed_at: Option<String>,
}

#[derive(Serialize)]
pub struct ListOracleWorkerCommandsResponse {
    pub commands: Vec<OracleWorkerCommandInfo>,
}

#[derive(Deserialize)]
pub struct UploadLoginSnapshotRequest {
    pub format_version: u32,
    pub worker_token_sha256: String,
    pub sealed_blob_base64: String,
}

#[derive(Serialize)]
pub struct LoginSnapshotTargetInfo {
    pub worker_label: String,
    pub command_id: String,
}

#[derive(Serialize)]
pub struct UploadLoginSnapshotResponse {
    pub snapshot_id: String,
    pub envelope_size: u64,
    pub expires_at: String,
    pub queued_workers: Vec<LoginSnapshotTargetInfo>,
    pub skipped_workers: Vec<String>,
}

fn decode_login_snapshot_envelope(encoded: &str) -> AppResult<Zeroizing<Vec<u8>>> {
    if encoded.len() > oracle_login_snapshot_service::MAX_LOGIN_SNAPSHOT_BASE64_CHARS {
        return Err(AppError::OraclePayloadTooLarge(format!(
            "login snapshot must be at most {} base64 characters",
            oracle_login_snapshot_service::MAX_LOGIN_SNAPSHOT_BASE64_CHARS
        )));
    }
    let sealed_envelope = base64::engine::general_purpose::STANDARD
        .decode(encoded.as_bytes())
        .map_err(|_| {
            AppError::ValidationError("sealed_blob_base64 must be valid base64".to_string())
        })?;
    Ok(Zeroizing::new(sealed_envelope))
}

fn worker_info(worker: OracleWorker) -> OracleWorkerInfo {
    let last_seen_secs_ago = (Utc::now() - worker.last_seen_at).num_seconds().max(0);
    OracleWorkerInfo {
        label: worker.worker_label,
        online: last_seen_secs_ago <= ONLINE_WINDOW_SECS,
        last_seen_at: worker.last_seen_at.to_rfc3339(),
        last_seen_secs_ago,
        version: worker.script_version,
        platform: worker.platform,
        logged_in: worker.logged_in,
        current_task_id: worker.current_task_id,
        chrome_alive: worker.chrome_alive,
        desired_state: match worker.desired_state {
            OracleWorkerDesiredState::Active => "active",
            OracleWorkerDesiredState::Draining => "draining",
        }
        .to_string(),
        last_error: worker.last_error,
        capabilities: worker.capabilities,
        provisioned_at: worker.provisioned_at.map(|value| value.to_rfc3339()),
    }
}

fn command_info(command: OracleWorkerCommand) -> OracleWorkerCommandInfo {
    OracleWorkerCommandInfo {
        id: command.id,
        worker_label: command.worker_label,
        command: command.kind.as_str().to_string(),
        status: match command.status {
            OracleWorkerCommandStatus::Queued => "queued",
            OracleWorkerCommandStatus::Delivered => "delivered",
            OracleWorkerCommandStatus::Succeeded => "succeeded",
            OracleWorkerCommandStatus::Failed => "failed",
            OracleWorkerCommandStatus::Expired => "expired",
            OracleWorkerCommandStatus::Cancelled => "cancelled",
        }
        .to_string(),
        delivery_count: command.delivery_count,
        result_code: command.result_code,
        snapshot_id: command.snapshot_id,
        bundle_version: command.bundle_version,
        created_at: command.created_at.to_rfc3339(),
        completed_at: command.completed_at.map(|value| value.to_rfc3339()),
    }
}

fn parse_command(value: &str) -> AppResult<OracleWorkerCommandKind> {
    match value {
        "drain" => Ok(OracleWorkerCommandKind::Drain),
        "resume" => Ok(OracleWorkerCommandKind::Resume),
        "restart" => Ok(OracleWorkerCommandKind::Restart),
        "relaunch_browser" => Ok(OracleWorkerCommandKind::RelaunchBrowser),
        "relogin" => Ok(OracleWorkerCommandKind::Relogin),
        "upgrade" => Ok(OracleWorkerCommandKind::Upgrade),
        _ => Err(AppError::ValidationError(
            "command must be drain|resume|restart|relaunch_browser|relogin|upgrade".to_string(),
        )),
    }
}

async fn managed_pool(
    state: &AppState,
    actor_user_id: &str,
    id_or_slug: &str,
) -> AppResult<OraclePool> {
    let pool = oracle_pool_service::get_pool(&state.db, id_or_slug).await?;
    oracle_pool_service::ensure_can_manage(&state.db, actor_user_id, &pool).await?;
    Ok(pool)
}

pub async fn allocate_worker(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(id_or_slug): Path<String>,
    // Older CLIs post a JSON `null` body; absent bodies are accepted too.
    body: Option<Json<Option<AllocateOracleWorkerRequest>>>,
) -> AppResult<impl IntoResponse> {
    let actor = auth_user.user_id.to_string();
    let pool = managed_pool(&state, &actor, &id_or_slug).await?;
    let requested = body
        .and_then(|Json(inner)| inner)
        .and_then(|request| request.label)
        .map(|label| label.trim().to_string())
        .filter(|label| !label.is_empty());
    let allocated =
        oracle_worker_service::allocate_worker(&state.db, &pool, requested.as_deref()).await?;
    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "oracle_worker_allocated",
        Some(serde_json::json!({
            "pool_id": &pool.id,
            "worker_label": &allocated.worker.worker_label,
            "requested": requested.is_some(),
            "adopted": allocated.adopted,
        })),
    );
    Ok((
        StatusCode::CREATED,
        Json(AllocateOracleWorkerResponse {
            label: allocated.worker.worker_label,
            adopted: allocated.adopted,
        }),
    ))
}

pub async fn list_workers(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(id_or_slug): Path<String>,
) -> AppResult<Json<ListOracleWorkersResponse>> {
    let actor = auth_user.user_id.to_string();
    let pool = managed_pool(&state, &actor, &id_or_slug).await?;
    let workers = oracle_worker_service::list_workers(&state.db, &pool.id)
        .await?
        .into_iter()
        .map(worker_info)
        .collect();
    Ok(Json(ListOracleWorkersResponse { workers }))
}

pub async fn show_worker(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path((id_or_slug, label)): Path<(String, String)>,
) -> AppResult<Json<OracleWorkerInfo>> {
    let actor = auth_user.user_id.to_string();
    let pool = managed_pool(&state, &actor, &id_or_slug).await?;
    let worker = oracle_worker_service::get_worker(&state.db, &pool.id, &label).await?;
    Ok(Json(worker_info(worker)))
}

pub async fn forget_worker(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path((id_or_slug, label)): Path<(String, String)>,
    Query(query): Query<ForgetWorkerQuery>,
) -> AppResult<Json<ForgetWorkerResponse>> {
    let actor = auth_user.user_id.to_string();
    let pool = managed_pool(&state, &actor, &id_or_slug).await?;
    let outcome =
        oracle_worker_service::forget_worker(&state.db, &pool, &label, query.force).await?;
    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "oracle_worker_forgotten",
        Some(serde_json::json!({
            "pool_id": &pool.id,
            "worker_label": &label,
            "force": query.force,
            "commands_removed": outcome.commands_removed,
            "sessions_released": outcome.sessions_released,
            "tasks_released": outcome.tasks_released,
        })),
    );
    Ok(Json(ForgetWorkerResponse {
        label,
        commands_removed: outcome.commands_removed,
        sessions_released: outcome.sessions_released,
        tasks_released: outcome.tasks_released,
    }))
}

pub async fn enqueue_command(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path((id_or_slug, label)): Path<(String, String)>,
    Json(body): Json<EnqueueWorkerCommandRequest>,
) -> AppResult<impl IntoResponse> {
    let actor = auth_user.user_id.to_string();
    let pool = managed_pool(&state, &actor, &id_or_slug).await?;
    let kind = parse_command(&body.command)?;
    let bundle = if kind == OracleWorkerCommandKind::Upgrade {
        let current = oracle_worker_bundle_service::current_bundle();
        Some((current.version.to_string(), current.sha256.to_string()))
    } else {
        None
    };
    let command = oracle_worker_service::enqueue_command(
        &state.db, &pool.id, &actor, &label, kind, None, bundle,
    )
    .await?;
    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "oracle_worker_command_queued",
        Some(serde_json::json!({
            "pool_id": &pool.id,
            "worker_label": &label,
            "command_id": &command.id,
            "command": command.kind.as_str(),
        })),
    );
    Ok((StatusCode::ACCEPTED, Json(command_info(command))))
}

pub async fn cancel_command(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path((id_or_slug, label, command_id)): Path<(String, String, String)>,
) -> AppResult<Json<OracleWorkerCommandInfo>> {
    let actor = auth_user.user_id.to_string();
    let pool = managed_pool(&state, &actor, &id_or_slug).await?;
    let command =
        oracle_worker_service::cancel_command(&state.db, &pool.id, &label, &command_id).await?;
    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "oracle_worker_command_cancelled",
        Some(serde_json::json!({
            "pool_id": &pool.id,
            "worker_label": &label,
            "command_id": &command.id,
            "command": command.kind.as_str(),
            "was_delivered": command.delivery_count > 0,
        })),
    );
    Ok(Json(command_info(command)))
}

pub async fn list_commands(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path((id_or_slug, label)): Path<(String, String)>,
) -> AppResult<Json<ListOracleWorkerCommandsResponse>> {
    let actor = auth_user.user_id.to_string();
    let pool = managed_pool(&state, &actor, &id_or_slug).await?;
    oracle_worker_service::get_worker(&state.db, &pool.id, &label).await?;
    let commands = oracle_worker_service::list_commands(&state.db, &pool.id, Some(&label))
        .await?
        .into_iter()
        .map(command_info)
        .collect();
    Ok(Json(ListOracleWorkerCommandsResponse { commands }))
}

pub async fn upload_login_snapshot(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(id_or_slug): Path<String>,
    Json(body): Json<UploadLoginSnapshotRequest>,
) -> AppResult<impl IntoResponse> {
    let actor = auth_user.user_id.to_string();
    let pool = managed_pool(&state, &actor, &id_or_slug).await?;
    let sealed_envelope = decode_login_snapshot_envelope(&body.sealed_blob_base64)?;
    let fanout = oracle_login_snapshot_service::create_and_fanout(
        &state.db,
        &state.encryption_keys,
        &pool,
        &actor,
        oracle_login_snapshot_service::CreateLoginSnapshotInput {
            format_version: body.format_version,
            worker_token_sha256: Zeroizing::new(body.worker_token_sha256),
            sealed_envelope,
        },
    )
    .await?;

    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "oracle_login_snapshot_queued",
        Some(serde_json::json!({
            "pool_id": &pool.id,
            "snapshot_id": &fanout.snapshot_id,
            "envelope_size": fanout.envelope_size,
            "queued_worker_count": fanout.queued_workers.len(),
            "skipped_worker_count": fanout.skipped_workers.len(),
        })),
    );

    Ok((
        StatusCode::ACCEPTED,
        Json(UploadLoginSnapshotResponse {
            snapshot_id: fanout.snapshot_id,
            envelope_size: fanout.envelope_size,
            expires_at: fanout.expires_at.to_rfc3339(),
            queued_workers: fanout
                .queued_workers
                .into_iter()
                .map(|(worker_label, command_id)| LoginSnapshotTargetInfo {
                    worker_label,
                    command_id,
                })
                .collect(),
            skipped_workers: fanout.skipped_workers,
        }),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manager_command_parser_rejects_internal_session_import() {
        assert!(parse_command("drain").is_ok());
        assert!(parse_command("session_import").is_err());
        assert!(parse_command("unknown").is_err());
    }

    #[test]
    fn login_snapshot_rejects_oversized_base64_before_decoding() {
        let oversized =
            "!".repeat(oracle_login_snapshot_service::MAX_LOGIN_SNAPSHOT_BASE64_CHARS + 1);
        assert!(matches!(
            decode_login_snapshot_envelope(&oversized),
            Err(AppError::OraclePayloadTooLarge(_))
        ));
        assert!(matches!(
            decode_login_snapshot_envelope("not base64"),
            Err(AppError::ValidationError(_))
        ));
    }
}

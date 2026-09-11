use axum::{
    Json,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
};
use base64::Engine;
use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use super::oracle_workers::{UploadLoginSnapshotRequest, decode_login_snapshot_envelope};
use crate::models::{
    oracle_login_profile::{OracleLoginBinding, OracleLoginProfile},
    oracle_pool::OraclePool,
};
use crate::services::{
    audit_service, oracle_login_profile_service as profiles, oracle_pool_service,
    oracle_worker_service,
};
use crate::{AppState, errors::AppResult, mw::auth::AuthUser};

#[cfg(test)]
mod tests;

#[derive(Serialize)]
pub struct LoginProfileInfo {
    pub id: String,
    pub name: String,
    pub generation: String,
    pub revision: String,
    pub status: String,
    pub envelope_size: u64,
    pub updated_at: String,
    pub expires_at: String,
    pub workers: Vec<String>,
}

fn info(profile: &OracleLoginProfile, pool: &OraclePool) -> LoginProfileInfo {
    metadata_info(&profile.into(), pool)
}

fn metadata_info(profile: &profiles::LoginProfileMetadata, pool: &OraclePool) -> LoginProfileInfo {
    LoginProfileInfo {
        id: profile.id.clone(),
        name: profile.name.clone(),
        generation: profile.generation.clone(),
        revision: profile.revision.clone(),
        status: profiles::metadata_availability(profile, pool).into(),
        envelope_size: profile.envelope_size,
        updated_at: profile.updated_at.to_rfc3339(),
        expires_at: profile.expires_at.to_rfc3339(),
        workers: profile
            .bindings
            .iter()
            .map(|b| b.worker_label.clone())
            .collect(),
    }
}

async fn managed_pool(state: &AppState, auth: &AuthUser, pool: &str) -> AppResult<OraclePool> {
    let pool = oracle_pool_service::get_pool(&state.db, pool).await?;
    oracle_pool_service::ensure_can_manage(&state.db, &auth.user_id.to_string(), &pool).await?;
    Ok(pool)
}

pub async fn list(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(pool): Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    let pool = managed_pool(&state, &auth, &pool).await?;
    let rows = profiles::list(&state.db, &pool.id).await?;
    Ok(Json(
        serde_json::json!({ "profiles": rows.iter().map(|p| metadata_info(p, &pool)).collect::<Vec<_>>() }),
    ))
}

pub async fn save(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((pool, name)): Path<(String, String)>,
    Json(body): Json<SaveLoginRequest>,
) -> AppResult<Json<LoginProfileInfo>> {
    let pool = managed_pool(&state, &auth, &pool).await?;
    let profile = profiles::save(
        &state.db,
        &state.encryption_keys,
        &pool,
        &name,
        body.expected_generation.as_deref(),
        crate::services::oracle_login_snapshot_service::CreateLoginSnapshotInput {
            format_version: body.envelope.format_version,
            worker_token_sha256: Zeroizing::new(body.envelope.worker_token_sha256),
            sealed_envelope: decode_login_snapshot_envelope(&body.envelope.sealed_blob_base64)?,
        },
    )
    .await?;
    audit_service::log_for_user(
        state.db.clone(),
        &auth,
        "oracle_login_profile_saved",
        Some(
            serde_json::json!({ "pool_id": pool.id, "profile_id": profile.id,
            "generation": profile.generation, "envelope_size": profile.envelope_size }),
        ),
    );
    Ok(Json(info(&profile, &pool)))
}

#[derive(Deserialize)]
pub struct SaveLoginRequest {
    pub expected_generation: Option<String>,
    #[serde(flatten)]
    pub envelope: UploadLoginSnapshotRequest,
}

impl std::fmt::Debug for SaveLoginRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SaveLoginRequest")
            .field("envelope", &"[REDACTED]")
            .finish()
    }
}

pub async fn delete(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((pool, name)): Path<(String, String)>,
) -> AppResult<StatusCode> {
    let pool = managed_pool(&state, &auth, &pool).await?;
    profiles::delete(&state.db, &pool.id, &name).await?;
    audit_service::log_for_user(
        state.db.clone(),
        &auth,
        "oracle_login_profile_deleted",
        Some(serde_json::json!({ "pool_id": pool.id, "profile_name": name })),
    );
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct BindLoginRequest {
    pub login_profile: String,
    #[serde(default)]
    pub installation_id: Option<String>,
    #[serde(default)]
    pub replace_existing: bool,
}

#[derive(Serialize)]
pub struct BindingInfo {
    pub binding_id: String,
    pub worker_label: String,
    pub replace_existing: bool,
    pub requires_upgrade: bool,
}

fn binding_info(binding: OracleLoginBinding, requires_upgrade: bool) -> BindingInfo {
    BindingInfo {
        binding_id: binding.binding_id,
        worker_label: binding.worker_label,
        replace_existing: binding.replace_existing,
        requires_upgrade,
    }
}

pub async fn bind(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((pool, label)): Path<(String, String)>,
    Json(body): Json<BindLoginRequest>,
) -> AppResult<Json<BindingInfo>> {
    let pool = managed_pool(&state, &auth, &pool).await?;
    let binding = profiles::bind(
        &state.db,
        &pool,
        &body.login_profile,
        &label,
        body.installation_id.as_deref(),
        body.replace_existing,
    )
    .await?;
    audit_service::log_for_user(
        state.db.clone(),
        &auth,
        "oracle_login_profile_bound",
        Some(
            serde_json::json!({ "pool_id": pool.id, "worker_label": label,
            "binding_id": binding.binding_id, "replace_existing": binding.replace_existing }),
        ),
    );
    let worker = oracle_worker_service::get_worker(&state.db, &pool.id, &label).await?;
    let requires_upgrade = !worker
        .capabilities
        .iter()
        .any(|c| c == profiles::SAVED_LOGIN_CAPABILITY);
    Ok(Json(binding_info(binding, requires_upgrade)))
}

pub async fn unbind(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((pool, label)): Path<(String, String)>,
) -> AppResult<StatusCode> {
    let pool = managed_pool(&state, &auth, &pool).await?;
    profiles::unbind(&state.db, &pool.id, &label).await?;
    audit_service::log_for_user(
        state.db.clone(),
        &auth,
        "oracle_login_profile_unbound",
        Some(serde_json::json!({ "pool_id": pool.id, "worker_label": label })),
    );
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
pub struct WorkerLoginQuery {
    pub worker: String,
    pub instance_id: String,
    pub known_revision: Option<String>,
}

#[derive(Serialize)]
pub struct WorkerLoginResponse {
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub profile: Option<LoginProfileInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub binding: Option<BindingInfo>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub format_version: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sealed_blob_base64: Option<String>,
}

pub async fn current(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(query): Query<WorkerLoginQuery>,
) -> AppResult<Json<WorkerLoginResponse>> {
    let worker_auth = super::oracle_worker::authenticate_worker(&state, &headers).await?;
    worker_auth.ensure_identity(&query.worker, Some(&query.instance_id))?;
    let pool = worker_auth.pool;
    let current = if worker_auth.installation.is_some() {
        None
    } else {
        profiles::current(&state.db, &pool, &query.worker, &query.instance_id).await?
    };
    let Some((profile, binding)) = current else {
        return Ok(Json(WorkerLoginResponse {
            status: "unbound".into(),
            profile: None,
            binding: None,
            format_version: None,
            sealed_blob_base64: None,
        }));
    };
    let status = profiles::availability(&profile, &pool);
    let envelope =
        if status == "available" && query.known_revision.as_deref() != Some(&profile.revision) {
            Some(
                base64::engine::general_purpose::STANDARD.encode(
                    profiles::decrypt(&state.encryption_keys, &profile)
                        .await?
                        .as_slice(),
                ),
            )
        } else {
            None
        };
    Ok(Json(WorkerLoginResponse {
        status: status.into(),
        profile: Some(info(&profile, &pool)),
        binding: Some(binding_info(binding, false)),
        format_version: Some(profile.format_version),
        sealed_blob_base64: envelope,
    }))
}

#[derive(Deserialize)]
pub struct RefreshLoginRequest {
    pub worker: String,
    pub instance_id: String,
    pub profile_id: String,
    pub binding_id: String,
    pub generation: String,
    pub expected_revision: String,
    pub publication_id: String,
    pub format_version: u32,
    pub sealed_blob_base64: String,
}

impl std::fmt::Debug for RefreshLoginRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RefreshLoginRequest")
            .field("profile_id", &self.profile_id)
            .field("sealed_blob_base64", &"[REDACTED]")
            .finish()
    }
}

pub async fn refresh(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<RefreshLoginRequest>,
) -> AppResult<Json<LoginProfileInfo>> {
    let worker_auth = super::oracle_worker::authenticate_worker(&state, &headers).await?;
    worker_auth.ensure_identity(&body.worker, Some(&body.instance_id))?;
    worker_auth.ensure_pool_credential()?;
    let pool = worker_auth.pool;
    let profile = profiles::refresh(
        &state.db,
        &state.encryption_keys,
        &pool,
        &body.worker,
        &body.instance_id,
        profiles::RefreshLoginInput {
            profile_id: body.profile_id,
            binding_id: body.binding_id,
            generation: body.generation,
            expected_revision: body.expected_revision,
            publication_id: body.publication_id,
            format_version: body.format_version,
            sealed_envelope: decode_login_snapshot_envelope(&body.sealed_blob_base64)?,
        },
    )
    .await?;
    Ok(Json(info(&profile, &pool)))
}

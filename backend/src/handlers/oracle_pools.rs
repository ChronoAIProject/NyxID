//! Oracle pool management endpoints (`/api/v1/oracle/pools`).
//!
//! Consumer-authenticated (JWT or API key). The worker-facing endpoints
//! live in `handlers::oracle_worker` behind the pool worker token.

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::models::oracle_pool::{OraclePool, OraclePoolVisibility};
use crate::mw::auth::AuthUser;
use crate::services::{audit_service, oracle_pool_service, org_service};

#[derive(Deserialize)]
pub struct CreateOraclePoolRequest {
    pub slug: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    /// "private" (default) | "org" | "platform"
    #[serde(default)]
    pub visibility: Option<String>,
    #[serde(default)]
    pub chatgpt_project_url: Option<String>,
    #[serde(default)]
    pub default_model_label: Option<String>,
    #[serde(default)]
    pub allow_extract: Option<bool>,
    #[serde(default)]
    pub max_workers: Option<u32>,
    #[serde(default)]
    pub max_queue_length: Option<u32>,
    #[serde(default)]
    pub per_user_max_inflight: Option<u32>,
    #[serde(default)]
    pub task_timeout_secs: Option<u64>,
    /// Create the pool under this org (caller must be an org admin).
    #[serde(default)]
    pub target_org_id: Option<String>,
}

#[derive(Deserialize)]
pub struct UpdateOraclePoolRequest {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub visibility: Option<String>,
    #[serde(default)]
    pub chatgpt_project_url: Option<String>,
    #[serde(default)]
    pub default_model_label: Option<String>,
    #[serde(default)]
    pub allow_extract: Option<bool>,
    #[serde(default)]
    pub max_workers: Option<u32>,
    #[serde(default)]
    pub max_queue_length: Option<u32>,
    #[serde(default)]
    pub per_user_max_inflight: Option<u32>,
    #[serde(default)]
    pub task_timeout_secs: Option<u64>,
    #[serde(default)]
    pub is_active: Option<bool>,
}

#[derive(Serialize)]
pub struct OraclePoolInfo {
    pub id: String,
    pub slug: String,
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub visibility: String,
    pub owner_user_id: String,
    /// True when the caller may manage this pool.
    pub can_manage: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub chatgpt_project_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_model_label: Option<String>,
    pub allow_extract: bool,
    pub max_workers: u32,
    pub max_queue_length: u32,
    pub per_user_max_inflight: u32,
    pub task_timeout_secs: u64,
    pub is_active: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Serialize)]
pub struct CreateOraclePoolResponse {
    #[serde(flatten)]
    pub pool: OraclePoolInfo,
    /// Shown exactly once; only the SHA-256 hash is stored.
    pub worker_token: String,
}

#[derive(Serialize)]
pub struct RotateTokenResponse {
    pub id: String,
    pub slug: String,
    /// Shown exactly once; all worker tabs must be re-paired.
    pub worker_token: String,
}

#[derive(Serialize)]
pub struct ListOraclePoolsResponse {
    pub pools: Vec<OraclePoolInfo>,
}

fn parse_visibility(value: &str) -> AppResult<OraclePoolVisibility> {
    match value {
        "private" => Ok(OraclePoolVisibility::Private),
        "org" => Ok(OraclePoolVisibility::Org),
        "platform" => Ok(OraclePoolVisibility::Platform),
        other => Err(AppError::ValidationError(format!(
            "visibility must be private|org|platform, got '{other}'"
        ))),
    }
}

fn pool_info(pool: &OraclePool, can_manage: bool) -> OraclePoolInfo {
    OraclePoolInfo {
        id: pool.id.clone(),
        slug: pool.slug.clone(),
        name: pool.name.clone(),
        description: pool.description.clone(),
        visibility: pool.visibility.as_str().to_string(),
        owner_user_id: pool.user_id.clone(),
        can_manage,
        chatgpt_project_url: pool.chatgpt_project_url.clone(),
        default_model_label: pool.default_model_label.clone(),
        allow_extract: pool.allow_extract,
        max_workers: pool.max_workers,
        max_queue_length: pool.max_queue_length,
        per_user_max_inflight: pool.per_user_max_inflight,
        task_timeout_secs: pool.task_timeout_secs,
        is_active: pool.is_active,
        created_at: pool.created_at.to_rfc3339(),
        updated_at: pool.updated_at.to_rfc3339(),
    }
}

async fn can_manage(state: &AppState, actor: &str, pool: &OraclePool) -> bool {
    oracle_pool_service::ensure_can_manage(&state.db, actor, pool)
        .await
        .is_ok()
}

pub async fn create_pool(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<CreateOraclePoolRequest>,
) -> AppResult<impl IntoResponse> {
    let actor = auth_user.user_id.to_string();
    let owner = match body.target_org_id.as_deref() {
        Some(org_id) => {
            let access = org_service::resolve_owner_access(&state.db, &actor, org_id).await?;
            if !access.can_write() {
                return Err(AppError::OrgRoleInsufficient(
                    "you must be an admin of the target org to create a pool under it".to_string(),
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
    let (pool, worker_token) = oracle_pool_service::create_pool(
        &state.db,
        &owner,
        oracle_pool_service::CreatePoolInput {
            slug: body.slug,
            name: body.name,
            description: body.description,
            visibility,
            chatgpt_project_url: body.chatgpt_project_url,
            default_model_label: body.default_model_label,
            allow_extract: body.allow_extract,
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
        "oracle_pool_created",
        Some(serde_json::json!({
            "pool_id": &pool.id,
            "slug": &pool.slug,
            "visibility": pool.visibility.as_str(),
            "owner_user_id": &pool.user_id,
        })),
    );

    let info = pool_info(&pool, true);
    Ok((
        StatusCode::CREATED,
        Json(CreateOraclePoolResponse {
            pool: info,
            worker_token,
        }),
    ))
}

pub async fn list_pools(
    State(state): State<AppState>,
    auth_user: AuthUser,
) -> AppResult<Json<ListOraclePoolsResponse>> {
    let actor = auth_user.user_id.to_string();
    let pools = oracle_pool_service::list_visible_pools(&state.db, &actor).await?;
    let mut infos = Vec::with_capacity(pools.len());
    for pool in &pools {
        let manage = can_manage(&state, &actor, pool).await;
        infos.push(pool_info(pool, manage));
    }
    Ok(Json(ListOraclePoolsResponse { pools: infos }))
}

pub async fn get_pool(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(id_or_slug): Path<String>,
) -> AppResult<Json<OraclePoolInfo>> {
    let actor = auth_user.user_id.to_string();
    let pool = oracle_pool_service::get_pool(&state.db, &id_or_slug).await?;
    oracle_pool_service::ensure_can_view(&state.db, &actor, &pool).await?;
    let manage = can_manage(&state, &actor, &pool).await;
    Ok(Json(pool_info(&pool, manage)))
}

pub async fn update_pool(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(id_or_slug): Path<String>,
    Json(body): Json<UpdateOraclePoolRequest>,
) -> AppResult<Json<OraclePoolInfo>> {
    let actor = auth_user.user_id.to_string();
    let visibility = body
        .visibility
        .as_deref()
        .map(parse_visibility)
        .transpose()?;
    let pool = oracle_pool_service::update_pool(
        &state.db,
        &actor,
        &id_or_slug,
        oracle_pool_service::UpdatePoolInput {
            name: body.name,
            description: body.description,
            visibility,
            chatgpt_project_url: body.chatgpt_project_url,
            default_model_label: body.default_model_label,
            allow_extract: body.allow_extract,
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
        "oracle_pool_updated",
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
) -> AppResult<Json<RotateTokenResponse>> {
    let actor = auth_user.user_id.to_string();
    let (pool, worker_token) =
        oracle_pool_service::rotate_worker_token(&state.db, &actor, &id_or_slug).await?;

    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "oracle_pool_token_rotated",
        Some(serde_json::json!({
            "pool_id": &pool.id,
            "slug": &pool.slug,
        })),
    );

    Ok(Json(RotateTokenResponse {
        id: pool.id,
        slug: pool.slug,
        worker_token,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visibility_parsing() {
        assert!(matches!(
            parse_visibility("private"),
            Ok(OraclePoolVisibility::Private)
        ));
        assert!(matches!(
            parse_visibility("org"),
            Ok(OraclePoolVisibility::Org)
        ));
        assert!(matches!(
            parse_visibility("platform"),
            Ok(OraclePoolVisibility::Platform)
        ));
        assert!(parse_visibility("public").is_err());
        assert!(parse_visibility("").is_err());
    }

    #[test]
    fn pool_info_redacts_nothing_but_token() {
        // The response struct has no token/hash field at all — the only
        // way a worker token leaves the server is the one-shot create /
        // rotate response.
        let now = chrono::Utc::now();
        let pool = OraclePool {
            id: "p1".to_string(),
            user_id: "u1".to_string(),
            slug: "s".to_string(),
            name: "n".to_string(),
            description: None,
            visibility: OraclePoolVisibility::Platform,
            worker_token_hash: "secret-hash".to_string(),
            chatgpt_project_url: None,
            default_model_label: None,
            allow_extract: false,
            max_workers: 3,
            max_queue_length: 50,
            per_user_max_inflight: 2,
            task_timeout_secs: 14_400,
            is_active: true,
            created_at: now,
            updated_at: now,
        };
        let json = serde_json::to_string(&pool_info(&pool, false)).unwrap();
        assert!(!json.contains("secret-hash"));
        assert!(!json.contains("worker_token"));
        assert!(json.contains("\"can_manage\":false"));
    }
}

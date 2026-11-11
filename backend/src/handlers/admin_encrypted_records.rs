//! Explicit control-plane migration actions. No runner is activated by startup.
use crate::{
    AppState,
    errors::{AppError, AppResult},
    mw::auth::AuthUser,
    services::{audit_service, encrypted_record_migration as migration},
};
use axum::{
    Json,
    extract::{Path, Query, State},
};
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StartRequest {
    pub mode: migration::Mode,
    /// Explicit replacement of a stopped/expired generation after deployment
    /// registry changes. A currently leased batch cannot be superseded.
    pub restart_generation: Option<uuid::Uuid>,
}

#[derive(Serialize)]
pub struct StartResponse {
    pub generation: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FindingLookupRequest {
    pub after: Option<String>,
    #[serde(default)]
    pub rewrap: bool,
}

#[derive(Deserialize)]
pub struct FindingQuery {
    pub after: Option<String>,
}

pub async fn findings(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(query): Query<FindingQuery>,
) -> AppResult<Json<migration::FindingPage>> {
    super::admin_helpers::require_admin_or_operator(
        &state,
        &auth,
        "admin.encrypted_records.findings",
    )
    .await?;
    Ok(Json(
        migration::list_findings(&state.db, query.after.as_deref()).await?,
    ))
}
crate::errors::redacted_debug!(FindingLookupRequest);

pub async fn lookup_finding(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(finding_id): Path<String>,
    Json(body): Json<FindingLookupRequest>,
) -> AppResult<Json<migration::FindingLookup>> {
    // POST keeps continuations out of URLs. Rewrap is storage-only but is an
    // explicit write, so this endpoint always requires the full admin role.
    super::admin_helpers::require_admin(&state, &auth).await?;
    if finding_id.len() != 64 || !finding_id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(AppError::ValidationError(
            "Invalid migration finding".into(),
        ));
    }
    audit_service::log_actor_event(
        state.db.clone(),
        &audit_service::AuditActor::from_auth_user(&auth),
        "encrypted_records.finding_lookup_requested",
        Some(serde_json::json!({"finding":finding_id,"rewrap":body.rewrap})),
    )
    .await?;
    Ok(Json(
        migration::lookup_finding(
            &state.db,
            &state.encryption_keys,
            &finding_id,
            body.after.as_deref(),
            body.rewrap,
        )
        .await?,
    ))
}

pub async fn status(
    State(state): State<AppState>,
    auth: AuthUser,
) -> AppResult<Json<Option<migration::Report>>> {
    super::admin_helpers::require_admin_or_operator(
        &state,
        &auth,
        "admin.encrypted_records.status",
    )
    .await?;
    Ok(Json(
        migration::status(&state.db, &state.encryption_keys).await?,
    ))
}

pub async fn start(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<StartRequest>,
) -> AppResult<Json<StartResponse>> {
    super::admin_helpers::require_admin(&state, &auth).await?;
    let restart = body.restart_generation.map(|value| value.to_string());
    audit_service::log_actor_event(
        state.db.clone(),
        &audit_service::AuditActor::from_auth_user(&auth),
        "encrypted_records.walk_requested",
        Some(serde_json::json!({"mode":body.mode,"restart_generation":restart})),
    )
    .await?;
    let generation = migration::start(
        &state.db,
        &state.encryption_keys,
        body.mode,
        restart.as_deref(),
    )
    .await?;
    Ok(Json(StartResponse { generation }))
}

pub async fn advance(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(generation): Path<String>,
) -> AppResult<Json<Option<migration::Report>>> {
    super::admin_helpers::require_admin(&state, &auth).await?;
    uuid::Uuid::parse_str(&generation)
        .map_err(|_| AppError::ValidationError("Invalid migration generation".into()))?;
    audit_service::log_actor_event(
        state.db.clone(),
        &audit_service::AuditActor::from_auth_user(&auth),
        "encrypted_records.batch_requested",
        Some(serde_json::json!({"generation":generation})),
    )
    .await?;
    migration::advance(&state.db, &state.encryption_keys, &generation).await?;
    Ok(Json(
        migration::status(&state.db, &state.encryption_keys).await?,
    ))
}

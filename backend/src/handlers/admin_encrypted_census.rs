use super::admin_helpers::{require_admin, require_admin_or_operator};
use crate::{
    AppState,
    errors::AppResult,
    mw::auth::{AuthUser, require_first_party_human},
    services::{
        audit_service::{self, AuditActor},
        encrypted_record_census::report,
    },
};
use axum::{
    Json,
    extract::{Path, Query, State},
};
use uuid::Uuid;

async fn write(
    state: &AppState,
    auth: &AuthUser,
    event: &'static str,
    id: Option<Uuid>,
) -> AppResult<()> {
    require_first_party_human(auth)?;
    require_admin(state, auth).await?;
    audit_service::log_actor_event(
        state.db.clone(),
        &AuditActor::from_auth_user(auth),
        event,
        id.map(|id| serde_json::json!({"generation":id})),
    )
    .await?;
    Ok(())
}

pub async fn start(
    State(state): State<AppState>,
    auth: AuthUser,
) -> AppResult<Json<report::Status>> {
    write(&state, &auth, "encrypted_records.census_requested", None).await?;
    Ok(Json(
        report::start(&state.db, &state.encryption_keys, &state.config).await?,
    ))
}
pub async fn advance(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
) -> AppResult<Json<report::Status>> {
    write(
        &state,
        &auth,
        "encrypted_records.census_advance_requested",
        Some(id),
    )
    .await?;
    Ok(Json(
        report::advance(
            &state.db,
            &state.encryption_keys,
            &state.config,
            &id.to_string(),
        )
        .await?,
    ))
}
pub async fn status(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
) -> AppResult<Json<report::Status>> {
    require_first_party_human(&auth)?;
    require_admin_or_operator(&state, &auth, "admin.census.status").await?;
    Ok(Json(
        report::status(&state.db, &state.encryption_keys, &id.to_string()).await?,
    ))
}
pub async fn families(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
    Query(q): Query<report::Query>,
) -> AppResult<Json<report::Page>> {
    require_first_party_human(&auth)?;
    require_admin_or_operator(&state, &auth, "admin.census.families").await?;
    Ok(Json(
        report::families(&state.db, &state.encryption_keys, &id.to_string(), q).await?,
    ))
}
pub async fn findings(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<Uuid>,
    Query(q): Query<report::Query>,
) -> AppResult<Json<report::Page>> {
    require_first_party_human(&auth)?;
    require_admin_or_operator(&state, &auth, "admin.census.findings").await?;
    Ok(Json(
        report::findings(&state.db, &state.encryption_keys, &id.to_string(), q).await?,
    ))
}

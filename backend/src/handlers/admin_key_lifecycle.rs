use super::admin_helpers::{require_admin, require_admin_or_operator};
use crate::{
    AppState,
    errors::AppResult,
    mw::auth::{AuthUser, require_first_party_human},
    services::{audit_service::AuditActor, key_lifecycle as lifecycle},
};
use axum::{
    Json,
    extract::{Query, State},
};

pub async fn status(
    State(state): State<AppState>,
    auth: AuthUser,
) -> AppResult<Json<Option<lifecycle::Status>>> {
    require_first_party_human(&auth)?;
    require_admin_or_operator(&state, &auth, "admin.key_lifecycle.status").await?;
    Ok(Json(lifecycle::status(&state.db, &state.config).await?))
}
pub async fn dependencies(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(query): Query<lifecycle::Page>,
) -> AppResult<Json<Vec<lifecycle::Dependency>>> {
    require_first_party_human(&auth)?;
    require_admin_or_operator(&state, &auth, "admin.key_lifecycle.dependencies").await?;
    Ok(Json(lifecycle::dependencies(&state.db, query).await?))
}
pub async fn history(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(query): Query<lifecycle::Page>,
) -> AppResult<Json<Vec<lifecycle::Evidence>>> {
    require_first_party_human(&auth)?;
    require_admin_or_operator(&state, &auth, "admin.key_lifecycle.evidence").await?;
    Ok(Json(lifecycle::evidence(&state.db, query).await?))
}
pub async fn enroll(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(input): Json<lifecycle::Enroll>,
) -> AppResult<Json<lifecycle::Receipt>> {
    require_first_party_human(&auth)?;
    require_admin(&state, &auth).await?;
    Ok(Json(
        lifecycle::enroll(
            &state.db,
            &state.config,
            &AuditActor::from_auth_user(&auth),
            &input,
        )
        .await?,
    ))
}
pub async fn append(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(input): Json<lifecycle::Append>,
) -> AppResult<Json<lifecycle::Receipt>> {
    require_first_party_human(&auth)?;
    require_admin(&state, &auth).await?;
    Ok(Json(
        lifecycle::append(
            &state.db,
            &state.config,
            &AuditActor::from_auth_user(&auth),
            &input,
        )
        .await?,
    ))
}

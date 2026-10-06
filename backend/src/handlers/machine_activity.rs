//! Human-only machine metadata and thread-local preview consent. No execution.
use crate::{
    AppState,
    errors::{AppError, AppResult},
    mw::auth::AuthUser,
    services::{machine_activity_service as activity, node_service, org_service},
};
use axum::{
    Json,
    extract::{Path, Query, State},
};
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PreviewPolicy {
    pub enabled: bool,
}
#[derive(Serialize)]
pub struct PreviewPolicyResponse {
    enabled: bool,
    maximum_days: u32,
}

pub async fn preview_policy(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(input): Json<PreviewPolicy>,
) -> AppResult<Json<PreviewPolicyResponse>> {
    super::login_client_context::require_first_party_human(&auth)?;
    let actor = auth.user_id.to_string();
    Box::pin(activity::preview_policy(
        &state.db,
        &actor,
        &id,
        Some(input.enabled),
    ))
    .await?;
    Ok(Json(PreviewPolicyResponse {
        enabled: input.enabled,
        maximum_days: 30,
    }))
}

pub async fn get_preview_policy(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<PreviewPolicyResponse>> {
    super::login_client_context::require_first_party_human(&auth)?;
    let enabled = Box::pin(activity::preview_policy(
        &state.db,
        &auth.user_id.to_string(),
        &id,
        None,
    ))
    .await?;
    Ok(Json(PreviewPolicyResponse {
        enabled,
        maximum_days: 30,
    }))
}

pub async fn list(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(node_id): Path<String>,
    Query(query): Query<activity::ActivityQuery>,
) -> AppResult<Json<activity::ActivityPage>> {
    super::login_client_context::require_first_party_human(&auth)?;
    let node = node_service::get_node_by_id(&state.db, &node_id)
        .await?
        .filter(|n| n.is_active && n.machine.is_some())
        .ok_or_else(|| AppError::NotFound("Machine not found".into()))?;
    if !org_service::resolve_owner_access(&state.db, &auth.user_id.to_string(), &node.user_id)
        .await?
        .can_write()
    {
        return Err(AppError::Forbidden(
            "Machine activity is available to its owner or organization admin".into(),
        ));
    }
    let mut page = Box::pin(activity::list(&state.db, &node.id, query)).await?;
    page.machine_name = Some(node.name);
    page.agents = Box::pin(activity::activity_agents(
        &state.db,
        &auth.user_id.to_string(),
        &node.user_id,
    ))
    .await?;
    Ok(Json(page))
}

//! Human-only machine metadata and thread-local preview consent. No execution.
use crate::{
    AppState, errors::AppResult, mw::auth::AuthUser, services::machine_activity_service as activity,
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
    Ok(Json(
        Box::pin(activity::list_for_owner(
            &state.db,
            &auth.user_id.to_string(),
            &node_id,
            query,
        ))
        .await?,
    ))
}

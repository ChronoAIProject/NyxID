//! Sanitized admin review of captured gateway cleanup authority.
use super::admin_helpers::require_admin;
use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::nyxbot_gateway_release::{NyxbotGatewayRelease, ReleaseState},
    mw::auth::AuthUser,
    services::nyxbot_gateway_release_service as service,
};
use axum::{
    Json,
    extract::{Path, Query, State},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListQuery {
    pub after: Option<String>,
    pub limit: Option<i64>,
    pub states: Option<String>,
}

#[derive(Serialize)]
pub struct ReleaseResponse {
    id: String,
    channel_id: String,
    owner_user_id: String,
    gateway_service_id: String,
    record_id: Option<String>,
    gateway_channel_id: Option<String>,
    state: ReleaseState,
    revision: i64,
    expected_version: Option<i64>,
    observed_version: Option<i64>,
    last_error: Option<String>,
    resolution: Option<String>,
    created_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
}
impl From<NyxbotGatewayRelease> for ReleaseResponse {
    fn from(row: NyxbotGatewayRelease) -> Self {
        Self {
            id: row.id,
            channel_id: row.channel_id,
            owner_user_id: row.owner_user_id,
            gateway_service_id: row.gateway_service_id,
            record_id: row.record_id,
            gateway_channel_id: row.gateway_channel_id,
            state: row.state,
            revision: row.revision,
            expected_version: row.expected_version,
            observed_version: row.observed_version,
            last_error: row.last_error,
            resolution: row.resolution,
            created_at: row.created_at,
            updated_at: row.updated_at,
        }
    }
}
#[derive(Serialize)]
pub struct ListResponse {
    items: Vec<ReleaseResponse>,
    next_cursor: Option<String>,
}

pub async fn list(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(query): Query<ListQuery>,
) -> AppResult<Json<ListResponse>> {
    require_admin(&state, &auth).await?;
    let limit = query.limit.unwrap_or(50).clamp(1, 100);
    let states = match query.states {
        None => vec![ReleaseState::Due, ReleaseState::ReviewRequired],
        Some(value) => value
            .split(',')
            .map(|state| match state {
                "due" => Ok(ReleaseState::Due),
                "review_required" => Ok(ReleaseState::ReviewRequired),
                "settled" => Ok(ReleaseState::Settled),
                "provisional" => Ok(ReleaseState::Provisional),
                _ => Err(AppError::ValidationError("Invalid release state".into())),
            })
            .collect::<AppResult<Vec<_>>>()?,
    };
    let rows = service::list(&state.db, query.after.as_deref(), states, limit)
        .await
        .map_err(storage_error)?;
    let next_cursor = if rows.len() == limit as usize {
        rows.last().map(|r| r.id.clone())
    } else {
        None
    };
    Ok(Json(ListResponse {
        items: rows.into_iter().map(Into::into).collect(),
        next_cursor,
    }))
}

#[derive(Serialize)]
pub struct InspectionResponse {
    captured: ReleaseResponse,
    observation: service::Observation,
}
pub async fn inspect(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<InspectionResponse>> {
    require_admin(&state, &auth).await?;
    let row = service::get(&state.db, &id).await.map_err(storage_error)?;
    let observation = service::observe(&state, &row)
        .await
        .map_err(storage_error)?;
    Ok(Json(InspectionResponse {
        captured: row.into(),
        observation,
    }))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolveRequest {
    revision: i64,
    observation: service::Observation,
    action: service::Resolution,
    #[serde(default)]
    dispatch_drained: bool,
}
pub async fn resolve(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(body): Json<ResolveRequest>,
) -> AppResult<axum::http::StatusCode> {
    require_admin(&state, &auth).await?;
    service::resolve(
        &state,
        &auth.user_id.to_string(),
        &id,
        body.revision,
        body.observation,
        body.action,
        body.dispatch_drained,
    )
    .await
    .map_err(storage_error)?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

fn storage_error(error: AppError) -> AppError {
    match error {
        AppError::Forbidden(code) if code == "gateway_authority_refused" => {
            AppError::OAuthAuthorityUnavailable
        }
        AppError::Conflict(_)
        | AppError::ValidationError(_)
        | AppError::NotFound(_)
        | AppError::Forbidden(_) => error,
        _ => AppError::OAuthAuthorityUnavailable,
    }
}

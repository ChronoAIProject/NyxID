use axum::{
    Json,
    extract::{Query, State},
};
use chrono::Utc;

use super::admin_helpers::require_admin_or_operator;
use crate::services::admin_usage_service::{self, AdminUsageQuery, AdminUsageResponse};
use crate::telemetry::{TelemetryContext, TelemetryEvent, emit_event};
use crate::{AppState, errors::AppResult, mw::auth::AuthUser};

#[utoipa::path(
    get, path = "/api/v1/admin/usage", tag = "Admin",
    params(AdminUsageQuery),
    responses((status = 200, description = "Platform-wide usage", body = AdminUsageResponse),
        (status = 400, description = "Invalid window or filter"),
        (status = 403, description = "Admin or operator required"),
        (status = 503, description = "Usage query timed out; narrow the window or filters")),
    security(("bearer_auth" = []))
)]
pub async fn get_usage(
    State(state): State<AppState>,
    auth: AuthUser,
    tele: TelemetryContext,
    Query(query): Query<AdminUsageQuery>,
) -> AppResult<Json<AdminUsageResponse>> {
    require_admin_or_operator(&state, &auth, "admin.usage.list").await?;
    let applied: Vec<_> = [
        ("window", query.period.is_some() || query.from.is_some()),
        ("user", query.user.is_some()),
        ("service", query.service.is_some()),
        ("sort", query.sort.is_some()),
        ("metric", query.metric.is_some()),
    ]
    .into_iter()
    .filter_map(|(key, present)| present.then_some(key))
    .collect();
    let response = admin_usage_service::get_usage(&state.db, query.validate(Utc::now())?).await?;
    emit_event(
        state.telemetry.as_deref(),
        &auth.user_id.to_string(),
        auth.api_key_id.as_deref(),
        &tele,
        TelemetryEvent::AdminUsageViewed {
            filter: (!applied.is_empty()).then(|| applied.join(",")),
        },
    );
    Ok(Json(response))
}

#[utoipa::path(
    get, path = "/api/v1/admin/usage/analytics", tag = "Admin",
    params(admin_usage_service::analytics::AnalyticsQuery),
    responses((status = 200, description = "Bounded billing analytics", body = admin_usage_service::analytics::AnalyticsResponse)),
    security(("bearer_auth" = []))
)]
pub async fn get_analytics(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(query): Query<admin_usage_service::analytics::AnalyticsQuery>,
) -> AppResult<Json<admin_usage_service::analytics::AnalyticsResponse>> {
    require_admin_or_operator(&state, &auth, "admin.usage.analytics").await?;
    Ok(Json(
        admin_usage_service::analytics::get_analytics(&state.db, query.validate(Utc::now())?)
            .await?,
    ))
}

#[utoipa::path(
    get, path = "/api/v1/admin/usage/workspace", tag = "Admin",
    responses((status = 200, description = "Private analytics workspace", body = crate::services::usage_workspace_service::WorkspaceResponse)),
    security(("bearer_auth" = []))
)]
pub async fn get_workspace(
    State(state): State<AppState>,
    auth: AuthUser,
) -> AppResult<Json<crate::services::usage_workspace_service::WorkspaceResponse>> {
    require_admin_or_operator(&state, &auth, "admin.usage.workspace").await?;
    Ok(Json(
        crate::services::usage_workspace_service::get(&state.db, &auth.user_id.to_string()).await?,
    ))
}

#[utoipa::path(
    put, path = "/api/v1/admin/usage/workspace", tag = "Admin",
    request_body = crate::services::usage_workspace_service::SaveWorkspaceRequest,
    responses((status = 200, description = "Saved workspace", body = crate::services::usage_workspace_service::WorkspaceResponse), (status = 409, description = "Stale revision")),
    security(("bearer_auth" = []))
)]
pub async fn save_workspace(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(request): Json<crate::services::usage_workspace_service::SaveWorkspaceRequest>,
) -> AppResult<Json<crate::services::usage_workspace_service::WorkspaceResponse>> {
    super::admin_helpers::require_admin(&state, &auth).await?;
    Ok(Json(
        crate::services::usage_workspace_service::save(
            &state.db,
            &auth.user_id.to_string(),
            request,
        )
        .await?,
    ))
}

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

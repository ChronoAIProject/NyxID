//! Metadata-only visibility for managed provider liabilities.
use axum::{
    Json,
    extract::{Path, State},
};
use chrono::{DateTime, Utc};
use serde::Serialize;
use utoipa::ToSchema;

use crate::models::{
    credits::Credits,
    managed_execution::{ManagedExecution, ManagedExecutionState},
};
use crate::{AppState, errors::AppResult, mw::auth::AuthUser};

#[derive(Serialize, ToSchema)]
pub struct ExecutionStatus {
    pub id: String,
    pub service_slug: Option<String>,
    pub model: String,
    pub state: ManagedExecutionState,
    pub maximum_credits: Credits,
    pub pending_reason: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

fn response(row: ManagedExecution) -> ExecutionStatus {
    ExecutionStatus {
        id: row.id,
        service_slug: row.service_slug,
        model: row
            .billing
            .managed_execution
            .map(|c| c.model)
            .unwrap_or_default(),
        state: row.state,
        maximum_credits: row.maximum,
        pending_reason: row.pending_reason,
        created_at: row.created_at,
        updated_at: row.updated_at,
    }
}

fn wallet_visibility(auth: &AuthUser) -> Option<String> {
    // A person can inspect liabilities paid by their wallet, including their
    // service accounts. A service account sees only its own executions.
    let actor = auth.user_id.to_string();
    if auth.proxy_resolution_user_id() != actor {
        None
    } else {
        Some(actor)
    }
}

#[utoipa::path(
    get,
    path = "/api/v1/billing/executions",
    tag = "Billing",
    responses(
        (status = 200, description = "Latest 50 managed executions visible to the caller", body = [ExecutionStatus])
    ),
    security(("bearer_auth" = []))
)]
pub async fn list(
    State(state): State<AppState>,
    auth: AuthUser,
) -> AppResult<Json<Vec<ExecutionStatus>>> {
    let rows = crate::services::managed_execution_service::list_visible(
        &state.db,
        &auth.user_id.to_string(),
        wallet_visibility(&auth).as_deref(),
    )
    .await?;
    Ok(Json(rows.into_iter().map(response).collect()))
}

#[utoipa::path(
    get,
    path = "/api/v1/billing/executions/{id}",
    tag = "Billing",
    params(("id" = String, Path, description = "NyxID execution UUID")),
    responses(
        (status = 200, description = "Managed execution metadata without request content or credentials", body = ExecutionStatus),
        (status = 404, description = "Execution absent or not visible to the caller")
    ),
    security(("bearer_auth" = []))
)]
pub async fn get(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<ExecutionStatus>> {
    let row = crate::services::managed_execution_service::get_visible(
        &state.db,
        &auth.user_id.to_string(),
        wallet_visibility(&auth).as_deref(),
        &id,
    )
    .await?;
    Ok(Json(response(row)))
}

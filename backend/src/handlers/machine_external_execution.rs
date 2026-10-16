//! Human-only history and confirmation management. Machine execution scopes
//! never grant resource-management or approval authority.
use crate::{
    AppState, errors::AppResult, mw::auth::AuthUser,
    services::machine_external_service as execution,
};
use axum::{
    Json,
    extract::{Path, Query, State},
};
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryQuery {
    pub cursor: Option<String>,
    pub state: Option<String>,
    pub limit: Option<u32>,
    pub from: Option<String>,
    pub to: Option<String>,
}
pub async fn jobs(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(node): Path<String>,
    Query(query): Query<HistoryQuery>,
) -> AppResult<Json<Value>> {
    super::login_client_context::require_first_party_human(&auth)?;
    let mut args = json!({});
    if let Some(v) = query.cursor {
        args["cursor"] = json!(v);
    }
    if let Some(v) = query.state {
        args["state"] = json!(v);
    }
    if let Some(v) = query.limit {
        args["limit"] = json!(v);
    }
    if let Some(v) = query.from {
        args["from"] = json!(v);
    }
    if let Some(v) = query.to {
        args["to"] = json!(v);
    }
    Ok(Json(
        execution::owner_history(&state.db, &auth.user_id.to_string(), &node, &args).await?,
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConfirmationsQuery {
    pub node_id: Option<String>,
    pub cursor: Option<String>,
    pub limit: Option<u32>,
}
pub async fn confirmations(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(query): Query<ConfirmationsQuery>,
) -> AppResult<Json<Value>> {
    super::login_client_context::require_first_party_human(&auth)?;
    Ok(Json(
        execution::owner_confirmations_page(
            &state.db,
            &auth.user_id.to_string(),
            query.node_id.as_deref(),
            query.cursor.as_deref(),
            query.limit,
        )
        .await?,
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Decision {
    pub approve: bool,
}
pub async fn decide(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(body): Json<Decision>,
) -> AppResult<Json<Value>> {
    super::login_client_context::require_first_party_human(&auth)?;
    Ok(Json(
        execution::owner_decide(&state.db, &auth.user_id.to_string(), &id, body.approve).await?,
    ))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutputQuery {
    pub output_offset: Option<u64>,
    pub stderr_offset: Option<u64>,
}
pub async fn output(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((node, job)): Path<(String, String)>,
    Query(query): Query<OutputQuery>,
) -> AppResult<impl axum::response::IntoResponse> {
    super::login_client_context::require_first_party_human(&auth)?;
    let result = execution::owner_output(
        &state,
        &auth.user_id.to_string(),
        &node,
        &job,
        query.output_offset.unwrap_or(0),
        query.stderr_offset.unwrap_or(0),
    )
    .await?;
    Ok((
        [
            (axum::http::header::CACHE_CONTROL, "no-store"),
            (axum::http::header::PRAGMA, "no-cache"),
        ],
        Json(result),
    ))
}
pub async fn cancel(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((node, job)): Path<(String, String)>,
) -> AppResult<Json<Value>> {
    super::login_client_context::require_first_party_human(&auth)?;
    Ok(Json(
        execution::owner_cancel(&state, &auth.user_id.to_string(), &node, &job).await?,
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoundDecision {
    pub approve: bool,
    pub expected_grant_revision: i64,
    pub expected_runtime_generation: u64,
    pub parameters_digest: String,
}
pub async fn decide_bound(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((node, id)): Path<(String, String)>,
    Json(body): Json<BoundDecision>,
) -> AppResult<Json<Value>> {
    super::login_client_context::require_first_party_human(&auth)?;
    Ok(Json(
        execution::owner_decide_bound(
            &state.db,
            &auth.user_id.to_string(),
            &node,
            &id,
            body.approve,
            execution::ExpectedConfirmation {
                grant_revision: body.expected_grant_revision,
                runtime_generation: body.expected_runtime_generation,
                parameters_digest: &body.parameters_digest,
            },
        )
        .await?,
    ))
}

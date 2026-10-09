use crate::{
    AppState,
    errors::{AppError, AppResult},
    mw::auth::AuthUser,
    services::tools_service::{self, ToolOffering},
};
use axum::{
    Json,
    extract::{Path, State},
};

pub async fn list(
    State(state): State<AppState>,
    auth: AuthUser,
) -> AppResult<Json<Vec<ToolOffering>>> {
    let admin = super::services_helpers::is_admin(&state, &auth).await?;
    Ok(Json(
        tools_service::list(
            &state.db,
            &auth.user_id.to_string(),
            admin,
            (
                state.config.platform_service_rate_limit_per_second,
                state.config.platform_service_rate_limit_burst,
            ),
        )
        .await?,
    ))
}

pub async fn get(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(slug): Path<String>,
) -> AppResult<Json<ToolOffering>> {
    let Json(rows) = list(State(state), auth).await?;
    rows.into_iter()
        .find(|row| row.slug == slug)
        .map(Json)
        .ok_or_else(|| AppError::NotFound("Tool not found".into()))
}

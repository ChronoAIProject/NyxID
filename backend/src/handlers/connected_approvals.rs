use crate::services::execution_permission_dispatch::proposals::v2::connected::{
    self, Cancel, Observation,
};
use crate::{
    AppState,
    errors::{AppError, AppResult},
    mw::auth::{AuthMethod, AuthUser},
};
use axum::{
    Json,
    extract::{Path, State},
};
use serde::Serialize;

#[derive(Serialize)]
pub struct Capability {
    version: u32,
    mode: &'static str,
}

pub async fn capability(
    State(state): State<AppState>,
    auth: AuthUser,
) -> AppResult<Json<Capability>> {
    if auth.auth_method != AuthMethod::ApiKey
        || auth.assistant_chat.is_some()
        || auth.execution_permission.is_none()
    {
        return Err(AppError::NotFound(
            "Connected approval contract not found".into(),
        ));
    }
    connected::require_enabled(&state, &auth.user_id.to_string()).await?;
    Ok(Json(Capability {
        version: 1,
        mode: "connected",
    }))
}

pub async fn observe(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(key): Path<String>,
) -> AppResult<Json<Observation>> {
    connected::observe(&state, &auth, &key).await.map(Json)
}

pub async fn cancel(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(key): Path<String>,
    Json(expected): Json<Cancel>,
) -> AppResult<Json<Observation>> {
    connected::cancel(&state, &auth, &key, &expected)
        .await
        .map(Json)
}

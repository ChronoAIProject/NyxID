use crate::{
    AppState,
    errors::AppResult,
    models::saved_login::SavedLogin,
    mw::auth::AuthUser,
    services::{audit_service, saved_login_service as service},
};
use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use serde::{Deserialize, Serialize};

use super::login_client_context::require_first_party_human;

#[derive(Deserialize)]
pub struct Owner {
    #[serde(default)]
    pub available: bool,
    pub owner_id: Option<String>,
}
#[derive(Serialize)]
pub struct Metadata {
    pub id: String,
    pub owner_id: String,
    pub label: String,
    pub allowed_origins: Vec<String>,
    pub username_hint: String,
    pub has_password: bool,
    pub has_totp: bool,
    pub confirm_each_sign_in: bool,
    pub created_at: String,
    pub updated_at: String,
    pub last_used_at: Option<String>,
}
impl From<SavedLogin> for Metadata {
    fn from(row: SavedLogin) -> Self {
        Self {
            id: row.id,
            owner_id: row.user_id,
            label: row.label,
            allowed_origins: row.allowed_origins,
            username_hint: row.username_hint,
            has_password: row.password_encrypted.is_some(),
            has_totp: row.totp_secret_encrypted.is_some(),
            confirm_each_sign_in: row.confirm_each_sign_in,
            created_at: row.created_at.to_rfc3339(),
            updated_at: row.updated_at.to_rfc3339(),
            last_used_at: row.last_used_at.map(|v| v.to_rfc3339()),
        }
    }
}

pub async fn list(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(query): Query<Owner>,
) -> AppResult<Json<Vec<Metadata>>> {
    require_first_party_human(&auth)?;
    let actor = auth.user_id.to_string();
    let owner = query.owner_id.as_deref().unwrap_or(&actor);
    let rows = if query.available {
        service::available(&state.db, &actor).await?
    } else {
        service::list(&state.db, &actor, owner).await?
    };
    Ok(Json(rows.into_iter().map(Into::into).collect()))
}
pub async fn create(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(query): Query<Owner>,
    Json(input): Json<service::Input>,
) -> AppResult<(StatusCode, Json<Metadata>)> {
    require_first_party_human(&auth)?;
    let actor = auth.user_id.to_string();
    let owner = query.owner_id.as_deref().unwrap_or(&actor);
    let login = service::put(
        &state.db,
        &state.encryption_keys,
        &actor,
        owner,
        None,
        input,
    )
    .await?;
    audit_service::log_for_user(
        state.db.clone(),
        &auth,
        "saved_login_created",
        Some(serde_json::json!({"login_id":login.id,"owner_id":login.user_id})),
    );
    Ok((StatusCode::CREATED, Json(login.into())))
}
pub async fn replace(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(input): Json<service::Input>,
) -> AppResult<Json<Metadata>> {
    require_first_party_human(&auth)?;
    let actor = auth.user_id.to_string();
    let prior = service::get(&state.db, &actor, &id).await?;
    let login = service::put(
        &state.db,
        &state.encryption_keys,
        &actor,
        &prior.user_id,
        Some(&id),
        input,
    )
    .await?;
    audit_service::log_for_user(
        state.db.clone(),
        &auth,
        "saved_login_replaced",
        Some(serde_json::json!({"login_id":id})),
    );
    Ok(Json(login.into()))
}
pub async fn delete(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> AppResult<StatusCode> {
    require_first_party_human(&auth)?;
    service::delete(&state.db, &auth.user_id.to_string(), &id).await?;
    audit_service::log_for_user(
        state.db.clone(),
        &auth,
        "saved_login_deleted",
        Some(serde_json::json!({"login_id":id})),
    );
    Ok(StatusCode::NO_CONTENT)
}

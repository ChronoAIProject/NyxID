//! First-party human management and per-member sender confirmation.
use crate::{
    AppState,
    errors::{AppError, AppResult},
    mw::auth::AuthUser,
    services::org_channel_service as org,
};
use axum::{
    Json,
    extract::{Path, State},
};
use serde_json::{Value, json};

pub async fn get(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    super::login_client_context::require_first_party_human(&auth)?;
    let actor = auth.user_id.to_string();
    let row = org::management::load(&state.db, &actor, &id, false).await?;
    Ok(Json(org::management::view(&state.db, &actor, &row).await?))
}
pub async fn update(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(input): Json<org::management::GuestSettings>,
) -> AppResult<Json<Value>> {
    super::login_client_context::require_first_party_human(&auth)?;
    let actor = auth.user_id.to_string();
    let row = org::management::load(&state.db, &actor, &id, true).await?;
    org::management::update(&state.db, &actor, &row, input).await?;
    let row = org::management::load(&state.db, &actor, &id, true).await?;
    audit(&state, &actor, "org_channel_guest_policy_changed", &id).await;
    Ok(Json(org::management::view(&state.db, &actor, &row).await?))
}

/// Called only through NyxBot's existing sensitive channel-management tool.
/// The actor is its authenticated conversation owner, never a model argument.
pub(crate) async fn update_tool(
    state: &AppState,
    actor: &str,
    id: &str,
    input: &Value,
) -> AppResult<Value> {
    let input: org::management::GuestSettingsPatch = serde_json::from_value(input.clone())
        .map_err(|_| AppError::ValidationError("Invalid organization guest settings".into()))?;
    let row = org::management::load(&state.db, actor, id, true).await?;
    org::management::patch(&state.db, actor, &row, input).await?;
    let row = org::management::load(&state.db, actor, id, true).await?;
    audit(state, actor, "org_channel_guest_policy_changed", id).await;
    org::management::view(&state.db, actor, &row).await
}
pub async fn issue(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    super::login_client_context::require_first_party_human(&auth)?;
    let actor = auth.user_id.to_string();
    let row = org::management::load(&state.db, &actor, &id, false).await?;
    Ok(Json(org::links::issue(&state.db, &actor, &row).await?))
}
pub async fn challenge(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((id, challenge)): Path<(String, String)>,
) -> AppResult<Json<Value>> {
    super::login_client_context::require_first_party_human(&auth)?;
    let actor = auth.user_id.to_string();
    let row = org::management::load(&state.db, &actor, &id, false).await?;
    Ok(Json(
        org::links::status(&state.db, &actor, &row, &challenge).await?,
    ))
}
pub async fn confirm(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((id, challenge)): Path<(String, String)>,
) -> AppResult<Json<Value>> {
    super::login_client_context::require_first_party_human(&auth)?;
    let actor = auth.user_id.to_string();
    let row = org::management::load(&state.db, &actor, &id, false).await?;
    org::links::confirm(&state.db, &actor, &row, &challenge).await?;
    audit(&state, &actor, "org_channel_sender_confirmed", &id).await;
    Ok(Json(json!({"confirmed":true})))
}
pub async fn revoke_self(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    super::login_client_context::require_first_party_human(&auth)?;
    let actor = auth.user_id.to_string();
    // Removal remains possible after losing org access. Retain tombstones so a
    // removed member never silently becomes an organization-funded guest.
    let context = (state.db.clone(), id.clone(), actor.clone());
    crate::services::transaction_service::run_db_only(
        &state.db,
        &context,
        |tx, (db, id, actor)| {
            Box::pin(
                async move { org::lifecycle::retire_in_session(db, tx, id, Some(actor)).await },
            )
        },
    )
    .await?;
    audit(&state, &actor, "org_channel_sender_revoked", &id).await;
    Ok(Json(json!({"revoked":true})))
}
async fn audit(state: &AppState, actor: &str, action: &str, channel: &str) {
    crate::services::audit_service::log_async(
        state.db.clone(),
        Some(actor.into()),
        action.into(),
        Some(json!({"channel_id":channel})),
        None,
        None,
        None,
        None,
    );
}

/// Human-only, explicit whole-policy adoption from the specialist's Grants page.
pub async fn adoption_preview(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<Value>> {
    super::login_client_context::require_first_party_human(&auth)?;
    Ok(Json(
        org::adoption::preview(&state.db, &auth.user_id.to_string(), &id).await?,
    ))
}
pub async fn adopt_grants(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    Json(input): Json<org::adoption::Adoption>,
) -> AppResult<Json<Value>> {
    super::login_client_context::require_first_party_human(&auth)?;
    let actor = auth.user_id.to_string();
    org::adoption::adopt(&state.db, &actor, &id, input).await?;
    Ok(Json(org::adoption::preview(&state.db, &actor, &id).await?))
}

use axum::{
    Json,
    extract::{Path, Query, State},
};
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::models::trigger::{Trigger, TriggerDelivery, TriggerStatus, TriggerVerification};
use crate::mw::auth::AuthUser;
use crate::services::{audit_service, org_service, trigger_service};

#[derive(Debug, Deserialize)]
pub struct CreateTriggerRequest {
    pub label: String,
    pub user_service_id: Option<String>,
    pub verification: TriggerVerification,
    pub delivery: TriggerDelivery,
    pub target_org_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateTriggerRequest {
    pub label: Option<String>,
    pub status: Option<TriggerStatus>,
    pub delivery: Option<TriggerDelivery>,
}

#[derive(Debug, Deserialize)]
pub struct ListTriggersQuery {
    pub org_id: Option<String>,
}

#[derive(Serialize)]
pub struct TriggerResponse {
    pub id: String,
    pub user_id: String,
    pub label: String,
    pub user_service_id: Option<String>,
    pub status: TriggerStatus,
    pub verification: TriggerVerification,
    pub delivery: TriggerDelivery,
    pub inbound_url: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Serialize)]
pub struct CreateTriggerResponse {
    pub trigger: TriggerResponse,
    pub secret: String,
    pub delivery_signing_secret: Option<String>,
}

#[derive(Serialize)]
pub struct UpdateTriggerResponse {
    pub trigger: TriggerResponse,
    pub delivery_signing_secret: Option<String>,
}

#[derive(Serialize)]
pub struct ListTriggersResponse {
    pub triggers: Vec<TriggerResponse>,
}

#[derive(Serialize)]
pub struct RotateTriggerSecretResponse {
    pub trigger: TriggerResponse,
    pub secret: String,
}

#[derive(Serialize)]
pub struct DeleteTriggerResponse {
    pub message: &'static str,
}

pub async fn create_trigger(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<CreateTriggerRequest>,
) -> AppResult<Json<CreateTriggerResponse>> {
    let actor = auth_user.user_id.to_string();
    let owner = resolve_target_owner(&state, &actor, body.target_org_id.as_deref(), true).await?;
    let created = trigger_service::create(
        &state.db,
        &state.encryption_keys,
        trigger_service::CreateInput {
            user_id: owner,
            label: body.label,
            user_service_id: body.user_service_id,
            verification: body.verification,
            delivery: body.delivery,
        },
    )
    .await?;
    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "trigger_created",
        Some(serde_json::json!({
            "trigger_id": &created.trigger.id,
            "owner_user_id": &created.trigger.user_id,
            "user_service_id": &created.trigger.user_service_id,
            "delivery_type": delivery_name(&created.trigger.delivery),
        })),
    );
    Ok(Json(CreateTriggerResponse {
        trigger: response(&state, created.trigger),
        secret: created.raw_secret,
        delivery_signing_secret: created.delivery_signing_secret,
    }))
}

pub async fn list_triggers(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Query(query): Query<ListTriggersQuery>,
) -> AppResult<Json<ListTriggersResponse>> {
    let actor = auth_user.user_id.to_string();
    let owner = resolve_target_owner(&state, &actor, query.org_id.as_deref(), false).await?;
    let triggers = trigger_service::list_for_owner(&state.db, &owner).await?;
    Ok(Json(ListTriggersResponse {
        triggers: triggers
            .into_iter()
            .map(|item| response(&state, item))
            .collect(),
    }))
}

pub async fn get_trigger(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<TriggerResponse>> {
    let trigger =
        trigger_service::get_for_actor(&state.db, &auth_user.user_id.to_string(), &id).await?;
    Ok(Json(response(&state, trigger)))
}

pub async fn update_trigger(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(id): Path<String>,
    Json(body): Json<UpdateTriggerRequest>,
) -> AppResult<Json<UpdateTriggerResponse>> {
    let current =
        trigger_service::ensure_actor_can_write(&state.db, &auth_user.user_id.to_string(), &id)
            .await?;
    let mut changed_fields = Vec::new();
    if body.label.is_some() {
        changed_fields.push("label");
    }
    if body.status.is_some() {
        changed_fields.push("status");
    }
    if body.delivery.is_some() {
        changed_fields.push("delivery");
    }
    let updated = trigger_service::update(
        &state.db,
        &state.encryption_keys,
        &current,
        trigger_service::UpdateInput {
            label: body.label,
            status: body.status,
            delivery: body.delivery,
        },
    )
    .await?;
    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "trigger_updated",
        Some(serde_json::json!({
            "trigger_id": &updated.trigger.id,
            "owner_user_id": &updated.trigger.user_id,
            "changed_fields": changed_fields,
        })),
    );
    Ok(Json(UpdateTriggerResponse {
        trigger: response(&state, updated.trigger),
        delivery_signing_secret: updated.delivery_signing_secret,
    }))
}

pub async fn delete_trigger(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<DeleteTriggerResponse>> {
    let current =
        trigger_service::ensure_actor_can_write(&state.db, &auth_user.user_id.to_string(), &id)
            .await?;
    trigger_service::delete(&state.db, &current).await?;
    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "trigger_deleted",
        Some(serde_json::json!({ "trigger_id": id })),
    );
    Ok(Json(DeleteTriggerResponse {
        message: "Trigger deleted",
    }))
}

pub async fn rotate_trigger_secret(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<RotateTriggerSecretResponse>> {
    let current =
        trigger_service::ensure_actor_can_write(&state.db, &auth_user.user_id.to_string(), &id)
            .await?;
    let (trigger, secret) =
        trigger_service::rotate_secret(&state.db, &state.encryption_keys, &current).await?;
    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "trigger_secret_rotated",
        Some(serde_json::json!({
            "trigger_id": &trigger.id,
            "owner_user_id": &trigger.user_id,
        })),
    );
    Ok(Json(RotateTriggerSecretResponse {
        trigger: response(&state, trigger),
        secret,
    }))
}

async fn resolve_target_owner(
    state: &AppState,
    actor: &str,
    target: Option<&str>,
    require_write: bool,
) -> AppResult<String> {
    let Some(target) = target else {
        return Ok(actor.to_string());
    };
    let access = org_service::resolve_owner_access(&state.db, actor, target).await?;
    if (require_write && !access.can_write()) || (!require_write && !access.can_read()) {
        return Err(AppError::TriggerNotFound);
    }
    Ok(target.to_string())
}

fn response(state: &AppState, trigger: Trigger) -> TriggerResponse {
    TriggerResponse {
        id: trigger.id.clone(),
        user_id: trigger.user_id,
        label: trigger.label,
        user_service_id: trigger.user_service_id,
        status: trigger.status,
        verification: trigger.verification,
        delivery: trigger.delivery,
        inbound_url: format!(
            "{}/api/v1/webhooks/triggers/{}",
            state.config.base_url.trim_end_matches('/'),
            trigger.id
        ),
        created_at: trigger.created_at.to_rfc3339(),
        updated_at: trigger.updated_at.to_rfc3339(),
    }
}

fn delivery_name(delivery: &TriggerDelivery) -> &'static str {
    match delivery {
        TriggerDelivery::Webhook { .. } => "webhook",
        TriggerDelivery::Agent { .. } => "agent",
        TriggerDelivery::Notification => "notification",
    }
}

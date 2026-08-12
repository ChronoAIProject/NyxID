use axum::{
    Json,
    extract::{Path, Query, State},
};
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::models::trigger::{Trigger, TriggerDelivery, TriggerStatus, TriggerVerification};
use crate::models::trigger_delivery::{TriggerDeliveryRecord, TriggerDeliveryRecordStatus};
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
    pub delivery_signing_key_id: Option<String>,
    pub inbound_url: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Serialize)]
pub struct CreateTriggerResponse {
    pub trigger: TriggerResponse,
    pub secret: String,
    pub delivery_signing_secret: Option<String>,
    pub delivery_signing_key_id: Option<String>,
}

#[derive(Serialize)]
pub struct UpdateTriggerResponse {
    pub trigger: TriggerResponse,
    pub delivery_signing_secret: Option<String>,
    pub delivery_signing_key_id: Option<String>,
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
pub struct RotateTriggerDeliverySecretResponse {
    pub trigger: TriggerResponse,
    pub delivery_signing_secret: String,
    pub key_id: String,
}

#[derive(Debug, Deserialize)]
pub struct ListTriggerDeliveriesQuery {
    #[serde(default = "default_page")]
    pub page: u64,
    #[serde(default = "default_per_page")]
    pub per_page: u64,
}

#[derive(Serialize)]
pub struct TriggerDeliveryResponse {
    pub event_id: String,
    pub status: TriggerDeliveryRecordStatus,
    pub attempts: u32,
    pub last_status_code: Option<u16>,
    pub replay_available: bool,
    pub created_at: String,
    pub updated_at: String,
    pub delivered_at: Option<String>,
}

#[derive(Serialize)]
pub struct ListTriggerDeliveriesResponse {
    pub deliveries: Vec<TriggerDeliveryResponse>,
    pub page: u64,
    pub per_page: u64,
    pub total: u64,
}

#[derive(Serialize)]
pub struct RedeliverTriggerResponse {
    pub delivery: TriggerDeliveryResponse,
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
        delivery_signing_key_id: created.delivery_signing_key_id,
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
        delivery_signing_key_id: updated.delivery_signing_key_id,
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

pub async fn rotate_trigger_delivery_secret(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<RotateTriggerDeliverySecretResponse>> {
    let current =
        trigger_service::ensure_actor_can_write(&state.db, &auth_user.user_id.to_string(), &id)
            .await?;
    let rotated =
        trigger_service::rotate_delivery_secret(&state.db, &state.encryption_keys, &current)
            .await?;
    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "trigger_delivery_secret_rotated",
        Some(serde_json::json!({
            "trigger_id": &rotated.trigger.id,
            "owner_user_id": &rotated.trigger.user_id,
            "key_id": &rotated.key_id,
        })),
    );
    Ok(Json(RotateTriggerDeliverySecretResponse {
        trigger: response(&state, rotated.trigger),
        delivery_signing_secret: rotated.raw_secret,
        key_id: rotated.key_id,
    }))
}

pub async fn list_trigger_deliveries(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(id): Path<String>,
    Query(query): Query<ListTriggerDeliveriesQuery>,
) -> AppResult<Json<ListTriggerDeliveriesResponse>> {
    trigger_service::get_for_actor(&state.db, &auth_user.user_id.to_string(), &id).await?;
    let page = query.page.max(1);
    let per_page = query.per_page.clamp(1, 100);
    let (deliveries, total) =
        trigger_service::list_deliveries(&state.db, &id, page, per_page).await?;
    Ok(Json(ListTriggerDeliveriesResponse {
        deliveries: deliveries.into_iter().map(delivery_response).collect(),
        page,
        per_page,
        total,
    }))
}

pub async fn redeliver_trigger_delivery(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path((id, event_id)): Path<(String, String)>,
) -> AppResult<Json<RedeliverTriggerResponse>> {
    let trigger =
        trigger_service::ensure_actor_can_write(&state.db, &auth_user.user_id.to_string(), &id)
            .await?;
    let delivery = trigger_service::redeliver(
        &state.db,
        &state.encryption_keys,
        &state.http_client,
        &trigger,
        &event_id,
        state
            .config
            .trigger_payload_max_bytes
            .saturating_add(trigger_service::TRIGGER_ENVELOPE_OVERHEAD_BYTES),
    )
    .await?;
    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "trigger_delivery_redelivered",
        Some(serde_json::json!({
            "trigger_id": &id,
            "event_id": &event_id,
            "status": delivery.status,
        })),
    );
    Ok(Json(RedeliverTriggerResponse {
        delivery: delivery_response(delivery),
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
        delivery_signing_key_id: trigger.delivery_key_id,
        inbound_url: format!(
            "{}/api/v1/webhooks/triggers/{}",
            state.config.base_url.trim_end_matches('/'),
            trigger.id
        ),
        created_at: trigger.created_at.to_rfc3339(),
        updated_at: trigger.updated_at.to_rfc3339(),
    }
}

fn delivery_response(record: TriggerDeliveryRecord) -> TriggerDeliveryResponse {
    TriggerDeliveryResponse {
        event_id: record.event_id,
        status: record.status,
        attempts: record.attempts,
        last_status_code: record.last_status_code,
        replay_available: record.envelope_encrypted.is_some(),
        created_at: record.created_at.to_rfc3339(),
        updated_at: record.updated_at.to_rfc3339(),
        delivered_at: record.delivered_at.map(|value| value.to_rfc3339()),
    }
}

const fn default_page() -> u64 {
    1
}

const fn default_per_page() -> u64 {
    20
}

fn delivery_name(delivery: &TriggerDelivery) -> &'static str {
    match delivery {
        TriggerDelivery::Webhook { .. } => "webhook",
        TriggerDelivery::Agent { .. } => "agent",
        TriggerDelivery::Notification => "notification",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use uuid::Uuid;

    use crate::crypto::token::hash_token;
    use crate::models::trigger::{COLLECTION_NAME as TRIGGERS, TriggerTokenLocation};
    use crate::models::trigger_delivery::{
        COLLECTION_NAME as TRIGGER_DELIVERIES, TriggerDeliveryRecord,
    };
    use crate::test_utils::{connect_test_database, test_app_state, test_auth_user};

    #[tokio::test]
    async fn delivery_history_is_owner_scoped_and_metadata_only() {
        let Some(db) = connect_test_database("trigger_delivery_history_handler").await else {
            return;
        };
        let owner = Uuid::new_v4().to_string();
        let trigger_id = Uuid::new_v4().to_string();
        let now = Utc::now();
        db.collection::<Trigger>(TRIGGERS)
            .insert_one(Trigger {
                id: trigger_id.clone(),
                user_id: owner.clone(),
                label: "Inbound event".to_string(),
                user_service_id: None,
                status: TriggerStatus::Active,
                secret_hash: hash_token("nyx_trg_fixture"),
                verification: TriggerVerification::Token {
                    location: TriggerTokenLocation::Bearer,
                },
                verification_secret_encrypted: None,
                delivery: TriggerDelivery::Webhook {
                    url: "https://receiver.example.test/events".to_string(),
                },
                delivery_secret_encrypted: Some(vec![1, 2, 3]),
                delivery_key_id: Some("key_fixture".to_string()),
                created_at: now,
                updated_at: now,
            })
            .await
            .unwrap();
        db.collection::<TriggerDeliveryRecord>(TRIGGER_DELIVERIES)
            .insert_one(TriggerDeliveryRecord {
                id: "record-id".to_string(),
                trigger_id: trigger_id.clone(),
                user_id: owner.clone(),
                event_id: "event-id".to_string(),
                status: TriggerDeliveryRecordStatus::Failed,
                attempts: 1,
                last_status_code: Some(503),
                envelope_encrypted: Some(vec![4, 5, 6]),
                created_at: now,
                updated_at: now,
                expires_at: now + chrono::Duration::hours(72),
                delivered_at: None,
            })
            .await
            .unwrap();
        let state = test_app_state(db);

        let Json(response) = list_trigger_deliveries(
            State(state.clone()),
            test_auth_user(&owner),
            Path(trigger_id.clone()),
            Query(ListTriggerDeliveriesQuery {
                page: 1,
                per_page: 20,
            }),
        )
        .await
        .expect("owner can list deliveries");
        assert_eq!(response.total, 1);
        assert_eq!(response.deliveries[0].event_id, "event-id");
        assert!(response.deliveries[0].replay_available);
        let serialized = serde_json::to_value(&response.deliveries[0]).unwrap();
        assert!(serialized.get("envelope_encrypted").is_none());

        let unauthorized = list_trigger_deliveries(
            State(state),
            test_auth_user(&Uuid::new_v4().to_string()),
            Path(trigger_id),
            Query(ListTriggerDeliveriesQuery {
                page: 1,
                per_page: 20,
            }),
        )
        .await;
        assert!(matches!(unauthorized, Err(AppError::TriggerNotFound)));
    }
}

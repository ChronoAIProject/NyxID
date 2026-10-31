//! Human management API for independent CMAEG custom-event channels.

use axum::{
    Json,
    extract::{Path, State},
};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::models::nyxbot_event_binding::NyxbotEventBinding;
use crate::{AppState, errors::AppResult, mw::auth::AuthUser};

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateRequest {
    pub agent_id: String,
    pub destination_channel_id: String,
    pub destination_partition: String,
}

#[derive(Debug, Serialize)]
struct Item<'a> {
    id: &'a str,
    agent_id: &'a str,
    destination_channel_id: &'a str,
    destination_partition: &'a str,
    partition_policy: &'a str,
    status: &'a str,
    producer_key_id: &'a str,
    provider_binding_id: Option<&'a str>,
    gateway_channel_id: Option<&'a str>,
    custom_event_url: Option<&'a str>,
    last_error: Option<&'a str>,
    created_at: String,
    updated_at: String,
}

fn item(row: &NyxbotEventBinding) -> Item<'_> {
    Item {
        id: &row.id,
        agent_id: &row.agent_id,
        destination_channel_id: &row.destination_channel_id,
        destination_partition: &row.destination_partition,
        partition_policy: &row.partition_policy,
        status: &row.status,
        producer_key_id: &row.producer_key_id,
        provider_binding_id: row.provider_binding_id.as_deref(),
        gateway_channel_id: row.gateway_channel_id.as_deref(),
        custom_event_url: row.custom_event_url.as_deref(),
        last_error: row.last_error.as_deref(),
        created_at: row.created_at.to_rfc3339(),
        updated_at: row.updated_at.to_rfc3339(),
    }
}

fn owner(auth: &AuthUser) -> AppResult<String> {
    super::login_client_context::require_first_party_human(auth)?;
    Ok(auth.user_id.to_string())
}

pub async fn list(
    State(state): State<AppState>,
    auth: AuthUser,
) -> AppResult<Json<serde_json::Value>> {
    let owner = owner(&auth)?;
    let rows = crate::services::nyxbot_event_binding_service::list(&state, &owner).await?;
    Ok(Json(
        json!({"items": rows.iter().map(item).collect::<Vec<_>>() }),
    ))
}

pub async fn get(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    let owner = owner(&auth)?;
    let row = crate::services::nyxbot_event_binding_service::get(&state, &owner, &id).await?;
    Ok(Json(json!({"binding": item(&row)})))
}

pub async fn create(
    State(state): State<AppState>,
    auth: AuthUser,
    Json(body): Json<CreateRequest>,
) -> AppResult<Json<serde_json::Value>> {
    let owner = owner(&auth)?;
    let (row, secret, record_id) = crate::services::nyxbot_event_binding_service::create(
        &state,
        &owner,
        &body.agent_id,
        &body.destination_channel_id,
        &body.destination_partition,
    )
    .await?;
    // This is the only response that contains the producer credential. The
    // provider bearer and execution key are never serialized or returned.
    Ok(Json(json!({
        "binding": item(&row),
        "record_id": record_id,
        "producer": {"key_id": row.producer_key_id, "secret": secret.as_str()},
    })))
}

pub async fn revoke(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
) -> AppResult<Json<serde_json::Value>> {
    let owner = owner(&auth)?;
    crate::services::nyxbot_event_binding_service::revoke(&state, &owner, &id).await?;
    Ok(Json(json!({"status":"revoked", "id": id})))
}

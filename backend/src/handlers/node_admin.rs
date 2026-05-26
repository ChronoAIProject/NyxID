use std::collections::HashMap;

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
    response::IntoResponse,
};
use futures::TryStreamExt;
use mongodb::bson::doc;
use serde::{Deserialize, Serialize};

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::models::downstream_service::{
    COLLECTION_NAME as DOWNSTREAM_SERVICES, DownstreamService,
};
use crate::models::node::NodeMetadata;
use crate::models::node_pending_credential::InjectionMethod;
use crate::mw::auth::AuthUser;
use crate::services::{
    audit_service, node_pending_credential_service, node_routing_service, node_service, org_service,
};
use crate::telemetry::{
    context::{TelemetryContext, emit_event},
    sampling::hash_short_id,
    schema::TelemetryEvent,
};

// NodeCredentialConfigured is emitted from the nyxid CLI, not backend -- see TELEMETRY.md §6.5

// --- Request types ---

#[derive(Debug, Deserialize)]
pub struct CreateRegistrationTokenRequest {
    pub name: String,
    pub owner_user_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct CreateBindingRequest {
    pub service_id: String,
}

#[derive(Debug, Deserialize)]
pub struct UpdateBindingRequest {
    pub priority: Option<i32>,
}

#[derive(Debug, Deserialize)]
pub struct TransferNodeRequest {
    pub new_owner_user_id: String,
}

#[derive(Debug, Deserialize)]
pub struct PushPendingCredentialRequest {
    pub service_slug: String,
    pub injection_method: InjectionMethod,
    pub field_name: String,
    pub target_url: Option<String>,
    pub label: Option<String>,
}

#[derive(Debug, Deserialize, Default)]
pub struct PendingCredentialListQuery {
    pub include_history: Option<bool>,
}

// --- Response types ---

#[derive(Debug, Serialize)]
pub struct CreateRegistrationTokenResponse {
    pub token_id: String,
    pub token: String,
    pub name: String,
    pub expires_at: String,
}

#[derive(Debug, Serialize)]
pub struct NodeListResponse {
    pub nodes: Vec<NodeInfo>,
}

#[derive(Debug, Serialize)]
pub struct NodeMetricsInfo {
    pub total_requests: u64,
    pub success_count: u64,
    pub error_count: u64,
    pub success_rate: f64,
    pub avg_latency_ms: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_error_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_success_at: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct NodeInfo {
    pub id: String,
    pub name: String,
    pub owner: node_service::NodeOwnerInfo,
    pub status: String,
    pub is_connected: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_heartbeat_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connected_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<NodeMetadata>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metrics: Option<NodeMetricsInfo>,
    pub binding_count: u64,
    pub created_at: String,
}

#[derive(Debug, Serialize)]
pub struct RotateTokenResponse {
    pub auth_token: String,
    pub signing_secret: String,
    pub message: String,
}

#[derive(Debug, Serialize)]
pub struct BindingListResponse {
    pub bindings: Vec<BindingInfo>,
}

#[derive(Debug, Serialize)]
pub struct BindingInfo {
    pub id: String,
    pub service_id: String,
    pub service_name: String,
    pub service_slug: String,
    pub is_active: bool,
    pub priority: i32,
    pub created_at: String,
}

#[derive(Debug, Serialize)]
pub struct CreateBindingResponse {
    pub id: String,
    pub service_id: String,
    pub service_name: String,
    pub message: String,
}

#[derive(Debug, Serialize)]
pub struct NodeAdminsResponse {
    pub admins: Vec<node_service::NodeAdminInfo>,
}

#[derive(Debug, Serialize)]
pub struct TransferNodeResponse {
    pub node_id: String,
    pub previous_owner: node_service::NodeOwnerInfo,
    pub new_owner: node_service::NodeOwnerInfo,
    pub deactivated_bindings_count: u64,
    pub cleared_user_service_count: u64,
}

#[derive(Debug, Serialize)]
pub struct PendingCredentialInfo {
    pub id: String,
    pub node_id: String,
    pub service_slug: String,
    pub injection_method: String,
    pub field_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub created_by_user_id: String,
    pub owner_user_id: String,
    pub created_at: String,
    pub expires_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub consumed_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub declined_at: Option<String>,
    pub is_active: bool,
}

#[derive(Debug, Serialize)]
pub struct PendingCredentialListResponse {
    pub pending_credentials: Vec<PendingCredentialInfo>,
}

// --- Helpers ---

/// Build NodeMetricsInfo from the embedded metrics on a Node model.
pub fn build_metrics_info(metrics: &crate::models::node::NodeMetrics) -> NodeMetricsInfo {
    let success_rate = if metrics.total_requests > 0 {
        metrics.success_count as f64 / metrics.total_requests as f64
    } else {
        0.0
    };

    NodeMetricsInfo {
        total_requests: metrics.total_requests,
        success_count: metrics.success_count,
        error_count: metrics.error_count,
        success_rate,
        avg_latency_ms: metrics.avg_latency_ms,
        last_error: metrics.last_error.clone(),
        last_error_at: metrics.last_error_at.map(|dt| dt.to_rfc3339()),
        last_success_at: metrics.last_success_at.map(|dt| dt.to_rfc3339()),
    }
}

fn audit_event_data_with_owner(
    actor_user_id: &str,
    owner_user_id: &str,
    mut event_data: serde_json::Value,
) -> serde_json::Value {
    if actor_user_id != owner_user_id
        && let serde_json::Value::Object(ref mut object) = event_data
    {
        object.insert(
            "owner_user_id".to_string(),
            serde_json::Value::String(owner_user_id.to_string()),
        );
    }
    event_data
}

fn pending_credential_info(
    pending: crate::models::node_pending_credential::NodePendingCredential,
) -> PendingCredentialInfo {
    PendingCredentialInfo {
        id: pending.id,
        node_id: pending.node_id,
        service_slug: pending.service_slug,
        injection_method: pending.injection_method.as_str().to_string(),
        field_name: pending.field_name,
        target_url: pending.target_url,
        label: pending.label,
        created_by_user_id: pending.created_by_user_id,
        owner_user_id: pending.owner_user_id,
        created_at: pending.created_at.to_rfc3339(),
        expires_at: pending.expires_at.to_rfc3339(),
        consumed_at: pending.consumed_at.map(|dt| dt.to_rfc3339()),
        declined_at: pending.declined_at.map(|dt| dt.to_rfc3339()),
        is_active: pending.is_active,
    }
}

// --- Handlers ---

/// POST /api/v1/nodes/register-token
pub async fn create_registration_token(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<CreateRegistrationTokenRequest>,
) -> AppResult<Json<CreateRegistrationTokenResponse>> {
    let user_id_str = auth_user.user_id.to_string();
    let owner_user_id = body.owner_user_id.as_deref().unwrap_or(&user_id_str);

    if let Some(requested_owner) = body.owner_user_id.as_deref() {
        let access =
            org_service::resolve_owner_access(&state.db, &user_id_str, requested_owner).await?;
        if !matches!(
            access,
            org_service::OwnerAccess::Direct | org_service::OwnerAccess::AsOrgAdmin { .. }
        ) {
            return Err(AppError::Forbidden(
                "Only org admins can create registration tokens for that owner".to_string(),
            ));
        }
    }

    let (token_id, raw_token, expires_at): (String, String, chrono::DateTime<chrono::Utc>) =
        node_service::create_registration_token(
            &state.db,
            owner_user_id,
            &body.name,
            state.config.node_max_per_user,
            state.config.node_registration_token_ttl_secs,
        )
        .await?;
    let owner_differs = owner_user_id != user_id_str;
    let owner_user_id_for_audit = owner_user_id.to_string();
    let event_data = if owner_differs {
        serde_json::json!({
            "token_id": &token_id,
            "name": &body.name,
            "owner_user_id": &owner_user_id_for_audit,
        })
    } else {
        serde_json::json!({
            "token_id": &token_id,
            "name": &body.name,
        })
    };

    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "node_registration_token_created",
        Some(event_data),
    );

    Ok(Json(CreateRegistrationTokenResponse {
        token_id,
        token: raw_token,
        name: body.name,
        expires_at: expires_at.to_rfc3339(),
    }))
}

/// GET /api/v1/nodes
pub async fn list_nodes(
    State(state): State<AppState>,
    auth_user: AuthUser,
) -> AppResult<Json<NodeListResponse>> {
    let user_id_str = auth_user.user_id.to_string();
    let nodes = node_service::list_user_nodes(&state.db, &user_id_str).await?;

    // Batch-fetch binding counts in a single aggregation instead of N+1 queries
    let binding_counts: HashMap<String, u64> = if nodes.is_empty() {
        HashMap::new()
    } else {
        let node_id_array: bson::Array = nodes
            .iter()
            .map(|n| bson::Bson::String(n.node.id.clone()))
            .collect();
        let pipeline = vec![
            doc! { "$match": { "node_id": { "$in": node_id_array }, "is_active": true } },
            doc! { "$group": { "_id": "$node_id", "count": { "$sum": 1 } } },
        ];
        let mut cursor = state
            .db
            .collection::<mongodb::bson::Document>("node_service_bindings")
            .aggregate(pipeline)
            .await?;
        let mut counts = HashMap::new();
        while let Some(result) = cursor.try_next().await? {
            if let Ok(node_id) = result.get_str("_id") {
                // $sum may return Int32 or Int64 depending on value size
                let count = result
                    .get("count")
                    .and_then(|v| match v {
                        bson::Bson::Int32(n) => Some(*n as u64),
                        bson::Bson::Int64(n) => Some(*n as u64),
                        _ => None,
                    })
                    .unwrap_or(0);
                counts.insert(node_id.to_string(), count);
            }
        }
        counts
    };

    let node_infos: Vec<NodeInfo> = nodes
        .iter()
        .map(|node_with_owner| {
            let node = &node_with_owner.node;
            NodeInfo {
                id: node.id.clone(),
                name: node.name.clone(),
                owner: node_with_owner.owner.clone(),
                status: node.status.as_str().to_string(),
                is_connected: state.node_ws_manager.is_connected(&node.id),
                last_heartbeat_at: node.last_heartbeat_at.map(|dt| dt.to_rfc3339()),
                connected_at: node.connected_at.map(|dt| dt.to_rfc3339()),
                metadata: node.metadata.clone(),
                metrics: Some(build_metrics_info(&node.metrics)),
                binding_count: binding_counts.get(&node.id).copied().unwrap_or(0),
                created_at: node.created_at.to_rfc3339(),
            }
        })
        .collect();

    Ok(Json(NodeListResponse { nodes: node_infos }))
}

/// GET /api/v1/nodes/{node_id}
pub async fn get_node(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(node_id): Path<String>,
) -> AppResult<Json<NodeInfo>> {
    let user_id_str = auth_user.user_id.to_string();
    let node = node_service::get_node(&state.db, &user_id_str, &node_id).await?;
    let owner = node_service::owner_info_for_node(&state.db, &node).await?;

    let binding_count = state
        .db
        .collection::<mongodb::bson::Document>("node_service_bindings")
        .count_documents(doc! { "node_id": &node.id, "is_active": true })
        .await?;

    Ok(Json(NodeInfo {
        id: node.id.clone(),
        name: node.name,
        owner,
        status: node.status.as_str().to_string(),
        is_connected: state.node_ws_manager.is_connected(&node.id),
        last_heartbeat_at: node.last_heartbeat_at.map(|dt| dt.to_rfc3339()),
        connected_at: node.connected_at.map(|dt| dt.to_rfc3339()),
        metadata: node.metadata,
        metrics: Some(build_metrics_info(&node.metrics)),
        binding_count,
        created_at: node.created_at.to_rfc3339(),
    }))
}

/// DELETE /api/v1/nodes/{node_id}
pub async fn delete_node(
    State(state): State<AppState>,
    auth_user: AuthUser,
    tele: TelemetryContext,
    Path(node_id): Path<String>,
) -> AppResult<impl IntoResponse> {
    let user_id_str = auth_user.user_id.to_string();
    let node = node_service::get_node(&state.db, &user_id_str, &node_id).await?;

    node_service::delete_node(&state.db, &user_id_str, &node_id).await?;

    // Disconnect WebSocket if connected
    if state.node_ws_manager.is_connected(&node_id) {
        state
            .node_ws_manager
            .disconnect_connection(&node_id, 4006, "node deleted")
            .await;
    }

    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "node_deleted",
        Some(audit_event_data_with_owner(
            &user_id_str,
            &node.user_id,
            serde_json::json!({ "node_id": &node_id }),
        )),
    );

    emit_event(
        state.telemetry.as_deref(),
        &user_id_str,
        auth_user.api_key_id.as_deref(),
        &tele,
        TelemetryEvent::NodeDeleted {
            // Raw UUID would be scrubbed to `[UUID_REDACTED]`; hash keeps
            // per-node granularity without leaking the UUID.
            node_id: hash_short_id(&node_id),
        },
    );

    Ok(StatusCode::NO_CONTENT)
}

/// POST /api/v1/nodes/{node_id}/rotate-token
pub async fn rotate_token(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(node_id): Path<String>,
) -> AppResult<Json<RotateTokenResponse>> {
    let user_id_str = auth_user.user_id.to_string();
    let node = node_service::get_node(&state.db, &user_id_str, &node_id).await?;

    let (raw_token, raw_signing_secret) =
        node_service::rotate_auth_token(&state.db, &state.encryption_keys, &user_id_str, &node_id)
            .await?;

    // Disconnect the node since its old token is now invalid
    if state.node_ws_manager.is_connected(&node_id) {
        state
            .node_ws_manager
            .disconnect_connection(&node_id, 4002, "node credentials rotated")
            .await;
        node_service::set_node_status(
            &state.db,
            &node_id,
            crate::models::node::NodeStatus::Offline,
        )
        .await?;
    }

    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "node_token_rotated",
        Some(audit_event_data_with_owner(
            &user_id_str,
            &node.user_id,
            serde_json::json!({ "node_id": &node_id }),
        )),
    );

    Ok(Json(RotateTokenResponse {
        auth_token: raw_token,
        signing_secret: raw_signing_secret,
        message:
            "Auth token and signing secret rotated. The node must reconnect with the new token."
                .to_string(),
    }))
}

/// GET /api/v1/nodes/{node_id}/admins
pub async fn list_admins(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(node_id): Path<String>,
) -> AppResult<Json<NodeAdminsResponse>> {
    let user_id_str = auth_user.user_id.to_string();
    let admins = node_service::list_node_admins(&state.db, &user_id_str, &node_id).await?;

    Ok(Json(NodeAdminsResponse { admins }))
}

/// POST /api/v1/nodes/{node_id}/transfer
pub async fn transfer_node(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(node_id): Path<String>,
    Json(body): Json<TransferNodeRequest>,
) -> AppResult<Json<TransferNodeResponse>> {
    let user_id_str = auth_user.user_id.to_string();
    let node = node_service::get_node(&state.db, &user_id_str, &node_id).await?;
    let previous_owner = node_service::owner_info_for_node(&state.db, &node).await?;

    let result = node_service::transfer_node_owner(
        &state.db,
        &user_id_str,
        &node_id,
        &body.new_owner_user_id,
        state.config.node_max_per_user,
    )
    .await?;

    let mut transferred_node = node.clone();
    transferred_node.user_id = result.new_owner_user_id.clone();
    let new_owner = node_service::owner_info_for_node(&state.db, &transferred_node).await?;

    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "node_transferred",
        Some(audit_event_data_with_owner(
            &user_id_str,
            &result.new_owner_user_id,
            serde_json::json!({
                "actor_user_id": &user_id_str,
                "node_id": &result.node_id,
                "previous_owner_user_id": &result.previous_owner_user_id,
                "new_owner_user_id": &result.new_owner_user_id,
                "deactivated_bindings_count": result.deactivated_bindings_count,
                "cleared_user_service_count": result.cleared_user_service_count,
                "deactivated_pending_credentials_count": result.deactivated_pending_credentials_count,
            }),
        )),
    );

    Ok(Json(TransferNodeResponse {
        node_id: result.node_id,
        previous_owner,
        new_owner,
        deactivated_bindings_count: result.deactivated_bindings_count,
        cleared_user_service_count: result.cleared_user_service_count,
    }))
}

/// POST /api/v1/nodes/{node_id}/credentials/push
pub async fn push_pending_credential(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(node_id): Path<String>,
    Json(body): Json<PushPendingCredentialRequest>,
) -> AppResult<Json<PendingCredentialInfo>> {
    let user_id_str = auth_user.user_id.to_string();
    let pending = node_pending_credential_service::create_pending_credential(
        &state.db,
        &user_id_str,
        &node_id,
        node_pending_credential_service::CreatePendingCredentialInput {
            service_slug: body.service_slug,
            injection_method: body.injection_method,
            field_name: body.field_name,
            target_url: body.target_url,
            label: body.label,
            ttl_secs: state.config.node_pending_credential_ttl_secs,
        },
    )
    .await?;

    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "node_credential_push_created",
        Some(serde_json::json!({
            "node_id": &pending.node_id,
            "service_slug": &pending.service_slug,
            "injection_method": pending.injection_method.as_str(),
            "owner_user_id": &pending.owner_user_id,
        })),
    );

    if state.node_ws_manager.is_connected(&node_id)
        && let Err(err) = state
            .node_ws_manager
            .send_pending_credentials_available(&node_id)
    {
        tracing::warn!(
            node_id = %node_id,
            error = %err,
            "Failed to nudge node about pending credential"
        );
    }

    Ok(Json(pending_credential_info(pending)))
}

/// GET /api/v1/nodes/{node_id}/credentials/pending
pub async fn list_pending_credentials(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(node_id): Path<String>,
    Query(query): Query<PendingCredentialListQuery>,
) -> AppResult<Json<PendingCredentialListResponse>> {
    let user_id_str = auth_user.user_id.to_string();
    let pending = node_pending_credential_service::list_pending_credentials_for_admin(
        &state.db,
        &user_id_str,
        &node_id,
        query.include_history.unwrap_or(false),
    )
    .await?;

    Ok(Json(PendingCredentialListResponse {
        pending_credentials: pending.into_iter().map(pending_credential_info).collect(),
    }))
}

/// DELETE /api/v1/nodes/{node_id}/credentials/pending/{pending_id}
pub async fn cancel_pending_credential(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path((node_id, pending_id)): Path<(String, String)>,
) -> AppResult<impl IntoResponse> {
    let user_id_str = auth_user.user_id.to_string();
    let pending = node_pending_credential_service::cancel_pending_credential(
        &state.db,
        &user_id_str,
        &node_id,
        &pending_id,
    )
    .await?;

    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "node_credential_push_canceled",
        Some(serde_json::json!({
            "node_id": &pending.node_id,
            "pending_credential_id": &pending.id,
            "service_slug": &pending.service_slug,
            "owner_user_id": &pending.owner_user_id,
        })),
    );

    Ok(StatusCode::NO_CONTENT)
}

/// GET /api/v1/nodes/{node_id}/bindings
pub async fn list_bindings(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(node_id): Path<String>,
) -> AppResult<Json<BindingListResponse>> {
    let user_id_str = auth_user.user_id.to_string();
    let bindings = node_service::list_bindings(&state.db, &user_id_str, &node_id).await?;

    // M3: Batch-fetch all referenced services in a single query instead of N+1
    let service_id_array: bson::Array = bindings
        .iter()
        .map(|b| bson::Bson::String(b.service_id.clone()))
        .collect();

    let services: Vec<DownstreamService> = if service_id_array.is_empty() {
        vec![]
    } else {
        state
            .db
            .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
            .find(doc! { "_id": { "$in": service_id_array } })
            .await?
            .try_collect()
            .await?
    };

    let service_map: HashMap<&str, &DownstreamService> =
        services.iter().map(|s| (s.id.as_str(), s)).collect();

    let binding_infos: Vec<BindingInfo> = bindings
        .iter()
        .map(|binding| {
            let (service_name, service_slug) = match service_map.get(binding.service_id.as_str()) {
                Some(s) => (s.name.clone(), s.slug.clone()),
                None => ("Unknown".to_string(), "unknown".to_string()),
            };

            BindingInfo {
                id: binding.id.clone(),
                service_id: binding.service_id.clone(),
                service_name,
                service_slug,
                is_active: binding.is_active,
                priority: binding.priority,
                created_at: binding.created_at.to_rfc3339(),
            }
        })
        .collect();

    Ok(Json(BindingListResponse {
        bindings: binding_infos,
    }))
}

/// POST /api/v1/nodes/{node_id}/bindings
pub async fn create_binding(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(node_id): Path<String>,
    Json(body): Json<CreateBindingRequest>,
) -> AppResult<Json<CreateBindingResponse>> {
    let user_id_str = auth_user.user_id.to_string();

    // Verify the service exists
    let service = state
        .db
        .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .find_one(doc! { "_id": &body.service_id, "is_active": true })
        .await?
        .ok_or_else(|| AppError::NotFound("Service not found".to_string()))?;

    let binding =
        node_service::create_binding(&state.db, &user_id_str, &node_id, &body.service_id).await?;

    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "node_binding_created",
        Some(audit_event_data_with_owner(
            &user_id_str,
            &binding.user_id,
            serde_json::json!({
                "binding_id": &binding.id,
                "node_id": &node_id,
                "service_id": &body.service_id,
            }),
        )),
    );

    Ok(Json(CreateBindingResponse {
        id: binding.id,
        service_id: body.service_id,
        service_name: service.name,
        message: "Service binding created".to_string(),
    }))
}

/// PATCH /api/v1/nodes/{node_id}/bindings/{binding_id}
pub async fn update_binding(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path((node_id, binding_id)): Path<(String, String)>,
    Json(body): Json<UpdateBindingRequest>,
) -> AppResult<impl IntoResponse> {
    let user_id_str = auth_user.user_id.to_string();
    let node = node_service::get_node(&state.db, &user_id_str, &node_id).await?;

    if let Some(priority) = body.priority {
        node_service::update_binding_priority(
            &state.db,
            &user_id_str,
            &node_id,
            &binding_id,
            priority,
        )
        .await?;
    }

    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "node_binding_updated",
        Some(audit_event_data_with_owner(
            &user_id_str,
            &node.user_id,
            serde_json::json!({
                "binding_id": &binding_id,
                "node_id": &node_id,
                "priority": body.priority,
            }),
        )),
    );

    Ok(StatusCode::NO_CONTENT)
}

/// DELETE /api/v1/nodes/{node_id}/bindings/{binding_id}
pub async fn delete_binding(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path((node_id, binding_id)): Path<(String, String)>,
) -> AppResult<impl IntoResponse> {
    let user_id_str = auth_user.user_id.to_string();
    let node = node_service::get_node(&state.db, &user_id_str, &node_id).await?;

    node_service::delete_binding(&state.db, &user_id_str, &node_id, &binding_id).await?;

    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "node_binding_deleted",
        Some(audit_event_data_with_owner(
            &user_id_str,
            &node.user_id,
            serde_json::json!({
                "binding_id": &binding_id,
                "node_id": &node_id,
            }),
        )),
    );

    Ok(StatusCode::NO_CONTENT)
}

// --- My Bindings ---

#[derive(Debug, Serialize)]
pub struct MyBoundServicesResponse {
    pub service_ids: Vec<String>,
}

/// GET /api/v1/nodes/my-bindings
///
/// List all service IDs for which the authenticated user currently has a viable node route.
pub async fn list_my_bound_services(
    State(state): State<AppState>,
    auth_user: AuthUser,
) -> AppResult<Json<MyBoundServicesResponse>> {
    let user_id_str = auth_user.user_id.to_string();
    let service_ids = node_routing_service::list_routable_service_ids(
        &state.db,
        &user_id_str,
        state.node_ws_manager.as_ref(),
    )
    .await?;

    Ok(Json(MyBoundServicesResponse { service_ids }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::audit_log::{AuditLog, COLLECTION_NAME as AUDIT_LOG};
    use crate::models::node::{COLLECTION_NAME as NODES, Node, NodeMetrics, NodeStatus};
    use crate::models::node_registration_token::{
        COLLECTION_NAME as NODE_REG_TOKENS, NodeRegistrationToken,
    };
    use crate::models::node_service_binding::{
        COLLECTION_NAME as NODE_SERVICE_BINDINGS, NodeServiceBinding,
    };
    use crate::models::org_membership::{
        COLLECTION_NAME as ORG_MEMBERSHIPS, OrgMembership, OrgRole,
    };
    use crate::models::user::{COLLECTION_NAME as USERS, User, UserType};
    use crate::models::user_service::{COLLECTION_NAME as USER_SERVICES, UserService};
    use crate::test_utils::{
        connect_test_database, test_app_state, test_auth_user, test_membership, test_user,
        test_user_service,
    };
    use chrono::Utc;
    use uuid::Uuid;

    fn test_node(owner_id: &str, name: &str) -> Node {
        let now = Utc::now();
        Node {
            id: Uuid::new_v4().to_string(),
            user_id: owner_id.to_string(),
            name: name.to_string(),
            status: NodeStatus::Offline,
            auth_token_hash: "auth-hash".to_string(),
            signing_secret_encrypted: None,
            signing_secret_hash: "signing-hash".to_string(),
            last_heartbeat_at: None,
            connected_at: None,
            metadata: None,
            metrics: NodeMetrics::default(),
            is_active: true,
            created_at: now,
            updated_at: now,
        }
    }

    fn test_binding(owner_id: &str, node_id: &str, service_id: &str) -> NodeServiceBinding {
        let now = Utc::now();
        NodeServiceBinding {
            id: Uuid::new_v4().to_string(),
            node_id: node_id.to_string(),
            user_id: owner_id.to_string(),
            service_id: service_id.to_string(),
            is_active: true,
            priority: 0,
            created_at: now,
            updated_at: now,
        }
    }

    async fn wait_for_transfer_audit(db: &mongodb::Database, node_id: &str) -> Option<AuditLog> {
        for _ in 0..100 {
            let found = db
                .collection::<AuditLog>(AUDIT_LOG)
                .find_one(doc! {
                    "event_type": "node_transferred",
                    "event_data.node_id": node_id,
                })
                .await
                .expect("query audit log");
            if found.is_some() {
                return found;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        None
    }

    async fn insert_users(db: &mongodb::Database, users: Vec<User>) {
        db.collection::<User>(USERS)
            .insert_many(users)
            .await
            .expect("insert users");
    }

    #[tokio::test]
    async fn create_registration_token_accepts_explicit_direct_owner_scope() {
        let Some(db) = connect_test_database("node_token_direct").await else {
            eprintln!("skipping node handler test: no local MongoDB available");
            return;
        };

        let actor_id = Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_one(test_user(&actor_id, UserType::Person))
            .await
            .expect("insert user");

        let state = test_app_state(db.clone());
        let Json(response) = create_registration_token(
            State(state),
            test_auth_user(&actor_id),
            Json(CreateRegistrationTokenRequest {
                name: "direct-node".to_string(),
                owner_user_id: Some(actor_id.clone()),
            }),
        )
        .await
        .expect("explicit direct owner should be allowed");

        let stored = db
            .collection::<NodeRegistrationToken>(NODE_REG_TOKENS)
            .find_one(doc! { "_id": &response.token_id })
            .await
            .expect("query token")
            .expect("token exists");
        assert_eq!(stored.user_id, actor_id);
        assert_eq!(stored.name, "direct-node");
    }

    #[tokio::test]
    async fn create_registration_token_accepts_org_admin_owner_scope() {
        let Some(db) = connect_test_database("node_token_org_admin").await else {
            eprintln!("skipping node handler test: no local MongoDB available");
            return;
        };

        let admin_id = Uuid::new_v4().to_string();
        let org_id = Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_many([
                test_user(&admin_id, UserType::Person),
                test_user(&org_id, UserType::Org),
            ])
            .await
            .expect("insert users");
        db.collection::<OrgMembership>(ORG_MEMBERSHIPS)
            .insert_one(test_membership(&org_id, &admin_id, OrgRole::Admin, None))
            .await
            .expect("insert membership");

        let state = test_app_state(db.clone());
        let Json(response) = create_registration_token(
            State(state),
            test_auth_user(&admin_id),
            Json(CreateRegistrationTokenRequest {
                name: "org-node".to_string(),
                owner_user_id: Some(org_id.clone()),
            }),
        )
        .await
        .expect("org admin can create owner-scoped token");

        let stored = db
            .collection::<NodeRegistrationToken>(NODE_REG_TOKENS)
            .find_one(doc! { "_id": &response.token_id })
            .await
            .expect("query token")
            .expect("token exists");
        assert_eq!(stored.user_id, org_id);
        assert_eq!(stored.name, "org-node");
    }

    #[tokio::test]
    async fn create_registration_token_rejects_non_admin_owner_scope() {
        let Some(db) = connect_test_database("node_token_non_admin").await else {
            eprintln!("skipping node handler test: no local MongoDB available");
            return;
        };

        let member_id = Uuid::new_v4().to_string();
        let org_id = Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_many([
                test_user(&member_id, UserType::Person),
                test_user(&org_id, UserType::Org),
            ])
            .await
            .expect("insert users");
        db.collection::<OrgMembership>(ORG_MEMBERSHIPS)
            .insert_one(test_membership(&org_id, &member_id, OrgRole::Member, None))
            .await
            .expect("insert membership");

        let state = test_app_state(db);
        let err = create_registration_token(
            State(state),
            test_auth_user(&member_id),
            Json(CreateRegistrationTokenRequest {
                name: "org-node".to_string(),
                owner_user_id: Some(org_id),
            }),
        )
        .await
        .expect_err("org member cannot create owner-scoped token");

        assert!(matches!(err, AppError::Forbidden(_)));
    }

    #[tokio::test]
    async fn create_registration_token_counts_nodes_against_requested_owner_not_actor() {
        let Some(db) = connect_test_database("node_token_owner_cap").await else {
            eprintln!("skipping node handler test: no local MongoDB available");
            return;
        };

        let admin_id = Uuid::new_v4().to_string();
        let org_id = Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_many([
                test_user(&admin_id, UserType::Person),
                test_user(&org_id, UserType::Org),
            ])
            .await
            .expect("insert users");
        db.collection::<OrgMembership>(ORG_MEMBERSHIPS)
            .insert_one(test_membership(&org_id, &admin_id, OrgRole::Admin, None))
            .await
            .expect("insert membership");

        let actor_nodes: Vec<Node> = (0..10)
            .map(|idx| test_node(&admin_id, &format!("actor-node-{idx}")))
            .collect();
        db.collection::<Node>(NODES)
            .insert_many(actor_nodes)
            .await
            .expect("insert actor nodes at personal cap");

        let state = test_app_state(db.clone());
        let Json(response) = create_registration_token(
            State(state),
            test_auth_user(&admin_id),
            Json(CreateRegistrationTokenRequest {
                name: "org-node".to_string(),
                owner_user_id: Some(org_id.clone()),
            }),
        )
        .await
        .expect("actor personal cap should not block org-owned token");

        let stored = db
            .collection::<NodeRegistrationToken>(NODE_REG_TOKENS)
            .find_one(doc! { "_id": &response.token_id })
            .await
            .expect("query token")
            .expect("token exists");
        assert_eq!(stored.user_id, org_id);
    }

    #[tokio::test]
    async fn list_nodes_returns_owner_metadata_for_personal_and_org_nodes() {
        let Some(db) = connect_test_database("node_list_owner_metadata").await else {
            eprintln!("skipping node handler test: no local MongoDB available");
            return;
        };

        let actor_id = Uuid::new_v4().to_string();
        let org_id = Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_many([
                test_user(&actor_id, UserType::Person),
                test_user(&org_id, UserType::Org),
            ])
            .await
            .expect("insert users");
        db.collection::<OrgMembership>(ORG_MEMBERSHIPS)
            .insert_one(test_membership(&org_id, &actor_id, OrgRole::Admin, None))
            .await
            .expect("insert membership");

        let personal_node = test_node(&actor_id, "personal-node");
        let org_node = test_node(&org_id, "org-node");
        db.collection::<Node>(NODES)
            .insert_many([personal_node.clone(), org_node.clone()])
            .await
            .expect("insert nodes");

        let state = test_app_state(db);
        let Json(response) = list_nodes(State(state), test_auth_user(&actor_id))
            .await
            .expect("list nodes");

        let personal = response
            .nodes
            .iter()
            .find(|node| node.id == personal_node.id)
            .expect("personal node listed");
        assert_eq!(personal.owner.kind, node_service::NodeOwnerKind::User);
        assert_eq!(personal.owner.id, actor_id);

        let org = response
            .nodes
            .iter()
            .find(|node| node.id == org_node.id)
            .expect("org node listed");
        assert_eq!(org.owner.kind, node_service::NodeOwnerKind::Org);
        assert_eq!(org.owner.id, org_id);
        assert_eq!(org.owner.display_name, "Test Org");
    }

    #[tokio::test]
    async fn get_node_returns_owner_metadata_for_org_member() {
        let Some(db) = connect_test_database("node_get_owner_metadata").await else {
            eprintln!("skipping node handler test: no local MongoDB available");
            return;
        };

        let member_id = Uuid::new_v4().to_string();
        let org_id = Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_many([
                test_user(&member_id, UserType::Person),
                test_user(&org_id, UserType::Org),
            ])
            .await
            .expect("insert users");
        db.collection::<OrgMembership>(ORG_MEMBERSHIPS)
            .insert_one(test_membership(&org_id, &member_id, OrgRole::Member, None))
            .await
            .expect("insert membership");
        let org_node = test_node(&org_id, "org-node");
        db.collection::<Node>(NODES)
            .insert_one(org_node.clone())
            .await
            .expect("insert node");

        let state = test_app_state(db);
        let Json(response) = get_node(
            State(state),
            test_auth_user(&member_id),
            Path(org_node.id.clone()),
        )
        .await
        .expect("get org node");

        assert_eq!(response.id, org_node.id);
        assert_eq!(response.owner.kind, node_service::NodeOwnerKind::Org);
        assert_eq!(response.owner.id, org_id);
    }

    #[tokio::test]
    async fn transfer_personal_node_to_admin_org_succeeds_and_detaches_old_routes() {
        let Some(db) = connect_test_database("node_transfer_personal_to_org").await else {
            eprintln!("skipping node handler test: no local MongoDB available");
            return;
        };

        let admin_id = Uuid::new_v4().to_string();
        let org_id = Uuid::new_v4().to_string();
        insert_users(
            &db,
            vec![
                test_user(&admin_id, UserType::Person),
                test_user(&org_id, UserType::Org),
            ],
        )
        .await;
        db.collection::<OrgMembership>(ORG_MEMBERSHIPS)
            .insert_one(test_membership(&org_id, &admin_id, OrgRole::Admin, None))
            .await
            .expect("insert membership");

        let node = test_node(&admin_id, "edge-node");
        let binding = test_binding(&admin_id, &node.id, "svc-old");
        let old_service = test_user_service(
            &Uuid::new_v4().to_string(),
            &admin_id,
            "old-service",
            &Uuid::new_v4().to_string(),
            Some("svc-old"),
            Some(&node.id),
        );
        let new_owner_service = test_user_service(
            &Uuid::new_v4().to_string(),
            &org_id,
            "org-service",
            &Uuid::new_v4().to_string(),
            Some("svc-org"),
            Some(&node.id),
        );
        db.collection::<Node>(NODES)
            .insert_one(node.clone())
            .await
            .expect("insert node");
        db.collection::<NodeServiceBinding>(NODE_SERVICE_BINDINGS)
            .insert_one(binding.clone())
            .await
            .expect("insert binding");
        db.collection::<UserService>(USER_SERVICES)
            .insert_many([old_service.clone(), new_owner_service.clone()])
            .await
            .expect("insert user services");

        let state = test_app_state(db.clone());
        let Json(response) = transfer_node(
            State(state),
            test_auth_user(&admin_id),
            Path(node.id.clone()),
            Json(TransferNodeRequest {
                new_owner_user_id: org_id.clone(),
            }),
        )
        .await
        .expect("transfer succeeds");

        assert_eq!(response.node_id, node.id);
        assert_eq!(response.previous_owner.id, admin_id);
        assert_eq!(response.new_owner.id, org_id);
        assert_eq!(response.deactivated_bindings_count, 1);
        assert_eq!(response.cleared_user_service_count, 1);

        let transferred = db
            .collection::<Node>(NODES)
            .find_one(doc! { "_id": &node.id })
            .await
            .expect("query node")
            .expect("node exists");
        assert_eq!(transferred.user_id, org_id);

        let updated_binding = db
            .collection::<NodeServiceBinding>(NODE_SERVICE_BINDINGS)
            .find_one(doc! { "_id": &binding.id })
            .await
            .expect("query binding")
            .expect("binding exists");
        assert!(!updated_binding.is_active);

        let old_service_after = db
            .collection::<UserService>(USER_SERVICES)
            .find_one(doc! { "_id": &old_service.id })
            .await
            .expect("query old service")
            .expect("old service exists");
        assert_eq!(old_service_after.node_id, None);

        let new_owner_service_after = db
            .collection::<UserService>(USER_SERVICES)
            .find_one(doc! { "_id": &new_owner_service.id })
            .await
            .expect("query new owner service")
            .expect("new owner service exists");
        assert_eq!(
            new_owner_service_after.node_id.as_deref(),
            Some(node.id.as_str())
        );

        let audit = wait_for_transfer_audit(&db, &node.id)
            .await
            .expect("transfer audit");
        let data = audit.event_data.expect("audit data");
        assert_eq!(
            data.get("previous_owner_user_id").and_then(|v| v.as_str()),
            Some(admin_id.as_str())
        );
        assert_eq!(
            data.get("new_owner_user_id").and_then(|v| v.as_str()),
            Some(org_id.as_str())
        );
        assert_eq!(
            data.get("deactivated_bindings_count")
                .and_then(|v| v.as_u64()),
            Some(1)
        );
        assert_eq!(
            data.get("cleared_user_service_count")
                .and_then(|v| v.as_u64()),
            Some(1)
        );
    }

    #[tokio::test]
    async fn transfer_org_node_between_administered_orgs_succeeds() {
        let Some(db) = connect_test_database("node_transfer_org_to_org").await else {
            eprintln!("skipping node handler test: no local MongoDB available");
            return;
        };

        let admin_id = Uuid::new_v4().to_string();
        let org_a_id = Uuid::new_v4().to_string();
        let org_b_id = Uuid::new_v4().to_string();
        insert_users(
            &db,
            vec![
                test_user(&admin_id, UserType::Person),
                test_user(&org_a_id, UserType::Org),
                test_user(&org_b_id, UserType::Org),
            ],
        )
        .await;
        db.collection::<OrgMembership>(ORG_MEMBERSHIPS)
            .insert_many([
                test_membership(&org_a_id, &admin_id, OrgRole::Admin, None),
                test_membership(&org_b_id, &admin_id, OrgRole::Admin, None),
            ])
            .await
            .expect("insert memberships");
        let node = test_node(&org_a_id, "shared-node");
        db.collection::<Node>(NODES)
            .insert_one(node.clone())
            .await
            .expect("insert node");

        let state = test_app_state(db.clone());
        let _ = transfer_node(
            State(state),
            test_auth_user(&admin_id),
            Path(node.id.clone()),
            Json(TransferNodeRequest {
                new_owner_user_id: org_b_id.clone(),
            }),
        )
        .await
        .expect("transfer succeeds");

        let transferred = db
            .collection::<Node>(NODES)
            .find_one(doc! { "_id": &node.id })
            .await
            .expect("query node")
            .expect("node exists");
        assert_eq!(transferred.user_id, org_b_id);
    }

    #[tokio::test]
    async fn transfer_org_node_by_member_returns_not_found() {
        let Some(db) = connect_test_database("node_transfer_member_denied").await else {
            eprintln!("skipping node handler test: no local MongoDB available");
            return;
        };

        let member_id = Uuid::new_v4().to_string();
        let org_id = Uuid::new_v4().to_string();
        insert_users(
            &db,
            vec![
                test_user(&member_id, UserType::Person),
                test_user(&org_id, UserType::Org),
            ],
        )
        .await;
        db.collection::<OrgMembership>(ORG_MEMBERSHIPS)
            .insert_one(test_membership(&org_id, &member_id, OrgRole::Member, None))
            .await
            .expect("insert membership");
        let node = test_node(&org_id, "member-denied-node");
        db.collection::<Node>(NODES)
            .insert_one(node.clone())
            .await
            .expect("insert node");

        let state = test_app_state(db);
        let err = transfer_node(
            State(state),
            test_auth_user(&member_id),
            Path(node.id),
            Json(TransferNodeRequest {
                new_owner_user_id: member_id,
            }),
        )
        .await
        .expect_err("member cannot transfer org node");

        assert!(matches!(err, AppError::NodeNotFound(_)));
    }

    #[tokio::test]
    async fn transfer_to_same_owner_returns_bad_request() {
        let Some(db) = connect_test_database("node_transfer_same_owner").await else {
            eprintln!("skipping node handler test: no local MongoDB available");
            return;
        };

        let owner_id = Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_one(test_user(&owner_id, UserType::Person))
            .await
            .expect("insert owner");
        let node = test_node(&owner_id, "same-owner-node");
        db.collection::<Node>(NODES)
            .insert_one(node.clone())
            .await
            .expect("insert node");

        let state = test_app_state(db);
        let err = transfer_node(
            State(state),
            test_auth_user(&owner_id),
            Path(node.id),
            Json(TransferNodeRequest {
                new_owner_user_id: owner_id,
            }),
        )
        .await
        .expect_err("same-owner transfer rejected");

        assert!(
            matches!(err, AppError::BadRequest(message) if message == "node already belongs to that owner")
        );
    }

    #[tokio::test]
    async fn transfer_name_collision_returns_explicit_bad_request() {
        let Some(db) = connect_test_database("node_transfer_name_collision").await else {
            eprintln!("skipping node handler test: no local MongoDB available");
            return;
        };

        let admin_id = Uuid::new_v4().to_string();
        let org_id = Uuid::new_v4().to_string();
        insert_users(
            &db,
            vec![
                test_user(&admin_id, UserType::Person),
                test_user(&org_id, UserType::Org),
            ],
        )
        .await;
        db.collection::<OrgMembership>(ORG_MEMBERSHIPS)
            .insert_one(test_membership(&org_id, &admin_id, OrgRole::Admin, None))
            .await
            .expect("insert membership");
        let source_node = test_node(&admin_id, "duplicate-node");
        let colliding_node = test_node(&org_id, "duplicate-node");
        db.collection::<Node>(NODES)
            .insert_many([source_node.clone(), colliding_node])
            .await
            .expect("insert nodes");

        let state = test_app_state(db);
        let err = transfer_node(
            State(state),
            test_auth_user(&admin_id),
            Path(source_node.id),
            Json(TransferNodeRequest {
                new_owner_user_id: org_id,
            }),
        )
        .await
        .expect_err("name collision rejected");

        assert!(
            matches!(err, AppError::BadRequest(message) if message.contains("An active node named 'duplicate-node' already exists for the destination owner"))
        );
    }

    #[tokio::test]
    async fn transfer_destination_at_node_cap_returns_bad_request() {
        let Some(db) = connect_test_database("node_transfer_cap").await else {
            eprintln!("skipping node handler test: no local MongoDB available");
            return;
        };

        let admin_id = Uuid::new_v4().to_string();
        let org_id = Uuid::new_v4().to_string();
        insert_users(
            &db,
            vec![
                test_user(&admin_id, UserType::Person),
                test_user(&org_id, UserType::Org),
            ],
        )
        .await;
        db.collection::<OrgMembership>(ORG_MEMBERSHIPS)
            .insert_one(test_membership(&org_id, &admin_id, OrgRole::Admin, None))
            .await
            .expect("insert membership");

        let source_node = test_node(&admin_id, "source-node");
        let mut nodes = vec![source_node.clone()];
        nodes.extend((0..10).map(|idx| test_node(&org_id, &format!("org-node-{idx}"))));
        db.collection::<Node>(NODES)
            .insert_many(nodes)
            .await
            .expect("insert nodes");

        let state = test_app_state(db);
        let err = transfer_node(
            State(state),
            test_auth_user(&admin_id),
            Path(source_node.id),
            Json(TransferNodeRequest {
                new_owner_user_id: org_id,
            }),
        )
        .await
        .expect_err("cap rejected");

        assert!(
            matches!(err, AppError::BadRequest(message) if message == "Maximum of 10 nodes per user reached")
        );
    }

    #[tokio::test]
    async fn transfer_updates_list_visibility_for_previous_and_new_owner_members() {
        let Some(db) = connect_test_database("node_transfer_list_visibility").await else {
            eprintln!("skipping node handler test: no local MongoDB available");
            return;
        };

        let admin_id = Uuid::new_v4().to_string();
        let org_a_member_id = Uuid::new_v4().to_string();
        let org_b_member_id = Uuid::new_v4().to_string();
        let org_a_id = Uuid::new_v4().to_string();
        let org_b_id = Uuid::new_v4().to_string();
        insert_users(
            &db,
            vec![
                test_user(&admin_id, UserType::Person),
                test_user(&org_a_member_id, UserType::Person),
                test_user(&org_b_member_id, UserType::Person),
                test_user(&org_a_id, UserType::Org),
                test_user(&org_b_id, UserType::Org),
            ],
        )
        .await;
        db.collection::<OrgMembership>(ORG_MEMBERSHIPS)
            .insert_many([
                test_membership(&org_a_id, &admin_id, OrgRole::Admin, None),
                test_membership(&org_b_id, &admin_id, OrgRole::Admin, None),
                test_membership(&org_a_id, &org_a_member_id, OrgRole::Member, None),
                test_membership(&org_b_id, &org_b_member_id, OrgRole::Member, None),
            ])
            .await
            .expect("insert memberships");
        let node = test_node(&org_a_id, "moving-node");
        db.collection::<Node>(NODES)
            .insert_one(node.clone())
            .await
            .expect("insert node");

        let state = test_app_state(db.clone());
        let _ = transfer_node(
            State(state.clone()),
            test_auth_user(&admin_id),
            Path(node.id.clone()),
            Json(TransferNodeRequest {
                new_owner_user_id: org_b_id,
            }),
        )
        .await
        .expect("transfer succeeds");

        let Json(previous_response) =
            list_nodes(State(state.clone()), test_auth_user(&org_a_member_id))
                .await
                .expect("previous owner member can list nodes");
        assert!(
            !previous_response
                .nodes
                .iter()
                .any(|item| item.id == node.id)
        );

        let Json(new_response) = list_nodes(State(state), test_auth_user(&org_b_member_id))
            .await
            .expect("new owner member can list nodes");
        assert!(new_response.nodes.iter().any(|item| item.id == node.id));
    }

    #[tokio::test]
    async fn transfer_orders_cleanup_before_ownership_flip() {
        let Some(db) = connect_test_database("node_transfer_cleanup_order").await else {
            eprintln!("skipping node handler test: no local MongoDB available");
            return;
        };

        let actor_id = Uuid::new_v4().to_string();
        let other_old_owner_id = Uuid::new_v4().to_string();
        let org_id = Uuid::new_v4().to_string();
        insert_users(
            &db,
            vec![
                test_user(&actor_id, UserType::Person),
                test_user(&other_old_owner_id, UserType::Person),
                test_user(&org_id, UserType::Org),
            ],
        )
        .await;
        db.collection::<OrgMembership>(ORG_MEMBERSHIPS)
            .insert_one(test_membership(&org_id, &actor_id, OrgRole::Admin, None))
            .await
            .expect("insert membership");

        let node = test_node(&actor_id, "edge-node");
        let binding = test_binding(&actor_id, &node.id, "svc-old");
        let actor_service = test_user_service(
            &Uuid::new_v4().to_string(),
            &actor_id,
            "actor-service",
            &Uuid::new_v4().to_string(),
            Some("svc-old"),
            Some(&node.id),
        );
        let orphaned_service = test_user_service(
            &Uuid::new_v4().to_string(),
            &other_old_owner_id,
            "orphaned-service",
            &Uuid::new_v4().to_string(),
            Some("svc-orphaned"),
            Some(&node.id),
        );
        let destination_service = test_user_service(
            &Uuid::new_v4().to_string(),
            &org_id,
            "destination-service",
            &Uuid::new_v4().to_string(),
            Some("svc-destination"),
            Some(&node.id),
        );
        db.collection::<Node>(NODES)
            .insert_one(node.clone())
            .await
            .expect("insert node");
        db.collection::<NodeServiceBinding>(NODE_SERVICE_BINDINGS)
            .insert_one(binding.clone())
            .await
            .expect("insert binding");
        db.collection::<UserService>(USER_SERVICES)
            .insert_many([actor_service, orphaned_service, destination_service])
            .await
            .expect("insert user services");

        let state = test_app_state(db.clone());
        let _ = transfer_node(
            State(state),
            test_auth_user(&actor_id),
            Path(node.id.clone()),
            Json(TransferNodeRequest {
                new_owner_user_id: org_id.clone(),
            }),
        )
        .await
        .expect("transfer succeeds");

        let cross_owner_routes = db
            .collection::<UserService>(USER_SERVICES)
            .count_documents(doc! {
                "node_id": &node.id,
                "user_id": { "$ne": &org_id },
                "is_active": true,
            })
            .await
            .expect("count cross-owner routes");
        assert_eq!(cross_owner_routes, 0);

        let active_bindings = db
            .collection::<NodeServiceBinding>(NODE_SERVICE_BINDINGS)
            .count_documents(doc! { "node_id": &node.id, "is_active": true })
            .await
            .expect("count active bindings");
        assert_eq!(active_bindings, 0);

        let transferred = db
            .collection::<Node>(NODES)
            .find_one(doc! { "_id": &node.id })
            .await
            .expect("query node")
            .expect("node exists");
        assert_eq!(transferred.user_id, org_id);

        let audit = wait_for_transfer_audit(&db, &node.id)
            .await
            .expect("transfer audit");
        let data = audit.event_data.expect("audit data");
        assert_eq!(
            data.get("actor_user_id").and_then(|v| v.as_str()),
            Some(actor_id.as_str())
        );
        assert_eq!(
            data.get("owner_user_id").and_then(|v| v.as_str()),
            Some(org_id.as_str())
        );
        assert_eq!(
            data.get("deactivated_bindings_count")
                .and_then(|v| v.as_u64()),
            Some(1)
        );
    }

    #[tokio::test]
    async fn list_admins_returns_personal_owner() {
        let Some(db) = connect_test_database("node_admins_personal").await else {
            eprintln!("skipping node handler test: no local MongoDB available");
            return;
        };

        let owner_id = Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_one(test_user(&owner_id, UserType::Person))
            .await
            .expect("insert owner");
        let node = test_node(&owner_id, "personal-node");
        db.collection::<Node>(NODES)
            .insert_one(node.clone())
            .await
            .expect("insert node");

        let state = test_app_state(db);
        let Json(response) = list_admins(
            State(state),
            test_auth_user(&owner_id),
            Path(node.id.clone()),
        )
        .await
        .expect("list admins");

        assert_eq!(response.admins.len(), 1);
        assert_eq!(response.admins[0].user_id, owner_id);
        assert_eq!(response.admins[0].role, node_service::NodeAdminRole::Owner);
    }

    #[tokio::test]
    async fn list_admins_returns_org_admins_for_readable_org_node() {
        let Some(db) = connect_test_database("node_admins_org").await else {
            eprintln!("skipping node handler test: no local MongoDB available");
            return;
        };

        let admin_a_id = Uuid::new_v4().to_string();
        let admin_b_id = Uuid::new_v4().to_string();
        let member_id = Uuid::new_v4().to_string();
        let org_id = Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_many([
                test_user(&admin_a_id, UserType::Person),
                test_user(&admin_b_id, UserType::Person),
                test_user(&member_id, UserType::Person),
                test_user(&org_id, UserType::Org),
            ])
            .await
            .expect("insert users");
        db.collection::<OrgMembership>(ORG_MEMBERSHIPS)
            .insert_many([
                test_membership(&org_id, &admin_a_id, OrgRole::Admin, None),
                test_membership(&org_id, &admin_b_id, OrgRole::Admin, None),
                test_membership(&org_id, &member_id, OrgRole::Member, None),
            ])
            .await
            .expect("insert memberships");
        let node = test_node(&org_id, "org-node");
        db.collection::<Node>(NODES)
            .insert_one(node.clone())
            .await
            .expect("insert node");

        let state = test_app_state(db);
        let Json(response) = list_admins(
            State(state),
            test_auth_user(&member_id),
            Path(node.id.clone()),
        )
        .await
        .expect("member can list node admins");

        let mut admin_ids: Vec<&str> = response
            .admins
            .iter()
            .map(|admin| admin.user_id.as_str())
            .collect();
        admin_ids.sort_unstable();
        let mut expected = vec![admin_a_id.as_str(), admin_b_id.as_str()];
        expected.sort_unstable();
        assert_eq!(admin_ids, expected);
        assert!(
            response
                .admins
                .iter()
                .all(|admin| admin.role == node_service::NodeAdminRole::Admin)
        );
    }

    #[tokio::test]
    async fn list_nodes_returns_empty_for_user_with_no_nodes() {
        let Some(db) = connect_test_database("node_ext_list_empty").await else {
            eprintln!("skipping node handler test: no local MongoDB available");
            return;
        };
        let actor_id = Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_one(test_user(&actor_id, UserType::Person))
            .await
            .expect("insert user");

        let state = test_app_state(db);
        let Json(response) = list_nodes(State(state), test_auth_user(&actor_id))
            .await
            .expect("list nodes");

        assert!(response.nodes.is_empty());
    }

    #[tokio::test]
    async fn get_node_returns_not_found_for_nonexistent() {
        let Some(db) = connect_test_database("node_ext_get_notfound").await else {
            eprintln!("skipping node handler test: no local MongoDB available");
            return;
        };
        let actor_id = Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_one(test_user(&actor_id, UserType::Person))
            .await
            .expect("insert user");

        let state = test_app_state(db);
        let err = get_node(
            State(state),
            test_auth_user(&actor_id),
            Path("nonexistent-node-id".to_string()),
        )
        .await
        .expect_err("should return not found");

        assert!(matches!(err, AppError::NodeNotFound(_)));
    }

    #[tokio::test]
    async fn delete_node_returns_not_found_for_nonexistent() {
        let Some(db) = connect_test_database("node_ext_del_notfound").await else {
            eprintln!("skipping node handler test: no local MongoDB available");
            return;
        };
        let actor_id = Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_one(test_user(&actor_id, UserType::Person))
            .await
            .expect("insert user");

        let state = test_app_state(db);
        let result = delete_node(
            State(state),
            test_auth_user(&actor_id),
            crate::telemetry::TelemetryContext::default(),
            Path("nonexistent-node-id".to_string()),
        )
        .await;

        let err = result
            .err()
            .expect("should return error for nonexistent node");
        assert!(matches!(err, AppError::NodeNotFound(_)));
    }

    #[tokio::test]
    async fn delete_node_removes_node_and_makes_it_unfindable() {
        let Some(db) = connect_test_database("node_ext_delete_ok").await else {
            eprintln!("skipping node handler test: no local MongoDB available");
            return;
        };
        let owner_id = Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_one(test_user(&owner_id, UserType::Person))
            .await
            .expect("insert user");
        let node = test_node(&owner_id, "deletable-node");
        let node_id = node.id.clone();
        db.collection::<Node>(NODES)
            .insert_one(node)
            .await
            .expect("insert node");

        let state = test_app_state(db.clone());
        let result = delete_node(
            State(state.clone()),
            test_auth_user(&owner_id),
            crate::telemetry::TelemetryContext::default(),
            Path(node_id.clone()),
        )
        .await;
        assert!(result.is_ok());

        let err = get_node(State(state), test_auth_user(&owner_id), Path(node_id))
            .await
            .expect_err("node should be gone");
        assert!(matches!(err, AppError::NodeNotFound(_)));
    }

    #[tokio::test]
    async fn list_bindings_returns_empty_for_node_with_no_bindings() {
        let Some(db) = connect_test_database("node_ext_bindings_empty").await else {
            eprintln!("skipping node handler test: no local MongoDB available");
            return;
        };
        let owner_id = Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_one(test_user(&owner_id, UserType::Person))
            .await
            .expect("insert user");
        let node = test_node(&owner_id, "empty-bindings-node");
        let node_id = node.id.clone();
        db.collection::<Node>(NODES)
            .insert_one(node)
            .await
            .expect("insert node");

        let state = test_app_state(db);
        let Json(response) = list_bindings(State(state), test_auth_user(&owner_id), Path(node_id))
            .await
            .expect("list bindings");

        assert!(response.bindings.is_empty());
    }

    #[tokio::test]
    async fn rotate_token_returns_new_credentials() {
        let Some(db) = connect_test_database("node_ext_rotate_token").await else {
            eprintln!("skipping node handler test: no local MongoDB available");
            return;
        };
        let owner_id = Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_one(test_user(&owner_id, UserType::Person))
            .await
            .expect("insert user");
        let node = test_node(&owner_id, "rotate-node");
        let node_id = node.id.clone();
        db.collection::<Node>(NODES)
            .insert_one(node)
            .await
            .expect("insert node");

        let state = test_app_state(db);
        let Json(response) = rotate_token(State(state), test_auth_user(&owner_id), Path(node_id))
            .await
            .expect("rotate token");

        assert!(!response.auth_token.is_empty());
        assert!(!response.signing_secret.is_empty());
        assert!(response.message.contains("rotated"));
    }

    #[test]
    fn audit_event_data_with_owner_adds_owner_only_when_shared() {
        let personal =
            audit_event_data_with_owner("user-1", "user-1", serde_json::json!({ "node_id": "n1" }));
        assert!(personal.get("owner_user_id").is_none());

        let shared =
            audit_event_data_with_owner("user-1", "org-1", serde_json::json!({ "node_id": "n1" }));
        assert_eq!(
            shared.get("owner_user_id").and_then(|v| v.as_str()),
            Some("org-1")
        );
    }

    // --- Pure function tests: build_metrics_info ---

    #[test]
    fn build_metrics_info_zero_requests_yields_zero_success_rate() {
        let metrics = NodeMetrics::default();
        let info = build_metrics_info(&metrics);

        assert_eq!(info.total_requests, 0);
        assert_eq!(info.success_count, 0);
        assert_eq!(info.error_count, 0);
        assert!((info.success_rate - 0.0).abs() < f64::EPSILON);
        assert!((info.avg_latency_ms - 0.0).abs() < f64::EPSILON);
        assert!(info.last_error.is_none());
        assert!(info.last_error_at.is_none());
        assert!(info.last_success_at.is_none());
    }

    #[test]
    fn build_metrics_info_computes_success_rate_correctly() {
        let metrics = NodeMetrics {
            total_requests: 200,
            success_count: 150,
            error_count: 50,
            avg_latency_ms: 42.5,
            last_error: Some("timeout".to_string()),
            last_error_at: Some(Utc::now()),
            last_success_at: Some(Utc::now()),
        };
        let info = build_metrics_info(&metrics);

        assert_eq!(info.total_requests, 200);
        assert_eq!(info.success_count, 150);
        assert_eq!(info.error_count, 50);
        assert!((info.success_rate - 0.75).abs() < f64::EPSILON);
        assert!((info.avg_latency_ms - 42.5).abs() < f64::EPSILON);
        assert_eq!(info.last_error.as_deref(), Some("timeout"));
        assert!(info.last_error_at.is_some());
        assert!(info.last_success_at.is_some());
    }

    #[test]
    fn build_metrics_info_all_successes_yields_one() {
        let metrics = NodeMetrics {
            total_requests: 1000,
            success_count: 1000,
            error_count: 0,
            avg_latency_ms: 10.0,
            last_error: None,
            last_error_at: None,
            last_success_at: Some(Utc::now()),
        };
        let info = build_metrics_info(&metrics);
        assert!((info.success_rate - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn build_metrics_info_all_errors_yields_zero_rate() {
        let metrics = NodeMetrics {
            total_requests: 50,
            success_count: 0,
            error_count: 50,
            avg_latency_ms: 500.0,
            last_error: Some("connection refused".to_string()),
            last_error_at: Some(Utc::now()),
            last_success_at: None,
        };
        let info = build_metrics_info(&metrics);
        assert!((info.success_rate - 0.0).abs() < f64::EPSILON);
        assert!(info.last_success_at.is_none());
    }

    // --- Serialization tests: NodeMetricsInfo ---

    #[test]
    fn node_metrics_info_serialization_skips_none_fields() {
        let info = NodeMetricsInfo {
            total_requests: 10,
            success_count: 8,
            error_count: 2,
            success_rate: 0.8,
            avg_latency_ms: 25.0,
            last_error: None,
            last_error_at: None,
            last_success_at: None,
        };
        let json = serde_json::to_value(&info).unwrap();

        assert_eq!(json["total_requests"], 10);
        assert_eq!(json["success_count"], 8);
        assert_eq!(json["error_count"], 2);
        assert!((json["success_rate"].as_f64().unwrap() - 0.8).abs() < f64::EPSILON);
        assert!((json["avg_latency_ms"].as_f64().unwrap() - 25.0).abs() < f64::EPSILON);
        // skip_serializing_if fields should be absent
        assert!(json.get("last_error").is_none());
        assert!(json.get("last_error_at").is_none());
        assert!(json.get("last_success_at").is_none());
    }

    #[test]
    fn node_metrics_info_serialization_includes_present_fields() {
        let info = NodeMetricsInfo {
            total_requests: 5,
            success_count: 3,
            error_count: 2,
            success_rate: 0.6,
            avg_latency_ms: 100.0,
            last_error: Some("bad gateway".to_string()),
            last_error_at: Some("2025-01-15T10:00:00+00:00".to_string()),
            last_success_at: Some("2025-01-15T09:00:00+00:00".to_string()),
        };
        let json = serde_json::to_value(&info).unwrap();

        assert_eq!(json["last_error"], "bad gateway");
        assert_eq!(json["last_error_at"], "2025-01-15T10:00:00+00:00");
        assert_eq!(json["last_success_at"], "2025-01-15T09:00:00+00:00");
    }

    // --- Serialization tests: NodeInfo ---

    #[test]
    fn node_info_serialization_skips_none_optional_fields() {
        let info = NodeInfo {
            id: "node-1".to_string(),
            name: "test-node".to_string(),
            owner: node_service::NodeOwnerInfo {
                kind: node_service::NodeOwnerKind::User,
                id: "user-1".to_string(),
                display_name: "Test User".to_string(),
            },
            status: "online".to_string(),
            is_connected: true,
            last_heartbeat_at: None,
            connected_at: None,
            metadata: None,
            metrics: None,
            binding_count: 3,
            created_at: "2025-01-01T00:00:00+00:00".to_string(),
        };
        let json = serde_json::to_value(&info).unwrap();

        assert_eq!(json["id"], "node-1");
        assert_eq!(json["name"], "test-node");
        assert_eq!(json["status"], "online");
        assert_eq!(json["is_connected"], true);
        assert_eq!(json["binding_count"], 3);
        assert_eq!(json["created_at"], "2025-01-01T00:00:00+00:00");
        assert_eq!(json["owner"]["kind"], "user");
        assert_eq!(json["owner"]["id"], "user-1");
        assert_eq!(json["owner"]["display_name"], "Test User");
        // Optional fields absent
        assert!(json.get("last_heartbeat_at").is_none());
        assert!(json.get("connected_at").is_none());
        assert!(json.get("metadata").is_none());
        assert!(json.get("metrics").is_none());
    }

    #[test]
    fn node_info_serialization_includes_all_fields_when_present() {
        let info = NodeInfo {
            id: "node-2".to_string(),
            name: "prod-node".to_string(),
            owner: node_service::NodeOwnerInfo {
                kind: node_service::NodeOwnerKind::Org,
                id: "org-1".to_string(),
                display_name: "Acme Corp".to_string(),
            },
            status: "draining".to_string(),
            is_connected: false,
            last_heartbeat_at: Some("2025-06-01T12:00:00+00:00".to_string()),
            connected_at: Some("2025-06-01T10:00:00+00:00".to_string()),
            metadata: Some(NodeMetadata {
                agent_version: Some("1.2.3".to_string()),
                os: Some("linux".to_string()),
                arch: Some("x86_64".to_string()),
                ip_address: Some("10.0.0.1".to_string()),
                provisioning_source: None,
            }),
            metrics: Some(NodeMetricsInfo {
                total_requests: 100,
                success_count: 90,
                error_count: 10,
                success_rate: 0.9,
                avg_latency_ms: 50.0,
                last_error: None,
                last_error_at: None,
                last_success_at: None,
            }),
            binding_count: 5,
            created_at: "2025-01-01T00:00:00+00:00".to_string(),
        };
        let json = serde_json::to_value(&info).unwrap();

        assert_eq!(json["owner"]["kind"], "org");
        assert_eq!(json["last_heartbeat_at"], "2025-06-01T12:00:00+00:00");
        assert_eq!(json["connected_at"], "2025-06-01T10:00:00+00:00");
        assert_eq!(json["metadata"]["agent_version"], "1.2.3");
        assert_eq!(json["metadata"]["os"], "linux");
        assert_eq!(json["metadata"]["arch"], "x86_64");
        assert_eq!(json["metadata"]["ip_address"], "10.0.0.1");
        assert_eq!(json["metrics"]["total_requests"], 100);
    }

    // --- Serialization tests: BindingInfo ---

    #[test]
    fn binding_info_serialization() {
        let info = BindingInfo {
            id: "binding-1".to_string(),
            service_id: "svc-1".to_string(),
            service_name: "OpenAI".to_string(),
            service_slug: "openai".to_string(),
            is_active: true,
            priority: 10,
            created_at: "2025-03-01T00:00:00+00:00".to_string(),
        };
        let json = serde_json::to_value(&info).unwrap();

        assert_eq!(json["id"], "binding-1");
        assert_eq!(json["service_id"], "svc-1");
        assert_eq!(json["service_name"], "OpenAI");
        assert_eq!(json["service_slug"], "openai");
        assert_eq!(json["is_active"], true);
        assert_eq!(json["priority"], 10);
        assert_eq!(json["created_at"], "2025-03-01T00:00:00+00:00");
    }

    // --- Serialization tests: CreateRegistrationTokenResponse ---

    #[test]
    fn create_registration_token_response_serialization() {
        let resp = CreateRegistrationTokenResponse {
            token_id: "tid-1".to_string(),
            token: "nyx_nreg_abc123".to_string(),
            name: "my-node".to_string(),
            expires_at: "2025-06-01T12:00:00+00:00".to_string(),
        };
        let json = serde_json::to_value(&resp).unwrap();

        assert_eq!(json["token_id"], "tid-1");
        assert_eq!(json["token"], "nyx_nreg_abc123");
        assert_eq!(json["name"], "my-node");
        assert_eq!(json["expires_at"], "2025-06-01T12:00:00+00:00");
    }

    // --- Serialization tests: RotateTokenResponse ---

    #[test]
    fn rotate_token_response_serialization() {
        let resp = RotateTokenResponse {
            auth_token: "new-token".to_string(),
            signing_secret: "new-secret".to_string(),
            message: "Rotated".to_string(),
        };
        let json = serde_json::to_value(&resp).unwrap();

        assert_eq!(json["auth_token"], "new-token");
        assert_eq!(json["signing_secret"], "new-secret");
        assert_eq!(json["message"], "Rotated");
    }

    // --- Serialization tests: TransferNodeResponse ---

    #[test]
    fn transfer_node_response_serialization() {
        let resp = TransferNodeResponse {
            node_id: "node-1".to_string(),
            previous_owner: node_service::NodeOwnerInfo {
                kind: node_service::NodeOwnerKind::User,
                id: "user-1".to_string(),
                display_name: "Alice".to_string(),
            },
            new_owner: node_service::NodeOwnerInfo {
                kind: node_service::NodeOwnerKind::Org,
                id: "org-1".to_string(),
                display_name: "Acme".to_string(),
            },
            deactivated_bindings_count: 2,
            cleared_user_service_count: 1,
        };
        let json = serde_json::to_value(&resp).unwrap();

        assert_eq!(json["node_id"], "node-1");
        assert_eq!(json["previous_owner"]["kind"], "user");
        assert_eq!(json["previous_owner"]["id"], "user-1");
        assert_eq!(json["new_owner"]["kind"], "org");
        assert_eq!(json["new_owner"]["id"], "org-1");
        assert_eq!(json["deactivated_bindings_count"], 2);
        assert_eq!(json["cleared_user_service_count"], 1);
    }

    // --- Serialization tests: PendingCredentialInfo ---

    #[test]
    fn pending_credential_info_serialization_skips_none_fields() {
        let info = PendingCredentialInfo {
            id: "pc-1".to_string(),
            node_id: "node-1".to_string(),
            service_slug: "openai".to_string(),
            injection_method: "header".to_string(),
            field_name: "Authorization".to_string(),
            target_url: None,
            label: None,
            created_by_user_id: "user-1".to_string(),
            owner_user_id: "user-1".to_string(),
            created_at: "2025-01-01T00:00:00+00:00".to_string(),
            expires_at: "2025-01-01T01:00:00+00:00".to_string(),
            consumed_at: None,
            declined_at: None,
            is_active: true,
        };
        let json = serde_json::to_value(&info).unwrap();

        assert_eq!(json["id"], "pc-1");
        assert_eq!(json["injection_method"], "header");
        assert_eq!(json["field_name"], "Authorization");
        assert_eq!(json["is_active"], true);
        // skip_serializing_if fields absent
        assert!(json.get("target_url").is_none());
        assert!(json.get("label").is_none());
        assert!(json.get("consumed_at").is_none());
        assert!(json.get("declined_at").is_none());
    }

    #[test]
    fn pending_credential_info_serialization_includes_optional_fields() {
        let info = PendingCredentialInfo {
            id: "pc-2".to_string(),
            node_id: "node-1".to_string(),
            service_slug: "anthropic".to_string(),
            injection_method: "query-param".to_string(),
            field_name: "api_key".to_string(),
            target_url: Some("https://api.anthropic.com".to_string()),
            label: Some("Production".to_string()),
            created_by_user_id: "user-1".to_string(),
            owner_user_id: "org-1".to_string(),
            created_at: "2025-01-01T00:00:00+00:00".to_string(),
            expires_at: "2025-01-01T01:00:00+00:00".to_string(),
            consumed_at: Some("2025-01-01T00:30:00+00:00".to_string()),
            declined_at: None,
            is_active: false,
        };
        let json = serde_json::to_value(&info).unwrap();

        assert_eq!(json["target_url"], "https://api.anthropic.com");
        assert_eq!(json["label"], "Production");
        assert_eq!(json["consumed_at"], "2025-01-01T00:30:00+00:00");
        assert_eq!(json["is_active"], false);
        assert!(json.get("declined_at").is_none());
    }

    // --- Pure function tests: pending_credential_info mapping ---

    #[test]
    fn pending_credential_info_maps_model_fields_correctly() {
        let now = Utc::now();
        let expires = now + chrono::Duration::hours(1);
        let model = crate::models::node_pending_credential::NodePendingCredential {
            id: "pc-map-1".to_string(),
            node_id: "node-map-1".to_string(),
            service_slug: "github".to_string(),
            injection_method: crate::models::node_pending_credential::InjectionMethod::Header,
            field_name: "Authorization".to_string(),
            target_url: Some("https://api.github.com".to_string()),
            label: Some("GH Token".to_string()),
            created_by_user_id: "creator-1".to_string(),
            owner_user_id: "owner-1".to_string(),
            created_at: now,
            expires_at: expires,
            consumed_at: None,
            declined_at: Some(now),
            is_active: false,
        };
        let info = pending_credential_info(model.clone());

        assert_eq!(info.id, "pc-map-1");
        assert_eq!(info.node_id, "node-map-1");
        assert_eq!(info.service_slug, "github");
        assert_eq!(info.injection_method, "header");
        assert_eq!(info.field_name, "Authorization");
        assert_eq!(info.target_url.as_deref(), Some("https://api.github.com"));
        assert_eq!(info.label.as_deref(), Some("GH Token"));
        assert_eq!(info.created_by_user_id, "creator-1");
        assert_eq!(info.owner_user_id, "owner-1");
        assert_eq!(info.created_at, now.to_rfc3339());
        assert_eq!(info.expires_at, expires.to_rfc3339());
        assert!(info.consumed_at.is_none());
        assert!(info.declined_at.is_some());
        assert!(!info.is_active);
    }

    // --- Serialization tests: MyBoundServicesResponse ---

    #[test]
    fn my_bound_services_response_serialization() {
        let resp = MyBoundServicesResponse {
            service_ids: vec!["svc-1".to_string(), "svc-2".to_string()],
        };
        let json = serde_json::to_value(&resp).unwrap();
        let ids = json["service_ids"].as_array().unwrap();
        assert_eq!(ids.len(), 2);
        assert_eq!(ids[0], "svc-1");
        assert_eq!(ids[1], "svc-2");
    }

    #[test]
    fn my_bound_services_response_empty_serialization() {
        let resp = MyBoundServicesResponse {
            service_ids: vec![],
        };
        let json = serde_json::to_value(&resp).unwrap();
        assert!(json["service_ids"].as_array().unwrap().is_empty());
    }

    // --- Serialization tests: CreateBindingResponse ---

    #[test]
    fn create_binding_response_serialization() {
        let resp = CreateBindingResponse {
            id: "bind-1".to_string(),
            service_id: "svc-1".to_string(),
            service_name: "Anthropic".to_string(),
            message: "Service binding created".to_string(),
        };
        let json = serde_json::to_value(&resp).unwrap();

        assert_eq!(json["id"], "bind-1");
        assert_eq!(json["service_id"], "svc-1");
        assert_eq!(json["service_name"], "Anthropic");
        assert_eq!(json["message"], "Service binding created");
    }

    // --- Serialization tests: NodeListResponse ---

    #[test]
    fn node_list_response_serialization_empty() {
        let resp = NodeListResponse { nodes: vec![] };
        let json = serde_json::to_value(&resp).unwrap();
        assert!(json["nodes"].as_array().unwrap().is_empty());
    }

    // --- Serialization tests: BindingListResponse ---

    #[test]
    fn binding_list_response_serialization_empty() {
        let resp = BindingListResponse { bindings: vec![] };
        let json = serde_json::to_value(&resp).unwrap();
        assert!(json["bindings"].as_array().unwrap().is_empty());
    }

    // --- Serialization tests: NodeAdminsResponse ---

    #[test]
    fn node_admins_response_serialization_empty() {
        let resp = NodeAdminsResponse { admins: vec![] };
        let json = serde_json::to_value(&resp).unwrap();
        assert!(json["admins"].as_array().unwrap().is_empty());
    }

    // --- Pure function tests: audit_event_data_with_owner edge cases ---

    #[test]
    fn audit_event_data_with_owner_preserves_existing_fields() {
        let result = audit_event_data_with_owner(
            "actor-1",
            "org-1",
            serde_json::json!({ "node_id": "n1", "extra": true }),
        );
        assert_eq!(result.get("node_id").and_then(|v| v.as_str()), Some("n1"));
        assert_eq!(result.get("extra").and_then(|v| v.as_bool()), Some(true));
        assert_eq!(
            result.get("owner_user_id").and_then(|v| v.as_str()),
            Some("org-1")
        );
    }

    #[test]
    fn audit_event_data_with_owner_non_object_value_unchanged() {
        let result = audit_event_data_with_owner("actor-1", "org-1", serde_json::json!("scalar"));
        // Non-object values should be returned unchanged (no owner_user_id insertion possible)
        assert_eq!(result, serde_json::json!("scalar"));
    }
}

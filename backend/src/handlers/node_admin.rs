use std::collections::HashMap;

use axum::{
    Json,
    extract::{Path, State},
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
use crate::mw::auth::AuthUser;
use crate::services::{audit_service, node_routing_service, node_service};

// --- Request types ---

#[derive(Debug, Deserialize)]
pub struct CreateRegistrationTokenRequest {
    pub name: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateBindingRequest {
    pub service_id: String,
}

#[derive(Debug, Deserialize)]
pub struct UpdateBindingRequest {
    pub priority: Option<i32>,
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

// --- Handlers ---

/// POST /api/v1/nodes/register-token
pub async fn create_registration_token(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Json(body): Json<CreateRegistrationTokenRequest>,
) -> AppResult<Json<CreateRegistrationTokenResponse>> {
    let user_id_str = auth_user.user_id.to_string();

    let (token_id, raw_token, expires_at): (String, String, chrono::DateTime<chrono::Utc>) =
        node_service::create_registration_token(
            &state.db,
            &user_id_str,
            &body.name,
            state.config.node_max_per_user,
            state.config.node_registration_token_ttl_secs,
        )
        .await?;

    audit_service::log_async(
        state.db.clone(),
        Some(user_id_str),
        "node_registration_token_created".to_string(),
        Some(serde_json::json!({
            "token_id": &token_id,
            "name": &body.name,
        })),
        None,
        None,
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
            .map(|n| bson::Bson::String(n.id.clone()))
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
            if let Some(node_id) = result.get_str("_id").ok() {
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
        .map(|node| NodeInfo {
            id: node.id.clone(),
            name: node.name.clone(),
            status: node.status.as_str().to_string(),
            is_connected: state.node_ws_manager.is_connected(&node.id),
            last_heartbeat_at: node.last_heartbeat_at.map(|dt| dt.to_rfc3339()),
            connected_at: node.connected_at.map(|dt| dt.to_rfc3339()),
            metadata: node.metadata.clone(),
            metrics: Some(build_metrics_info(&node.metrics)),
            binding_count: binding_counts.get(&node.id).copied().unwrap_or(0),
            created_at: node.created_at.to_rfc3339(),
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

    let binding_count = state
        .db
        .collection::<mongodb::bson::Document>("node_service_bindings")
        .count_documents(doc! { "node_id": &node.id, "is_active": true })
        .await?;

    Ok(Json(NodeInfo {
        id: node.id.clone(),
        name: node.name,
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
    Path(node_id): Path<String>,
) -> AppResult<impl IntoResponse> {
    let user_id_str = auth_user.user_id.to_string();

    node_service::delete_node(&state.db, &user_id_str, &node_id).await?;

    // Disconnect WebSocket if connected
    if state.node_ws_manager.is_connected(&node_id) {
        state
            .node_ws_manager
            .disconnect_connection(&node_id, 4006, "node deleted")
            .await;
    }

    audit_service::log_async(
        state.db.clone(),
        Some(user_id_str),
        "node_deleted".to_string(),
        Some(serde_json::json!({ "node_id": &node_id })),
        None,
        None,
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

    audit_service::log_async(
        state.db.clone(),
        Some(user_id_str),
        "node_token_rotated".to_string(),
        Some(serde_json::json!({ "node_id": &node_id })),
        None,
        None,
    );

    Ok(Json(RotateTokenResponse {
        auth_token: raw_token,
        signing_secret: raw_signing_secret,
        message:
            "Auth token and signing secret rotated. The node must reconnect with the new token."
                .to_string(),
    }))
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

    audit_service::log_async(
        state.db.clone(),
        Some(user_id_str),
        "node_binding_created".to_string(),
        Some(serde_json::json!({
            "binding_id": &binding.id,
            "node_id": &node_id,
            "service_id": &body.service_id,
        })),
        None,
        None,
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

    audit_service::log_async(
        state.db.clone(),
        Some(user_id_str),
        "node_binding_updated".to_string(),
        Some(serde_json::json!({
            "binding_id": &binding_id,
            "node_id": &node_id,
            "priority": body.priority,
        })),
        None,
        None,
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

    node_service::delete_binding(&state.db, &user_id_str, &node_id, &binding_id).await?;

    audit_service::log_async(
        state.db.clone(),
        Some(user_id_str),
        "node_binding_deleted".to_string(),
        Some(serde_json::json!({ "binding_id": &binding_id })),
        None,
        None,
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

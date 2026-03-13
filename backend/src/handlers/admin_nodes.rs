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
use crate::errors::AppResult;
use crate::handlers::admin_helpers::require_admin;
use crate::handlers::node_admin::{NodeMetricsInfo, build_metrics_info};
use crate::models::node::{NodeMetadata, NodeStatus};
use crate::models::user::{COLLECTION_NAME as USERS, User};
use crate::mw::auth::AuthUser;
use crate::services::{audit_service, node_service};

// --- Request/Response types ---

#[derive(Debug, Deserialize)]
pub struct AdminNodeListQuery {
    pub page: Option<u64>,
    pub per_page: Option<u64>,
    pub status: Option<String>,
    pub user_id: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct AdminNodeListResponse {
    pub nodes: Vec<AdminNodeInfo>,
    pub total: u64,
    pub page: u64,
    pub per_page: u64,
}

#[derive(Debug, Serialize)]
pub struct AdminNodeInfo {
    pub id: String,
    pub name: String,
    pub user_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub user_email: Option<String>,
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

// --- Handlers ---

/// GET /api/v1/admin/nodes
pub async fn admin_list_nodes(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Query(query): Query<AdminNodeListQuery>,
) -> AppResult<Json<AdminNodeListResponse>> {
    require_admin(&state, &auth_user).await?;

    let page = query.page.unwrap_or(1).max(1);
    let per_page = query.per_page.unwrap_or(50).min(100);

    let (nodes, total) = node_service::list_all_nodes(
        &state.db,
        page,
        per_page,
        query.status.as_deref(),
        query.user_id.as_deref(),
    )
    .await?;

    // Batch-fetch user emails
    let user_ids: Vec<&str> = nodes.iter().map(|n| n.user_id.as_str()).collect();
    let user_emails: HashMap<String, String> = if user_ids.is_empty() {
        HashMap::new()
    } else {
        let user_id_array: bson::Array = user_ids
            .iter()
            .map(|id| bson::Bson::String(id.to_string()))
            .collect();
        let users: Vec<User> = state
            .db
            .collection::<User>(USERS)
            .find(doc! { "_id": { "$in": user_id_array } })
            .await?
            .try_collect()
            .await?;
        users.into_iter().map(|u| (u.id, u.email)).collect()
    };

    // Batch-fetch binding counts
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

    let node_infos: Vec<AdminNodeInfo> = nodes
        .iter()
        .map(|node| AdminNodeInfo {
            id: node.id.clone(),
            name: node.name.clone(),
            user_id: node.user_id.clone(),
            user_email: user_emails.get(&node.user_id).cloned(),
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

    Ok(Json(AdminNodeListResponse {
        nodes: node_infos,
        total,
        page,
        per_page,
    }))
}

/// GET /api/v1/admin/nodes/{node_id}
pub async fn admin_get_node(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(node_id): Path<String>,
) -> AppResult<Json<AdminNodeInfo>> {
    require_admin(&state, &auth_user).await?;

    let node = node_service::get_node_by_id(&state.db, &node_id)
        .await?
        .ok_or_else(|| crate::errors::AppError::NodeNotFound("Node not found".to_string()))?;

    // Fetch user email
    let user_email = state
        .db
        .collection::<User>(USERS)
        .find_one(doc! { "_id": &node.user_id })
        .await?
        .map(|u| u.email);

    let binding_count = state
        .db
        .collection::<mongodb::bson::Document>("node_service_bindings")
        .count_documents(doc! { "node_id": &node.id, "is_active": true })
        .await?;

    Ok(Json(AdminNodeInfo {
        id: node.id.clone(),
        name: node.name,
        user_id: node.user_id,
        user_email,
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

/// POST /api/v1/admin/nodes/{node_id}/disconnect
pub async fn admin_disconnect_node(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(node_id): Path<String>,
) -> AppResult<impl IntoResponse> {
    require_admin(&state, &auth_user).await?;

    if state.node_ws_manager.is_connected(&node_id) {
        state
            .node_ws_manager
            .disconnect_connection(&node_id, 4000, "admin disconnected node")
            .await;
        node_service::set_node_status(&state.db, &node_id, NodeStatus::Offline).await?;
    }

    audit_service::log_async(
        state.db.clone(),
        Some(auth_user.user_id.to_string()),
        "admin_node_disconnected".to_string(),
        Some(serde_json::json!({ "node_id": &node_id })),
        None,
        None,
    );

    Ok(StatusCode::NO_CONTENT)
}

/// DELETE /api/v1/admin/nodes/{node_id}
pub async fn admin_delete_node(
    State(state): State<AppState>,
    auth_user: AuthUser,
    Path(node_id): Path<String>,
) -> AppResult<impl IntoResponse> {
    require_admin(&state, &auth_user).await?;

    node_service::admin_delete_node(&state.db, &node_id).await?;

    // Disconnect WebSocket if connected
    if state.node_ws_manager.is_connected(&node_id) {
        state
            .node_ws_manager
            .disconnect_connection(&node_id, 4006, "node deleted by admin")
            .await;
    }

    audit_service::log_async(
        state.db.clone(),
        Some(auth_user.user_id.to_string()),
        "admin_node_deleted".to_string(),
        Some(serde_json::json!({ "node_id": &node_id })),
        None,
        None,
    );

    Ok(StatusCode::NO_CONTENT)
}

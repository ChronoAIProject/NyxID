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
use crate::handlers::admin_helpers::{require_admin, require_admin_or_operator};
use crate::handlers::node_admin::{NodeMetricsInfo, build_metrics_info};
use crate::models::node::{NodeMetadata, NodeStatus};
use crate::models::user::{COLLECTION_NAME as USERS, User};
use crate::mw::auth::AuthUser;
use crate::services::{audit_service, node_service};
use crate::telemetry::{
    context::{TelemetryContext, emit_event},
    sampling::hash_short_id,
    schema::TelemetryEvent,
};

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
    require_admin_or_operator(&state, &auth_user, "admin.nodes.list").await?;

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
    require_admin_or_operator(&state, &auth_user, "admin.nodes.get").await?;

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
    tele: TelemetryContext,
    Path(node_id): Path<String>,
) -> AppResult<impl IntoResponse> {
    require_admin(&state, &auth_user).await?;

    let was_connected = state.node_ws_manager.is_connected(&node_id);
    if was_connected {
        state
            .node_ws_manager
            .disconnect_connection(&node_id, 4000, "admin disconnected node")
            .await;
        node_service::set_node_status(&state.db, &node_id, NodeStatus::Offline).await?;
    }

    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "admin_node_disconnected",
        Some(serde_json::json!({ "node_id": &node_id })),
    );

    // Only emit when a real disconnect actually happened. Posting to
    // /disconnect for an already-offline node (or a typo / nonexistent
    // id) is idempotent and should not fabricate disconnect activity
    // in telemetry.
    if was_connected {
        emit_event(
            state.telemetry.as_deref(),
            &auth_user.user_id.to_string(),
            auth_user.api_key_id.as_deref(),
            &tele,
            TelemetryEvent::AdminNodeDisconnected {
                // Raw UUID would be scrubbed to `[UUID_REDACTED]`; hash
                // keeps per-node granularity without leaking the UUID.
                node_id: hash_short_id(&node_id),
            },
        );
    }

    Ok(StatusCode::NO_CONTENT)
}

/// DELETE /api/v1/admin/nodes/{node_id}
pub async fn admin_delete_node(
    State(state): State<AppState>,
    auth_user: AuthUser,
    tele: TelemetryContext,
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

    audit_service::log_for_user(
        state.db.clone(),
        &auth_user,
        "admin_node_deleted",
        Some(serde_json::json!({ "node_id": &node_id })),
    );

    emit_event(
        state.telemetry.as_deref(),
        &auth_user.user_id.to_string(),
        auth_user.api_key_id.as_deref(),
        &tele,
        TelemetryEvent::AdminNodeDeleted {
            // Raw UUID would be scrubbed to `[UUID_REDACTED]`; hash keeps
            // per-node granularity without leaking the UUID.
            node_id: hash_short_id(&node_id),
        },
    );

    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::node::{COLLECTION_NAME as NODES, Node, NodeMetrics, NodeStatus};
    use crate::models::user::{COLLECTION_NAME as USERS, User, UserType};
    use crate::services::role_service;
    use crate::test_utils::{connect_test_database, test_app_state, test_auth_user, test_user};
    use axum::extract::{Path, Query, State};
    use chrono::Utc;
    use uuid::Uuid;

    async fn insert_admin(db: &mongodb::Database) -> String {
        role_service::seed_system_roles(db)
            .await
            .expect("seed platform roles");
        let platform_role_ids = role_service::get_platform_role_ids(db)
            .await
            .expect("platform role ids");
        let id = Uuid::new_v4().to_string();
        let mut user = test_user(&id, UserType::Person);
        user.role_ids.push(platform_role_ids.admin);
        db.collection::<User>(USERS)
            .insert_one(&user)
            .await
            .expect("insert admin user");
        id
    }

    fn make_test_node(user_id: &str) -> Node {
        Node {
            id: Uuid::new_v4().to_string(),
            user_id: user_id.to_string(),
            name: "test-node".to_string(),
            status: NodeStatus::Offline,
            auth_token_hash: "deadbeef".repeat(8),
            signing_secret_encrypted: None,
            signing_secret_hash: "abcdef01".repeat(8),
            last_heartbeat_at: None,
            connected_at: None,
            metadata: None,
            metrics: NodeMetrics::default(),
            is_active: true,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[tokio::test]
    async fn test_admin_list_nodes_empty() {
        let Some(db) = connect_test_database("h_admin_nodes_list").await else {
            return;
        };
        let admin_id = insert_admin(&db).await;
        let state = test_app_state(db);

        let result = admin_list_nodes(
            State(state),
            test_auth_user(&admin_id),
            Query(AdminNodeListQuery {
                page: None,
                per_page: None,
                status: None,
                user_id: None,
            }),
        )
        .await
        .expect("admin_list_nodes should succeed");

        assert_eq!(result.0.total, 0);
        assert!(result.0.nodes.is_empty());
    }

    #[tokio::test]
    async fn test_admin_list_nodes_with_data() {
        let Some(db) = connect_test_database("h_admin_nodes_list_data").await else {
            return;
        };
        let admin_id = insert_admin(&db).await;

        let owner_id = Uuid::new_v4().to_string();
        let owner = test_user(&owner_id, UserType::Person);
        db.collection::<User>(USERS)
            .insert_one(&owner)
            .await
            .expect("insert owner");

        let node = make_test_node(&owner_id);
        db.collection::<Node>(NODES)
            .insert_one(&node)
            .await
            .expect("insert node");

        let state = test_app_state(db);

        let result = admin_list_nodes(
            State(state),
            test_auth_user(&admin_id),
            Query(AdminNodeListQuery {
                page: Some(1),
                per_page: Some(10),
                status: None,
                user_id: None,
            }),
        )
        .await
        .expect("admin_list_nodes should succeed");

        assert_eq!(result.0.total, 1);
        assert_eq!(result.0.nodes.len(), 1);
        assert_eq!(result.0.nodes[0].id, node.id);
        assert_eq!(result.0.nodes[0].user_email, Some(owner.email));
    }

    #[tokio::test]
    async fn test_admin_get_node() {
        let Some(db) = connect_test_database("h_admin_nodes_get").await else {
            return;
        };
        let admin_id = insert_admin(&db).await;

        let owner_id = Uuid::new_v4().to_string();
        let owner = test_user(&owner_id, UserType::Person);
        db.collection::<User>(USERS)
            .insert_one(&owner)
            .await
            .expect("insert owner");

        let node = make_test_node(&owner_id);
        let node_id = node.id.clone();
        db.collection::<Node>(NODES)
            .insert_one(&node)
            .await
            .expect("insert node");

        let state = test_app_state(db);

        let result = admin_get_node(
            State(state),
            test_auth_user(&admin_id),
            Path(node_id.clone()),
        )
        .await
        .expect("admin_get_node should succeed");

        assert_eq!(result.0.id, node_id);
        assert_eq!(result.0.name, "test-node");
        assert_eq!(result.0.status, "offline");
    }

    #[tokio::test]
    async fn test_admin_get_node_not_found() {
        let Some(db) = connect_test_database("h_admin_nodes_get_404").await else {
            return;
        };
        let admin_id = insert_admin(&db).await;
        let state = test_app_state(db);

        let err = admin_get_node(
            State(state),
            test_auth_user(&admin_id),
            Path(Uuid::new_v4().to_string()),
        )
        .await;

        assert!(err.is_err());
    }

    #[tokio::test]
    async fn test_admin_disconnect_node_noop() {
        let Some(db) = connect_test_database("h_admin_nodes_disconnect").await else {
            return;
        };
        let admin_id = insert_admin(&db).await;

        let owner_id = Uuid::new_v4().to_string();
        let owner = test_user(&owner_id, UserType::Person);
        db.collection::<User>(USERS)
            .insert_one(&owner)
            .await
            .expect("insert owner");

        let node = make_test_node(&owner_id);
        let node_id = node.id.clone();
        db.collection::<Node>(NODES)
            .insert_one(&node)
            .await
            .expect("insert node");

        let state = test_app_state(db);

        let result = admin_disconnect_node(
            State(state),
            test_auth_user(&admin_id),
            TelemetryContext::default(),
            Path(node_id),
        )
        .await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_admin_delete_node() {
        let Some(db) = connect_test_database("h_admin_nodes_delete").await else {
            return;
        };
        let admin_id = insert_admin(&db).await;

        let owner_id = Uuid::new_v4().to_string();
        let owner = test_user(&owner_id, UserType::Person);
        db.collection::<User>(USERS)
            .insert_one(&owner)
            .await
            .expect("insert owner");

        let node = make_test_node(&owner_id);
        let node_id = node.id.clone();
        db.collection::<Node>(NODES)
            .insert_one(&node)
            .await
            .expect("insert node");

        let state = test_app_state(db.clone());

        let result = admin_delete_node(
            State(state),
            test_auth_user(&admin_id),
            TelemetryContext::default(),
            Path(node_id.clone()),
        )
        .await;

        assert!(result.is_ok());

        let deleted = db
            .collection::<Node>(NODES)
            .find_one(doc! { "_id": &node_id, "is_active": true })
            .await
            .expect("query node");
        assert!(deleted.is_none());
    }
}

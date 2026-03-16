use std::collections::HashMap;

use futures::TryStreamExt;
use mongodb::bson::doc;

use crate::errors::AppResult;
use crate::models::node::{COLLECTION_NAME as NODES, Node, NodeStatus};
use crate::models::node_service_binding::{
    COLLECTION_NAME as NODE_SERVICE_BINDINGS, NodeServiceBinding,
};
use crate::services::node_ws_manager::NodeWsManager;

/// Result of a routing decision.
pub struct NodeRoute {
    pub node_id: String,
    /// Ordered list of fallback node IDs (for failover)
    pub fallback_node_ids: Vec<String>,
}

async fn load_viable_bindings(
    db: &mongodb::Database,
    user_id: &str,
    service_id: Option<&str>,
    ws_manager: &NodeWsManager,
) -> AppResult<Vec<NodeServiceBinding>> {
    let mut filter = doc! {
        "user_id": user_id,
        "is_active": true,
    };
    if let Some(service_id) = service_id {
        filter.insert("service_id", service_id);
    }

    let bindings: Vec<NodeServiceBinding> = db
        .collection::<NodeServiceBinding>(NODE_SERVICE_BINDINGS)
        .find(filter)
        .sort(doc! { "priority": 1 })
        .await?
        .try_collect()
        .await?;

    if bindings.is_empty() {
        return Ok(vec![]);
    }

    let node_id_array: bson::Array = bindings
        .iter()
        .map(|b| bson::Bson::String(b.node_id.clone()))
        .collect();

    let nodes: Vec<Node> = db
        .collection::<Node>(NODES)
        .find(doc! {
            "_id": { "$in": node_id_array },
            "is_active": true,
            "status": NodeStatus::Online.as_str(),
        })
        .await?
        .try_collect()
        .await?;

    let online_nodes: HashMap<&str, &Node> = nodes.iter().map(|n| (n.id.as_str(), n)).collect();

    let mut viable_bindings = Vec::new();
    for binding in bindings {
        if let Some(node) = online_nodes.get(binding.node_id.as_str()) {
            if !ws_manager.is_connected(&node.id) {
                continue;
            }

            if node.metrics.total_requests > 10 {
                let error_rate =
                    node.metrics.error_count as f64 / node.metrics.total_requests as f64;
                if error_rate > 0.5 {
                    tracing::warn!(
                        node_id = %node.id,
                        error_rate = %error_rate,
                        "Skipping unhealthy node"
                    );
                    continue;
                }
            }

            viable_bindings.push(binding);
        }
    }

    Ok(viable_bindings)
}

/// Check if a user has a node binding for this service.
/// Returns Some(NodeRoute) if the user has an active binding to an active online node.
/// Returns None to fall through to standard proxy.
///
/// Selection logic:
/// 1. Find active bindings for (user_id, service_id) ordered by priority
/// 2. Batch-fetch all referenced nodes in a single query
/// 3. Filter to nodes that are both DB-online AND WS-connected
/// 4. Skip nodes with >50% error rate (if enough samples)
/// 5. Return the first viable node as primary, rest as fallbacks
/// 6. Return None if no viable node found
pub async fn resolve_node_route(
    db: &mongodb::Database,
    user_id: &str,
    service_id: &str,
    ws_manager: &NodeWsManager,
) -> AppResult<Option<NodeRoute>> {
    let viable_nodes: Vec<String> = load_viable_bindings(db, user_id, Some(service_id), ws_manager)
        .await?
        .into_iter()
        .map(|binding| binding.node_id)
        .collect();

    if viable_nodes.is_empty() {
        return Ok(None);
    }

    Ok(Some(NodeRoute {
        node_id: viable_nodes[0].clone(),
        fallback_node_ids: viable_nodes[1..].to_vec(),
    }))
}

/// Check if a user has any currently routable node bindings for a specific service.
pub async fn has_routable_node_bindings(
    db: &mongodb::Database,
    user_id: &str,
    service_id: &str,
    ws_manager: &NodeWsManager,
) -> AppResult<bool> {
    Ok(
        !load_viable_bindings(db, user_id, Some(service_id), ws_manager)
            .await?
            .is_empty(),
    )
}

/// Return all service IDs for which the user currently has at least one viable node route.
pub async fn list_routable_service_ids(
    db: &mongodb::Database,
    user_id: &str,
    ws_manager: &NodeWsManager,
) -> AppResult<Vec<String>> {
    let mut service_ids: Vec<String> = load_viable_bindings(db, user_id, None, ws_manager)
        .await?
        .into_iter()
        .map(|binding| binding.service_id)
        .collect();
    service_ids.sort();
    service_ids.dedup();
    Ok(service_ids)
}

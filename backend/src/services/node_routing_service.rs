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
    // Find active bindings for this user+service, ordered by priority
    let bindings: Vec<NodeServiceBinding> = db
        .collection::<NodeServiceBinding>(NODE_SERVICE_BINDINGS)
        .find(doc! {
            "user_id": user_id,
            "service_id": service_id,
            "is_active": true,
        })
        .sort(doc! { "priority": 1 })
        .await?
        .try_collect()
        .await?;

    if bindings.is_empty() {
        return Ok(None);
    }

    // Batch-fetch all referenced nodes in a single query
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

    // Filter to nodes that are both DB-online AND WS-connected,
    // skipping unhealthy nodes (>50% error rate with sufficient samples)
    let mut viable_nodes: Vec<String> = Vec::new();
    for binding in &bindings {
        if let Some(node) = online_nodes.get(binding.node_id.as_str()) {
            if !ws_manager.is_connected(&node.id) {
                continue;
            }

            // Skip nodes with >50% error rate if they have enough samples
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

            viable_nodes.push(node.id.clone());
        }
    }

    if viable_nodes.is_empty() {
        return Ok(None);
    }

    Ok(Some(NodeRoute {
        node_id: viable_nodes[0].clone(),
        fallback_node_ids: viable_nodes[1..].to_vec(),
    }))
}

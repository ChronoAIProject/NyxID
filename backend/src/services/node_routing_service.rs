use std::collections::{HashMap, HashSet};

use futures::TryStreamExt;
use mongodb::bson::doc;

use crate::errors::AppResult;
use crate::models::node::{COLLECTION_NAME as NODES, Node, NodeStatus};
use crate::models::node_service_binding::{
    COLLECTION_NAME as NODE_SERVICE_BINDINGS, NodeServiceBinding,
};
use crate::models::user_service::{COLLECTION_NAME as USER_SERVICES, UserService};
use crate::services::node_ws_manager::NodeWsManager;

/// Result of a routing decision.
pub struct NodeRoute {
    pub node_id: String,
    /// Ordered list of fallback node IDs (for failover)
    pub fallback_node_ids: Vec<String>,
}

fn is_node_viable(node: &Node, ws_manager: &NodeWsManager) -> bool {
    if !ws_manager.is_connected(&node.id) {
        return false;
    }

    if node.metrics.total_requests > 10 {
        let error_rate = node.metrics.error_count as f64 / node.metrics.total_requests as f64;
        if error_rate > 0.5 {
            tracing::warn!(
                node_id = %node.id,
                error_rate = %error_rate,
                "Skipping unhealthy node"
            );
            return false;
        }
    }

    true
}

/// Validate that a specific node_id is viable for proxy routing on this backend instance.
///
/// "Viable" means the node is active and marked Online in MongoDB, AND has an active
/// WebSocket connection on this instance, AND is not unhealthy (error rate check).
///
/// This is the single source of truth for "can we actually send a proxy request to
/// this node right now?" and must stay aligned with the pre-send check in
/// `NodeWsManager::send_proxy_request`.
pub async fn is_node_id_viable(
    db: &mongodb::Database,
    node_id: &str,
    ws_manager: &NodeWsManager,
) -> AppResult<bool> {
    let node: Option<Node> = db
        .collection::<Node>(NODES)
        .find_one(doc! {
            "_id": node_id,
            "is_active": true,
            "status": NodeStatus::Online.as_str(),
        })
        .await?;

    Ok(node.as_ref().is_some_and(|n| is_node_viable(n, ws_manager)))
}

fn collect_ordered_node_ids(
    primary_node_id: Option<String>,
    bindings: Vec<NodeServiceBinding>,
) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut node_ids = Vec::new();

    if let Some(primary_node_id) = primary_node_id
        && seen.insert(primary_node_id.clone())
    {
        node_ids.push(primary_node_id);
    }

    for binding in bindings {
        if seen.insert(binding.node_id.clone()) {
            node_ids.push(binding.node_id);
        }
    }

    node_ids
}

pub fn build_node_route(node_ids: Vec<String>) -> Option<NodeRoute> {
    let mut node_ids = node_ids.into_iter();
    let node_id = node_ids.next()?;

    Some(NodeRoute {
        node_id,
        fallback_node_ids: node_ids.collect(),
    })
}

async fn load_viable_bindings(
    db: &mongodb::Database,
    owner_user_id: &str,
    service_id: Option<&str>,
    ws_manager: &NodeWsManager,
) -> AppResult<Vec<NodeServiceBinding>> {
    let mut filter = doc! {
        "user_id": owner_user_id,
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
        if let Some(node) = online_nodes.get(binding.node_id.as_str())
            && is_node_viable(node, ws_manager)
        {
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
/// 1. Check UserService.node_id (streamlined services path) for the catalog service
/// 2. If no UserService node_id, find active NodeServiceBindings ordered by priority
/// 3. Batch-fetch all referenced nodes in a single query
/// 4. Filter to nodes that are both DB-online AND WS-connected
/// 5. Skip nodes with >50% error rate (if enough samples)
/// 6. Return the first viable node as primary, rest as fallbacks
/// 7. Return None if no viable node found
pub async fn resolve_node_route(
    db: &mongodb::Database,
    user_id: &str,
    service_id: &str,
    ws_manager: &NodeWsManager,
) -> AppResult<Option<NodeRoute>> {
    let owner_user_id = effective_service_owner_id(db, user_id, service_id).await?;
    resolve_node_route_for_owner(db, &owner_user_id, service_id, ws_manager).await
}

async fn resolve_node_route_for_owner(
    db: &mongodb::Database,
    owner_user_id: &str,
    service_id: &str,
    ws_manager: &NodeWsManager,
) -> AppResult<Option<NodeRoute>> {
    let primary_node_id =
        resolve_from_user_service(db, owner_user_id, service_id, ws_manager).await?;
    let viable_bindings =
        load_viable_bindings(db, owner_user_id, Some(service_id), ws_manager).await?;

    Ok(build_node_route(collect_ordered_node_ids(
        primary_node_id,
        viable_bindings,
    )))
}

async fn effective_service_owner_id(
    db: &mongodb::Database,
    actor_user_id: &str,
    service_id: &str,
) -> AppResult<String> {
    let effective_owner = crate::services::proxy_service::find_effective_service_owner(
        db,
        actor_user_id,
        None,
        Some(service_id),
    )
    .await?;

    Ok(effective_owner.unwrap_or_else(|| actor_user_id.to_string()))
}

/// Resolve a node route from UserService.node_id for a given catalog service.
///
/// Returns Some(node_id) if the user has a UserService with catalog_service_id
/// matching the given service_id and a viable node_id set.
async fn resolve_from_user_service(
    db: &mongodb::Database,
    owner_user_id: &str,
    service_id: &str,
    ws_manager: &NodeWsManager,
) -> AppResult<Option<String>> {
    let user_service: Option<UserService> = db
        .collection::<UserService>(USER_SERVICES)
        .find_one(doc! {
            "user_id": owner_user_id,
            "catalog_service_id": service_id,
            "is_active": true,
        })
        .await?;

    let node_id = match user_service.and_then(|s| s.node_id) {
        Some(nid) if !nid.is_empty() => nid,
        _ => return Ok(None),
    };

    if !is_node_id_viable(db, &node_id, ws_manager).await? {
        return Ok(None);
    }

    Ok(Some(node_id))
}

/// Check if a user has any currently routable node bindings for a specific service.
/// Checks UserService.node_id first, then falls back to NodeServiceBindings.
pub async fn has_routable_node_bindings(
    db: &mongodb::Database,
    user_id: &str,
    service_id: &str,
    ws_manager: &NodeWsManager,
) -> AppResult<bool> {
    let owner_user_id = effective_service_owner_id(db, user_id, service_id).await?;
    Ok(
        resolve_node_route_for_owner(db, &owner_user_id, service_id, ws_manager)
            .await?
            .is_some(),
    )
}

/// Check whether the user's `UserService` for this catalog service explicitly
/// pins routing to a node, regardless of whether that node is currently online.
///
/// Used by the proxy to enforce the "Route via Node" contract: when a service
/// is explicitly bound to a node, requests must not silently fall back to direct
/// routing if the node is unavailable. See ChronoAIProject/NyxID#328.
pub async fn user_service_has_explicit_node(
    db: &mongodb::Database,
    user_id: &str,
    service_id: &str,
) -> AppResult<bool> {
    let owner_user_id = effective_service_owner_id(db, user_id, service_id).await?;
    let user_service: Option<UserService> = db
        .collection::<UserService>(USER_SERVICES)
        .find_one(doc! {
            "user_id": owner_user_id,
            "catalog_service_id": service_id,
            "is_active": true,
        })
        .await?;

    Ok(user_service
        .and_then(|s| s.node_id)
        .filter(|nid| !nid.is_empty())
        .is_some())
}

pub async fn list_viable_binding_node_ids(
    db: &mongodb::Database,
    user_id: &str,
    service_id: &str,
    ws_manager: &NodeWsManager,
) -> AppResult<Vec<String>> {
    let owner_user_id = effective_service_owner_id(db, user_id, service_id).await?;
    Ok(collect_ordered_node_ids(
        None,
        load_viable_bindings(db, &owner_user_id, Some(service_id), ws_manager).await?,
    ))
}

async fn load_viable_user_service_catalog_ids(
    db: &mongodb::Database,
    user_id: &str,
    ws_manager: &NodeWsManager,
) -> AppResult<Vec<String>> {
    load_viable_user_service_catalog_ids_filtered(db, user_id, ws_manager, |_| true).await
}

/// Scope-aware variant: a `UserService` catalog entry only counts as a
/// viable route when its `node_id` passes `node_filter`. Used by the
/// scoped MCP discovery path so a scoped API key doesn't see platform
/// tools whose only user-service route is pinned to an out-of-scope
/// node (twenty-third-round Codex P2). `execute_tool` rejects the call
/// anyway once it sees the primary `node_id` is out of scope, so
/// discovery and execution must agree or scoped agents get broken tool
/// listings.
async fn load_viable_user_service_catalog_ids_filtered<F>(
    db: &mongodb::Database,
    user_id: &str,
    ws_manager: &NodeWsManager,
    node_filter: F,
) -> AppResult<Vec<String>>
where
    F: Fn(&str) -> bool,
{
    let services: Vec<UserService> = db
        .collection::<UserService>(USER_SERVICES)
        .find(doc! {
            "user_id": user_id,
            "catalog_service_id": { "$type": "string" },
            "node_id": { "$type": "string", "$ne": "" },
            "is_active": true,
        })
        .await?
        .try_collect()
        .await?;

    if services.is_empty() {
        return Ok(vec![]);
    }

    let node_id_array: bson::Array = services
        .iter()
        .filter_map(|service| service.node_id.as_ref())
        .map(|node_id| bson::Bson::String(node_id.clone()))
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
    let mut service_ids = Vec::new();

    for service in services {
        let Some(node_id) = service.node_id.as_deref() else {
            continue;
        };
        let Some(catalog_service_id) = service.catalog_service_id.as_deref() else {
            continue;
        };

        if !node_filter(node_id) {
            continue;
        }

        if let Some(node) = online_nodes.get(node_id)
            && is_node_viable(node, ws_manager)
        {
            service_ids.push(catalog_service_id.to_string());
        }
    }

    Ok(service_ids)
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
    service_ids.extend(load_viable_user_service_catalog_ids(db, user_id, ws_manager).await?);
    service_ids.sort();
    service_ids.dedup();
    Ok(service_ids)
}

/// Scope-aware variant: a `NodeServiceBinding` only counts as a viable
/// route when its `node_id` passes `node_filter`. The catalog `UserService`
/// contribution is kept because it reflects the user's own `node_id`
/// choice (the caller applies its own scope check separately).
///
/// Used by MCP discovery so scoped API keys don't see tools whose only
/// viable bindings are on nodes they can't reach (eighteenth-round
/// Codex P2).
pub async fn list_routable_service_ids_filtered<F>(
    db: &mongodb::Database,
    user_id: &str,
    ws_manager: &NodeWsManager,
    node_filter: F,
) -> AppResult<Vec<String>>
where
    F: Fn(&str) -> bool,
{
    let mut service_ids: Vec<String> = load_viable_bindings(db, user_id, None, ws_manager)
        .await?
        .into_iter()
        .filter(|binding| node_filter(&binding.node_id))
        .map(|binding| binding.service_id)
        .collect();
    // Pass the same scope filter into the user-service catalog path so
    // an out-of-scope `UserService.node_id` doesn't make a platform
    // tool look executable (twenty-third-round Codex P2). The caller's
    // `execute_tool` would later 403 on the primary node scope check.
    service_ids.extend(
        load_viable_user_service_catalog_ids_filtered(db, user_id, ws_manager, &node_filter)
            .await?,
    );
    service_ids.sort();
    service_ids.dedup();
    Ok(service_ids)
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::{
        build_node_route, collect_ordered_node_ids, has_routable_node_bindings,
        list_viable_binding_node_ids, resolve_node_route, user_service_has_explicit_node,
    };
    use crate::models::node::{COLLECTION_NAME as NODES, Node, NodeMetrics, NodeStatus};
    use crate::models::node_service_binding::{
        COLLECTION_NAME as NODE_SERVICE_BINDINGS, NodeServiceBinding,
    };
    use crate::models::org_membership::{COLLECTION_NAME as ORG_MEMBERSHIPS, OrgRole};
    use crate::models::user::{COLLECTION_NAME as USERS, UserType};
    use crate::models::user_service::COLLECTION_NAME as USER_SERVICES;
    use crate::services::node_ws_manager::NodeWsManager;
    use crate::test_utils::{connect_test_database, test_membership, test_user, test_user_service};

    fn binding(node_id: &str) -> NodeServiceBinding {
        NodeServiceBinding {
            id: uuid::Uuid::new_v4().to_string(),
            node_id: node_id.to_string(),
            user_id: "user-1".to_string(),
            service_id: "svc-1".to_string(),
            is_active: true,
            priority: 0,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn node(node_id: &str, owner_id: &str) -> Node {
        Node {
            id: node_id.to_string(),
            user_id: owner_id.to_string(),
            name: format!("test-node-{node_id}"),
            status: NodeStatus::Online,
            auth_token_hash: "auth-token-hash".to_string(),
            signing_secret_encrypted: None,
            signing_secret_hash: "signing-secret-hash".to_string(),
            last_heartbeat_at: None,
            connected_at: Some(Utc::now()),
            metadata: None,
            metrics: NodeMetrics::default(),
            is_active: true,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn connected_ws_manager(node_id: &str) -> NodeWsManager {
        let manager = NodeWsManager::new(30, 100);
        let (tx, _rx) = tokio::sync::mpsc::channel(8);
        manager.register_connection(node_id, tx);
        manager
    }

    async fn insert_actor_org_membership(
        db: &mongodb::Database,
        actor_id: &str,
        org_id: &str,
        role: OrgRole,
    ) {
        db.collection(USERS)
            .insert_one(test_user(actor_id, UserType::Person))
            .await
            .unwrap();
        db.collection(USERS)
            .insert_one(test_user(org_id, UserType::Org))
            .await
            .unwrap();
        db.collection(ORG_MEMBERSHIPS)
            .insert_one(test_membership(org_id, actor_id, role, None))
            .await
            .unwrap();
    }

    async fn insert_user_service(
        db: &mongodb::Database,
        service_id: &str,
        owner_id: &str,
        slug: &str,
        catalog_service_id: &str,
        node_id: Option<&str>,
    ) {
        let endpoint_id = uuid::Uuid::new_v4().to_string();
        db.collection(USER_SERVICES)
            .insert_one(test_user_service(
                service_id,
                owner_id,
                slug,
                &endpoint_id,
                Some(catalog_service_id),
                node_id,
            ))
            .await
            .unwrap();
    }

    async fn insert_node(db: &mongodb::Database, node_id: &str, owner_id: &str) {
        db.collection(NODES)
            .insert_one(node(node_id, owner_id))
            .await
            .unwrap();
    }

    async fn insert_node_binding(
        db: &mongodb::Database,
        node_id: &str,
        owner_id: &str,
        catalog_service_id: &str,
    ) {
        db.collection(NODE_SERVICE_BINDINGS)
            .insert_one(NodeServiceBinding {
                id: uuid::Uuid::new_v4().to_string(),
                node_id: node_id.to_string(),
                user_id: owner_id.to_string(),
                service_id: catalog_service_id.to_string(),
                is_active: true,
                priority: 0,
                created_at: Utc::now(),
                updated_at: Utc::now(),
            })
            .await
            .unwrap();
    }

    #[test]
    fn collect_ordered_node_ids_keeps_primary_first_and_dedups() {
        let node_ids = collect_ordered_node_ids(
            Some("node-primary".to_string()),
            vec![
                binding("node-primary"),
                binding("node-fallback"),
                binding("node-fallback"),
                binding("node-third"),
            ],
        );

        assert_eq!(
            node_ids,
            vec![
                "node-primary".to_string(),
                "node-fallback".to_string(),
                "node-third".to_string()
            ]
        );
    }

    #[test]
    fn build_node_route_returns_primary_and_fallbacks() {
        let route = build_node_route(vec![
            "node-primary".to_string(),
            "node-fallback".to_string(),
            "node-third".to_string(),
        ])
        .expect("route should exist");

        assert_eq!(route.node_id, "node-primary");
        assert_eq!(
            route.fallback_node_ids,
            vec!["node-fallback".to_string(), "node-third".to_string()]
        );
    }

    #[test]
    fn build_node_route_returns_none_for_empty_candidates() {
        assert!(build_node_route(vec![]).is_none());
    }

    #[tokio::test]
    async fn resolve_node_route_walks_to_org_owner_for_admin_membership() {
        let Some(db) = connect_test_database("node_route_org_admin").await else {
            eprintln!("skipping node routing integration test: no local MongoDB available");
            return;
        };
        let actor_id = uuid::Uuid::new_v4().to_string();
        let org_id = uuid::Uuid::new_v4().to_string();
        let service_id = uuid::Uuid::new_v4().to_string();
        let catalog_service_id = "catalog-ssh-admin";
        let node_id = "node-org-admin";
        insert_actor_org_membership(&db, &actor_id, &org_id, OrgRole::Admin).await;
        insert_user_service(
            &db,
            &service_id,
            &org_id,
            "routeros",
            catalog_service_id,
            Some(node_id),
        )
        .await;
        insert_node(&db, node_id, &org_id).await;
        let ws_manager = connected_ws_manager(node_id);

        let route = resolve_node_route(&db, &actor_id, catalog_service_id, &ws_manager)
            .await
            .unwrap()
            .expect("admin should route through org-owned node");

        assert_eq!(route.node_id, node_id);
        assert!(route.fallback_node_ids.is_empty());
    }

    #[tokio::test]
    async fn resolve_node_route_walks_to_org_owner_for_member_membership() {
        let Some(db) = connect_test_database("node_route_org_member").await else {
            eprintln!("skipping node routing integration test: no local MongoDB available");
            return;
        };
        let actor_id = uuid::Uuid::new_v4().to_string();
        let org_id = uuid::Uuid::new_v4().to_string();
        let service_id = uuid::Uuid::new_v4().to_string();
        let catalog_service_id = "catalog-ssh-member";
        let node_id = "node-org-member";
        insert_actor_org_membership(&db, &actor_id, &org_id, OrgRole::Member).await;
        insert_user_service(
            &db,
            &service_id,
            &org_id,
            "routeros",
            catalog_service_id,
            Some(node_id),
        )
        .await;
        insert_node(&db, node_id, &org_id).await;
        let ws_manager = connected_ws_manager(node_id);

        let route = resolve_node_route(&db, &actor_id, catalog_service_id, &ws_manager)
            .await
            .unwrap()
            .expect("member should route through org-owned node");

        assert_eq!(route.node_id, node_id);
    }

    #[tokio::test]
    async fn resolve_node_route_returns_none_for_viewer_membership() {
        let Some(db) = connect_test_database("node_route_org_viewer").await else {
            eprintln!("skipping node routing integration test: no local MongoDB available");
            return;
        };
        let actor_id = uuid::Uuid::new_v4().to_string();
        let org_id = uuid::Uuid::new_v4().to_string();
        let service_id = uuid::Uuid::new_v4().to_string();
        let catalog_service_id = "catalog-ssh-viewer";
        let node_id = "node-org-viewer";
        insert_actor_org_membership(&db, &actor_id, &org_id, OrgRole::Viewer).await;
        insert_user_service(
            &db,
            &service_id,
            &org_id,
            "routeros",
            catalog_service_id,
            Some(node_id),
        )
        .await;
        insert_node(&db, node_id, &org_id).await;
        let ws_manager = connected_ws_manager(node_id);

        let route = resolve_node_route(&db, &actor_id, catalog_service_id, &ws_manager)
            .await
            .unwrap();

        assert!(route.is_none());
    }

    #[tokio::test]
    async fn resolve_node_route_prefers_personal_service_over_org_service() {
        let Some(db) = connect_test_database("node_route_personal_first").await else {
            eprintln!("skipping node routing integration test: no local MongoDB available");
            return;
        };
        let actor_id = uuid::Uuid::new_v4().to_string();
        let org_id = uuid::Uuid::new_v4().to_string();
        let personal_service_id = uuid::Uuid::new_v4().to_string();
        let org_service_id = uuid::Uuid::new_v4().to_string();
        let catalog_service_id = "catalog-ssh-precedence";
        let personal_node_id = "node-personal";
        let org_node_id = "node-org-precedence";
        insert_actor_org_membership(&db, &actor_id, &org_id, OrgRole::Admin).await;
        insert_user_service(
            &db,
            &personal_service_id,
            &actor_id,
            "routeros",
            catalog_service_id,
            Some(personal_node_id),
        )
        .await;
        insert_user_service(
            &db,
            &org_service_id,
            &org_id,
            "routeros",
            catalog_service_id,
            Some(org_node_id),
        )
        .await;
        insert_node(&db, personal_node_id, &actor_id).await;
        insert_node(&db, org_node_id, &org_id).await;
        let ws_manager = connected_ws_manager(personal_node_id);
        let (tx, _rx) = tokio::sync::mpsc::channel(8);
        ws_manager.register_connection(org_node_id, tx);

        let route = resolve_node_route(&db, &actor_id, catalog_service_id, &ws_manager)
            .await
            .unwrap()
            .expect("personal service should route");

        assert_eq!(route.node_id, personal_node_id);
    }

    #[tokio::test]
    async fn routable_and_explicit_node_checks_walk_org_owner() {
        let Some(db) = connect_test_database("node_route_has_explicit_org").await else {
            eprintln!("skipping node routing integration test: no local MongoDB available");
            return;
        };
        let actor_id = uuid::Uuid::new_v4().to_string();
        let org_id = uuid::Uuid::new_v4().to_string();
        let service_id = uuid::Uuid::new_v4().to_string();
        let catalog_service_id = "catalog-ssh-has-explicit";
        let node_id = "node-org-explicit";
        insert_actor_org_membership(&db, &actor_id, &org_id, OrgRole::Member).await;
        insert_user_service(
            &db,
            &service_id,
            &org_id,
            "routeros",
            catalog_service_id,
            Some(node_id),
        )
        .await;
        insert_node(&db, node_id, &org_id).await;
        let ws_manager = connected_ws_manager(node_id);

        assert!(
            has_routable_node_bindings(&db, &actor_id, catalog_service_id, &ws_manager)
                .await
                .unwrap()
        );
        assert!(
            user_service_has_explicit_node(&db, &actor_id, catalog_service_id)
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn routable_and_explicit_node_checks_return_false_for_viewer() {
        let Some(db) = connect_test_database("node_route_has_explicit_viewer").await else {
            eprintln!("skipping node routing integration test: no local MongoDB available");
            return;
        };
        let actor_id = uuid::Uuid::new_v4().to_string();
        let org_id = uuid::Uuid::new_v4().to_string();
        let service_id = uuid::Uuid::new_v4().to_string();
        let catalog_service_id = "catalog-ssh-has-explicit-viewer";
        let node_id = "node-org-explicit-viewer";
        insert_actor_org_membership(&db, &actor_id, &org_id, OrgRole::Viewer).await;
        insert_user_service(
            &db,
            &service_id,
            &org_id,
            "routeros",
            catalog_service_id,
            Some(node_id),
        )
        .await;
        insert_node(&db, node_id, &org_id).await;
        let ws_manager = connected_ws_manager(node_id);

        assert!(
            !has_routable_node_bindings(&db, &actor_id, catalog_service_id, &ws_manager)
                .await
                .unwrap()
        );
        assert!(
            !user_service_has_explicit_node(&db, &actor_id, catalog_service_id)
                .await
                .unwrap()
        );
    }

    #[tokio::test]
    async fn list_viable_binding_node_ids_walks_org_owner() {
        let Some(db) = connect_test_database("node_route_list_bindings_org").await else {
            eprintln!("skipping node routing integration test: no local MongoDB available");
            return;
        };
        let actor_id = uuid::Uuid::new_v4().to_string();
        let org_id = uuid::Uuid::new_v4().to_string();
        let service_id = uuid::Uuid::new_v4().to_string();
        let catalog_service_id = "catalog-ssh-bindings";
        let node_id = "node-org-binding";
        insert_actor_org_membership(&db, &actor_id, &org_id, OrgRole::Member).await;
        insert_user_service(
            &db,
            &service_id,
            &org_id,
            "routeros",
            catalog_service_id,
            None,
        )
        .await;
        insert_node(&db, node_id, &org_id).await;
        insert_node_binding(&db, node_id, &org_id, catalog_service_id).await;
        let ws_manager = connected_ws_manager(node_id);

        let node_ids =
            list_viable_binding_node_ids(&db, &actor_id, catalog_service_id, &ws_manager)
                .await
                .unwrap();

        assert_eq!(node_ids, vec![node_id.to_string()]);
    }
}

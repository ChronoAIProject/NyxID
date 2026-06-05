use std::collections::{HashMap, HashSet};

use futures::TryStreamExt;
use mongodb::bson::{self, doc};

use crate::errors::{AppError, AppResult};
use crate::models::node::{COLLECTION_NAME as NODES, Node};
use crate::models::node_service_binding::{
    COLLECTION_NAME as NODE_SERVICE_BINDINGS, NodeServiceBinding,
};
use crate::models::user_service::{COLLECTION_NAME as USER_SERVICES, UserService};
use crate::services::org_service;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FanOutTarget {
    pub node_id: String,
}

pub async fn resolve_credential_fan_out_targets(
    db: &mongodb::Database,
    actor_user_id: &str,
    owner_user_id: &str,
    service_id: &str,
) -> AppResult<Vec<FanOutTarget>> {
    let primary_node_id = resolve_primary_user_service_node(db, owner_user_id, service_id).await?;
    let bindings = load_active_node_bindings(db, owner_user_id, service_id).await?;
    let ordered = collect_ordered_node_ids(primary_node_id, bindings);
    if ordered.is_empty() {
        return Err(AppError::Conflict(
            "no active node targets for service fan-out".to_string(),
        ));
    }

    let nodes = load_active_nodes(db, &ordered).await?;
    let mut targets = Vec::new();
    for node_id in ordered {
        let Some(node) = nodes.get(node_id.as_str()) else {
            continue;
        };
        let access = org_service::resolve_owner_access(db, actor_user_id, &node.user_id).await?;
        if !access.can_write() || !access.allows_resource(service_id) {
            return Err(AppError::Forbidden(
                "Not allowed to fan out credentials to one or more node targets".to_string(),
            ));
        }
        targets.push(FanOutTarget {
            node_id: node.id.clone(),
        });
    }

    if targets.is_empty() {
        return Err(AppError::Conflict(
            "no active node targets for service fan-out".to_string(),
        ));
    }
    Ok(targets)
}

async fn resolve_primary_user_service_node(
    db: &mongodb::Database,
    owner_user_id: &str,
    service_id: &str,
) -> AppResult<Option<String>> {
    let user_service = db
        .collection::<UserService>(USER_SERVICES)
        .find_one(doc! {
            "user_id": owner_user_id,
            "catalog_service_id": service_id,
            "is_active": true,
        })
        .await?;

    Ok(user_service
        .and_then(|service| service.node_id)
        .filter(|node_id| !node_id.is_empty()))
}

async fn load_active_node_bindings(
    db: &mongodb::Database,
    owner_user_id: &str,
    service_id: &str,
) -> AppResult<Vec<NodeServiceBinding>> {
    db.collection::<NodeServiceBinding>(NODE_SERVICE_BINDINGS)
        .find(doc! {
            "user_id": owner_user_id,
            "service_id": service_id,
            "is_active": true,
        })
        .sort(doc! { "priority": 1, "created_at": 1 })
        .await?
        .try_collect()
        .await
        .map_err(AppError::from)
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

async fn load_active_nodes(
    db: &mongodb::Database,
    ordered_node_ids: &[String],
) -> AppResult<HashMap<String, Node>> {
    let node_ids: bson::Array = ordered_node_ids
        .iter()
        .cloned()
        .map(bson::Bson::String)
        .collect();
    let nodes: Vec<Node> = db
        .collection::<Node>(NODES)
        .find(doc! {
            "_id": { "$in": node_ids },
            "is_active": true,
        })
        .await?
        .try_collect()
        .await?;

    Ok(nodes
        .into_iter()
        .map(|node| (node.id.clone(), node))
        .collect())
}

#[cfg(test)]
mod tests {
    use chrono::Utc;
    use uuid::Uuid;

    use super::resolve_credential_fan_out_targets;
    use crate::errors::AppError;
    use crate::models::node::{COLLECTION_NAME as NODES, Node, NodeMetrics, NodeStatus};
    use crate::models::node_service_binding::{
        COLLECTION_NAME as NODE_SERVICE_BINDINGS, NodeServiceBinding,
    };
    use crate::models::user::{COLLECTION_NAME as USERS, UserType};
    use crate::models::user_service::{COLLECTION_NAME as USER_SERVICES, UserService};
    use crate::test_utils::{connect_test_database, test_user, test_user_service};

    #[test]
    fn module_import_guard_excludes_online_routing_dependencies() {
        let source = include_str!("node_fanout_resolver.rs")
            .split("#[cfg(test)]")
            .next()
            .expect("production module source");
        assert!(!source.contains("NodeWsManager"));
        assert!(!source.contains("NodeRoute"));
        assert!(!source.contains("resolve_node_route"));
        assert!(!source.contains("node_routing_service"));
        assert!(!source.contains("NodeStatus"));
    }

    async fn test_db(prefix: &str) -> mongodb::Database {
        connect_test_database(prefix)
            .await
            .expect("local MongoDB required for fan-out resolver tests")
    }

    fn test_node(owner_id: &str, status: NodeStatus) -> Node {
        let now = Utc::now();
        Node {
            id: Uuid::new_v4().to_string(),
            user_id: owner_id.to_string(),
            name: "fanout-node".to_string(),
            status,
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

    fn test_binding(
        owner_id: &str,
        node_id: &str,
        service_id: &str,
        priority: i32,
    ) -> NodeServiceBinding {
        let now = Utc::now();
        NodeServiceBinding {
            id: Uuid::new_v4().to_string(),
            node_id: node_id.to_string(),
            user_id: owner_id.to_string(),
            service_id: service_id.to_string(),
            is_active: true,
            priority,
            created_at: now,
            updated_at: now,
        }
    }

    async fn insert_user_service(db: &mongodb::Database, service: UserService) {
        db.collection::<UserService>(USER_SERVICES)
            .insert_one(service)
            .await
            .expect("insert user service");
    }

    #[tokio::test]
    async fn resolver_includes_offline_active_nodes() {
        let db = test_db("fanout_resolver_offline").await;
        let owner = Uuid::new_v4().to_string();
        db.collection(USERS)
            .insert_one(test_user(&owner, UserType::Person))
            .await
            .expect("insert user");
        let node = test_node(&owner, NodeStatus::Offline);
        db.collection::<Node>(NODES)
            .insert_one(&node)
            .await
            .expect("insert node");
        db.collection::<NodeServiceBinding>(NODE_SERVICE_BINDINGS)
            .insert_one(test_binding(&owner, &node.id, "svc", 0))
            .await
            .expect("insert binding");

        let targets = resolve_credential_fan_out_targets(&db, &owner, &owner, "svc")
            .await
            .expect("resolve targets");

        assert_eq!(
            targets
                .iter()
                .map(|target| target.node_id.as_str())
                .collect::<Vec<_>>(),
            vec![node.id.as_str()]
        );
    }

    #[tokio::test]
    async fn resolver_excludes_inactive_bindings_and_nodes() {
        let db = test_db("fanout_resolver_inactive").await;
        let owner = Uuid::new_v4().to_string();
        db.collection(USERS)
            .insert_one(test_user(&owner, UserType::Person))
            .await
            .expect("insert user");
        let active_node = test_node(&owner, NodeStatus::Online);
        let mut inactive_node = test_node(&owner, NodeStatus::Offline);
        inactive_node.is_active = false;
        db.collection::<Node>(NODES)
            .insert_many([active_node.clone(), inactive_node.clone()])
            .await
            .expect("insert nodes");
        let mut inactive_binding = test_binding(&owner, &active_node.id, "svc", 0);
        inactive_binding.is_active = false;
        db.collection::<NodeServiceBinding>(NODE_SERVICE_BINDINGS)
            .insert_many([
                inactive_binding,
                test_binding(&owner, &inactive_node.id, "svc", 1),
            ])
            .await
            .expect("insert bindings");

        let err = resolve_credential_fan_out_targets(&db, &owner, &owner, "svc")
            .await
            .expect_err("no active target");
        assert!(matches!(err, AppError::Conflict(_)));
    }

    #[tokio::test]
    async fn resolver_dedupes_primary_and_bindings_in_priority_order() {
        let db = test_db("fanout_resolver_dedupe").await;
        let owner = Uuid::new_v4().to_string();
        db.collection(USERS)
            .insert_one(test_user(&owner, UserType::Person))
            .await
            .expect("insert user");
        let primary = test_node(&owner, NodeStatus::Offline);
        let secondary = test_node(&owner, NodeStatus::Offline);
        db.collection::<Node>(NODES)
            .insert_many([primary.clone(), secondary.clone()])
            .await
            .expect("insert nodes");
        insert_user_service(
            &db,
            test_user_service(
                "usvc",
                &owner,
                "slug",
                "endpoint",
                Some("svc"),
                Some(&primary.id),
            ),
        )
        .await;
        db.collection::<NodeServiceBinding>(NODE_SERVICE_BINDINGS)
            .insert_many([
                test_binding(&owner, &secondary.id, "svc", -10),
                test_binding(&owner, &primary.id, "svc", -20),
            ])
            .await
            .expect("insert bindings");

        let targets = resolve_credential_fan_out_targets(&db, &owner, &owner, "svc")
            .await
            .expect("resolve targets");

        assert_eq!(
            targets
                .iter()
                .map(|target| target.node_id.as_str())
                .collect::<Vec<_>>(),
            vec![primary.id.as_str(), secondary.id.as_str()]
        );
    }

    #[tokio::test]
    async fn resolver_denies_actor_without_per_node_owner_write_access() {
        let db = test_db("fanout_resolver_acl").await;
        let owner = Uuid::new_v4().to_string();
        let actor = Uuid::new_v4().to_string();
        db.collection(USERS)
            .insert_many([
                test_user(&owner, UserType::Person),
                test_user(&actor, UserType::Person),
            ])
            .await
            .expect("insert users");
        let node = test_node(&owner, NodeStatus::Offline);
        db.collection::<Node>(NODES)
            .insert_one(&node)
            .await
            .expect("insert node");
        db.collection::<NodeServiceBinding>(NODE_SERVICE_BINDINGS)
            .insert_one(test_binding(&owner, &node.id, "svc", 0))
            .await
            .expect("insert binding");

        let err = resolve_credential_fan_out_targets(&db, &actor, &owner, "svc")
            .await
            .expect_err("actor denied");

        assert!(matches!(err, AppError::Forbidden(_)));
    }
}

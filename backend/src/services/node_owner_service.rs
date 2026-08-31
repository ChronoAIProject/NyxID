use chrono::{DateTime, Duration, Utc};
use mongodb::bson::{self, doc};
use mongodb::options::{FindOneAndUpdateOptions, ReturnDocument};

use crate::errors::AppResult;
use crate::models::node::{COLLECTION_NAME as NODES, Node, NodeConnectionOwner, NodeStatus};
use crate::services::node_ws_manager::NodeCapabilitiesFlags;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplicaIdentity {
    pub instance_name: String,
    pub generation_id: String,
    pub internal_base_url: String,
}

impl ReplicaIdentity {
    pub fn new(instance_name: String, internal_base_url: String) -> Self {
        Self {
            instance_name,
            generation_id: uuid::Uuid::new_v4().to_string(),
            internal_base_url,
        }
    }

    #[cfg(test)]
    fn with_generation(
        instance_name: impl Into<String>,
        generation_id: impl Into<String>,
        internal_base_url: impl Into<String>,
    ) -> Self {
        Self {
            instance_name: instance_name.into(),
            generation_id: generation_id.into(),
            internal_base_url: internal_base_url.into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NodeOwnerFence {
    pub node_id: String,
    pub instance_name: String,
    pub generation_id: String,
    pub connection_id: String,
}

impl NodeOwnerFence {
    pub fn from_owner(node_id: impl Into<String>, owner: &NodeConnectionOwner) -> Self {
        Self {
            node_id: node_id.into(),
            instance_name: owner.instance_name.clone(),
            generation_id: owner.generation_id.clone(),
            connection_id: owner.connection_id.clone(),
        }
    }

    fn filter(&self) -> bson::Document {
        doc! {
            "_id": &self.node_id,
            "connection_owner.instance_name": &self.instance_name,
            "connection_owner.generation_id": &self.generation_id,
            "connection_owner.connection_id": &self.connection_id,
        }
    }
}

pub fn live_owner(node: &Node, now: DateTime<Utc>) -> Option<&NodeConnectionOwner> {
    if !node.is_active {
        return None;
    }
    node.connection_owner
        .as_ref()
        .filter(|owner| owner.is_live_at(now))
}

/// Whether a node has a live socket on any replica.
///
/// Once an owner record exists it is authoritative, including when expired.
/// The local-map fallback is only for sockets accepted by a pre-owner-record
/// replica during a rolling upgrade.
pub fn is_connected_somewhere(node: &Node, local_connected: bool, now: DateTime<Utc>) -> bool {
    if !node.is_active {
        return false;
    }
    match node.connection_owner.as_ref() {
        Some(owner) => owner.is_live_at(now),
        None => local_connected,
    }
}

pub async fn claim(
    db: &mongodb::Database,
    node_id: &str,
    identity: &ReplicaIdentity,
    connection_id: &str,
    lease_ttl: std::time::Duration,
) -> AppResult<Option<NodeConnectionOwner>> {
    let now = Utc::now();
    let expires_at =
        now + Duration::from_std(lease_ttl).unwrap_or_else(|_| Duration::seconds(i64::MAX / 4));
    let owner = NodeConnectionOwner {
        instance_name: identity.instance_name.clone(),
        generation_id: identity.generation_id.clone(),
        connection_id: connection_id.to_string(),
        internal_base_url: identity.internal_base_url.clone(),
        claimed_at: now,
        renewed_at: now,
        expires_at,
        credential_ack_correlation: false,
        remote_credential_crypto_v1: false,
        proxy_max_body_size: None,
        capabilities_resolved: false,
    };
    let owner_doc = bson::to_document(&owner).map_err(|error| {
        crate::errors::AppError::Internal(format!(
            "Failed to serialize node connection owner: {error}"
        ))
    })?;
    let now_bson = bson::DateTime::from_chrono(now);
    let options = FindOneAndUpdateOptions::builder()
        .return_document(ReturnDocument::After)
        .build();

    let claimed = db
        .collection::<Node>(NODES)
        .find_one_and_update(
            doc! {
                "_id": node_id,
                "is_active": true,
                "$or": [
                    { "connection_owner": { "$exists": false } },
                    { "connection_owner": null },
                    { "connection_owner.expires_at": { "$lte": &now_bson } },
                    {
                        "connection_owner.instance_name": &identity.instance_name,
                        "connection_owner.generation_id": &identity.generation_id,
                    },
                ],
            },
            doc! {
                "$set": {
                    "connection_owner": owner_doc,
                    "status": NodeStatus::Online.as_str(),
                    "connected_at": &now_bson,
                    "last_heartbeat_at": &now_bson,
                    "updated_at": &now_bson,
                },
            },
        )
        .with_options(options)
        .await?;

    Ok(claimed.and_then(|node| node.connection_owner))
}

pub async fn renew(
    db: &mongodb::Database,
    fence: &NodeOwnerFence,
    lease_ttl: std::time::Duration,
    observed_node_heartbeat: bool,
) -> AppResult<bool> {
    let now = Utc::now();
    let expires_at =
        now + Duration::from_std(lease_ttl).unwrap_or_else(|_| Duration::seconds(i64::MAX / 4));
    let now_bson = bson::DateTime::from_chrono(now);
    let expires_bson = bson::DateTime::from_chrono(expires_at);
    let mut set = doc! {
        "connection_owner.renewed_at": &now_bson,
        "connection_owner.expires_at": expires_bson,
        "updated_at": &now_bson,
    };
    if observed_node_heartbeat {
        set.insert("last_heartbeat_at", now_bson);
    }

    let result = db
        .collection::<Node>(NODES)
        .update_one(fence.filter(), doc! { "$set": set })
        .await?;
    Ok(result.matched_count == 1)
}

pub async fn record_capabilities(
    db: &mongodb::Database,
    fence: &NodeOwnerFence,
    capabilities: NodeCapabilitiesFlags,
    resolved: bool,
) -> AppResult<bool> {
    let now = bson::DateTime::from_chrono(Utc::now());
    let result = db
        .collection::<Node>(NODES)
        .update_one(
            fence.filter(),
            doc! {
                "$set": {
                    "connection_owner.credential_ack_correlation": capabilities.credential_ack_correlation,
                    "connection_owner.remote_credential_crypto_v1": capabilities.remote_credential_crypto_v1,
                    "connection_owner.proxy_max_body_size": capabilities.proxy_max_body_size.map(|value| value as i64),
                    "connection_owner.capabilities_resolved": resolved,
                    "updated_at": now,
                },
            },
        )
        .await?;
    Ok(result.matched_count == 1)
}

pub async fn release(db: &mongodb::Database, fence: &NodeOwnerFence) -> AppResult<bool> {
    let now = bson::DateTime::from_chrono(Utc::now());
    let result = db
        .collection::<Node>(NODES)
        .update_one(
            fence.filter(),
            doc! {
                "$unset": { "connection_owner": "" },
                "$set": {
                    "status": NodeStatus::Offline.as_str(),
                    "updated_at": now,
                },
            },
        )
        .await?;
    Ok(result.matched_count == 1)
}

pub async fn clear_expired(db: &mongodb::Database, now: DateTime<Utc>) -> AppResult<u64> {
    let now_bson = bson::DateTime::from_chrono(now);
    let result = db
        .collection::<Node>(NODES)
        .update_many(
            doc! { "connection_owner.expires_at": { "$lte": &now_bson } },
            doc! {
                "$unset": { "connection_owner": "" },
                "$set": {
                    "status": NodeStatus::Offline.as_str(),
                    "updated_at": &now_bson,
                },
            },
        )
        .await?;
    Ok(result.modified_count)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::node::{NodeMetadata, NodeMetrics};

    fn node(id: &str) -> Node {
        let now = Utc::now();
        Node {
            id: id.to_string(),
            user_id: uuid::Uuid::new_v4().to_string(),
            name: "owner-test".to_string(),
            status: NodeStatus::Offline,
            auth_token_hash: "hash".to_string(),
            signing_secret_encrypted: None,
            signing_secret_hash: "signing-hash".to_string(),
            last_heartbeat_at: None,
            connected_at: None,
            metadata: None::<NodeMetadata>,
            metrics: NodeMetrics::default(),
            connection_owner: None,
            is_active: true,
            created_at: now,
            updated_at: now,
        }
    }

    #[tokio::test]
    async fn stale_owner_can_be_reclaimed_and_old_fence_cannot_release_replacement() {
        let Some(db) = crate::test_utils::connect_test_database("node_owner_fence").await else {
            return;
        };
        let node_id = uuid::Uuid::new_v4().to_string();
        db.collection::<Node>(NODES)
            .insert_one(node(&node_id))
            .await
            .unwrap();
        let identity_a =
            ReplicaIdentity::with_generation("backend-0", "generation-a", "http://10.0.0.1:3002");
        let identity_b =
            ReplicaIdentity::with_generation("backend-1", "generation-b", "http://10.0.0.2:3002");

        let owner_a = claim(
            &db,
            &node_id,
            &identity_a,
            "connection-a",
            std::time::Duration::from_secs(30),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(
            claim(
                &db,
                &node_id,
                &identity_b,
                "connection-b",
                std::time::Duration::from_secs(30),
            )
            .await
            .unwrap()
            .is_none()
        );

        db.collection::<Node>(NODES)
            .update_one(
                doc! { "_id": &node_id },
                doc! { "$set": { "connection_owner.expires_at": bson::DateTime::from_chrono(Utc::now() - Duration::seconds(1)) } },
            )
            .await
            .unwrap();
        let owner_b = claim(
            &db,
            &node_id,
            &identity_b,
            "connection-b",
            std::time::Duration::from_secs(30),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(
            !release(&db, &NodeOwnerFence::from_owner(&node_id, &owner_a))
                .await
                .unwrap()
        );
        let stored = db
            .collection::<Node>(NODES)
            .find_one(doc! { "_id": &node_id })
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            stored.connection_owner.unwrap().connection_id,
            owner_b.connection_id
        );
    }

    #[tokio::test]
    async fn reconnect_in_same_generation_fences_old_connection() {
        let Some(db) = crate::test_utils::connect_test_database("node_owner_reconnect").await
        else {
            return;
        };
        let node_id = uuid::Uuid::new_v4().to_string();
        db.collection::<Node>(NODES)
            .insert_one(node(&node_id))
            .await
            .unwrap();
        let identity =
            ReplicaIdentity::with_generation("backend-0", "generation-a", "http://10.0.0.1:3002");
        let first = claim(
            &db,
            &node_id,
            &identity,
            "connection-a",
            std::time::Duration::from_secs(30),
        )
        .await
        .unwrap()
        .unwrap();
        let second = claim(
            &db,
            &node_id,
            &identity,
            "connection-b",
            std::time::Duration::from_secs(30),
        )
        .await
        .unwrap()
        .unwrap();

        assert!(
            !renew(
                &db,
                &NodeOwnerFence::from_owner(&node_id, &first),
                std::time::Duration::from_secs(30),
                false,
            )
            .await
            .unwrap()
        );
        assert!(
            renew(
                &db,
                &NodeOwnerFence::from_owner(&node_id, &second),
                std::time::Duration::from_secs(30),
                false,
            )
            .await
            .unwrap()
        );
    }
}

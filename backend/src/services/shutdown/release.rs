//! Releases use process generations; names alone never identify an owner.
use futures::{StreamExt, TryStreamExt};
use mongodb::{
    Database,
    bson::{Document, doc},
};

use crate::{
    AppState,
    errors::AppResult,
    models::coordination::{LEASE_COLLECTION_NAME, SLOT_COLLECTION_NAME},
};

/// These constants are also used at each owner's acquisition site.
pub(super) const CLUSTER_LEASES: &[&str] = &[
    super::super::telegram_poller::TELEGRAM_POLLER_LEASE_NAME,
    super::super::assistant_upload_retention::LEASE,
    super::super::machine_access_service::RENEWAL_LEASE,
    super::super::machine_access_service::DISPATCH_LEASE,
    super::super::machine_access_service::MIGRATION_LEASE,
    super::super::machine_access_service::EXTERNAL_CLEANUP_LEASE,
];

pub async fn cluster(
    db: &Database,
    identity: &super::super::node_owner_service::ReplicaIdentity,
) -> AppResult<u64> {
    let result = db
        .collection::<Document>(LEASE_COLLECTION_NAME)
        .update_many(
            doc! {
                "holder.instance_id": &identity.instance_name,
                "holder.generation_id": &identity.generation_id,
                // Only stopped ownership/maintenance. billing:reconcile releases
                // its exact token in its wrapper after the active unit completes;
                // deadline abort leaves its 30s default TTL. Provider mutation
                // locks (OAuth, channel webhook, Telegram setup), financial row
                // locks and ambiguous delivery claims retain their own recovery
                // protocols. Discord identify is a rate window, not ownership.
                // Credit-schedule progress has no exact token; its 300s lease
                // expires normally rather than releasing a successor's walk.
                "$or": [
                    {"_id": {"$in": CLUSTER_LEASES}},
                    {"_id": {"$regex": format!("^{}", regex::escape(super::super::discord_gateway::SOCKET_LEASE_PREFIX))}},
                ],
            },
            doc! {"$set": {"expires_at": bson::DateTime::from_millis(0)}},
        )
        .await?;
    Ok(result.modified_count)
}

pub async fn slots(
    db: &Database,
    identity: &super::super::node_owner_service::ReplicaIdentity,
) -> AppResult<u64> {
    Ok(db
        .collection::<Document>(SLOT_COLLECTION_NAME)
        .delete_many(doc! {
            "holder.instance_id": &identity.instance_name,
            "holder.generation_id": &identity.generation_id,
        })
        .await?
        .deleted_count)
}

pub async fn registered(runtime: &super::Runtime) -> AppResult<u64> {
    let mut releases = futures::stream::iter(runtime.releases())
        .filter(|release| std::future::ready(release.safe_after_stop))
        .map(|release| async move {
            release
                .db
                .collection::<Document>(release.collection)
                .update_one(release.filter, release.update)
                .await
        })
        .buffer_unordered(16);
    let mut count = 0;
    let mut failed = false;
    while let Some(result) = releases.next().await {
        match result {
            Ok(result) => count += result.modified_count,
            Err(_) => failed = true,
        }
    }
    if failed {
        tracing::warn!("Some fenced shutdown releases failed");
    }
    Ok(count)
}

/// Call only after stopping and joining the observation writer.
pub async fn observation(
    db: &Database,
    identity: &super::super::node_owner_service::ReplicaIdentity,
) -> AppResult<u64> {
    Ok(db
        .collection::<Document>(crate::models::security_replica::COLLECTION_NAME)
        .delete_one(doc! {
            "_id": &identity.generation_id,
            "generation_id": &identity.generation_id,
            "instance_name": &identity.instance_name,
        })
        .await?
        .deleted_count)
}

pub async fn nodes(state: &AppState) -> AppResult<u64> {
    // A claim can finish before its socket is published locally. Admission
    // stops at `stop`, but already admitted sockets must still reach teardown.
    loop {
        futures::stream::iter(state.node_ws_manager.connected_nodes())
            .for_each_concurrent(16, |(node, connection)| async move {
                state
                    .node_ws_manager
                    .disconnect_connection_if(&node, &connection, 1001, "server shutdown")
                    .await;
            })
            .await;
        if tokio::time::timeout(
            std::time::Duration::from_millis(25),
            state.shutdown.node_sockets_closed(),
        )
        .await
        .is_ok()
        {
            break;
        }
    }
    // Signalling close is not proof that a reader/writer stopped. Only after
    // their normal teardown may we retry remaining generation/connection fences.
    let identity = &state.replica_identity;
    let nodes: Vec<crate::models::node::Node> = state
        .db
        .collection(crate::models::node::COLLECTION_NAME)
        .find(doc! {
            "connection_owner.instance_name": &identity.instance_name,
            "connection_owner.generation_id": &identity.generation_id,
        })
        .await?
        .try_collect()
        .await?;
    let fences: Vec<_> = nodes
        .into_iter()
        .filter_map(|node| {
            node.connection_owner.map(|owner| {
                super::super::node_owner_service::NodeOwnerFence::from_owner(&node.id, &owner)
            })
        })
        .collect();
    let mut results = futures::stream::iter(fences)
        .map(|fence| async move {
            super::super::node_owner_service::release(&state.db, &fence)
                .await
                .map(u64::from)
        })
        .buffer_unordered(16);
    let mut released = 0;
    while let Some(result) = results.next().await {
        released += result?;
    }
    Ok(released)
}

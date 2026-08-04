use std::collections::HashMap;
use std::fmt;

use chrono::{DateTime, Duration, Utc};
use futures::TryStreamExt;
use mongodb::bson::{self, doc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zeroize::Zeroizing;

use crate::crypto::aes::EncryptionKeys;
use crate::crypto::token::hash_token;
use crate::errors::{AppError, AppResult};
use crate::models::node::{COLLECTION_NAME as NODES, Node, NodeMetadata, NodeStatus};
use crate::models::node_pending_credential::{
    COLLECTION_NAME as NODE_PENDING_CREDENTIALS, NodePendingCredential,
};
use crate::models::node_registration_token::{
    COLLECTION_NAME as NODE_REG_TOKENS, NodeRegistrationToken,
};
use crate::models::node_service_binding::{
    COLLECTION_NAME as NODE_SERVICE_BINDINGS, NodeServiceBinding,
};
use crate::models::org_membership::OrgRole;
use crate::models::user::{COLLECTION_NAME as USERS, User, UserType};
use crate::models::user_service::{COLLECTION_NAME as USER_SERVICES, UserService};
use crate::redaction::RedactedLen;
use crate::services::org_service::{self, OwnerAccess};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NodeOwnerKind {
    User,
    Org,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeOwnerInfo {
    pub kind: NodeOwnerKind,
    pub id: String,
    pub display_name: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NodeWithOwner {
    pub node: Node,
    pub owner: NodeOwnerInfo,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NodeAdminRole {
    Owner,
    Admin,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeAdminInfo {
    pub user_id: String,
    pub display_name: Option<String>,
    pub email: Option<String>,
    pub role: NodeAdminRole,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TransferNodeResult {
    pub node_id: String,
    pub previous_owner_user_id: String,
    pub new_owner_user_id: String,
    pub deactivated_bindings_count: u64,
    pub cleared_user_service_count: u64,
    pub deactivated_pending_credentials_count: u64,
}

#[derive(Clone)]
pub struct DeviceNodeInput<'a> {
    pub user_id: &'a str,
    pub api_key_id: &'a str,
    pub hw_id: &'a str,
    pub label: &'a str,
    pub device_pubkey: Option<&'a [u8; 32]>,
    pub provisioning_source: &'a str,
}

impl fmt::Debug for DeviceNodeInput<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DeviceNodeInput")
            .field("user_id", &self.user_id)
            .field("api_key_id", &RedactedLen(self.api_key_id.len()))
            .field("hw_id", &self.hw_id)
            .field("label", &self.label)
            .field(
                "device_pubkey",
                &self.device_pubkey.map(|pubkey| RedactedLen(pubkey.len())),
            )
            .field("provisioning_source", &self.provisioning_source)
            .finish()
    }
}

pub const DEVICE_CODE_PROVISIONING_SOURCE: &str = "device-code";
pub const DEVICE_ONBOARD_PROVISIONING_SOURCE: &str = "device-onboard";
const DEVICE_PROVISIONING_SOURCES: &[&str] = &[
    DEVICE_CODE_PROVISIONING_SOURCE,
    DEVICE_ONBOARD_PROVISIONING_SOURCE,
];

/// Create a one-time registration token for a new node.
/// Returns (token_id, raw_token, expires_at). The raw token is shown once and never stored.
pub async fn create_registration_token(
    db: &mongodb::Database,
    user_id: &str,
    name: &str,
    max_nodes_per_user: u32,
    ttl_secs: i64,
) -> AppResult<(String, String, DateTime<Utc>)> {
    // Validate name
    if name.is_empty() || name.len() > 64 {
        return Err(AppError::ValidationError(
            "Node name must be 1-64 characters".to_string(),
        ));
    }

    if !name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        return Err(AppError::ValidationError(
            "Node name must contain only lowercase letters, digits, and hyphens".to_string(),
        ));
    }

    // Check max nodes limit
    let existing_count = db
        .collection::<Node>(NODES)
        .count_documents(doc! { "user_id": user_id, "is_active": true })
        .await?;

    if existing_count >= max_nodes_per_user as u64 {
        return Err(AppError::BadRequest(format!(
            "Maximum of {max_nodes_per_user} nodes per user reached"
        )));
    }

    // Check if node name already exists for this user
    let existing_name = db
        .collection::<Node>(NODES)
        .find_one(doc! { "user_id": user_id, "name": name, "is_active": true })
        .await?;

    if existing_name.is_some() {
        return Err(AppError::Conflict(format!(
            "A node with name '{name}' already exists"
        )));
    }

    // Generate token
    let raw_token = format!("nyx_nreg_{}", hex::encode(rand::random::<[u8; 32]>()));
    let token_hash = hash_token(&raw_token);
    let now = Utc::now();
    let expires_at = now + Duration::seconds(ttl_secs);
    let token_id = uuid::Uuid::new_v4().to_string();

    let token = NodeRegistrationToken {
        id: token_id.clone(),
        user_id: user_id.to_string(),
        token_hash,
        name: name.to_string(),
        used: false,
        expires_at,
        created_at: now,
    };

    db.collection::<NodeRegistrationToken>(NODE_REG_TOKENS)
        .insert_one(&token)
        .await?;

    Ok((token_id, raw_token, expires_at))
}

/// Consume a registration token and create a new Node record.
/// Returns (Node, raw_auth_token, raw_signing_secret).
pub async fn register_node(
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
    raw_token: &str,
    metadata: Option<NodeMetadata>,
) -> AppResult<(Node, String, String)> {
    let token_hash = hash_token(raw_token);
    let now = Utc::now();

    // Find and consume the token atomically
    let token = db
        .collection::<NodeRegistrationToken>(NODE_REG_TOKENS)
        .find_one_and_update(
            doc! {
                "token_hash": &token_hash,
                "used": false,
                "expires_at": { "$gt": bson::DateTime::from_chrono(now) },
            },
            doc! {
                "$set": { "used": true },
            },
        )
        .await?
        .ok_or_else(|| {
            AppError::NodeRegistrationFailed("Invalid or expired registration token".to_string())
        })?;

    // Validate metadata fields if present
    if let Some(ref meta) = metadata {
        validate_node_metadata(meta)?;
    }

    // Generate auth token for the node
    let raw_auth_token = format!("nyx_nauth_{}", hex::encode(rand::random::<[u8; 32]>()));
    let auth_token_hash = hash_token(&raw_auth_token);

    // Generate HMAC signing secret
    let raw_signing_secret = hex::encode(rand::random::<[u8; 32]>());
    let signing_secret_encrypted = Some(
        encryption_keys
            .encrypt(raw_signing_secret.as_bytes())
            .await?,
    );
    let signing_secret_hash = hash_token(&raw_signing_secret);

    let node = Node {
        id: uuid::Uuid::new_v4().to_string(),
        user_id: token.user_id,
        name: token.name,
        status: NodeStatus::Online,
        auth_token_hash,
        signing_secret_encrypted,
        signing_secret_hash,
        last_heartbeat_at: Some(now),
        connected_at: Some(now),
        metadata,
        metrics: crate::models::node::NodeMetrics::default(),
        is_active: true,
        created_at: now,
        updated_at: now,
    };

    db.collection::<Node>(NODES).insert_one(&node).await?;

    tracing::info!(
        node_id = %node.id,
        user_id = %node.user_id,
        name = %node.name,
        "Node registered"
    );

    Ok((node, raw_auth_token, raw_signing_secret))
}

/// Create a Node row for an API-key-provisioned headless device.
///
/// Device-code and QR-onboarded devices receive a scoped NyxID API key and
/// authenticate through the proxy API, not the node WebSocket protocol. The row
/// still carries valid random node auth material so it satisfies the Node model
/// invariants and cannot collide on an empty auth hash.
pub async fn create_for_device(
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
    input: DeviceNodeInput<'_>,
) -> AppResult<Node> {
    if input.hw_id.trim().is_empty() || input.hw_id.len() > 256 {
        return Err(AppError::ValidationError(
            "hw_id must be between 1 and 256 characters".to_string(),
        ));
    }
    if input.label.trim().is_empty() || input.label.len() > 200 {
        return Err(AppError::ValidationError(
            "Device label must be between 1 and 200 characters".to_string(),
        ));
    }
    if input.provisioning_source.trim().is_empty() || input.provisioning_source.len() > 64 {
        return Err(AppError::ValidationError(
            "Device provisioning source must be between 1 and 64 characters".to_string(),
        ));
    }

    let now = Utc::now();
    let node_id = uuid::Uuid::new_v4().to_string();
    let pubkey_fingerprint = input.device_pubkey.map(|device_pubkey| {
        let pubkey_digest = Sha256::digest(device_pubkey);
        hex::encode(&pubkey_digest[..8])
    });
    let raw_auth_token = Zeroizing::new(format!(
        "nyx_nauth_{}",
        hex::encode(rand::random::<[u8; 32]>())
    ));
    let auth_token_hash = hash_token(raw_auth_token.as_str());
    let raw_signing_secret = Zeroizing::new(hex::encode(rand::random::<[u8; 32]>()));
    let signing_secret_encrypted = encryption_keys
        .encrypt(raw_signing_secret.as_bytes())
        .await?;
    let signing_secret_hash = hash_token(raw_signing_secret.as_str());

    let node = Node {
        id: node_id.clone(),
        user_id: input.user_id.to_string(),
        name: device_node_name(input.label, &node_id),
        status: NodeStatus::Offline,
        auth_token_hash,
        signing_secret_encrypted: Some(signing_secret_encrypted),
        signing_secret_hash,
        last_heartbeat_at: None,
        connected_at: None,
        metadata: Some(NodeMetadata {
            agent_version: None,
            os: None,
            arch: None,
            ip_address: None,
            provisioning_source: Some(input.provisioning_source.to_string()),
        }),
        metrics: crate::models::node::NodeMetrics::default(),
        is_active: true,
        created_at: now,
        updated_at: now,
    };

    db.collection::<Node>(NODES).insert_one(&node).await?;

    tracing::info!(
        node_id = %node.id,
        user_id = %node.user_id,
        api_key_id = %input.api_key_id,
        hw_id = %input.hw_id,
        device_pubkey_fingerprint = pubkey_fingerprint.as_deref(),
        provisioning_source = %input.provisioning_source,
        "Device node record created"
    );

    Ok(node)
}

/// Get a single node by ID without ownership check.
/// Used internally (e.g., heartbeat sweep).
pub async fn get_node_by_id(db: &mongodb::Database, node_id: &str) -> AppResult<Option<Node>> {
    let node = db
        .collection::<Node>(NODES)
        .find_one(doc! { "_id": node_id, "is_active": true })
        .await?;
    Ok(node)
}

/// Get multiple active nodes by their IDs without ownership checks.
pub async fn get_nodes_by_ids(db: &mongodb::Database, node_ids: &[String]) -> AppResult<Vec<Node>> {
    if node_ids.is_empty() {
        return Ok(Vec::new());
    }
    let filter = doc! {
        "_id": { "$in": node_ids },
        "is_active": true
    };
    let cursor = db.collection::<Node>(NODES).find(filter).await?;
    let nodes: Vec<Node> = cursor.try_collect().await?;
    Ok(nodes)
}

/// Decrypt and decode a node's HMAC signing secret.
pub async fn get_node_signing_secret(
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
    node_id: &str,
) -> AppResult<Zeroizing<Vec<u8>>> {
    let Some(node) = get_node_by_id(db, node_id).await? else {
        return Err(AppError::NodeNotFound(format!(
            "Node {node_id} not found during request signing"
        )));
    };

    let Some(encrypted_secret) = node.signing_secret_encrypted.as_deref() else {
        return Err(AppError::NodeOffline(format!(
            "Node {node_id} is missing its signing secret"
        )));
    };

    let decrypted_secret = Zeroizing::new(
        encryption_keys
            .decrypt(encrypted_secret)
            .await
            .map_err(|e| {
                AppError::Internal(format!(
                    "Failed to decrypt node signing secret for {node_id}: {e}"
                ))
            })?,
    );

    decode_node_signing_secret(&decrypted_secret, node_id)
}

/// Get a single node by ID, verifying ownership.
pub async fn get_node(db: &mongodb::Database, user_id: &str, node_id: &str) -> AppResult<Node> {
    let node = load_active_node(db, node_id).await?;
    ensure_node_readable_by_access(db, user_id, &node).await?;
    Ok(node)
}

/// Look up a node and verify the actor has write access to it -- either as
/// the direct owner or as an admin of the org that owns it.
///
/// Used by `user_service_service::create_user_service` and
/// `update_user_service` so that an admin can route an org-owned service
/// through their personal node (where they're the direct owner) without
/// having to also re-register the node under the org. The check is
/// actor-based rather than service-owner based: it's the human (or API
/// key) making the request who needs node access, not the service's
/// effective owner.
///
/// Returns `NodeNotFound` for any of: missing row, inactive node, or
/// actor without write access (no metadata leak).
pub async fn ensure_node_writable_by_actor(
    db: &mongodb::Database,
    actor_user_id: &str,
    node_id: &str,
) -> AppResult<Node> {
    let node = db
        .collection::<Node>(NODES)
        .find_one(doc! { "_id": node_id, "is_active": true })
        .await?
        .ok_or_else(|| AppError::NodeNotFound("Node not found".to_string()))?;

    ensure_node_writable_by_access(db, actor_user_id, &node).await?;
    Ok(node)
}

/// List all active nodes reachable by a user, including org-owned nodes for
/// orgs where the actor is an admin or member.
pub async fn list_user_nodes(
    db: &mongodb::Database,
    user_id: &str,
) -> AppResult<Vec<NodeWithOwner>> {
    let memberships = org_service::list_memberships_for_member(db, user_id, false).await?;
    let mut owner_ids = vec![user_id.to_string()];
    for membership in memberships {
        if membership.role.can_proxy() && !owner_ids.iter().any(|id| id == &membership.org_user_id)
        {
            owner_ids.push(membership.org_user_id);
        }
    }

    let owner_id_array: bson::Array = owner_ids
        .iter()
        .map(|id| bson::Bson::String(id.clone()))
        .collect();
    let nodes: Vec<Node> = db
        .collection::<Node>(NODES)
        .find(doc! { "user_id": { "$in": owner_id_array }, "is_active": true })
        .sort(doc! { "created_at": -1 })
        .await?
        .try_collect()
        .await?;
    attach_owner_info(db, nodes).await
}

/// Transfer an active node to another owner and detach owner-scoped routing state.
pub async fn transfer_node_owner(
    db: &mongodb::Database,
    actor_user_id: &str,
    node_id: &str,
    new_owner_user_id: &str,
    max_nodes_per_user: u32,
) -> AppResult<TransferNodeResult> {
    let node = load_active_node(db, node_id).await?;
    ensure_node_writable_by_access(db, actor_user_id, &node).await?;

    if new_owner_user_id == node.user_id {
        return Err(AppError::BadRequest(
            "node already belongs to that owner".to_string(),
        ));
    }

    let destination_access =
        org_service::resolve_owner_access(db, actor_user_id, new_owner_user_id).await?;
    if !destination_access.can_write() {
        return Err(AppError::Forbidden(
            "You must be the destination owner or an admin of the destination org".to_string(),
        ));
    }

    let destination_name_collision = db
        .collection::<Node>(NODES)
        .find_one(doc! {
            "user_id": new_owner_user_id,
            "name": &node.name,
            "is_active": true,
        })
        .await?;
    if destination_name_collision.is_some() {
        return Err(AppError::BadRequest(format!(
            "An active node named '{}' already exists for the destination owner. Rename one of them and retry.",
            node.name
        )));
    }

    let destination_node_count = db
        .collection::<Node>(NODES)
        .count_documents(doc! {
            "user_id": new_owner_user_id,
            "is_active": true,
        })
        .await?;
    if destination_node_count >= max_nodes_per_user as u64 {
        return Err(AppError::BadRequest(format!(
            "Maximum of {max_nodes_per_user} nodes per user reached"
        )));
    }

    let now = bson::DateTime::from_chrono(Utc::now());

    // This deployment path does not use multi-document transactions elsewhere.
    // Cleanup runs first so a partial failure leaves the source owner with
    // stale-but-recoverable state, never cross-tenant routing through a node
    // whose owner has already changed.
    let pending_credential_result = db
        .collection::<NodePendingCredential>(NODE_PENDING_CREDENTIALS)
        .update_many(
            doc! { "node_id": node_id, "is_active": true },
            doc! { "$set": { "is_active": false } },
        )
        .await?;

    let service_result = db
        .collection::<UserService>(USER_SERVICES)
        .update_many(
            doc! {
                "node_id": node_id,
                "user_id": { "$ne": new_owner_user_id },
                "is_active": true,
            },
            doc! {
                "$unset": { "node_id": "" },
                "$set": { "updated_at": &now },
            },
        )
        .await?;

    let binding_result = db
        .collection::<NodeServiceBinding>(NODE_SERVICE_BINDINGS)
        .update_many(
            doc! { "node_id": node_id, "is_active": true },
            doc! { "$set": { "is_active": false, "updated_at": &now } },
        )
        .await?;

    let update_result = db
        .collection::<Node>(NODES)
        .update_one(
            doc! {
                "_id": node_id,
                "user_id": &node.user_id,
                "is_active": true,
            },
            doc! {
                "$set": {
                    "user_id": new_owner_user_id,
                    "updated_at": &now,
                }
            },
        )
        .await?;
    if update_result.matched_count == 0 {
        return Err(AppError::NodeNotFound("Node not found".to_string()));
    }

    Ok(TransferNodeResult {
        node_id: node_id.to_string(),
        previous_owner_user_id: node.user_id,
        new_owner_user_id: new_owner_user_id.to_string(),
        deactivated_bindings_count: binding_result.modified_count,
        cleared_user_service_count: service_result.modified_count,
        deactivated_pending_credentials_count: pending_credential_result.modified_count,
    })
}

/// Soft-delete a node and its bindings.
pub async fn delete_node(db: &mongodb::Database, user_id: &str, node_id: &str) -> AppResult<()> {
    let now = bson::DateTime::from_chrono(Utc::now());
    let node = load_active_node(db, node_id).await?;
    ensure_node_writable_by_access(db, user_id, &node).await?;

    let result = db
        .collection::<Node>(NODES)
        .update_one(
            doc! { "_id": node_id, "is_active": true },
            doc! { "$set": { "is_active": false, "status": NodeStatus::Offline.as_str(), "updated_at": &now } },
        )
        .await?;

    if result.matched_count == 0 {
        return Err(AppError::NodeNotFound("Node not found".to_string()));
    }

    // Deactivate all bindings for this node
    db.collection::<NodeServiceBinding>(NODE_SERVICE_BINDINGS)
        .update_many(
            doc! { "node_id": node_id },
            doc! { "$set": { "is_active": false, "updated_at": &now } },
        )
        .await?;

    tracing::info!(node_id = %node_id, "Node deleted");
    Ok(())
}

/// Validate NodeMetadata string field lengths (M4).
fn validate_node_metadata(meta: &NodeMetadata) -> AppResult<()> {
    const MAX_METADATA_FIELD_LEN: usize = 64;
    if meta
        .agent_version
        .as_ref()
        .is_some_and(|v| v.len() > MAX_METADATA_FIELD_LEN)
    {
        return Err(AppError::ValidationError(
            "agent_version must be 64 characters or fewer".to_string(),
        ));
    }
    if meta
        .os
        .as_ref()
        .is_some_and(|v| v.len() > MAX_METADATA_FIELD_LEN)
    {
        return Err(AppError::ValidationError(
            "os must be 64 characters or fewer".to_string(),
        ));
    }
    if meta
        .arch
        .as_ref()
        .is_some_and(|v| v.len() > MAX_METADATA_FIELD_LEN)
    {
        return Err(AppError::ValidationError(
            "arch must be 64 characters or fewer".to_string(),
        ));
    }
    if let Some(ref ip) = meta.ip_address
        && ip.parse::<std::net::IpAddr>().is_err()
    {
        return Err(AppError::ValidationError(
            "Invalid IP address format".to_string(),
        ));
    }
    Ok(())
}

fn device_node_name(label: &str, node_id: &str) -> String {
    let suffix: String = node_id.chars().take(8).collect();
    let max_base_len = 64_usize.saturating_sub(suffix.len() + 1);
    let mut base = String::with_capacity(max_base_len);
    let mut last_dash = false;

    for c in label.chars() {
        if c.is_ascii_alphanumeric() {
            if base.len() == max_base_len {
                break;
            }
            base.push(c.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash && !base.is_empty() {
            if base.len() == max_base_len {
                break;
            }
            base.push('-');
            last_dash = true;
        }
    }

    let base = base.trim_matches('-');
    let base = if base.is_empty() { "device" } else { base };
    format!("{base}-{suffix}")
}

/// Update last_heartbeat_at and optionally metadata.
pub async fn update_heartbeat(
    db: &mongodb::Database,
    node_id: &str,
    metadata: Option<NodeMetadata>,
) -> AppResult<()> {
    let now = bson::DateTime::from_chrono(Utc::now());

    let mut update = doc! {
        "$set": {
            "last_heartbeat_at": &now,
            "updated_at": &now,
        },
    };

    if let Some(ref meta) = metadata {
        validate_node_metadata(meta)?;
    }

    if let Some(meta) = metadata {
        let meta_doc = bson::to_document(&meta)
            .map_err(|e| AppError::Internal(format!("Failed to serialize metadata: {e}")))?;
        update
            .get_document_mut("$set")
            .unwrap()
            .insert("metadata", meta_doc);
    }

    db.collection::<Node>(NODES)
        .update_one(doc! { "_id": node_id, "is_active": true }, update)
        .await?;

    Ok(())
}

/// Set node status.
pub async fn set_node_status(
    db: &mongodb::Database,
    node_id: &str,
    status: NodeStatus,
) -> AppResult<()> {
    let now = bson::DateTime::from_chrono(Utc::now());

    let mut update_fields = doc! {
        "status": status.as_str(),
        "updated_at": &now,
    };

    if status == NodeStatus::Online {
        update_fields.insert("connected_at", now);
        update_fields.insert("last_heartbeat_at", now);
    }

    db.collection::<Node>(NODES)
        .update_one(
            doc! { "_id": node_id, "is_active": true },
            doc! { "$set": update_fields },
        )
        .await?;

    Ok(())
}

/// Rotate the node's auth token and signing secret. Invalidates old values immediately.
/// Returns (raw_auth_token, raw_signing_secret).
pub async fn rotate_auth_token(
    db: &mongodb::Database,
    encryption_keys: &EncryptionKeys,
    user_id: &str,
    node_id: &str,
) -> AppResult<(String, String)> {
    let _node = ensure_node_writable_by_actor(db, user_id, node_id).await?;

    let raw_token = format!("nyx_nauth_{}", hex::encode(rand::random::<[u8; 32]>()));
    let token_hash = hash_token(&raw_token);
    let raw_signing_secret = hex::encode(rand::random::<[u8; 32]>());
    let signing_secret_encrypted = encryption_keys
        .encrypt(raw_signing_secret.as_bytes())
        .await?;
    let signing_secret_hash = hash_token(&raw_signing_secret);
    let now = bson::DateTime::from_chrono(Utc::now());

    let result = db
        .collection::<Node>(NODES)
        .update_one(
            doc! { "_id": node_id, "is_active": true },
            doc! { "$set": {
                "auth_token_hash": &token_hash,
                "signing_secret_encrypted": bson::Binary {
                    subtype: bson::spec::BinarySubtype::Generic,
                    bytes: signing_secret_encrypted,
                },
                "signing_secret_hash": &signing_secret_hash,
                "updated_at": &now,
            } },
        )
        .await?;

    if result.matched_count == 0 {
        return Err(AppError::NodeNotFound("Node not found".to_string()));
    }

    tracing::info!(node_id = %node_id, "Node auth token and signing secret rotated");
    Ok((raw_token, raw_signing_secret))
}

/// Update a binding's priority.
pub async fn update_binding_priority(
    db: &mongodb::Database,
    user_id: &str,
    node_id: &str,
    binding_id: &str,
    priority: i32,
) -> AppResult<()> {
    let node = ensure_node_writable_by_actor(db, user_id, node_id).await?;

    let now = bson::DateTime::from_chrono(Utc::now());
    let result = db
        .collection::<NodeServiceBinding>(NODE_SERVICE_BINDINGS)
        .update_one(
            doc! { "_id": binding_id, "user_id": &node.user_id, "node_id": node_id, "is_active": true },
            doc! { "$set": { "priority": priority, "updated_at": &now } },
        )
        .await?;

    if result.matched_count == 0 {
        return Err(AppError::NotFound("Binding not found".to_string()));
    }

    tracing::info!(binding_id = %binding_id, priority, "Binding priority updated");
    Ok(())
}

/// Admin: list active operational nodes (no user filter).
///
/// Device-code provisioning creates node-shaped ownership records, but those
/// devices authenticate with API keys and are not operational node WebSocket
/// agents, so the operational admin list excludes them.
pub async fn list_all_nodes(
    db: &mongodb::Database,
    page: u64,
    per_page: u64,
    status_filter: Option<&str>,
    user_id_filter: Option<&str>,
) -> AppResult<(Vec<Node>, u64)> {
    let mut filter = doc! {
        "is_active": true,
        "metadata.provisioning_source": { "$nin": DEVICE_PROVISIONING_SOURCES },
    };
    if let Some(status) = status_filter {
        filter.insert("status", status);
    }
    if let Some(uid) = user_id_filter {
        filter.insert("user_id", uid);
    }

    let total = db
        .collection::<Node>(NODES)
        .count_documents(filter.clone())
        .await?;

    let offset = (page - 1) * per_page;
    let nodes: Vec<Node> = db
        .collection::<Node>(NODES)
        .find(filter)
        .sort(doc! { "created_at": -1 })
        .skip(offset)
        .limit(per_page as i64)
        .await?
        .try_collect()
        .await?;

    Ok((nodes, total))
}

/// Admin: soft-delete a node without ownership check.
pub async fn admin_delete_node(db: &mongodb::Database, node_id: &str) -> AppResult<()> {
    let now = bson::DateTime::from_chrono(Utc::now());

    let result = db
        .collection::<Node>(NODES)
        .update_one(
            doc! { "_id": node_id, "is_active": true },
            doc! { "$set": { "is_active": false, "status": NodeStatus::Offline.as_str(), "updated_at": &now } },
        )
        .await?;

    if result.matched_count == 0 {
        return Err(AppError::NodeNotFound("Node not found".to_string()));
    }

    // Deactivate all bindings for this node
    db.collection::<NodeServiceBinding>(NODE_SERVICE_BINDINGS)
        .update_many(
            doc! { "node_id": node_id },
            doc! { "$set": { "is_active": false, "updated_at": &now } },
        )
        .await?;

    tracing::info!(node_id = %node_id, "Node admin-deleted");
    Ok(())
}

/// Validate a raw auth token. Returns the Node if valid.
pub async fn validate_auth_token(db: &mongodb::Database, raw_token: &str) -> AppResult<Node> {
    let token_hash = hash_token(raw_token);

    db.collection::<Node>(NODES)
        .find_one(doc! { "auth_token_hash": &token_hash, "is_active": true })
        .await?
        .ok_or_else(|| AppError::Unauthorized("Invalid node auth token".to_string()))
}

fn decode_node_signing_secret(
    secret_hex_bytes: &[u8],
    node_id: &str,
) -> AppResult<Zeroizing<Vec<u8>>> {
    let secret_hex = std::str::from_utf8(secret_hex_bytes).map_err(|e| {
        AppError::Internal(format!(
            "Node signing secret for {node_id} is not valid UTF-8: {e}"
        ))
    })?;

    let secret = hex::decode(secret_hex).map_err(|e| {
        AppError::Internal(format!(
            "Node signing secret for {node_id} is not valid hex: {e}"
        ))
    })?;

    Ok(Zeroizing::new(secret))
}

// --- Binding operations ---

/// Create a service binding for a node.
/// Auto-sync `NodeServiceBinding` when `UserService.node_id` changes.
///
/// Call this after creating or updating a `UserService` with a node routing change.
/// Creates a binding when `node_id` is set, deactivates the old one when it changes.
///
/// `user_id` is the *binding owner* -- for org-shared services this is the
/// org's user_id, so that proxy-time routing (which queries bindings by the
/// effective service owner) finds it. `actor_user_id` is the human (or API
/// key) that actually owns the node and is performing the operation; it is
/// used to validate that the actor is allowed to bind this node. Both are
/// the same value for personal services. For org services they differ:
/// the actor is an org admin and the node is their personal node.
pub async fn sync_node_binding_for_user_service(
    db: &mongodb::Database,
    user_id: &str,
    actor_user_id: &str,
    catalog_service_id: Option<&str>,
    new_node_id: Option<&str>,
    old_node_id: Option<&str>,
) -> AppResult<()> {
    let Some(service_id) = catalog_service_id else {
        return Ok(());
    };

    // Validate the new node before mutating bindings so an invalid update does not
    // tear down the previous route. The node is owned by the *actor*, not the
    // binding owner -- a personal node may be referenced by an org-owned service
    // when the actor is an admin of that org. Use the actor-based access check
    // so the actor's org admin role on the node owner is honored.
    if let Some(new_nid) = new_node_id.filter(|nid| !nid.is_empty()) {
        ensure_node_writable_by_actor(db, actor_user_id, new_nid).await?;
    }

    // Deactivate old binding if the node changed or was cleared.
    if let Some(old_nid) = old_node_id {
        let changed = match new_node_id {
            Some(new_nid) if !new_nid.is_empty() => new_nid != old_nid,
            _ => true, // cleared
        };
        if changed && !has_active_user_service_for_node(db, user_id, service_id, old_nid).await? {
            deactivate_binding_by_node_and_service(db, user_id, old_nid, service_id).await?;
        }
    }

    // Create binding if new node_id is set.
    if let Some(new_nid) = new_node_id.filter(|nid| !nid.is_empty()) {
        ensure_binding_exists(db, user_id, new_nid, service_id).await?;
    }

    Ok(())
}

async fn has_active_user_service_for_node(
    db: &mongodb::Database,
    user_id: &str,
    service_id: &str,
    node_id: &str,
) -> AppResult<bool> {
    let count = db
        .collection::<mongodb::bson::Document>(USER_SERVICES)
        .count_documents(doc! {
            "user_id": user_id,
            "catalog_service_id": service_id,
            "node_id": node_id,
            "is_active": true,
        })
        .await?;

    Ok(count > 0)
}

/// Create a `NodeServiceBinding` if one does not already exist for this node + service.
/// Uses insert-first with duplicate-key handling to avoid race conditions.
async fn ensure_binding_exists(
    db: &mongodb::Database,
    user_id: &str,
    node_id: &str,
    service_id: &str,
) -> AppResult<()> {
    let existing = db
        .collection::<NodeServiceBinding>(NODE_SERVICE_BINDINGS)
        .find_one(doc! {
            "node_id": node_id,
            "service_id": service_id,
            "user_id": user_id,
            "is_active": true,
        })
        .await?;

    if existing.is_some() {
        return Ok(());
    }

    let now = Utc::now();
    let binding = NodeServiceBinding {
        id: uuid::Uuid::new_v4().to_string(),
        node_id: node_id.to_string(),
        user_id: user_id.to_string(),
        service_id: service_id.to_string(),
        is_active: true,
        priority: 0,
        created_at: now,
        updated_at: now,
    };

    match db
        .collection::<NodeServiceBinding>(NODE_SERVICE_BINDINGS)
        .insert_one(&binding)
        .await
    {
        Ok(_) => {
            tracing::info!(
                binding_id = %binding.id,
                node_id = %node_id,
                service_id = %service_id,
                "Auto-created node service binding from UserService.node_id"
            );
        }
        Err(e) => {
            // Duplicate key error (E11000) means a concurrent request already created the
            // binding -- treat as success (idempotent).
            let is_dup = e.kind.as_ref().to_string().contains("E11000");
            if !is_dup {
                return Err(e.into());
            }
        }
    }

    Ok(())
}

/// Deactivate a binding by node + service (not by binding ID).
async fn deactivate_binding_by_node_and_service(
    db: &mongodb::Database,
    user_id: &str,
    node_id: &str,
    service_id: &str,
) -> AppResult<()> {
    let now = bson::DateTime::from_chrono(Utc::now());

    let result = db
        .collection::<NodeServiceBinding>(NODE_SERVICE_BINDINGS)
        .update_many(
            doc! {
                "node_id": node_id,
                "service_id": service_id,
                "user_id": user_id,
                "is_active": true,
            },
            doc! { "$set": { "is_active": false, "updated_at": &now } },
        )
        .await?;

    if result.modified_count > 0 {
        tracing::info!(
            node_id = %node_id,
            service_id = %service_id,
            count = result.modified_count,
            "Auto-deactivated node service binding(s) from UserService.node_id change"
        );
    }

    Ok(())
}

pub async fn create_binding(
    db: &mongodb::Database,
    user_id: &str,
    node_id: &str,
    service_id: &str,
) -> AppResult<NodeServiceBinding> {
    let node = ensure_node_writable_by_actor(db, user_id, node_id).await?;

    // Check for existing binding (same node + service)
    let existing = db
        .collection::<NodeServiceBinding>(NODE_SERVICE_BINDINGS)
        .find_one(doc! {
            "node_id": node_id,
            "service_id": service_id,
            "user_id": &node.user_id,
            "is_active": true,
        })
        .await?;

    if existing.is_some() {
        return Err(AppError::Conflict(
            "Binding already exists for this node and service".to_string(),
        ));
    }

    let now = Utc::now();
    let binding = NodeServiceBinding {
        id: uuid::Uuid::new_v4().to_string(),
        node_id: node_id.to_string(),
        user_id: node.user_id.clone(),
        service_id: service_id.to_string(),
        is_active: true,
        priority: 0,
        created_at: now,
        updated_at: now,
    };

    db.collection::<NodeServiceBinding>(NODE_SERVICE_BINDINGS)
        .insert_one(&binding)
        .await?;

    tracing::info!(
        binding_id = %binding.id,
        node_id = %node_id,
        service_id = %service_id,
        "Node service binding created"
    );

    Ok(binding)
}

/// List all active bindings for a node.
pub async fn list_bindings(
    db: &mongodb::Database,
    user_id: &str,
    node_id: &str,
) -> AppResult<Vec<NodeServiceBinding>> {
    let node = get_node(db, user_id, node_id).await?;

    let bindings: Vec<NodeServiceBinding> = db
        .collection::<NodeServiceBinding>(NODE_SERVICE_BINDINGS)
        .find(doc! { "node_id": node_id, "user_id": &node.user_id, "is_active": true })
        .sort(doc! { "created_at": -1 })
        .await?
        .try_collect()
        .await?;

    Ok(bindings)
}

/// Delete a binding. Verifies the binding belongs to the specified node and user.
pub async fn delete_binding(
    db: &mongodb::Database,
    user_id: &str,
    node_id: &str,
    binding_id: &str,
) -> AppResult<()> {
    let node = ensure_node_writable_by_actor(db, user_id, node_id).await?;
    let now = bson::DateTime::from_chrono(Utc::now());

    let result = db
        .collection::<NodeServiceBinding>(NODE_SERVICE_BINDINGS)
        .update_one(
            doc! { "_id": binding_id, "user_id": &node.user_id, "node_id": node_id, "is_active": true },
            doc! { "$set": { "is_active": false, "updated_at": &now } },
        )
        .await?;

    if result.matched_count == 0 {
        return Err(AppError::NotFound("Binding not found".to_string()));
    }

    tracing::info!(binding_id = %binding_id, "Node service binding deleted");
    Ok(())
}

async fn load_active_node(db: &mongodb::Database, node_id: &str) -> AppResult<Node> {
    db.collection::<Node>(NODES)
        .find_one(doc! { "_id": node_id, "is_active": true })
        .await?
        .ok_or_else(|| AppError::NodeNotFound("Node not found".to_string()))
}

async fn ensure_node_readable_by_access(
    db: &mongodb::Database,
    actor_user_id: &str,
    node: &Node,
) -> AppResult<OwnerAccess> {
    let access = org_service::resolve_owner_access(db, actor_user_id, &node.user_id).await?;
    if node_access_can_read(&access) {
        Ok(access)
    } else {
        Err(AppError::NodeNotFound("Node not found".to_string()))
    }
}

async fn ensure_node_writable_by_access(
    db: &mongodb::Database,
    actor_user_id: &str,
    node: &Node,
) -> AppResult<OwnerAccess> {
    let access = org_service::resolve_owner_access(db, actor_user_id, &node.user_id).await?;
    if access.can_write() {
        Ok(access)
    } else {
        Err(AppError::NodeNotFound("Node not found".to_string()))
    }
}

pub(crate) fn node_access_can_read(access: &OwnerAccess) -> bool {
    match access {
        OwnerAccess::Direct | OwnerAccess::AsOrgAdmin { .. } => true,
        OwnerAccess::AsOrgMember { role, .. } => matches!(role, OrgRole::Member),
        OwnerAccess::Forbidden => false,
    }
}

async fn attach_owner_info(
    db: &mongodb::Database,
    nodes: Vec<Node>,
) -> AppResult<Vec<NodeWithOwner>> {
    if nodes.is_empty() {
        return Ok(vec![]);
    }

    let mut owner_ids = Vec::<String>::new();
    for node in &nodes {
        if !owner_ids.iter().any(|id| id == &node.user_id) {
            owner_ids.push(node.user_id.clone());
        }
    }

    let owner_id_array: bson::Array = owner_ids
        .iter()
        .map(|id| bson::Bson::String(id.clone()))
        .collect();
    let owners: Vec<User> = db
        .collection::<User>(USERS)
        .find(doc! { "_id": { "$in": owner_id_array } })
        .await?
        .try_collect()
        .await?;
    let owner_map: HashMap<String, User> = owners.into_iter().map(|u| (u.id.clone(), u)).collect();

    Ok(nodes
        .into_iter()
        .map(|node| {
            let owner = owner_info_from_user_id(&node.user_id, owner_map.get(&node.user_id));
            NodeWithOwner { node, owner }
        })
        .collect())
}

pub async fn owner_info_for_node(db: &mongodb::Database, node: &Node) -> AppResult<NodeOwnerInfo> {
    let owner = db
        .collection::<User>(USERS)
        .find_one(doc! { "_id": &node.user_id })
        .await?;
    Ok(owner_info_from_user_id(&node.user_id, owner.as_ref()))
}

pub async fn list_node_admins(
    db: &mongodb::Database,
    actor_user_id: &str,
    node_id: &str,
) -> AppResult<Vec<NodeAdminInfo>> {
    let node = get_node(db, actor_user_id, node_id).await?;
    let owner = db
        .collection::<User>(USERS)
        .find_one(doc! { "_id": &node.user_id })
        .await?;

    if !matches!(owner.as_ref().map(|u| u.user_type), Some(UserType::Org)) {
        return Ok(vec![admin_info_from_user(
            &node.user_id,
            owner.as_ref(),
            NodeAdminRole::Owner,
        )]);
    }

    let memberships = org_service::list_members_for_org(db, &node.user_id, false).await?;
    let admin_user_ids: Vec<String> = memberships
        .into_iter()
        .filter(|membership| matches!(membership.role, OrgRole::Admin))
        .map(|membership| membership.member_user_id)
        .collect();
    if admin_user_ids.is_empty() {
        return Ok(vec![]);
    }

    let admin_id_array: bson::Array = admin_user_ids
        .iter()
        .map(|id| bson::Bson::String(id.clone()))
        .collect();
    let users: Vec<User> = db
        .collection::<User>(USERS)
        .find(doc! { "_id": { "$in": admin_id_array } })
        .await?
        .try_collect()
        .await?;
    let user_map: HashMap<String, User> = users.into_iter().map(|u| (u.id.clone(), u)).collect();

    Ok(admin_user_ids
        .into_iter()
        .map(|user_id| admin_info_from_user(&user_id, user_map.get(&user_id), NodeAdminRole::Admin))
        .collect())
}

fn owner_info_from_user_id(owner_id: &str, owner: Option<&User>) -> NodeOwnerInfo {
    let kind = match owner.map(|u| u.user_type) {
        Some(UserType::Org) => NodeOwnerKind::Org,
        _ => NodeOwnerKind::User,
    };
    let display_name = owner
        .and_then(|u| u.display_name.clone())
        .filter(|name| !name.trim().is_empty())
        .or_else(|| {
            owner
                .map(|u| u.email.clone())
                .filter(|email| !email.trim().is_empty())
        })
        .unwrap_or_else(|| owner_id.to_string());

    NodeOwnerInfo {
        kind,
        id: owner_id.to_string(),
        display_name,
    }
}

fn admin_info_from_user(user_id: &str, user: Option<&User>, role: NodeAdminRole) -> NodeAdminInfo {
    NodeAdminInfo {
        user_id: user_id.to_string(),
        display_name: user.and_then(|u| u.display_name.clone()),
        email: user.map(|u| u.email.clone()),
        role,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::node::NodeMetrics;
    use crate::models::node_service_binding::COLLECTION_NAME as NODE_SERVICE_BINDINGS;
    use crate::models::org_membership::{
        COLLECTION_NAME as ORG_MEMBERSHIPS, OrgMembership, OrgRole,
    };
    use crate::models::user::{COLLECTION_NAME as USERS, UserType};
    use crate::test_utils::{
        connect_test_database, test_encryption_keys, test_membership, test_user,
    };
    use uuid::Uuid;

    #[derive(Clone, Copy, Debug)]
    enum AccessCase {
        Direct,
        AsOrgAdmin,
        AsOrgMember,
        NoAccess,
    }

    impl AccessCase {
        fn all() -> [Self; 4] {
            [
                Self::Direct,
                Self::AsOrgAdmin,
                Self::AsOrgMember,
                Self::NoAccess,
            ]
        }

        fn label(self) -> &'static str {
            match self {
                Self::Direct => "direct",
                Self::AsOrgAdmin => "org_admin",
                Self::AsOrgMember => "org_member",
                Self::NoAccess => "no_access",
            }
        }

        fn can_read(self) -> bool {
            matches!(self, Self::Direct | Self::AsOrgAdmin | Self::AsOrgMember)
        }

        fn can_write(self) -> bool {
            matches!(self, Self::Direct | Self::AsOrgAdmin)
        }
    }

    #[test]
    fn device_node_name_sanitizes_and_keeps_uuid_suffix() {
        let node_id = "12345678-aaaa-bbbb-cccc-123456789012";

        assert_eq!(
            device_node_name("Kitchen Camera 01", node_id),
            "kitchen-camera-01-12345678"
        );
        assert_eq!(device_node_name("!!!", node_id), "device-12345678");
        assert!(device_node_name(&"A".repeat(200), node_id).len() <= 64);
    }

    #[tokio::test]
    async fn create_for_device_creates_device_node_with_valid_auth_material() {
        let Some(db) = connect_test_database("node_device_code_stub").await else {
            return;
        };
        let owner_id = Uuid::new_v4().to_string();

        let node = create_for_device(
            &db,
            &test_encryption_keys(),
            DeviceNodeInput {
                user_id: &owner_id,
                api_key_id: "api-key-1",
                hw_id: "esp32-p4-cam",
                label: "Kitchen Camera",
                device_pubkey: Some(&[7u8; 32]),
                provisioning_source: DEVICE_CODE_PROVISIONING_SOURCE,
            },
        )
        .await
        .expect("create device stub");

        assert_eq!(node.auth_token_hash.len(), 64);
        assert_ne!(node.auth_token_hash, hash_token(""));
        assert_eq!(node.signing_secret_hash.len(), 64);
        assert_ne!(node.signing_secret_hash, hash_token(""));
        assert!(node.signing_secret_encrypted.is_some());
        assert_eq!(
            node.metadata
                .as_ref()
                .and_then(|metadata| metadata.provisioning_source.as_deref()),
            Some(DEVICE_CODE_PROVISIONING_SOURCE)
        );
    }

    #[tokio::test]
    async fn list_all_nodes_excludes_device_code_stubs_from_operational_view() {
        let Some(db) = connect_test_database("node_device_code_stub_admin_list").await else {
            return;
        };
        let owner_id = Uuid::new_v4().to_string();
        let operational_node = make_node(&owner_id, "operational-node");
        db.collection::<Node>(NODES)
            .insert_one(&operational_node)
            .await
            .expect("insert operational node");
        create_for_device(
            &db,
            &test_encryption_keys(),
            DeviceNodeInput {
                user_id: &owner_id,
                api_key_id: "api-key-1",
                hw_id: "esp32-p4-cam",
                label: "Kitchen Camera",
                device_pubkey: Some(&[7u8; 32]),
                provisioning_source: DEVICE_CODE_PROVISIONING_SOURCE,
            },
        )
        .await
        .expect("create device stub");

        let (nodes, total) = list_all_nodes(&db, 1, 50, None, None)
            .await
            .expect("list nodes");

        assert_eq!(total, 1);
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].id, operational_node.id);
    }

    struct NodeAclFixture {
        actor_id: String,
        owner_id: String,
        node: Node,
        binding: NodeServiceBinding,
    }

    fn make_node(owner_id: &str, name: &str) -> Node {
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

    fn make_binding(owner_id: &str, node_id: &str, service_id: &str) -> NodeServiceBinding {
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

    async fn seed_acl_fixture(db: &mongodb::Database, case: AccessCase) -> NodeAclFixture {
        let actor_id = Uuid::new_v4().to_string();
        let owner_id = match case {
            AccessCase::Direct => actor_id.clone(),
            AccessCase::AsOrgAdmin | AccessCase::AsOrgMember | AccessCase::NoAccess => {
                Uuid::new_v4().to_string()
            }
        };
        let service_id = Uuid::new_v4().to_string();
        let node = make_node(&owner_id, &format!("node-{}", case.label()));
        let binding = make_binding(&owner_id, &node.id, &service_id);

        let mut users = vec![test_user(&actor_id, UserType::Person)];
        if owner_id != actor_id {
            users.push(test_user(&owner_id, UserType::Org));
        }
        db.collection::<User>(USERS)
            .insert_many(users)
            .await
            .expect("insert users");

        match case {
            AccessCase::AsOrgAdmin => {
                db.collection::<OrgMembership>(ORG_MEMBERSHIPS)
                    .insert_one(test_membership(&owner_id, &actor_id, OrgRole::Admin, None))
                    .await
                    .expect("insert admin membership");
            }
            AccessCase::AsOrgMember => {
                db.collection::<OrgMembership>(ORG_MEMBERSHIPS)
                    .insert_one(test_membership(&owner_id, &actor_id, OrgRole::Member, None))
                    .await
                    .expect("insert member membership");
            }
            AccessCase::Direct | AccessCase::NoAccess => {}
        }

        db.collection::<Node>(NODES)
            .insert_one(node.clone())
            .await
            .expect("insert node");
        db.collection::<NodeServiceBinding>(NODE_SERVICE_BINDINGS)
            .insert_one(binding.clone())
            .await
            .expect("insert binding");

        NodeAclFixture {
            actor_id,
            owner_id,
            node,
            binding,
        }
    }

    fn assert_node_not_found<T: std::fmt::Debug>(result: AppResult<T>, case: AccessCase) {
        let err = result.expect_err("operation should fail");
        assert!(
            matches!(err, AppError::NodeNotFound(_)),
            "case {case:?} returned unexpected error: {err}"
        );
    }

    #[test]
    fn decodes_node_signing_secret_hex() {
        let secret = decode_node_signing_secret(b"616263", "node-1").expect("valid secret");
        assert_eq!(secret.as_slice(), b"abc");
    }

    #[test]
    fn rejects_node_signing_secret_invalid_utf8() {
        let error = decode_node_signing_secret(&[0xff], "node-1").expect_err("invalid utf8");
        assert!(error.to_string().contains("not valid UTF-8"));
    }

    #[test]
    fn rejects_node_signing_secret_invalid_hex() {
        let error = decode_node_signing_secret(b"not-hex", "node-1").expect_err("invalid hex");
        assert!(error.to_string().contains("not valid hex"));
    }

    #[tokio::test]
    async fn get_node_uses_org_owner_acl_matrix() {
        let Some(db) = connect_test_database("node_get_acl").await else {
            eprintln!("skipping node service ACL test: no local MongoDB available");
            return;
        };

        for case in AccessCase::all() {
            let fixture = seed_acl_fixture(&db, case).await;
            let result = get_node(&db, &fixture.actor_id, &fixture.node.id).await;
            if case.can_read() {
                let node = result.expect("readable case should load node");
                assert_eq!(node.id, fixture.node.id, "case {case:?}");
            } else {
                assert_node_not_found(result, case);
            }
        }
    }

    #[tokio::test]
    async fn list_bindings_uses_org_owner_acl_matrix() {
        let Some(db) = connect_test_database("node_list_bindings_acl").await else {
            eprintln!("skipping node service ACL test: no local MongoDB available");
            return;
        };

        for case in AccessCase::all() {
            let fixture = seed_acl_fixture(&db, case).await;
            let result = list_bindings(&db, &fixture.actor_id, &fixture.node.id).await;
            if case.can_read() {
                let bindings = result.expect("readable case should list bindings");
                assert_eq!(bindings.len(), 1, "case {case:?}");
                assert_eq!(bindings[0].id, fixture.binding.id, "case {case:?}");
            } else {
                assert_node_not_found(result, case);
            }
        }
    }

    #[tokio::test]
    async fn delete_node_uses_org_owner_acl_matrix() {
        let Some(db) = connect_test_database("node_delete_acl").await else {
            eprintln!("skipping node service ACL test: no local MongoDB available");
            return;
        };

        for case in AccessCase::all() {
            let fixture = seed_acl_fixture(&db, case).await;
            let result = delete_node(&db, &fixture.actor_id, &fixture.node.id).await;
            if case.can_write() {
                result.expect("writable case should delete node");
                let stored = get_node_by_id(&db, &fixture.node.id)
                    .await
                    .expect("query node");
                assert!(stored.is_none(), "case {case:?}");
            } else {
                assert_node_not_found(result, case);
            }
        }
    }

    #[tokio::test]
    async fn rotate_auth_token_uses_org_owner_acl_matrix() {
        let Some(db) = connect_test_database("node_rotate_acl").await else {
            eprintln!("skipping node service ACL test: no local MongoDB available");
            return;
        };
        let encryption_keys = test_encryption_keys();

        for case in AccessCase::all() {
            let fixture = seed_acl_fixture(&db, case).await;
            let result =
                rotate_auth_token(&db, &encryption_keys, &fixture.actor_id, &fixture.node.id).await;
            if case.can_write() {
                let (auth_token, signing_secret) =
                    result.expect("writable case should rotate credentials");
                assert!(auth_token.starts_with("nyx_nauth_"), "case {case:?}");
                assert_eq!(signing_secret.len(), 64, "case {case:?}");
            } else {
                assert_node_not_found(result, case);
            }
        }
    }

    #[tokio::test]
    async fn update_binding_priority_uses_org_owner_acl_matrix() {
        let Some(db) = connect_test_database("node_update_binding_acl").await else {
            eprintln!("skipping node service ACL test: no local MongoDB available");
            return;
        };

        for case in AccessCase::all() {
            let fixture = seed_acl_fixture(&db, case).await;
            let result = update_binding_priority(
                &db,
                &fixture.actor_id,
                &fixture.node.id,
                &fixture.binding.id,
                5,
            )
            .await;
            if case.can_write() {
                result.expect("writable case should update binding");
                let stored = db
                    .collection::<NodeServiceBinding>(NODE_SERVICE_BINDINGS)
                    .find_one(doc! { "_id": &fixture.binding.id })
                    .await
                    .expect("query binding")
                    .expect("binding exists");
                assert_eq!(stored.priority, 5, "case {case:?}");
            } else {
                assert_node_not_found(result, case);
            }
        }
    }

    #[tokio::test]
    async fn create_binding_uses_org_owner_acl_matrix() {
        let Some(db) = connect_test_database("node_create_binding_acl").await else {
            eprintln!("skipping node service ACL test: no local MongoDB available");
            return;
        };

        for case in AccessCase::all() {
            let fixture = seed_acl_fixture(&db, case).await;
            let service_id = Uuid::new_v4().to_string();
            let result =
                create_binding(&db, &fixture.actor_id, &fixture.node.id, &service_id).await;
            if case.can_write() {
                let binding = result.expect("writable case should create binding");
                assert_eq!(binding.user_id, fixture.owner_id, "case {case:?}");
                assert_eq!(binding.node_id, fixture.node.id, "case {case:?}");
                assert_eq!(binding.service_id, service_id, "case {case:?}");
            } else {
                assert_node_not_found(result, case);
            }
        }
    }

    #[tokio::test]
    async fn delete_binding_uses_org_owner_acl_matrix() {
        let Some(db) = connect_test_database("node_delete_binding_acl").await else {
            eprintln!("skipping node service ACL test: no local MongoDB available");
            return;
        };

        for case in AccessCase::all() {
            let fixture = seed_acl_fixture(&db, case).await;
            let result = delete_binding(
                &db,
                &fixture.actor_id,
                &fixture.node.id,
                &fixture.binding.id,
            )
            .await;
            if case.can_write() {
                result.expect("writable case should delete binding");
                let stored = db
                    .collection::<NodeServiceBinding>(NODE_SERVICE_BINDINGS)
                    .find_one(doc! { "_id": &fixture.binding.id })
                    .await
                    .expect("query binding")
                    .expect("binding exists");
                assert!(!stored.is_active, "case {case:?}");
            } else {
                assert_node_not_found(result, case);
            }
        }
    }

    #[tokio::test]
    async fn list_user_nodes_includes_personal_and_member_org_nodes_with_owner_metadata() {
        let Some(db) = connect_test_database("node_list_owner_info").await else {
            eprintln!("skipping node service ACL test: no local MongoDB available");
            return;
        };

        let actor_id = Uuid::new_v4().to_string();
        let org_id = Uuid::new_v4().to_string();
        let viewer_org_id = Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_many([
                test_user(&actor_id, UserType::Person),
                test_user(&org_id, UserType::Org),
                test_user(&viewer_org_id, UserType::Org),
            ])
            .await
            .expect("insert users");
        db.collection::<OrgMembership>(ORG_MEMBERSHIPS)
            .insert_many([
                test_membership(&org_id, &actor_id, OrgRole::Member, None),
                test_membership(&viewer_org_id, &actor_id, OrgRole::Viewer, None),
            ])
            .await
            .expect("insert memberships");

        let personal_node = make_node(&actor_id, "personal-node");
        let org_node = make_node(&org_id, "org-node");
        let viewer_org_node = make_node(&viewer_org_id, "viewer-org-node");
        db.collection::<Node>(NODES)
            .insert_many([
                personal_node.clone(),
                org_node.clone(),
                viewer_org_node.clone(),
            ])
            .await
            .expect("insert nodes");

        let listed = list_user_nodes(&db, &actor_id)
            .await
            .expect("list reachable nodes");
        let ids: Vec<&str> = listed.iter().map(|entry| entry.node.id.as_str()).collect();
        assert!(ids.contains(&personal_node.id.as_str()));
        assert!(ids.contains(&org_node.id.as_str()));
        assert!(!ids.contains(&viewer_org_node.id.as_str()));

        let personal = listed
            .iter()
            .find(|entry| entry.node.id == personal_node.id)
            .expect("personal node listed");
        assert_eq!(personal.owner.kind, NodeOwnerKind::User);
        assert_eq!(personal.owner.id, actor_id);
        assert_eq!(personal.owner.display_name, "Test User");

        let org = listed
            .iter()
            .find(|entry| entry.node.id == org_node.id)
            .expect("org node listed");
        assert_eq!(org.owner.kind, NodeOwnerKind::Org);
        assert_eq!(org.owner.id, org_id);
        assert_eq!(org.owner.display_name, "Test Org");
    }

    #[tokio::test]
    async fn personal_user_node_access_remains_unchanged() {
        let Some(db) = connect_test_database("node_personal_regression").await else {
            eprintln!("skipping node service ACL test: no local MongoDB available");
            return;
        };

        let actor_id = Uuid::new_v4().to_string();
        let other_id = Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_many([
                test_user(&actor_id, UserType::Person),
                test_user(&other_id, UserType::Person),
            ])
            .await
            .expect("insert users");
        let owned = make_node(&actor_id, "owned-node");
        let other = make_node(&other_id, "other-node");
        db.collection::<Node>(NODES)
            .insert_many([owned.clone(), other.clone()])
            .await
            .expect("insert nodes");

        let loaded = get_node(&db, &actor_id, &owned.id)
            .await
            .expect("owner can get personal node");
        assert_eq!(loaded.id, owned.id);
        assert_node_not_found(
            get_node(&db, &actor_id, &other.id).await,
            AccessCase::NoAccess,
        );

        let listed = list_user_nodes(&db, &actor_id)
            .await
            .expect("list personal nodes");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].node.id, owned.id);
        assert_eq!(listed[0].owner.kind, NodeOwnerKind::User);
        assert_eq!(listed[0].owner.id, actor_id);
    }

    // ────────────────────────────────────────────────────────────────────
    // Pure-function tests (no MongoDB required)
    // ────────────────────────────────────────────────────────────────────

    // --- validate_node_metadata ---

    #[test]
    fn validate_node_metadata_accepts_all_none_fields() {
        let meta = NodeMetadata {
            agent_version: None,
            os: None,
            arch: None,
            ip_address: None,
            provisioning_source: None,
        };
        validate_node_metadata(&meta).expect("all-None metadata should be valid");
    }

    #[test]
    fn validate_node_metadata_accepts_valid_fields() {
        let meta = NodeMetadata {
            agent_version: Some("1.2.3".to_string()),
            os: Some("linux".to_string()),
            arch: Some("x86_64".to_string()),
            ip_address: Some("192.168.1.1".to_string()),
            provisioning_source: None,
        };
        validate_node_metadata(&meta).expect("valid metadata should pass");
    }

    #[test]
    fn validate_node_metadata_accepts_ipv6_address() {
        let meta = NodeMetadata {
            agent_version: None,
            os: None,
            arch: None,
            ip_address: Some("::1".to_string()),
            provisioning_source: None,
        };
        validate_node_metadata(&meta).expect("IPv6 loopback should be valid");
    }

    #[test]
    fn validate_node_metadata_accepts_full_ipv6() {
        let meta = NodeMetadata {
            agent_version: None,
            os: None,
            arch: None,
            ip_address: Some("2001:0db8:85a3:0000:0000:8a2e:0370:7334".to_string()),
            provisioning_source: None,
        };
        validate_node_metadata(&meta).expect("full IPv6 should be valid");
    }

    #[test]
    fn validate_node_metadata_rejects_overlong_agent_version() {
        let meta = NodeMetadata {
            agent_version: Some("v".repeat(65)),
            os: None,
            arch: None,
            ip_address: None,
            provisioning_source: None,
        };
        let err = validate_node_metadata(&meta).expect_err("agent_version too long");
        assert!(matches!(err, AppError::ValidationError(msg) if msg.contains("agent_version")));
    }

    #[test]
    fn validate_node_metadata_accepts_exactly_64_char_agent_version() {
        let meta = NodeMetadata {
            agent_version: Some("v".repeat(64)),
            os: None,
            arch: None,
            ip_address: None,
            provisioning_source: None,
        };
        validate_node_metadata(&meta).expect("64-char agent_version should pass");
    }

    #[test]
    fn validate_node_metadata_rejects_overlong_os() {
        let meta = NodeMetadata {
            agent_version: None,
            os: Some("x".repeat(65)),
            arch: None,
            ip_address: None,
            provisioning_source: None,
        };
        let err = validate_node_metadata(&meta).expect_err("os too long");
        assert!(matches!(err, AppError::ValidationError(msg) if msg.contains("os")));
    }

    #[test]
    fn validate_node_metadata_rejects_overlong_arch() {
        let meta = NodeMetadata {
            agent_version: None,
            os: None,
            arch: Some("a".repeat(65)),
            ip_address: None,
            provisioning_source: None,
        };
        let err = validate_node_metadata(&meta).expect_err("arch too long");
        assert!(matches!(err, AppError::ValidationError(msg) if msg.contains("arch")));
    }

    #[test]
    fn validate_node_metadata_rejects_invalid_ip_address() {
        let meta = NodeMetadata {
            agent_version: None,
            os: None,
            arch: None,
            ip_address: Some("not-an-ip".to_string()),
            provisioning_source: None,
        };
        let err = validate_node_metadata(&meta).expect_err("invalid IP");
        assert!(matches!(err, AppError::ValidationError(msg) if msg.contains("Invalid IP")));
    }

    #[test]
    fn validate_node_metadata_rejects_empty_ip_address() {
        let meta = NodeMetadata {
            agent_version: None,
            os: None,
            arch: None,
            ip_address: Some(String::new()),
            provisioning_source: None,
        };
        let err = validate_node_metadata(&meta).expect_err("empty IP");
        assert!(matches!(err, AppError::ValidationError(msg) if msg.contains("Invalid IP")));
    }

    // --- node_access_can_read ---

    #[test]
    fn node_access_can_read_allows_direct() {
        assert!(node_access_can_read(&OwnerAccess::Direct));
    }

    #[test]
    fn node_access_can_read_allows_org_admin() {
        let access = OwnerAccess::AsOrgAdmin {
            org_user_id: "org-1".to_string(),
            membership_id: "m-1".to_string(),
            allowed_service_ids: None,
        };
        assert!(node_access_can_read(&access));
    }

    #[test]
    fn node_access_can_read_allows_org_member() {
        let access = OwnerAccess::AsOrgMember {
            org_user_id: "org-1".to_string(),
            membership_id: "m-1".to_string(),
            role: OrgRole::Member,
            allowed_service_ids: None,
        };
        assert!(node_access_can_read(&access));
    }

    #[test]
    fn node_access_can_read_denies_org_viewer() {
        let access = OwnerAccess::AsOrgMember {
            org_user_id: "org-1".to_string(),
            membership_id: "m-1".to_string(),
            role: OrgRole::Viewer,
            allowed_service_ids: None,
        };
        assert!(!node_access_can_read(&access));
    }

    #[test]
    fn node_access_can_read_denies_forbidden() {
        assert!(!node_access_can_read(&OwnerAccess::Forbidden));
    }

    // --- owner_info_from_user_id ---

    #[test]
    fn owner_info_from_user_id_with_person_user() {
        let mut user = test_user("user-1", UserType::Person);
        user.display_name = Some("Alice".to_string());
        let info = owner_info_from_user_id("user-1", Some(&user));
        assert_eq!(info.kind, NodeOwnerKind::User);
        assert_eq!(info.id, "user-1");
        assert_eq!(info.display_name, "Alice");
    }

    #[test]
    fn owner_info_from_user_id_with_org_user() {
        let mut user = test_user("org-1", UserType::Org);
        user.display_name = Some("Acme Corp".to_string());
        let info = owner_info_from_user_id("org-1", Some(&user));
        assert_eq!(info.kind, NodeOwnerKind::Org);
        assert_eq!(info.display_name, "Acme Corp");
    }

    #[test]
    fn owner_info_from_user_id_falls_back_to_email_when_display_name_blank() {
        let mut user = test_user("user-1", UserType::Person);
        user.email = "bob@example.com".to_string();
        user.display_name = Some("   ".to_string());
        let info = owner_info_from_user_id("user-1", Some(&user));
        assert_eq!(info.display_name, "bob@example.com");
    }

    #[test]
    fn owner_info_from_user_id_falls_back_to_email_when_display_name_none() {
        let mut user = test_user("user-1", UserType::Person);
        user.email = "bob@example.com".to_string();
        user.display_name = None;
        let info = owner_info_from_user_id("user-1", Some(&user));
        assert_eq!(info.display_name, "bob@example.com");
    }

    #[test]
    fn owner_info_from_user_id_falls_back_to_owner_id_when_no_user() {
        let info = owner_info_from_user_id("unknown-id", None);
        assert_eq!(info.kind, NodeOwnerKind::User);
        assert_eq!(info.id, "unknown-id");
        assert_eq!(info.display_name, "unknown-id");
    }

    #[test]
    fn owner_info_from_user_id_falls_back_to_id_when_email_blank() {
        let mut user = test_user("user-1", UserType::Person);
        user.email = "   ".to_string();
        user.display_name = None;
        let info = owner_info_from_user_id("user-1", Some(&user));
        assert_eq!(info.display_name, "user-1");
    }

    // --- admin_info_from_user ---

    #[test]
    fn admin_info_from_user_with_user_present() {
        let mut user = test_user("user-1", UserType::Person);
        user.email = "admin@example.com".to_string();
        user.display_name = Some("Admin User".to_string());
        let info = admin_info_from_user("user-1", Some(&user), NodeAdminRole::Owner);
        assert_eq!(info.user_id, "user-1");
        assert_eq!(info.display_name, Some("Admin User".to_string()));
        assert_eq!(info.email, Some("admin@example.com".to_string()));
        assert_eq!(info.role, NodeAdminRole::Owner);
    }

    #[test]
    fn admin_info_from_user_without_user() {
        let info = admin_info_from_user("user-1", None, NodeAdminRole::Admin);
        assert_eq!(info.user_id, "user-1");
        assert_eq!(info.display_name, None);
        assert_eq!(info.email, None);
        assert_eq!(info.role, NodeAdminRole::Admin);
    }

    // ────────────────────────────────────────────────────────────────────
    // Integration tests (require MongoDB)
    // ────────────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn register_node_uses_registration_token_user_id_as_owner() {
        let Some(db) = connect_test_database("node_register_token_owner").await else {
            eprintln!("skipping node service registration test: no local MongoDB available");
            return;
        };

        let org_id = Uuid::new_v4().to_string();
        db.collection::<User>(USERS)
            .insert_one(test_user(&org_id, UserType::Org))
            .await
            .expect("insert org user");

        let (_token_id, raw_token, _expires_at) =
            create_registration_token(&db, &org_id, "org-node", 10, 3600)
                .await
                .expect("create org-owned token");
        let (node, _raw_auth_token, _raw_signing_secret) =
            register_node(&db, &test_encryption_keys(), &raw_token, None)
                .await
                .expect("register node from org token");

        assert_eq!(node.user_id, org_id);
        assert_eq!(node.name, "org-node");
    }
}

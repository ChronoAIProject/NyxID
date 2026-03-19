use chrono::{DateTime, Duration, Utc};
use futures::TryStreamExt;
use mongodb::bson::doc;
use zeroize::Zeroizing;

use crate::crypto::aes::EncryptionKeys;
use crate::crypto::token::hash_token;
use crate::errors::{AppError, AppResult};
use crate::models::node::{COLLECTION_NAME as NODES, Node, NodeMetadata, NodeStatus};
use crate::models::node_registration_token::{
    COLLECTION_NAME as NODE_REG_TOKENS, NodeRegistrationToken,
};
use crate::models::node_service_binding::{
    COLLECTION_NAME as NODE_SERVICE_BINDINGS, NodeServiceBinding,
};

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

/// Get a single node by ID without ownership check.
/// Used internally (e.g., heartbeat sweep).
pub async fn get_node_by_id(db: &mongodb::Database, node_id: &str) -> AppResult<Option<Node>> {
    let node = db
        .collection::<Node>(NODES)
        .find_one(doc! { "_id": node_id, "is_active": true })
        .await?;
    Ok(node)
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
    db.collection::<Node>(NODES)
        .find_one(doc! { "_id": node_id, "user_id": user_id, "is_active": true })
        .await?
        .ok_or_else(|| AppError::NodeNotFound("Node not found".to_string()))
}

/// List all active nodes for a user.
pub async fn list_user_nodes(db: &mongodb::Database, user_id: &str) -> AppResult<Vec<Node>> {
    let nodes: Vec<Node> = db
        .collection::<Node>(NODES)
        .find(doc! { "user_id": user_id, "is_active": true })
        .sort(doc! { "created_at": -1 })
        .await?
        .try_collect()
        .await?;
    Ok(nodes)
}

/// Soft-delete a node and its bindings.
pub async fn delete_node(db: &mongodb::Database, user_id: &str, node_id: &str) -> AppResult<()> {
    let now = bson::DateTime::from_chrono(Utc::now());

    let result = db
        .collection::<Node>(NODES)
        .update_one(
            doc! { "_id": node_id, "user_id": user_id, "is_active": true },
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
            doc! { "_id": node_id, "user_id": user_id, "is_active": true },
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
    // Verify node ownership
    let _node = get_node(db, user_id, node_id).await?;

    let now = bson::DateTime::from_chrono(Utc::now());
    let result = db
        .collection::<NodeServiceBinding>(NODE_SERVICE_BINDINGS)
        .update_one(
            doc! { "_id": binding_id, "user_id": user_id, "node_id": node_id, "is_active": true },
            doc! { "$set": { "priority": priority, "updated_at": &now } },
        )
        .await?;

    if result.matched_count == 0 {
        return Err(AppError::NotFound("Binding not found".to_string()));
    }

    tracing::info!(binding_id = %binding_id, priority, "Binding priority updated");
    Ok(())
}

/// Admin: list all active nodes (no user filter). Supports pagination and optional filters.
pub async fn list_all_nodes(
    db: &mongodb::Database,
    page: u64,
    per_page: u64,
    status_filter: Option<&str>,
    user_id_filter: Option<&str>,
) -> AppResult<(Vec<Node>, u64)> {
    let mut filter = doc! { "is_active": true };
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
pub async fn create_binding(
    db: &mongodb::Database,
    user_id: &str,
    node_id: &str,
    service_id: &str,
) -> AppResult<NodeServiceBinding> {
    // Verify node ownership
    let _node = get_node(db, user_id, node_id).await?;

    // Check for existing binding (same node + service)
    let existing = db
        .collection::<NodeServiceBinding>(NODE_SERVICE_BINDINGS)
        .find_one(doc! { "node_id": node_id, "service_id": service_id, "is_active": true })
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
        user_id: user_id.to_string(),
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
    // Verify node ownership
    let _node = get_node(db, user_id, node_id).await?;

    let bindings: Vec<NodeServiceBinding> = db
        .collection::<NodeServiceBinding>(NODE_SERVICE_BINDINGS)
        .find(doc! { "node_id": node_id, "is_active": true })
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
    let now = bson::DateTime::from_chrono(Utc::now());

    let result = db
        .collection::<NodeServiceBinding>(NODE_SERVICE_BINDINGS)
        .update_one(
            doc! { "_id": binding_id, "user_id": user_id, "node_id": node_id, "is_active": true },
            doc! { "$set": { "is_active": false, "updated_at": &now } },
        )
        .await?;

    if result.matched_count == 0 {
        return Err(AppError::NotFound("Binding not found".to_string()));
    }

    tracing::info!(binding_id = %binding_id, "Node service binding deleted");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::decode_node_signing_secret;

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
}

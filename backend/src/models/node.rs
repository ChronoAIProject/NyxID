use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::bson_datetime;

pub const COLLECTION_NAME: &str = "nodes";

/// Node connection status.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum NodeStatus {
    Online,
    Offline,
    Draining,
}

impl NodeStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Online => "online",
            Self::Offline => "offline",
            Self::Draining => "draining",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NodeMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub os: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub arch: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ip_address: Option<String>,
}

/// Per-node proxy metrics. Stored as an embedded document in the Node model.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct NodeMetrics {
    /// Total proxy requests handled
    #[serde(default)]
    pub total_requests: u64,
    /// Successful proxy responses (2xx-4xx from downstream)
    #[serde(default)]
    pub success_count: u64,
    /// Failed proxy requests (node errors, timeouts, 5xx)
    #[serde(default)]
    pub error_count: u64,
    /// Average response latency in milliseconds (exponential moving average)
    #[serde(default)]
    pub avg_latency_ms: f64,
    /// Last error message (for diagnostics)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_error: Option<String>,
    /// Timestamp of the last error
    #[serde(default, with = "bson_datetime::optional")]
    pub last_error_at: Option<DateTime<Utc>>,
    /// Timestamp of the last successful request
    #[serde(default, with = "bson_datetime::optional")]
    pub last_success_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Node {
    #[serde(rename = "_id")]
    pub id: String,
    pub user_id: String,
    pub name: String,
    pub status: NodeStatus,
    /// SHA-256 hash of the node's long-lived auth token
    pub auth_token_hash: String,
    /// Encrypted HMAC signing secret (raw hex string encrypted with app keys)
    #[serde(default, with = "crate::models::bson_bytes::optional")]
    pub signing_secret_encrypted: Option<Vec<u8>>,
    /// SHA-256 hash of the HMAC signing secret
    #[serde(default)]
    pub signing_secret_hash: String,
    #[serde(default, with = "bson_datetime::optional")]
    pub last_heartbeat_at: Option<DateTime<Utc>>,
    #[serde(default, with = "bson_datetime::optional")]
    pub connected_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<NodeMetadata>,
    /// Embedded proxy metrics
    #[serde(default)]
    pub metrics: NodeMetrics,
    pub is_active: bool,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collection_name() {
        assert_eq!(COLLECTION_NAME, "nodes");
    }

    fn make_node() -> Node {
        Node {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: uuid::Uuid::new_v4().to_string(),
            name: "test-node".to_string(),
            status: NodeStatus::Offline,
            auth_token_hash: "deadbeef".repeat(8),
            signing_secret_encrypted: Some(vec![1, 2, 3, 4]),
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

    #[test]
    fn bson_roundtrip() {
        let node = make_node();
        let doc = bson::to_document(&node).expect("serialize");
        let restored: Node = bson::from_document(doc).expect("deserialize");
        assert_eq!(node.id, restored.id);
        assert_eq!(node.name, restored.name);
        assert_eq!(node.status, NodeStatus::Offline);
        assert_eq!(node.auth_token_hash, restored.auth_token_hash);
        assert_eq!(
            node.signing_secret_encrypted,
            restored.signing_secret_encrypted
        );
    }

    #[test]
    fn bson_roundtrip_with_optional_dates() {
        let mut node = make_node();
        node.last_heartbeat_at = Some(Utc::now());
        node.connected_at = Some(Utc::now());
        node.metadata = Some(NodeMetadata {
            agent_version: Some("0.1.0".to_string()),
            os: Some("linux".to_string()),
            arch: Some("x86_64".to_string()),
            ip_address: None,
        });
        let doc = bson::to_document(&node).expect("serialize");
        let restored: Node = bson::from_document(doc).expect("deserialize");
        assert!(restored.last_heartbeat_at.is_some());
        assert!(restored.connected_at.is_some());
        assert!(restored.metadata.is_some());
    }
}

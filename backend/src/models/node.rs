use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provisioning_source: Option<String>,
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

/// Fenced ownership of a live node WebSocket.
///
/// The pod name alone is not a fence because Kubernetes reuses it after a
/// restart. `generation_id` distinguishes processes and `connection_id`
/// distinguishes reconnects handled by the same process.
#[derive(Clone, Serialize, Deserialize, PartialEq)]
pub struct NodeConnectionOwner {
    pub instance_name: String,
    pub generation_id: String,
    pub connection_id: String,
    pub internal_base_url: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub claimed_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub renewed_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
    #[serde(default)]
    pub credential_ack_correlation: bool,
    #[serde(default)]
    pub remote_credential_crypto_v1: bool,
    #[serde(default)]
    pub proxy_max_body_size: Option<usize>,
    #[serde(default)]
    pub capabilities_resolved: bool,
}

impl fmt::Debug for NodeConnectionOwner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NodeConnectionOwner")
            .field("instance_name", &self.instance_name)
            .field("generation_id", &self.generation_id)
            .field("connection_id", &self.connection_id)
            .field("internal_base_url", &"[REDACTED]")
            .field("claimed_at", &self.claimed_at)
            .field("renewed_at", &self.renewed_at)
            .field("expires_at", &self.expires_at)
            .field(
                "credential_ack_correlation",
                &self.credential_ack_correlation,
            )
            .field(
                "remote_credential_crypto_v1",
                &self.remote_credential_crypto_v1,
            )
            .field("proxy_max_body_size", &self.proxy_max_body_size)
            .field("capabilities_resolved", &self.capabilities_resolved)
            .finish()
    }
}

impl NodeConnectionOwner {
    pub fn is_live_at(&self, now: DateTime<Utc>) -> bool {
        self.expires_at > now
    }
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub connection_owner: Option<NodeConnectionOwner>,
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
            connection_owner: None,
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
            provisioning_source: None,
        });
        let doc = bson::to_document(&node).expect("serialize");
        let restored: Node = bson::from_document(doc).expect("deserialize");
        assert!(restored.last_heartbeat_at.is_some());
        assert!(restored.connected_at.is_some());
        assert!(restored.metadata.is_some());
    }

    #[test]
    fn node_status_as_str() {
        assert_eq!(NodeStatus::Online.as_str(), "online");
        assert_eq!(NodeStatus::Offline.as_str(), "offline");
        assert_eq!(NodeStatus::Draining.as_str(), "draining");
    }

    #[test]
    fn node_status_serde_roundtrip() {
        for status in [
            NodeStatus::Online,
            NodeStatus::Offline,
            NodeStatus::Draining,
        ] {
            let json = serde_json::to_string(&status).unwrap();
            let back: NodeStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(back, status);
        }
    }

    #[test]
    fn node_metrics_default() {
        let m = NodeMetrics::default();
        assert_eq!(m.total_requests, 0);
        assert_eq!(m.success_count, 0);
        assert_eq!(m.error_count, 0);
        assert_eq!(m.avg_latency_ms, 0.0);
        assert!(m.last_error.is_none());
        assert!(m.last_error_at.is_none());
        assert!(m.last_success_at.is_none());
    }

    #[test]
    fn bson_roundtrip_with_metrics() {
        let mut node = make_node();
        node.metrics = NodeMetrics {
            total_requests: 100,
            success_count: 95,
            error_count: 5,
            avg_latency_ms: 42.5,
            last_error: Some("timeout".to_string()),
            last_error_at: Some(Utc::now()),
            last_success_at: Some(Utc::now()),
        };
        let doc = bson::to_document(&node).expect("serialize");
        let restored: Node = bson::from_document(doc).expect("deserialize");
        assert_eq!(restored.metrics.total_requests, 100);
        assert_eq!(restored.metrics.error_count, 5);
        assert!(restored.metrics.last_error.is_some());
        assert!(restored.metrics.last_error_at.is_some());
    }

    #[test]
    fn bson_backward_compat_missing_metrics() {
        let node = make_node();
        let mut doc = bson::to_document(&node).expect("serialize");
        doc.remove("metrics");
        let restored: Node = bson::from_document(doc).expect("deserialize");
        assert_eq!(restored.metrics.total_requests, 0);
    }

    #[test]
    fn connection_owner_debug_redacts_internal_address() {
        let now = Utc::now();
        let owner = NodeConnectionOwner {
            instance_name: "backend-0".to_string(),
            generation_id: uuid::Uuid::new_v4().to_string(),
            connection_id: uuid::Uuid::new_v4().to_string(),
            internal_base_url: "http://10.0.0.7:3002".to_string(),
            claimed_at: now,
            renewed_at: now,
            expires_at: now + chrono::Duration::seconds(30),
            credential_ack_correlation: true,
            remote_credential_crypto_v1: true,
            proxy_max_body_size: Some(1024),
            capabilities_resolved: true,
        };

        let rendered = format!("{owner:?}");
        assert!(!rendered.contains("10.0.0.7"));
        assert!(rendered.contains("[REDACTED]"));
    }
}

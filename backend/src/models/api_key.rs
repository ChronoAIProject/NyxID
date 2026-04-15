use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::bson_datetime;

pub const COLLECTION_NAME: &str = "api_keys";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApiKey {
    #[serde(rename = "_id")]
    pub id: String,
    pub user_id: String,
    pub name: String,
    /// First 8+ characters of the key, used for identification in the UI
    pub key_prefix: String,
    /// SHA-256 hash of the full API key
    pub key_hash: String,
    pub scopes: String,
    #[serde(default, with = "bson_datetime::optional")]
    pub last_used_at: Option<DateTime<Utc>>,
    #[serde(default, with = "bson_datetime::optional")]
    pub expires_at: Option<DateTime<Utc>>,
    pub is_active: bool,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,

    // --- Service Scope (absorbed from AgentGroup) ---
    /// Optional description of what this key is used for
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,

    /// List of UserService IDs this key can access via proxy.
    /// Only checked when `allow_all_services` is false.
    #[serde(default)]
    pub allowed_service_ids: Vec<String>,

    /// List of Node IDs this key can route through.
    /// Only checked when `allow_all_nodes` is false.
    #[serde(default)]
    pub allowed_node_ids: Vec<String>,

    /// If true, key can access ALL of the user's external services.
    /// Default: true (backward compatible -- existing keys have no restrictions).
    #[serde(default = "default_true")]
    pub allow_all_services: bool,

    /// If true, key can route through ALL of the user's nodes.
    /// Default: true (backward compatible).
    #[serde(default = "default_true")]
    pub allow_all_nodes: bool,

    /// Per-agent rate limit override: max requests per second.
    /// When set, this key gets its own rate limit bucket.
    /// When None, falls back to user-level rate limits.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rate_limit_per_second: Option<u32>,

    /// Per-agent burst capacity override.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rate_limit_burst: Option<u32>,

    /// Platform label for this key (e.g. "claude-code", "codex", "openclaw", "generic").
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform: Option<String>,

    /// Callback URL for channel bot relay: the agent receives inbound messages here.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub callback_url: Option<String>,
}

fn default_true() -> bool {
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collection_name() {
        assert_eq!(COLLECTION_NAME, "api_keys");
    }

    fn make_api_key() -> ApiKey {
        ApiKey {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: uuid::Uuid::new_v4().to_string(),
            name: "My Key".to_string(),
            key_prefix: "abcdef01".to_string(),
            key_hash: "deadbeef".repeat(8),
            scopes: "read write".to_string(),
            last_used_at: None,
            expires_at: None,
            is_active: true,
            created_at: Utc::now(),
            description: None,
            allowed_service_ids: vec![],
            allowed_node_ids: vec![],
            allow_all_services: true,
            allow_all_nodes: true,
            rate_limit_per_second: None,
            rate_limit_burst: None,
            platform: None,
            callback_url: None,
        }
    }

    #[test]
    fn bson_roundtrip() {
        let key = make_api_key();
        let doc = bson::to_document(&key).expect("serialize");
        let restored: ApiKey = bson::from_document(doc).expect("deserialize");
        assert_eq!(key.id, restored.id);
        assert_eq!(key.name, restored.name);
        assert_eq!(key.scopes, restored.scopes);
    }

    #[test]
    fn bson_roundtrip_with_optional_dates() {
        let mut key = make_api_key();
        key.last_used_at = Some(Utc::now());
        key.expires_at = Some(Utc::now());
        let doc = bson::to_document(&key).expect("serialize");
        let restored: ApiKey = bson::from_document(doc).expect("deserialize");
        assert!(restored.last_used_at.is_some());
        assert!(restored.expires_at.is_some());
    }

    #[test]
    fn bson_roundtrip_with_rate_limit_and_platform() {
        let mut key = make_api_key();
        key.rate_limit_per_second = Some(10);
        key.rate_limit_burst = Some(20);
        key.platform = Some("claude-code".to_string());
        let doc = bson::to_document(&key).expect("serialize");
        let restored: ApiKey = bson::from_document(doc).expect("deserialize");
        assert_eq!(restored.rate_limit_per_second, Some(10));
        assert_eq!(restored.rate_limit_burst, Some(20));
        assert_eq!(restored.platform.as_deref(), Some("claude-code"));
    }

    #[test]
    fn bson_backward_compat_missing_rate_limit_fields() {
        let key = make_api_key();
        let mut doc = bson::to_document(&key).expect("serialize");
        // Simulate old document without the new fields
        doc.remove("rate_limit_per_second");
        doc.remove("rate_limit_burst");
        doc.remove("platform");
        doc.remove("callback_url");
        let restored: ApiKey = bson::from_document(doc).expect("deserialize");
        assert_eq!(restored.rate_limit_per_second, None);
        assert_eq!(restored.rate_limit_burst, None);
        assert_eq!(restored.platform, None);
        assert_eq!(restored.callback_url, None);
    }

    #[test]
    fn bson_roundtrip_with_callback_url() {
        let mut key = make_api_key();
        key.callback_url = Some("https://agent.example.com/callback".to_string());
        let doc = bson::to_document(&key).expect("serialize");
        let restored: ApiKey = bson::from_document(doc).expect("deserialize");
        assert_eq!(
            restored.callback_url.as_deref(),
            Some("https://agent.example.com/callback")
        );
    }

    #[test]
    fn bson_null_callback_url_deserializes_to_none() {
        let mut key = make_api_key();
        key.callback_url = Some("https://old.example.com/hook".to_string());
        let mut doc = bson::to_document(&key).expect("serialize");
        doc.insert("callback_url", bson::Bson::Null);
        let restored: ApiKey = bson::from_document(doc).expect("deserialize");
        assert_eq!(restored.callback_url, None);
    }
}

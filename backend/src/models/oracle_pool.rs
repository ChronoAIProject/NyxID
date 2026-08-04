use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "oracle_pools";

/// Who may submit tasks to a pool (beyond the owner, who always can).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum OraclePoolVisibility {
    /// Only the owner (person, or org members when org-owned).
    Private,
    /// Members of the owning org. Only valid for org-owned pools.
    Org,
    /// Any authenticated user on this NyxID instance.
    Platform,
}

impl OraclePoolVisibility {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Private => "private",
            Self::Org => "org",
            Self::Platform => "platform",
        }
    }
}

/// A browser-oracle capacity pool: one ChatGPT (or similar) account whose
/// logged-in browser tabs serve tasks via the worker API. The pool owner
/// installs the userscript and holds the worker token; consumers submit
/// tasks through `/api/v1/oracle` and never touch the browser side.
///
/// NyxID itself stays a generic async task relay: nothing in this model is
/// specific to ChatGPT — `chatgpt_project_url` and `default_model_label`
/// are opaque hints forwarded to workers.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OraclePool {
    #[serde(rename = "_id")]
    pub id: String,
    /// Polymorphic owner: a person user or an org user (`user_type=Org`),
    /// matching the `UserService` / `Node` convention. Use
    /// `org_service::resolve_owner_access` for ACL checks.
    pub user_id: String,
    /// URL-safe unique identifier used in API paths.
    pub slug: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub visibility: OraclePoolVisibility,
    /// SHA-256 hash of the pool worker token (`nyx_owk_...`). The raw token
    /// is shown once at creation/rotation and never stored.
    pub worker_token_hash: String,
    /// Optional browser-side pinning hint (e.g. a ChatGPT Project URL).
    /// Relayed verbatim to workers; never interpreted by the server.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chatgpt_project_url: Option<String>,
    /// Opaque model label forwarded to workers and recorded on results.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_model_label: Option<String>,
    /// Whether this pool may drive a worker browser to extract arbitrary URLs.
    #[serde(default)]
    pub allow_extract: bool,
    /// Maximum tasks dispatched (in flight) at once across all workers.
    pub max_workers: u32,
    /// Maximum queued (not yet dispatched) tasks before submits are rejected.
    pub max_queue_length: u32,
    /// Per-submitter cap on queued + dispatched tasks in this pool.
    pub per_user_max_inflight: u32,
    /// Dispatch lease in seconds; heartbeats refresh it. Expired leases are
    /// requeued. Default 14400 (4h) — browser deep-reasoning turns are slow.
    pub task_timeout_secs: u64,
    pub is_active: bool,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

pub const DEFAULT_MAX_WORKERS: u32 = 3;
pub const DEFAULT_MAX_QUEUE_LENGTH: u32 = 50;
pub const DEFAULT_PER_USER_MAX_INFLIGHT: u32 = 2;
pub const DEFAULT_TASK_TIMEOUT_SECS: u64 = 14_400;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collection_name() {
        assert_eq!(COLLECTION_NAME, "oracle_pools");
    }

    fn make_pool() -> OraclePool {
        OraclePool {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: uuid::Uuid::new_v4().to_string(),
            slug: "chatgpt-pro".to_string(),
            name: "ChatGPT Pro".to_string(),
            description: Some("Lexa's Pro account".to_string()),
            visibility: OraclePoolVisibility::Platform,
            worker_token_hash: "deadbeef".repeat(8),
            chatgpt_project_url: None,
            default_model_label: Some("chatgpt-5.5-pro".to_string()),
            allow_extract: false,
            max_workers: DEFAULT_MAX_WORKERS,
            max_queue_length: DEFAULT_MAX_QUEUE_LENGTH,
            per_user_max_inflight: DEFAULT_PER_USER_MAX_INFLIGHT,
            task_timeout_secs: DEFAULT_TASK_TIMEOUT_SECS,
            is_active: true,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn bson_roundtrip() {
        let pool = make_pool();
        let doc = bson::to_document(&pool).expect("serialize");
        let restored: OraclePool = bson::from_document(doc).expect("deserialize");
        assert_eq!(pool.id, restored.id);
        assert_eq!(pool.slug, restored.slug);
        assert_eq!(restored.visibility, OraclePoolVisibility::Platform);
        assert!(!restored.allow_extract);
        assert_eq!(restored.max_workers, DEFAULT_MAX_WORKERS);
        assert_eq!(restored.task_timeout_secs, DEFAULT_TASK_TIMEOUT_SECS);
    }

    #[test]
    fn visibility_serde_roundtrip() {
        for v in [
            OraclePoolVisibility::Private,
            OraclePoolVisibility::Org,
            OraclePoolVisibility::Platform,
        ] {
            let json = serde_json::to_string(&v).unwrap();
            let back: OraclePoolVisibility = serde_json::from_str(&json).unwrap();
            assert_eq!(back, v);
        }
        assert_eq!(
            serde_json::to_string(&OraclePoolVisibility::Platform).unwrap(),
            "\"platform\""
        );
    }

    #[test]
    fn visibility_as_str() {
        assert_eq!(OraclePoolVisibility::Private.as_str(), "private");
        assert_eq!(OraclePoolVisibility::Org.as_str(), "org");
        assert_eq!(OraclePoolVisibility::Platform.as_str(), "platform");
    }
}

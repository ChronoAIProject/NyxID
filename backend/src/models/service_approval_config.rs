use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "service_approval_configs";

/// Per-service approval override for a user.
///
/// When a user has global `approval_required = true`, they can exempt specific
/// services (set `approval_required = false`). Conversely, when global is false,
/// they can require approval for specific high-risk services.
///
/// If no config exists for a (user, service) pair, the global
/// `notification_channels.approval_required` setting applies.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ServiceApprovalConfig {
    /// UUID v4 string
    #[serde(rename = "_id")]
    pub id: String,

    /// Owner user ID
    pub user_id: String,

    /// Downstream service ID
    pub service_id: String,

    /// Human-readable service name (denormalized for display)
    pub service_name: String,

    /// Whether approval is required for this specific service.
    /// Overrides the global `notification_channels.approval_required`.
    pub approval_required: bool,

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
        assert_eq!(COLLECTION_NAME, "service_approval_configs");
    }

    fn make_config() -> ServiceApprovalConfig {
        ServiceApprovalConfig {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: uuid::Uuid::new_v4().to_string(),
            service_id: uuid::Uuid::new_v4().to_string(),
            service_name: "OpenAI API".to_string(),
            approval_required: true,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn bson_roundtrip() {
        let cfg = make_config();
        let doc = bson::to_document(&cfg).expect("serialize");
        assert!(doc.get_str("_id").is_ok());
        assert!(doc.get("id").is_none(), "raw 'id' should not exist in bson");
        let restored: ServiceApprovalConfig = bson::from_document(doc).expect("deserialize");
        assert_eq!(cfg.id, restored.id);
        assert_eq!(cfg.user_id, restored.user_id);
        assert_eq!(cfg.service_id, restored.service_id);
        assert_eq!(cfg.approval_required, restored.approval_required);
    }

    #[test]
    fn bson_all_fields_serialized() {
        let cfg = make_config();
        let doc = bson::to_document(&cfg).expect("serialize");
        let keys: Vec<&str> = doc.keys().map(|k| k.as_str()).collect();
        assert!(keys.contains(&"_id"));
        assert!(keys.contains(&"user_id"));
        assert!(keys.contains(&"service_id"));
        assert!(keys.contains(&"service_name"));
        assert!(keys.contains(&"approval_required"));
        assert!(keys.contains(&"created_at"));
        assert!(keys.contains(&"updated_at"));
    }
}

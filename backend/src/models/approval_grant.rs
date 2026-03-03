use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "approval_grants";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ApprovalGrant {
    /// UUID v4 string
    #[serde(rename = "_id")]
    pub id: String,

    /// The user who granted approval
    pub user_id: String,

    /// The service this grant applies to
    pub service_id: String,

    /// Human-readable service name (denormalized from ApprovalRequest)
    pub service_name: String,

    /// Who was granted access (requester_type + requester_id pair)
    pub requester_type: String,
    pub requester_id: String,

    /// Human-readable requester label (denormalized from ApprovalRequest)
    pub requester_label: Option<String>,

    /// The approval_request._id that created this grant
    pub approval_request_id: String,

    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub granted_at: DateTime<Utc>,

    /// When this grant expires (user-configurable, default 30 days)
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,

    /// Whether this grant has been explicitly revoked
    #[serde(default)]
    pub revoked: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collection_name() {
        assert_eq!(COLLECTION_NAME, "approval_grants");
    }

    fn make_approval_grant() -> ApprovalGrant {
        ApprovalGrant {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: uuid::Uuid::new_v4().to_string(),
            service_id: uuid::Uuid::new_v4().to_string(),
            service_name: "OpenAI API".to_string(),
            requester_type: "service_account".to_string(),
            requester_id: uuid::Uuid::new_v4().to_string(),
            requester_label: Some("CI Pipeline".to_string()),
            approval_request_id: uuid::Uuid::new_v4().to_string(),
            granted_at: Utc::now(),
            expires_at: Utc::now() + chrono::Duration::days(30),
            revoked: false,
        }
    }

    #[test]
    fn bson_roundtrip() {
        let grant = make_approval_grant();
        let doc = bson::to_document(&grant).expect("serialize");
        assert!(doc.get_str("_id").is_ok());
        assert!(doc.get("id").is_none(), "raw 'id' should not exist in bson");
        let restored: ApprovalGrant = bson::from_document(doc).expect("deserialize");
        assert_eq!(grant.id, restored.id);
        assert_eq!(grant.user_id, restored.user_id);
        assert_eq!(grant.revoked, restored.revoked);
    }

    #[test]
    fn bson_all_fields_serialized() {
        let grant = make_approval_grant();
        let doc = bson::to_document(&grant).expect("serialize");
        let keys: Vec<&str> = doc.keys().map(|k| k.as_str()).collect();
        assert!(keys.contains(&"_id"));
        assert!(keys.contains(&"user_id"));
        assert!(keys.contains(&"service_id"));
        assert!(keys.contains(&"service_name"));
        assert!(keys.contains(&"requester_type"));
        assert!(keys.contains(&"requester_id"));
        assert!(keys.contains(&"requester_label"));
        assert!(keys.contains(&"approval_request_id"));
        assert!(keys.contains(&"granted_at"));
        assert!(keys.contains(&"expires_at"));
        assert!(keys.contains(&"revoked"));
    }
}

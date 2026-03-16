use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::bson_datetime;

pub const COLLECTION_NAME: &str = "approval_requests";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ApprovalRequest {
    /// UUID v4 string
    #[serde(rename = "_id")]
    pub id: String,

    /// The user who must approve this request
    pub user_id: String,

    /// The downstream service being accessed
    pub service_id: String,

    /// Human-readable service name (denormalized for Telegram message)
    pub service_name: String,

    /// The service slug (denormalized for display)
    pub service_slug: String,

    /// Who is making the request: "user", "service_account", or "delegated"
    pub requester_type: String,

    /// ID of the requester (user_id, service_account_id, or client_id)
    pub requester_id: String,

    /// Human-readable requester label (e.g. SA name, OAuth client name)
    pub requester_label: Option<String>,

    /// What operation is being performed (e.g. "proxy:GET /v1/chat/completions")
    pub operation_summary: String,

    /// "pending" | "approved" | "rejected" | "expired"
    pub status: String,

    /// SHA-256 of (user_id + service_id + requester_id + requester_type).
    /// Prevents duplicate pending requests for the same context.
    pub idempotency_key: String,

    /// Which notification channel delivered this request (e.g. "telegram")
    #[serde(default)]
    pub notification_channel: Option<String>,

    /// Telegram message_id for editing the message after decision
    #[serde(default)]
    pub telegram_message_id: Option<i64>,

    /// Telegram chat_id where the notification was sent
    #[serde(default)]
    pub telegram_chat_id: Option<i64>,

    /// When the approval request expires (auto-reject after this time)
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,

    /// When the user made their decision (approved/rejected)
    #[serde(default, with = "bson_datetime::optional")]
    pub decided_at: Option<DateTime<Utc>>,

    /// Channel through which the decision was made (e.g. "telegram", "web")
    #[serde(default)]
    pub decision_channel: Option<String>,

    /// Idempotency key used for the final decision submission.
    #[serde(default)]
    pub decision_idempotency_key: Option<String>,

    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collection_name() {
        assert_eq!(COLLECTION_NAME, "approval_requests");
    }

    fn make_approval_request() -> ApprovalRequest {
        ApprovalRequest {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: uuid::Uuid::new_v4().to_string(),
            service_id: uuid::Uuid::new_v4().to_string(),
            service_name: "OpenAI API".to_string(),
            service_slug: "openai".to_string(),
            requester_type: "service_account".to_string(),
            requester_id: uuid::Uuid::new_v4().to_string(),
            requester_label: Some("CI Pipeline".to_string()),
            operation_summary: "proxy:POST /v1/chat/completions".to_string(),
            status: "pending".to_string(),
            idempotency_key: "abc123".to_string(),
            notification_channel: Some("telegram".to_string()),
            telegram_message_id: Some(12345),
            telegram_chat_id: Some(67890),
            expires_at: Utc::now(),
            decided_at: None,
            decision_channel: None,
            decision_idempotency_key: None,
            created_at: Utc::now(),
        }
    }

    #[test]
    fn bson_roundtrip() {
        let req = make_approval_request();
        let doc = bson::to_document(&req).expect("serialize");
        assert!(doc.get_str("_id").is_ok());
        assert!(doc.get("id").is_none(), "raw 'id' should not exist in bson");
        let restored: ApprovalRequest = bson::from_document(doc).expect("deserialize");
        assert_eq!(req.id, restored.id);
        assert_eq!(req.user_id, restored.user_id);
        assert_eq!(req.status, restored.status);
    }

    #[test]
    fn bson_roundtrip_with_optional_datetime() {
        let mut req = make_approval_request();
        req.decided_at = Some(Utc::now());
        req.decision_channel = Some("telegram".to_string());
        let doc = bson::to_document(&req).expect("serialize");
        let restored: ApprovalRequest = bson::from_document(doc).expect("deserialize");
        assert!(restored.decided_at.is_some());
    }

    #[test]
    fn bson_all_fields_serialized() {
        let req = make_approval_request();
        let doc = bson::to_document(&req).expect("serialize");
        let keys: Vec<&str> = doc.keys().map(|k| k.as_str()).collect();
        assert!(keys.contains(&"_id"));
        assert!(keys.contains(&"user_id"));
        assert!(keys.contains(&"service_id"));
        assert!(keys.contains(&"service_name"));
        assert!(keys.contains(&"service_slug"));
        assert!(keys.contains(&"requester_type"));
        assert!(keys.contains(&"requester_id"));
        assert!(keys.contains(&"operation_summary"));
        assert!(keys.contains(&"status"));
        assert!(keys.contains(&"idempotency_key"));
        assert!(keys.contains(&"expires_at"));
        assert!(keys.contains(&"created_at"));
    }
}

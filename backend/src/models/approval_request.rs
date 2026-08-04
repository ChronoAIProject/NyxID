use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::bson_datetime;
use crate::models::service_approval_config::{ApprovalMode, legacy_approval_mode_default};

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

    /// Rich human-readable description of what the API request does.
    /// e.g., "POST /v1/chat/completions (model: gpt-4, max_tokens: 1000)"
    /// Falls back to operation_summary if not generated.
    #[serde(default)]
    pub action_description: Option<String>,

    /// Structured operation identity captured from OperationDescriptor.
    #[serde(default)]
    pub http_method: Option<String>,

    /// Normalized resource path/tool/command for display and grant scoping.
    #[serde(default)]
    pub resource: Option<String>,

    /// Semantic operation verb: "read", "write", or "destructive".
    #[serde(default)]
    pub verb: Option<String>,

    /// Internal grant scope minted if a grant-mode request is approved.
    #[serde(default)]
    pub grant_scope: Option<String>,

    /// Tool approval fields (set when created via POST /api/v1/approvals/requests).
    /// All optional -- `None` for proxy-initiated approval requests.

    /// Name of the agent tool requesting approval (e.g. "invoke_service")
    #[serde(default)]
    pub tool_name: Option<String>,

    /// LLM-generated tool call ID for correlation
    #[serde(default)]
    pub tool_call_id: Option<String>,

    /// Serialized JSON of tool arguments
    #[serde(default)]
    pub tool_arguments: Option<String>,

    /// Whether the tool performs irreversible operations
    #[serde(default)]
    pub is_destructive: Option<bool>,

    /// Approval semantics captured at request creation time.
    /// Legacy requests created before this field existed default to grant mode
    /// so their original behavior is preserved when decided later.
    #[serde(default = "legacy_approval_mode_default")]
    pub approval_mode: ApprovalMode,

    /// "pending" | "approved" | "rejected" | "expired"
    pub status: String,

    /// Pending request dedupe key.
    /// Grant mode uses a stable hash for `(user, service, requester)`;
    /// per-request mode uses a unique value per incoming API call.
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
    /// System-generated expiry sweeps may also stamp an internal marker here
    /// to identify the rows they expired.
    #[serde(default)]
    pub decision_idempotency_key: Option<String>,

    /// Users who were notified of this approval request and are authorized
    /// to decide on it. For personal approvals this is `[user_id]`. For
    /// org-policy approvals (where the org has set a `service_approval_config`
    /// on a shared service) this is the list of admin user_ids of the
    /// owning org at the time the request was created. The decide endpoint
    /// allows any user in this list to approve/reject in addition to
    /// `request.user_id` (for backward compat with pre-org rows where this
    /// field is empty).
    ///
    /// Default `vec![]` so legacy rows deserialize cleanly; the decide
    /// endpoint treats an empty list as "fall back to user_id only".
    #[serde(default)]
    pub notify_user_ids: Vec<String>,

    /// Whether this request was created under an org's per-service approval
    /// policy (rather than the actor's personal policy). When true, `user_id`
    /// is the org_id, `notify_user_ids` holds the org's admin list, and any
    /// resulting grant is reusable by every member of that org (see #364).
    ///
    /// Default `false` so legacy rows (created before the org cascade was
    /// introduced) keep their original per-actor semantics on decision.
    #[serde(default)]
    pub from_org_policy: bool,

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
            action_description: Some(
                "POST /v1/chat/completions (model: gpt-4, 3 messages)".to_string(),
            ),
            http_method: Some("POST".to_string()),
            resource: Some("/v1/chat/completions".to_string()),
            verb: Some("write".to_string()),
            grant_scope: Some("v1:http:post:write:/v1/chat/completions".to_string()),
            tool_name: None,
            tool_call_id: None,
            tool_arguments: None,
            is_destructive: None,
            approval_mode: ApprovalMode::PerRequest,
            status: "pending".to_string(),
            idempotency_key: "abc123".to_string(),
            notification_channel: Some("telegram".to_string()),
            telegram_message_id: Some(12345),
            telegram_chat_id: Some(67890),
            expires_at: Utc::now(),
            decided_at: None,
            decision_channel: None,
            decision_idempotency_key: None,
            notify_user_ids: vec![],
            from_org_policy: false,
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
    fn missing_action_description_defaults_to_none() {
        let req = make_approval_request();
        let mut doc = bson::to_document(&req).expect("serialize");
        doc.remove("action_description");
        let restored: ApprovalRequest = bson::from_document(doc).expect("deserialize");
        assert!(restored.action_description.is_none());
    }

    #[test]
    fn action_description_roundtrips() {
        let req = make_approval_request();
        let doc = bson::to_document(&req).expect("serialize");
        let restored: ApprovalRequest = bson::from_document(doc).expect("deserialize");
        assert_eq!(
            restored.action_description.as_deref(),
            Some("POST /v1/chat/completions (model: gpt-4, 3 messages)")
        );
    }

    #[test]
    fn missing_operation_identity_defaults_to_none() {
        let req = make_approval_request();
        let mut doc = bson::to_document(&req).expect("serialize");
        doc.remove("http_method");
        doc.remove("resource");
        doc.remove("verb");
        doc.remove("grant_scope");
        let restored: ApprovalRequest = bson::from_document(doc).expect("deserialize");
        assert!(restored.http_method.is_none());
        assert!(restored.resource.is_none());
        assert!(restored.verb.is_none());
        assert!(restored.grant_scope.is_none());
    }

    #[test]
    fn missing_from_org_policy_defaults_to_false_for_legacy_rows() {
        // Legacy approval_requests documents predate from_org_policy (added
        // as part of the org grant reuse fix for ChronoAIProject/NyxID#364
        // and ChronoAIProject/NyxID#370). They must still deserialize, and
        // process_decision must treat them as personal requests so a grant
        // created from one does not silently become org-reusable.
        let req = make_approval_request();
        let mut doc = bson::to_document(&req).expect("serialize");
        doc.remove("from_org_policy");
        let restored: ApprovalRequest = bson::from_document(doc).expect("deserialize");
        assert!(!restored.from_org_policy);
    }

    #[test]
    fn from_org_policy_roundtrips() {
        let mut req = make_approval_request();
        req.from_org_policy = true;
        let doc = bson::to_document(&req).expect("serialize");
        assert!(doc.get_bool("from_org_policy").unwrap());
        let restored: ApprovalRequest = bson::from_document(doc).expect("deserialize");
        assert!(restored.from_org_policy);
    }

    #[test]
    fn missing_approval_mode_defaults_to_grant_for_legacy_requests() {
        let req = make_approval_request();
        let mut doc = bson::to_document(&req).expect("serialize");
        doc.remove("approval_mode");
        let restored: ApprovalRequest = bson::from_document(doc).expect("deserialize");
        assert_eq!(restored.approval_mode, ApprovalMode::Grant);
    }

    #[test]
    fn bson_roundtrip_with_tool_fields() {
        let mut req = make_approval_request();
        req.tool_name = Some("invoke_service".to_string());
        req.tool_call_id = Some("call_abc123".to_string());
        req.tool_arguments = Some(r#"{"service_id":"svc_1","endpoint_id":"ep_1"}"#.to_string());
        req.is_destructive = Some(true);

        let doc = bson::to_document(&req).expect("serialize");
        assert_eq!(doc.get_str("tool_name").unwrap(), "invoke_service");
        assert!(doc.get_bool("is_destructive").unwrap());

        let restored: ApprovalRequest = bson::from_document(doc).expect("deserialize");
        assert_eq!(restored.tool_name.as_deref(), Some("invoke_service"));
        assert_eq!(restored.tool_call_id.as_deref(), Some("call_abc123"));
        assert!(restored.tool_arguments.is_some());
        assert_eq!(restored.is_destructive, Some(true));
    }

    #[test]
    fn missing_tool_fields_default_to_none() {
        let req = make_approval_request();
        let mut doc = bson::to_document(&req).expect("serialize");
        // Remove all tool fields (simulates old document without them)
        doc.remove("tool_name");
        doc.remove("tool_call_id");
        doc.remove("tool_arguments");
        doc.remove("is_destructive");

        let restored: ApprovalRequest = bson::from_document(doc).expect("deserialize");
        assert!(restored.tool_name.is_none());
        assert!(restored.tool_call_id.is_none());
        assert!(restored.tool_arguments.is_none());
        assert!(restored.is_destructive.is_none());
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
        assert!(keys.contains(&"approval_mode"));
        assert!(keys.contains(&"status"));
        assert!(keys.contains(&"idempotency_key"));
        assert!(keys.contains(&"expires_at"));
        assert!(keys.contains(&"created_at"));
    }
}

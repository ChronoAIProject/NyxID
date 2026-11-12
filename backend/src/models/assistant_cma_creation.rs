//! A creation identity exists before a remote Agent ID, independent of watches.
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "assistant_cma_creations";

#[derive(Clone, Serialize, Deserialize)]
pub struct AssistantCmaCreation {
    #[serde(rename = "_id")]
    pub id: String,
    pub user_id: String,
    /// Exact UserService connection used for this creation. This is the
    /// durable routing identity; `cma_service_id` remains the catalog class.
    pub user_service_id: String,
    pub cma_service_id: String,
    /// The effective endpoint resolved at admission time. It is used only to
    /// fence recovery against a changed connection target.
    pub execution_base_url: String,
    pub method: String,
    pub path: String,
    pub idempotency_key: String,
    pub request_hash: String,
    pub sandbox_ref_id: String,
    pub sandbox_generation: i64,
    pub creates_sandbox: bool,
    #[serde(default)]
    pub expected_workspace_id: Option<String>,
    pub submission_state: String,
    pub operation_id: Option<String>,
    pub cma_agent_id: Option<String>,
    #[serde(default)]
    pub cma_sandbox_id: Option<String>,
    #[serde(default)]
    pub revision: i64,
    /// Separates an explicit retry after proven no-resource from late replies
    /// to the preceding submission, even when the parent Sandbox is unchanged.
    #[serde(default)]
    pub submission_generation: i64,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub recovery_checked_at: Option<DateTime<Utc>>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub first_dispatched_at: Option<DateTime<Utc>>,
    #[serde(default, with = "crate::models::bson_bytes::optional")]
    pub request_ciphertext: Option<Vec<u8>>,
    #[serde(default)]
    /// Absent/null selects legacy encryption; 3 selects contextual encryption.
    pub request_context_version: Option<i32>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}
impl std::fmt::Debug for AssistantCmaCreation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AssistantCmaCreation")
            .field("id", &self.id)
            .field("submission_state", &self.submission_state)
            .finish_non_exhaustive()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn recovery_material_is_stored_but_never_debugged() {
        let row = AssistantCmaCreation {
            id: "id".into(),
            user_id: "owner".into(),
            user_service_id: "user-service".into(),
            cma_service_id: "service".into(),
            execution_base_url: "https://cma.example".into(),
            method: "POST".into(),
            path: "/api/v1/agents".into(),
            idempotency_key: "private-key".into(),
            request_hash: "hash".into(),
            sandbox_ref_id: "slot".into(),
            sandbox_generation: 1,
            creates_sandbox: false,
            expected_workspace_id: None,
            submission_state: "pending".into(),
            operation_id: None,
            cma_agent_id: None,
            cma_sandbox_id: None,
            revision: 0,
            submission_generation: 0,
            recovery_checked_at: Some(Utc::now()),
            first_dispatched_at: Some(Utc::now()),
            request_ciphertext: Some(vec![1, 2, 3]),
            request_context_version: Some(3),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        assert!(!format!("{row:?}").contains("https://cma.example"));
        let document = mongodb::bson::to_document(&row).unwrap();
        assert!(document.get("request_ciphertext").is_some());
        for key in [
            "created_at",
            "updated_at",
            "recovery_checked_at",
            "first_dispatched_at",
        ] {
            assert!(document.get_datetime(key).is_ok());
        }
        assert!(!format!("{row:?}").contains("private-key"));
        assert!(!format!("{row:?}").contains("ciphertext"));
    }
}

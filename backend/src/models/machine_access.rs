//! Durable capability policy and metadata-only authority leases.
use chrono::{DateTime, Utc};
use nyxid_machine::authority::{Authority, Capabilities};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const CONTEXTS: &str = "machine_contexts";
pub const LEASES: &str = "machine_authority_leases";
pub const OUTBOX: &str = "machine_revocations";
pub const CUTOVER: &str = "machine-capabilities-v2";
pub const SEPARATION_CUTOVER: &str = "machine-separation-defaults-v1";
pub const SEPARATION_COMPATIBILITY: &str = "machine_separation_compatibility";
pub const SELECTION_REQUIRED: &str = "selection_required";

/// Explicit rollout initialization; omission retains established legacy writers.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AssignmentIntent {
    MembershipOnly,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Assignment {
    pub capabilities: Capabilities,
    pub mode: String,
    pub revision: i64,
    /// Cutover snapshots and flag-off compatibility grants; explicit edits write false.
    pub legacy: bool,
    #[serde(default)]
    pub saved_login_ids: Option<Vec<String>>,
}
impl Default for Assignment {
    fn default() -> Self {
        Self {
            capabilities: Capabilities::default(),
            mode: "shared_legacy".into(),
            revision: 1,
            legacy: false,
            saved_login_ids: None,
        }
    }
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct Policy {
    pub version: u32,
    /// Editor CAS and revision allocator. New/re-added assignments take this value;
    /// unchanged assignments retain their own execution revision.
    pub revision: i64,
    pub assignments: BTreeMap<String, Assignment>,
}
impl Default for Policy {
    fn default() -> Self {
        Self {
            version: 2,
            revision: 1,
            assignments: BTreeMap::new(),
        }
    }
}
#[derive(Clone, Deserialize, Serialize)]
pub struct Context {
    #[serde(rename = "_id")]
    pub id: String,
    pub node_id: String,
    pub agent_id: String,
    pub owner_id: String,
    pub actor_id: String,
    pub group_id: Option<String>,
    pub generation: i64,
    /// Authenticated daemon for which this profile generation was authorized.
    /// A replacement must advance before ephemeral profile reconstruction.
    pub runtime_id: String,
    pub mode: String,
}
#[derive(Clone, Deserialize, Serialize)]
pub struct Lease {
    #[serde(rename = "_id")]
    pub id: String,
    pub node_id: String,
    pub api_key_id: String,
    pub authority: Box<Authority>,
    pub job_id: Option<String>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Selection {
    /// Omitted preserves the assignment mode; old writers cannot widen it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    pub expected_revision: i64,
    pub capabilities: Capabilities,
    #[serde(default)]
    pub saved_login_ids: Option<Vec<String>>,
}

/// Bounded, server-authored payload attached to a human context-change card.
/// It contains no free-form content or credentials and is applied only after
/// the existing acknowledgement is allowed.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct HumanContextAction {
    pub agent_id: String,
    pub node_id: String,
    pub selection: Selection,
    pub snapshot: ContextActionSnapshot,
}

pub const CONTEXT_POLICY_VERSION: &str = "machine-context-mode-v1";
pub const CONTEXT_REVIEW_FENCE: &str = "machine-context-review-dependencies";

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ContextActionSnapshot {
    pub actor_id: String,
    pub agent_owner_id: String,
    pub owner_id: String,
    pub previous_assignment: Assignment,
    pub policy_version: String,
    pub profile_digest: String,
    pub capability_edit_enabled: bool,
    pub context_setup_enabled: bool,
    pub flag_epoch: i64,
    pub memberships_digest: String,
    pub machine_configuration: Option<ContextMachineConfiguration>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct ContextMachineConfiguration {
    pub revision: u64,
    pub digest: String,
    pub runtime_generation: i64,
    pub credential_epoch: i64,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ContextReview {
    #[serde(rename = "_id")]
    pub id: String,
    pub actor_id: String,
    pub action: HumanContextAction,
    pub status: String,
    pub committed_revision: Option<i64>,
    pub error_code: Option<String>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
}
impl ContextReview {
    pub const COLLECTION_NAME: &str = "machine_context_reviews";
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn context_selection_accepts_mode_without_leaking_it_into_legacy_writers() {
        let value = json!({
            "mode": "separated",
            "expected_revision": 7,
            "capabilities": {
                "shell": true,
                "files": true,
                "browser": true,
                "computer": false,
                "developer_browser": false
            },
            "saved_login_ids": []
        });
        let selection: Selection = serde_json::from_value(value).unwrap();
        assert_eq!(selection.mode.as_deref(), Some("separated"));
        let legacy = serde_json::to_value(&Selection {
            mode: None,
            expected_revision: selection.expected_revision,
            capabilities: selection.capabilities,
            saved_login_ids: None,
        })
        .unwrap();
        assert!(legacy.get("mode").is_none());
    }

    #[test]
    fn context_selection_rejects_unrelated_fields() {
        let error = serde_json::from_value::<Selection>(json!({
            "mode": "separated",
            "expected_revision": 1,
            "capabilities": {
                "shell": false,
                "files": false,
                "browser": true,
                "computer": false,
                "developer_browser": false
            },
            "saved_login_ids": [],
            "skill": "wrong-schema"
        }));
        assert!(error.is_err());
    }
}

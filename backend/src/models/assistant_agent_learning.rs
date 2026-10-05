//! Additive state for L1 automatic agent learning.
//!
//! The learning collections are deliberately separate from AssistantAgent and
//! AssistantConversation. Older binaries therefore retain their existing
//! behavior and cannot erase configuration while rewriting grants or threads.
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const CONFIG_COLLECTION_NAME: &str = "assistant_agent_learning";
pub const MEMBERS_COLLECTION_NAME: &str = "assistant_agent_learning_members";
pub const RUNS_COLLECTION_NAME: &str = "assistant_agent_learning_runs";
pub const PROPOSALS_COLLECTION_NAME: &str = "assistant_agent_learning_proposals";
pub const REJECTIONS_COLLECTION_NAME: &str = "assistant_agent_learning_rejections";
pub const ROOTS_COLLECTION_NAME: &str = "assistant_agent_learning_skill_roots";

pub const DEFAULT_THRESHOLD: i64 = 15;
#[allow(dead_code)]
pub const MIN_THRESHOLD: i64 = 1;
#[allow(dead_code)]
pub const MAX_THRESHOLD: i64 = 50;
#[allow(dead_code)]
pub const REJECTION_TTL_DAYS: i64 = 180;
#[allow(dead_code)]
pub const MAX_REJECTIONS_PER_AGENT: i64 = 256;
pub const MAX_EVIDENCE_PER_RUN: usize = 55;
pub const MAX_PROPOSAL_BYTES: usize = 8_000;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct LearningCursor {
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    pub conversation_id: String,
    pub turn_id: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AssistantAgentLearning {
    #[serde(rename = "_id")]
    pub agent_id: String,
    pub owner_id: String,
    #[serde(default)]
    pub enabled: bool,
    #[serde(default = "default_threshold")]
    pub threshold: i64,
    #[serde(default)]
    pub learning_epoch: i64,
    #[serde(default)]
    pub config_revision: i64,
    /// The maintainer whose enable action owns threshold-run billing for an
    /// org agent. Personal rows store their personal owner.
    #[serde(default)]
    pub enabled_by: Option<String>,
    #[serde(default)]
    pub last_success_cursor: Option<LearningCursor>,
    #[serde(default)]
    pub last_success_run_id: Option<String>,
    #[serde(default)]
    pub eligible_count: i64,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub last_run_at: Option<DateTime<Utc>>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub last_success_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub last_error_code: Option<String>,
    #[serde(default)]
    pub lease_owner: Option<String>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub lease_expires_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub fence: i64,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

fn default_threshold() -> i64 {
    DEFAULT_THRESHOLD
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AssistantAgentLearningMember {
    #[serde(rename = "_id")]
    pub id: String,
    pub agent_id: String,
    pub member_user_id: String,
    #[serde(default)]
    pub opted_in: bool,
    #[serde(default)]
    pub revision: i64,
    #[serde(default)]
    pub learning_epoch: i64,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LearningEvidence {
    pub label: String,
    pub conversation_id: String,
    pub turn_id: String,
    #[serde(default)]
    pub principal_id: String,
    #[serde(default)]
    pub consent_revision: Option<i64>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AssistantAgentLearningRun {
    #[serde(rename = "_id")]
    pub id: String,
    pub agent_id: String,
    pub owner_id: String,
    pub actor_id: String,
    pub mode: String,
    pub config_revision: i64,
    pub learning_epoch: i64,
    #[serde(default)]
    pub cursor_before: Option<LearningCursor>,
    #[serde(default)]
    pub candidate_watermark: Option<LearningCursor>,
    #[serde(default)]
    pub evidence: Vec<LearningEvidence>,
    #[serde(default)]
    pub input_digest: Option<String>,
    pub status: String,
    #[serde(default)]
    pub attempt: i32,
    #[serde(default)]
    pub lease_owner: Option<String>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub lease_expires_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub fence: i64,
    #[serde(default)]
    pub error_code: Option<String>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct AssistantAgentLearningProposal {
    #[serde(rename = "_id")]
    pub id: String,
    pub agent_id: String,
    pub owner_id: String,
    pub run_id: String,
    pub status: String,
    #[serde(default)]
    pub revision: i64,
    pub config_revision: i64,
    pub agent_skills_revision: i64,
    pub fingerprint: String,
    #[serde(default)]
    pub input_digest: String,
    #[serde(default)]
    pub model_contract: String,
    #[serde(default)]
    pub publication: Option<LearningPublication>,
    #[serde(default)]
    pub failure_code: Option<String>,
    #[serde(default)]
    pub evidence: Vec<LearningEvidence>,
    #[serde(default, with = "crate::models::bson_bytes::required")]
    pub body_encrypted: Vec<u8>,
    pub body_bytes: i64,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

impl std::fmt::Debug for AssistantAgentLearningProposal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AssistantAgentLearningProposal")
            .field("id", &self.id)
            .field("agent_id", &self.agent_id)
            .field("status", &self.status)
            .field("body_bytes", &self.body_bytes)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AssistantAgentLearningRejection {
    #[serde(rename = "_id")]
    pub id: String,
    pub agent_id: String,
    pub fingerprint: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AssistantAgentLearningSkillRoot {
    #[serde(rename = "_id")]
    pub id: String,
    pub agent_id: String,
    pub owner_id: String,
    pub skill_id: String,
    pub version: String,
    pub sha256: String,
    pub operation_id: String,
    #[serde(default)]
    pub active: bool,
    pub proposal_id: String,
    pub config_revision: i64,
    pub agent_skills_revision: i64,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
}

/// Metadata only; the reviewed archive is deterministically regenerated from
/// the encrypted draft. A started write is never issued a second time.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct LearningPublication {
    pub operation_id: String,
    pub name: String,
    pub version: String,
    pub sha256: String,
    pub skill_id: Option<String>,
    pub started: bool,
    pub approved_by: Option<String>,
    pub approval_digest: Option<String>,
    pub acknowledgement_id: Option<String>,
    pub skills_revision: i64,
    pub lease_id: Option<String>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub lease_expires_at: Option<DateTime<Utc>>,
}

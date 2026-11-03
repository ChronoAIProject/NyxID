use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::bson_datetime;

pub const COLLECTION_NAME: &str = "nyxbot_cma_agents";

/// Last known remote lifecycle state for a CMA Agent. Missing/failed reads are
/// represented explicitly as `Unknown`; they never authorize a new resource or
/// release a Sandbox slot.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentObservationState {
    #[default]
    Unknown,
    Provisioning,
    Ready,
    Running,
    Paused,
    Stopped,
    Deleted,
    Failed,
}

impl AgentObservationState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Unknown => "unknown",
            Self::Provisioning => "provisioning",
            Self::Ready => "ready",
            Self::Running => "running",
            Self::Paused => "paused",
            Self::Stopped => "stopped",
            Self::Deleted => "deleted",
            Self::Failed => "failed",
        }
    }
}

/// Durable owner/service/remote-Agent identity used by NyxBot discovery and
/// the read-only execution view. The row contains only safe profile facts and
/// bounded source metadata; task prompts and credentials remain elsewhere.
#[derive(Clone, Serialize, Deserialize)]
pub struct NyxbotCmaAgent {
    #[serde(rename = "_id")]
    pub id: String,
    pub user_id: String,
    pub cma_service_id: String,
    pub cma_agent_id: String,
    #[serde(default)]
    pub cma_sandbox_id: Option<String>,
    #[serde(default)]
    pub cma_workspace_id: Option<String>,
    #[serde(default)]
    pub cma_worktree_id: Option<String>,
    #[serde(default)]
    pub sandbox_generation: Option<i64>,
    #[serde(default)]
    pub label: String,
    /// Safe CMA profile metadata (model/permission mode/services only).
    #[serde(default)]
    pub profile_snapshot: Option<serde_json::Value>,
    #[serde(default)]
    pub observation_state: AgentObservationState,
    #[serde(default)]
    pub resource_state: Option<String>,
    #[serde(default)]
    pub phase: Option<String>,
    #[serde(default, with = "bson_datetime::optional")]
    pub last_observed: Option<DateTime<Utc>>,
    #[serde(default)]
    pub last_observed_revision: Option<i64>,
    #[serde(default)]
    pub source_conversation_id: Option<String>,
    #[serde(default)]
    pub source_role: Option<String>,
    #[serde(default)]
    pub source_agent_id: Option<String>,
    #[serde(default)]
    pub source_turn_id: Option<String>,
    #[serde(default)]
    pub source_watch_id: Option<String>,
    #[serde(default)]
    pub source_response_id: Option<String>,
    #[serde(default)]
    pub callback_url: Option<String>,
    #[serde(default)]
    pub creation_operation_id: Option<String>,
    #[serde(default)]
    pub deletion_operation_id: Option<String>,
    #[serde(default, with = "bson_datetime::optional")]
    pub retired_at: Option<DateTime<Utc>>,
    #[serde(default, with = "bson_datetime::optional")]
    pub deleted_at: Option<DateTime<Utc>>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

impl std::fmt::Debug for NyxbotCmaAgent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NyxbotCmaAgent")
            .field("id", &self.id)
            .field("user_id", &self.user_id)
            .field("cma_service_id", &self.cma_service_id)
            .field("cma_agent_id", &self.cma_agent_id)
            .field("cma_sandbox_id", &self.cma_sandbox_id)
            .field("cma_workspace_id", &self.cma_workspace_id)
            .field("observation_state", &self.observation_state)
            .field("resource_state", &self.resource_state)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use mongodb::bson::{self, Bson};
    use serde_json::json;

    #[test]
    fn bson_serialization_keeps_dates_and_safe_profile() {
        let now = Utc::now();
        let row = NyxbotCmaAgent {
            id: "agent-row".into(),
            user_id: "user".into(),
            cma_service_id: "service".into(),
            cma_agent_id: "remote-agent".into(),
            cma_sandbox_id: Some("remote-sandbox".into()),
            cma_workspace_id: Some("remote-workspace".into()),
            cma_worktree_id: Some("remote-worktree".into()),
            sandbox_generation: Some(3),
            label: "Coding agent".into(),
            profile_snapshot: Some(json!({"model": "gpt", "permission_mode": "autonomous"})),
            observation_state: AgentObservationState::Ready,
            resource_state: Some("ready".into()),
            phase: None,
            last_observed: Some(now),
            last_observed_revision: Some(7),
            source_conversation_id: Some("conversation".into()),
            source_role: Some("nyxbot".into()),
            source_agent_id: Some("nyx-agent".into()),
            source_turn_id: Some("turn".into()),
            source_watch_id: Some("watch".into()),
            source_response_id: Some("response".into()),
            callback_url: Some("https://cma.example/agents/remote-agent".into()),
            creation_operation_id: Some("create-op".into()),
            deletion_operation_id: None,
            retired_at: Some(now),
            deleted_at: Some(now),
            created_at: now,
            updated_at: now,
        };

        let document = bson::to_document(&row).expect("serialize agent");
        for field in [
            "created_at",
            "updated_at",
            "last_observed",
            "retired_at",
            "deleted_at",
        ] {
            assert!(
                matches!(document.get(field), Some(Bson::DateTime(_))),
                "{field} must be a BSON datetime"
            );
        }
        assert!(matches!(
            document.get("created_at"),
            Some(Bson::DateTime(_))
        ));
        assert!(matches!(
            document.get("last_observed"),
            Some(Bson::DateTime(_))
        ));
        assert!(matches!(
            document.get("profile_snapshot"),
            Some(Bson::Document(_))
        ));
        let restored: NyxbotCmaAgent = bson::from_document(document).expect("deserialize agent");
        assert_eq!(restored.cma_agent_id, "remote-agent");
        assert_eq!(restored.observation_state, AgentObservationState::Ready);
    }
}

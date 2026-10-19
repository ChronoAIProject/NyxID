use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "assistant_profile_routes";
/// Single platform-wide document.
pub const DOCUMENT_ID: &str = "default";

/// Admin-authored NyxAgent profile per agent role. Stored and validated now;
/// turns ignore it until `assistant_profile_routing::ROUTING_ACTIVE` is enabled.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AssistantProfileRoutes {
    #[serde(rename = "_id")]
    pub id: String,
    /// NyxAgent profile (`nyxagent/<name>`) for orchestrators; `None` keeps the default.
    #[serde(default)]
    pub orchestrator: Option<String>,
    #[serde(default)]
    pub subagent: Option<String>,
    #[serde(default)]
    pub channel: Option<String>,
    /// Subagent charter role (`research`, `writer`, ...) to profile overrides.
    #[serde(default)]
    pub subagent_roles: std::collections::BTreeMap<String, String>,
    pub updated_by: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

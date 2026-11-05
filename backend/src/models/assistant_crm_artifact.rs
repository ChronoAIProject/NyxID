use serde::{Deserialize, Serialize};

pub const MAX_TURN_CRM_ARTIFACTS: usize = 8;

/// Live resource references only. CRM values and view envelopes never enter chat history.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TurnCrmArtifact {
    pub kind: CrmArtifactKind,
    pub id: String,
    pub owner_id: String,
    pub object_key: Option<String>,
    pub title: String,
    pub role: CrmArtifactRole,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CrmArtifactKind {
    View,
    Record,
    Object,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CrmArtifactRole {
    Created,
    Presented,
}

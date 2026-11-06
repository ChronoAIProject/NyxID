use serde::{Deserialize, Serialize};

pub const MAX_TURN_CRM_ARTIFACTS: usize = 8;

/// Live resource references only. CRM values and view envelopes never enter chat history.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum TurnCrmArtifact {
    View {
        id: String,
        owner_id: String,
        title: String,
        role: CrmArtifactRole,
    },
    Record {
        id: String,
        owner_id: String,
        object_key: String,
        title: String,
        role: CrmArtifactRole,
    },
    Object {
        id: String,
        owner_id: String,
        object_key: String,
        title: String,
        role: CrmArtifactRole,
    },
}

impl TurnCrmArtifact {
    pub fn kind(&self) -> CrmArtifactKind {
        match self {
            Self::View { .. } => CrmArtifactKind::View,
            Self::Record { .. } => CrmArtifactKind::Record,
            Self::Object { .. } => CrmArtifactKind::Object,
        }
    }
    pub fn id(&self) -> &str {
        match self {
            Self::View { id, .. } | Self::Record { id, .. } | Self::Object { id, .. } => id,
        }
    }
    pub fn owner_id(&self) -> &str {
        match self {
            Self::View { owner_id, .. }
            | Self::Record { owner_id, .. }
            | Self::Object { owner_id, .. } => owner_id,
        }
    }
    pub fn title(&self) -> &str {
        match self {
            Self::View { title, .. } | Self::Record { title, .. } | Self::Object { title, .. } => {
                title
            }
        }
    }
    pub fn title_mut(&mut self) -> &mut String {
        match self {
            Self::View { title, .. } | Self::Record { title, .. } | Self::Object { title, .. } => {
                title
            }
        }
    }
    pub fn role(&self) -> CrmArtifactRole {
        match self {
            Self::View { role, .. } | Self::Record { role, .. } | Self::Object { role, .. } => {
                *role
            }
        }
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn crm_artifact_requires_object_keys_and_retains_wire_shape() {
        for kind in ["record", "object"] {
            let mut value =
                json!({"kind":kind,"id":"id","owner_id":"owner","title":"Title","role":"created"});
            assert!(serde_json::from_value::<TurnCrmArtifact>(value.clone()).is_err());
            value["object_key"] = json!("asset");
            let artifact: TurnCrmArtifact = serde_json::from_value(value.clone()).unwrap();
            assert_eq!(serde_json::to_value(artifact).unwrap(), value);
        }
        assert!(serde_json::from_value::<TurnCrmArtifact>(json!({"kind":"view","id":"id","owner_id":"owner","title":"View","role":"presented"})).is_ok());
    }
}

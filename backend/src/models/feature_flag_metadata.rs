use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "feature_flag_metadata";

/// Admin-authored documentation for a code-declared feature flag.
///
/// The flag registry (`feature_flag_service::FEATURE_FLAGS`) stays the source
/// of truth for which flags exist and what they default to; this collection
/// only carries the editorial fields staff need to keep a growing flag list
/// legible — what the flag is actually about, and who owns it. One row per
/// `flag_key`, created lazily on first edit and deleted again when an admin
/// clears every field.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeatureFlagMetadata {
    #[serde(rename = "_id")]
    pub id: String,
    /// Registry key this documents. Unknown keys are rejected at the service
    /// layer, not stored.
    pub flag_key: String,
    /// Admin-written description replacing the code-declared one in admin
    /// surfaces. `None` falls back to `FeatureFlagDef::description`.
    #[serde(default)]
    pub description: Option<String>,
    /// Free-form owner: a person, a team, or a contact address. `None` when
    /// nobody has claimed the flag.
    #[serde(default)]
    pub owner: Option<String>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
    /// User id of the platform admin who last wrote this row.
    pub updated_by: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collection_name() {
        assert_eq!(COLLECTION_NAME, "feature_flag_metadata");
    }

    #[test]
    fn bson_roundtrip() {
        let row = FeatureFlagMetadata {
            id: uuid::Uuid::new_v4().to_string(),
            flag_key: "example_ui".to_string(),
            description: Some("Gates the new panel.".to_string()),
            owner: Some("Platform team".to_string()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            updated_by: uuid::Uuid::new_v4().to_string(),
        };
        let doc = bson::to_document(&row).expect("serialize");
        let restored: FeatureFlagMetadata = bson::from_document(doc).expect("deserialize");
        assert_eq!(restored.flag_key, "example_ui");
        assert_eq!(
            restored.description.as_deref(),
            Some("Gates the new panel.")
        );
        assert_eq!(restored.owner.as_deref(), Some("Platform team"));
    }

    #[test]
    fn missing_optional_fields_default_to_none() {
        let mut doc = bson::to_document(&FeatureFlagMetadata {
            id: "x".to_string(),
            flag_key: "example_ui".to_string(),
            description: Some("d".to_string()),
            owner: Some("o".to_string()),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            updated_by: "actor".to_string(),
        })
        .expect("serialize");
        doc.remove("description");
        doc.remove("owner");
        let restored: FeatureFlagMetadata = bson::from_document(doc).expect("deserialize");
        assert!(restored.description.is_none());
        assert!(restored.owner.is_none());
    }
}

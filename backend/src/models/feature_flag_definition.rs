use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "feature_flag_definitions";

/// Runtime-created flags. Creator identity is immutable and independent of
/// editable documentation; code-declared flags always take precedence.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FeatureFlagDefinition {
    #[serde(rename = "_id")]
    pub id: String,
    pub flag_key: String,
    pub description: String,
    pub default_enabled: bool,
    pub created_by: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
}

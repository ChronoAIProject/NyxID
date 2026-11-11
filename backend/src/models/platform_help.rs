use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const LIBRARIES: &str = "platform_help_libraries";
pub const REVISIONS: &str = "platform_help_revisions";
pub const ASSIGNMENTS: &str = "platform_help_assignments";
pub const DEFAULT_LIBRARY: &str = "49c91235-9f03-4d3f-b962-9748fd78f3c0";

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HelpArticle {
    pub id: String,
    pub title: String,
    pub body: String,
    #[serde(default)]
    pub tags: Vec<String>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HelpUseCase {
    pub id: String,
    pub title: String,
    pub description: String,
    pub example_prompts: Vec<String>,
}
#[derive(Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HelpContent {
    pub articles: Vec<HelpArticle>,
    pub use_cases: Vec<HelpUseCase>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct HelpReference {
    pub library_id: String,
    pub revision: i64,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct HelpLibrary {
    #[serde(rename = "_id")]
    pub id: String,
    pub name: String,
    pub version: i64,
    pub published_revision: i64,
    pub draft: HelpContent,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct HelpRevision {
    #[serde(rename = "_id")]
    pub id: String,
    pub library_id: String,
    pub revision: i64,
    pub content: HelpContent,
    pub published_by: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub published_at: DateTime<Utc>,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct HelpAssignment {
    /// A profile UUID. Missing assignments use the default shared library.
    #[serde(rename = "_id")]
    pub id: String,
    pub library_id: String,
    pub version: i64,
}

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::catalog_skill_revision::SkillReference;

pub const COLLECTION_NAME: &str = "assistant_system_skills";
pub const DOCUMENT_ID: &str = "platform";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AssistantSystemSkills {
    #[serde(rename = "_id")]
    pub id: String,
    pub revision: i64,
    pub skills: Vec<SystemSkill>,
    pub updated_by: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SystemSkill {
    pub reference: SkillReference,
    pub description: String,
    pub size_bytes: usize,
}

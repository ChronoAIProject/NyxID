use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "catalog_skill_revisions";
pub const OPERATIONS: &str = "catalog_skill_operations";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SkillPin {
    pub source: String,
    pub skill_id: String,
    pub name: String,
    pub version: String,
    pub sha256: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SkillReference {
    pub source: String,
    pub skill_id: String,
    pub name: String,
    pub version: String,
    pub sha256: String,
    #[serde(default)]
    pub dependencies: Vec<SkillPin>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, utoipa::ToSchema)]
pub struct SkillState {
    pub recommended_skills: Option<Vec<String>>,
    pub recommended_skill_refs: Option<Vec<SkillReference>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CatalogSkillRevision {
    #[serde(rename = "_id")]
    pub id: String,
    pub service_id: String,
    pub previous_revision: i64,
    pub revision: i64,
    pub previous: SkillState,
    pub current: SkillState,
    pub actor_kind: String,
    pub actor_id: String,
    pub grant_id: Option<String>,
    pub request_id: String,
    pub fingerprint: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CatalogSkillOperation {
    #[serde(rename = "_id")]
    pub id: String,
    pub actor_kind: String,
    pub actor_id: String,
    pub request_id: String,
    pub service_id: String,
    pub fingerprint: String,
    pub revision: i64,
    pub state: SkillState,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
}

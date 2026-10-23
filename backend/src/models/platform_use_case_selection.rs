use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "platform_use_case_selections";

/// Informational sender preference. It carries no account or execution authority.
#[derive(Clone, Serialize, Deserialize)]
pub struct UseCaseSelection {
    #[serde(rename = "_id")]
    pub id: String,
    pub sender_key: String,
    pub profile_id: String,
    pub manager_authority_epoch: i64,
    pub help: super::platform_help::HelpReference,
    pub use_case_id: String,
    pub selection_ref: String,
    pub binding_id: Option<String>,
    pub binding_generation: Option<i64>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub purge_after: DateTime<Utc>,
}

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "machine_setups";

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Choices {
    #[serde(default)]
    pub owner_id: Option<String>,
    pub name: String,
    #[serde(rename = "where")]
    pub location: String,
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub grant_to: Option<String>,
}

/// Public intent and hashed possession proofs. Never stores a setup credential.
#[derive(Clone, Deserialize, Serialize)]
pub struct MachineSetup {
    #[serde(rename = "_id")]
    pub id: String,
    pub user_id: String,
    pub choices: Choices,
    pub status: String,
    #[serde(default)]
    pub code_hmac: Option<String>,
    #[serde(default)]
    pub device_hmac: Option<String>,
    #[serde(default)]
    pub hostname: Option<String>,
    #[serde(default)]
    pub os: Option<String>,
    #[serde(default)]
    pub ip: Option<String>,
    #[serde(default)]
    pub conversation_id: Option<String>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub last_poll_at: Option<DateTime<Utc>>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub purge_at: DateTime<Utc>,
}
impl std::fmt::Debug for MachineSetup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MachineSetup { [REDACTED] }")
    }
}

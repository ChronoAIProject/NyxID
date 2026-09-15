use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "oracle_login_profiles";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OracleLoginBinding {
    pub worker_label: String,
    pub instance_id: String,
    pub binding_id: String,
    pub replace_existing: bool,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct OracleLoginProfile {
    #[serde(rename = "_id")]
    pub id: String,
    pub pool_id: String,
    pub name: String,
    pub generation: String,
    pub revision: String,
    pub worker_token_hash: String,
    pub format_version: u32,
    #[serde(with = "super::bson_bytes::required")]
    pub encrypted_envelope: Vec<u8>,
    pub envelope_size: u64,
    pub bindings: Vec<OracleLoginBinding>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
}

impl std::fmt::Debug for OracleLoginProfile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OracleLoginProfile")
            .field("id", &self.id)
            .field("pool_id", &self.pool_id)
            .field("name", &self.name)
            .field("generation", &self.generation)
            .field("revision", &self.revision)
            .field("worker_token_hash", &"[REDACTED]")
            .field("encrypted_envelope", &"[REDACTED]")
            .finish()
    }
}

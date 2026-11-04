//! Private control rows. Never serialize these models to an API or audit log.
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "assistant_model_control_roots";

#[derive(Clone, Serialize, Deserialize)]
pub struct ModelControlRoot {
    #[serde(rename = "_id")]
    pub id: String,
    pub user_id: String,
    pub conversation_id: String,
    pub api_key_id: String,
    pub turn_ids: Vec<String>,
    pub runtime_catalog_id: String,
    pub phase: String,
    pub acknowledgement_id: Option<String>,
    /// Saved server-created authority arguments (IDs/digests only). Decision
    /// transactions recompute this digest without decrypting transcript data.
    pub card_arguments: Option<serde_json::Value>,
    pub revision: i64,
    pub control_hashes: Vec<String>,
    #[serde(with = "super::bson_bytes::required")]
    pub private_ciphertext: Vec<u8>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
}
impl std::fmt::Debug for ModelControlRoot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ModelControlRoot { [REDACTED] }")
    }
}

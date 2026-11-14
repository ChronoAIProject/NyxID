//! Metadata-only delivery intent; never stores a credential or a frame payload.
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "node_credential_desired";

#[derive(Clone, Serialize, Deserialize)]
pub struct NodeCredentialDesired {
    #[serde(rename = "_id")]
    pub id: String,
    pub node_id: String,
    pub service_slug: String,
    pub service_id: String,
    pub owner_id: String,
    #[serde(default)]
    pub actor_user_id: Option<String>,
    pub generation: i64,
    #[serde(default)]
    pub acknowledged_generation: i64,
    /// Queue success for agents without receipts; never an acknowledgement.
    #[serde(default)]
    pub sent_unconfirmed_generation: i64,
    #[serde(default)]
    pub parked_reason: Option<String>,
    #[serde(default)]
    pub retry_attempts: i64,
    /// Reconnect scheduling fence, independent of material generation.
    #[serde(default)]
    pub retry_revision: i64,
    pub pending: bool,
    #[serde(default)]
    pub suppressed: bool,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub retry_at: DateTime<Utc>,
    #[serde(default)]
    pub claim_token: Option<String>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub claim_until: Option<DateTime<Utc>>,
    #[serde(default)]
    pub preparing_token: Option<String>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub preparing_until: Option<DateTime<Utc>>,
}

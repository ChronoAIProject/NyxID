use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize)]
pub struct MobileSocialFlow {
    #[serde(rename = "_id")]
    pub id: String,
    pub state_hash: String,
    pub provider: String,
    pub callback_origin: String,
    pub redirect_uri: String,
    #[serde(default)]
    pub nonce_hash: Option<String>,
    #[serde(default)]
    pub code_challenge: Option<String>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
}

#[derive(Serialize, Deserialize)]
pub struct MobileSocialDelivery {
    #[serde(rename = "_id")]
    pub id: String,
    pub user_id: String,
    pub code_hash: String,
    pub code_challenge: String,
    pub ciphertext: bson::Binary,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
}

impl MobileSocialFlow {
    pub const COLLECTION_NAME: &'static str = "mobile_social_flows";
}

impl MobileSocialDelivery {
    pub const COLLECTION_NAME: &'static str = "mobile_social_deliveries";
}

crate::errors::redacted_debug!(MobileSocialFlow, MobileSocialDelivery);

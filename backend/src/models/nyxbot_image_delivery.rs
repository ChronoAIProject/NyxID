use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "nyxbot_image_deliveries";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ImageDeliveryItem {
    pub id: String,
    pub state: String,
    #[serde(default)]
    pub code: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NyxbotImageDelivery {
    #[serde(rename = "_id")]
    pub id: String,
    pub user_id: String,
    pub conversation_id: String,
    pub turn_id: String,
    pub channel_id: String,
    pub destination_key: String,
    pub state: String,
    pub claim_id: String,
    pub items: Vec<ImageDeliveryItem>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
}

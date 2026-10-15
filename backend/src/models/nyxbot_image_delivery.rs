use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "nyxbot_image_deliveries_v2";
pub const LEGACY_COLLECTION_NAME: &str = "nyxbot_image_deliveries";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ImageDeliveryState {
    Pending,
    Sending,
    Sent,
    Failed,
    Skipped,
    Unknown,
}

impl ImageDeliveryState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Sending => "sending",
            Self::Sent => "sent",
            Self::Failed => "failed",
            Self::Skipped => "skipped",
            Self::Unknown => "unknown",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NyxbotImageDelivery {
    #[serde(rename = "_id")]
    pub id: String,
    pub user_id: String,
    pub conversation_id: String,
    pub attachment_id: String,
    pub audience_key: String,
    pub state: ImageDeliveryState,
    pub attempt: Option<String>,
    pub code: Option<String>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
}

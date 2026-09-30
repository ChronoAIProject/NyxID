use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "channel_connect_links";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkStatus {
    Pending,
    Completed,
    Cancelled,
    Expired,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Receiver {
    App {
        app_id: String,
    },
    Direct {
        url: String,
        secret_encrypted: Vec<u8>,
        key_id: String,
    },
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ChannelConnectLink {
    #[serde(rename = "_id")]
    pub id: String,
    pub user_id: String,
    pub created_by_user_id: String,
    pub platform: String,
    pub label: String,
    pub requested_by: Option<String>,
    pub token_hash: String,
    pub callback_url: Option<String>,
    pub receiver: Option<Receiver>,
    pub status: LinkStatus,
    pub flow: Option<String>,
    pub bot_id: Option<String>,
    pub connection_id: Option<String>,
    pub telegram_request_id: Option<String>,
    pub claim_id: Option<String>,
    #[serde(default, with = "super::bson_datetime::optional")]
    pub claim_until: Option<DateTime<Utc>>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
    #[serde(default, with = "super::bson_datetime::optional")]
    pub terminal_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
    pub event_id: Option<String>,
    pub event_data: Option<bson::Document>,
    pub delivery_status: Option<String>,
    pub delivery_attempts: u32,
    #[serde(default, with = "super::bson_datetime::optional")]
    pub delivery_after: Option<DateTime<Utc>>,
}

impl std::fmt::Debug for ChannelConnectLink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChannelConnectLink")
            .field("id", &self.id)
            .field("status", &self.status)
            .finish_non_exhaustive()
    }
}

impl std::fmt::Debug for Receiver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Receiver([REDACTED])")
    }
}

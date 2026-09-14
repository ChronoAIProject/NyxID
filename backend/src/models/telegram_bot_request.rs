use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "telegram_bot_requests";
pub const MANAGED_BOTS: &str = "telegram_managed_bot_events";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ManagedBotEvents {
    #[serde(rename = "_id")]
    pub id: String,
    pub manager_bot_id: i64,
    pub telegram_bot_id: i64,
    pub telegram_user_id: i64,
    pub revision: i64,
    pub update_ids: Vec<i64>,
    #[serde(default)]
    pub created_by: Option<i64>,
    #[serde(default)]
    pub bot_username: Option<String>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub created_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub observation_id: Option<String>,
    #[serde(default)]
    pub retired: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TelegramRequestStatus {
    WaitingTelegram,
    WaitingBot,
    WaitingConsent,
    Ready,
    Provisioning,
    Connected,
    Cancelled,
    Expired,
    Suspended,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct TelegramBotRequest {
    #[serde(rename = "_id")]
    pub id: String,
    pub actor_user_id: String,
    pub owner_user_id: String,
    pub manager_bot_id: i64,
    pub observation_id: String,
    pub label: String,
    pub destination: String,
    pub status: TelegramRequestStatus,
    pub active: bool,
    pub revision: i64,
    pub challenge_hash: String,
    pub telegram_user_id: Option<i64>,
    pub telegram_bot_id: Option<i64>,
    pub bot_username: Option<String>,
    pub consent_hash: Option<String>,
    pub manager_revision: Option<i64>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub purge_after: Option<DateTime<Utc>>,
}

impl std::fmt::Debug for TelegramBotRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TelegramBotRequest")
            .field("id", &self.id)
            .field("status", &self.status)
            .finish_non_exhaustive()
    }
}

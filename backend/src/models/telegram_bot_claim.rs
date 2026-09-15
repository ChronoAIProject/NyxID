use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "telegram_bot_claims";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TelegramClaimStatus {
    Pending,
    Redeemed,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct TelegramBotClaim {
    #[serde(rename = "_id")]
    pub id: String,
    pub code_hash: String,
    pub manager_bot_id: i64,
    pub telegram_bot_id: i64,
    pub telegram_user_id: i64,
    pub bot_username: String,
    pub observation_id: String,
    pub manager_revision: i64,
    pub status: TelegramClaimStatus,
    pub request_id: Option<String>,
    pub actor_user_id: Option<String>,
    pub owner_user_id: Option<String>,
    pub label: Option<String>,
    pub delivered: bool,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub purge_after: Option<DateTime<Utc>>,
}

impl std::fmt::Debug for TelegramBotClaim {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TelegramBotClaim")
            .field("id", &self.id)
            .field("status", &self.status)
            .finish_non_exhaustive()
    }
}

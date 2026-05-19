use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "invite_codes";

/// Per-usage record of who redeemed an invite code and when.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InviteCodeUsage {
    pub user_id: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub used_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct InviteCode {
    #[serde(rename = "_id")]
    pub id: String,
    pub code: String,
    pub max_uses: i32,
    pub used_count: i32,
    pub created_by: String,
    pub note: Option<String>,
    pub is_active: bool,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
    /// List of users who redeemed this invite code, with timestamps.
    /// Defaults to empty for back-compat with codes created before the
    /// usage-tracking change landed.
    #[serde(default)]
    pub usages: Vec<InviteCodeUsage>,
}

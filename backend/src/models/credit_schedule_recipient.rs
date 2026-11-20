use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "credit_schedule_recipients";
pub const SCAN_COLLECTION_NAME: &str = "credit_schedule_recipient_scans";

/// A recipient's durable cycle identity. Retain the anchor when eligibility is
/// removed or restored: rejoining must never create a second first-period grant.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CreditScheduleRecipient {
    #[serde(rename = "_id")]
    pub id: String,
    pub schedule_id: String,
    pub recipient_user_id: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub anchor_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub next_due_at: DateTime<Utc>,
    pub is_eligible: bool,
    #[serde(default)]
    pub eligibility_version: i64,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

/// Discovery progress only. Resetting a scan cannot reset recipient anchors or
/// the deterministic grant identities.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CreditScheduleRecipientScan {
    #[serde(rename = "_id")]
    pub id: String,
    pub cursor_user_id: Option<String>,
}

//! Email channel coordination; no email content, recipient list, or credentials.
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const SUBSCRIPTIONS: &str = EmailSubscription::COLLECTION_NAME;
pub const SENDS: &str = EmailSend::COLLECTION_NAME;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EmailSubscription {
    /// Same UUID-v4 as the owning bot; one binding per bot.
    #[serde(rename = "_id")]
    pub id: String,
    pub user_id: String,
    pub account_id: String,
    pub notification_url: String,
    pub subscription_id: Option<String>,
    #[serde(default)]
    pub verified_secret_fingerprint: Option<String>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
}

impl EmailSubscription {
    pub const COLLECTION_NAME: &'static str = "channel_email_subscriptions";
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EmailSend {
    #[serde(rename = "_id")]
    pub id: String,
    pub bot_id: String,
    pub user_id: String,
    pub account_id: String,
    pub platform_message_id: String,
    pub inbound_message_id: String,
    /// submitting is deliberately durable: a crash must never permit a resend.
    pub status: String,
    pub platform_reply_message_id: Option<String>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

impl EmailSend {
    pub const COLLECTION_NAME: &'static str = "channel_email_sends";
}

/// Progress through a producer-supplied batch; stores only its digest and offset.
/// Redelivery must supply the original body. This is not a work queue.
pub const BATCHES: &str = EmailBatch::COLLECTION_NAME;
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EmailBatch {
    #[serde(rename = "_id")]
    pub id: String,
    pub bot_id: String,
    pub user_id: String,
    pub digest: String,
    pub next_offset: i64,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
}

impl EmailBatch {
    pub const COLLECTION_NAME: &'static str = "channel_email_batches";
}

pub const RECEIPTS: &str = EmailReceipt::COLLECTION_NAME;
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EmailReceipt {
    /// Stable UUID-v4 also used for the inbound ChannelMessage.
    #[serde(rename = "_id")]
    pub id: String,
    pub bot_id: String,
    pub user_id: String,
    pub platform_message_id: String,
    pub completed: bool,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
}

impl EmailReceipt {
    pub const COLLECTION_NAME: &'static str = "channel_email_receipts";
}

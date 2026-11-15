use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::service_billing::{AudioTokens, ImageTokens, TokenBreakdown};

/// Bounded numeric prefix, never a final socket quantity while open.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct RealtimeUsagePrefix {
    pub bytes: i64,
    pub reported_tokens: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_write_tokens: i64,
    pub images: i64,
    pub token_breakdown: Option<TokenBreakdown>,
    pub audio_tokens: Option<AudioTokens>,
    pub image_tokens: Option<ImageTokens>,
    pub uncovered_bytes: i64,
    pub reported_responses: i64,
    pub estimated_responses: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RealtimeSession {
    pub version: i32,
    pub token: String,
    pub sequence: i64,
    pub closed: bool,
    pub recovered: bool,
    pub use_reported_usage: bool,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub close_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub recover_after: DateTime<Utc>,
    pub prefix: RealtimeUsagePrefix,
}

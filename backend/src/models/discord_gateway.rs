//! Additive metadata from authenticated Discord Gateway sessions. No content,
//! mentioned identities, bot tokens, or interaction credentials are retained.
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscordGatewayFacts {
    pub version: u8,
    pub bot_id: String,
    pub bot_mentioned: bool,
    pub mentions_others: bool,
    /// Discord supplies one boolean for @everyone OR @here, not separate facts.
    pub broadcast_everyone_or_here: bool,
    pub broadcast_roles: bool,
    pub reply_to_bot: bool,
    pub author_is_bot: bool,
    pub channel_type: u8,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DiscordGatewayStatus {
    pub state: String,
    pub reason: Option<String>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

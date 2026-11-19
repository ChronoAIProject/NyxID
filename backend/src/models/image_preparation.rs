//! Identifiers for recovering an attachment before its first possible provider effect.
use super::assistant_conversation::ChannelOrigin;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ImageAnchor {
    Inbound { message_id: String },
    Thread { origin: ChannelOrigin },
    Initiated { chat_id: String },
    NoAnchor,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Source {
    Event {
        event_id: String,
    },
    Supplemental {
        origin: ChannelOrigin,
    },
    Post {
        api_key_id: String,
        #[serde(default)]
        key_version: i64,
    },
    Automation {
        run_id: String,
    },
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ChannelRoute {
    pub conversation_id: String,
    pub turn_id: String,
    pub channel_id: String,
    pub thread_id: String,
    pub bot_id: String,
    pub transport: String,
    pub gateway_version: Option<i64>,
    pub binding_generation: i64,
    pub route_id: Option<String>,
    pub route_key_id: String,
    pub anchor: ImageAnchor,
    pub source: Source,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Route {
    Channel { route: Box<ChannelRoute> },
    Manager { delivery_id: String },
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Preparation {
    pub version: u32,
    pub phase: String,
    #[serde(default)]
    pub parked: bool,
    #[serde(default)]
    pub effects: u32,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub lease_until: DateTime<Utc>,
    pub route: Route,
}

impl std::fmt::Debug for Preparation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Preparation")
            .field("version", &self.version)
            .field("phase", &self.phase)
            .finish_non_exhaustive()
    }
}

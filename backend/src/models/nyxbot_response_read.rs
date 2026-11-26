//! A response identity observes local delivery; it grants no replay authority.
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Serialize, Deserialize)]
pub struct Command {
    pub response_id: String,
    pub metadata: BTreeMap<String, String>,
    pub request_hash: String,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ResponseRead {
    pub version: u8,
    pub binding_id: Option<String>,
    pub conversation_id: String,
    pub commands: Vec<Command>,
    #[serde(default)]
    pub thread_reply: Option<bool>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
}

#[derive(Serialize, Deserialize)]
pub struct ReplyPayload {
    pub version: u8,
    pub response_id: String,
    #[serde(with = "crate::models::bson_bytes::required")]
    pub ciphertext: Vec<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context_version: Option<u8>,
}

#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NoticeKind {
    GuestUnavailable,
    GuestBusy,
    FollowBusy,
    StartFailure,
    ServiceSelection,
}

/// An exact, server-classified no-turn output. The source, actor, child and
/// route remain bound by the event's prepared delivery.
#[derive(Clone, Serialize, Deserialize)]
pub struct OrgNotice {
    pub version: u8,
    pub kind: NoticeKind,
    pub response_id: String,
    pub text_sha256: String,
    pub target_sha256: String,
    pub binding_id: String,
    pub bot_id: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
    pub pending: bool,
}

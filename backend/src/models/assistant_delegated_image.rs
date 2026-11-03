//! Internal provenance only; never accepted from model/tool input or mixed into owned attachments.
use super::assistant_conversation::{ChannelOrigin, TurnAttachment};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DelegatedImages {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assignment: Option<Assignment>,
    /// A business refusal while publishing the result; the legacy settlement
    /// hook may attempt a text-only notice, but this result grants no images.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publication_refusal: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub references: Vec<DeliveryReference>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Assignment {
    pub id: String,
    pub actor: String,
    pub source_agent: String,
    pub source_conversation: String,
    pub source_turn: String,
    pub recipient_conversation: String,
    pub assigning_turn: String,
    #[serde(default, with = "super::bson_datetime::optional")]
    pub recipient_reset_at: Option<DateTime<Utc>>,
    pub destinations: Vec<ChannelOrigin>,
    pub trigger_chat: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DeliveryReference {
    pub assignment: Assignment,
    pub recipient_turn: String,
    pub attachment: TurnAttachment,
}

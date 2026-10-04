//! Token-free source facts only. A root may be unresolved; these facts do not
//! grant permission to follow, fetch history, or reply.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThreadKind {
    Native,
    Topic,
    ReplyChain,
    Email,
    #[default]
    #[serde(other)]
    Unknown,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThreadSenderKind {
    Human,
    Bot,
    #[default]
    #[serde(other)]
    Unknown,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThreadAddress {
    Mention,
    ReplyToBot,
    /// An Aurinko message explicitly addresses the connected mailbox in To.
    MailboxTo,
    /// An Aurinko message is a verified reply to a retained NyxID message.
    VerifiedReply,
    NotAddressed,
    #[default]
    #[serde(other)]
    Unknown,
}

#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ChannelThreadFacts {
    pub version: u8,
    pub kind: ThreadKind,
    pub chat_id: String,
    pub parent_chat_id: Option<String>,
    pub message_id: String,
    pub root_id: Option<String>,
    pub native_thread_id: Option<String>,
    pub parent_message_id: Option<String>,
    pub sender_kind: ThreadSenderKind,
    pub address: ThreadAddress,
    /// Keyed HMAC-SHA256 fingerprints of provider participants (email only),
    /// domain-separated as `email-participant`. These prove guest visibility
    /// without retaining recipient addresses or reversible unkeyed digests.
    #[serde(default)]
    pub participant_hashes: Vec<String>,
    /// Keyed HMAC-SHA256 fingerprint of the inbound sender, used as the
    /// requesting participant proof.
    #[serde(default)]
    pub sender_hash: Option<String>,
}

impl std::fmt::Debug for ChannelThreadFacts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChannelThreadFacts")
            .field("version", &self.version)
            .field("kind", &self.kind)
            .field("sender_kind", &self.sender_kind)
            .field("address", &self.address)
            .finish_non_exhaustive()
    }
}

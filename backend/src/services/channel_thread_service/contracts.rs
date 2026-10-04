//! Request-lifetime contracts. Deliberately no serde implementations: provider
//! history and bound authority must never enter callback metadata or a queue.
use chrono::{DateTime, Utc};

use crate::models::channel_thread::{ChannelThreadFacts, ThreadSenderKind};

pub const HISTORY_MESSAGES: usize = 20;
pub const HISTORY_BYTES: usize = 32 * 1024;
pub const HISTORY_MESSAGE_BYTES: usize = 4 * 1024;
pub const HISTORY_REQUESTS: usize = 3;
pub const HISTORY_SECONDS: u64 = 8;
pub const HISTORY_RESPONSE_BYTES: u64 = 2 * 1024 * 1024;
pub const ANCESTORS: usize = 16;

/// Created only by the service from a retained inbound row and adapter proof.
/// Native adapters can read the facts, but callers cannot deserialize a target.
#[derive(Clone)]
pub struct ThreadReplyTarget {
    pub(super) bot_id: String,
    pub(super) owner_id: String,
    pub(super) message_id: String,
    pub(super) route_key_id: String,
    pub(super) platform: String,
    pub(super) facts: ChannelThreadFacts,
    pub(super) admitted: Option<(
        String,
        crate::models::assistant_conversation::ChannelOrigin,
        String,
    )>,
}

impl ThreadReplyTarget {
    #[cfg(test)]
    pub(crate) fn fixture(platform: &str, facts: ChannelThreadFacts) -> Self {
        Self {
            bot_id: "bot".into(),
            owner_id: "owner".into(),
            message_id: "source".into(),
            route_key_id: "key".into(),
            platform: platform.into(),
            facts,
            admitted: None,
        }
    }
    pub fn facts(&self) -> &ChannelThreadFacts {
        &self.facts
    }

    pub(crate) fn matches(
        &self,
        bot: &crate::models::channel_bot::ChannelBot,
        source: &crate::models::channel_message::ChannelMessage,
        chat: &str,
    ) -> bool {
        self.bot_id == bot.id
            && self.owner_id == bot.user_id
            && self.platform == bot.platform
            && self.message_id == source.id
            && source.channel_bot_id.as_deref() == Some(&self.bot_id)
            && source.user_id == self.owner_id
            && source.platform == self.platform
            && source.agent_api_key_id.as_deref() == Some(&self.route_key_id)
            && source.direction == "inbound"
            && self.facts.chat_id == chat
            && source.thread_context.as_ref() == Some(&self.facts)
            && super::resolution::source_matches(&self.facts, source)
    }
}

impl std::fmt::Debug for ThreadReplyTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ThreadReplyTarget([REDACTED])")
    }
}

pub struct ThreadHistoryMessage {
    pub message_id: String,
    pub sender_id: String,
    pub sender_kind: ThreadSenderKind,
    pub created_at: DateTime<Utc>,
    pub text: String,
}

impl std::fmt::Debug for ThreadHistoryMessage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ThreadHistoryMessage([REDACTED])")
    }
}

#[derive(Debug, Default)]
pub struct ThreadHistory {
    pub messages: Vec<ThreadHistoryMessage>,
    /// True for bounded/unsupported/denied history; never claim a full recap.
    pub partial: bool,
}

/// Metadata fallback never reads legacy body fields or attachments.
pub struct ThreadHistoryMetadata {
    pub message_id: String,
    pub sender_id: String,
    pub created_at: DateTime<Utc>,
    pub content_type: String,
}

impl std::fmt::Debug for ThreadHistoryMetadata {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ThreadHistoryMetadata([REDACTED])")
    }
}

#[derive(Debug, Default)]
pub struct ThreadContext {
    pub history: ThreadHistory,
    pub metadata: Vec<ThreadHistoryMetadata>,
}

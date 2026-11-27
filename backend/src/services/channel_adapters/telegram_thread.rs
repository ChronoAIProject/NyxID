use super::*;
use crate::models::channel_thread::{ChannelThreadFacts, ThreadKind};
use crate::services::channel_adapters::thread_support as support;
use crate::services::channel_platform::BotCredentials;
use crate::services::channel_thread_service::{self as threads, ThreadReplyTarget};

impl TelegramAdapter {
    pub(super) fn resolve_native_thread(
        &self,
        facts: &ChannelThreadFacts,
        ancestors: &[ChannelThreadFacts],
    ) -> Option<ChannelThreadFacts> {
        if facts.chat_id.parse::<i64>().is_err() || !positive(&facts.message_id) {
            return None;
        }
        let mut resolved = facts.clone();
        match facts.kind {
            ThreadKind::Topic if facts.root_id == facts.native_thread_id => {}
            ThreadKind::ReplyChain if facts.native_thread_id.is_none() => {
                resolved.root_id = support::chain_root(facts, ancestors);
            }
            _ => return None,
        }
        resolved.root_id.as_deref().filter(|id| positive(id))?;
        Some(resolved)
    }

    pub(super) async fn reply_in_thread(
        &self,
        http: &reqwest::Client,
        credentials: &BotCredentials<'_>,
        target: &ThreadReplyTarget,
        reply: &OutboundReply,
    ) -> AppResult<Option<String>> {
        let facts = self
            .resolve_native_thread(target.facts(), &[])
            .ok_or_else(threads::unavailable)?;
        let mut reply = reply.clone();
        let entities = reply
            .metadata
            .as_ref()
            .filter(|metadata| metadata["text_format"] == "telegram_entities")
            .and_then(|metadata| metadata.get("entities"))
            .cloned();
        let formatted = entities.is_some();
        // Ignore conflicting caller routing metadata, including non-topic IDs.
        reply.metadata = crate::services::channel_image_audience::facts_thread("telegram", &facts)
            .map(|id| serde_json::json!({ "message_thread_id": id }));
        if let Some(entities) = entities {
            reply.metadata.get_or_insert_with(|| serde_json::json!({}))["entities"] = entities;
        }
        reply.reply_to_platform_message_id = Some(facts.message_id);
        let sent = self
            .send_reply(http, credentials, &facts.chat_id, &reply)
            .await;
        if formatted {
            sent
        } else {
            sent.map_err(|_| threads::unavailable())
        }
    }
}

fn positive(id: &str) -> bool {
    id.parse::<i64>().is_ok_and(|n| n > 0)
}

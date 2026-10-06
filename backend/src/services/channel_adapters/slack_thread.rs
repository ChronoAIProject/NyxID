use super::*;
use crate::models::channel_thread::{ChannelThreadFacts, ThreadKind, ThreadSenderKind};
use crate::services::channel_adapters::thread_support as support;
use crate::services::channel_platform::BotCredentials;
use crate::services::channel_thread_service::{self as threads, *};
use chrono::{DateTime, Utc};
use serde_json::Value;

impl SlackAdapter {
    pub(super) fn resolve_native_thread(
        &self,
        facts: &ChannelThreadFacts,
    ) -> Option<ChannelThreadFacts> {
        (facts.kind == ThreadKind::Native
            && support::path_id(&facts.chat_id)
            && facts
                .root_id
                .as_deref()
                .is_some_and(|id| support::slack_time(id).is_some())
            && support::slack_time(&facts.message_id).is_some())
        .then(|| facts.clone())
    }

    pub(super) async fn reply_in_thread(
        &self,
        http: &reqwest::Client,
        credentials: &BotCredentials<'_>,
        target: &ThreadReplyTarget,
        reply: &OutboundReply,
    ) -> AppResult<Option<String>> {
        let facts = self
            .resolve_native_thread(target.facts())
            .ok_or_else(threads::unavailable)?;
        let mut reply = reply.clone();
        let mut metadata = reply
            .metadata
            .take()
            .filter(Value::is_object)
            .unwrap_or_else(|| serde_json::json!({}));
        metadata["thread_ts"] = serde_json::json!(facts.root_id);
        reply.metadata = Some(metadata);
        reply.reply_to_platform_message_id = facts.root_id;
        // Both text and external file completion already preserve thread_ts.
        self.send_reply(http, credentials, &facts.chat_id, &reply)
            .await
            .map_err(|_| threads::unavailable())
    }

    pub(super) async fn history_in_thread(
        &self,
        http: &reqwest::Client,
        credentials: &BotCredentials<'_>,
        target: &ThreadReplyTarget,
        before: DateTime<Utc>,
    ) -> AppResult<ThreadHistory> {
        let facts = self
            .resolve_native_thread(target.facts())
            .ok_or_else(threads::unavailable)?;
        let root = facts.root_id.as_deref().ok_or_else(threads::unavailable)?;
        let before =
            before.min(support::slack_time(&facts.message_id).ok_or_else(threads::unavailable)?);
        let mut history = ThreadHistory::default();
        let mut cursor = String::new();
        let mut seen_cursors = std::collections::HashSet::new();
        for _ in 0..HISTORY_REQUESTS {
            let page = support::json(
                http.get(format!("{}/conversations.replies", self.base_url))
                    .bearer_auth(credentials.token)
                    .query(&[
                        ("channel", facts.chat_id.as_str()),
                        ("ts", root),
                        ("limit", "20"),
                        ("latest", facts.message_id.as_str()),
                        ("inclusive", "false"),
                        (
                            "oldest",
                            &(before - chrono::Duration::hours(24))
                                .timestamp()
                                .to_string(),
                        ),
                        ("cursor", cursor.as_str()),
                    ]),
            )
            .await;
            let Ok(page) = page else {
                history.partial = true;
                break;
            };
            if page["ok"] != true {
                history.partial = true;
                break;
            }
            let Some(messages) = page["messages"].as_array() else {
                history.partial = true;
                break;
            };
            for m in messages {
                let (Some(id), Some(text)) = (m["ts"].as_str(), m["text"].as_str()) else {
                    continue;
                };
                if id != root && m["thread_ts"].as_str() != Some(root) {
                    continue;
                }
                if m.get("subtype").is_some() && m["subtype"] != "bot_message" {
                    continue;
                }
                let (sender, kind) = if let Some(id) = m["bot_id"].as_str() {
                    (id, ThreadSenderKind::Bot)
                } else if let Some(id) = m["user"].as_str() {
                    // Slack can identify bot_message senders by user alone.
                    // Preserve that evidence for the service's own-bot filter.
                    let kind = if m["subtype"] == "bot_message" {
                        ThreadSenderKind::Bot
                    } else {
                        ThreadSenderKind::Human
                    };
                    (id, kind)
                } else {
                    continue;
                };
                let Some(time) = support::slack_time(id) else {
                    continue;
                };
                support::push(
                    &mut history,
                    ThreadHistoryMessage {
                        message_id: id.into(),
                        sender_id: sender.into(),
                        sender_kind: kind,
                        created_at: time,
                        text: text.into(),
                        participant_hashes: Vec::new(),
                    },
                    before,
                );
            }
            cursor = page["response_metadata"]["next_cursor"]
                .as_str()
                .unwrap_or_default()
                .into();
            history.partial |= page["has_more"] == true;
            if cursor.is_empty() {
                break;
            }
            if !seen_cursors.insert(cursor.clone())
                || cursor.len() > 2048
                || history.messages.len() >= HISTORY_MESSAGES
            {
                history.partial = true;
                break;
            }
        }
        Ok(history)
    }
}

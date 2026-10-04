use super::*;
use crate::models::channel_message::{COLLECTION_NAME as MESSAGES, ChannelMessage};
use crate::models::channel_thread::ThreadSenderKind;
use crate::services::channel_platform::BotCredentials;
use futures::TryStreamExt;

/// `eligible_human` is supplied by admitted chat policy; bot replies require
/// the exact bot identity independently. No fetched content is serializable.
pub async fn context(
    db: &mongodb::Database,
    adapter: &dyn PlatformAdapter,
    bot: &ChannelBot,
    credentials: &BotCredentials<'_>,
    target: &ThreadReplyTarget,
    eligible_human: &(dyn Fn(&str) -> bool + Send + Sync),
) -> AppResult<ThreadContext> {
    let mut fallback = Vec::new();
    let work = async {
        let source = db
            .collection::<ChannelMessage>(MESSAGES)
            .find_one(doc! {"_id": &target.message_id})
            .await?
            .ok_or_else(unavailable)?;
        if !target.matches(bot, &source, &target.facts.chat_id)
            || adapter.platform_id() != target.platform
            || !resolution::eligible_source(db, bot, &source).await?
        {
            return Err(unavailable());
        }
        let since = source.created_at - chrono::Duration::hours(24);
        // Fetch the fallback first so a slow provider cannot consume its budget.
        let rows: Vec<ChannelMessage> = db
            .collection(MESSAGES)
            .find(doc! {
                "channel_bot_id": &bot.id, "user_id": &bot.user_id, "platform": &bot.platform,
                "platform_conversation_id": &target.facts.chat_id,
                "thread_context.root_id": target.facts.root_id.as_deref(),
                "thread_context.kind": bson::to_bson(&target.facts.kind).map_err(|_| unavailable())?,
                "direction": "inbound", "created_at": {
                    "$gte": bson::DateTime::from_chrono(since),
                    "$lt": bson::DateTime::from_chrono(source.created_at),
                },
            })
            .projection(doc! {
                "_id": 1, "channel_bot_id": 1, "conversation_id": 1, "user_id": 1,
                "platform": 1, "platform_conversation_id": 1, "platform_message_id": 1,
                "direction": 1, "sender_platform_id": 1, "content_type": 1,
                "thread_context": 1, "created_at": 1,
            })
            .sort(doc! {"created_at": -1, "_id": -1})
            .limit(HISTORY_MESSAGES as i64)
            .await?
            .try_collect()
            .await?;
        let mut metadata_bytes = 0;
        let mut metadata: Vec<_> = rows
            .into_iter()
            .filter_map(|row| {
                let id = row.platform_message_id?;
                let sender = row.sender_platform_id?;
                let size = id.len() + sender.len() + row.content_type.len() + 64;
                if !valid_id(&id) || !valid_id(&sender) || size > HISTORY_BYTES - metadata_bytes {
                    return None;
                }
                metadata_bytes += size;
                (id != target.facts.message_id
                    && eligible_human(&sender)
                    && row.thread_context.as_ref().is_some_and(|f| {
                        f.version == 1 && f.sender_kind == ThreadSenderKind::Human
                    }))
                .then_some(ThreadHistoryMetadata {
                    message_id: id,
                    sender_id: sender,
                    created_at: row.created_at,
                    content_type: row.content_type,
                })
            })
            .collect();
        metadata.reverse();
        fallback = metadata;
        let mut history = ThreadHistory {
            messages: Vec::new(),
            partial: true,
        };
        if adapter.thread_capabilities().thread_history {
            let http = resolution::client()?;
            // Keep fallback after timeout/denial. No provider error prose escapes.
            if let Ok(Ok(fetched)) = tokio::time::timeout(
                std::time::Duration::from_secs(HISTORY_SECONDS),
                adapter.thread_history(&http, credentials, target, source.created_at),
            )
            .await
            {
                history = fetched;
            }
        }
        let mut seen = std::collections::HashSet::new();
        let mut bytes = 0;
        history
            .messages
            .sort_by(|a, b| (a.created_at, &a.message_id).cmp(&(b.created_at, &b.message_id)));
        history.messages.retain_mut(|m| {
            let overhead = m.message_id.len() + m.sender_id.len() + 64;
            let eligible = match m.sender_kind {
                ThreadSenderKind::Human => eligible_human(&m.sender_id),
                ThreadSenderKind::Bot => m.sender_id == bot.platform_bot_id,
                ThreadSenderKind::Unknown => false,
            };
            if !eligible
                || !valid_id(&m.message_id)
                || !valid_id(&m.sender_id)
                || m.message_id == target.facts.message_id
                || m.created_at < since
                || m.created_at >= source.created_at
                || !seen.insert(m.message_id.clone())
                || seen.len() > HISTORY_MESSAGES
                || overhead >= HISTORY_BYTES - bytes
            {
                history.partial = true;
                return false;
            }
            let mut end = m
                .text
                .len()
                .min(HISTORY_MESSAGE_BYTES)
                .min(HISTORY_BYTES - bytes - overhead);
            while !m.text.is_char_boundary(end) {
                end -= 1;
            }
            history.partial |= end < m.text.len();
            m.text.truncate(end);
            bytes += end + overhead;
            true
        });
        // The total context, not each source separately, is bounded to 20
        // messages / 32 KiB. Prefer bodies; metadata fills only missing entries.
        fallback.retain(|m| {
            let size = m.message_id.len() + m.sender_id.len() + m.content_type.len() + 64;
            if history
                .messages
                .iter()
                .any(|h| h.message_id == m.message_id)
                || size > HISTORY_BYTES - bytes
            {
                return false;
            }
            bytes += size;
            true
        });
        fallback.truncate(HISTORY_MESSAGES - history.messages.len());
        Ok(ThreadContext {
            history,
            metadata: std::mem::take(&mut fallback),
        })
    };
    // An outer bound includes MongoDB and client setup; provider gets only the
    // remaining time. A timed-out fetch retains the already loaded fallback.
    tokio::time::timeout(std::time::Duration::from_secs(HISTORY_SECONDS), work)
        .await
        .unwrap_or_else(|_| {
            Ok(ThreadContext {
                history: ThreadHistory {
                    messages: Vec::new(),
                    partial: true,
                },
                metadata: fallback,
            })
        })
}

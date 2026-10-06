//! Shared bounds only. Native paths, root semantics and schemas remain in adapters.
use crate::errors::AppResult;
use crate::models::channel_thread::ChannelThreadFacts;
use crate::services::channel_thread_service::{self as threads, *};
use chrono::{DateTime, Utc};
use serde_json::Value;

pub(super) fn path_id(id: &str) -> bool {
    !id.is_empty()
        && !matches!(id, "." | "..")
        && id.len() <= 256
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-' || b == b'.')
}

pub(super) fn number(id: &str) -> bool {
    !id.is_empty() && id.len() <= 20 && id.bytes().all(|b| b.is_ascii_digit())
}

pub(super) fn chain_root(
    facts: &ChannelThreadFacts,
    ancestors: &[ChannelThreadFacts],
) -> Option<String> {
    if let Some(root) = &facts.root_id {
        return Some(root.clone());
    }
    let mut next = facts.parent_message_id.as_deref();
    for parent in ancestors.iter().take(ANCESTORS) {
        if Some(parent.message_id.as_str()) != next
            || parent.chat_id != facts.chat_id
            || parent.kind != facts.kind
            || parent.version != 1
        {
            return None;
        }
        if let Some(root) = &parent.root_id {
            return Some(root.clone());
        }
        next = parent.parent_message_id.as_deref();
    }
    None
}

pub(super) async fn json(request: reqwest::RequestBuilder) -> AppResult<Value> {
    let response = request.send().await.map_err(|_| threads::unavailable())?;
    let bounded =
        crate::services::channel_media_service::bounded_response(response, HISTORY_RESPONSE_BYTES)
            .await
            .map_err(|_| threads::unavailable())?;
    serde_json::from_slice(&bounded.bytes).map_err(|_| threads::unavailable())
}

pub(super) fn timestamp(value: &Value) -> Option<DateTime<Utc>> {
    value
        .as_str()
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|t| t.with_timezone(&Utc))
}

pub(super) fn slack_time(value: &str) -> Option<DateTime<Utc>> {
    let (secs, micros) = value.split_once('.')?;
    if micros.len() != 6 {
        return None;
    }
    DateTime::from_timestamp(
        secs.parse().ok()?,
        micros.parse::<u32>().ok()?.checked_mul(1000)?,
    )
}

pub(crate) fn push(
    history: &mut ThreadHistory,
    mut message: ThreadHistoryMessage,
    before: DateTime<Utc>,
) {
    if message.created_at >= before
        || message.created_at < before - chrono::Duration::hours(24)
        || history
            .messages
            .iter()
            .any(|m| m.message_id == message.message_id)
    {
        return;
    }
    let used: usize = history.messages.iter().map(|m| m.text.len()).sum();
    if history.messages.len() >= HISTORY_MESSAGES || used >= HISTORY_BYTES {
        history.partial = true;
        return;
    }
    let mut end = message
        .text
        .len()
        .min(HISTORY_MESSAGE_BYTES)
        .min(HISTORY_BYTES - used);
    while !message.text.is_char_boundary(end) {
        end -= 1;
    }
    history.partial |= end < message.text.len();
    message.text.truncate(end);
    history.messages.push(message);
}

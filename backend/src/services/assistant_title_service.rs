//! Conversation titles are untrusted display metadata, never agent instructions.
use crate::{
    errors::AppResult,
    models::{
        assistant_conversation::{AssistantConversation, COLLECTION_NAME, TitleSource, TurnOrigin},
        assistant_message::{AssistantMessage, COLLECTION_NAME as MESSAGES},
    },
};
use futures::TryStreamExt;
use mongodb::{Database, bson::doc};

fn clean(text: &str) -> String {
    text.chars()
        .filter(|c| (!c.is_control() || c.is_whitespace()) && !matches!(c, '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2060}'..='\u{206f}' | '\u{feff}'))
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

fn bounded(text: &str) -> String {
    if text.chars().count() <= 60 {
        return text.to_owned();
    }
    let prefix: String = text.chars().take(60).collect();
    // Scripts without spaces, or one long word, still need a hard bound.
    match prefix.rfind(' ') {
        Some(end) => prefix[..end].to_owned(),
        None => prefix,
    }
}

pub fn provisional(text: &str) -> String {
    let first = text
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or_default();
    let title = bounded(&clean(first));
    if title.is_empty() {
        "New chat".into()
    } else {
        title
    }
}

pub fn generated(text: &str) -> Option<String> {
    let text = clean(text);
    let text = text
        .trim_matches(|c: char| matches!(c, '\'' | '"' | '`' | '*' | '#' | '“' | '”' | '‘' | '’'));
    let title = bounded(text)
        .trim_end_matches(|c: char| {
            c.is_ascii_punctuation() || matches!(c, '。' | '！' | '？' | '…')
        })
        .trim()
        .to_owned();
    (!title.is_empty()).then_some(title)
}

pub fn eligible(row: &AssistantConversation) -> bool {
    row.title_source == TitleSource::Provisional
        && !row.guest_turn
        && row.channel.is_none()
        && row.group_id.is_none()
        && !row.automation_thread
}

/// Only the first owner exchange, never guest/channel content or later history.
/// Reads through the normal owner + live organization authority check.
pub async fn first_exchange(
    db: &Database,
    actor: &str,
    id: &str,
) -> AppResult<Option<(AssistantConversation, String, String)>> {
    let row = super::assistant_nyxagent::get(db, actor, id).await?;
    if !eligible(&row) {
        return Ok(None);
    }
    let messages: Vec<_> = db
        .collection::<AssistantMessage>(MESSAGES)
        .find(doc! {"user_id": actor, "conversation_id": id})
        .sort(doc! {"seq": 1})
        .limit(2)
        .await?
        .try_collect()
        .await?;
    let [user, assistant] = messages.as_slice() else {
        return Ok(None);
    };
    if user.role != "user"
        || user.origin != Some(TurnOrigin::User)
        || assistant.role != "assistant"
        || assistant.status != "completed"
        || user.turn_id != assistant.turn_id
        || assistant.text.trim().is_empty()
    {
        return Ok(None);
    }
    // Attachment-only messages have no user text. The completed reply still
    // supplies a topic; do not fetch attachment contents for title generation.
    Ok(Some((
        row,
        user.text.chars().take(2000).collect(),
        assistant.text.chars().take(2000).collect(),
    )))
}

/// Compare-and-set: a concurrent rename or another generator always wins.
/// Missing source is deliberately excluded (legacy rows are user-final).
pub async fn apply(db: &Database, actor: &str, id: &str, text: &str) -> AppResult<bool> {
    let Some(title) = generated(text) else {
        return Ok(false);
    };
    let row = super::assistant_nyxagent::get(db, actor, id).await?;
    if !eligible(&row) {
        return Ok(false);
    }
    Ok(db.collection::<AssistantConversation>(COLLECTION_NAME).update_one(
        doc! {"_id": id, "user_id": actor, "title_source": "provisional", "guest_turn": {"$ne": true}},
        doc! {"$set": {"title": title, "title_source": "generated"}},
    ).await?.modified_count == 1)
}

#[cfg(test)]
#[path = "assistant_title_service_tests.rs"]
mod tests;

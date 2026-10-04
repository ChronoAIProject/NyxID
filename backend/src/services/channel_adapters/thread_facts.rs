//! Pure native event interpretation. No I/O, conversation allocation or admission.
use serde_json::Value;

use crate::models::{
    channel_bot::ChannelBot,
    channel_thread::{ChannelThreadFacts, ThreadAddress, ThreadKind, ThreadSenderKind},
};
use crate::services::channel_platform::InboundMessage;

fn base(message: &InboundMessage, kind: ThreadKind) -> ChannelThreadFacts {
    ChannelThreadFacts {
        version: 1,
        kind,
        chat_id: message.conversation_id.clone(),
        message_id: message.platform_message_id.clone(),
        parent_message_id: message.reply_to_platform_message_id.clone(),
        ..Default::default()
    }
}

fn string(value: &Value) -> Option<String> {
    value.as_str().filter(|s| !s.is_empty()).map(str::to_owned)
}

fn numeric_id(value: &Value) -> Option<String> {
    value.as_i64().filter(|id| *id > 0).map(|id| id.to_string())
}

fn human(is_bot: Option<bool>) -> ThreadSenderKind {
    match is_bot {
        Some(false) => ThreadSenderKind::Human,
        Some(true) => ThreadSenderKind::Bot,
        None => ThreadSenderKind::Unknown,
    }
}

pub(super) fn telegram(message: &InboundMessage, bot: &ChannelBot) -> Option<ChannelThreadFacts> {
    if message.conversation_type == "private" {
        return None;
    }
    let raw = &message.raw_data;
    let native = raw
        .get("message")
        .or_else(|| raw.get("edited_message"))
        .or_else(|| raw.get("channel_post"))?;
    // message_thread_id can also appear on non-forum reply threads. Only
    // positive forum evidence makes the follow scope an entire topic.
    let topic = (native["is_topic_message"] == true || native["chat"]["is_forum"] == true)
        .then(|| numeric_id(&native["message_thread_id"]))
        .flatten();
    let mut facts = base(
        message,
        if topic.is_some() {
            ThreadKind::Topic
        } else {
            ThreadKind::ReplyChain
        },
    );
    facts.sender_kind = if native.get("sender_chat").is_some() {
        ThreadSenderKind::Unknown
    } else {
        human(native["from"]["is_bot"].as_bool())
    };
    facts.native_thread_id = topic.clone();
    facts.root_id = topic.or_else(|| {
        facts
            .parent_message_id
            .is_none()
            .then(|| facts.message_id.clone())
    });
    facts.address = if numeric_id(&native["reply_to_message"]["from"]["id"]).as_deref()
        == Some(bot.platform_bot_id.as_str())
    {
        ThreadAddress::ReplyToBot
    } else if telegram_mentions(native, bot) {
        ThreadAddress::Mention
    } else {
        ThreadAddress::NotAddressed
    };
    Some(facts)
}

fn telegram_mentions(message: &Value, bot: &ChannelBot) -> bool {
    let text = message["text"]
        .as_str()
        .or_else(|| message["caption"].as_str())
        .unwrap_or_default();
    message["entities"]
        .as_array()
        .or_else(|| message["caption_entities"].as_array())
        .is_some_and(|entities| {
            entities.iter().any(|entity| {
                if entity["type"] == "text_mention" {
                    return numeric_id(&entity["user"]["id"]).as_deref()
                        == Some(bot.platform_bot_id.as_str());
                }
                if entity["type"] != "mention" || bot.platform_bot_username.is_empty() {
                    return false;
                }
                let Some(offset) = entity["offset"]
                    .as_u64()
                    .and_then(|n| usize::try_from(n).ok())
                else {
                    return false;
                };
                let Some(length) = entity["length"]
                    .as_u64()
                    .and_then(|n| usize::try_from(n).ok())
                else {
                    return false;
                };
                // Telegram entity offsets count UTF-16 code units, not UTF-8 bytes.
                let units: Vec<_> = text
                    .encode_utf16()
                    .skip(offset)
                    .take(length.min(256))
                    .collect();
                length <= 256
                    && units.len() == length
                    && String::from_utf16(&units).is_ok_and(|mention| {
                        mention.eq_ignore_ascii_case(&format!(
                            "@{}",
                            bot.platform_bot_username.trim_start_matches('@')
                        ))
                    })
            })
        })
}

pub(super) fn slack(
    message: &InboundMessage,
    bot: &ChannelBot,
    own_id: Option<&str>,
) -> Option<ChannelThreadFacts> {
    if message.conversation_type == "private" {
        return None;
    }
    let event = message.raw_data.get("event")?;
    if !matches!(event["type"].as_str(), Some("message" | "app_mention")) {
        return None;
    }
    // auth.test normally stores bot_id (B...), which cannot identify a user
    // mention (U.../W...). An app_mention still supplies positive evidence.
    let own_id = own_id.filter(|id| !id.is_empty()).or_else(|| {
        (bot.platform_bot_id.starts_with('U') || bot.platform_bot_id.starts_with('W'))
            .then_some(bot.platform_bot_id.as_str())
    });
    let mut facts = base(message, ThreadKind::Native);
    facts.root_id = string(&event["thread_ts"]).or_else(|| Some(facts.message_id.clone()));
    facts.native_thread_id = facts.root_id.clone();
    facts.sender_kind = if event.get("bot_id").is_some()
        || event["subtype"] == "bot_message"
        || own_id.is_some_and(|id| event["user"].as_str() == Some(id))
    {
        ThreadSenderKind::Bot
    } else if event["user"].as_str().is_some_and(|id| !id.is_empty()) {
        ThreadSenderKind::Human
    } else {
        ThreadSenderKind::Unknown
    };
    facts.address = if event["type"] == "app_mention"
        || own_id.is_some_and(|id| {
            event["text"]
                .as_str()
                .is_some_and(|text| text.contains(&format!("<@{id}>")))
        }) {
        ThreadAddress::Mention
    } else if own_id.is_some() {
        ThreadAddress::NotAddressed
    } else {
        ThreadAddress::Unknown
    };
    Some(facts)
}

pub(super) fn discord(message: &InboundMessage, bot: &ChannelBot) -> Option<ChannelThreadFacts> {
    if message.conversation_type == "private" {
        return None;
    }
    let raw = &message.raw_data;
    let event = raw.get("d").unwrap_or(raw);
    // Interactions carry reply credentials, not persistent thread identity.
    event.get("author")?;
    let mut facts = base(message, ThreadKind::Unknown);
    facts.sender_kind = if event.get("webhook_id").is_some()
        || (!bot.platform_bot_id.is_empty()
            && event["author"]["id"].as_str() == Some(bot.platform_bot_id.as_str()))
    {
        ThreadSenderKind::Bot
    } else if string(&event["author"]["id"]).is_some() {
        // Discord's optional bot field is absent for ordinary users.
        human(
            event["author"]
                .get("bot")
                .map_or(Some(false), Value::as_bool),
        )
    } else {
        ThreadSenderKind::Unknown
    };
    let channel_type = event["channel_type"]
        .as_u64()
        .or_else(|| event["channel"]["type"].as_u64());
    if matches!(channel_type, Some(10..=12)) {
        facts.kind = ThreadKind::Native;
        facts.root_id = Some(facts.chat_id.clone());
        facts.native_thread_id = facts.root_id.clone();
        facts.parent_chat_id = string(&event["channel"]["parent_id"]);
    } else if matches!(event["thread"]["type"].as_u64(), Some(10..=12)) {
        facts.kind = ThreadKind::Native;
        facts.root_id = string(&event["thread"]["id"]);
        facts.native_thread_id = facts.root_id.clone();
        facts.parent_chat_id = string(&event["thread"]["parent_id"]);
    } else if matches!(channel_type, Some(0 | 5)) {
        facts.kind = ThreadKind::ReplyChain;
        facts.root_id = facts
            .parent_message_id
            .is_none()
            .then(|| facts.message_id.clone());
    }
    // Missing channel metadata stays unresolved; a native thread also uses channel_id.
    facts.address = if event["referenced_message"]["author"]["id"].as_str()
        == Some(bot.platform_bot_id.as_str())
    {
        ThreadAddress::ReplyToBot
    } else if event["mentions"].as_array().is_some_and(|users| {
        users
            .iter()
            .any(|user| user["id"].as_str() == Some(bot.platform_bot_id.as_str()))
    }) {
        ThreadAddress::Mention
    } else {
        ThreadAddress::NotAddressed
    };
    Some(facts)
}

pub(super) fn lark(message: &InboundMessage, own_id: Option<&str>) -> Option<ChannelThreadFacts> {
    if message.conversation_type == "private" {
        return None;
    }
    let event = message.raw_data.get("event")?;
    let native = event.get("message")?;
    let mut facts = base(message, ThreadKind::Native);
    // Lark represents absent ancestry with empty strings on some events.
    // Normalize only the new facts; legacy reply metadata stays unchanged.
    facts.parent_message_id = string(&native["parent_id"]);
    facts.native_thread_id = string(&native["thread_id"]);
    facts.root_id = string(&native["root_id"]).or_else(|| {
        (facts.parent_message_id.is_none() && facts.native_thread_id.is_none())
            .then(|| facts.message_id.clone())
    });
    facts.sender_kind = match event["sender"]["sender_type"].as_str() {
        Some("user") => ThreadSenderKind::Human,
        Some("app") => ThreadSenderKind::Bot,
        _ => ThreadSenderKind::Unknown,
    };
    facts.address = match own_id.filter(|id| !id.is_empty()) {
        Some(id)
            if native["mentions"].as_array().is_some_and(|mentions| {
                mentions
                    .iter()
                    .any(|mention| mention["id"]["open_id"].as_str() == Some(id))
            }) =>
        {
            ThreadAddress::Mention
        }
        Some(_) => ThreadAddress::NotAddressed,
        None => ThreadAddress::Unknown,
    };
    Some(facts)
}

/// Aurinko is private-classified by the legacy router, but its provider
/// `threadId` is shared by every participant. Keep only keyed HMAC-SHA256
/// fingerprints of addresses in durable facts so guest history can be proved
/// without storing recipients or reversible unkeyed digests.
pub(super) fn aurinko(message: &InboundMessage) -> Option<ChannelThreadFacts> {
    if message.conversation_type != "private" {
        return None;
    }
    let email = message.raw_data.get("email")?;
    let thread = string(&email["thread_id"])?;
    let account = string(&email["account_id"])?;
    let mut facts = base(message, ThreadKind::Email);
    facts.chat_id = message.conversation_id.clone();
    facts.parent_chat_id = Some(account);
    facts.root_id = Some(thread.clone());
    facts.native_thread_id = Some(thread);
    facts.parent_message_id = string(&email["reply_parent_id"]);
    facts.sender_kind = ThreadSenderKind::Human;
    facts.sender_hash = email["sender_hash"].as_str().map(str::to_owned);
    facts.participant_hashes = email["participant_hashes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|hash| hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()))
        .map(str::to_owned)
        .collect();
    facts.address = if email["mailbox_to"].as_bool().unwrap_or(false) {
        ThreadAddress::MailboxTo
    } else {
        ThreadAddress::NotAddressed
    };
    Some(facts)
}

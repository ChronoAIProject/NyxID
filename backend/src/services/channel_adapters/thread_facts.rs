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

#[derive(Clone, Copy, Default)]
struct MentionState {
    bot: bool,
    other: bool,
    everyone: bool,
}

impl MentionState {
    fn mentions_others(self) -> bool {
        self.other && !self.bot && !self.everyone
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
    let mentions = telegram_mentions(native, bot);
    facts.mentions_others = mentions.mentions_others();
    facts.address = if numeric_id(&native["reply_to_message"]["from"]["id"]).as_deref()
        == Some(bot.platform_bot_id.as_str())
    {
        ThreadAddress::ReplyToBot
    } else if mentions.bot {
        ThreadAddress::Mention
    } else {
        ThreadAddress::NotAddressed
    };
    Some(facts)
}

fn telegram_mentions(message: &Value, bot: &ChannelBot) -> MentionState {
    let mut state = MentionState::default();
    let text = message["text"]
        .as_str()
        .or_else(|| message["caption"].as_str())
        .unwrap_or_default();
    let Some(entities) = message["entities"]
        .as_array()
        .or_else(|| message["caption_entities"].as_array())
    else {
        return state;
    };
    for entity in entities {
        if entity["type"] == "text_mention" {
            match numeric_id(&entity["user"]["id"]) {
                Some(id) if id == bot.platform_bot_id => state.bot = true,
                Some(_) => state.other = true,
                None => {}
            }
            continue;
        }
        if entity["type"] != "mention" || bot.platform_bot_username.is_empty() {
            continue;
        }
        let Some(offset) = entity["offset"]
            .as_u64()
            .and_then(|n| usize::try_from(n).ok())
        else {
            continue;
        };
        let Some(length) = entity["length"]
            .as_u64()
            .and_then(|n| usize::try_from(n).ok())
        else {
            continue;
        };
        // Telegram entity offsets count UTF-16 code units, not UTF-8 bytes.
        let units: Vec<_> = text
            .encode_utf16()
            .skip(offset)
            .take(length.min(256))
            .collect();
        if length > 256 || units.len() != length {
            continue;
        }
        let Ok(mention) = String::from_utf16(&units) else {
            continue;
        };
        if mention.eq_ignore_ascii_case(&format!(
            "@{}",
            bot.platform_bot_username.trim_start_matches('@')
        )) {
            state.bot = true;
        } else {
            state.other = true;
        }
    }
    state
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
    let mentions = slack_mentions(event, own_id, event["type"] == "app_mention");
    facts.mentions_others = mentions.mentions_others();
    facts.address = if mentions.bot
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

fn slack_mentions(event: &Value, own_id: Option<&str>, app_mention: bool) -> MentionState {
    let mut state = MentionState {
        bot: app_mention,
        ..Default::default()
    };
    for key in ["blocks", "elements"] {
        if let Some(value) = event.get(key) {
            collect_slack_mentions(value, own_id, &mut state);
        }
    }
    state
}

fn collect_slack_mentions(value: &Value, own_id: Option<&str>, state: &mut MentionState) {
    match value {
        Value::String(_) => {}
        Value::Array(values) => {
            for value in values {
                collect_slack_mentions(value, own_id, state);
            }
        }
        Value::Object(values) => {
            if values.get("type").and_then(Value::as_str) == Some("user")
                && let Some(id) = values.get("user_id").and_then(Value::as_str)
            {
                if own_id == Some(id) {
                    state.bot = true;
                } else if !id.is_empty() {
                    state.other = true;
                }
            }
            if values.get("type").and_then(Value::as_str) == Some("broadcast")
                && matches!(
                    values.get("range").and_then(Value::as_str),
                    Some("here" | "channel" | "everyone")
                )
            {
                state.everyone = true;
            }
            if values.get("type").and_then(Value::as_str) == Some("mrkdwn")
                && let Some(text) = values.get("text").and_then(Value::as_str)
            {
                collect_slack_mrkdwn(text, own_id, state);
            }
            for (key, value) in values {
                if key != "text" || values.get("type").and_then(Value::as_str) != Some("mrkdwn") {
                    collect_slack_mentions(value, own_id, state);
                }
            }
        }
        _ => {}
    }
}

fn collect_slack_mrkdwn(text: &str, own_id: Option<&str>, state: &mut MentionState) {
    let mut rest = text;
    while let Some(start) = rest.find('<') {
        rest = &rest[start + 1..];
        let Some(end) = rest.find('>') else { break };
        let token = &rest[..end];
        if let Some(id) = token.strip_prefix('@') {
            if own_id.is_some_and(|own| id.split('|').next() == Some(own)) {
                state.bot = true;
            } else if !id.is_empty() {
                state.other = true;
            }
        } else if matches!(
            token.split('|').next(),
            Some("!here" | "!channel" | "!everyone")
        ) {
            state.everyone = true;
        }
        rest = &rest[end + 1..];
    }
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
    let mentions = discord_mentions(event, bot);
    facts.mentions_others = mentions.mentions_others();
    facts.address = if event["referenced_message"]["author"]["id"].as_str()
        == Some(bot.platform_bot_id.as_str())
    {
        ThreadAddress::ReplyToBot
    } else if mentions.bot {
        ThreadAddress::Mention
    } else {
        ThreadAddress::NotAddressed
    };
    Some(facts)
}

fn discord_mentions(event: &Value, bot: &ChannelBot) -> MentionState {
    let mut state = MentionState {
        everyone: event["mention_everyone"].as_bool().unwrap_or(false)
            || event["mention_roles"]
                .as_array()
                .is_some_and(|roles| !roles.is_empty()),
        ..Default::default()
    };
    if let Some(users) = event["mentions"].as_array() {
        for user in users {
            match user["id"].as_str() {
                Some(id) if id == bot.platform_bot_id => state.bot = true,
                Some(_) => state.other = true,
                None => {}
            }
        }
    }
    state
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
    let mentions = lark_mentions(native, own_id);
    facts.mentions_others = mentions.mentions_others();
    facts.address = match own_id.filter(|id| !id.is_empty()) {
        Some(_) if mentions.bot => ThreadAddress::Mention,
        Some(_) => ThreadAddress::NotAddressed,
        None => ThreadAddress::Unknown,
    };
    Some(facts)
}

fn lark_mentions(message: &Value, own_id: Option<&str>) -> MentionState {
    let mut state = MentionState::default();
    let Some(mentions) = message["mentions"].as_array() else {
        return state;
    };
    for mention in mentions {
        let key = mention["key"].as_str().unwrap_or_default();
        if matches!(key, "@_all" | "_all") {
            state.everyone = true;
            continue;
        }
        let ids: Vec<&str> = [
            mention["id"]["open_id"].as_str(),
            mention["id"]["user_id"].as_str(),
            mention["id"].as_str(),
            mention["open_id"].as_str(),
            mention["user_id"].as_str(),
        ]
        .into_iter()
        .flatten()
        .filter(|id| !id.is_empty())
        .collect();
        let user_key = key.starts_with("@_user_");
        if !user_key && ids.is_empty() {
            continue;
        }
        if own_id.is_some_and(|own| ids.contains(&own)) {
            state.bot = true;
        } else {
            state.other = true;
        }
    }
    state
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

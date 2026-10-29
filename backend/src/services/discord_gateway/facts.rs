use super::protocol::Failure;
use crate::{
    models::{
        channel_bot::ChannelBot,
        channel_thread::{ChannelThreadFacts, ThreadAddress, ThreadKind, ThreadSenderKind},
        discord_gateway::DiscordGatewayFacts,
    },
    services::channel_platform::InboundMessage,
};
use serde_json::Value;

pub(crate) fn snowflake(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 20
        && id.bytes().all(|b| b.is_ascii_digit())
        && id
            .parse::<u64>()
            .is_ok_and(|n| n > 0 && n.to_string() == id)
}

/// Called only with MESSAGE_CREATE from the authenticated socket and a channel
/// fetched with that same bot token. No caller-supplied metadata is authority.
pub(super) fn normalize(
    bot: &ChannelBot,
    event: &Value,
    channel: &Value,
) -> Result<Option<(InboundMessage, ChannelThreadFacts)>, Failure> {
    let mut message = event["d"].clone();
    let id = message["id"]
        .as_str()
        .filter(|id| snowflake(id))
        .ok_or(Failure::Protocol)?;
    let chat = message["channel_id"]
        .as_str()
        .filter(|id| snowflake(id))
        .ok_or(Failure::Protocol)?;
    let sender = message["author"]["id"]
        .as_str()
        .filter(|id| snowflake(id))
        .ok_or(Failure::Protocol)?;
    if !matches!(message["type"].as_u64(), Some(0 | 19))
        || message.get("webhook_id").is_some_and(|v| !v.is_null())
    {
        return Ok(None);
    }
    let author_is_bot = match message["author"].get("bot") {
        None | Some(Value::Bool(false)) => sender == bot.platform_bot_id,
        Some(Value::Bool(true)) => true,
        _ => return Err(Failure::Protocol),
    };
    if author_is_bot {
        return Ok(None);
    }
    let kind = channel["type"].as_u64().ok_or(Failure::Protocol)?;
    if channel["id"].as_str() != Some(chat)
        || !matches!(kind, 0..=3 | 5 | 10..=13)
        || (matches!(kind, 1 | 3) != message.get("guild_id").is_none_or(Value::is_null))
        || (!matches!(kind, 1 | 3) && channel["guild_id"] != message["guild_id"])
    {
        return Err(Failure::Protocol);
    }
    let mentions = message["mentions"].as_array().ok_or(Failure::Protocol)?;
    if mentions.len() > 256
        || mentions
            .iter()
            .any(|m| m["id"].as_str().is_none_or(|id| !snowflake(id)))
    {
        return Err(Failure::Protocol);
    }
    let bot_mentioned = mentions
        .iter()
        .any(|m| m["id"].as_str() == Some(&bot.platform_bot_id));
    let others = mentions
        .iter()
        .any(|m| m["id"].as_str().is_some_and(|id| id != bot.platform_bot_id));
    let broadcast_everyone_or_here = message["mention_everyone"]
        .as_bool()
        .ok_or(Failure::Protocol)?;
    let roles = message["mention_roles"]
        .as_array()
        .ok_or(Failure::Protocol)?;
    if roles.len() > 256
        || roles
            .iter()
            .any(|v| v.as_str().is_none_or(|id| !snowflake(id)))
    {
        return Err(Failure::Protocol);
    }
    let broadcast_roles = !roles.is_empty();
    let mentions_others =
        others && !bot_mentioned && !broadcast_everyone_or_here && !broadcast_roles;
    let reference = &message["message_reference"];
    let valid_reference = reference.get("type").is_none_or(|v| v.as_u64() == Some(0))
        && reference["channel_id"].as_str().is_none_or(|id| id == chat);
    let parent = valid_reference
        .then(|| reference["message_id"].as_str().filter(|id| snowflake(id)))
        .flatten();
    let reply_to_bot = parent.is_some()
        && reference.get("type").is_none_or(|v| v.as_u64() == Some(0))
        && reference["channel_id"].as_str().is_none_or(|id| id == chat)
        && message["referenced_message"]["id"].as_str() == parent
        && message["referenced_message"]["channel_id"].as_str() == Some(chat)
        && message["referenced_message"]["author"]["id"].as_str() == Some(&bot.platform_bot_id);
    let is_thread = matches!(kind, 10..=12);
    let parent_chat = if is_thread {
        Some(
            channel["parent_id"]
                .as_str()
                .filter(|id| snowflake(id) && *id != chat)
                .ok_or(Failure::Protocol)?
                .to_owned(),
        )
    } else {
        None
    };
    let facts = ChannelThreadFacts {
        version: 1,
        kind: if is_thread {
            ThreadKind::Native
        } else if matches!(kind, 0 | 5) {
            ThreadKind::ReplyChain
        } else {
            ThreadKind::Unknown
        },
        chat_id: chat.into(),
        message_id: id.into(),
        parent_chat_id: parent_chat,
        root_id: if is_thread {
            Some(chat.into())
        } else if matches!(kind, 0 | 5) && parent.is_none() {
            Some(id.into())
        } else {
            None
        },
        native_thread_id: is_thread.then(|| chat.into()),
        parent_message_id: parent.map(str::to_owned),
        sender_kind: ThreadSenderKind::Human,
        address: if bot_mentioned {
            ThreadAddress::Mention
        } else if reply_to_bot {
            ThreadAddress::ReplyToBot
        } else {
            ThreadAddress::NotAddressed
        },
        mentions_others,
        discord_gateway: Some(DiscordGatewayFacts {
            version: 1,
            bot_id: bot.platform_bot_id.clone(),
            bot_mentioned,
            mentions_others,
            broadcast_everyone_or_here,
            broadcast_roles,
            reply_to_bot,
            author_is_bot,
            channel_type: kind as u8,
        }),
        ..Default::default()
    };
    // Native consumers use exactly this verified view. Remove uncorrelated
    // quoted authors, and never mistake a message's attached thread for the
    // channel in which this message was received.
    if !reply_to_bot {
        message
            .as_object_mut()
            .ok_or(Failure::Protocol)?
            .remove("referenced_message");
    }
    if !valid_reference {
        message
            .as_object_mut()
            .ok_or(Failure::Protocol)?
            .remove("message_reference");
    }
    message["channel"] = channel.clone();
    message
        .as_object_mut()
        .ok_or(Failure::Protocol)?
        .remove("thread");
    let mut inbound = crate::services::channel_adapters::discord::parse_gateway_message(&message)
        .ok_or(Failure::Protocol)?;
    inbound.conversation_type = if kind == 1 { "private" } else { "group" }.into();
    inbound.thread_id = facts.native_thread_id.clone();
    Ok(Some((inbound, facts)))
}

/// Reuse the socket verifier for authenticated REST evidence without exposing
/// the socket protocol's internal failure type to provider handlers.
pub(crate) fn authenticated_facts(
    bot: &ChannelBot,
    event: &Value,
    channel: &Value,
) -> Option<ChannelThreadFacts> {
    normalize(bot, event, channel)
        .ok()
        .flatten()
        .map(|(_, facts)| facts)
}

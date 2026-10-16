use crate::services::channel_platform::OutboundReply;

pub fn chat_thread(thread: &crate::models::nyxbot_channel::NyxbotThread) -> Option<&str> {
    match thread.telegram_topic.as_ref() {
        Some(crate::models::channel_thread::TelegramTopic::Chat) => None,
        Some(crate::models::channel_thread::TelegramTopic::Topic { id }) => Some(id),
        None => thread.platform_thread_id.as_deref(),
    }
}

pub fn message_thread(message: &crate::models::channel_message::ChannelMessage) -> Option<&str> {
    match message
        .thread_context
        .as_ref()
        .and_then(|facts| facts.telegram_topic.as_ref())
    {
        Some(crate::models::channel_thread::TelegramTopic::Chat) => None,
        Some(crate::models::channel_thread::TelegramTopic::Topic { id }) => Some(id),
        None => message.thread_id.as_deref(),
    }
}

pub fn facts_thread<'a>(
    platform: &str,
    facts: &'a crate::models::channel_thread::ChannelThreadFacts,
) -> Option<&'a str> {
    if platform == "slack" {
        return facts.root_id.as_deref();
    }
    match facts.telegram_topic.as_ref() {
        Some(crate::models::channel_thread::TelegramTopic::Chat) => None,
        Some(crate::models::channel_thread::TelegramTopic::Topic { id }) => Some(id),
        None => facts.native_thread_id.as_deref(),
    }
}

pub fn native_thread(platform: &str, reply: &OutboundReply) -> Option<String> {
    match platform {
        "telegram" | "telegram-new" => reply
            .metadata
            .as_ref()
            .and_then(|metadata| metadata.get("message_thread_id"))
            .and_then(|value| {
                value
                    .as_i64()
                    .or_else(|| value.as_str()?.parse::<i64>().ok())
            })
            .map(|id| id.to_string()),
        "slack" => reply
            .metadata
            .as_ref()
            .and_then(|metadata| metadata.get("thread_ts"))
            .and_then(|value| value.as_str())
            .map(str::to_owned)
            .or_else(|| reply.reply_to_platform_message_id.clone()),
        _ => None,
    }
}

pub fn resolve(bot_id: &str, platform: &str, chat_id: &str, reply: &OutboundReply) -> String {
    let chat_id = normalized_chat(platform, chat_id);
    format!(
        "{bot_id}\u{1f}{chat_id}\u{1f}{}",
        native_thread(platform, reply).unwrap_or_default()
    )
}

pub fn normalized_chat(platform: &str, chat_id: &str) -> String {
    if platform == "whatsapp" {
        let recipient: String = chat_id.chars().filter(|c| *c != ' ' && *c != '-').collect();
        recipient.strip_prefix('+').unwrap_or(&recipient).to_owned()
    } else {
        chat_id.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn reply(metadata: Option<serde_json::Value>, parent: Option<&str>) -> OutboundReply {
        OutboundReply {
            text: None,
            metadata,
            attachments: Vec::new(),
            reply_to_platform_message_id: parent.map(str::to_owned),
        }
    }

    #[test]
    fn image_audiences_follow_adapter_topics_and_slack_roots() {
        let root = reply(None, None);
        let chain = reply(None, Some("123"));
        assert_eq!(
            resolve("bot", "telegram", "chat", &root),
            "bot\u{1f}chat\u{1f}"
        );
        assert_eq!(
            resolve("bot", "telegram", "chat", &root),
            resolve("bot", "telegram", "chat", &chain)
        );
        let topic = reply(Some(json!({"message_thread_id":"0007"})), Some("123"));
        assert_eq!(
            resolve("bot", "telegram", "chat", &topic),
            "bot\u{1f}chat\u{1f}7"
        );
        assert_eq!(
            resolve("bot", "telegram", "chat", &topic),
            resolve(
                "bot",
                "telegram",
                "chat",
                &reply(Some(json!({"message_thread_id":7})), None)
            )
        );
        assert_ne!(
            resolve("bot", "telegram", "chat", &topic),
            resolve(
                "bot",
                "telegram",
                "chat",
                &reply(Some(json!({"message_thread_id":8})), None)
            )
        );
        assert_eq!(
            resolve(
                "bot",
                "slack",
                "chat",
                &reply(Some(json!({"thread_ts":"root"})), Some("child"))
            ),
            "bot\u{1f}chat\u{1f}root"
        );
        assert_ne!(
            resolve("bot", "slack", "chat", &root),
            resolve("bot", "slack", "chat", &chain)
        );
        assert_eq!(
            resolve("bot", "whatsapp", "+1 555-123", &root),
            "bot\u{1f}1555123\u{1f}"
        );
        for platform in [
            "discord", "lark", "feishu", "whatsapp", "x", "aurinko", "openclaw", "device",
        ] {
            assert_eq!(
                resolve("bot", platform, "chat", &root),
                resolve("bot", platform, "chat", &topic)
            );
        }
    }
}

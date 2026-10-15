use super::{
    channel_platform::OutboundReply, channel_reply_delivery::MAX_REPLY_PARTS,
    channel_text::TextLimit,
};
use crate::errors::{AppError, AppResult};

/// Fully plan before the first provider effect. Routing metadata is copied;
/// interactive content cannot be duplicated across multiple messages.
pub fn text_parts(reply: &OutboundReply, platform: &str) -> AppResult<Vec<OutboundReply>> {
    let text = reply.text.as_deref().unwrap_or_default();
    let chunks: Vec<_> = TextLimit::for_platform(platform)
        .chunks(text)
        .take(MAX_REPLY_PARTS + 1)
        .collect();
    if chunks.len() > MAX_REPLY_PARTS {
        return Err(AppError::ValidationError(
            "Reply exceeds the 64-part limit".into(),
        ));
    }
    if chunks.len() > 1 && chunks.iter().any(|part| part.trim().is_empty()) {
        return Err(AppError::ValidationError(
            "Reply would require a whitespace-only platform message".into(),
        ));
    }
    if chunks.len() > 1 && (!reply.attachments.is_empty() || has_structured_content(reply)) {
        return Err(AppError::ValidationError(
            "Long replies with attachments or interactive metadata require a separate complete text reply".into(),
        ));
    }
    if chunks.is_empty() {
        return Ok(vec![reply.clone()]);
    }
    let parts = chunks
        .into_iter()
        .map(|text| OutboundReply {
            text: Some(text.into()),
            attachments: reply.attachments.clone(),
            metadata: reply.metadata.clone(),
            reply_to_platform_message_id: reply.reply_to_platform_message_id.clone(),
        })
        .collect();
    Ok(parts)
}

pub fn has_structured_content(reply: &OutboundReply) -> bool {
    reply.metadata.as_ref().is_some_and(|metadata| {
        [
            "card",
            "blocks",
            "reply_markup",
            "entities",
            "parse_mode",
            "template",
            "interactive",
            "attachments",
        ]
        .iter()
        .any(|key| metadata.get(key).is_some())
    })
}

/// Rich/media and non-IM transports retain their adapter-owned send semantics.
/// Reject oversized mixed content rather than sending a prefix before failing.
pub fn ordinary(
    reply: &OutboundReply,
    platform: &str,
    chat: &str,
) -> AppResult<Option<Vec<OutboundReply>>> {
    if platform == "aurinko"
        || (platform == "x" && super::channel_adapters::x::is_public_conversation(chat))
    {
        return Ok(None);
    }
    let parts = text_parts(reply, platform)?;
    if !reply.attachments.is_empty() || has_structured_content(reply) {
        let limit = TextLimit::for_platform(platform);
        if reply
            .attachments
            .iter()
            .filter_map(|a| a.caption.as_deref())
            .any(|text| limit.chunks(text).count() > 1)
        {
            return Err(AppError::ValidationError(
                "Attachment caption exceeds the platform text limit".into(),
            ));
        }
        return Ok(None);
    }
    Ok(Some(parts))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reply(text: String) -> OutboundReply {
        OutboundReply {
            text: Some(text),
            attachments: vec![],
            metadata: None,
            reply_to_platform_message_id: Some("42".into()),
        }
    }

    #[test]
    fn preflight_rejects_part_limit_and_interactive_overflow() {
        assert!(text_parts(&reply("a".repeat(4096 * 64 + 1)), "telegram").is_err());
        let mut message = reply("a".repeat(5000));
        message.metadata = Some(serde_json::json!({"reply_markup": {"inline_keyboard": []}}));
        assert!(text_parts(&message, "telegram").is_err());
    }

    #[test]
    fn preflight_keeps_templates_interactions_and_media_on_their_existing_paths() {
        for (platform, key) in [
            ("whatsapp", "template"),
            ("whatsapp", "interactive"),
            ("x", "attachments"),
        ] {
            let mut message = reply("short".into());
            message.metadata = Some(serde_json::json!({key: {}}));
            assert!(ordinary(&message, platform, "123").unwrap().is_none());
            message.text = Some("a".repeat(10_001));
            assert!(ordinary(&message, platform, "123").is_err());
        }
    }

    #[test]
    fn preflight_rejects_whitespace_only_parts_before_sending_a_prefix() {
        let message = reply("a".repeat(4096) + &" ".repeat(4096) + "tail");
        assert!(text_parts(&message, "telegram").is_err());
    }

    #[test]
    fn preserves_full_text_and_routing_in_all_parts() {
        let mut message = reply("🦉界\\* paragraph\n\n".repeat(1000));
        message.metadata = Some(serde_json::json!({"message_thread_id": 123}));
        let parts = text_parts(&message, "telegram").unwrap();
        assert_eq!(
            parts
                .iter()
                .filter_map(|p| p.text.as_deref())
                .collect::<String>(),
            message.text.unwrap()
        );
        assert!(
            parts
                .iter()
                .all(|p| p.reply_to_platform_message_id.as_deref() == Some("42")
                    && p.metadata == message.metadata)
        );
    }
}

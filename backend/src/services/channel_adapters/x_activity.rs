use super::*;
use crate::models::channel_activity::ActivityMetadata;
use crate::services::channel_activity_service::{ActivityDescriptor, occurrence};

pub(super) const DESCRIPTORS: &[ActivityDescriptor] = &[
    ActivityDescriptor {
        kind: "dm",
        label: "Direct message",
        subscription: "dm",
        subscription_label: "Direct messages",
        description: "Receive unencrypted direct messages and reply privately.",
        content_availability: "available",
        reply_supported: true,
    },
    ActivityDescriptor {
        kind: "encrypted_chat",
        label: "Encrypted chat",
        subscription: "chat",
        subscription_label: "Encrypted chat notifications",
        description: "Count encrypted messages and notify compatible agents. Message text and replies are unavailable.",
        content_availability: "encrypted",
        reply_supported: false,
    },
    ActivityDescriptor {
        kind: "mention",
        label: "Mention",
        subscription: "mentions",
        subscription_label: "Mentions",
        description: "Receive posts mentioning your account. Agent replies are public.",
        content_availability: "available",
        reply_supported: true,
    },
    ActivityDescriptor {
        kind: "reply",
        label: "Reply to my post",
        subscription: "replies",
        subscription_label: "Replies to my posts",
        description: "Receive direct replies to your posts. Agent replies are public.",
        content_availability: "available",
        reply_supported: true,
    },
    ActivityDescriptor {
        kind: "post",
        label: "My post",
        subscription: "posts",
        subscription_label: "My posts",
        description: "Count posts authored by your account and notify compatible agents without automatic replies.",
        content_availability: "metadata_only",
        reply_supported: false,
    },
];

pub(super) fn metadata(inbound: &InboundMessage) -> ActivityMetadata {
    let name = inbound.raw_data["data"]["event_type"]
        .as_str()
        .unwrap_or("dm.received");
    let selection = match name {
        "chat.received" => XChannelEvent::Chat,
        "post.mention.create" => XChannelEvent::Mentions,
        "post.reply.create" => XChannelEvent::Replies,
        "post.create" => XChannelEvent::Posts,
        _ => XChannelEvent::Dm,
    };
    let subscription = match selection {
        XChannelEvent::Dm => "dm",
        XChannelEvent::Chat => "chat",
        XChannelEvent::Mentions => "mentions",
        XChannelEvent::Replies => "replies",
        XChannelEvent::Posts => "posts",
    };
    let descriptor = DESCRIPTORS
        .iter()
        .find(|entry| entry.subscription == subscription)
        .expect("X activity descriptor");
    let payload = &inbound.raw_data["data"]["payload"];
    let millis = payload["created_at_msec"]
        .as_str()
        .or_else(|| inbound.raw_data["created_timestamp"].as_str());
    let occurred_at = millis
        .and_then(|value| value.parse::<i64>().ok())
        .and_then(chrono::DateTime::from_timestamp_millis)
        .or_else(|| occurrence(payload["created_at"].as_str()))
        .or_else(|| occurrence(inbound.raw_data["created_at"].as_str()));
    ActivityMetadata {
        kind: descriptor.kind.into(),
        provider_event_type: event_name(selection).into(),
        content_availability: if descriptor.reply_supported
            && inbound.text.is_none()
            && inbound.attachments.is_empty()
        {
            "unavailable"
        } else {
            descriptor.content_availability
        }
        .into(),
        reply_supported: descriptor.reply_supported,
        occurred_at,
    }
}

pub(super) fn parse_chat(body: &Value, own_id: &str) -> AppResult<Vec<InboundMessage>> {
    let payload = &body["data"]["payload"];
    let sender = payload["sender_id"]
        .as_str()
        .filter(|id| numeric_id(id))
        .ok_or_else(protocol_error)?;
    if sender == own_id {
        return Ok(vec![]);
    }
    let id = payload["id"]
        .as_str()
        .filter(|id| id.len() == 36 && uuid::Uuid::parse_str(id).is_ok())
        .ok_or_else(protocol_error)?;
    let conversation = payload["conversation_id"]
        .as_str()
        .filter(|value| {
            !value.is_empty()
                && value.len() <= 256
                && value
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b':' || b == b'-' || b == b'_')
        })
        .ok_or_else(protocol_error)?;
    // Direct chat IDs contain both participants. Opaque group IDs are bound by
    // the signed, selected user filter checked by verify_webhook.
    let participants: Vec<_> = conversation.split(':').collect();
    if participants.len() > 1
        && (participants.len() != 2
            || !participants.contains(&own_id)
            || !participants.contains(&sender))
    {
        return Err(protocol_error());
    }
    Ok(vec![InboundMessage {
        platform_message_id: uuid::Uuid::parse_str(id)
            .map_err(|_| protocol_error())?
            .to_string(),
        conversation_id: format!("chat:{conversation}"),
        conversation_type: if participants.len() == 2 {
            "private"
        } else {
            "group"
        }
        .into(),
        sender_platform_id: sender.into(),
        sender_display_name: None,
        content_type: "unknown".into(),
        text: None,
        attachments: vec![],
        reply_to_platform_message_id: None,
        thread_id: None,
        // Crypto material never crosses the parser boundary.
        raw_data: json!({"data": {"event_type": "chat.received", "payload": {"created_at_msec": payload["created_at_msec"]}}}),
    }])
}

pub(super) fn parse_own_post(body: &Value, own_id: &str) -> AppResult<Vec<InboundMessage>> {
    let post = &body["data"]["payload"];
    if post["author_id"] != own_id {
        return Ok(vec![]);
    }
    let id = post["id"]
        .as_str()
        .filter(|id| numeric_id(id))
        .ok_or_else(protocol_error)?;
    let conversation = post["conversation_id"]
        .as_str()
        .filter(|id| numeric_id(id))
        .ok_or_else(protocol_error)?;
    Ok(vec![InboundMessage {
        platform_message_id: id.into(),
        conversation_id: format!("post:{conversation}"),
        conversation_type: "channel".into(),
        sender_platform_id: own_id.into(),
        sender_display_name: None,
        content_type: "unknown".into(),
        text: None,
        attachments: vec![],
        reply_to_platform_message_id: None,
        thread_id: None,
        raw_data: json!({"data": {"event_type": "post.create", "payload": {"created_at": post["created_at"]}}}),
    }])
}

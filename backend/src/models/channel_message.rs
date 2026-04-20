use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "channel_messages";

/// A metadata-only record of a message flowing through the channel bot
/// relay pipeline.
///
/// **Per ADR-013 (NyxID Pure Passthrough) this record does not store message
/// content.** Historical deployments may still have `text`, `attachments`, or
/// `raw_platform_data` fields on existing documents; a startup migration in
/// `db::ensure_indexes` unsets them on first run. Serde's default behavior of
/// ignoring unknown fields keeps old documents readable during the rollout
/// window.
///
/// Message *content* now lives exclusively in the downstream agent (Aevatar
/// grain state, or wherever the agent persists its conversation history).
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChannelMessage {
    #[serde(rename = "_id")]
    pub id: String,
    /// The bot that hosts this message's conversation. `None` for messages on
    /// device channels (HTTP Event Gateway, platform = "device").
    #[serde(default)]
    pub channel_bot_id: Option<String>,
    pub conversation_id: String,
    /// The actual platform chat/channel ID (e.g. Telegram chat_id).
    /// Stored separately from the conversation route's platform_conversation_id
    /// because default routes may use a wildcard.
    #[serde(default)]
    pub platform_conversation_id: Option<String>,
    pub user_id: String,
    /// Message direction: "inbound" (platform -> agent) or "outbound" (agent -> platform)
    pub direction: String,
    /// Platform identifier: "telegram", "discord", "lark", "feishu"
    pub platform: String,
    /// Platform-assigned message identifier
    #[serde(default)]
    pub platform_message_id: Option<String>,
    /// Platform-native sender identifier
    #[serde(default)]
    pub sender_platform_id: Option<String>,
    /// Display name of the message sender
    #[serde(default)]
    pub sender_display_name: Option<String>,
    /// Content type: "text", "image", "file", "audio", "video", "unknown"
    pub content_type: String,
    /// Platform-specific routing metadata: Telegram `message_thread_id`,
    /// Discord deferred-interaction token (`interaction:{app}:{token}`),
    /// Lark `thread_id`, etc. **This is routing metadata, not message
    /// content** — it is required to dispatch async replies back to the
    /// correct platform surface (e.g. Discord follow-up webhook for deferred
    /// interactions). Keeping it here is consistent with ADR-013: we only
    /// avoid storing *content*.
    #[serde(default)]
    pub thread_id: Option<String>,
    /// The agent API key that handled this message (set for inbound after routing)
    #[serde(default)]
    pub agent_api_key_id: Option<String>,
    /// Callback delivery status: "pending", "delivered", "failed", "timeout"
    #[serde(default)]
    pub callback_status: Option<String>,
    /// Internal message ID this message is a reply to
    #[serde(default)]
    pub reply_to_message_id: Option<String>,
    /// Platform message ID of the sent reply (set after outbound delivery)
    #[serde(default)]
    pub platform_reply_message_id: Option<String>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collection_name() {
        assert_eq!(COLLECTION_NAME, "channel_messages");
    }

    fn make_message() -> ChannelMessage {
        ChannelMessage {
            id: uuid::Uuid::new_v4().to_string(),
            channel_bot_id: Some(uuid::Uuid::new_v4().to_string()),
            conversation_id: uuid::Uuid::new_v4().to_string(),
            platform_conversation_id: Some("chat_789".to_string()),
            user_id: uuid::Uuid::new_v4().to_string(),
            direction: "inbound".to_string(),
            platform: "telegram".to_string(),
            platform_message_id: Some("msg_123".to_string()),
            sender_platform_id: Some("user_456".to_string()),
            sender_display_name: Some("Alice".to_string()),
            content_type: "text".to_string(),
            thread_id: None,
            agent_api_key_id: None,
            callback_status: None,
            reply_to_message_id: None,
            platform_reply_message_id: None,
            created_at: Utc::now(),
        }
    }

    #[test]
    fn bson_roundtrip() {
        let msg = make_message();
        let doc = bson::to_document(&msg).expect("serialize");
        let restored: ChannelMessage = bson::from_document(doc).expect("deserialize");
        assert_eq!(msg.id, restored.id);
        assert_eq!(msg.direction, restored.direction);
        assert_eq!(msg.content_type, restored.content_type);
        assert_eq!(msg.platform_message_id, restored.platform_message_id);
    }

    #[test]
    fn bson_no_content_fields() {
        // ADR-013 compliance: content must never be persisted.
        let msg = make_message();
        let doc = bson::to_document(&msg).expect("serialize");
        assert!(!doc.contains_key("text"));
        assert!(!doc.contains_key("attachments"));
        assert!(!doc.contains_key("raw_platform_data"));
        assert!(!doc.contains_key("body"));
        assert!(!doc.contains_key("content"));
    }

    #[test]
    fn bson_roundtrip_with_callback_status() {
        let mut msg = make_message();
        msg.callback_status = Some("delivered".to_string());
        msg.agent_api_key_id = Some(uuid::Uuid::new_v4().to_string());
        msg.reply_to_message_id = Some(uuid::Uuid::new_v4().to_string());
        msg.platform_reply_message_id = Some("platform_msg_789".to_string());
        let doc = bson::to_document(&msg).expect("serialize");
        let restored: ChannelMessage = bson::from_document(doc).expect("deserialize");
        assert_eq!(restored.callback_status.as_deref(), Some("delivered"));
        assert!(restored.agent_api_key_id.is_some());
        assert!(restored.reply_to_message_id.is_some());
        assert_eq!(
            restored.platform_reply_message_id.as_deref(),
            Some("platform_msg_789")
        );
    }

    #[test]
    fn bson_required_fields_present() {
        let msg = make_message();
        let doc = bson::to_document(&msg).expect("serialize");
        assert!(doc.contains_key("_id"));
        assert!(doc.contains_key("channel_bot_id"));
        assert!(doc.contains_key("conversation_id"));
        assert!(doc.contains_key("user_id"));
        assert!(doc.contains_key("direction"));
        assert!(doc.contains_key("platform"));
        assert!(doc.contains_key("content_type"));
        assert!(doc.contains_key("created_at"));
    }

    #[test]
    fn bson_backward_compat_missing_optional_fields() {
        let msg = make_message();
        let mut doc = bson::to_document(&msg).expect("serialize");
        doc.remove("platform_message_id");
        doc.remove("sender_platform_id");
        doc.remove("sender_display_name");
        doc.remove("agent_api_key_id");
        doc.remove("callback_status");
        doc.remove("reply_to_message_id");
        doc.remove("platform_reply_message_id");
        let restored: ChannelMessage = bson::from_document(doc).expect("deserialize");
        assert_eq!(restored.platform_message_id, None);
        assert_eq!(restored.callback_status, None);
    }

    #[test]
    fn bson_roundtrip_device_message() {
        // Device events persist a ChannelMessage with no backing bot.
        let mut msg = make_message();
        msg.channel_bot_id = None;
        msg.platform = "device".to_string();
        msg.sender_platform_id = Some("camera-analyzer".to_string());
        let doc = bson::to_document(&msg).expect("serialize");
        let restored: ChannelMessage = bson::from_document(doc).expect("deserialize");
        assert_eq!(restored.channel_bot_id, None);
        assert_eq!(restored.platform, "device");
    }

    #[test]
    fn bson_missing_channel_bot_id_defaults_to_none() {
        let msg = make_message();
        let mut doc = bson::to_document(&msg).expect("serialize");
        doc.remove("channel_bot_id");
        let restored: ChannelMessage = bson::from_document(doc).expect("deserialize");
        assert_eq!(restored.channel_bot_id, None);
    }

    #[test]
    fn legacy_documents_with_content_fields_still_deserialize() {
        // Pre-ADR-013 documents stored text/attachments/raw_platform_data.
        // Removing those fields from the struct must not break deserialization
        // of existing rows — serde drops unknown fields by default.
        let msg = make_message();
        let mut doc = bson::to_document(&msg).expect("serialize");
        doc.insert("text", "legacy message body");
        doc.insert(
            "attachments",
            bson::to_bson(&vec![bson::doc! {
                "content_type": "image",
                "url": "https://example.com/legacy.jpg",
            }])
            .unwrap(),
        );
        doc.insert("raw_platform_data", bson::doc! { "update_id": 42 });
        let restored: ChannelMessage = bson::from_document(doc).expect("deserialize legacy doc");
        assert_eq!(restored.id, msg.id);
    }
}

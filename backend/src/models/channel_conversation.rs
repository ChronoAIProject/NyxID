use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::bson_datetime;

pub const COLLECTION_NAME: &str = "channel_conversations";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ChannelConversation {
    #[serde(rename = "_id")]
    pub id: String,
    pub user_id: String,
    /// The bot that hosts this conversation. `None` when `platform == "device"`
    /// (HTTP Event Gateway channels are not backed by a bot).
    #[serde(default)]
    pub channel_bot_id: Option<String>,
    /// Platform identifier: "telegram" | "discord" | "lark" | "feishu" | "device"
    pub platform: String,
    /// Platform-native conversation/chat identifier
    pub platform_conversation_id: String,
    /// Conversation type: "private", "group", "channel"
    pub platform_conversation_type: String,
    /// Optional sender identifier within the conversation (for group contexts)
    #[serde(default)]
    pub platform_sender_id: Option<String>,
    /// The API key (agent) that handles messages for this conversation
    pub agent_api_key_id: String,
    /// Whether this is the default agent for new conversations from this bot
    #[serde(default)]
    pub default_agent: bool,
    pub is_active: bool,
    #[serde(default, with = "bson_datetime::optional")]
    pub last_message_at: Option<DateTime<Utc>>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collection_name() {
        assert_eq!(COLLECTION_NAME, "channel_conversations");
    }

    fn make_conversation() -> ChannelConversation {
        ChannelConversation {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: uuid::Uuid::new_v4().to_string(),
            channel_bot_id: Some(uuid::Uuid::new_v4().to_string()),
            platform: "telegram".to_string(),
            platform_conversation_id: "chat_12345".to_string(),
            platform_conversation_type: "private".to_string(),
            platform_sender_id: None,
            agent_api_key_id: uuid::Uuid::new_v4().to_string(),
            default_agent: false,
            is_active: true,
            last_message_at: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn bson_roundtrip() {
        let conv = make_conversation();
        let doc = bson::to_document(&conv).expect("serialize");
        let restored: ChannelConversation = bson::from_document(doc).expect("deserialize");
        assert_eq!(conv.id, restored.id);
        assert_eq!(conv.platform, restored.platform);
        assert_eq!(
            conv.platform_conversation_id,
            restored.platform_conversation_id
        );
        assert_eq!(conv.agent_api_key_id, restored.agent_api_key_id);
    }

    #[test]
    fn bson_roundtrip_with_optional_fields() {
        let mut conv = make_conversation();
        conv.platform_sender_id = Some("user_789".to_string());
        conv.last_message_at = Some(Utc::now());
        conv.default_agent = true;
        let doc = bson::to_document(&conv).expect("serialize");
        let restored: ChannelConversation = bson::from_document(doc).expect("deserialize");
        assert_eq!(restored.platform_sender_id.as_deref(), Some("user_789"));
        assert!(restored.last_message_at.is_some());
        assert!(restored.default_agent);
    }

    #[test]
    fn bson_all_fields_serialized() {
        let conv = make_conversation();
        let doc = bson::to_document(&conv).expect("serialize");
        assert!(doc.contains_key("_id"));
        assert!(doc.contains_key("user_id"));
        assert!(doc.contains_key("channel_bot_id"));
        assert!(doc.contains_key("platform"));
        assert!(doc.contains_key("platform_conversation_id"));
        assert!(doc.contains_key("platform_conversation_type"));
        assert!(doc.contains_key("agent_api_key_id"));
        assert!(doc.contains_key("default_agent"));
        assert!(doc.contains_key("is_active"));
        assert!(doc.contains_key("created_at"));
        assert!(doc.contains_key("updated_at"));
    }

    #[test]
    fn bson_backward_compat_missing_optional_fields() {
        let conv = make_conversation();
        let mut doc = bson::to_document(&conv).expect("serialize");
        doc.remove("platform_sender_id");
        doc.remove("default_agent");
        doc.remove("last_message_at");
        let restored: ChannelConversation = bson::from_document(doc).expect("deserialize");
        assert_eq!(restored.platform_sender_id, None);
        assert!(!restored.default_agent);
        assert_eq!(restored.last_message_at, None);
    }

    #[test]
    fn bson_roundtrip_device_conversation() {
        // Device conversations have platform = "device" and no backing bot.
        let mut conv = make_conversation();
        conv.channel_bot_id = None;
        conv.platform = "device".to_string();
        conv.platform_conversation_id = "household-camera".to_string();
        conv.platform_conversation_type = "device".to_string();
        let doc = bson::to_document(&conv).expect("serialize");
        let restored: ChannelConversation = bson::from_document(doc).expect("deserialize");
        assert_eq!(restored.channel_bot_id, None);
        assert_eq!(restored.platform, "device");
        assert_eq!(restored.platform_conversation_id, "household-camera");
    }

    #[test]
    fn bson_missing_channel_bot_id_defaults_to_none() {
        // Forward compat: a document written without channel_bot_id (device
        // conversations emit it as null/absent) must deserialize with
        // channel_bot_id = None.
        let conv = make_conversation();
        let mut doc = bson::to_document(&conv).expect("serialize");
        doc.remove("channel_bot_id");
        let restored: ChannelConversation = bson::from_document(doc).expect("deserialize");
        assert_eq!(restored.channel_bot_id, None);
    }
}

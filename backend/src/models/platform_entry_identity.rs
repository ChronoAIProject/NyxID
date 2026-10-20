//! Native shared-entry identity; legacy Telegram rows retain their v1 fields.
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EntryPlatform {
    Slack,
    Discord,
    Whatsapp,
    Lark,
    Feishu,
    X,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeEntryIdentity {
    pub version: u32,
    pub platform: EntryPlatform,
    pub channel_bot_id: String,
    pub platform_bot_id: String,
}

/// Transport binding is an explicit admin choice, independent of sender consent.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum EntryTransport {
    Direct,
    NyxagentGateway {
        channel_id: String,
        gateway_channel_id: String,
        gateway_version: i64,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeEntryConfiguration {
    pub identity: NativeEntryIdentity,
    pub transport: EntryTransport,
    pub transport_generation: i64,
    pub credential_fingerprint: String,
    pub control_surface: NativeControlSurface,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum NativeControlSurface {
    SlackDirectMessage {
        app_id: String,
        team_id: String,
        bot_user_id: String,
    },
    DiscordCommand {
        application_id: String,
        command_id: String,
        command_digest: String,
    },
    PrivateMessage,
}

/// A native ID remains opaque even if a provider currently emits only digits.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NativeSenderIdentity {
    pub sender_id: String,
    pub private_conversation_id: String,
}

pub const NATIVE_RECEIPTS: &str = "platform_native_entry_receipts";

/// Positive adapter provenance and a keyed content commitment, never message
/// text, confirmation codes, interaction tokens or link bearers.
#[derive(Clone, Serialize, Deserialize)]
pub struct NativeEntryReceipt {
    #[serde(rename = "_id")]
    pub id: String,
    pub profile_id: String,
    pub manager_authority_epoch: i64,
    pub configuration: NativeEntryConfiguration,
    pub sender: NativeSenderIdentity,
    pub event_id: String,
    pub message_id: String,
    pub original_private_human: bool,
    pub content_commitment: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub received_at: chrono::DateTime<chrono::Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: chrono::DateTime<chrono::Utc>,
}

impl std::fmt::Debug for NativeEntryReceipt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeEntryReceipt")
            .field("platform", &self.configuration.identity.platform)
            .finish_non_exhaustive()
    }
}

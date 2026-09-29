use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "nyxbot_channels";
pub const THREADS_COLLECTION_NAME: &str = "nyxbot_threads";
pub const EVENTS_COLLECTION_NAME: &str = "nyxbot_events";
pub const WATCHES_COLLECTION_NAME: &str = "nyxbot_watches";

/// A channel bot linked to one of the owner's agents (NyxBot or a
/// specialist). Telegram bots are reached through the Agent Event Gateway (`transport = "gateway"`, NyxID is the gateway's `nyxbot`
/// provider); other platforms use NyxID's relay directly (`"direct"`).
#[derive(Clone, Serialize, Deserialize)]
pub struct NyxbotChannel {
    #[serde(rename = "_id")]
    pub id: String,
    pub user_id: String,
    pub channel_bot_id: String,
    /// Canonical platform (`telegram-new` is stored as `telegram`).
    pub platform: String,
    pub bot_label: String,
    #[serde(default)]
    pub bot_username: Option<String>,
    /// `gateway` or `direct`.
    pub transport: String,
    /// `pending`, `active`, `failed`, or `disconnected`.
    pub status: String,
    /// Stable code only, never upstream prose.
    #[serde(default)]
    pub last_error: Option<String>,
    /// Route key: the channel route's agent key. Its callback URL points at
    /// the gateway (or NyxID's direct receiver). Never serialized to clients.
    pub route_api_key_id: String,
    #[serde(default)]
    pub route_id: Option<String>,
    /// Gateway only: the channel's agent key, the bearer the gateway uses to
    /// call NyxID as provider and NyxID uses for gateway tools.
    #[serde(default)]
    pub agent_api_key_id: Option<String>,
    #[serde(default, with = "crate::models::bson_bytes::optional")]
    pub agent_key_ciphertext: Option<Vec<u8>>,
    #[serde(default)]
    pub gateway_channel_id: Option<String>,
    #[serde(default)]
    pub gateway_record_id: Option<String>,
    #[serde(default)]
    pub gateway_version: Option<i64>,
    #[serde(default)]
    pub binding_id: Option<String>,
    /// Platform sender IDs verified as the owner. Only these senders reach the
    /// owner's Full-access NyxBot; everyone else gets a short refusal.
    #[serde(default)]
    pub owner_sender_ids: Vec<String>,
    /// SHA-256 of the one-time link code the owner sends from the chat app.
    #[serde(default)]
    pub link_code_hash: Option<String>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub link_code_expires_at: Option<DateTime<Utc>>,
    /// The NyxBot chat that connected it, when connected from a chat.
    #[serde(default)]
    pub source_conversation_id: Option<String>,
    /// The agent this bot reaches: the owner's NyxBot (`None` on rows written
    /// before agents existed) or a specialist.
    #[serde(default)]
    pub agent_id: Option<String>,
    /// Whether the chat app's messages reach the agent, judged from the
    /// newest inbound message: `ok` or `failing`; `None` before any message.
    #[serde(default)]
    pub delivery_status: Option<String>,
    /// Stable code of the newest delivery failure (`refused_{status}`,
    /// `undelivered`, `not_received`), never upstream prose.
    #[serde(default)]
    pub delivery_error: Option<String>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub delivery_failed_at: Option<DateTime<Utc>>,
    /// Creation time of the newest inbound message the check has judged.
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub delivery_seen_at: Option<DateTime<Utc>>,
    /// When the sweep last checked this channel (least recent first).
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub delivery_checked_at: Option<DateTime<Utc>>,
    /// When the agent was last told that delivery is failing.
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub delivery_notified_at: Option<DateTime<Utc>>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

impl std::fmt::Debug for NyxbotChannel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NyxbotChannel")
            .field("id", &self.id)
            .field("platform", &self.platform)
            .field("status", &self.status)
            .finish_non_exhaustive()
    }
}

/// One chat partition answered by one NyxBot conversation.
#[derive(Clone, Serialize, Deserialize)]
pub struct NyxbotThread {
    #[serde(rename = "_id")]
    pub id: String,
    pub channel_id: String,
    pub user_id: String,
    /// Gateway conversation ID, or a digest of the direct chat/sender.
    pub partition: String,
    #[serde(default)]
    pub conversation_id: Option<String>,
    /// Gateway only: the newest sealed event reference, encrypted; used to
    /// deliver asynchronous replies while it is unexpired. Opaque, never logged.
    #[serde(default, with = "crate::models::bson_bytes::optional")]
    pub event_ref_ciphertext: Option<Vec<u8>>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub event_ref_expires_at: Option<DateTime<Utc>>,
    /// Direct only: the newest inbound NyxID message ID, for asynchronous replies.
    #[serde(default)]
    pub last_message_id: Option<String>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

impl std::fmt::Debug for NyxbotThread {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NyxbotThread")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

/// Gateway turn admission, keyed by binding and idempotency key. Holds the
/// gateway's `event_context` verbatim (encrypted; it carries message text) so
/// `readEventContext` returns exactly what was admitted. TTL-expired.
#[derive(Clone, Serialize, Deserialize)]
pub struct NyxbotEvent {
    /// SHA-256 of `binding_id` and the idempotency key.
    #[serde(rename = "_id")]
    pub id: String,
    pub channel_id: String,
    pub user_id: String,
    pub partition: String,
    pub event_id: String,
    #[serde(with = "crate::models::bson_bytes::optional", default)]
    pub event_context_ciphertext: Option<Vec<u8>>,
    /// `running`, `completed`, `failed`, or `refused`.
    pub status: String,
    #[serde(default)]
    pub conversation_id: Option<String>,
    #[serde(default)]
    pub turn_id: Option<String>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
}

impl std::fmt::Debug for NyxbotEvent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NyxbotEvent")
            .field("status", &self.status)
            .finish_non_exhaustive()
    }
}

/// Something the owner is finishing outside the chat that a NyxBot or
/// specialist thread is waiting on. NyxID notices when it completes and wakes
/// that thread, so the user never has to come back and say "done".
/// TTL-expired.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NyxbotWatch {
    #[serde(rename = "_id")]
    pub id: String,
    pub user_id: String,
    /// `channel_bot` (a bot created from a setup link) or `connect_link`
    /// (a hosted service connection).
    pub kind: String,
    /// The thread that is waiting and receives the outcome.
    pub conversation_id: String,
    /// `pending`, `claimed`, `done`, or `failed`.
    pub status: String,
    /// channel_bot: canonical platform (`telegram-new` is stored as `telegram`).
    #[serde(default)]
    pub platform: Option<String>,
    /// channel_bot: the agent the new bot is linked to.
    #[serde(default)]
    pub agent_id: Option<String>,
    /// channel_bot: the bot that was linked.
    #[serde(default)]
    pub channel_bot_id: Option<String>,
    /// connect_link: the hosted link being completed.
    #[serde(default)]
    pub connect_link_id: Option<String>,
    /// Stable code only, never upstream prose.
    #[serde(default)]
    pub last_error: Option<String>,
    /// When the sweep last looked at it: sweeps take the least recently
    /// checked first, so old abandoned watches never starve new ones.
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub checked_at: Option<DateTime<Utc>>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
}

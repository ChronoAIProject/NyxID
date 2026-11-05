//! Dedicated-app WhatsApp enrollment, separate from legacy relay credentials.
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "channel_whatsapp_native";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CallbackPhase {
    #[default]
    Relay,
    AwaitingGateway,
    Gateway,
    AwaitingRelay,
    #[serde(other)]
    Unknown,
}

/// Metadata only. Callback URLs/opaque gateway references are never Debug output.
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct CallbackState {
    #[serde(default)]
    pub phase: CallbackPhase,
    #[serde(default)]
    pub gateway_channel_id: Option<String>,
    #[serde(default)]
    pub gateway_webhook_url: Option<String>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub gateway_verified_at: Option<DateTime<Utc>>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub relay_verified_at: Option<DateTime<Utc>>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub transition_started_at: Option<DateTime<Utc>>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub intake_seen_at: Option<DateTime<Utc>>,
    /// The authenticated relay callback observed while native restoration was
    /// pending. This fences native admission before the final route swap.
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub relay_observed_at: Option<DateTime<Utc>>,
    /// Durable retry fence for failed native activation attempts.
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub activation_retry_at: Option<DateTime<Utc>>,
    /// Metadata-only stable failure code shown by the settings surface.
    #[serde(default)]
    pub last_error: Option<String>,
    /// A rollback Verify Token was replaced; plaintext is never persisted.
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub relay_token_replaced_at: Option<DateTime<Utc>>,
}

impl std::fmt::Debug for CallbackState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CallbackState")
            .field("phase", &self.phase)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct NativeWhatsapp {
    /// The channel bot UUID. All lookups use MongoDB's built-in unique _id index.
    #[serde(rename = "_id")]
    pub bot_id: String,
    pub owner_id: String,
    #[serde(default)]
    pub ownership_version: i64,
    /// The real Meta App ID. ChannelBot.app_id is a WABA ID, never this field.
    pub meta_app_id: String,
    pub waba_id: String,
    pub phone_number_id: String,
    /// Owner confirmation; CMA still verifies token/app/scopes/phone membership.
    #[serde(default)]
    pub dedicated_app: bool,
    /// Native CMA callback Verify Token, envelope-encrypted. The relay token
    /// remains hash-only on ChannelBot and is never reused for native setup.
    #[serde(default, with = "crate::models::bson_bytes::optional")]
    pub native_verify_token_encrypted: Option<Vec<u8>>,
    #[serde(default)]
    pub revision: i64,
    #[serde(default)]
    pub callback: CallbackState,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

impl std::fmt::Debug for NativeWhatsapp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeWhatsapp")
            .field("revision", &self.revision)
            .field("callback", &self.callback)
            .finish_non_exhaustive()
    }
}

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "telegram_bot_requests";
pub const MANAGED_BOTS: &str = "telegram_managed_bot_events";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ManagedBotEvents {
    #[serde(rename = "_id")]
    pub id: String,
    pub manager_bot_id: i64,
    pub telegram_bot_id: i64,
    pub telegram_user_id: i64,
    pub revision: i64,
    pub update_ids: Vec<i64>,
    #[serde(default)]
    pub created_by: Option<i64>,
    #[serde(default)]
    pub bot_username: Option<String>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub created_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub observation_id: Option<String>,
    #[serde(default)]
    pub retired: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TelegramRequestStatus {
    WaitingTelegram,
    WaitingBot,
    WaitingConsent,
    Ready,
    Provisioning,
    Connected,
    Cancelled,
    Expired,
    Suspended,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TelegramFailure {
    Network,
    Timeout,
    HttpRejected,
    ProviderRejected,
    InvalidResponse,
    ResponseTooLarge,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TelegramReason {
    RateLimited,
    Unauthorized,
    Forbidden,
    BadRequest,
    ProviderUnavailable,
    Unknown,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TelegramDiagnostic {
    pub failure: TelegramFailure,
    pub http_status: Option<u16>,
    pub error_code: Option<i32>,
    pub reason: TelegramReason,
    pub retry_after: Option<u32>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptPhase {
    Instructions,
    Keyboard,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PromptOutcome {
    Sending,
    Accepted,
    Rejected,
    Unknown,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PromptDelivery {
    pub attempt_id: String,
    pub update_id: i64,
    pub phase: PromptPhase,
    pub outcome: PromptOutcome,
    /// Start of the current dispatch phase, renewed before each provider call.
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub attempted_at: DateTime<Utc>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub completed_at: Option<DateTime<Utc>>,
    pub diagnostic: Option<TelegramDiagnostic>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CreationContext {
    Standalone,
    Hosted {
        link_id: String,
        link_created_by_user_id: String,
        #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
        link_created_at: DateTime<Utc>,
        watch: Option<Box<CreationWatch>>,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CreationWatch {
    pub id: String,
    pub user_id: String,
    pub kind: String,
    pub platform: Option<String>,
    pub agent_id: Option<String>,
    pub conversation_id: String,
    pub invocation_id: Option<String>,
    pub report_to: Option<String>,
    pub report_provenance: u8,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct TelegramBotRequest {
    #[serde(rename = "_id")]
    pub id: String,
    #[serde(default)]
    pub setup_authority: Option<super::platform_installation::SetupStepAuthority>,
    #[serde(default)]
    pub connection_generation: i64,
    pub actor_user_id: String,
    pub owner_user_id: String,
    pub manager_bot_id: i64,
    pub observation_id: String,
    pub label: String,
    pub destination: String,
    pub status: TelegramRequestStatus,
    pub active: bool,
    pub revision: i64,
    pub challenge_hash: String,
    pub telegram_user_id: Option<i64>,
    pub telegram_bot_id: Option<i64>,
    pub bot_username: Option<String>,
    pub consent_hash: Option<String>,
    pub manager_revision: Option<i64>,
    #[serde(default)]
    pub auto_connect: bool,
    #[serde(default)]
    pub start_update_id: Option<i64>,
    #[serde(default)]
    pub connection_attempts: u32,
    #[serde(default)]
    pub prompt_delivery: Option<PromptDelivery>,
    #[serde(default)]
    pub creation_context: Option<CreationContext>,
    #[serde(default)]
    pub prompt_update_boundary: Option<i64>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub next_connection_attempt_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub connection_error: Option<String>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub purge_after: Option<DateTime<Utc>>,
}

impl std::fmt::Debug for TelegramBotRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TelegramBotRequest")
            .field("id", &self.id)
            .field("status", &self.status)
            .finish_non_exhaustive()
    }
}

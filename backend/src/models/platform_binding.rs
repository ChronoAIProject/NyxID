use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "platform_onboarding_bindings";
pub const REQUESTS: &str = "platform_onboarding_binding_requests";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BindingRequestState {
    Pending,
    AwaitingChatConfirmation,
    Bound,
    Denied,
    Expired,
    Cancelled,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct PlatformBindingRequest {
    #[serde(rename = "_id")]
    pub id: String,
    pub platform_config_id: String,
    pub manager_bot_id: String,
    pub profile_generation: i64,
    pub telegram_sender_id: String,
    pub telegram_chat_id: String,
    pub code_hmac: String,
    #[serde(default)]
    pub confirmation_hmac: Option<String>,
    pub initiation_ref: i64,
    pub state: BindingRequestState,
    pub approved_user_id: Option<String>,
    pub replace_existing: bool,
    pub binding_id: Option<String>,
    pub binding_generation_at_review: Option<i64>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub purge_after: DateTime<Utc>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct PlatformBinding {
    #[serde(rename = "_id")]
    pub id: String,
    pub platform_config_id: String,
    pub manager_bot_id: String,
    pub profile_generation: i64,
    pub telegram_sender_id: String,
    pub telegram_chat_id: String,
    pub user_id: String,
    pub generation: i64,
    #[serde(default)]
    pub revocation_generation: i64,
    pub is_active: bool,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
}

impl std::fmt::Debug for PlatformBindingRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlatformBindingRequest")
            .field("id", &self.id)
            .field("state", &self.state)
            .finish_non_exhaustive()
    }
}
impl std::fmt::Debug for PlatformBinding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlatformBinding")
            .field("generation", &self.generation)
            .field("is_active", &self.is_active)
            .finish_non_exhaustive()
    }
}

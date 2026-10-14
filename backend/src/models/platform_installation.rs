use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "platform_bot_installations";
pub const STEPS: &str = "platform_setup_steps";
pub const NOTICES: &str = "platform_setup_notices";
pub const TESTS: &str = "platform_delivery_tests";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SetupStep {
    BeginTelegram,
    ConnectTelegram,
}

/// Server-resolved authority; never accepted from a model or ordinary connect input.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SetupStepAuthority {
    pub setup_id: String,
    pub owner_user_id: String,
    pub setup_revision: i64,
    pub digest: String,
    pub authority_generation: i64,
    pub platform_config_id: String,
    pub profile_generation: i64,
    pub binding_id: String,
    pub binding_generation: i64,
    #[serde(default)]
    pub binding_revocation_generation: i64,
    pub manager_bot_id: i64,
    pub telegram_sender_id: i64,
    pub step: SetupStep,
    pub approved_child_bot_id: Option<i64>,
    pub approved_child_revision: Option<i64>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NativeCreationProof {
    pub observation_id: String,
    pub sender_id: i64,
    pub start_update_id: i64,
    pub child_revision: i64,
    pub child_username: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub request_created_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PlatformInstallation {
    #[serde(rename = "_id")]
    pub id: String,
    pub setup_id: String,
    pub owner_user_id: String,
    pub channel_bot_id: String,
    pub agent_id: String,
    pub telegram_bot_id: Option<String>,
    #[serde(default)]
    pub native_creation: Option<NativeCreationProof>,
    pub generation: i64,
    pub state: String,
    pub authority: SetupStepAuthority,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SetupNotice {
    #[serde(rename = "_id")]
    pub id: String,
    pub setup_id: String,
    pub revision: i64,
    pub authority_generation: i64,
    pub kind: String,
    pub binding_id: String,
    pub binding_generation: i64,
    pub profile_id: String,
    pub profile_generation: i64,
    pub completed: bool,
    pub attempts: u32,
    pub claim_id: Option<String>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub retry_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct DeliveryTest {
    #[serde(rename = "_id")]
    pub id: String,
    pub installation_id: String,
    pub installation_generation: i64,
    pub setup_revision: i64,
    pub digest: String,
    pub customer_sender_id: String,
    pub customer_chat_id: String,
    pub challenge_hmac: String,
    pub reply_message_id: Option<String>,
    pub state: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
}
impl std::fmt::Debug for DeliveryTest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DeliveryTest { [REDACTED] }")
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SetupStepJob {
    #[serde(rename = "_id")]
    pub id: String,
    pub setup_id: String,
    pub authority: SetupStepAuthority,
    pub phase: String,
    pub generation: i64,
    pub claim_id: Option<String>,
    pub attempts: u32,
    pub error_code: Option<String>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub retry_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
}

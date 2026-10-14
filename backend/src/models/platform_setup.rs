use super::credits::Credits;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "platform_setups";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum SetupTemplate {
    TelegramAssistant,
    RestaurantStarter,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SetupState {
    Draft,
    AwaitingIdentity,
    Collecting,
    AwaitingApproval,
    Provisioning,
    AwaitingChildReview,
    ReadyForTest,
    Active,
    FailedRetryable,
    Paused,
    Cancelled,
    Expired,
}

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct FaqEntry {
    pub question: String,
    pub answer: String,
}

#[derive(Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct SetupFacts {
    pub name: String,
    pub timezone: String,
    #[serde(default)]
    pub hours: String,
    #[serde(default)]
    pub address: String,
    #[serde(default)]
    pub menu_url: Option<String>,
    #[serde(default)]
    pub reservation_url: Option<String>,
    #[serde(default)]
    pub contact: String,
    #[serde(default)]
    pub policies: String,
    #[serde(default)]
    pub faq: Vec<FaqEntry>,
    #[serde(default)]
    pub assistant_instructions: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeLimits {
    #[serde(with = "crate::models::credits::whole")]
    pub daily_credits: Credits,
    pub turns_per_hour: u32,
    pub concurrent_turns: u32,
    pub max_input_chars: u32,
    pub max_output_tokens: u32,
}

#[derive(Clone, Default, Debug, Serialize, Deserialize)]
pub struct SetupResources {
    #[serde(default)]
    pub observed_telegram_bot_id: Option<String>,
    #[serde(default)]
    pub runtime_model_service_id: Option<String>,
    #[serde(default)]
    pub runtime_model_id: Option<String>,
    pub agent_id: Option<String>,
    pub creation_request_id: Option<String>,
    pub channel_bot_id: Option<String>,
    pub approved_telegram_bot_id: Option<String>,
    pub observed_creation_revision: Option<i64>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct PublishedSetupContent {
    pub revision: i64,
    pub facts: SetupFacts,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub published_at: DateTime<Utc>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct PlatformSetup {
    #[serde(rename = "_id")]
    pub id: String,
    pub owner_user_id: Option<String>,
    pub initiating_actor_id: Option<String>,
    pub platform_config_id: Option<String>,
    pub binding_id: Option<String>,
    pub binding_generation: Option<i64>,
    pub profile_generation: Option<i64>,
    pub source: String,
    pub idempotency_key: String,
    pub idempotency_scope: String,
    pub template: SetupTemplate,
    pub template_version: i64,
    pub revision: i64,
    pub authority_generation: i64,
    pub state: SetupState,
    #[serde(with = "crate::models::bson_bytes::required")]
    pub facts_ciphertext: Vec<u8>,
    pub limits: RuntimeLimits,
    pub resources: SetupResources,
    pub approved_digest: Option<String>,
    pub approved_revision: Option<i64>,
    pub approved_by: Option<String>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub approved_at: Option<DateTime<Utc>>,
    pub published: Option<PublishedSetupContent>,
    pub delivery_tested: bool,
    pub delivery_test_message_id: Option<String>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub expires_at: Option<DateTime<Utc>>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub purge_after: Option<DateTime<Utc>>,
}

impl std::fmt::Debug for PlatformSetup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlatformSetup")
            .field("id", &self.id)
            .field("state", &self.state)
            .field("revision", &self.revision)
            .finish_non_exhaustive()
    }
}

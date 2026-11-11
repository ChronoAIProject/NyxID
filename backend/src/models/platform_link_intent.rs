use super::platform_entry_projection::{
    StoredEntryIdentity, StoredSenderIdentity, checked_identity_record, same_version,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "platform_personal_link_intents";
pub const CHALLENGES: &str = "platform_personal_link_challenges";
pub const PURPOSE: &str = "personal_nyxbot_v1";
pub const CONSENT_VERSION: u32 = 1;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntentState {
    Pending,
    Consumed,
    Cancelled,
    Expired,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChallengeState {
    Review,
    AwaitingChatConfirmation,
    Confirmed,
    Denied,
    Cancelled,
    Expired,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersonalBindingCompletion {
    pub binding_id: String,
    pub binding_generation: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewedPersonalBinding {
    pub binding_id: String,
    pub generation: i64,
    pub user_id: String,
    pub state: super::platform_personal_binding::PersonalBindingState,
}

checked_identity_record! {
#[derive(Clone, Serialize)]
pub struct PlatformLinkIntent, RawPlatformLinkIntent {
    #[serde(rename = "_id")]
    pub id: String,
    pub purpose: String,
    pub platform_config_id: String,
    pub manager_bot_id: String,
    #[serde(flatten)]
    pub entry: StoredEntryIdentity,
    pub manager_authority_epoch: i64,
    #[serde(flatten)]
    pub sender: StoredSenderIdentity,
    pub initiation_ref: String,
    pub token_lookup_hmac: String,
    pub token_hmac: String,
    pub state: IntentState,
    pub claimant_user_id: Option<String>,
    pub current_challenge_id: Option<String>,
    pub completion: Option<PersonalBindingCompletion>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub purge_after: DateTime<Utc>,
}
|record| same_version(&record.entry, &record.sender, None) && super::platform_entry_projection::entry_matches(&record.entry, &record.manager_bot_id)
}

checked_identity_record! {
#[derive(Clone, Serialize)]
pub struct PlatformLinkChallenge, RawPlatformLinkChallenge {
    #[serde(rename = "_id")]
    pub id: String,
    pub intent_id: String,
    pub purpose: String,
    pub platform_config_id: String,
    pub manager_bot_id: String,
    #[serde(flatten)]
    pub entry: StoredEntryIdentity,
    pub manager_authority_epoch: i64,
    #[serde(flatten)]
    pub sender: StoredSenderIdentity,
    pub reviewed_user_id: String,
    pub reviewed_access_mode: String,
    pub consent_version: u32,
    pub review_digest: String,
    pub previous_binding: Option<ReviewedPersonalBinding>,
    pub state: ChallengeState,
    pub code_lookup_hmac: Option<String>,
    pub code_hmac: Option<String>,
    pub completion: Option<PersonalBindingCompletion>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub purge_after: DateTime<Utc>,
}
|record| same_version(&record.entry, &record.sender, None) && super::platform_entry_projection::entry_matches(&record.entry, &record.manager_bot_id)
}

impl std::fmt::Debug for PlatformLinkIntent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlatformLinkIntent")
            .field("state", &self.state)
            .finish_non_exhaustive()
    }
}
impl std::fmt::Debug for PlatformLinkChallenge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlatformLinkChallenge")
            .field("state", &self.state)
            .finish_non_exhaustive()
    }
}

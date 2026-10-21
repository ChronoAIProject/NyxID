use super::platform_entry_projection::{
    StoredEntryIdentity, StoredSenderIdentity, checked_identity_record, same_version,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "platform_personal_bindings";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PersonalBindingState {
    Active,
    Revoked,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PersonalRevocationReason {
    Unlinked,
    Replaced,
    OwnerDisabled,
    OwnerDeleted,
    ManagerRetired,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PersonalOriginSource {
    Ingress {
        receipt_id: String,
        claim_id: String,
    },
    ThreadEvent {
        admission_id: String,
    },
}

checked_identity_record! {
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PersonalTurnProof, RawPersonalTurnProof {
    pub platform_config_id: String,
    pub manager_bot_id: String,
    #[serde(flatten)]
    pub entry: StoredEntryIdentity,
    pub manager_authority_epoch: i64,
    #[serde(flatten)]
    pub sender: StoredSenderIdentity,
    pub binding_id: String,
    pub binding_generation: i64,
    pub personal_control_revision: i64,
    pub user_id: String,
    pub nyxbot_agent_id: String,
    pub source: PersonalOriginSource,
}
|record| same_version(&record.entry, &record.sender, None) && super::platform_entry_projection::entry_matches(&record.entry, &record.manager_bot_id)
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PersonalActiveTurn {
    pub conversation_id: String,
    pub turn_id: String,
    pub origin: PersonalTurnProof,
}

checked_identity_record! {
#[derive(Clone, Serialize)]
pub struct PlatformPersonalBinding, RawPlatformPersonalBinding {
    #[serde(rename = "_id")]
    pub id: String,
    pub platform_config_id: String,
    pub manager_bot_id: String,
    #[serde(flatten)]
    pub entry: StoredEntryIdentity,
    pub manager_authority_epoch: i64,
    #[serde(flatten)]
    pub sender: StoredSenderIdentity,
    pub user_id: String,
    pub generation: i64,
    pub state: PersonalBindingState,
    pub revocation_reason: Option<PersonalRevocationReason>,
    pub consent_version: u32,
    pub review_digest: String,
    pub reviewed_access_mode: String,
    pub nyxbot_agent_id: Option<String>,
    pub conversation_id: Option<String>,
    pub active_turn: Option<PersonalActiveTurn>,
    pub effect_revision: i64,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub consented_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
    #[serde(with = "crate::models::bson_datetime::optional")]
    pub revoked_at: Option<DateTime<Utc>>,
    /// A sender can explicitly return to public chat after owner retirement.
    /// This does not replace the original revocation reason or restore authority.
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub logged_out_at: Option<DateTime<Utc>>,
}
|record| same_version(&record.entry, &record.sender, None) && super::platform_entry_projection::entry_matches(&record.entry, &record.manager_bot_id)
}

impl std::fmt::Debug for PlatformPersonalBinding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlatformPersonalBinding")
            .field("state", &self.state)
            .field("generation", &self.generation)
            .finish_non_exhaustive()
    }
}

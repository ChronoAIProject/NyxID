//! Durable adapter metadata. Credentials and transcript bodies belong to existing stores.
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const BINDINGS: &str = "channel_adapter_bindings";
pub const ALLOCATIONS: &str = "channel_adapter_allocations";
pub const SESSIONS: &str = "channel_adapter_sessions";
pub const ACCOUNTS: &str = "channel_adapter_accounts";
pub const EVENTS: &str = "channel_adapter_events";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Profile {
    TelnyxRelay,
    TelnyxSms,
    TwilioRelay,
    TwilioSms,
    ElevenlabsCustomLlm,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum CredentialBinding {
    Connection { user_service_id: String },
    Platform { catalog_service_id: String },
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Binding {
    #[serde(rename = "_id")]
    pub id: String,
    pub user_id: String,
    pub profile: Profile,
    pub contract_version: u32,
    pub revision: i64,
    pub label: String,
    pub agent_id: String,
    pub credential: CredentialBinding,
    pub account_id: String,
    pub resource_id: String,
    pub phone_number: String,
    pub application_id: String,
    pub webhook_public_key: Option<String>,
    pub voice: String,
    pub language: String,
    pub enabled: bool,
    pub verified_credential_identity: String,
    #[serde(default)]
    pub trusted_account_revision: Option<i64>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub verified_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ResourceAllocation {
    #[serde(rename = "_id")]
    pub id: String,
    pub catalog_service_id: String,
    pub user_id: String,
    pub provider: String,
    pub account_id: String,
    pub resource_id: String,
    pub phone_number: String,
    pub application_id: String,
    pub allocated_by: String,
    pub revision: i64,
    pub tariff_version: String,
    pub tariff_source: String,
    pub includes_all_provider_charges: bool,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub tariff_verified_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    Prepared,
    Active,
    Closed,
    Uncertain,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct CallSession {
    #[serde(rename = "_id")]
    pub id: String,
    pub identity_key: String,
    pub user_id: String,
    pub binding_id: String,
    pub binding_revision: i64,
    pub profile: Profile,
    pub contract_version: u32,
    pub credential_identity: String,
    pub provider_call_id: String,
    pub provider_session_id: Option<String>,
    pub from: String,
    pub to: String,
    pub conversation_id: String,
    pub state: SessionState,
    pub claim_id: Option<String>,
    #[serde(default)]
    pub answered: bool,
    #[serde(default)]
    pub answer_dispatched: bool,
    #[serde(default)]
    pub relay_dispatched: bool,
    #[serde(default)]
    pub hangup_dispatched: bool,
    #[serde(default)]
    pub cap_dispatched: bool,
    #[serde(default)]
    pub cap_confirmed: bool,
    #[serde(default)]
    pub bootstrap_dispatched: bool,
    pub generation: i64,
    pub next_utterance: i64,
    #[serde(default)]
    pub custom_history: Vec<String>,
    pub active_turn_id: Option<String>,
    #[serde(default)]
    pub reply_turn_id: Option<String>,
    pub socket_ticket_hash: Option<String>,
    pub billing_request_id: String,
    pub observed_seconds: i64,
    /// Immutable terminal evidence; settlement retries use this quantity.
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub ended_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub terminal_evidence: Option<String>,
    pub billing_finalized: bool,
    pub delivery: String,
    #[serde(default)]
    pub heard_prefix_chars: Option<i64>,
    #[serde(default)]
    pub heard_prefix_matches_sent: bool,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "crate::models::bson_datetime::optional", default)]
    pub connected_at: Option<DateTime<Utc>>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub lease_expires_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct EventReceipt {
    #[serde(rename = "_id")]
    pub id: String,
    pub identity_key: String,
    pub user_id: String,
    pub binding_id: String,
    pub binding_revision: i64,
    pub session_id: Option<String>,
    pub event_kind: String,
    pub status: String,
    pub provider_message_id: Option<String>,
    pub delivery: String,
    pub billing_request_id: Option<String>,
    pub observed_parts: Option<i64>,
    #[serde(default)]
    pub reserved_parts: Option<i64>,
    #[serde(default)]
    pub billing_finalized: bool,
    #[serde(default)]
    pub from: Option<String>,
    #[serde(default)]
    pub to: Option<String>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
}

/// Platform account trust is writable only by a platform administrator.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ProviderAccount {
    #[serde(rename = "_id")]
    pub id: String,
    pub catalog_service_id: String,
    pub provider: String,
    pub account_id: String,
    pub webhook_public_key: Option<String>,
    pub revision: i64,
    pub configured_by: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

impl std::fmt::Debug for Binding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AdapterBinding")
            .field("id", &self.id)
            .field("profile", &self.profile)
            .field("revision", &self.revision)
            .finish_non_exhaustive()
    }
}
impl std::fmt::Debug for CallSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AdapterSession")
            .field("id", &self.id)
            .field("state", &self.state)
            .field("generation", &self.generation)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TurnAuthority {
    pub binding_id: String,
    pub session_id: String,
    pub binding_revision: i64,
    pub generation: i64,
    pub credential_identity: String,
}
impl std::fmt::Debug for TurnAuthority {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ChannelTurnAuthority { [REDACTED] }")
    }
}

/// Provider-global resource IDs prevent account aliases or profile changes from
/// allowing another owner to take over a number or its application.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ResourceClaim {
    #[serde(rename = "_id")]
    pub id: String,
    pub identity_key: String,
    #[serde(default)]
    pub revision: i64,
    pub user_id: String,
    pub provider: String,
    pub resource_kind: String,
    pub resource_id: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
}
pub const CLAIMS: &str = "channel_adapter_resource_claims";

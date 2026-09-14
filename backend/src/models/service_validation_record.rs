use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "service_validation_records";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ValidationOutcome {
    Authenticated,
    PermissionDenied,
    CredentialRejected,
    ConfigurationError,
    BillingBlocked,
    RateLimited { retry_after: Option<Duration> },
    TransportUnknown,
    Unsupported,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CallerContext {
    Human { session: String },
    App { client_id: String },
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ServiceValidationRecord {
    #[serde(rename = "_id")]
    pub id: String,
    pub user_service_id: String,
    pub owner_id: String,
    pub validator_id: String,
    pub validator_version: u32,
    pub execution_authority_digest: String,
    pub api_key_id: Option<String>,
    pub credential_epoch: Option<i64>,
    pub attempt_id: String,
    pub completed: bool,
    pub credential_revision: Option<String>,
    pub reason_code: String,
    pub outcome: ValidationOutcome,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub checked_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub valid_until: DateTime<Utc>,
    pub caller_context: CallerContext,
}

impl std::fmt::Debug for ServiceValidationRecord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ServiceValidationRecord")
            .field("validator_id", &self.validator_id)
            .field("validator_version", &self.validator_version)
            .field("outcome", &self.outcome)
            .field("checked_at", &self.checked_at)
            .field("valid_until", &self.valid_until)
            .finish_non_exhaustive()
    }
}

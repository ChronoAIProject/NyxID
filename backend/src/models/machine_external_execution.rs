//! Typed external rows share the Machine context, lease and revocation stores.
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const CONFIRMATIONS: &str = "machine_confirmations";

#[derive(Clone, Deserialize, Serialize)]
pub struct ExternalLease {
    #[serde(rename = "_id")]
    pub id: String,
    pub kind: String,
    pub node_id: String,
    pub job_id: Option<String>,
    pub external_authority: Box<nyxid_machine::authority::ExternalAuthority>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct ExternalContext {
    #[serde(rename = "_id")]
    pub id: String,
    pub kind: String,
    pub node_id: String,
    pub owner_id: String,
    pub grant_id: String,
    pub generation: i64,
    pub mode: String,
}

/// No command, bearer or output is retained. The canonical digest commits to
/// the exact operation, source child, grant and concrete runtime.
#[derive(Clone, Deserialize, Serialize)]
pub struct ExternalConfirmation {
    #[serde(rename = "_id")]
    pub id: String,
    pub user_id: String,
    pub node_id: String,
    pub principal: super::machine_external_grant::ExternalPrincipalRef,
    /// Exact verified source at request time. Missing legacy fields fail closed
    /// at decision/consumption instead of borrowing the owner's credentials.
    #[serde(default)]
    pub protected_consent: Option<super::consent::ProtectedConsentRef>,
    #[serde(default, with = "super::bson_datetime::optional")]
    pub authentication_expires_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub service_ceiling: Vec<String>,
    pub grant_id: String,
    pub grant_revision: i64,
    pub assignment_revision: i64,
    pub runtime_id: String,
    pub runtime_generation: Option<u64>,
    pub credential_epoch: Option<u64>,
    pub operation: nyxid_machine::Operation,
    pub parameters_digest: String,
    /// Server-selected non-secret metadata only. Never command/input/content.
    #[serde(default)]
    pub safe_parameters: Vec<ConfirmationReviewParameter>,
    pub binding_digest: String,
    pub state: String,
    pub job_id: Option<String>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
    #[serde(default, with = "super::bson_datetime::optional")]
    pub decided_at: Option<DateTime<Utc>>,
    #[serde(default, with = "super::bson_datetime::optional")]
    pub consumed_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct ConfirmationReviewParameter {
    pub name: String,
    pub value: String,
}

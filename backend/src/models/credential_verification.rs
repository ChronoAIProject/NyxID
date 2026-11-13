use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Trusted catalog assertion: this published GET authenticates the supplied key.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, utoipa::ToSchema)]
#[serde(deny_unknown_fields)]
pub struct CredentialVerificationCheck {
    pub operation: String,
    #[serde(default)]
    pub query: std::collections::BTreeMap<String, String>,
}

/// Operational metadata on the connection, never credential material or a body.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CredentialVerificationObservation {
    pub binding: String,
    pub state: String,
    pub reason_code: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub started_at: DateTime<Utc>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub observed_at: Option<DateTime<Utc>>,
}

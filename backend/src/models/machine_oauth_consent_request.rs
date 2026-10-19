use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "machine_oauth_consent_requests";

/// One-time owner selection within the existing signed OAuth consent protocol.
/// No token secret is retained; the digest binds the complete signed request.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MachineOauthConsentRequest {
    #[serde(rename = "_id")]
    pub id: String,
    pub user_id: String,
    pub client_id: String,
    pub signed_request_digest: String,
    pub previous_consent_fingerprint: Option<String>,
    /// Protected source versions captured by the server at owner selection.
    /// Missing versions are intentionally not defaulted into fresh authority.
    pub user_security_revision: i64,
    pub client_security_revision: i64,
    pub selection: bson::Document,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub consumed_at: Option<DateTime<Utc>>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
}

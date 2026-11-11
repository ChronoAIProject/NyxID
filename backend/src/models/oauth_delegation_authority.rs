use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::consent::ProtectedConsentRef;

pub const COLLECTION_NAME: &str = "oauth_delegation_authorities";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConsentBinding {
    pub client_id: String,
    /// OAuth delegation requires openid permission, independently of the
    /// receiving client's configured execution/delegation scopes.
    pub scopes: String,
    pub reference: ProtectedConsentRef,
}

/// Original OAuth grants/session retained by derived service callback tokens.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OauthAuthorityLineage {
    #[serde(default)]
    pub delegation_jti: Option<String>,
    pub consents: Vec<ConsentBinding>,
    pub source_session_id: Option<String>,
}

/// Short-lived online authority for RFC 8693 exchange. No bearer material is
/// stored. Refresh carries these original bindings rather than a current grant.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OauthDelegationAuthority {
    #[serde(rename = "_id")]
    pub id: String,
    pub user_id: String,
    pub actor_client_id: String,
    pub receiving_client_id: Option<String>,
    pub scope: String,
    pub claims_fingerprint: String,
    pub consents: Vec<ConsentBinding>,
    pub source_session_id: Option<String>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
}

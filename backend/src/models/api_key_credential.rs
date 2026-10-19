use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;

use super::bson_datetime;
use crate::redaction::RedactedLen;

pub const COLLECTION_NAME: &str = "api_key_credentials";

#[derive(Clone, Copy, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ApiKeyCredentialKind {
    #[default]
    AgentKeyLogin,
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq, utoipa::ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum CredentialRevokedReason {
    Logout,
    WebRevoke,
    ParentRevoked,
    ParentRotated,
    UndeliveredExpired,
}

#[derive(Clone, Deserialize, Serialize)]
pub struct ApiKeyCredential {
    #[serde(rename = "_id")]
    pub id: String,
    pub api_key_id: String,
    pub user_id: String,
    pub secret_hash: String,
    pub secret_prefix: String,
    #[serde(default)]
    pub kind: ApiKeyCredentialKind,
    pub label: String,
    pub login_request_id: String,
    pub is_active: bool,
    #[serde(default, with = "bson_datetime::optional")]
    pub revoked_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub revoked_reason: Option<CredentialRevokedReason>,
    #[serde(default, with = "bson_datetime::optional")]
    pub expires_at: Option<DateTime<Utc>>,
    #[serde(default, with = "bson_datetime::optional")]
    pub last_used_at: Option<DateTime<Utc>>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
}

impl fmt::Debug for ApiKeyCredential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ApiKeyCredential")
            .field("id", &RedactedLen(self.id.len()))
            .field("api_key_id", &RedactedLen(self.api_key_id.len()))
            .field("secret_hash", &RedactedLen(self.secret_hash.len()))
            .field("secret_prefix", &RedactedLen(self.secret_prefix.len()))
            .field("kind", &self.kind)
            .field("is_active", &self.is_active)
            .field("revoked_reason", &self.revoked_reason)
            .field("expires_at", &self.expires_at)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bson::doc;

    #[test]
    fn bson_dates_legacy_defaults_and_secret_redaction() {
        let now = bson::DateTime::now();
        let mut row: ApiKeyCredential = bson::from_document(doc! {
            "_id": "credential", "api_key_id": "parent", "user_id": "owner",
            "secret_hash": "private-hash", "secret_prefix": "nyxid_ag_private",
            "label": "workstation", "login_request_id": "request", "is_active": true,
            "created_at": now,
        })
        .unwrap();
        assert_eq!(row.kind, ApiKeyCredentialKind::AgentKeyLogin);
        assert!(row.expires_at.is_none() && row.revoked_at.is_none() && row.last_used_at.is_none());
        row.expires_at = Some(now.to_chrono());
        row.revoked_at = Some(now.to_chrono());
        row.last_used_at = Some(now.to_chrono());
        row.revoked_reason = Some(CredentialRevokedReason::Logout);
        let document = bson::to_document(&row).unwrap();
        for field in ["created_at", "expires_at", "revoked_at", "last_used_at"] {
            assert_eq!(document.get_datetime(field).unwrap(), &now);
        }
        assert_eq!(document.get_str("secret_hash").unwrap(), "private-hash");
        let restored: ApiKeyCredential = bson::from_document(document).unwrap();
        assert_eq!(restored.revoked_reason, row.revoked_reason);
        let debug = format!("{restored:?}");
        assert!(!debug.contains("private-hash") && !debug.contains("nyxid_ag_private"));
    }
}

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::{bson_datetime, login_client_context::LoginClientContext, login_grant::Selection};

pub const COLLECTION_NAME: &str = "login_codes";

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LoginCodeStatus {
    Pending,
    Redeemed,
    Cancelled,
    Revoked,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct LoginCode {
    #[serde(rename = "_id")]
    pub id: String,
    pub user_id: String,
    pub code_hmac: String,
    pub status: LoginCodeStatus,
    pub selection: Option<Selection>,
    #[serde(default, with = "bson_datetime::optional")]
    pub credential_expires_at: Option<DateTime<Utc>>,
    // Browser handoffs refer to an already approved restricted device grant.
    pub device_request_id: Option<String>,
    pub session_id: Option<String>,
    pub credential_id: Option<String>,
    pub context: Option<LoginClientContext>,
    #[serde(default, with = "bson_datetime::optional")]
    pub redeemed_at: Option<DateTime<Utc>>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub purge_at: DateTime<Utc>,
}

impl std::fmt::Debug for LoginCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoginCode")
            .field("status", &self.status)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_redacts_codes_and_credential_identifiers() {
        let now = Utc::now();
        let row = LoginCode {
            id: "fixture-request-id".into(),
            user_id: "fixture-owner-id".into(),
            code_hmac: "fixture-secret-code-hmac".into(),
            status: LoginCodeStatus::Pending,
            selection: None,
            credential_expires_at: None,
            device_request_id: None,
            session_id: Some("fixture-session-id".into()),
            credential_id: Some("fixture-credential-id".into()),
            context: None,
            redeemed_at: None,
            created_at: now,
            expires_at: now,
            purge_at: now,
        };
        let debug = format!("{row:?}");
        assert!(debug.contains("Pending"));
        assert!(!debug.contains("fixture-"));
        let document = bson::to_document(&row).unwrap();
        assert_eq!(document.get_str("code_hmac").unwrap(), row.code_hmac);
        assert!(document.get_datetime("expires_at").is_ok());
    }
}

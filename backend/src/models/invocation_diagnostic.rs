use chrono::{DateTime, Utc};
use mongodb::bson::Document;
use serde::{Deserialize, Serialize};

use super::bson_datetime;
use crate::redaction::RedactedLen;

pub const COLLECTION_NAME: &str = "invocation_diagnostics";

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum InvocationDiagnosticState {
    Pending,
    Running,
    Completed,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct InvocationDiagnostic {
    #[serde(rename = "_id")]
    pub id: String,
    pub handle_hash: String,
    pub actor_user_id: String,
    pub original_auth_method: String,
    pub original_api_key_id: Option<String>,
    pub original_client_id: Option<String>,
    pub exact_user_service_id: String,
    pub state: InvocationDiagnosticState,
    pub diagnostic: Option<Document>,
    pub claim_id: Option<String>,
    #[serde(default, with = "bson_datetime::optional")]
    pub claimed_at: Option<DateTime<Utc>>,
    #[serde(default, with = "bson_datetime::optional")]
    pub completed_at: Option<DateTime<Utc>>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
}

impl std::fmt::Debug for InvocationDiagnostic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InvocationDiagnostic")
            .field("id", &self.id)
            .field("handle_hash", &RedactedLen(self.handle_hash.len()))
            .field("state", &self.state)
            .field("created_at", &self.created_at)
            .field("expires_at", &self.expires_at)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_does_not_expose_bound_identifiers_or_hashes() {
        let now = Utc::now();
        let row = InvocationDiagnostic {
            id: uuid::Uuid::new_v4().to_string(),
            handle_hash: "secret-hash".into(),
            actor_user_id: "secret-actor".into(),
            original_auth_method: "access_token".into(),
            original_api_key_id: Some("secret-key".into()),
            original_client_id: Some("secret-client".into()),
            exact_user_service_id: "secret-service".into(),
            state: InvocationDiagnosticState::Pending,
            diagnostic: None,
            claim_id: None,
            claimed_at: None,
            completed_at: None,
            created_at: now,
            expires_at: now + chrono::Duration::seconds(60),
        };
        let debug = format!("{row:?}");
        assert!(debug.contains("Pending"));
        assert!(!debug.contains("secret-"));
    }
}

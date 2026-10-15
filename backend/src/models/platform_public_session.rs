use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "platform_public_sessions";
pub const CLEANUP_RECORDS: &str = "platform_public_cleanup_records";
pub const READINESS_RECORDS: &str = "platform_public_readiness_records";
pub const REVOCATION_SCOPES: &str = "platform_public_revocation_scopes";

/// Public context never belongs to an assistant owner conversation. The key
/// binding is immutable; revoked rows remain until exact cleanup is confirmed.
#[derive(Clone, Serialize, Deserialize)]
pub struct PlatformPublicSession {
    #[serde(rename = "_id")]
    pub id: String,
    pub profile_id: String,
    pub manager_bot_id: String,
    pub telegram_bot_id: i64,
    pub manager_authority_epoch: i64,
    pub sender_id: i64,
    pub chat_id: i64,
    pub sponsor_owner_user_id: String,
    pub public_policy_revision: i64,
    pub policy_digest: String,
    pub nyxagent_profile: String,
    pub credential_api_key_id: String,
    #[serde(with = "crate::models::bson_bytes::required")]
    pub credential_ciphertext: Vec<u8>,
    #[serde(with = "crate::models::bson_bytes::required")]
    pub upstream_binding_ciphertext: Vec<u8>,
    /// Set durably before dispatch, including an attempt that yields no handle.
    /// Unknown legacy evidence is conservative and cannot imply erasure.
    #[serde(default = "default_true")]
    pub upstream_may_exist: bool,
    pub active_turn: Option<PublicTurnBinding>,
    pub effect_revision: i64,
    pub cleanup_record_id: Option<String>,
    pub cleanup_complete: bool,
    pub revocation_reason: Option<String>,
    #[serde(with = "crate::models::bson_datetime::optional")]
    pub revoked_at: Option<DateTime<Utc>>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    /// Absolute lifetime; a request or upstream touch never extends this field.
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
}

fn default_true() -> bool {
    true
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PublicTurnBinding {
    pub conversation_id: String,
    pub turn_id: String,
    pub receipt_id: String,
    pub claim_id: String,
    pub stop_requested: bool,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct PublicCleanupRecord {
    #[serde(rename = "_id")]
    pub id: String,
    pub session_ids: Vec<String>,
    #[serde(default)]
    pub scope_marker_id: Option<String>,
    pub reason: String,
    pub completed: bool,
    pub claim_id: Option<String>,
    pub attempts: i64,
    pub blocked_reason: Option<String>,
    #[serde(with = "crate::models::bson_datetime::optional")]
    pub lease_expires_at: Option<DateTime<Utc>>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub retry_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
}

/// A scope fence is written by both local admission and revocation. A revoked
/// cutoff denies old sessions immediately while keys are drained in pages.
#[derive(Clone, Serialize, Deserialize)]
pub struct PublicRevocationMarker {
    #[serde(rename = "_id")]
    pub id: String,
    pub scope_key: String,
    pub session_filter: bson::Document,
    pub maximum_epoch: Option<i64>,
    #[serde(with = "crate::models::bson_datetime::optional")]
    pub revoked_before: Option<DateTime<Utc>>,
    pub drain_pending: bool,
    pub reason: String,
    pub effect_revision: i64,
}

impl std::fmt::Debug for PublicRevocationMarker {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PublicRevocationMarker")
            .field("drain_pending", &self.drain_pending)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct PublicReadinessRecord {
    #[serde(rename = "_id")]
    pub id: String,
    pub profile_id: String,
    pub public_policy_revision: i64,
    pub candidate_digest: String,
    pub status: String,
    pub proof_gaps: Vec<String>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub checked_at: DateTime<Utc>,
}

impl std::fmt::Debug for PlatformPublicSession {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PlatformPublicSession { [REDACTED] }")
    }
}
impl std::fmt::Debug for PublicCleanupRecord {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PublicCleanupRecord")
            .field("completed", &self.completed)
            .field("attempts", &self.attempts)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_upstream_evidence_requires_cleanup_and_times_remain_bson_dates() {
        let now = Utc::now();
        let row = PlatformPublicSession {
            id: "session-sensitive".into(),
            profile_id: "profile".into(),
            manager_bot_id: "99".into(),
            telegram_bot_id: 99,
            manager_authority_epoch: 1,
            sender_id: 5,
            chat_id: 5,
            sponsor_owner_user_id: "sponsor-sensitive".into(),
            public_policy_revision: 1,
            policy_digest: "candidate".into(),
            nyxagent_profile: "unverified".into(),
            credential_api_key_id: "key-sensitive".into(),
            credential_ciphertext: vec![1, 2, 3],
            upstream_binding_ciphertext: vec![],
            upstream_may_exist: false,
            active_turn: None,
            effect_revision: 0,
            cleanup_record_id: None,
            cleanup_complete: false,
            revocation_reason: None,
            revoked_at: None,
            created_at: now,
            expires_at: now,
        };
        let debug = format!("{row:?}");
        assert!(!debug.contains("session-sensitive"));
        assert!(!debug.contains("key-sensitive"));
        assert!(!debug.contains("sponsor-sensitive"));
        assert!(!debug.contains("[1, 2, 3]"));
        let mut stored = bson::to_document(&row).unwrap();
        assert!(stored.get_datetime("created_at").is_ok());
        assert!(stored.get_datetime("expires_at").is_ok());
        assert!(stored.get_binary_generic("credential_ciphertext").is_ok());
        stored.remove("upstream_may_exist");
        let unknown: PlatformPublicSession = bson::from_document(stored).unwrap();
        assert!(unknown.upstream_may_exist);
    }
}

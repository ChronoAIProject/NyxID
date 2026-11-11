use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "sessions";
pub const MFA_CHALLENGES: &str = "mfa_challenges";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionPurpose {
    Authenticated,
    MfaPending,
    #[serde(other)]
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MfaExemption {
    MobileRollout,
    LegacyMigration,
    ApprovedSignInPolicy,
}

/// Authentication accepts explicit exemptions; ordinary derivation requires verified MFA.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MfaAssurance {
    Verified,
    Unverified,
    Legacy,
    MobileExempt,
    ApprovedSignInExempt,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    #[serde(rename = "_id")]
    pub id: String,
    pub user_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub purpose: Option<SessionPurpose>,
    /// Non-secret first-factor method retained on pending MFA challenges.
    /// Completed sessions leave this unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mfa_method: Option<String>,
    /// Absent/null means a pre-assurance session. Every current writer sets an
    /// explicit bool, including the bounded migration of sid-less refreshes.
    #[serde(default)]
    pub mfa_assured: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mfa_exemption: Option<MfaExemption>,
    /// SHA-256 hash of the session token stored in the cookie
    pub token_hash: String,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
    pub revoked: bool,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub last_active_at: DateTime<Utc>,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("id", &self.id)
            .field("user_id", &self.user_id)
            .field("purpose", &self.purpose)
            .field("token_hash", &"[REDACTED]")
            .field("revoked", &self.revoked)
            .field("expires_at", &self.expires_at)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collection_name() {
        assert_eq!(COLLECTION_NAME, "sessions");
    }

    fn make_session() -> Session {
        Session {
            purpose: Some(crate::models::session::SessionPurpose::Authenticated),
            mfa_method: None,
            mfa_assured: Some(false),
            mfa_exemption: None,
            id: uuid::Uuid::new_v4().to_string(),
            user_id: uuid::Uuid::new_v4().to_string(),
            token_hash: "abcdef0123456789".to_string(),
            ip_address: Some("127.0.0.1".to_string()),
            user_agent: Some("test-agent".to_string()),
            expires_at: Utc::now(),
            revoked: false,
            created_at: Utc::now(),
            last_active_at: Utc::now(),
        }
    }

    #[test]
    fn assurance_classification_and_derivation_cover_every_persisted_state() {
        use crate::services::session_authority::{
            DerivedMfaAuthority, classification, may_derive, require_derived_mfa, satisfies_mfa,
        };
        let base = bson::to_document(&make_session()).unwrap();
        for (value, exemption, expected) in [
            (
                Some(bson::Bson::Boolean(true)),
                None,
                MfaAssurance::Verified,
            ),
            (
                Some(bson::Bson::Boolean(false)),
                None,
                MfaAssurance::Unverified,
            ),
            (None, None, MfaAssurance::Legacy),
            (Some(bson::Bson::Null), None, MfaAssurance::Legacy),
            (
                Some(bson::Bson::Boolean(false)),
                Some("legacy_migration"),
                MfaAssurance::Legacy,
            ),
            (
                Some(bson::Bson::Boolean(false)),
                Some("mobile_rollout"),
                MfaAssurance::MobileExempt,
            ),
            (
                Some(bson::Bson::Boolean(true)),
                Some("mobile_rollout"),
                MfaAssurance::Verified,
            ),
            (
                Some(bson::Bson::Boolean(false)),
                Some("approved_sign_in_policy"),
                MfaAssurance::ApprovedSignInExempt,
            ),
            (
                Some(bson::Bson::Boolean(true)),
                Some("approved_sign_in_policy"),
                MfaAssurance::Verified,
            ),
        ] {
            let mut row = base.clone();
            row.remove("mfa_assured");
            if let Some(value) = value {
                row.insert("mfa_assured", value);
            }
            if let Some(exemption) = exemption {
                row.insert("mfa_exemption", exemption);
            }
            let session: Session = bson::from_document(row).unwrap();
            assert_eq!(classification(&session), expected);
            assert_eq!(
                satisfies_mfa(&session),
                expected != MfaAssurance::Unverified
            );
            for mfa in [false, true] {
                let verified = expected == MfaAssurance::Verified;
                let allowed = !mfa || verified;
                assert_eq!(may_derive(mfa, verified), allowed);
                assert_eq!(
                    require_derived_mfa(DerivedMfaAuthority::new(mfa, verified)).is_ok(),
                    allowed
                );
            }
        }
    }

    #[test]
    fn bson_roundtrip() {
        let session = make_session();
        let doc = bson::to_document(&session).expect("serialize");
        assert!(doc.get_str("_id").is_ok());
        let restored: Session = bson::from_document(doc).expect("deserialize");
        assert_eq!(session.id, restored.id);
        assert_eq!(session.user_id, restored.user_id);
        assert_eq!(session.revoked, restored.revoked);
    }

    #[test]
    fn bson_roundtrip_with_nulls() {
        let mut session = make_session();
        session.ip_address = None;
        session.user_agent = None;
        let doc = bson::to_document(&session).expect("serialize");
        let restored: Session = bson::from_document(doc).expect("deserialize");
        assert!(restored.ip_address.is_none());
        assert!(restored.user_agent.is_none());
    }
}

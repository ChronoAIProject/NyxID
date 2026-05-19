use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::bson_datetime;
use super::org_membership::{MemberScopeSource, OrgRole, default_scope_source};

pub const COLLECTION_NAME: &str = "org_invites";

/// One-time invite token used to bring a person user into an org.
///
/// Distinct from [`crate::models::invite_code::InviteCode`] which gates
/// new-user signup. Org invites are scoped to an existing org, single-use,
/// and have a TTL enforced via a MongoDB TTL index on `expires_at`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrgInvite {
    #[serde(rename = "_id")]
    pub id: String,
    /// FK to User where `user_type = Org`.
    pub org_user_id: String,
    /// URL-safe single-use token. Indexed unique.
    pub nonce: String,
    /// Role granted to the new member when redeemed.
    pub role: OrgRole,
    /// Where the redeemed membership's service scope comes from.
    ///
    /// Missing values deserialize as `Override` to preserve behavior for
    /// invites issued before role defaults existed.
    #[serde(default = "default_scope_source")]
    pub scope_source: MemberScopeSource,
    /// Optional service scope applied to the new membership when
    /// `scope_source = Override`.
    #[serde(default)]
    pub allowed_service_ids: Option<Vec<String>>,
    /// User id of the org admin who issued this invite.
    pub created_by: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
    /// Set when the invite is redeemed. After redemption the invite is
    /// kept around (until TTL deletes it) for audit / "who used my invite".
    #[serde(default)]
    pub redeemed_by: Option<String>,
    #[serde(default, with = "bson_datetime::optional")]
    pub redeemed_at: Option<DateTime<Utc>>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
}

impl OrgInvite {
    pub fn is_redeemed(&self) -> bool {
        self.redeemed_at.is_some()
    }

    pub fn is_expired(&self, now: DateTime<Utc>) -> bool {
        self.expires_at <= now
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn make_invite() -> OrgInvite {
        OrgInvite {
            id: uuid::Uuid::new_v4().to_string(),
            org_user_id: uuid::Uuid::new_v4().to_string(),
            nonce: "INV-ABCDEFGH".to_string(),
            role: OrgRole::Member,
            scope_source: MemberScopeSource::Override,
            allowed_service_ids: None,
            created_by: uuid::Uuid::new_v4().to_string(),
            expires_at: Utc::now() + Duration::hours(24),
            redeemed_by: None,
            redeemed_at: None,
            created_at: Utc::now(),
        }
    }

    #[test]
    fn collection_name() {
        assert_eq!(COLLECTION_NAME, "org_invites");
    }

    #[test]
    fn fresh_invite_is_not_redeemed_or_expired() {
        let invite = make_invite();
        assert!(!invite.is_redeemed());
        assert!(!invite.is_expired(Utc::now()));
    }

    #[test]
    fn expired_invite() {
        let mut invite = make_invite();
        invite.expires_at = Utc::now() - Duration::seconds(1);
        assert!(invite.is_expired(Utc::now()));
    }

    #[test]
    fn redeemed_invite() {
        let mut invite = make_invite();
        invite.redeemed_by = Some("user-id".to_string());
        invite.redeemed_at = Some(Utc::now());
        assert!(invite.is_redeemed());
    }

    #[test]
    fn bson_roundtrip() {
        let invite = make_invite();
        let doc = bson::to_document(&invite).expect("serialize");
        let restored: OrgInvite = bson::from_document(doc).expect("deserialize");
        assert_eq!(invite.id, restored.id);
        assert_eq!(invite.nonce, restored.nonce);
        assert_eq!(invite.org_user_id, restored.org_user_id);
        assert_eq!(invite.role, restored.role);
        assert_eq!(invite.scope_source, restored.scope_source);
        assert!(!restored.is_redeemed());
    }

    #[test]
    fn missing_scope_source_defaults_to_override() {
        let invite = make_invite();
        let mut doc = bson::to_document(&invite).expect("serialize");
        doc.remove("scope_source");
        let restored: OrgInvite = bson::from_document(doc).expect("deserialize");
        assert_eq!(restored.scope_source, MemberScopeSource::Override);
    }
}

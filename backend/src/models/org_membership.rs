use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::bson_datetime;

pub const COLLECTION_NAME: &str = "org_memberships";

/// Role of a member within an org. Controls what they can do with the
/// org's shared resources via the proxy and management APIs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OrgRole {
    /// Manages org metadata, members, services. Can write all org-owned resources.
    Admin,
    /// Can use org services through the proxy. Cannot manage org resources.
    Member,
    /// Can see org services exist but cannot proxy through them.
    Viewer,
}

impl OrgRole {
    pub const ALL: [Self; 3] = [Self::Admin, Self::Member, Self::Viewer];

    pub fn can_proxy(&self) -> bool {
        matches!(self, Self::Admin | Self::Member)
    }

    pub fn can_admin(&self) -> bool {
        matches!(self, Self::Admin)
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Admin => "admin",
            Self::Member => "member",
            Self::Viewer => "viewer",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemberScopeSource {
    /// Follow the org's current role-level service scope.
    Inherit,
    /// Use this membership row's `allowed_service_ids` as-is.
    Override,
}

/// Serde default for legacy rows that predate role scopes.
///
/// Existing memberships without `scope_source` already interpreted
/// `allowed_service_ids = None` as full access. That is override semantics,
/// so deserializing missing values as `Override` preserves behavior.
pub fn default_scope_source() -> MemberScopeSource {
    MemberScopeSource::Override
}

impl MemberScopeSource {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Inherit => "inherit",
            Self::Override => "override",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrgMembership {
    #[serde(rename = "_id")]
    pub id: String,
    /// User where `user_type = Org`. The owner of all org-shared resources.
    pub org_user_id: String,
    /// The person user who is a member of the org.
    pub member_user_id: String,
    pub role: OrgRole,
    /// Where this membership's effective service scope comes from.
    ///
    /// New memberships created by the service layer default to `Inherit`.
    /// The serde default remains `Override` so legacy rows keep their old
    /// behavior until explicitly reset by an admin.
    #[serde(default = "default_scope_source")]
    pub scope_source: MemberScopeSource,
    /// Optional scope: when set, this member can only access these
    /// `UserService` ids inside the org. `None` means access to all org
    /// services (subject to role) when `scope_source = Override`. Ignored
    /// while `scope_source = Inherit`.
    #[serde(default)]
    pub allowed_service_ids: Option<Vec<String>>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    /// Soft-delete timestamp. Active memberships have `revoked_at = None`.
    #[serde(default, with = "bson_datetime::optional")]
    pub revoked_at: Option<DateTime<Utc>>,
}

impl OrgMembership {
    pub fn is_active(&self) -> bool {
        self.revoked_at.is_none()
    }

    /// Whether this member is allowed to access the given user_service id
    /// based on this row's `allowed_service_ids` scope. Does not check role
    /// and does not resolve role-level inherited scopes.
    ///
    /// Use `org_role_scope_service::effective_scope_for_membership` plus
    /// `org_role_scope_service::scope_allows` for new authorization checks.
    #[allow(dead_code)]
    #[deprecated(
        note = "use org_role_scope_service::effective_scope_for_membership plus scope_allows"
    )]
    pub fn allows_service(&self, user_service_id: &str) -> bool {
        match &self.allowed_service_ids {
            None => true,
            Some(ids) => ids.iter().any(|id| id == user_service_id),
        }
    }
}

#[cfg(test)]
#[allow(deprecated)]
mod tests {
    use super::*;

    fn make_membership() -> OrgMembership {
        OrgMembership {
            id: uuid::Uuid::new_v4().to_string(),
            org_user_id: uuid::Uuid::new_v4().to_string(),
            member_user_id: uuid::Uuid::new_v4().to_string(),
            role: OrgRole::Member,
            scope_source: MemberScopeSource::Override,
            allowed_service_ids: None,
            created_at: Utc::now(),
            revoked_at: None,
        }
    }

    #[test]
    fn collection_name() {
        assert_eq!(COLLECTION_NAME, "org_memberships");
    }

    #[test]
    fn role_capabilities() {
        assert!(OrgRole::Admin.can_proxy());
        assert!(OrgRole::Admin.can_admin());
        assert!(OrgRole::Member.can_proxy());
        assert!(!OrgRole::Member.can_admin());
        assert!(!OrgRole::Viewer.can_proxy());
        assert!(!OrgRole::Viewer.can_admin());
        assert_eq!(OrgRole::Admin.as_str(), "admin");
        assert_eq!(OrgRole::Member.as_str(), "member");
        assert_eq!(OrgRole::Viewer.as_str(), "viewer");
    }

    #[test]
    fn role_serializes_snake_case() {
        let admin = bson::to_bson(&OrgRole::Admin).expect("ser admin");
        let member = bson::to_bson(&OrgRole::Member).expect("ser member");
        let viewer = bson::to_bson(&OrgRole::Viewer).expect("ser viewer");
        assert_eq!(admin.as_str().unwrap(), "admin");
        assert_eq!(member.as_str().unwrap(), "member");
        assert_eq!(viewer.as_str().unwrap(), "viewer");
    }

    #[test]
    fn bson_roundtrip_active() {
        let m = make_membership();
        let doc = bson::to_document(&m).expect("serialize");
        let restored: OrgMembership = bson::from_document(doc).expect("deserialize");
        assert_eq!(m.id, restored.id);
        assert_eq!(m.org_user_id, restored.org_user_id);
        assert_eq!(m.member_user_id, restored.member_user_id);
        assert!(restored.is_active());
    }

    #[test]
    fn missing_scope_source_defaults_to_override() {
        let m = make_membership();
        let mut doc = bson::to_document(&m).expect("serialize");
        doc.remove("scope_source");
        let restored: OrgMembership = bson::from_document(doc).expect("deserialize");
        assert_eq!(restored.scope_source, MemberScopeSource::Override);
    }

    #[test]
    fn bson_roundtrip_revoked() {
        let mut m = make_membership();
        m.revoked_at = Some(Utc::now());
        let doc = bson::to_document(&m).expect("serialize");
        let restored: OrgMembership = bson::from_document(doc).expect("deserialize");
        assert!(!restored.is_active());
    }

    #[test]
    fn allows_service_with_no_scope() {
        let m = make_membership();
        assert!(m.allows_service("any-service-id"));
    }

    #[test]
    fn allows_service_with_scope() {
        let mut m = make_membership();
        m.allowed_service_ids = Some(vec!["svc-1".to_string(), "svc-2".to_string()]);
        assert!(m.allows_service("svc-1"));
        assert!(m.allows_service("svc-2"));
        assert!(!m.allows_service("svc-3"));
    }

    #[test]
    fn allows_service_with_empty_scope_blocks_all() {
        let mut m = make_membership();
        m.allowed_service_ids = Some(vec![]);
        assert!(!m.allows_service("svc-1"));
    }
}

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "feature_flag_overrides";

/// The scope a single feature-flag override applies to.
///
/// Resolution applies overrides most-specific-first. Within an org context:
/// `User` > `Role` > `Org` > `Global` > code default. In the personal (non-org)
/// context: `User` (personal) > `Global` > code default.
///
/// `Global` rows are platform-level (no org) and affect everyone, org or not.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FlagTargetKind {
    /// Platform-wide rollout / killswitch. `org_user_id` is `None`, `target_key`
    /// is `None`. Applies to everyone (org or not) unless a narrower scope wins.
    Global,
    /// Applies to every member of the org (unless a narrower override wins).
    Org,
    /// Applies to members holding a specific `OrgRole`.
    Role,
    /// Applies to a single user (`target_key` = their user id). Org-scoped when
    /// `org_user_id` is set; personal (everywhere) when `org_user_id` is `None`.
    User,
}

impl FlagTargetKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Global => "global",
            Self::Org => "org",
            Self::Role => "role",
            Self::User => "user",
        }
    }
}

/// A stored per-org feature-flag override.
///
/// One row per `(org_user_id, flag_key, target_kind, target_key)`. `flag_key`
/// must match an entry in the code registry (`feature_flag_service::FEATURE_FLAGS`);
/// unknown keys are rejected at the service layer, not stored. `target_key` is
/// `None` for org-scope rows, the `OrgRole` string for role-scope rows, and the
/// member's user id for user-scope rows.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FeatureFlagOverride {
    #[serde(rename = "_id")]
    pub id: String,
    /// The org (`user_type = Org`) this override belongs to. `None` for
    /// platform-level rows — global rollout and personal (org-independent) user
    /// overrides. Existing org rows always have this set.
    #[serde(default)]
    pub org_user_id: Option<String>,
    pub flag_key: String,
    pub target_kind: FlagTargetKind,
    /// `None` for org scope; role string for role scope; member_user_id for
    /// user scope. Legacy/org rows deserialize the missing field as `None`.
    #[serde(default)]
    pub target_key: Option<String>,
    pub enabled: bool,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
    /// member_user_id of the org admin who last wrote this override.
    pub updated_by: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collection_name() {
        assert_eq!(COLLECTION_NAME, "feature_flag_overrides");
    }

    #[test]
    fn target_kind_serializes_snake_case() {
        let org = bson::to_bson(&FlagTargetKind::Org).expect("ser org");
        let role = bson::to_bson(&FlagTargetKind::Role).expect("ser role");
        let user = bson::to_bson(&FlagTargetKind::User).expect("ser user");
        assert_eq!(org.as_str().unwrap(), "org");
        assert_eq!(role.as_str().unwrap(), "role");
        assert_eq!(user.as_str().unwrap(), "user");
    }

    #[test]
    fn bson_roundtrip() {
        let row = FeatureFlagOverride {
            id: uuid::Uuid::new_v4().to_string(),
            org_user_id: Some(uuid::Uuid::new_v4().to_string()),
            flag_key: "example_ui".to_string(),
            target_kind: FlagTargetKind::Role,
            target_key: Some("admin".to_string()),
            enabled: true,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            updated_by: uuid::Uuid::new_v4().to_string(),
        };
        let doc = bson::to_document(&row).expect("serialize");
        let restored: FeatureFlagOverride = bson::from_document(doc).expect("deserialize");
        assert_eq!(row.id, restored.id);
        assert_eq!(row.flag_key, restored.flag_key);
        assert_eq!(restored.target_kind, FlagTargetKind::Role);
        assert_eq!(restored.target_key.as_deref(), Some("admin"));
        assert!(restored.enabled);
    }

    #[test]
    fn missing_target_key_defaults_to_none() {
        let row = FeatureFlagOverride {
            id: uuid::Uuid::new_v4().to_string(),
            org_user_id: Some(uuid::Uuid::new_v4().to_string()),
            flag_key: "example_ui".to_string(),
            target_kind: FlagTargetKind::Org,
            target_key: None,
            enabled: false,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            updated_by: uuid::Uuid::new_v4().to_string(),
        };
        let mut doc = bson::to_document(&row).expect("serialize");
        doc.remove("target_key");
        let restored: FeatureFlagOverride = bson::from_document(doc).expect("deserialize");
        assert!(restored.target_key.is_none());
    }

    #[test]
    fn global_row_has_no_org() {
        let row = FeatureFlagOverride {
            id: uuid::Uuid::new_v4().to_string(),
            org_user_id: None,
            flag_key: "example_ui".to_string(),
            target_kind: FlagTargetKind::Global,
            target_key: None,
            enabled: true,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            updated_by: "actor".to_string(),
        };
        let doc = bson::to_document(&row).expect("serialize");
        let restored: FeatureFlagOverride = bson::from_document(doc).expect("deserialize");
        assert_eq!(restored.target_kind, FlagTargetKind::Global);
        assert!(restored.org_user_id.is_none());
        assert_eq!(
            bson::to_bson(&FlagTargetKind::Global).unwrap().as_str(),
            Some("global")
        );
    }

    #[test]
    fn missing_org_user_id_defaults_to_none() {
        let mut doc = bson::to_document(&FeatureFlagOverride {
            id: "x".to_string(),
            org_user_id: Some("org".to_string()),
            flag_key: "example_ui".to_string(),
            target_kind: FlagTargetKind::User,
            target_key: Some("u".to_string()),
            enabled: true,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            updated_by: "actor".to_string(),
        })
        .expect("serialize");
        doc.remove("org_user_id");
        let restored: FeatureFlagOverride = bson::from_document(doc).expect("deserialize");
        assert!(restored.org_user_id.is_none());
    }
}

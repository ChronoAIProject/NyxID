use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::bson_datetime;

pub const COLLECTION_NAME: &str = "users";

/// Distinguishes a real person account from an organization account.
///
/// Org accounts are users with `user_type = Org`. They cannot log in directly
/// (password / social / refresh paths reject them) and exist purely as the
/// owner record for shared resources (UserService, UserApiKey, ApiKey, etc.).
/// Membership in an org is tracked via the `org_memberships` collection.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UserType {
    #[default]
    Person,
    Org,
}

impl UserType {
    pub fn is_org(&self) -> bool {
        matches!(self, Self::Org)
    }

    pub fn is_person(&self) -> bool {
        matches!(self, Self::Person)
    }
}

/// Platform-level role derived from RBAC platform role membership. The
/// platform has three tiers, ordered low-to-high:
///
/// - `User` — regular user, no admin access
/// - `Operator` — read-only access to admin GET endpoints (no writes).
///   Intended for strategy / share-ops accounts that need cross-org
///   platform data without write privileges.
/// - `Admin` — full read + write access to all `/admin/*` endpoints
///
/// `Admin` always implies `Operator`-level read access.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlatformRole {
    User,
    Operator,
    Admin,
}

impl PlatformRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Operator => "operator",
            Self::Admin => "admin",
        }
    }

    pub fn has_admin_read(&self) -> bool {
        matches!(self, Self::Admin | Self::Operator)
    }

    pub fn is_admin(&self) -> bool {
        matches!(self, Self::Admin)
    }

    pub fn legacy_flags(&self) -> (bool, bool) {
        match self {
            Self::Admin => (true, false),
            Self::Operator => (false, true),
            Self::User => (false, false),
        }
    }
}

/// User-scoped configuration and preferences. Embedded sub-document on
/// `User`; extend by adding new preference groups as fields here.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserProfileConfig {
    #[serde(default)]
    pub onboarding: OnboardingState,
}

/// Tracks which first-run onboarding flows the user has completed (or
/// skipped). Timestamps rather than bools record *when*; `None` is the
/// signal the post-login wizard redirect keys off.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OnboardingState {
    /// Set when the user finishes or skips the AI-services wizard.
    /// `None` => wizard not yet seen => eligible for the redirect.
    #[serde(default, with = "bson_datetime::optional")]
    pub ai_services_completed_at: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct User {
    #[serde(rename = "_id")]
    pub id: String,
    pub email: String,
    pub password_hash: Option<String>,
    pub display_name: Option<String>,
    /// Stable user-facing org slug. Populated only for `user_type = Org`;
    /// person users and legacy rows deserialize as `None`.
    #[serde(default)]
    pub slug: Option<String>,
    pub avatar_url: Option<String>,
    pub email_verified: bool,
    pub email_verification_token: Option<String>,
    pub password_reset_token: Option<String>,
    #[serde(default, with = "bson_datetime::optional")]
    pub password_reset_expires_at: Option<DateTime<Utc>>,
    pub is_active: bool,
    /// Legacy mirror of platform admin RBAC membership. Kept for storage and
    /// response compatibility during the migration window; RBAC role_ids are
    /// authoritative for access checks.
    pub is_admin: bool,
    /// Legacy mirror of platform operator RBAC membership. Defaults to false;
    /// legacy rows without this field deserialize as false.
    #[serde(default)]
    pub is_operator: bool,
    #[serde(default)]
    pub role_ids: Vec<String>,
    #[serde(default)]
    pub group_ids: Vec<String>,
    #[serde(default)]
    pub invite_code_id: Option<String>,
    pub mfa_enabled: bool,
    #[serde(default)]
    pub social_provider: Option<String>,
    #[serde(default)]
    pub social_provider_id: Option<String>,
    /// Account type: `Person` (default) or `Org`. Existing rows without this
    /// field deserialize as `Person` via serde default.
    #[serde(default)]
    pub user_type: UserType,
    /// Optional preferred org for proxy credential resolution tiebreaking.
    /// When a user belongs to multiple orgs that share a service, this
    /// org's credentials win. Falls back to earliest membership when unset.
    #[serde(default)]
    pub primary_org_id: Option<String>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
    #[serde(default, with = "bson_datetime::optional")]
    pub last_login_at: Option<DateTime<Utc>>,
    /// User-scoped config / preferences (onboarding state, etc.). Legacy
    /// rows without this field deserialize as `UserProfileConfig::default()`.
    #[serde(default)]
    pub profile_config: UserProfileConfig,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collection_name() {
        assert_eq!(COLLECTION_NAME, "users");
    }

    fn make_user() -> User {
        User {
            id: "550e8400-e29b-41d4-a716-446655440000".to_string(),
            email: "test@example.com".to_string(),
            password_hash: Some("$argon2id$hash".to_string()),
            display_name: Some("Test User".to_string()),
            slug: None,
            avatar_url: None,
            email_verified: true,
            email_verification_token: None,
            password_reset_token: None,
            password_reset_expires_at: None,
            is_active: true,
            is_admin: false,
            is_operator: false,
            role_ids: vec![],
            group_ids: vec![],
            invite_code_id: None,
            mfa_enabled: false,
            social_provider: None,
            social_provider_id: None,
            user_type: UserType::Person,
            primary_org_id: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            last_login_at: None,
            profile_config: UserProfileConfig::default(),
        }
    }

    #[test]
    fn bson_roundtrip() {
        let user = make_user();
        let doc = bson::to_document(&user).expect("serialize user to bson");
        // _id field should be present (serde rename)
        assert!(doc.get_str("_id").is_ok());
        assert!(doc.get("id").is_none(), "raw 'id' should not exist in bson");
        let restored: User = bson::from_document(doc).expect("deserialize user from bson");
        assert_eq!(user.id, restored.id);
        assert_eq!(user.email, restored.email);
        assert_eq!(user.is_active, restored.is_active);
    }

    #[test]
    fn bson_roundtrip_with_optional_datetimes() {
        let mut user = make_user();
        user.password_reset_expires_at = Some(Utc::now());
        user.last_login_at = Some(Utc::now());
        let doc = bson::to_document(&user).expect("serialize");
        let restored: User = bson::from_document(doc).expect("deserialize");
        assert!(restored.password_reset_expires_at.is_some());
        assert!(restored.last_login_at.is_some());
    }

    #[test]
    fn bson_all_fields_serialized() {
        let user = make_user();
        let doc = bson::to_document(&user).expect("serialize");
        // Ensure critical fields are present in the document (not skipped)
        let keys: Vec<&str> = doc.keys().map(|k| k.as_str()).collect();
        assert!(keys.contains(&"_id"));
        assert!(keys.contains(&"email"));
        assert!(keys.contains(&"slug"));
        assert!(keys.contains(&"is_active"));
        assert!(keys.contains(&"is_admin"));
        assert!(keys.contains(&"mfa_enabled"));
        assert!(keys.contains(&"user_type"));
        assert!(keys.contains(&"created_at"));
        assert!(keys.contains(&"updated_at"));
    }

    #[test]
    fn user_type_default_is_person() {
        let ut = UserType::default();
        assert!(ut.is_person());
        assert!(!ut.is_org());
    }

    #[test]
    fn user_type_serializes_snake_case() {
        let person = bson::to_bson(&UserType::Person).expect("ser person");
        let org = bson::to_bson(&UserType::Org).expect("ser org");
        assert_eq!(person.as_str().unwrap(), "person");
        assert_eq!(org.as_str().unwrap(), "org");
    }

    #[test]
    fn org_user_roundtrip() {
        let mut org = make_user();
        org.user_type = UserType::Org;
        org.password_hash = None;
        org.display_name = Some("Chrono AI".to_string());
        org.slug = Some("chrono-ai".to_string());
        let doc = bson::to_document(&org).expect("serialize org");
        let restored: User = bson::from_document(doc).expect("deserialize org");
        assert!(restored.user_type.is_org());
        assert_eq!(restored.password_hash, None);
        assert_eq!(restored.slug.as_deref(), Some("chrono-ai"));
    }

    #[test]
    fn legacy_user_without_user_type_deserializes_as_person() {
        // Simulate a row written before the user_type field existed.
        let mut doc = bson::to_document(&make_user()).expect("serialize");
        doc.remove("user_type");
        doc.remove("primary_org_id");
        doc.remove("slug");
        let restored: User = bson::from_document(doc).expect("deserialize legacy");
        assert!(restored.user_type.is_person());
        assert_eq!(restored.primary_org_id, None);
        assert_eq!(restored.slug, None);
    }

    #[test]
    fn legacy_user_without_is_operator_deserializes_as_false() {
        // Simulate a row written before the is_operator field existed.
        let mut doc = bson::to_document(&make_user()).expect("serialize");
        doc.remove("is_operator");
        let restored: User = bson::from_document(doc).expect("deserialize legacy");
        assert!(!restored.is_operator);
    }

    #[test]
    fn platform_role_serializes_snake_case() {
        assert_eq!(PlatformRole::User.as_str(), "user");
        assert_eq!(PlatformRole::Operator.as_str(), "operator");
        assert_eq!(PlatformRole::Admin.as_str(), "admin");
    }
}

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "service_approval_configs";

/// Approval mode for a service.
///
/// `PerRequest` (default): every proxy call needs a fresh approval.
/// `Grant`: approval creates a time-limited grant (legacy behavior).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalMode {
    #[default]
    PerRequest,
    Grant,
}

impl ApprovalMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::PerRequest => "per_request",
            Self::Grant => "grant",
        }
    }
}

/// Legacy documents created before `approval_mode` existed should retain the
/// historical grant-based behavior when deserialized.
pub fn legacy_approval_mode_default() -> ApprovalMode {
    ApprovalMode::Grant
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalEffect {
    RequireApproval,
    AutoAllow,
    Deny,
}

impl ApprovalEffect {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::RequireApproval => "require_approval",
            Self::AutoAllow => "auto_allow",
            Self::Deny => "deny",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalVerb {
    Read,
    Write,
    Destructive,
}

impl ApprovalVerb {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
            Self::Destructive => "destructive",
        }
    }
}

fn default_rule_resource_pattern() -> String {
    "*".to_string()
}

fn default_rule_effect() -> ApprovalEffect {
    ApprovalEffect::RequireApproval
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovalRule {
    /// Match methods case-insensitively. Empty or ["*"] means any method.
    #[serde(default)]
    pub methods: Vec<String>,

    /// Glob-like resource pattern. Phase 1 treats empty / "*" as any;
    /// Phase 2 compiles this through globset for path/command matching.
    #[serde(default = "default_rule_resource_pattern")]
    pub resource_pattern: String,

    /// Optional semantic verb gate. Empty means any verb.
    #[serde(default)]
    pub verbs: Vec<ApprovalVerb>,

    /// The effect applied by this rule.
    #[serde(default = "default_rule_effect")]
    pub effect: ApprovalEffect,

    /// Approval mode when `effect = require_approval`.
    #[serde(default)]
    pub mode: ApprovalMode,
}

/// Per-service approval override for a user.
///
/// When a user has global `approval_required = true`, they can exempt specific
/// services (set `approval_required = false`). Conversely, when global is false,
/// they can require approval for specific high-risk services.
///
/// If no config exists for a (user, service) pair, the global
/// `notification_channels.approval_required` setting applies.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ServiceApprovalConfig {
    /// UUID v4 string
    #[serde(rename = "_id")]
    pub id: String,

    /// Owner user ID
    pub user_id: String,

    /// Downstream service ID
    pub service_id: String,

    /// Human-readable service name (denormalized for display)
    pub service_name: String,

    /// Whether approval is required for this specific service.
    /// Overrides the global `notification_channels.approval_required`.
    pub approval_required: bool,

    /// Approval mode for this service.
    #[serde(default = "legacy_approval_mode_default")]
    pub approval_mode: ApprovalMode,

    /// Ordered granular approval rules. Empty preserves legacy binary behavior
    /// unless `default_effect` is explicitly set.
    #[serde(default)]
    pub rules: Vec<ApprovalRule>,

    /// Fallback effect when no rule matches. `None` preserves the legacy
    /// `approval_required` fallback for empty rule sets.
    #[serde(default)]
    pub default_effect: Option<ApprovalEffect>,

    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,

    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collection_name() {
        assert_eq!(COLLECTION_NAME, "service_approval_configs");
    }

    fn make_config() -> ServiceApprovalConfig {
        ServiceApprovalConfig {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: uuid::Uuid::new_v4().to_string(),
            service_id: uuid::Uuid::new_v4().to_string(),
            service_name: "OpenAI API".to_string(),
            approval_required: true,
            approval_mode: ApprovalMode::default(),
            rules: vec![],
            default_effect: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn bson_roundtrip() {
        let cfg = make_config();
        let doc = bson::to_document(&cfg).expect("serialize");
        assert!(doc.get_str("_id").is_ok());
        assert!(doc.get("id").is_none(), "raw 'id' should not exist in bson");
        let restored: ServiceApprovalConfig = bson::from_document(doc).expect("deserialize");
        assert_eq!(cfg.id, restored.id);
        assert_eq!(cfg.user_id, restored.user_id);
        assert_eq!(cfg.service_id, restored.service_id);
        assert_eq!(cfg.approval_required, restored.approval_required);
        assert_eq!(cfg.approval_mode, restored.approval_mode);
        assert_eq!(cfg.rules, restored.rules);
        assert_eq!(cfg.default_effect, restored.default_effect);
    }

    #[test]
    fn default_approval_mode_is_per_request() {
        assert_eq!(ApprovalMode::default(), ApprovalMode::PerRequest);
    }

    #[test]
    fn missing_approval_mode_defaults_to_grant_for_legacy_docs() {
        let mut doc = bson::to_document(&make_config()).expect("serialize");
        doc.remove("approval_mode");
        let restored: ServiceApprovalConfig = bson::from_document(doc).expect("deserialize");
        assert_eq!(restored.approval_mode, ApprovalMode::Grant);
    }

    #[test]
    fn grant_approval_mode_roundtrips() {
        let cfg = ServiceApprovalConfig {
            approval_mode: ApprovalMode::Grant,
            ..make_config()
        };
        let doc = bson::to_document(&cfg).expect("serialize");
        let restored: ServiceApprovalConfig = bson::from_document(doc).expect("deserialize");
        assert_eq!(restored.approval_mode, ApprovalMode::Grant);
    }

    #[test]
    fn approval_mode_as_str() {
        assert_eq!(ApprovalMode::PerRequest.as_str(), "per_request");
        assert_eq!(ApprovalMode::Grant.as_str(), "grant");
    }

    #[test]
    fn legacy_string_values_deserialize() {
        // Existing documents stored as strings should still deserialize
        let mut doc = bson::to_document(&make_config()).expect("serialize");
        doc.insert("approval_mode", "grant");
        let restored: ServiceApprovalConfig = bson::from_document(doc).expect("deserialize");
        assert_eq!(restored.approval_mode, ApprovalMode::Grant);
    }

    #[test]
    fn bson_all_fields_serialized() {
        let cfg = make_config();
        let doc = bson::to_document(&cfg).expect("serialize");
        let keys: Vec<&str> = doc.keys().map(|k| k.as_str()).collect();
        assert!(keys.contains(&"_id"));
        assert!(keys.contains(&"user_id"));
        assert!(keys.contains(&"service_id"));
        assert!(keys.contains(&"service_name"));
        assert!(keys.contains(&"approval_required"));
        assert!(keys.contains(&"approval_mode"));
        assert!(keys.contains(&"rules"));
        assert!(keys.contains(&"default_effect"));
        assert!(keys.contains(&"created_at"));
        assert!(keys.contains(&"updated_at"));
    }

    #[test]
    fn missing_granular_fields_default_for_legacy_docs() {
        let mut doc = bson::to_document(&make_config()).expect("serialize");
        doc.remove("rules");
        doc.remove("default_effect");
        let restored: ServiceApprovalConfig = bson::from_document(doc).expect("deserialize");
        assert!(restored.rules.is_empty());
        assert_eq!(restored.default_effect, None);
    }

    #[test]
    fn approval_rule_roundtrips() {
        let cfg = ServiceApprovalConfig {
            rules: vec![ApprovalRule {
                methods: vec!["POST".to_string()],
                resource_pattern: "*".to_string(),
                verbs: vec![ApprovalVerb::Write],
                effect: ApprovalEffect::RequireApproval,
                mode: ApprovalMode::Grant,
            }],
            default_effect: Some(ApprovalEffect::AutoAllow),
            ..make_config()
        };
        let doc = bson::to_document(&cfg).expect("serialize");
        let restored: ServiceApprovalConfig = bson::from_document(doc).expect("deserialize");
        assert_eq!(restored.rules.len(), 1);
        assert_eq!(restored.rules[0].verbs, vec![ApprovalVerb::Write]);
        assert_eq!(restored.default_effect, Some(ApprovalEffect::AutoAllow));
    }
}

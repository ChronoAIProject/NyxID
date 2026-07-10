use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "platform_settings";
pub const PLATFORM_SETTINGS_ID: &str = "platform";

fn default_platform_settings_id() -> String {
    PLATFORM_SETTINGS_ID.to_string()
}

/// Single-row platform settings document.
///
/// `None` means the setting is not overridden in MongoDB and the process env
/// default from `AppConfig` remains authoritative.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlatformSettings {
    #[serde(rename = "_id", default = "default_platform_settings_id")]
    pub id: String,
    #[serde(default)]
    pub broker_require_sender_constraint: Option<bool>,
    #[serde(default)]
    pub broker_require_admin_capability: Option<bool>,
    #[serde(default)]
    pub broker_policy_revision: i64,
}

impl PlatformSettings {
    pub fn empty() -> Self {
        Self {
            id: PLATFORM_SETTINGS_ID.to_string(),
            broker_require_sender_constraint: None,
            broker_require_admin_capability: None,
            broker_policy_revision: 0,
        }
    }
}

impl Default for PlatformSettings {
    fn default() -> Self {
        Self::empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mongodb::bson::{self, doc};

    #[test]
    fn collection_name() {
        assert_eq!(COLLECTION_NAME, "platform_settings");
    }

    #[test]
    fn bson_roundtrip_preserves_overrides() {
        let settings = PlatformSettings {
            id: PLATFORM_SETTINGS_ID.to_string(),
            broker_require_sender_constraint: Some(true),
            broker_require_admin_capability: Some(false),
            broker_policy_revision: 7,
        };

        let doc = bson::to_document(&settings).expect("serialize platform settings");
        assert_eq!(doc.get_str("_id").expect("id"), PLATFORM_SETTINGS_ID);

        let restored: PlatformSettings =
            bson::from_document(doc).expect("deserialize platform settings");
        assert_eq!(restored, settings);
    }

    #[test]
    fn bson_legacy_defaults_missing_fields_to_no_overrides() {
        let doc = doc! { "_id": PLATFORM_SETTINGS_ID };

        let restored: PlatformSettings =
            bson::from_document(doc).expect("deserialize legacy platform settings");

        assert_eq!(restored.id, PLATFORM_SETTINGS_ID);
        assert_eq!(restored.broker_require_sender_constraint, None);
        assert_eq!(restored.broker_require_admin_capability, None);
        assert_eq!(restored.broker_policy_revision, 0);
    }

    #[test]
    fn bson_legacy_defaults_missing_id_to_platform() {
        let doc = doc! {
            "broker_require_sender_constraint": true,
        };

        let restored: PlatformSettings =
            bson::from_document(doc).expect("deserialize settings without id");

        assert_eq!(restored.id, PLATFORM_SETTINGS_ID);
        assert_eq!(restored.broker_require_sender_constraint, Some(true));
        assert_eq!(restored.broker_require_admin_capability, None);
        assert_eq!(restored.broker_policy_revision, 0);
    }
}

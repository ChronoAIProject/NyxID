use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "platform_service_preferences";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialIntent {
    #[default]
    Auto,
    OwnOnly,
    PlatformOnly,
}

impl CredentialIntent {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::OwnOnly => "own_only",
            Self::PlatformOnly => "platform_only",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlatformOperationPreferenceOverride {
    pub operation_id: String,
    pub platform_enabled: bool,
    pub max_credits_per_call: String,
    pub max_credits_per_day: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlatformServicePreference {
    #[serde(rename = "_id")]
    pub id: String,
    pub owner_id: String,
    pub catalog_service_id: String,
    pub platform_enabled: bool,
    pub max_credits_per_call: String,
    pub max_credits_per_day: String,
    #[serde(default)]
    pub operation_overrides: Vec<PlatformOperationPreferenceOverride>,
    pub created_by: String,
    pub updated_by: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bson_roundtrip_preserves_owner_consent_and_dates() {
        let timestamp = DateTime::from_timestamp(1_700_000_000, 0).expect("fixed timestamp");
        let preference = PlatformServicePreference {
            id: "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa".to_string(),
            owner_id: "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb".to_string(),
            catalog_service_id: "cccccccc-cccc-4ccc-8ccc-cccccccccccc".to_string(),
            platform_enabled: true,
            max_credits_per_call: "2.5".to_string(),
            max_credits_per_day: "25".to_string(),
            operation_overrides: vec![PlatformOperationPreferenceOverride {
                operation_id: "dddddddd-dddd-4ddd-8ddd-dddddddddddd".to_string(),
                platform_enabled: false,
                max_credits_per_call: "1".to_string(),
                max_credits_per_day: "5".to_string(),
            }],
            created_by: "eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeee".to_string(),
            updated_by: "eeeeeeee-eeee-4eee-8eee-eeeeeeeeeeee".to_string(),
            created_at: timestamp,
            updated_at: timestamp,
        };

        let document = bson::to_document(&preference).expect("serialize preference");
        let restored: PlatformServicePreference =
            bson::from_document(document).expect("deserialize preference");

        assert_eq!(restored, preference);
        assert_eq!(
            restored.created_at.timestamp_millis(),
            timestamp.timestamp_millis()
        );
    }

    #[test]
    fn credential_intent_has_stable_wire_names() {
        assert_eq!(
            serde_json::to_value(CredentialIntent::PlatformOnly).expect("serialize intent"),
            serde_json::json!("platform_only")
        );
        assert_eq!(CredentialIntent::OwnOnly.as_str(), "own_only");
    }
}

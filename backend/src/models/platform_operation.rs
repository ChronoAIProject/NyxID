use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "platform_operations";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PlatformOperationName {
    XSearch,
    Speak,
    CallAndSay,
    FlightSearch,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct XSearchConfig {
    #[serde(default = "default_x_search_max_results_cap")]
    pub max_results_cap: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpeakConfig {
    pub allowed_voice_ids: Vec<String>,
    #[serde(default = "default_speak_max_chars")]
    pub max_chars: u32,
    #[serde(default = "default_speak_model_id")]
    pub model_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CallAndSayConfig {
    #[serde(default)]
    pub allowed_destination_prefixes: Vec<String>,
    #[serde(default = "default_call_max_message_chars")]
    pub max_message_chars: u32,
    #[serde(default = "default_call_voice")]
    pub voice: String,
    #[serde(default = "default_call_max_per_user_per_day")]
    pub max_calls_per_user_per_day: u32,
    pub account_sid: String,
    pub call_from: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FlightSearchConfig {
    #[serde(default = "default_flight_search_max_offers_cap")]
    pub max_offers_cap: u32,
    #[serde(default = "default_flight_search_max_per_user_per_day")]
    pub max_searches_per_user_per_day: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum PlatformOperationConfig {
    XSearch(XSearchConfig),
    Speak(SpeakConfig),
    CallAndSay(CallAndSayConfig),
    FlightSearch(FlightSearchConfig),
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PlatformOperation {
    #[serde(rename = "_id")]
    pub id: String,
    pub op: PlatformOperationName,
    #[serde(default)]
    pub enabled: bool,
    pub vendor_service_slug: String,
    pub config: PlatformOperationConfig,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
    pub updated_by: String,
}

pub const fn default_x_search_max_results_cap() -> u32 {
    10
}

pub const fn default_speak_max_chars() -> u32 {
    1_000
}

pub fn default_speak_model_id() -> String {
    "eleven_multilingual_v2".to_string()
}

pub const fn default_call_max_message_chars() -> u32 {
    500
}

pub fn default_call_voice() -> String {
    "alice".to_string()
}

pub const fn default_call_max_per_user_per_day() -> u32 {
    3
}

pub const fn default_flight_search_max_offers_cap() -> u32 {
    10
}

pub const fn default_flight_search_max_per_user_per_day() -> u32 {
    20
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_rejects_unknown_fields() {
        let value = serde_json::json!({
            "type": "speak",
            "allowed_voice_ids": ["voice-a"],
            "max_chars": 100,
            "model_id": "eleven_multilingual_v2",
            "caller_supplied_body": true,
        });

        assert!(serde_json::from_value::<PlatformOperationConfig>(value).is_err());

        let flight = serde_json::json!({
            "type": "flight_search",
            "max_offers_cap": 10,
            "max_searches_per_user_per_day": 20,
            "create_order": true,
        });
        assert!(serde_json::from_value::<PlatformOperationConfig>(flight).is_err());
    }

    #[test]
    fn bson_roundtrip_preserves_typed_config_and_datetime() {
        let operation = PlatformOperation {
            id: uuid::Uuid::new_v4().to_string(),
            op: PlatformOperationName::XSearch,
            enabled: false,
            vendor_service_slug: "platform-x".to_string(),
            config: PlatformOperationConfig::XSearch(XSearchConfig {
                max_results_cap: 10,
            }),
            updated_at: Utc::now(),
            updated_by: uuid::Uuid::new_v4().to_string(),
        };

        let document = bson::to_document(&operation).expect("serialize platform operation");
        let restored: PlatformOperation =
            bson::from_document(document).expect("deserialize platform operation");

        assert_eq!(restored.id, operation.id);
        assert_eq!(restored.op, operation.op);
        assert_eq!(restored.config, operation.config);
        assert_eq!(
            restored.updated_at.timestamp_millis(),
            operation.updated_at.timestamp_millis()
        );
    }
}

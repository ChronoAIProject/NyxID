use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::models::service_billing::{BillingMetric, PricingSyncStatus};

pub const COLLECTION_NAME: &str = "platform_operations";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConstrainedOp {
    Speak,
    CallAndSay,
    FlightSearch,
}

// Kept as an API-facing name until the constrained routes move in Step 4.
pub type PlatformOperationName = ConstrainedOp;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SpeakOperationConfig {
    pub allowed_voice_ids: Vec<String>,
    #[serde(default = "default_speak_model_id")]
    pub model_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CallAndSayOperationConfig {
    #[serde(default)]
    pub allowed_destination_prefixes: Vec<String>,
    #[serde(default = "default_call_voice")]
    pub voice: String,
    pub account_sid: String,
    pub call_from: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FlightSearchOperationConfig {}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ConstrainedConfig {
    Speak(SpeakOperationConfig),
    CallAndSay(CallAndSayOperationConfig),
    FlightSearch(FlightSearchOperationConfig),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum PerRequestCaps {
    Endpoint,
    Speak {
        max_chars: u32,
    },
    CallAndSay {
        max_message_chars: u32,
        max_duration_seconds: u32,
    },
    FlightSearch {
        max_offers: u32,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperationLimits {
    pub per_request: PerRequestCaps,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub per_user_per_day: Option<u32>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperationBilling {
    pub metric: BillingMetric,
    pub price_per_unit: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_fee_per_call: Option<String>,
    pub lago_metric_code: String,
    #[serde(default)]
    pub sync_status: PricingSyncStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sync_error: Option<String>,
}

impl OperationBilling {
    pub fn free(metric: BillingMetric) -> Self {
        Self {
            metric,
            price_per_unit: "0".to_string(),
            base_fee_per_call: None,
            lago_metric_code: String::new(),
            sync_status: PricingSyncStatus::Pending,
            sync_error: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PlatformOperationKind {
    Endpoint {
        method: String,
        path_template: String,
        name: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        description: Option<String>,
    },
    Constrained {
        op: ConstrainedOp,
        config: ConstrainedConfig,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlatformOperationRow {
    #[serde(rename = "_id")]
    pub id: String,
    pub catalog_service_id: String,
    pub kind_key: String,
    #[serde(default)]
    pub enabled: bool,
    pub kind: PlatformOperationKind,
    pub limits: OperationLimits,
    pub billing: OperationBilling,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub billing_cleanup_metric_code: Option<String>,
    pub created_by: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

impl PlatformOperationRow {
    #[allow(clippy::too_many_arguments)]
    pub fn new_endpoint(
        catalog_service_id: String,
        method: String,
        path_template: String,
        name: String,
        description: Option<String>,
        limits: OperationLimits,
        billing: OperationBilling,
        created_by: String,
    ) -> Self {
        let method = method.trim().to_ascii_uppercase();
        let kind_key = endpoint_kind_key(&method, &path_template);
        let now = Utc::now();
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            catalog_service_id,
            kind_key,
            enabled: false,
            kind: PlatformOperationKind::Endpoint {
                method,
                path_template,
                name,
                description,
            },
            limits,
            billing,
            billing_cleanup_metric_code: None,
            created_by,
            created_at: now,
            updated_at: now,
        }
    }

    pub fn new_constrained(
        catalog_service_id: String,
        op: ConstrainedOp,
        config: ConstrainedConfig,
        limits: OperationLimits,
        billing: OperationBilling,
        created_by: String,
    ) -> Self {
        let now = Utc::now();
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            catalog_service_id,
            kind_key: constrained_kind_key(op),
            enabled: false,
            kind: PlatformOperationKind::Constrained { op, config },
            limits,
            billing,
            billing_cleanup_metric_code: None,
            created_by,
            created_at: now,
            updated_at: now,
        }
    }
}

pub fn constrained_op_name(op: ConstrainedOp) -> &'static str {
    match op {
        ConstrainedOp::Speak => "speak",
        ConstrainedOp::CallAndSay => "call_and_say",
        ConstrainedOp::FlightSearch => "flight_search",
    }
}

pub fn constrained_kind_key(op: ConstrainedOp) -> String {
    format!("constrained:{}", constrained_op_name(op))
}

pub fn endpoint_kind_key(method: &str, normalized_path_template: &str) -> String {
    format!(
        "endpoint:{} {}",
        method.trim().to_ascii_uppercase(),
        normalized_path_template
    )
}

// These combined config types are in-memory projections for the existing constrained
// handlers. Persisted rows always split configuration from limits above.
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
    #[serde(default = "default_call_max_duration_seconds")]
    pub max_duration_seconds: u32,
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
    Speak(SpeakConfig),
    CallAndSay(CallAndSayConfig),
    FlightSearch(FlightSearchConfig),
}

#[derive(Clone, Debug)]
pub struct PlatformOperation {
    pub id: String,
    pub catalog_service_id: String,
    pub op: PlatformOperationName,
    pub enabled: bool,
    pub vendor_service_slug: String,
    pub config: PlatformOperationConfig,
    pub billing: OperationBilling,
    pub billing_cleanup_metric_code: Option<String>,
    pub updated_at: DateTime<Utc>,
    pub updated_by: String,
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

pub const fn default_call_max_duration_seconds() -> u32 {
    600
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
    fn v2_endpoint_row_derives_kind_key_from_normalized_identity() {
        let row = PlatformOperationRow::new_endpoint(
            "catalog-duffel".to_string(),
            "post".to_string(),
            "/air/offer_requests".to_string(),
            "Create offer request".to_string(),
            None,
            OperationLimits {
                per_request: PerRequestCaps::Endpoint,
                per_user_per_day: None,
            },
            OperationBilling::free(BillingMetric::Requests),
            "admin-user".to_string(),
        );

        assert_eq!(row.kind_key, "endpoint:POST /air/offer_requests");
    }

    #[test]
    fn v2_bson_roundtrip_preserves_tagged_kind_limits_billing_and_datetimes() {
        let mut row = PlatformOperationRow::new_constrained(
            "catalog-elevenlabs".to_string(),
            ConstrainedOp::Speak,
            ConstrainedConfig::Speak(SpeakOperationConfig {
                allowed_voice_ids: vec!["voice-a".to_string()],
                model_id: "eleven_multilingual_v2".to_string(),
            }),
            OperationLimits {
                per_request: PerRequestCaps::Speak { max_chars: 500 },
                per_user_per_day: Some(25),
            },
            OperationBilling::free(BillingMetric::Requests),
            "admin-user".to_string(),
        );
        row.enabled = true;
        row.created_at = DateTime::from_timestamp_millis(row.created_at.timestamp_millis())
            .expect("valid created_at milliseconds");
        row.updated_at = DateTime::from_timestamp_millis(row.updated_at.timestamp_millis())
            .expect("valid updated_at milliseconds");

        let document = bson::to_document(&row).expect("serialize platform operation row");
        let restored: PlatformOperationRow =
            bson::from_document(document).expect("deserialize platform operation row");

        assert_eq!(restored, row);
        assert_eq!(restored.kind_key, "constrained:speak");
        assert_eq!(
            restored.created_at.timestamp_millis(),
            row.created_at.timestamp_millis()
        );
    }

    #[test]
    fn persisted_variants_reject_unknown_fields() {
        let kind = serde_json::json!({
            "kind": "endpoint",
            "method": "GET",
            "path_template": "/items/{id}",
            "name": "Get item",
            "caller_supplied_regex": ".*",
        });
        assert!(serde_json::from_value::<PlatformOperationKind>(kind).is_err());

        let caps = serde_json::json!({
            "type": "call_and_say",
            "max_message_chars": 100,
            "max_duration_seconds": 60,
            "record": true,
        });
        assert!(serde_json::from_value::<PerRequestCaps>(caps).is_err());
    }

    #[test]
    fn compatibility_config_rejects_unknown_fields() {
        let value = serde_json::json!({
            "type": "speak",
            "allowed_voice_ids": ["voice-a"],
            "max_chars": 100,
            "model_id": "eleven_multilingual_v2",
            "caller_supplied_body": true,
        });
        assert!(serde_json::from_value::<PlatformOperationConfig>(value).is_err());
    }
}

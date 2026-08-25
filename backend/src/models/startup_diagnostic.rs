use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "startup_diagnostics";
pub const EXACT_SERVICE_SEMANTIC_EFFECT_INDEX: &str = "exact_service_semantic_effect_index";

/// Durable operator-facing state for a startup check that could not safely
/// install an optional enforcement mechanism. Active diagnostics are surfaced
/// on the admin Integrity page until a later startup verifies the condition is
/// resolved.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct StartupDiagnostic {
    #[serde(rename = "_id")]
    pub id: String,
    pub active: bool,
    pub code: String,
    pub summary: String,
    pub detail: String,
    pub remediation: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub detected_at: DateTime<Utc>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub resolved_at: Option<DateTime<Utc>>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

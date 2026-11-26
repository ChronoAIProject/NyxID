use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::platform_model_tariff::{ModelChargeVector, NativeUsageEvidence};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UsageCheckpointProtocol {
    ChatCompletions,
    Responses,
    Messages,
    Gemini,
    Unknown,
}

/// Numeric observations are deliberately separate from final meter quantities.
/// Only a protocol-confirmed final report may be promoted by recovery.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct UsageCheckpoint {
    pub version: i32,
    pub sequence: i64,
    pub protocol: UsageCheckpointProtocol,
    pub final_report: bool,
    pub evidence: NativeUsageEvidence,
    #[serde(default)]
    pub model_charges: Option<Box<ModelChargeVector>>,
    #[serde(default)]
    pub recovered: bool,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub observed_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub recover_after: DateTime<Utc>,
}

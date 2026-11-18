//! Immutable CMA tariff versions; synchronization metadata may advance.
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::{sandbox_billing::SandboxPriceSnapshot, service_billing::PricingSyncStatus};

pub const COLLECTION_NAME: &str = "sandbox_rate_cards";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SandboxRateCard {
    #[serde(rename = "_id")]
    pub id: String,
    pub version: i64,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub effective_from: DateTime<Utc>,
    pub plan_code: String,
    pub prices: Vec<SandboxPriceSnapshot>,
    pub sync_status: PricingSyncStatus,
    pub created_by: Option<String>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
}

//! Service-owned CMA producer authority, separate from price data so exclusion
//! and historical receipt recovery do not depend on paid catalog dependencies.
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "sandbox_billing_settings";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SandboxBillingSettings {
    /// The catalog service UUID that owns these settings.
    #[serde(rename = "_id")]
    pub service_id: String,
    pub authorized_producer_service_account_id: Option<String>,
    pub revision: i64,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

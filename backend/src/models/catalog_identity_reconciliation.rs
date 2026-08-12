use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const FIELD_NAME: &str = "identity_reconciliation";

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CatalogIdentityReconciliationStatus {
    Pending,
    Failed,
}

/// Durable, value-free reconciliation state embedded in a catalog service.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct CatalogIdentityReconciliation {
    pub status: CatalogIdentityReconciliationStatus,
    pub fields: Vec<String>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub revision: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub requested_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_stage: Option<String>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub failed_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub matched_count: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub modified_count: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub skipped_customized_count: Option<i64>,
}

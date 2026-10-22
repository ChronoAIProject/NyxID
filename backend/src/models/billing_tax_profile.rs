//! Read-only Lago tax configuration used for gross wallet reservations.
use crate::models::credits::Credits;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use utoipa::ToSchema;

pub const COLLECTION_NAME: &str = "billing_tax_profiles";

/// `rate_percent` uses Credits as an exact decimal percentage, not a money amount.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
pub struct TaxComponent {
    pub code: String,
    pub name: String,
    #[serde(with = "crate::models::credits::whole")]
    pub rate_percent: Credits,
}

/// `rate_percent` uses Credits as an exact decimal percentage, not a money amount.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
pub struct AppliedUsageTax {
    pub taxes: Vec<TaxComponent>,
    #[serde(with = "crate::models::credits::whole")]
    pub rate_percent: Credits,
    pub fingerprint: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BillingTaxProfile {
    #[serde(rename = "_id")]
    pub id: String,
    pub owner_id: String,
    pub subscription_id: String,
    pub plan_code: String,
    pub billing_entity_code: String,
    pub default_taxes: Vec<TaxComponent>,
    /// Includes plan/customer/entity inheritance for each known metric.
    pub metric_taxes: BTreeMap<String, Vec<TaxComponent>>,
    /// Empty only for an unverified cold-cache failure marker.
    pub fingerprint: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub fetched_at: DateTime<Utc>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub refresh_failed_at: Option<DateTime<Utc>>,
}

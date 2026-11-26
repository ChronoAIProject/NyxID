//! Ordering state only; never a tax profile.
use serde::{Deserialize, Serialize};
pub const COLLECTION_NAME: &str = "billing_tax_plan_epoch";
/// A provider read can discover a different subscription plan.
pub const EPOCH_ID: &str = "all-plans";
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BillingTaxPlanEpoch {
    #[serde(rename = "_id")]
    pub id: String,
    #[serde(default)]
    pub generation: i64,
    #[serde(default)]
    pub guard_token: String,
}

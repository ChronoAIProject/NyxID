use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Frozen concrete services sharing a ceiling within an allocation's total.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CreditServiceLimit {
    pub key: String,
    pub service_ids: Vec<String>,
    pub service_slugs: Vec<String>,
    pub label: Option<String>,
    pub limit_micros: i64,
    #[serde(default)]
    pub consumed_micros: i64,
    #[serde(default)]
    pub reserved_micros: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct CreditServiceLimitInput {
    pub service_refs: Vec<String>,
    pub amount_credits: i64,
    pub label: Option<String>,
}

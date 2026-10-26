use crate::models::credits::Credits;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// A ceiling within a shared allocation, never a separate pot of money.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, ToSchema)]
pub struct CreditServiceLimitInput {
    pub service_refs: Vec<String>,
    pub amount: Credits,
    pub label: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CreditServiceLimit {
    pub key: String,
    pub service_ids: Vec<String>,
    pub service_slugs: Vec<String>,
    pub label: Option<String>,
    #[serde(with = "crate::models::credits::whole")]
    pub limit: Credits,
    #[serde(default, with = "crate::models::credits::whole")]
    pub consumed: Credits,
    #[serde(default, with = "crate::models::credits::whole")]
    pub reserved: Credits,
}

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BillingTargetKind {
    AllUsers,
    SelectedUsers,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
pub struct BillingServiceScope {
    #[serde(default)]
    pub all_services: bool,
    #[serde(default)]
    pub service_ids: Vec<String>,
    #[serde(default)]
    pub service_slugs: Vec<String>,
}

//! Owner-agnostic, persisted specialist operation authority.
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::downstream_service::ProxyOperationRule;

pub type OperationScopes = BTreeMap<String, AgentOperationScope>;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentOperationScope {
    pub revision: i64,
    /// Catalog identity for equivalent catalog/provider routes.
    pub catalog_service_id: Option<String>,
    pub operations: Vec<ScopedOperation>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScopedOperation {
    /// None only for an explicit rule on a service without endpoint rows.
    pub endpoint_id: Option<String>,
    pub rule: ProxyOperationRule,
    #[serde(default)]
    pub risk: Option<super::service_endpoint::EndpointRisk>,
    #[serde(default)]
    pub destructive: bool,
    #[serde(default)]
    pub changes_existing: Option<bool>,
}

/// Closed management input; compiled rules are always resolved by the server.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OperationSelection {
    pub expected_revision: i64,
    #[serde(default)]
    pub all_operations: bool,
    #[serde(default)]
    pub endpoint_ids: Vec<String>,
    #[serde(default)]
    pub rules: Vec<ProxyOperationRule>,
}

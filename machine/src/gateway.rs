//! Server-authored per-job environment. Nodes know paths, not catalog slugs.
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
pub struct Environment {
    #[serde(default)]
    pub variables: BTreeMap<String, Variable>,
    #[serde(default)]
    pub git: Vec<GitRewrite>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(tag = "kind", content = "path", rename_all = "snake_case")]
pub enum Variable {
    GatewayPath(String),
    GatewayToken,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct GitRewrite {
    pub origin: String,
    pub path: String,
}

/// Optional, negotiated controls carried inside the signed ServiceCall metadata.
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionApproval {
    pub version: u32,
    pub mode: ApprovalMode,
    pub waiter_until_ms: i64,
}
#[derive(Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ApprovalMode {
    Connected,
    Async,
}

#[cfg(test)]
mod execution_approval_tests {
    use super::*;
    #[test]
    fn machine_approval_controls_are_closed_and_do_not_change_legacy_envelopes() {
        let mut request = serde_json::json!({"version":1,"mode":"connected","waiter_until_ms":100});
        assert!(serde_json::from_value::<ExecutionApproval>(request.clone()).is_ok());
        request["actor_id"] = serde_json::json!("untrusted");
        assert!(serde_json::from_value::<ExecutionApproval>(request).is_err());
        assert!(
            serde_json::from_value::<ExecutionApproval>(
                serde_json::json!({"version":1,"mode":"automatic","waiter_until_ms":100})
            )
            .is_err()
        );
    }
}

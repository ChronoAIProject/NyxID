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

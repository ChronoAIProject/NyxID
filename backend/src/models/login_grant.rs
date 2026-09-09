use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
#[schema(as = AgentKeyLoginNewKeyInput)]
#[serde(deny_unknown_fields)]
pub struct NewKeyInput {
    pub name: String,
    pub scopes: String,
    #[serde(default)]
    pub allowed_service_ids: Vec<String>,
    #[serde(default)]
    pub allowed_node_ids: Vec<String>,
    #[serde(default)]
    pub allow_all_services: bool,
    #[serde(default)]
    pub allow_all_nodes: bool,
    pub expires_at: Option<String>,
    pub rate_limit_per_second: Option<u32>,
    pub rate_limit_burst: Option<u32>,
    pub platform: Option<String>,
    pub target_org_id: Option<String>,
    pub scope_plan_digest: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
#[schema(as = AgentKeyLoginSelection)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Selection {
    Existing { api_key_id: String },
    New(NewKeyInput),
}

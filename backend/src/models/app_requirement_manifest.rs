use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "app_requirement_manifests";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppRequirementManifest {
    #[serde(rename = "_id")]
    pub id: String,
    pub oauth_client_id: String,
    pub version: u32,
    pub enforcement: Enforcement,
    pub requirements: Vec<ServiceRequirement>,
    pub compiled: CompiledManifest,
    pub published_by: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub published_at: DateTime<Utc>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Enforcement {
    Gate,
    Advise,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceRequirement {
    pub id: String,
    pub label: String,
    #[serde(default)]
    pub any_of_catalog_slugs: Vec<String>,
    #[serde(default)]
    pub any_of_catalog_prefix: Option<String>,
    pub owner_policy: OwnerPolicy,
    #[serde(default)]
    pub accepted_credential_types: Vec<String>,
    #[serde(default)]
    pub allow_master_credential: bool,
    #[serde(default)]
    pub allow_no_credential: bool,
    #[serde(default)]
    pub required_downstream_scopes: Vec<String>,
    pub validator: ValidatorSelection,
    #[serde(default)]
    pub optional: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OwnerPolicy {
    PersonalOnly,
    PersonalOrOrgAllowed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ValidatorSelection {
    Profile { id: String },
    StoredOnly,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompiledManifest {
    pub catalog_service_ids: BTreeMap<String, String>,
    pub validator_versions: BTreeMap<String, u32>,
    #[serde(default)]
    pub validators_by_requirement: BTreeMap<String, BTreeMap<String, String>>,
}

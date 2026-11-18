//! Platform offering metadata. Private routing never belongs in a public DTO.
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "platform_models";
pub const ROUTING_COLLECTION: &str = "platform_model_routing_revisions";

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Lifecycle {
    #[default]
    Draft,
    Active,
    Retired,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ModelFamily {
    Gemini,
    Claude,
    Openai,
    OpenModel,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CallerProtocol {
    OpenaiChat,
    OpenaiResponses,
    AnthropicMessages,
    Gemini,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Modality {
    Text,
    Image,
    Audio,
    Video,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Capabilities {
    #[serde(default)]
    pub protocols: Vec<CallerProtocol>,
    #[serde(default)]
    pub input_modalities: Vec<Modality>,
    #[serde(default)]
    pub output_modalities: Vec<Modality>,
    pub context_tokens: Option<u32>,
    pub output_tokens: Option<u32>,
    pub streaming: Option<bool>,
    pub tools: Option<bool>,
    pub structured_output: Option<bool>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum SubjectKind {
    Person,
    Org,
    Group,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Effect {
    Allow,
    Deny,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct EntitlementRule {
    pub subject_kind: SubjectKind,
    pub subject_id: String,
    pub effect: Effect,
}

/// Embedded with the model so kill switch, policy and revision have one CAS.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Entitlements {
    #[serde(default)]
    pub all_people: bool,
    #[serde(default)]
    pub rules: Vec<EntitlementRule>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct PlatformModel {
    #[serde(rename = "_id")]
    pub id: String,
    pub public_id: String,
    pub display_name: String,
    pub description: String,
    pub family: ModelFamily,
    #[serde(default)]
    pub capabilities: Capabilities,
    #[serde(default)]
    pub lifecycle: Lifecycle,
    #[serde(default)]
    pub enabled: bool,
    pub revision: i64,
    pub authorization_revision: i64,
    pub routing_revision_id: Option<String>,
    pub billing_service_id: Option<String>,
    #[serde(default)]
    pub entitlements: Entitlements,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum VertexPublisher {
    Google,
    Anthropic,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DirectProvider {
    Openai,
    Anthropic,
}

/// Closed destinations, never caller-provided URLs or nested/user-owned pools.
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum RouteTarget {
    VertexPublisher {
        publisher: VertexPublisher,
        project: String,
        location: String,
        native_model: String,
    },
    VertexOpenapiMaas {
        project: String,
        location: String,
        publisher: String,
        native_model: String,
    },
    DirectProvider {
        provider: DirectProvider,
        native_model: String,
    },
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelRoute {
    pub id: String,
    /// Existing encrypted catalog master record; never the material itself.
    pub credential_service_id: String,
    pub enabled: bool,
    pub priority: u16,
    pub weight: u16,
    pub target: RouteTarget,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct RoutingRevision {
    #[serde(rename = "_id")]
    pub id: String,
    pub model_id: String,
    pub model_revision: i64,
    pub routes: Vec<ModelRoute>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
}

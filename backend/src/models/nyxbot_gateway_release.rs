use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "nyxbot_gateway_releases";
pub const RECONCILE_ID: &str = "nyxbot-gateway-release-reconcile";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReleaseState {
    Provisional,
    Due,
    ReviewRequired,
    Settled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReleaseReason {
    Setup,
    Disconnect,
    OrgAccessLost,
    Fallback,
    TransportMove,
    SetupFailed,
    StaleSetup,
    Reconcile,
}

/// Immutable cleanup authority. Observations never widen the captured version.
#[derive(Clone, Serialize, Deserialize)]
pub struct NyxbotGatewayRelease {
    #[serde(default)]
    pub gateway_source: Option<super::nyxbot_channel::GatewaySource>,
    #[serde(rename = "_id")]
    pub id: String,
    pub schema_version: i32,
    pub owner_user_id: String,
    pub channel_id: String,
    pub channel_bot_id: String,
    pub bot_owner_id: String,
    pub platform: String,
    pub gateway_service_id: String,
    pub gateway_base_url: String,
    pub creator_issuer: String,
    pub record_id: Option<String>,
    pub gateway_channel_id: Option<String>,
    pub binding_id: String,
    pub agent_api_key_id: String,
    pub route_api_key_id: String,
    pub expected_version: Option<i64>,
    pub state: ReleaseState,
    pub reason: ReleaseReason,
    #[serde(with = "crate::models::bson_datetime::optional")]
    pub dispatch_started_at: Option<DateTime<Utc>>,
    #[serde(with = "crate::models::bson_datetime::optional")]
    pub provisional_expires_at: Option<DateTime<Utc>>,
    #[serde(with = "crate::models::bson_datetime::optional")]
    pub due_at: Option<DateTime<Utc>>,
    #[serde(with = "crate::models::bson_datetime::optional")]
    pub next_attempt_at: Option<DateTime<Utc>>,
    pub attempt_count: i64,
    pub claim_id: Option<String>,
    #[serde(with = "crate::models::bson_datetime::optional")]
    pub lease_until: Option<DateTime<Utc>>,
    pub revision: i64,
    pub last_error: Option<String>,
    pub observed_version: Option<i64>,
    pub resolution: Option<String>,
    pub reviewed_by: Option<String>,
    #[serde(with = "crate::models::bson_datetime::optional")]
    pub settled_at: Option<DateTime<Utc>>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

impl std::fmt::Debug for NyxbotGatewayRelease {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NyxbotGatewayRelease")
            .field("id", &self.id)
            .field("channel_id", &self.channel_id)
            .field("state", &self.state)
            .field("revision", &self.revision)
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReleaseReconcileCursor {
    #[serde(rename = "_id")]
    pub id: String,
    pub last_id: Option<String>,
    pub claim_id: Option<String>,
    #[serde(with = "crate::models::bson_datetime::optional")]
    pub lease_until: Option<DateTime<Utc>>,
    #[serde(with = "crate::models::bson_datetime::optional")]
    pub pass_started_at: Option<DateTime<Utc>>,
    #[serde(with = "crate::models::bson_datetime::optional")]
    pub pass_completed_at: Option<DateTime<Utc>>,
}

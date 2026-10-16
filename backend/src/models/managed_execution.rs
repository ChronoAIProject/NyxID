use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use super::{
    credits::Credits,
    service_billing::{PlatformUsage, ServiceBilling},
};

pub const COLLECTION_NAME: &str = "managed_executions";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum ManagedExecutionState {
    Prepared,
    Funded,
    DispatchAdmitted,
    Pending,
    EvidenceReady,
    Cancelling,
    Released,
    Settled,
}

/// Metadata and validated usage only; request and response content are never stored.
#[derive(Clone, Serialize, Deserialize)]
pub struct ManagedExecution {
    #[serde(rename = "_id")]
    pub id: String,
    pub billing_request_id: String,
    pub billing_owner_id: String,
    pub actor_user_id: String,
    pub api_key_id: Option<String>,
    pub user_service_id: Option<String>,
    pub catalog_service_id: String,
    pub service_slug: Option<String>,
    pub ingress: String,
    pub request_bytes: i64,
    pub request_digest: String,
    pub catalog_digest: String,
    pub route_digest: String,
    pub billing: ServiceBilling,
    pub state: ManagedExecutionState,
    #[serde(with = "super::credits::whole")]
    pub maximum: Credits,
    pub meter_row_ids: Vec<String>,
    pub usage: Option<PlatformUsage>,
    pub provider_request_id: Option<String>,
    pub pending_reason: Option<String>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub next_reconcile_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

impl std::fmt::Debug for ManagedExecution {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ManagedExecution")
            .field("state", &self.state)
            .finish_non_exhaustive()
    }
}

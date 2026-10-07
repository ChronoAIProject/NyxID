//! NyxID-owned async contracts and durable assistant watches. No result plaintext.
use super::assistant_conversation::{ChannelOrigin, TurnOrigin};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "assistant_async_operations";
pub const QUOTAS_COLLECTION_NAME: &str = "assistant_async_quotas";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AsyncOperationContract {
    pub status_operation: String,
    pub result_operation: String,
    pub cancel_operation: Option<String>,
    pub id_field: String,
    pub id_parameter: String,
    pub status_field: String,
    pub success_states: Vec<String>,
    pub failure_states: Vec<String>,
    pub error_field: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct DeliveryBinding {
    pub origin: TurnOrigin,
    pub channel: Option<ChannelOrigin>,
    pub reply_channel: Option<ChannelOrigin>,
    pub group_id: Option<String>,
    pub group_request_id: Option<String>,
    pub voice_request_id: Option<String>,
    pub trigger_run_id: Option<String>,
    pub report_to: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct AsyncServiceOperation {
    #[serde(rename = "_id")]
    pub id: String,
    pub user_id: String,
    pub conversation_id: String,
    pub agent_id: String,
    pub turn_id: String,
    pub api_key_id: String,
    pub service_id: String,
    pub submit_endpoint_id: String,
    pub contract: AsyncOperationContract,
    pub delivery: DeliveryBinding,
    pub operation_id: Option<String>,
    /// submitting, waiting, cancelling, ready, queued, delivered, cancelled.
    pub state: String,
    pub reason: Option<String>,
    pub attempts: u32,
    pub lease_id: Option<String>,
    #[serde(with = "crate::models::bson_datetime::optional")]
    pub lease_until: Option<DateTime<Utc>>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub deadline: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub poll_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
    #[serde(default)]
    pub result_encrypted: Option<Vec<u8>>,
}

impl std::fmt::Debug for AsyncServiceOperation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AsyncServiceOperation { [REDACTED] }")
    }
}

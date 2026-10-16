use super::credits::Credits;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const LOOPS: &str = "platform_public_budget_loops";
pub const CALLS: &str = "platform_public_call_reservations";
pub const BUCKETS: &str = "platform_public_budget_buckets";

#[derive(Clone, Serialize, Deserialize)]
pub struct PublicBudgetLoop {
    #[serde(rename = "_id")]
    pub id: String,
    pub public_session_id: String,
    pub conversation_id: String,
    pub turn_id: String,
    pub receipt_id: String,
    pub claim_id: String,
    pub profile_id: String,
    pub sender_id: i64,
    pub sponsor_owner_user_id: String,
    pub credential_api_key_id: String,
    pub policy_digest: String,
    pub bucket_ids: Vec<String>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub day: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
    #[serde(with = "super::credits::whole")]
    pub maximum: Credits,
    #[serde(with = "super::credits::whole")]
    pub held: Credits,
    #[serde(with = "super::credits::whole")]
    pub allocated: Credits,
    #[serde(with = "super::credits::whole")]
    pub spent: Credits,
    pub calls: i64,
    pub max_calls: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub max_input_tokens: i64,
    pub max_output_tokens: i64,
    #[serde(default)]
    pub tool_calls: i64,
    #[serde(default)]
    pub max_tool_calls: i64,
    pub closed: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PublicCallState {
    Prepared,
    Funded,
    DispatchAdmitted,
    Cancelling,
    Released,
    Conservative,
    Settled,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct PublicCallReservation {
    #[serde(rename = "_id")]
    pub id: String,
    pub loop_id: String,
    pub sponsor_owner_user_id: String,
    pub credential_api_key_id: String,
    pub billing_request_id: String,
    pub route_digest: String,
    pub model_id: String,
    pub state: PublicCallState,
    #[serde(with = "super::credits::whole")]
    pub maximum: Credits,
    #[serde(with = "super::credits::whole")]
    pub cap_charge: Credits,
    pub meter_row_ids: Vec<String>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub next_reconcile_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct PublicBudgetBucket {
    #[serde(rename = "_id")]
    pub id: String,
    pub scope_key: String,
    #[serde(with = "super::credits::whole")]
    pub held: Credits,
    #[serde(with = "super::credits::whole")]
    pub spent: Credits,
}

macro_rules! redacted_debug {
    ($($ty:ty),*) => { $(impl std::fmt::Debug for $ty {
        fn fmt(&self,f:&mut std::fmt::Formatter<'_>)->std::fmt::Result {
            f.write_str(concat!(stringify!($ty), " { [REDACTED] }"))
        }
    })* };
}
redacted_debug!(PublicBudgetLoop, PublicCallReservation, PublicBudgetBucket);

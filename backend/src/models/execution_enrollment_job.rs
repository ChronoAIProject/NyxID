use super::bson_datetime;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "execution_enrollment_jobs";

/// One coalesced, leased scan per organization. No request or credential data.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExecutionEnrollmentJob {
    #[serde(rename = "_id")]
    pub id: String,
    pub org_id: String,
    pub generation: String,
    pub state: String,
    #[serde(default)]
    pub cursor: Option<String>,
    pub processed: i64,
    pub lease_token: Option<String>,
    #[serde(default, with = "bson_datetime::optional")]
    pub lease_until: Option<DateTime<Utc>>,
    #[serde(default)]
    pub last_error: Option<String>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

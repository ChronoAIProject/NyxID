use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "pool_recovery_diagnostics";

#[derive(Clone, Serialize, Deserialize)]
pub struct PoolRecoveryFailure {
    pub request_id: Option<String>,
    pub detail: String,
}

/// One bounded recent-error summary, exposed through billing Integrity.
#[derive(Clone, Serialize, Deserialize)]
pub struct PoolRecoveryDiagnostic {
    #[serde(rename = "_id")]
    pub id: String,
    pub name: String,
    pub failures: i64,
    pub samples: Vec<PoolRecoveryFailure>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

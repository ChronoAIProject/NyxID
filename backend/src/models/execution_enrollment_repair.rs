use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A deferred hint for one affected person, retained after membership deletion.
/// This is a coalesced repair receipt, never an enrollment or authority token.
pub const COLLECTION_NAME: &str = "execution_enrollment_repairs";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ExecutionEnrollmentRepair {
    #[serde(rename = "_id")]
    pub id: String,
    pub actor_id: String,
    pub generation: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub retry_after: DateTime<Utc>,
}

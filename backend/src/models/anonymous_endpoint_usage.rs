use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "anonymous_endpoint_usage";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct AnonymousEndpointUsage {
    #[serde(rename = "_id")]
    pub id: String,
    pub service_id: String,
    pub rule_id: String,
    /// UTC day in YYYY-MM-DD form.
    pub day: String,
    pub count: i64,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

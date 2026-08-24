use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::platform_operation::PlatformOperationName;

pub const COLLECTION_NAME: &str = "platform_op_usage";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PlatformOpUsage {
    #[serde(rename = "_id")]
    pub id: String,
    pub op: PlatformOperationName,
    pub user_id: String,
    pub yyyymmdd: String,
    pub count: u32,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

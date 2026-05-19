use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::models::bson_datetime;

pub const COLLECTION_NAME: &str = "node_pending_credentials";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum InjectionMethod {
    Header,
    QueryParam,
    PathPrefix,
}

impl InjectionMethod {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Header => "header",
            Self::QueryParam => "query-param",
            Self::PathPrefix => "path-prefix",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NodePendingCredential {
    #[serde(rename = "_id")]
    pub id: String,
    pub node_id: String,
    pub service_slug: String,
    pub injection_method: InjectionMethod,
    pub field_name: String,
    pub target_url: Option<String>,
    pub label: Option<String>,
    pub created_by_user_id: String,
    pub owner_user_id: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
    #[serde(default, with = "bson_datetime::optional")]
    pub consumed_at: Option<DateTime<Utc>>,
    #[serde(default, with = "bson_datetime::optional")]
    pub declined_at: Option<DateTime<Utc>>,
    pub is_active: bool,
}

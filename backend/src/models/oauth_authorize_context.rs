use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::app_connect_link::ValidatedAuthorizeParams;

pub const COLLECTION_NAME: &str = "oauth_authorize_contexts";

#[derive(Clone, Serialize, Deserialize)]
pub struct OauthAuthorizeContext {
    #[serde(rename = "_id")]
    pub id: String,
    pub authorize_params: ValidatedAuthorizeParams,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub consumed_at: Option<DateTime<Utc>>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
}

impl std::fmt::Debug for OauthAuthorizeContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OauthAuthorizeContext")
            .field("id", &"[REDACTED]")
            .field("authorize_params", &"[REDACTED]")
            .field("expires_at", &self.expires_at)
            .finish_non_exhaustive()
    }
}

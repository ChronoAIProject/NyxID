use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "oauth_consent_requests";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct OauthConsentRequest {
    #[serde(rename = "_id")]
    pub id: String,
    pub user_id: String,
    pub signed_request: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
}

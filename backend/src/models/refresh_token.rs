use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "refresh_tokens";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefreshToken {
    #[serde(rename = "_id")]
    pub id: String,
    /// JWT ID (jti) for this refresh token
    pub jti: String,
    pub client_id: String,
    pub user_id: String,
    pub session_id: Option<String>,
    pub expires_at: DateTime<Utc>,
    pub revoked: bool,
    pub replaced_by: Option<String>,
    pub created_at: DateTime<Utc>,
}

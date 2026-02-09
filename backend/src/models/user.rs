use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "users";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct User {
    #[serde(rename = "_id")]
    pub id: String,
    pub email: String,
    #[serde(skip_serializing)]
    pub password_hash: Option<String>,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
    pub email_verified: bool,
    #[serde(skip_serializing)]
    pub email_verification_token: Option<String>,
    #[serde(skip_serializing)]
    pub password_reset_token: Option<String>,
    #[serde(skip_serializing)]
    pub password_reset_expires_at: Option<DateTime<Utc>>,
    pub is_active: bool,
    pub is_admin: bool,
    pub mfa_enabled: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub last_login_at: Option<DateTime<Utc>>,
}

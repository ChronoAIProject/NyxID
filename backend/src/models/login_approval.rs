use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "login_approvals";

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum LoginFlow {
    Device,
    AgentKey,
}

/// A browser-bound identity proof for one pending request. Never an account session.
#[derive(Clone, Deserialize, Serialize)]
pub struct LoginApproval {
    #[serde(rename = "_id")]
    pub id: String,
    pub browser_hash: String,
    pub flow: LoginFlow,
    pub user_code: String,
    pub request_id: String,
    pub keep_signed_in: bool,
    pub user_id: Option<String>,
    pub verified: bool,
    pub mfa_verified: bool,
    pub closed: bool,
    pub social_state_hash: Option<String>,
    pub attempts: u32,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
}

impl std::fmt::Debug for LoginApproval {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoginApproval").finish_non_exhaustive()
    }
}

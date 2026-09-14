use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "app_connect_links";

#[derive(Clone, Serialize, Deserialize)]
pub struct AppConnectLink {
    #[serde(rename = "_id")]
    pub id: String,
    pub oauth_client_id: String,
    pub user_id: String,
    pub manifest_id: String,
    pub manifest_version: u32,
    pub origin: AppConnectOrigin,
    pub items: Vec<AppConnectItem>,
    pub status: AppConnectStatus,
    pub capability_hash: String,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub redeemed_at: Option<DateTime<Utc>>,
    /// All item and terminal writes compare this revision, including child cancellation.
    pub revision: i64,
    pub result_id: Option<String>,
    pub grant_update_required: bool,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub completed_at: Option<DateTime<Utc>>,
    pub failure_reason: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AppConnectOrigin {
    App {
        callback_url: String,
        state: String,
    },
    /// Reserved storage shape. No authorize-origin sessions are created in phase 1.
    Authorize {
        authorize_params: ValidatedAuthorizeParams,
        consent_nonce: String,
    },
}

#[derive(Clone, Serialize, Deserialize)]
pub struct ValidatedAuthorizeParams {
    pub client_id: String,
    pub redirect_uri: String,
    pub scope: String,
    pub state: Option<String>,
    pub nonce: Option<String>,
    pub code_challenge: Option<String>,
    pub code_challenge_method: Option<String>,
    pub resources: Vec<String>,
    pub prompt: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppConnectStatus {
    InProgress,
    ReadyForConsent,
    Completed,
    Cancelled,
    Expired,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppConnectItem {
    pub requirement_id: String,
    pub state: ItemState,
    pub connect_link_id: Option<String>,
    pub user_service_id: Option<String>,
    pub explicit_selection: bool,
    pub validation_record_id: Option<String>,
    pub attempt_id: Option<String>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub attempt_started_at: Option<DateTime<Utc>>,
    pub reason_code: Option<String>,
    /// Each requirement earns at most one TTL extension, even after repeated rechecks.
    pub extended_ttl: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemState {
    Unmet,
    Connecting,
    Reauthorizing,
    Validating,
    Met,
    Unknown,
    Failed,
    Skipped,
}

impl std::fmt::Debug for AppConnectLink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppConnectLink")
            .field("id", &self.id)
            .field("oauth_client_id", &self.oauth_client_id)
            .field("user_id", &self.user_id)
            .field("status", &self.status)
            .field("revision", &self.revision)
            .field("origin", &"[REDACTED]")
            .field("capability_hash", &"[REDACTED]")
            .finish_non_exhaustive()
    }
}

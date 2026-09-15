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
    #[serde(default)]
    pub selected_service_ids: Vec<String>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub completed_at: Option<DateTime<Utc>>,
    pub failure_reason: Option<String>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub webhook_event_reserved_at: Option<DateTime<Utc>>,
    /// Frozen at first reservation; delivery cycles never change occurrence time.
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub webhook_event_occurred_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub webhook_event_id: Option<String>,
    #[serde(default)]
    pub webhook_event_status: Option<crate::models::connect_link::ConnectLinkWebhookStatus>,
    #[serde(default)]
    pub webhook_event_attempts: u32,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub webhook_event_delivered_at: Option<DateTime<Utc>>,
    /// Safe metadata frozen at reservation; retries never reload mutable service slugs.
    #[serde(default)]
    pub webhook_event_data: Option<AppConnectWebhookData>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AppConnectWebhookData {
    Full(AppConnectWebhookFullData),
    UnredeemedExpiry(AppConnectWebhookExpiryData),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AppConnectWebhookExpiryData {
    pub app_connect_link_id: String,
    pub origin: AppConnectWebhookOrigin,
    pub status: AppConnectStatus,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppConnectWebhookFullData {
    pub user_id: String,
    pub app_connect_link_id: String,
    pub origin: AppConnectWebhookOrigin,
    pub requirements_version: u32,
    pub status: AppConnectStatus,
    pub failure_reason: Option<String>,
    pub grant_update_required: bool,
    pub items: Vec<AppConnectWebhookItem>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppConnectWebhookOrigin {
    App,
    Authorize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppConnectWebhookItem {
    pub requirement_id: String,
    pub state: ItemState,
    pub user_service_id: Option<String>,
    pub slug: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AppConnectOrigin {
    App {
        callback_url: String,
        state: String,
    },
    Authorize {
        authorize_params: Box<ValidatedAuthorizeParams>,
        consent_nonce: String,
    },
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
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
    #[serde(default)]
    pub external_subject: Option<crate::models::authorization_code::ExternalSubjectRef>,
    #[serde(default)]
    pub binding_grant_id: Option<String>,
    #[serde(default)]
    pub nyx_connect: Option<String>,
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

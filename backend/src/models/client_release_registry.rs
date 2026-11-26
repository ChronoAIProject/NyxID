//! Release metadata, not report ownership; never included in vendor envelopes.
use super::client_error_report::Platform;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
pub const COLLECTION_NAME: &str = "client_release_registry";
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ReleaseSource {
    FrontendManifest,
    Admin,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct ClientRelease {
    #[serde(rename = "_id")]
    pub id: String,
    pub platform: Platform,
    pub app_version: String,
    pub app_build: String,
    #[serde(default)]
    pub artifact_id: Option<String>,
    pub source: ReleaseSource,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub registered_at: DateTime<Utc>,
    #[serde(default)]
    pub registered_by: Option<String>,
}

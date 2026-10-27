//! Optional metadata shared by pending files and durable private attachments.
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "async_service_operation_artifacts";
pub const DELIVERIES: &str = "assistant_artifact_deliveries";

#[derive(Clone, Default, Serialize, Deserialize)]
pub struct Metadata {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub filename: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_watch_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_turn_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_operation_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_result_identity: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source_status_version: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_revision: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub preview_kind: Option<String>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "super::bson_datetime::optional"
    )]
    pub expires_at: Option<DateTime<Utc>>,
}
impl std::fmt::Debug for Metadata {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("GeneratedArtifactMetadata([REDACTED])")
    }
}

/// Owner-visible per-file outcome. Reasons are closed and never upstream text.
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RejectionReason {
    Invalid,
    TooLarge,
    Unavailable,
    Missing,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct RejectedFile {
    pub index: usize,
    pub filename: String,
    pub reason: RejectionReason,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct AcceptedFile {
    pub index: usize,
    pub pending_id: String,
    pub attachment_id: String,
    pub filename: String,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct ResultManifest {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<AcceptedFile>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub rejected: Vec<RejectedFile>,
    pub source_result_identity: String,
    pub source_status_version: Option<String>,
    pub content_revision: String,
    pub count: usize,
    pub delivery_reason: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct PendingArtifact {
    #[serde(rename = "_id")]
    pub id: String,
    pub watch_id: String,
    pub user_id: String,
    pub conversation_id: String,
    pub turn_id: String,
    pub operation_id: String,
    pub source_result_identity: String,
    pub artifact_index: usize,
    pub filename: String,
    pub source_path: String,
    pub content_type: String,
    pub size: i64,
    pub sha256: String,
    pub preview_kind: String,
    pub source_status_version: Option<String>,
    pub content_revision: String,
    pub state: String,
    pub assistant_attachment_id: String,
    #[serde(default, with = "super::bson_bytes::optional")]
    pub data_encrypted: Option<Vec<u8>>,
    #[serde(default)]
    pub secret_context_versions: std::collections::BTreeMap<String, u8>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
}
impl std::fmt::Debug for PendingArtifact {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PendingArtifact([REDACTED])")
    }
}

/// Written only by completion settlement, never from request/model arguments.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Delivery {
    #[serde(rename = "_id")]
    pub message_id: String,
    pub user_id: String,
    pub conversation_id: String,
    pub originating_turn_id: String,
    pub event_turn_id: String,
    pub watch_id: String,
    pub operation_id: String,
    pub source_result_identity: String,
    pub attachment_ids: Vec<String>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
}

/// A settled owner reply keeps bounded outcomes, not acquisition authority.
#[derive(Clone, Serialize, Deserialize)]
pub struct SavedOutcome {
    pub state: String,
    pub count: usize,
    pub rejected: Vec<RejectedFile>,
}
impl std::fmt::Debug for SavedOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SavedArtifactOutcome([REDACTED])")
    }
}

//! Metadata-only identity and dispatch barrier for managed Base writes.
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const COLLECTION_NAME: &str = "lark_base_writes";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WriteState {
    Uploading,
    Dispatching,
    Succeeded,
    FailedDefinite,
    FailedPreDispatch,
    Uncertain,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WritePrincipal {
    pub user_service_id: String,
    pub credential_id: String,
    pub epoch: i64,
    #[serde(rename = "override")]
    pub credential_override: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct UploadedFile {
    pub upload_id: String,
    pub content_sha256: String,
    pub file_token: String,
}

#[derive(Clone, Serialize, Deserialize)]
pub struct LarkBaseWrite {
    #[serde(rename = "_id")]
    pub id: String,
    pub key: String,
    pub user_id: String,
    pub attempt: String,
    pub state: WriteState,
    pub principal: WritePrincipal,
    pub contract_digests: BTreeMap<String, String>,
    pub upload_ids: Vec<String>,
    pub files: Vec<UploadedFile>,
    pub logical_request_sha256: String,
    pub create_body_sha256: Option<String>,
    pub client_token: String,
    pub record_id: Option<String>,
    pub last_code: Option<String>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub deadline_at: DateTime<Utc>,
    #[serde(default, with = "super::bson_datetime::optional")]
    pub expires_at: Option<DateTime<Utc>>,
}

macro_rules! redacted_debug {
    ($($ty:ty),+ $(,)?) => {$(
        impl std::fmt::Debug for $ty {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str(concat!(stringify!($ty), "([REDACTED])"))
            }
        }
    )+};
}
redacted_debug!(LarkBaseWrite, WritePrincipal, UploadedFile);

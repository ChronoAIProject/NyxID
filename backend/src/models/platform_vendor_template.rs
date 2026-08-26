use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "platform_vendor_templates";

/// Admin-managed provisioning metadata. This record is intentionally not the
/// runtime operation contract: operation binding always checks the code-owned
/// contract in `platform_operation_service`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PlatformVendorTemplate {
    #[serde(rename = "_id")]
    pub id: String,
    pub vendor: String,
    pub display_name: String,
    pub slug: String,
    pub base_url: String,
    pub auth_method: String,
    #[serde(default)]
    pub auth_key_name: Option<String>,
    pub credential_label: String,
    pub credential_note: String,
    #[serde(default)]
    pub operation: Option<String>,
    #[serde(default)]
    pub capability_summary: String,
    #[serde(default)]
    pub restriction_summary: String,
    pub is_active: bool,
    #[serde(default)]
    pub is_seeded: bool,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
    pub updated_by: String,
}

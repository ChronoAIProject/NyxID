use super::downstream_service::CatalogImportSource;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "catalog_spec_overlays";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CatalogSpecOverlay {
    #[serde(rename = "_id")]
    pub id: String,
    pub service_id: String,
    pub document: serde_json::Value,
    #[serde(default)]
    pub previous_document: Option<serde_json::Value>,
    pub sha256: String,
    pub revision: i64,
    pub source: Option<CatalogImportSource>,
    pub created_by: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

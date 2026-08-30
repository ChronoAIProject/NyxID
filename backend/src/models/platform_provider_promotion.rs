use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "platform_provider_promotions";

/// A row exists only after an administrator has approved the provider's
/// vendor terms and promoted that catalog service for platform use.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PlatformProviderPromotion {
    #[serde(rename = "_id")]
    pub id: String,
    pub catalog_service_id: String,
    pub vendor_terms_accepted_by: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub vendor_terms_accepted_at: DateTime<Utc>,
    pub promoted_by: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub promoted_at: DateTime<Utc>,
    pub updated_by: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

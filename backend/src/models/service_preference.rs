use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "service_preferences";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ServicePreference {
    #[serde(rename = "_id")]
    pub user_id: String,
    #[serde(default)]
    pub ordered: Vec<String>,
    #[serde(default)]
    pub version: i64,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use bson::{Bson, doc};

    #[test]
    fn service_preference_bson_dates_and_legacy_defaults() {
        let now = bson::DateTime::now();
        let mut document = doc! { "_id": "person", "created_at": now, "updated_at": now };
        let legacy: ServicePreference = bson::from_document(document.clone()).unwrap();
        assert!(legacy.ordered.is_empty());
        assert_eq!(legacy.version, 0);
        document.insert("ordered", vec![uuid::Uuid::new_v4().to_string()]);
        document.insert("version", 3_i64);
        let value: ServicePreference = bson::from_document(document.clone()).unwrap();
        let encoded = bson::to_document(&value).unwrap();
        assert!(matches!(encoded.get("created_at"), Some(Bson::DateTime(_))));
        assert!(matches!(encoded.get("updated_at"), Some(Bson::DateTime(_))));
        assert_eq!(encoded, document);
    }
}

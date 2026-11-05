use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::crm_record::CrmActor;

pub const COLLECTION_NAME: &str = "crm_saved_views";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct CrmSavedView {
    #[serde(rename = "_id")]
    pub id: String,
    pub owner_id: String,
    pub title: String,
    pub revision: i64,
    pub envelope: serde_json::Value,
    pub created_by: CrmActor,
    #[serde(default, with = "super::bson_datetime::optional")]
    pub deleted_at: Option<DateTime<Utc>>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use mongodb::bson::{self, doc};

    use super::*;

    #[test]
    fn legacy_document_without_deleted_at_deserializes_as_live() {
        let now = bson::DateTime::from_millis(1_700_000_000_000);
        let view: CrmSavedView = bson::from_document(doc! {
            "_id": "legacy-view",
            "owner_id": "owner",
            "title": "Legacy view",
            "revision": 1_i64,
            "envelope": {},
            "created_by": bson::to_bson(&CrmActor::human("owner")).unwrap(),
            "created_at": now,
            "updated_at": now,
        })
        .unwrap();

        assert_eq!(view.deleted_at, None);
    }
}

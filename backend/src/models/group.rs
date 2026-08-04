use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "groups";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Group {
    #[serde(rename = "_id")]
    pub id: String,
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    pub role_ids: Vec<String>,
    pub parent_group_id: Option<String>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collection_name() {
        assert_eq!(COLLECTION_NAME, "groups");
    }

    fn make_group() -> Group {
        Group {
            id: "550e8400-e29b-41d4-a716-446655440001".to_string(),
            name: "Engineering".to_string(),
            slug: "engineering".to_string(),
            description: Some("Engineering team".to_string()),
            role_ids: vec!["role-1".to_string()],
            parent_group_id: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn bson_roundtrip() {
        let group = make_group();
        let doc = bson::to_document(&group).expect("serialize group to bson");
        assert!(doc.get_str("_id").is_ok());
        assert!(doc.get("id").is_none(), "raw 'id' should not exist in bson");
        let restored: Group = bson::from_document(doc).expect("deserialize group from bson");
        assert_eq!(group.id, restored.id);
        assert_eq!(group.slug, restored.slug);
        assert_eq!(group.role_ids, restored.role_ids);
    }

    #[test]
    fn bson_all_fields_serialized() {
        let group = make_group();
        let doc = bson::to_document(&group).expect("serialize");
        let keys: Vec<&str> = doc.keys().map(|k| k.as_str()).collect();
        assert!(keys.contains(&"_id"));
        assert!(keys.contains(&"name"));
        assert!(keys.contains(&"slug"));
        assert!(keys.contains(&"role_ids"));
        assert!(keys.contains(&"created_at"));
        assert!(keys.contains(&"updated_at"));
    }
}

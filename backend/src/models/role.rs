use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "roles";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Role {
    #[serde(rename = "_id")]
    pub id: String,
    pub name: String,
    pub slug: String,
    pub description: Option<String>,
    pub permissions: Vec<String>,
    pub is_default: bool,
    pub is_system: bool,
    pub client_id: Option<String>,
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
        assert_eq!(COLLECTION_NAME, "roles");
    }

    fn make_role() -> Role {
        Role {
            id: "550e8400-e29b-41d4-a716-446655440000".to_string(),
            name: "Admin".to_string(),
            slug: "admin".to_string(),
            description: Some("Administrator role".to_string()),
            permissions: vec!["*".to_string()],
            is_default: false,
            is_system: true,
            client_id: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn bson_roundtrip() {
        let role = make_role();
        let doc = bson::to_document(&role).expect("serialize role to bson");
        assert!(doc.get_str("_id").is_ok());
        assert!(doc.get("id").is_none(), "raw 'id' should not exist in bson");
        let restored: Role = bson::from_document(doc).expect("deserialize role from bson");
        assert_eq!(role.id, restored.id);
        assert_eq!(role.slug, restored.slug);
        assert_eq!(role.permissions, restored.permissions);
    }

    #[test]
    fn bson_all_fields_serialized() {
        let role = make_role();
        let doc = bson::to_document(&role).expect("serialize");
        let keys: Vec<&str> = doc.keys().map(|k| k.as_str()).collect();
        assert!(keys.contains(&"_id"));
        assert!(keys.contains(&"name"));
        assert!(keys.contains(&"slug"));
        assert!(keys.contains(&"permissions"));
        assert!(keys.contains(&"is_default"));
        assert!(keys.contains(&"is_system"));
        assert!(keys.contains(&"created_at"));
        assert!(keys.contains(&"updated_at"));
    }
}

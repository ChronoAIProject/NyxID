use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "node_service_bindings";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NodeServiceBinding {
    #[serde(rename = "_id")]
    pub id: String,
    pub node_id: String,
    pub user_id: String,
    pub service_id: String,
    pub is_active: bool,
    /// Lower value = higher priority (for future multi-node failover)
    #[serde(default)]
    pub priority: i32,
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
        assert_eq!(COLLECTION_NAME, "node_service_bindings");
    }

    fn make_binding() -> NodeServiceBinding {
        NodeServiceBinding {
            id: uuid::Uuid::new_v4().to_string(),
            node_id: uuid::Uuid::new_v4().to_string(),
            user_id: uuid::Uuid::new_v4().to_string(),
            service_id: uuid::Uuid::new_v4().to_string(),
            is_active: true,
            priority: 0,
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn bson_roundtrip() {
        let binding = make_binding();
        let doc = bson::to_document(&binding).expect("serialize");
        let restored: NodeServiceBinding = bson::from_document(doc).expect("deserialize");
        assert_eq!(binding.id, restored.id);
        assert_eq!(binding.node_id, restored.node_id);
        assert_eq!(binding.service_id, restored.service_id);
        assert_eq!(binding.priority, restored.priority);
    }
}

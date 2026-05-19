use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "agent_service_bindings";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentServiceBinding {
    #[serde(rename = "_id")]
    pub id: String,
    /// FK to ApiKey._id (the agent identity)
    pub api_key_id: String,
    /// FK to UserService._id
    pub user_service_id: String,
    /// FK to UserApiKey._id (the credential to inject)
    pub user_api_key_id: String,
    /// Denormalized for query efficiency
    pub user_id: String,
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
        assert_eq!(COLLECTION_NAME, "agent_service_bindings");
    }

    #[test]
    fn bson_roundtrip() {
        let binding = AgentServiceBinding {
            id: uuid::Uuid::new_v4().to_string(),
            api_key_id: uuid::Uuid::new_v4().to_string(),
            user_service_id: uuid::Uuid::new_v4().to_string(),
            user_api_key_id: uuid::Uuid::new_v4().to_string(),
            user_id: uuid::Uuid::new_v4().to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };
        let doc = bson::to_document(&binding).expect("serialize");
        let restored: AgentServiceBinding = bson::from_document(doc).expect("deserialize");
        assert_eq!(binding.id, restored.id);
        assert_eq!(binding.api_key_id, restored.api_key_id);
        assert_eq!(binding.user_service_id, restored.user_service_id);
        assert_eq!(binding.user_api_key_id, restored.user_api_key_id);
        assert_eq!(binding.user_id, restored.user_id);
    }
}

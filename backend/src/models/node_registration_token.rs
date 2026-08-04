use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "node_registration_tokens";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct NodeRegistrationToken {
    #[serde(rename = "_id")]
    pub id: String,
    pub user_id: String,
    /// SHA-256 hash of the one-time registration token
    pub token_hash: String,
    /// Pre-assigned name for the node that will be created
    pub name: String,
    pub used: bool,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collection_name() {
        assert_eq!(COLLECTION_NAME, "node_registration_tokens");
    }

    fn make_token() -> NodeRegistrationToken {
        NodeRegistrationToken {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: uuid::Uuid::new_v4().to_string(),
            token_hash: "deadbeef".repeat(8),
            name: "my-node".to_string(),
            used: false,
            expires_at: chrono::Utc::now(),
            created_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn bson_roundtrip() {
        let token = make_token();
        let doc = bson::to_document(&token).expect("serialize");
        let restored: NodeRegistrationToken = bson::from_document(doc).expect("deserialize");
        assert_eq!(token.id, restored.id);
        assert_eq!(token.name, restored.name);
        assert_eq!(token.used, restored.used);
    }
}

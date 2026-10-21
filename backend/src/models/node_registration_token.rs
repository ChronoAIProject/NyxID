use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "node_registration_tokens";

#[derive(Clone, Serialize, Deserialize)]
pub struct NodeRegistrationToken {
    #[serde(rename = "_id")]
    pub id: String,
    pub user_id: String,
    /// Person that authorized registration; user_id remains the resource owner.
    /// Absent on legacy rows. Legacy org registrations require reissuance.
    #[serde(default)]
    pub issuer_user_id: Option<String>,
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

impl std::fmt::Debug for NodeRegistrationToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NodeRegistrationToken")
            .field("id", &self.id)
            .field("user_id", &self.user_id)
            .field("issuer_user_id", &self.issuer_user_id)
            .field("token_hash", &"[REDACTED]")
            .field("used", &self.used)
            .field("expires_at", &self.expires_at)
            .finish()
    }
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
            issuer_user_id: Some(uuid::Uuid::new_v4().to_string()),
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
        assert_eq!(token.issuer_user_id, restored.issuer_user_id);
        assert!(!format!("{token:?}").contains(&token.token_hash));
    }
}

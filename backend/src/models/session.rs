use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "sessions";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    #[serde(rename = "_id")]
    pub id: String,
    pub user_id: String,
    /// SHA-256 hash of the session token stored in the cookie
    pub token_hash: String,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
    pub revoked: bool,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub last_active_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collection_name() {
        assert_eq!(COLLECTION_NAME, "sessions");
    }

    fn make_session() -> Session {
        Session {
            id: uuid::Uuid::new_v4().to_string(),
            user_id: uuid::Uuid::new_v4().to_string(),
            token_hash: "abcdef0123456789".to_string(),
            ip_address: Some("127.0.0.1".to_string()),
            user_agent: Some("test-agent".to_string()),
            expires_at: Utc::now(),
            revoked: false,
            created_at: Utc::now(),
            last_active_at: Utc::now(),
        }
    }

    #[test]
    fn bson_roundtrip() {
        let session = make_session();
        let doc = bson::to_document(&session).expect("serialize");
        assert!(doc.get_str("_id").is_ok());
        let restored: Session = bson::from_document(doc).expect("deserialize");
        assert_eq!(session.id, restored.id);
        assert_eq!(session.user_id, restored.user_id);
        assert_eq!(session.revoked, restored.revoked);
    }

    #[test]
    fn bson_roundtrip_with_nulls() {
        let mut session = make_session();
        session.ip_address = None;
        session.user_agent = None;
        let doc = bson::to_document(&session).expect("serialize");
        let restored: Session = bson::from_document(doc).expect("deserialize");
        assert!(restored.ip_address.is_none());
        assert!(restored.user_agent.is_none());
    }
}

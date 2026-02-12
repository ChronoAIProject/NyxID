use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "refresh_tokens";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RefreshToken {
    #[serde(rename = "_id")]
    pub id: String,
    /// JWT ID (jti) for this refresh token
    pub jti: String,
    pub client_id: String,
    pub user_id: String,
    pub session_id: Option<String>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
    pub revoked: bool,
    pub replaced_by: Option<String>,
    #[serde(default, with = "crate::models::bson_datetime::optional")]
    pub revoked_at: Option<DateTime<Utc>>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collection_name() {
        assert_eq!(COLLECTION_NAME, "refresh_tokens");
    }

    #[test]
    fn bson_roundtrip() {
        let token = RefreshToken {
            id: uuid::Uuid::new_v4().to_string(),
            jti: uuid::Uuid::new_v4().to_string(),
            client_id: "default".to_string(),
            user_id: uuid::Uuid::new_v4().to_string(),
            session_id: Some(uuid::Uuid::new_v4().to_string()),
            expires_at: Utc::now(),
            revoked: false,
            replaced_by: None,
            revoked_at: None,
            created_at: Utc::now(),
        };
        let doc = bson::to_document(&token).expect("serialize");
        let restored: RefreshToken = bson::from_document(doc).expect("deserialize");
        assert_eq!(token.id, restored.id);
        assert_eq!(token.jti, restored.jti);
        assert_eq!(token.revoked, restored.revoked);
    }
}

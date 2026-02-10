use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "authorization_codes";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthorizationCode {
    #[serde(rename = "_id")]
    pub id: String,
    pub code_hash: String,
    pub client_id: String,
    pub user_id: String,
    pub redirect_uri: String,
    pub scope: String,
    pub code_challenge: Option<String>,
    pub code_challenge_method: Option<String>,
    pub nonce: Option<String>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
    pub used: bool,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collection_name() {
        assert_eq!(COLLECTION_NAME, "authorization_codes");
    }

    #[test]
    fn bson_roundtrip() {
        let code = AuthorizationCode {
            id: uuid::Uuid::new_v4().to_string(),
            code_hash: "hash123".to_string(),
            client_id: "default-client".to_string(),
            user_id: uuid::Uuid::new_v4().to_string(),
            redirect_uri: "http://localhost:3000/callback".to_string(),
            scope: "openid profile".to_string(),
            code_challenge: Some("challenge".to_string()),
            code_challenge_method: Some("S256".to_string()),
            nonce: Some("nonce123".to_string()),
            expires_at: Utc::now(),
            used: false,
            created_at: Utc::now(),
        };
        let doc = bson::to_document(&code).expect("serialize");
        let restored: AuthorizationCode = bson::from_document(doc).expect("deserialize");
        assert_eq!(code.id, restored.id);
        assert_eq!(code.scope, restored.scope);
        assert_eq!(code.code_challenge, restored.code_challenge);
    }

    #[test]
    fn bson_roundtrip_no_pkce() {
        let code = AuthorizationCode {
            id: uuid::Uuid::new_v4().to_string(),
            code_hash: "hash123".to_string(),
            client_id: "default-client".to_string(),
            user_id: uuid::Uuid::new_v4().to_string(),
            redirect_uri: "http://localhost:3000/callback".to_string(),
            scope: "openid".to_string(),
            code_challenge: None,
            code_challenge_method: None,
            nonce: None,
            expires_at: Utc::now(),
            used: true,
            created_at: Utc::now(),
        };
        let doc = bson::to_document(&code).expect("serialize");
        let restored: AuthorizationCode = bson::from_document(doc).expect("deserialize");
        assert!(restored.code_challenge.is_none());
        assert!(restored.nonce.is_none());
        assert!(restored.used);
    }
}

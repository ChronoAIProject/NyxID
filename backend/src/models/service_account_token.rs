use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "service_account_tokens";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ServiceAccountToken {
    #[serde(rename = "_id")]
    pub id: String,

    /// The JTI (JWT ID) of the issued token, for revocation lookups.
    pub jti: String,

    /// The service account that owns this token.
    pub service_account_id: String,

    /// Space-separated scopes granted to this token.
    pub scope: String,

    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,

    pub revoked: bool,

    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collection_name() {
        assert_eq!(COLLECTION_NAME, "service_account_tokens");
    }

    fn make_token() -> ServiceAccountToken {
        ServiceAccountToken {
            id: uuid::Uuid::new_v4().to_string(),
            jti: uuid::Uuid::new_v4().to_string(),
            service_account_id: uuid::Uuid::new_v4().to_string(),
            scope: "proxy:* llm:proxy".to_string(),
            expires_at: Utc::now(),
            revoked: false,
            created_at: Utc::now(),
        }
    }

    #[test]
    fn bson_roundtrip() {
        let token = make_token();
        let doc = bson::to_document(&token).expect("serialize");
        assert!(doc.get_str("_id").is_ok());
        let restored: ServiceAccountToken = bson::from_document(doc).expect("deserialize");
        assert_eq!(token.id, restored.id);
        assert_eq!(token.jti, restored.jti);
        assert_eq!(token.revoked, restored.revoked);
    }

    #[test]
    fn bson_all_fields_serialized() {
        let token = make_token();
        let doc = bson::to_document(&token).expect("serialize");
        let keys: Vec<&str> = doc.keys().map(|k| k.as_str()).collect();
        assert!(keys.contains(&"_id"));
        assert!(keys.contains(&"jti"));
        assert!(keys.contains(&"service_account_id"));
        assert!(keys.contains(&"scope"));
        assert!(keys.contains(&"expires_at"));
        assert!(keys.contains(&"revoked"));
        assert!(keys.contains(&"created_at"));
    }
}

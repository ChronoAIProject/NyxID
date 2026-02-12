use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::bson_datetime;

pub const COLLECTION_NAME: &str = "service_accounts";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ServiceAccount {
    /// UUID v4 string, serves as the identity subject (`sub` in JWT claims).
    #[serde(rename = "_id")]
    pub id: String,

    /// Human-readable name (e.g. "CI/CD Pipeline", "Monitoring Agent").
    pub name: String,

    /// Optional description of what this service account does.
    pub description: Option<String>,

    /// Unique client_id for OAuth2 Client Credentials Grant.
    /// Format: "sa_" + 24 random hex chars.
    pub client_id: String,

    /// SHA-256 hash of the client_secret.
    /// The raw secret is shown once at creation, never stored.
    pub client_secret_hash: String,

    /// First 8 chars of client_secret for UI identification.
    pub secret_prefix: String,

    /// Directly assigned role IDs (no group membership for service accounts).
    #[serde(default)]
    pub role_ids: Vec<String>,

    /// Space-separated allowed scopes. Token requests can request a subset.
    pub allowed_scopes: String,

    /// Whether this service account can authenticate.
    pub is_active: bool,

    /// Optional per-account rate limit override (requests per second).
    pub rate_limit_override: Option<u64>,

    /// The admin user ID who created this service account.
    pub created_by: String,

    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,

    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,

    #[serde(default, with = "bson_datetime::optional")]
    pub last_authenticated_at: Option<DateTime<Utc>>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collection_name() {
        assert_eq!(COLLECTION_NAME, "service_accounts");
    }

    fn make_service_account() -> ServiceAccount {
        ServiceAccount {
            id: uuid::Uuid::new_v4().to_string(),
            name: "CI Pipeline".to_string(),
            description: Some("Runs CI/CD tasks".to_string()),
            client_id: "sa_abcdef0123456789abcdef01".to_string(),
            client_secret_hash: "deadbeef".repeat(8),
            secret_prefix: "sas_abcd".to_string(),
            role_ids: vec![],
            allowed_scopes: "proxy:* llm:proxy".to_string(),
            is_active: true,
            rate_limit_override: None,
            created_by: uuid::Uuid::new_v4().to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
            last_authenticated_at: None,
        }
    }

    #[test]
    fn bson_roundtrip() {
        let sa = make_service_account();
        let doc = bson::to_document(&sa).expect("serialize");
        assert!(doc.get_str("_id").is_ok());
        assert!(doc.get("id").is_none(), "raw 'id' should not exist in bson");
        let restored: ServiceAccount = bson::from_document(doc).expect("deserialize");
        assert_eq!(sa.id, restored.id);
        assert_eq!(sa.name, restored.name);
        assert_eq!(sa.client_id, restored.client_id);
        assert_eq!(sa.is_active, restored.is_active);
    }

    #[test]
    fn bson_roundtrip_with_optional_datetime() {
        let mut sa = make_service_account();
        sa.last_authenticated_at = Some(Utc::now());
        let doc = bson::to_document(&sa).expect("serialize");
        let restored: ServiceAccount = bson::from_document(doc).expect("deserialize");
        assert!(restored.last_authenticated_at.is_some());
    }

    #[test]
    fn bson_all_fields_serialized() {
        let sa = make_service_account();
        let doc = bson::to_document(&sa).expect("serialize");
        let keys: Vec<&str> = doc.keys().map(|k| k.as_str()).collect();
        assert!(keys.contains(&"_id"));
        assert!(keys.contains(&"name"));
        assert!(keys.contains(&"client_id"));
        assert!(keys.contains(&"client_secret_hash"));
        assert!(keys.contains(&"secret_prefix"));
        assert!(keys.contains(&"allowed_scopes"));
        assert!(keys.contains(&"is_active"));
        assert!(keys.contains(&"created_by"));
        assert!(keys.contains(&"created_at"));
        assert!(keys.contains(&"updated_at"));
    }
}

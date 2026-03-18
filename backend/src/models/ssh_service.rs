use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub const COLLECTION_NAME: &str = "ssh_services";

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SshService {
    #[serde(rename = "_id")]
    pub id: String,
    pub host: String,
    pub port: u16,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub certificate_auth_enabled: bool,
    #[serde(default = "default_certificate_ttl_minutes")]
    pub certificate_ttl_minutes: u32,
    #[serde(default)]
    pub allowed_principals: Vec<String>,
    #[serde(default)]
    pub ca_private_key_encrypted: Option<Vec<u8>>,
    #[serde(default)]
    pub ca_public_key: Option<String>,
    pub created_by: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

fn default_true() -> bool {
    true
}

fn default_certificate_ttl_minutes() -> u32 {
    30
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collection_name() {
        assert_eq!(COLLECTION_NAME, "ssh_services");
    }

    #[test]
    fn bson_roundtrip() {
        let model = SshService {
            id: uuid::Uuid::new_v4().to_string(),
            host: "ssh.internal.example".to_string(),
            port: 22,
            enabled: true,
            certificate_auth_enabled: true,
            certificate_ttl_minutes: 30,
            allowed_principals: vec!["ubuntu".to_string()],
            ca_private_key_encrypted: Some(vec![1, 2, 3]),
            ca_public_key: Some("ssh-ed25519 AAAATEST ssh-ca".to_string()),
            created_by: "admin".to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        };

        let doc = bson::to_document(&model).expect("serialize");
        let restored: SshService = bson::from_document(doc).expect("deserialize");
        assert_eq!(restored.host, "ssh.internal.example");
        assert_eq!(restored.port, 22);
        assert!(restored.enabled);
        assert!(restored.certificate_auth_enabled);
        assert_eq!(restored.certificate_ttl_minutes, 30);
        assert_eq!(restored.allowed_principals, vec!["ubuntu".to_string()]);
    }
}

use std::fmt;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::models::{bson_bytes, bson_datetime};

pub const COLLECTION_NAME: &str = "node_pending_credentials";

#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct CryptoBundle {
    pub version: String,
    pub node_pubkey: String,
    #[serde(default)]
    pub admin_pubkey: Option<String>,
    #[serde(default)]
    pub nonce: Option<String>,
    #[serde(default, with = "bson_bytes::optional")]
    pub ciphertext: Option<Vec<u8>>,
}

impl fmt::Debug for CryptoBundle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CryptoBundle")
            .field("version", &self.version)
            .field("node_pubkey", &"[REDACTED]")
            .field(
                "admin_pubkey",
                &self.admin_pubkey.as_ref().map(|_| "[REDACTED]"),
            )
            .field("nonce", &self.nonce.as_ref().map(|_| "[REDACTED]"))
            .field(
                "ciphertext",
                &self
                    .ciphertext
                    .as_ref()
                    .map(|ciphertext| format!("[REDACTED; {} bytes]", ciphertext.len())),
            )
            .finish()
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RemoteCryptoState {
    PubkeyAwaiting,
    PubkeyPosted,
    CiphertextReceived,
    CiphertextQueued,
    Consumed,
    PartialDecrypted,
    DecryptFailed,
    Expired,
    Declined,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum FanOutDecryptOutcome {
    Ok,
    Error,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct FanOutNodeState {
    pub node_id: String,
    pub generation: i64,
    pub crypto: CryptoBundle,
    pub remote_state: Option<RemoteCryptoState>,
    pub decrypt_outcome: Option<FanOutDecryptOutcome>,
    pub error_code: Option<u32>,
    pub error_kind: Option<String>,
    #[serde(default, with = "bson_datetime::optional")]
    pub pubkey_posted_at: Option<DateTime<Utc>>,
    #[serde(default, with = "bson_datetime::optional")]
    pub ciphertext_queued_at: Option<DateTime<Utc>>,
    #[serde(default, with = "bson_datetime::optional")]
    pub ciphertext_expires_at: Option<DateTime<Utc>>,
    #[serde(default, with = "bson_datetime::optional")]
    pub consumed_at: Option<DateTime<Utc>>,
    #[serde(default, with = "bson_datetime::optional")]
    pub declined_at: Option<DateTime<Utc>>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub updated_at: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum InjectionMethod {
    Header,
    QueryParam,
    PathPrefix,
}

impl InjectionMethod {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Header => "header",
            Self::QueryParam => "query-param",
            Self::PathPrefix => "path-prefix",
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub struct NodePendingCredential {
    #[serde(rename = "_id")]
    pub id: String,
    pub node_id: String,
    pub service_slug: String,
    pub injection_method: InjectionMethod,
    pub field_name: String,
    pub target_url: Option<String>,
    pub label: Option<String>,
    pub created_by_user_id: String,
    pub owner_user_id: String,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "bson::serde_helpers::chrono_datetime_as_bson_datetime")]
    pub expires_at: DateTime<Utc>,
    #[serde(default, with = "bson_datetime::optional")]
    pub consumed_at: Option<DateTime<Utc>>,
    #[serde(default, with = "bson_datetime::optional")]
    pub declined_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub crypto: Option<CryptoBundle>,
    #[serde(default)]
    pub remote_state: Option<RemoteCryptoState>,
    #[serde(default, with = "bson_datetime::optional")]
    pub ciphertext_queued_at: Option<DateTime<Utc>>,
    #[serde(default, with = "bson_datetime::optional")]
    pub ciphertext_expires_at: Option<DateTime<Utc>>,
    pub is_active: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fan_out_nodes: Vec<FanOutNodeState>,
    #[serde(default, skip_serializing_if = "is_zero_i64")]
    pub fan_out_revision: i64,
}

impl fmt::Debug for NodePendingCredential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NodePendingCredential")
            .field("id", &self.id)
            .field("node_id", &self.node_id)
            .field("service_slug", &self.service_slug)
            .field("injection_method", &self.injection_method)
            .field("field_name", &self.field_name)
            .field("target_url", &self.target_url)
            .field("label", &self.label)
            .field("created_by_user_id", &self.created_by_user_id)
            .field("owner_user_id", &self.owner_user_id)
            .field("created_at", &self.created_at)
            .field("expires_at", &self.expires_at)
            .field("consumed_at", &self.consumed_at)
            .field("declined_at", &self.declined_at)
            .field("crypto", &self.crypto.as_ref().map(|_| "[REDACTED]"))
            .field("remote_state", &self.remote_state)
            .field("ciphertext_queued_at", &self.ciphertext_queued_at)
            .field("ciphertext_expires_at", &self.ciphertext_expires_at)
            .field("is_active", &self.is_active)
            .field("fan_out_nodes", &self.fan_out_nodes)
            .field("fan_out_revision", &self.fan_out_revision)
            .finish()
    }
}

fn is_zero_i64(value: &i64) -> bool {
    *value == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collection_name() {
        assert_eq!(COLLECTION_NAME, "node_pending_credentials");
    }

    #[test]
    fn injection_method_as_str() {
        assert_eq!(InjectionMethod::Header.as_str(), "header");
        assert_eq!(InjectionMethod::QueryParam.as_str(), "query-param");
        assert_eq!(InjectionMethod::PathPrefix.as_str(), "path-prefix");
    }

    #[test]
    fn injection_method_serde_kebab_case() {
        let json = serde_json::to_string(&InjectionMethod::QueryParam).unwrap();
        assert_eq!(json, "\"query-param\"");
        let back: InjectionMethod = serde_json::from_str(&json).unwrap();
        assert_eq!(back, InjectionMethod::QueryParam);
    }

    #[test]
    fn crypto_bundle_serde_roundtrip() {
        let bundle = CryptoBundle {
            version: "v1".to_string(),
            node_pubkey: "node-pubkey".to_string(),
            admin_pubkey: Some("admin-pubkey".to_string()),
            nonce: Some("nonce".to_string()),
            ciphertext: Some(vec![1, 2, 3, 255]),
        };

        let doc = bson::to_document(&bundle).expect("serialize");
        let restored: CryptoBundle = bson::from_document(doc.clone()).expect("deserialize");

        assert_eq!(restored, bundle);
        assert!(matches!(doc.get("ciphertext"), Some(bson::Bson::Binary(_))));
    }

    #[test]
    fn crypto_bundle_debug_redacts_key_material_and_ciphertext() {
        let bundle = CryptoBundle {
            version: "v1".to_string(),
            node_pubkey: "node-pubkey".to_string(),
            admin_pubkey: Some("admin-pubkey".to_string()),
            nonce: Some("nonce-value-secret".to_string()),
            ciphertext: Some(vec![1, 2, 3, 255]),
        };

        let debug = format!("{bundle:?}");

        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("node-pubkey"));
        assert!(!debug.contains("admin-pubkey"));
        assert!(!debug.contains("nonce-value-secret"));
        assert!(!debug.contains("[1, 2, 3, 255]"));
    }

    #[test]
    fn remote_state_enum_serde_roundtrip() {
        let json = serde_json::to_string(&RemoteCryptoState::CiphertextQueued).unwrap();
        assert_eq!(json, "\"ciphertext_queued\"");
        let restored: RemoteCryptoState = serde_json::from_str(&json).unwrap();
        assert_eq!(restored, RemoteCryptoState::CiphertextQueued);

        let awaiting = serde_json::to_string(&RemoteCryptoState::PubkeyAwaiting).unwrap();
        assert_eq!(awaiting, "\"pubkey_awaiting\"");
        assert_eq!(
            serde_json::from_str::<RemoteCryptoState>(&awaiting).unwrap(),
            RemoteCryptoState::PubkeyAwaiting
        );
    }

    #[test]
    fn partial_decrypted_and_declined_remote_states_serde() {
        let partial = serde_json::to_string(&RemoteCryptoState::PartialDecrypted).unwrap();
        let declined = serde_json::to_string(&RemoteCryptoState::Declined).unwrap();

        assert_eq!(partial, "\"partial_decrypted\"");
        assert_eq!(declined, "\"declined\"");
        assert_eq!(
            serde_json::from_str::<RemoteCryptoState>(&partial).unwrap(),
            RemoteCryptoState::PartialDecrypted
        );
        assert_eq!(
            serde_json::from_str::<RemoteCryptoState>(&declined).unwrap(),
            RemoteCryptoState::Declined
        );
    }

    #[test]
    fn fan_out_decrypt_outcome_serde() {
        let json = serde_json::to_string(&FanOutDecryptOutcome::Error).unwrap();
        assert_eq!(json, "\"error\"");
        assert_eq!(
            serde_json::from_str::<FanOutDecryptOutcome>(&json).unwrap(),
            FanOutDecryptOutcome::Error
        );
    }

    #[test]
    fn legacy_pending_without_crypto_roundtrip() {
        let now = Utc::now();
        let legacy_doc = bson::doc! {
            "_id": "legacy",
            "node_id": "node-1",
            "service_slug": "openai",
            "injection_method": "header",
            "field_name": "Authorization",
            "target_url": bson::Bson::Null,
            "label": bson::Bson::Null,
            "created_by_user_id": "user-1",
            "owner_user_id": "user-1",
            "created_at": bson::DateTime::from_chrono(now),
            "expires_at": bson::DateTime::from_chrono(now + chrono::Duration::hours(1)),
            "consumed_at": bson::Bson::Null,
            "declined_at": bson::Bson::Null,
            "is_active": true,
        };

        let restored: NodePendingCredential =
            bson::from_document(legacy_doc).expect("deserialize legacy");

        assert!(restored.crypto.is_none());
        assert!(restored.remote_state.is_none());
        assert!(restored.ciphertext_queued_at.is_none());
        assert!(restored.ciphertext_expires_at.is_none());
        assert!(restored.fan_out_nodes.is_empty());
        assert_eq!(restored.fan_out_revision, 0);
    }

    #[test]
    fn single_node_bson_omits_fan_out_fields() {
        let now = Utc::now();
        let cred = NodePendingCredential {
            id: "single".to_string(),
            node_id: "node-1".to_string(),
            service_slug: "openai".to_string(),
            injection_method: InjectionMethod::Header,
            field_name: "Authorization".to_string(),
            target_url: None,
            label: None,
            created_by_user_id: "user-1".to_string(),
            owner_user_id: "user-1".to_string(),
            created_at: now,
            expires_at: now + chrono::Duration::hours(1),
            consumed_at: None,
            declined_at: None,
            crypto: None,
            remote_state: None,
            ciphertext_queued_at: None,
            ciphertext_expires_at: None,
            is_active: true,
            fan_out_nodes: Vec::new(),
            fan_out_revision: 0,
        };

        let doc = bson::to_document(&cred).expect("serialize");

        assert!(!doc.contains_key("fan_out_nodes"));
        assert!(!doc.contains_key("fan_out_revision"));
    }

    #[test]
    fn fan_out_bson_includes_revision_and_nodes_with_chrono_helpers() {
        let now = Utc::now();
        let cred = NodePendingCredential {
            id: "fanout".to_string(),
            node_id: "node-1".to_string(),
            service_slug: "openai".to_string(),
            injection_method: InjectionMethod::Header,
            field_name: "Authorization".to_string(),
            target_url: None,
            label: None,
            created_by_user_id: "user-1".to_string(),
            owner_user_id: "user-1".to_string(),
            created_at: now,
            expires_at: now + chrono::Duration::hours(1),
            consumed_at: None,
            declined_at: None,
            crypto: None,
            remote_state: Some(RemoteCryptoState::PubkeyPosted),
            ciphertext_queued_at: None,
            ciphertext_expires_at: None,
            is_active: true,
            fan_out_nodes: vec![FanOutNodeState {
                node_id: "node-1".to_string(),
                generation: 0,
                crypto: CryptoBundle {
                    version: "v1".to_string(),
                    node_pubkey: "node-pubkey".to_string(),
                    admin_pubkey: None,
                    nonce: None,
                    ciphertext: None,
                },
                remote_state: Some(RemoteCryptoState::PubkeyPosted),
                decrypt_outcome: None,
                error_code: None,
                error_kind: None,
                pubkey_posted_at: Some(now),
                ciphertext_queued_at: None,
                ciphertext_expires_at: None,
                consumed_at: None,
                declined_at: None,
                updated_at: now,
            }],
            fan_out_revision: 1,
        };

        let doc = bson::to_document(&cred).expect("serialize");
        assert!(doc.contains_key("fan_out_nodes"));
        assert_eq!(doc.get_i64("fan_out_revision").unwrap(), 1);
        let restored: NodePendingCredential = bson::from_document(doc).expect("deserialize");
        assert_eq!(
            restored.fan_out_nodes[0]
                .pubkey_posted_at
                .expect("pubkey timestamp")
                .timestamp_millis(),
            now.timestamp_millis()
        );
    }

    #[test]
    fn queue_lifecycle_fields_bson_roundtrip() {
        let now = Utc::now();
        let cred = NodePendingCredential {
            id: "queued".to_string(),
            node_id: "node-1".to_string(),
            service_slug: "openai".to_string(),
            injection_method: InjectionMethod::Header,
            field_name: "Authorization".to_string(),
            target_url: None,
            label: Some("OpenAI key".to_string()),
            created_by_user_id: "user-1".to_string(),
            owner_user_id: "user-1".to_string(),
            created_at: now,
            expires_at: now + chrono::Duration::hours(1),
            consumed_at: None,
            declined_at: None,
            crypto: Some(CryptoBundle {
                version: "v1".to_string(),
                node_pubkey: "node-pubkey".to_string(),
                admin_pubkey: Some("admin-pubkey".to_string()),
                nonce: Some("nonce-secret".to_string()),
                ciphertext: Some(vec![4, 5, 6]),
            }),
            remote_state: Some(RemoteCryptoState::CiphertextQueued),
            ciphertext_queued_at: Some(now),
            ciphertext_expires_at: Some(now + chrono::Duration::minutes(15)),
            is_active: true,
            fan_out_nodes: Vec::new(),
            fan_out_revision: 0,
        };

        let doc = bson::to_document(&cred).expect("serialize");
        let restored: NodePendingCredential = bson::from_document(doc).expect("deserialize");

        assert_eq!(
            restored.remote_state,
            Some(RemoteCryptoState::CiphertextQueued)
        );
        assert_eq!(
            restored.crypto.and_then(|crypto| crypto.ciphertext),
            Some(vec![4, 5, 6])
        );
        assert_eq!(
            restored
                .ciphertext_queued_at
                .expect("queued timestamp")
                .timestamp_millis(),
            now.timestamp_millis()
        );
        assert!(restored.ciphertext_expires_at.is_some());
    }

    #[test]
    fn node_pending_credential_debug_redacts_crypto_bundle() {
        let now = Utc::now();
        let cred = NodePendingCredential {
            id: "queued".to_string(),
            node_id: "node-1".to_string(),
            service_slug: "openai".to_string(),
            injection_method: InjectionMethod::Header,
            field_name: "Authorization".to_string(),
            target_url: None,
            label: None,
            created_by_user_id: "user-1".to_string(),
            owner_user_id: "user-1".to_string(),
            created_at: now,
            expires_at: now + chrono::Duration::hours(1),
            consumed_at: None,
            declined_at: None,
            crypto: Some(CryptoBundle {
                version: "v1".to_string(),
                node_pubkey: "node-pubkey".to_string(),
                admin_pubkey: Some("admin-pubkey".to_string()),
                nonce: Some("nonce-value-secret".to_string()),
                ciphertext: Some(vec![4, 5, 6]),
            }),
            remote_state: Some(RemoteCryptoState::CiphertextQueued),
            ciphertext_queued_at: Some(now),
            ciphertext_expires_at: Some(now + chrono::Duration::minutes(15)),
            is_active: true,
            fan_out_nodes: Vec::new(),
            fan_out_revision: 0,
        };

        let debug = format!("{cred:?}");

        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("node-pubkey"));
        assert!(!debug.contains("admin-pubkey"));
        assert!(!debug.contains("nonce-value-secret"));
        assert!(!debug.contains("[4, 5, 6]"));
    }

    #[test]
    fn node_pending_credential_debug_redacts_nested_fan_out_crypto() {
        let now = Utc::now();
        let cred = NodePendingCredential {
            id: "fanout".to_string(),
            node_id: "node-1".to_string(),
            service_slug: "openai".to_string(),
            injection_method: InjectionMethod::Header,
            field_name: "Authorization".to_string(),
            target_url: None,
            label: None,
            created_by_user_id: "user-1".to_string(),
            owner_user_id: "user-1".to_string(),
            created_at: now,
            expires_at: now + chrono::Duration::hours(1),
            consumed_at: None,
            declined_at: None,
            crypto: None,
            remote_state: None,
            ciphertext_queued_at: None,
            ciphertext_expires_at: None,
            is_active: true,
            fan_out_nodes: vec![FanOutNodeState {
                node_id: "node-1".to_string(),
                generation: 0,
                crypto: CryptoBundle {
                    version: "v1".to_string(),
                    node_pubkey: "node-pubkey-secret".to_string(),
                    admin_pubkey: Some("admin-pubkey-secret".to_string()),
                    nonce: Some("nonce-secret".to_string()),
                    ciphertext: Some(vec![7, 8, 9]),
                },
                remote_state: Some(RemoteCryptoState::CiphertextQueued),
                decrypt_outcome: None,
                error_code: None,
                error_kind: None,
                pubkey_posted_at: None,
                ciphertext_queued_at: Some(now),
                ciphertext_expires_at: Some(now + chrono::Duration::minutes(15)),
                consumed_at: None,
                declined_at: None,
                updated_at: now,
            }],
            fan_out_revision: 1,
        };

        let debug = format!("{cred:?}");

        assert!(!debug.contains("node-pubkey-secret"));
        assert!(!debug.contains("admin-pubkey-secret"));
        assert!(!debug.contains("nonce-secret"));
        assert!(!debug.contains("[7, 8, 9]"));
    }

    #[test]
    fn bson_roundtrip() {
        let cred = NodePendingCredential {
            id: uuid::Uuid::new_v4().to_string(),
            node_id: "node-1".to_string(),
            service_slug: "openai".to_string(),
            injection_method: InjectionMethod::Header,
            field_name: "Authorization".to_string(),
            target_url: Some("https://api.openai.com".to_string()),
            label: Some("OpenAI key".to_string()),
            created_by_user_id: uuid::Uuid::new_v4().to_string(),
            owner_user_id: uuid::Uuid::new_v4().to_string(),
            created_at: Utc::now(),
            expires_at: Utc::now() + chrono::Duration::hours(1),
            consumed_at: None,
            declined_at: None,
            crypto: None,
            remote_state: None,
            ciphertext_queued_at: None,
            ciphertext_expires_at: None,
            is_active: true,
            fan_out_nodes: Vec::new(),
            fan_out_revision: 0,
        };
        let doc = bson::to_document(&cred).expect("serialize");
        let restored: NodePendingCredential = bson::from_document(doc).expect("deserialize");
        assert_eq!(cred.id, restored.id);
        assert_eq!(cred.service_slug, restored.service_slug);
        assert_eq!(restored.injection_method, InjectionMethod::Header);
        assert!(restored.consumed_at.is_none());
    }

    #[test]
    fn bson_roundtrip_with_consumed_and_declined() {
        let cred = NodePendingCredential {
            id: "id".to_string(),
            node_id: "n".to_string(),
            service_slug: "s".to_string(),
            injection_method: InjectionMethod::PathPrefix,
            field_name: "bot".to_string(),
            target_url: None,
            label: None,
            created_by_user_id: "u".to_string(),
            owner_user_id: "u".to_string(),
            created_at: Utc::now(),
            expires_at: Utc::now(),
            consumed_at: Some(Utc::now()),
            declined_at: Some(Utc::now()),
            crypto: None,
            remote_state: None,
            ciphertext_queued_at: None,
            ciphertext_expires_at: None,
            is_active: false,
            fan_out_nodes: Vec::new(),
            fan_out_revision: 0,
        };
        let doc = bson::to_document(&cred).expect("serialize");
        let restored: NodePendingCredential = bson::from_document(doc).expect("deserialize");
        assert!(restored.consumed_at.is_some());
        assert!(restored.declined_at.is_some());
        assert!(!restored.is_active);
    }
}

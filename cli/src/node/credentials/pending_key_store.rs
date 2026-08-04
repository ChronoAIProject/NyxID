use std::fmt;

use chrono::{DateTime, Utc};
use zeroize::Zeroizing;

use super::crypto::PendingCredentialCryptoMetadata;
use crate::node::config::{NodeConfig, PendingCryptoKeyConfig};
use crate::node::error::{Error, Result};
use crate::node::secret_backend::SecretBackend;

#[derive(Clone, PartialEq, Eq)]
pub struct PendingCredentialPublicKey {
    pub pending_id: String,
    pub version: String,
    pub node_pubkey: String,
}

impl fmt::Debug for PendingCredentialPublicKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PendingCredentialPublicKey")
            .field("pending_id", &self.pending_id)
            .field("version", &self.version)
            .field("node_pubkey", &"[REDACTED]")
            .finish()
    }
}

pub struct PendingCredentialKeyStore<'a> {
    config: &'a mut NodeConfig,
    backend: &'a SecretBackend,
}

impl<'a> PendingCredentialKeyStore<'a> {
    pub fn new(config: &'a mut NodeConfig, backend: &'a SecretBackend) -> Self {
        Self { config, backend }
    }

    pub fn create_or_load(
        &mut self,
        metadata: &PendingCredentialCryptoMetadata,
    ) -> Result<PendingCredentialPublicKey> {
        if let Some(existing) = self
            .config
            .pending_crypto_keys
            .get(&metadata.pending_id)
            .cloned()
        {
            self.load_private_key(&metadata.pending_id)?;
            return Ok(PendingCredentialPublicKey {
                pending_id: metadata.pending_id.clone(),
                version: existing.version,
                node_pubkey: existing.public_key,
            });
        }

        let keypair = nyxid_crypto::generate_node_keypair();
        let private_key_b64u = nyxid_crypto::encode_private_key_b64u(keypair.private_key());
        let private_key_encrypted = self
            .backend
            .store_pending_crypto_key(&metadata.pending_id, &private_key_b64u)?;
        let node_pubkey = keypair.public_key_b64u();

        self.config.pending_crypto_keys.insert(
            metadata.pending_id.clone(),
            PendingCryptoKeyConfig {
                version: metadata.version.clone(),
                service_slug: metadata.service_slug.clone(),
                injection_method: metadata.injection_method.clone(),
                field_name: metadata.field_name.clone(),
                target_url: metadata.target_url.clone(),
                expires_at: metadata.expires_at.clone(),
                public_key: node_pubkey.clone(),
                private_key_encrypted,
            },
        );

        Ok(PendingCredentialPublicKey {
            pending_id: metadata.pending_id.clone(),
            version: metadata.version.clone(),
            node_pubkey,
        })
    }

    pub fn load_private_key(&self, pending_id: &str) -> Result<Zeroizing<[u8; 32]>> {
        let entry = self
            .config
            .pending_crypto_keys
            .get(pending_id)
            .ok_or_else(|| Error::Config(format!("No pending crypto key for '{pending_id}'")))?;
        let private_key_b64u = self
            .backend
            .load_pending_crypto_key(pending_id, entry.private_key_encrypted.as_deref())?;
        nyxid_crypto::decode_private_key_b64u(&private_key_b64u)
            .map_err(|error| Error::Encryption(error.to_string()))
    }

    pub fn delete(&mut self, pending_id: &str) -> Result<bool> {
        let existed = self.config.pending_crypto_keys.remove(pending_id).is_some();
        self.backend.delete_pending_crypto_key(pending_id)?;
        Ok(existed)
    }

    pub fn sweep_expired(&mut self, now: DateTime<Utc>) -> Result<Vec<String>> {
        let expired = self
            .config
            .pending_crypto_keys
            .iter()
            .filter_map(|(pending_id, entry)| {
                let expires_at = DateTime::parse_from_rfc3339(&entry.expires_at)
                    .map(|dt| dt.with_timezone(&Utc));
                match expires_at {
                    Ok(expires_at) if expires_at <= now => Some(pending_id.clone()),
                    Err(_) => Some(pending_id.clone()),
                    _ => None,
                }
            })
            .collect::<Vec<_>>();

        for pending_id in &expired {
            self.delete(pending_id)?;
        }

        Ok(expired)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::encryption::LocalEncryption;

    fn metadata(id: &str, expires_at: &str) -> PendingCredentialCryptoMetadata {
        PendingCredentialCryptoMetadata {
            pending_id: id.to_string(),
            node_id: "node-1".to_string(),
            service_slug: "openai".to_string(),
            injection_method: "header".to_string(),
            field_name: "Authorization".to_string(),
            target_url: Some("https://api.openai.com/v1".to_string()),
            expires_at: expires_at.to_string(),
            version: nyxid_crypto::VERSION_V1.to_string(),
        }
    }

    fn file_config() -> NodeConfig {
        NodeConfig::new(
            "ws://localhost:3001/api/v1/nodes/ws".to_string(),
            "node-1".to_string(),
            "file".to_string(),
        )
    }

    #[test]
    fn public_key_debug_redacts_node_pubkey() {
        let public = PendingCredentialPublicKey {
            pending_id: "pending-1".to_string(),
            version: nyxid_crypto::VERSION_V1.to_string(),
            node_pubkey: "node-public-key-material".to_string(),
        };

        let debug = format!("{public:?}");

        assert!(debug.contains("PendingCredentialPublicKey"));
        assert!(debug.contains("pending-1"));
        assert!(debug.contains("[REDACTED]"));
        assert!(!debug.contains("node-public-key-material"));
    }

    #[test]
    fn persist_and_reload_privkey_across_restart() {
        let dir = tempfile::tempdir().unwrap();
        let backend = SecretBackend::File(LocalEncryption::load_or_generate(dir.path()).unwrap());
        let config_path = dir.path().join("config.toml");
        let mut config = file_config();
        let meta = metadata("pending-1", "2099-01-01T00:00:00Z");
        let first_public = {
            let mut store = PendingCredentialKeyStore::new(&mut config, &backend);
            let public = store.create_or_load(&meta).unwrap();
            assert!(store.load_private_key("pending-1").is_ok());
            public
        };
        config.save(&config_path).unwrap();

        let mut reloaded = NodeConfig::load(&config_path).unwrap();
        let mut reloaded_store = PendingCredentialKeyStore::new(&mut reloaded, &backend);
        let second_public = reloaded_store.create_or_load(&meta).unwrap();

        assert_eq!(first_public, second_public);
        assert!(reloaded_store.load_private_key("pending-1").is_ok());
    }

    #[test]
    fn file_backend_deletes_pending_key() {
        let dir = tempfile::tempdir().unwrap();
        let backend = SecretBackend::File(LocalEncryption::load_or_generate(dir.path()).unwrap());
        let mut config = file_config();
        let meta = metadata("pending-1", "2099-01-01T00:00:00Z");
        let mut store = PendingCredentialKeyStore::new(&mut config, &backend);
        store.create_or_load(&meta).unwrap();
        assert!(store.delete("pending-1").unwrap());
        assert!(store.load_private_key("pending-1").is_err());
    }

    #[test]
    fn keychain_backend_deletes_pending_key() {
        let backend = SecretBackend::new_mock_keychain("node-1");
        let mut config = NodeConfig::new(
            "ws://localhost:3001/api/v1/nodes/ws".to_string(),
            "node-1".to_string(),
            "keychain".to_string(),
        );
        let meta = metadata("pending-1", "2099-01-01T00:00:00Z");
        let mut store = PendingCredentialKeyStore::new(&mut config, &backend);
        store.create_or_load(&meta).unwrap();
        assert!(store.load_private_key("pending-1").is_ok());
        assert!(store.delete("pending-1").unwrap());
        assert!(
            backend
                .load_pending_crypto_key("pending-1", None)
                .unwrap_err()
                .to_string()
                .contains("pending-1")
        );
    }

    #[test]
    fn sweep_expired_deletes_only_expired() {
        let dir = tempfile::tempdir().unwrap();
        let backend = SecretBackend::File(LocalEncryption::load_or_generate(dir.path()).unwrap());
        let mut config = file_config();
        let now = DateTime::parse_from_rfc3339("2026-06-04T00:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        {
            let mut store = PendingCredentialKeyStore::new(&mut config, &backend);
            store
                .create_or_load(&metadata("expired", "2026-06-03T23:59:59Z"))
                .unwrap();
            store
                .create_or_load(&metadata("active", "2026-06-04T00:00:01Z"))
                .unwrap();
            let removed = store.sweep_expired(now).unwrap();
            assert_eq!(removed, vec!["expired".to_string()]);
        }
        assert!(!config.pending_crypto_keys.contains_key("expired"));
        assert!(config.pending_crypto_keys.contains_key("active"));
    }
}

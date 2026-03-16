use std::cell::RefCell;
#[cfg(test)]
use std::collections::HashMap;
#[cfg(test)]
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::config::NodeConfig;
use crate::error::{Error, Result};

const SERVICE_NAME: &str = "nyxid-node";

#[derive(Clone)]
enum KeychainClient {
    System,
    #[cfg(test)]
    Memory(Arc<Mutex<HashMap<String, String>>>),
    #[cfg(test)]
    Faulty(String),
}

/// OS keychain backend for secret storage.
/// Uses macOS Keychain, Windows Credential Manager, or Linux Secret Service.
#[derive(Clone)]
pub struct KeychainBackend {
    node_id: String,
    client: KeychainClient,
}

impl KeychainBackend {
    pub fn new(node_id: &str) -> Self {
        Self {
            node_id: node_id.to_string(),
            client: KeychainClient::System,
        }
    }

    #[cfg(test)]
    pub fn new_mock(node_id: &str) -> Self {
        Self {
            node_id: node_id.to_string(),
            client: KeychainClient::Memory(Arc::new(Mutex::new(HashMap::new()))),
        }
    }

    #[cfg(test)]
    pub fn new_failing_mock(node_id: &str, message: &str) -> Self {
        Self {
            node_id: node_id.to_string(),
            client: KeychainClient::Faulty(message.to_string()),
        }
    }

    /// Verify that the backing keychain is writable before depending on it.
    pub fn preflight(&self) -> Result<()> {
        let suffix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let key = format!("preflight/{suffix}");
        let expected = format!("nyxid-node-preflight-{suffix}");

        self.set(&key, &expected)?;
        let actual = self.get(&key)?;
        if actual != expected {
            let _ = self.delete(&key);
            return Err(Error::Keychain(
                "Keychain preflight returned an unexpected value".to_string(),
            ));
        }
        self.delete(&key)?;
        Ok(())
    }

    /// Store a secret in the OS keychain.
    pub fn set(&self, key: &str, value: &str) -> Result<()> {
        match &self.client {
            KeychainClient::System => {
                let entry = self.entry(key)?;
                entry
                    .set_password(value)
                    .map_err(|e| Error::Keychain(format!("Failed to store '{key}': {e}")))
            }
            #[cfg(test)]
            KeychainClient::Memory(store) => {
                store
                    .lock()
                    .expect("mock keychain lock poisoned")
                    .insert(self.user(key), value.to_string());
                Ok(())
            }
            #[cfg(test)]
            KeychainClient::Faulty(message) => Err(Error::Keychain(message.clone())),
        }
    }

    /// Retrieve a secret from the OS keychain.
    pub fn get(&self, key: &str) -> Result<String> {
        match &self.client {
            KeychainClient::System => {
                let entry = self.entry(key)?;
                entry
                    .get_password()
                    .map_err(|e| Error::Keychain(format!("Failed to retrieve '{key}': {e}")))
            }
            #[cfg(test)]
            KeychainClient::Memory(store) => store
                .lock()
                .expect("mock keychain lock poisoned")
                .get(&self.user(key))
                .cloned()
                .ok_or_else(|| Error::Keychain(format!("Failed to retrieve '{key}': no entry"))),
            #[cfg(test)]
            KeychainClient::Faulty(message) => Err(Error::Keychain(message.clone())),
        }
    }

    /// Retrieve a secret, returning None if not found.
    pub fn get_optional(&self, key: &str) -> Result<Option<String>> {
        match &self.client {
            KeychainClient::System => {
                let entry = self.entry(key)?;
                match entry.get_password() {
                    Ok(v) => Ok(Some(v)),
                    Err(keyring::Error::NoEntry) => Ok(None),
                    Err(e) => Err(Error::Keychain(format!("Failed to retrieve '{key}': {e}"))),
                }
            }
            #[cfg(test)]
            KeychainClient::Memory(store) => Ok(store
                .lock()
                .expect("mock keychain lock poisoned")
                .get(&self.user(key))
                .cloned()),
            #[cfg(test)]
            KeychainClient::Faulty(message) => Err(Error::Keychain(message.clone())),
        }
    }

    /// Delete a secret from the OS keychain (idempotent).
    pub fn delete(&self, key: &str) -> Result<()> {
        match &self.client {
            KeychainClient::System => {
                let entry = self.entry(key)?;
                match entry.delete_credential() {
                    Ok(()) => Ok(()),
                    Err(keyring::Error::NoEntry) => Ok(()),
                    Err(e) => Err(Error::Keychain(format!("Failed to delete '{key}': {e}"))),
                }
            }
            #[cfg(test)]
            KeychainClient::Memory(store) => {
                store
                    .lock()
                    .expect("mock keychain lock poisoned")
                    .remove(&self.user(key));
                Ok(())
            }
            #[cfg(test)]
            KeychainClient::Faulty(message) => Err(Error::Keychain(message.clone())),
        }
    }

    fn user(&self, key: &str) -> String {
        format!("{}/{key}", self.node_id)
    }

    fn entry(&self, key: &str) -> Result<keyring::Entry> {
        let user = self.user(key);
        keyring::Entry::new(SERVICE_NAME, &user)
            .map_err(|e| Error::Keychain(format!("Failed to create keyring entry: {e}")))
    }
}

// Well-known key names
pub const KEY_AUTH_TOKEN: &str = "auth_token";
pub const KEY_SIGNING_SECRET: &str = "signing_secret";
const VAULT_KEY: &str = "vault";

/// Keyring key for a service credential value.
pub fn credential_key(service_slug: &str) -> String {
    format!("cred/{service_slug}")
}

// ---------------------------------------------------------------------------
// Vault: single keychain entry holding all secrets
// ---------------------------------------------------------------------------

/// All secrets stored in one keychain entry to avoid per-item password prompts.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VaultData {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signing_secret: Option<String>,
    #[serde(default)]
    pub credentials: std::collections::BTreeMap<String, String>,
}

/// A single-entry keychain vault that caches all secrets in memory.
/// Reads trigger one keychain prompt; subsequent access is from memory.
pub struct KeychainVault {
    backend: KeychainBackend,
    vault: RefCell<VaultData>,
}

impl KeychainVault {
    /// Create an empty vault (used during registration before any secrets exist).
    pub fn new(node_id: &str) -> Self {
        Self {
            backend: KeychainBackend::new(node_id),
            vault: RefCell::new(VaultData::default()),
        }
    }

    #[cfg(test)]
    pub fn new_mock(node_id: &str) -> Self {
        Self {
            backend: KeychainBackend::new_mock(node_id),
            vault: RefCell::new(VaultData::default()),
        }
    }

    /// Load the vault from keychain. If no vault entry exists, migrate from
    /// individual keychain entries (one-time, then they are consolidated).
    pub fn load(node_id: &str, config: &NodeConfig) -> Result<Self> {
        Self::load_with_backend(KeychainBackend::new(node_id), config)
    }

    fn load_with_backend(backend: KeychainBackend, config: &NodeConfig) -> Result<Self> {
        match backend.get_optional(VAULT_KEY)? {
            Some(json) => {
                let vault: VaultData = serde_json::from_str(&json)
                    .map_err(|e| Error::Keychain(format!("Corrupt vault data: {e}")))?;
                Ok(Self {
                    backend,
                    vault: RefCell::new(vault),
                })
            }
            None => {
                // No vault yet -- migrate from individual entries
                tracing::info!("Migrating keychain secrets into vault (one-time)");
                let vault = Self::migrate_from_individual(&backend, config)?;
                let kv = Self {
                    backend,
                    vault: RefCell::new(vault),
                };
                kv.flush()?;
                kv.cleanup_individual(config);
                tracing::info!("Keychain vault migration complete");
                Ok(kv)
            }
        }
    }

    fn migrate_from_individual(
        backend: &KeychainBackend,
        config: &NodeConfig,
    ) -> Result<VaultData> {
        let auth_token = Some(Self::required_secret(
            backend,
            KEY_AUTH_TOKEN,
            "auth token",
        )?);
        let signing_secret = if config.signing.shared_secret_encrypted.is_some() {
            Some(Self::required_secret(
                backend,
                KEY_SIGNING_SECRET,
                "signing secret",
            )?)
        } else {
            None
        };

        let mut credentials = std::collections::BTreeMap::new();
        for slug in config.credentials.keys() {
            let key = credential_key(slug);
            let value = backend.get_optional(&key)?.ok_or_else(|| {
                Error::Keychain(format!(
                    "Missing credential '{slug}' during keychain vault migration"
                ))
            })?;
            credentials.insert(slug.clone(), value);
        }

        Ok(VaultData {
            auth_token,
            signing_secret,
            credentials,
        })
    }

    fn required_secret(backend: &KeychainBackend, key: &str, label: &str) -> Result<String> {
        backend.get_optional(key)?.ok_or_else(|| {
            Error::Keychain(format!("Missing {label} during keychain vault migration"))
        })
    }

    fn cleanup_individual(&self, config: &NodeConfig) {
        let _ = self.backend.delete(KEY_AUTH_TOKEN);
        let _ = self.backend.delete(KEY_SIGNING_SECRET);
        for slug in config.credentials.keys() {
            let _ = self.backend.delete(&credential_key(slug));
        }
    }

    fn flush(&self) -> Result<()> {
        let vault = self.vault.borrow();
        let json = serde_json::to_string(&*vault)
            .map_err(|e| Error::Keychain(format!("Failed to serialize vault: {e}")))?;
        self.backend.set(VAULT_KEY, &json)
    }

    /// Verify keychain is writable (delegates to backend).
    #[allow(dead_code)]
    pub fn preflight(&self) -> Result<()> {
        self.backend.preflight()
    }

    // -- Auth token --

    pub fn set_auth_token(&self, token: &str) -> Result<()> {
        self.vault.borrow_mut().auth_token = Some(token.to_string());
        self.flush()
    }

    pub fn get_auth_token(&self) -> Result<String> {
        self.vault
            .borrow()
            .auth_token
            .clone()
            .ok_or_else(|| Error::Keychain("No auth token in vault".to_string()))
    }

    // -- Signing secret --

    pub fn set_signing_secret(&self, secret: &str) -> Result<()> {
        self.vault.borrow_mut().signing_secret = Some(secret.to_string());
        self.flush()
    }

    pub fn get_signing_secret(&self) -> Result<Option<String>> {
        Ok(self.vault.borrow().signing_secret.clone())
    }

    // -- Service credentials --

    pub fn set_credential(&self, slug: &str, value: &str) -> Result<()> {
        self.vault
            .borrow_mut()
            .credentials
            .insert(slug.to_string(), value.to_string());
        self.flush()
    }

    pub fn get_credential(&self, slug: &str) -> Result<String> {
        self.vault
            .borrow()
            .credentials
            .get(slug)
            .cloned()
            .ok_or_else(|| Error::Keychain(format!("No credential for '{slug}' in vault")))
    }

    pub fn delete_credential(&self, slug: &str) -> Result<()> {
        self.vault.borrow_mut().credentials.remove(slug);
        self.flush()
    }

    pub fn delete_auth_token(&self) -> Result<()> {
        self.vault.borrow_mut().auth_token = None;
        self.flush()
    }

    pub fn delete_signing_secret(&self) -> Result<()> {
        self.vault.borrow_mut().signing_secret = None;
        self.flush()
    }

    /// Delete the entire vault entry from the keychain.
    #[allow(dead_code)]
    pub fn delete_all(&self) -> Result<()> {
        self.backend.delete(VAULT_KEY)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::CredentialConfig;

    fn keychain_config() -> NodeConfig {
        NodeConfig::new(
            "wss://example.com/api/v1/nodes/ws".to_string(),
            "node-1".to_string(),
            "keychain".to_string(),
        )
    }

    fn header_credential() -> CredentialConfig {
        CredentialConfig {
            injection_method: "header".to_string(),
            header_name: Some("Authorization".to_string()),
            header_value_encrypted: None,
            param_name: None,
            param_value_encrypted: None,
        }
    }

    #[test]
    fn load_with_backend_migrates_legacy_entries_without_losing_data() {
        let backend = KeychainBackend::new_mock("node-1");
        let mut config = keychain_config();
        config.signing.shared_secret_encrypted = Some(String::new());
        config
            .credentials
            .insert("openai".to_string(), header_credential());

        backend.set(KEY_AUTH_TOKEN, "nyx_nauth_test").unwrap();
        backend
            .set(KEY_SIGNING_SECRET, "00112233445566778899aabbccddeeff")
            .unwrap();
        backend
            .set(&credential_key("openai"), "Bearer sk-test")
            .unwrap();

        let vault = KeychainVault::load_with_backend(backend.clone(), &config).unwrap();

        assert_eq!(vault.get_auth_token().unwrap(), "nyx_nauth_test");
        assert_eq!(
            vault.get_signing_secret().unwrap(),
            Some("00112233445566778899aabbccddeeff".to_string())
        );
        assert_eq!(vault.get_credential("openai").unwrap(), "Bearer sk-test");
        assert_eq!(backend.get_optional(KEY_AUTH_TOKEN).unwrap(), None);
        assert_eq!(backend.get_optional(KEY_SIGNING_SECRET).unwrap(), None);
        assert_eq!(
            backend.get_optional(&credential_key("openai")).unwrap(),
            None
        );

        let stored_vault = backend.get_optional(VAULT_KEY).unwrap().unwrap();
        let parsed: VaultData = serde_json::from_str(&stored_vault).unwrap();
        assert_eq!(parsed.auth_token.as_deref(), Some("nyx_nauth_test"));
        assert_eq!(
            parsed.signing_secret.as_deref(),
            Some("00112233445566778899aabbccddeeff")
        );
        assert_eq!(
            parsed.credentials.get("openai").map(String::as_str),
            Some("Bearer sk-test")
        );
    }

    #[test]
    fn load_with_backend_propagates_vault_read_errors() {
        let backend = KeychainBackend::new_failing_mock("node-1", "keychain locked");
        let config = keychain_config();

        let err = match KeychainVault::load_with_backend(backend, &config) {
            Ok(_) => panic!("expected vault load to fail"),
            Err(err) => err,
        };

        assert!(matches!(err, Error::Keychain(message) if message.contains("keychain locked")));
    }

    #[test]
    fn migrate_from_individual_requires_all_expected_credentials() {
        let backend = KeychainBackend::new_mock("node-1");
        let mut config = keychain_config();
        config
            .credentials
            .insert("openai".to_string(), header_credential());

        backend.set(KEY_AUTH_TOKEN, "nyx_nauth_test").unwrap();

        let err = KeychainVault::migrate_from_individual(&backend, &config).unwrap_err();

        assert!(
            matches!(err, Error::Keychain(message) if message.contains("Missing credential 'openai'"))
        );
        assert_eq!(
            backend.get_optional(KEY_AUTH_TOKEN).unwrap().as_deref(),
            Some("nyx_nauth_test")
        );
        assert_eq!(backend.get_optional(VAULT_KEY).unwrap(), None);
    }
}

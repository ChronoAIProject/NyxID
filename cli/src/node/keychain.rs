use std::collections::BTreeMap;
#[cfg(test)]
use std::collections::HashMap;
#[cfg(test)]
use std::sync::Arc;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use super::config::NodeConfig;
use super::error::{Error, Result};

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
#[derive(Clone, Default, Serialize, Deserialize)]
pub struct VaultData {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signing_secret: Option<String>,
    #[serde(default)]
    pub credentials: BTreeMap<String, String>,
    #[serde(default)]
    #[serde(with = "zeroizing_string_btreemap")]
    pub pending_crypto_keys: BTreeMap<String, Zeroizing<String>>,
}

mod zeroizing_string_btreemap {
    use std::collections::BTreeMap;

    use serde::ser::SerializeMap;
    use serde::{Deserialize, Deserializer, Serializer};
    use zeroize::Zeroizing;

    pub fn serialize<S>(
        map: &BTreeMap<String, Zeroizing<String>>,
        serializer: S,
    ) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut output = serializer.serialize_map(Some(map.len()))?;
        for (key, value) in map {
            output.serialize_entry(key, value.as_str())?;
        }
        output.end()
    }

    pub fn deserialize<'de, D>(
        deserializer: D,
    ) -> Result<BTreeMap<String, Zeroizing<String>>, D::Error>
    where
        D: Deserializer<'de>,
    {
        Ok(BTreeMap::<String, String>::deserialize(deserializer)?
            .into_iter()
            .map(|(key, value)| (key, Zeroizing::new(value)))
            .collect())
    }
}

impl std::fmt::Debug for VaultData {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VaultData")
            .field(
                "auth_token",
                &self.auth_token.as_ref().map(|_| "[REDACTED]"),
            )
            .field(
                "signing_secret",
                &self.signing_secret.as_ref().map(|_| "[REDACTED]"),
            )
            .field(
                "credentials",
                &format!("[REDACTED; {} entries]", self.credentials.len()),
            )
            .field(
                "pending_crypto_keys",
                &format!("[REDACTED; {} entries]", self.pending_crypto_keys.len()),
            )
            .finish()
    }
}

/// A single-entry keychain vault that caches all secrets in memory.
/// Reads trigger one keychain prompt; subsequent access is from memory.
pub struct KeychainVault {
    backend: KeychainBackend,
    vault: Mutex<VaultData>,
}

impl KeychainVault {
    /// Create an empty vault (used during registration before any secrets exist).
    pub fn new(node_id: &str) -> Self {
        Self {
            backend: KeychainBackend::new(node_id),
            vault: Mutex::new(VaultData::default()),
        }
    }

    #[cfg(test)]
    pub fn new_mock(node_id: &str) -> Self {
        Self {
            backend: KeychainBackend::new_mock(node_id),
            vault: Mutex::new(VaultData::default()),
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
                    vault: Mutex::new(vault),
                })
            }
            None => {
                // No vault yet -- migrate from individual entries
                tracing::info!("Migrating keychain secrets into vault (one-time)");
                let vault = Self::migrate_from_individual(&backend, config)?;
                let kv = Self {
                    backend,
                    vault: Mutex::new(vault),
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

        let mut credentials = BTreeMap::new();
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
            pending_crypto_keys: BTreeMap::new(),
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
        let vault = self.vault.lock().unwrap();
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
        self.vault.lock().unwrap().auth_token = Some(token.to_string());
        self.flush()
    }

    pub fn get_auth_token(&self) -> Result<String> {
        self.vault
            .lock()
            .unwrap()
            .auth_token
            .clone()
            .ok_or_else(|| Error::Keychain("No auth token in vault".to_string()))
    }

    // -- Signing secret --

    pub fn set_signing_secret(&self, secret: &str) -> Result<()> {
        self.vault.lock().unwrap().signing_secret = Some(secret.to_string());
        self.flush()
    }

    pub fn get_signing_secret(&self) -> Result<Option<String>> {
        Ok(self.vault.lock().unwrap().signing_secret.clone())
    }

    // -- Service credentials --

    pub fn set_credential(&self, slug: &str, value: &str) -> Result<()> {
        self.vault
            .lock()
            .unwrap()
            .credentials
            .insert(slug.to_string(), value.to_string());
        self.flush()
    }

    pub fn get_credential(&self, slug: &str) -> Result<String> {
        self.vault
            .lock()
            .unwrap()
            .credentials
            .get(slug)
            .cloned()
            .ok_or_else(|| Error::Keychain(format!("No credential for '{slug}' in vault")))
    }

    pub fn delete_credential(&self, slug: &str) -> Result<()> {
        self.vault.lock().unwrap().credentials.remove(slug);
        self.flush()
    }

    pub fn set_pending_crypto_key(
        &self,
        pending_id: &str,
        private_key: &Zeroizing<String>,
    ) -> Result<()> {
        self.vault
            .lock()
            .unwrap()
            .pending_crypto_keys
            .insert(pending_id.to_string(), private_key.clone());
        self.flush()
    }

    pub fn get_pending_crypto_key(&self, pending_id: &str) -> Result<Zeroizing<String>> {
        self.vault
            .lock()
            .unwrap()
            .pending_crypto_keys
            .get(pending_id)
            .cloned()
            .ok_or_else(|| {
                Error::Keychain(format!("No pending crypto key for '{pending_id}' in vault"))
            })
    }

    pub fn delete_pending_crypto_key(&self, pending_id: &str) -> Result<()> {
        self.vault
            .lock()
            .unwrap()
            .pending_crypto_keys
            .remove(pending_id);
        self.flush()
    }

    pub fn delete_auth_token(&self) -> Result<()> {
        self.vault.lock().unwrap().auth_token = None;
        self.flush()
    }

    pub fn delete_signing_secret(&self) -> Result<()> {
        self.vault.lock().unwrap().signing_secret = None;
        self.flush()
    }

    /// Re-read the vault JSON from the keychain into memory.
    /// Called before credential reload so newly-added secrets are visible.
    pub fn refresh(&self) -> Result<()> {
        if let Some(json) = self.backend.get_optional(VAULT_KEY)? {
            let fresh: VaultData = serde_json::from_str(&json)
                .map_err(|e| Error::Keychain(format!("Corrupt vault data: {e}")))?;
            *self.vault.lock().unwrap() = fresh;
        }
        Ok(())
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
    use crate::node::config::CredentialConfig;

    fn keychain_config() -> NodeConfig {
        NodeConfig::new(
            "wss://example.com/api/v1/nodes/ws".to_string(),
            "node-1".to_string(),
            "keychain".to_string(),
        )
    }

    fn header_credential() -> CredentialConfig {
        CredentialConfig::new_header("Authorization".to_string(), None, None)
    }

    fn assert_pending_key_map_is_zeroizing(_: &BTreeMap<String, Zeroizing<String>>) {}

    #[test]
    fn pending_crypto_keys_are_zeroizing_and_serde_as_strings() {
        let mut vault = VaultData::default();
        assert_pending_key_map_is_zeroizing(&vault.pending_crypto_keys);

        vault.pending_crypto_keys.insert(
            "pending-1".to_string(),
            Zeroizing::new("private-key-b64u".to_string()),
        );

        let json = serde_json::to_string(&vault).unwrap();
        assert!(json.contains(r#""pending-1":"private-key-b64u""#));

        let parsed: VaultData = serde_json::from_str(&json).unwrap();
        assert_pending_key_map_is_zeroizing(&parsed.pending_crypto_keys);
        assert_eq!(
            parsed
                .pending_crypto_keys
                .get("pending-1")
                .map(|value| value.as_str()),
            Some("private-key-b64u")
        );
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

    #[test]
    fn refresh_picks_up_externally_added_credentials() {
        let backend = KeychainBackend::new_mock("node-1");
        let config = keychain_config();

        // Start with an empty vault
        backend.set(KEY_AUTH_TOKEN, "nyx_nauth_test").unwrap();
        let vault = KeychainVault::load_with_backend(backend.clone(), &config).unwrap();
        assert!(vault.get_credential("testing-gh4t").is_err());

        // Simulate another process adding a credential to the keychain vault
        let raw = backend.get_optional(VAULT_KEY).unwrap().unwrap();
        let mut data: VaultData = serde_json::from_str(&raw).unwrap();
        data.credentials
            .insert("testing-gh4t".to_string(), "Bearer 123456".to_string());
        let updated = serde_json::to_string(&data).unwrap();
        backend.set(VAULT_KEY, &updated).unwrap();

        // Before refresh: still stale
        assert!(vault.get_credential("testing-gh4t").is_err());

        // After refresh: picks up the new credential
        vault.refresh().unwrap();
        assert_eq!(
            vault.get_credential("testing-gh4t").unwrap(),
            "Bearer 123456"
        );
    }
}

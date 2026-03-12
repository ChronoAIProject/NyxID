#[cfg(test)]
use std::collections::HashMap;
#[cfg(test)]
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::error::{Error, Result};

const SERVICE_NAME: &str = "nyxid-node";

#[derive(Clone)]
enum KeychainClient {
    System,
    #[cfg(test)]
    Memory(Arc<Mutex<HashMap<String, String>>>),
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
        }
    }

    /// Retrieve a secret, returning None if not found.
    #[cfg(test)]
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

/// Keyring key for a service credential value.
pub fn credential_key(service_slug: &str) -> String {
    format!("cred/{service_slug}")
}

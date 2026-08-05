#![cfg(feature = "node-proxy-test")]
#![allow(dead_code)]

#[path = "node/config.rs"]
pub mod config;
#[path = "node/credential_store.rs"]
pub mod credential_store;
#[path = "node/encryption.rs"]
pub mod encryption;
#[path = "node/error.rs"]
pub mod error;
#[path = "node/keychain.rs"]
mod keychain;
#[path = "node/metrics.rs"]
mod metrics;
#[path = "node/proxy_executor.rs"]
pub mod proxy_executor;
#[path = "node/secret_backend.rs"]
mod secret_backend;
#[path = "node/signing.rs"]
mod signing;

pub mod ws_client {
    pub enum NodeWsMessage {
        Text(String),
        Binary(Vec<u8>),
    }
}

pub use credential_store::CredentialStore;
pub use metrics::NodeMetrics;
pub use signing::ReplayGuard;

pub fn no_auth_credentials(service_slug: &str, target_url: &str) -> error::Result<CredentialStore> {
    let config_dir = tempfile::tempdir()?;
    let backend = secret_backend::SecretBackend::new("file", "test-node", config_dir.path())?;
    let mut credentials = std::collections::BTreeMap::new();
    credentials.insert(
        service_slug.to_string(),
        config::CredentialConfig::new_no_auth(Some(target_url.to_string())),
    );
    let config = config::NodeConfig {
        server: config::ServerConfig { url: String::new() },
        node: config::NodeSection {
            id: "test-node".to_string(),
            auth_token_encrypted: String::new(),
        },
        signing: config::SigningConfig::default(),
        ssh: config::SshConfig::default(),
        storage_backend: "file".to_string(),
        credentials,
        ssh_keys: Vec::new(),
        pending_crypto_keys: std::collections::BTreeMap::new(),
    };
    CredentialStore::from_config_with_backend(&config, &backend)
}

#[cfg(test)]
mod test_support {
    pub fn env_lock() -> &'static std::sync::Mutex<()> {
        static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
        LOCK.get_or_init(|| std::sync::Mutex::new(()))
    }
}

#[cfg(test)]
mod node {
    pub use crate::config;
}

#![cfg(feature = "node-proxy-test")]
#![allow(dead_code)]

#[path = "node/config.rs"]
pub mod config;
#[path = "node/credential_store.rs"]
pub mod credential_store;
#[path = "node/custody.rs"]
pub(crate) mod custody;
#[path = "node/encryption.rs"]
pub mod encryption;
#[path = "node/error.rs"]
pub mod error;
#[path = "node/keychain.rs"]
mod keychain;
#[path = "node/legacy_secrets.rs"]
mod legacy_secrets;
#[path = "node/machine/runtime.rs"]
pub mod machine;
#[path = "node/managed_config.rs"]
pub mod managed_config;
#[path = "node/managed_deadline.rs"]
pub mod managed_deadline;
#[path = "node/metrics.rs"]
mod metrics;
#[path = "node/proxy_executor.rs"]
pub mod proxy_executor;
#[path = "node/proxy_upload.rs"]
pub mod proxy_upload;
#[path = "node/registration.rs"]
mod registration;
#[path = "node/secret_backend.rs"]
mod secret_backend;
#[path = "node/secret_fields.rs"]
mod secret_fields;
#[path = "node/secret_operation.rs"]
mod secret_operation;
#[path = "node/secret_snapshot.rs"]
mod secret_snapshot;
#[path = "node/secret_transaction.rs"]
mod secret_transaction;
#[path = "node/signing.rs"]
mod signing;
#[path = "node/storage.rs"]
pub mod storage;
pub mod tls;

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
    test_credentials(service_slug, target_url, None)
}

pub fn bearer_credentials(
    service_slug: &str,
    target_url: &str,
    token: &str,
) -> error::Result<CredentialStore> {
    test_credentials(service_slug, target_url, Some(token))
}

fn test_credentials(
    service_slug: &str,
    target_url: &str,
    token: Option<&str>,
) -> error::Result<CredentialStore> {
    let config_dir = tempfile::tempdir()?;
    let directory = storage::Directory::open(config_dir.path())?;
    let encryption = {
        let _lock = directory.writer_lock()?;
        encryption::LocalEncryption::initialize_in_locked(&directory)?
    };
    let mut credentials = std::collections::BTreeMap::new();
    credentials.insert(
        service_slug.to_string(),
        match token {
            Some(token) => config::CredentialConfig::new_header(
                "Authorization".into(),
                Some(encryption.encrypt(&format!("Bearer {token}"))?),
                Some(target_url.into()),
            ),
            None => config::CredentialConfig::new_no_auth(Some(target_url.to_string())),
        },
    );
    let config = config::NodeConfig {
        publication: Default::default(),
        storage_generation: None,
        keychain_format: None,
        server: config::ServerConfig {
            url: String::new(),
            proxy_max_body_size: config::DEFAULT_NODE_PROXY_MAX_BODY_SIZE,
        },
        node: config::NodeSection {
            id: "test-node".to_string(),
            auth_token_encrypted: encryption.encrypt("node-proxy-test-auth")?,
        },
        managed_runtime: None,
        managed_capabilities: None,
        signing: config::SigningConfig::default(),
        ssh: config::SshConfig::default(),
        machine: Default::default(),
        storage_backend: "file".to_string(),
        credentials,
        ssh_keys: Vec::new(),
        pending_crypto_keys: std::collections::BTreeMap::new(),
    };
    let path = config_dir.path().join("config.toml");
    config.save(&path)?;
    let config = config::NodeConfig::load(&path)?;
    let backend = secret_backend::SecretBackend::reader(&config, config_dir.path())?;
    CredentialStore::from_config_with_backend(&config, &backend)
}

#[cfg(test)]
mod test_support {
    pub fn env_lock() -> &'static std::sync::Mutex<()> {
        static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
        LOCK.get_or_init(|| std::sync::Mutex::new(()))
    }
}

pub mod node {
    pub(crate) use crate::custody;
    pub use crate::{
        config, encryption, machine, managed_config, managed_deadline, proxy_upload, ws_client,
    };
}

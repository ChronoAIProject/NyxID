use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::encryption::LocalEncryption;
use crate::error::{Error, Result};

/// Top-level configuration structure, serialized as TOML.
#[derive(Debug, Serialize, Deserialize)]
pub struct NodeConfig {
    pub server: ServerConfig,
    pub node: NodeSection,
    #[serde(default)]
    pub signing: SigningConfig,
    /// Map of service_slug -> credential config
    #[serde(default)]
    pub credentials: BTreeMap<String, CredentialConfig>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ServerConfig {
    pub url: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct NodeSection {
    pub id: String,
    /// AES-GCM encrypted auth token (base64)
    pub auth_token_encrypted: String,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct SigningConfig {
    /// AES-GCM encrypted HMAC shared secret (base64)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shared_secret_encrypted: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CredentialConfig {
    /// "header" or "query_param"
    pub injection_method: String,
    /// For header injection: the header name (e.g. "Authorization")
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub header_name: Option<String>,
    /// For header injection: AES-GCM encrypted header value (base64)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub header_value_encrypted: Option<String>,
    /// For query_param injection: the parameter name (e.g. "api_key")
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub param_name: Option<String>,
    /// For query_param injection: AES-GCM encrypted parameter value (base64)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub param_value_encrypted: Option<String>,
}

impl NodeConfig {
    /// Create a new config after registration (no credentials yet).
    pub fn new(server_url: String, node_id: String) -> Self {
        Self {
            server: ServerConfig { url: server_url },
            node: NodeSection {
                id: node_id,
                auth_token_encrypted: String::new(),
            },
            signing: SigningConfig::default(),
            credentials: BTreeMap::new(),
        }
    }

    /// Load config from a TOML file.
    pub fn load(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path).map_err(|e| {
            Error::Config(format!("Failed to read config at {}: {e}", path.display()))
        })?;
        let config: Self = toml::from_str(&content)?;
        Ok(config)
    }

    /// Save config to a TOML file.
    /// M1: Write to a temp file with correct permissions, then rename atomically
    /// to avoid a window where the file has default permissions.
    pub fn save(&self, path: &Path) -> Result<()> {
        let content = toml::to_string_pretty(self)?;
        #[cfg(unix)]
        {
            use std::io::Write;
            use std::os::unix::fs::OpenOptionsExt;
            let dir = path.parent().unwrap_or(Path::new("."));
            let tmp_path = dir.join(".config.toml.tmp");
            let mut file = std::fs::OpenOptions::new()
                .write(true)
                .create(true)
                .truncate(true)
                .mode(0o600)
                .open(&tmp_path)?;
            file.write_all(content.as_bytes())?;
            file.sync_all()?;
            std::fs::rename(&tmp_path, path)?;
        }
        #[cfg(not(unix))]
        {
            std::fs::write(path, content)?;
        }
        Ok(())
    }

    /// Encrypt and store the auth token.
    pub fn set_auth_token(&mut self, raw_token: &str, enc: &LocalEncryption) -> Result<()> {
        self.node.auth_token_encrypted = enc.encrypt(raw_token)?;
        Ok(())
    }

    /// Decrypt the stored auth token.
    pub fn decrypt_auth_token(&self, enc: &LocalEncryption) -> Result<String> {
        enc.decrypt(&self.node.auth_token_encrypted)
    }

    /// Encrypt and store the HMAC signing secret.
    pub fn set_signing_secret(&mut self, raw_secret: &str, enc: &LocalEncryption) -> Result<()> {
        self.signing.shared_secret_encrypted = Some(enc.encrypt(raw_secret)?);
        Ok(())
    }

    /// Decrypt the stored signing secret, if present.
    pub fn decrypt_signing_secret(&self, enc: &LocalEncryption) -> Result<Option<String>> {
        match &self.signing.shared_secret_encrypted {
            Some(encrypted) => Ok(Some(enc.decrypt(encrypted)?)),
            None => Ok(None),
        }
    }

    /// Add a header-based credential for a service.
    pub fn add_header_credential(
        &mut self,
        service_slug: &str,
        header_name: &str,
        header_value: &str,
        enc: &LocalEncryption,
    ) -> Result<()> {
        let encrypted_value = enc.encrypt(header_value)?;
        self.credentials.insert(
            service_slug.to_string(),
            CredentialConfig {
                injection_method: "header".to_string(),
                header_name: Some(header_name.to_string()),
                header_value_encrypted: Some(encrypted_value),
                param_name: None,
                param_value_encrypted: None,
            },
        );
        Ok(())
    }

    /// Add a query-param-based credential for a service.
    pub fn add_query_param_credential(
        &mut self,
        service_slug: &str,
        param_name: &str,
        param_value: &str,
        enc: &LocalEncryption,
    ) -> Result<()> {
        let encrypted_value = enc.encrypt(param_value)?;
        self.credentials.insert(
            service_slug.to_string(),
            CredentialConfig {
                injection_method: "query_param".to_string(),
                header_name: None,
                header_value_encrypted: None,
                param_name: Some(param_name.to_string()),
                param_value_encrypted: Some(encrypted_value),
            },
        );
        Ok(())
    }

    /// Remove a credential for a service.
    pub fn remove_credential(&mut self, service_slug: &str) -> Result<()> {
        if self.credentials.remove(service_slug).is_none() {
            return Err(Error::Config(format!(
                "No credential found for service '{service_slug}'"
            )));
        }
        Ok(())
    }
}

/// Resolve the config directory path.
/// If a custom path is provided, use it directly.
/// Otherwise, default to `~/.nyxid-node/`.
pub fn resolve_config_dir(custom_path: Option<&str>) -> PathBuf {
    if let Some(path) = custom_path {
        PathBuf::from(path)
    } else {
        directories::BaseDirs::new()
            .map(|dirs| dirs.home_dir().join(".nyxid-node"))
            .unwrap_or_else(|| PathBuf::from(".nyxid-node"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_config() {
        let dir = tempfile::tempdir().unwrap();
        let enc = LocalEncryption::load_or_generate(dir.path()).unwrap();

        let mut config = NodeConfig::new(
            "wss://example.com/api/v1/nodes/ws".to_string(),
            "test-node-id".to_string(),
        );
        config.set_auth_token("nyx_nauth_abc123", &enc).unwrap();
        config
            .add_header_credential("openai", "Authorization", "Bearer sk-test", &enc)
            .unwrap();

        let config_file = dir.path().join("config.toml");
        config.save(&config_file).unwrap();

        let loaded = NodeConfig::load(&config_file).unwrap();
        assert_eq!(loaded.node.id, "test-node-id");
        assert_eq!(loaded.server.url, "wss://example.com/api/v1/nodes/ws");
        assert!(loaded.credentials.contains_key("openai"));

        let token = loaded.decrypt_auth_token(&enc).unwrap();
        assert_eq!(token, "nyx_nauth_abc123");
    }

    #[test]
    fn remove_credential() {
        let dir = tempfile::tempdir().unwrap();
        let enc = LocalEncryption::load_or_generate(dir.path()).unwrap();

        let mut config = NodeConfig::new("wss://x".to_string(), "n".to_string());
        config.set_auth_token("tok", &enc).unwrap();
        config
            .add_header_credential("svc", "Auth", "val", &enc)
            .unwrap();
        assert!(config.credentials.contains_key("svc"));

        config.remove_credential("svc").unwrap();
        assert!(!config.credentials.contains_key("svc"));

        assert!(config.remove_credential("nonexistent").is_err());
    }
}

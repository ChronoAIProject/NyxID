use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::encryption::LocalEncryption;
use super::error::{Error, Result};
use super::secret_backend::SecretBackend;

/// Top-level configuration structure, serialized as TOML.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeConfig {
    pub server: ServerConfig,
    pub node: NodeSection,
    #[serde(default)]
    pub signing: SigningConfig,
    #[serde(default)]
    pub ssh: SshConfig,
    /// "file" (default, AES-GCM encrypted) or "keychain" (OS keychain)
    #[serde(default = "default_storage_backend")]
    pub storage_backend: String,
    /// Map of service_slug -> credential config
    #[serde(default)]
    pub credentials: BTreeMap<String, CredentialConfig>,
}

fn default_storage_backend() -> String {
    "file".to_string()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    pub url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeSection {
    pub id: String,
    /// AES-GCM encrypted auth token (base64)
    pub auth_token_encrypted: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SigningConfig {
    /// AES-GCM encrypted HMAC shared secret (base64)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shared_secret_encrypted: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SshConfig {
    #[serde(default = "default_max_ssh_tunnels")]
    pub max_tunnels: usize,
    /// Idle timeout for each SSH TCP read/write. Defaults to 3600s to match
    /// NyxID's tunnel lifetime cap; lower it if you want more aggressive
    /// cleanup of stalled or idle interactive sessions.
    #[serde(default = "default_ssh_io_timeout_secs")]
    pub io_timeout_secs: u64,
    #[serde(default)]
    pub allowed_targets: Vec<SshTargetConfig>,
}

impl Default for SshConfig {
    fn default() -> Self {
        Self {
            max_tunnels: default_max_ssh_tunnels(),
            io_timeout_secs: default_ssh_io_timeout_secs(),
            allowed_targets: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SshTargetConfig {
    pub host: String,
    #[serde(default)]
    pub port: Option<u16>,
}

fn default_max_ssh_tunnels() -> usize {
    10
}

fn default_ssh_io_timeout_secs() -> u64 {
    3600
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CredentialConfig {
    /// "header" or "query_param"
    pub injection_method: String,
    /// Target URL for this service (e.g., "https://api.openai.com/v1").
    /// Used when NyxID sends an empty base_url (node-resolved routing).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_url: Option<String>,
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

    // --- OAuth fields ---
    /// OAuth-managed credential: token refresh handled automatically
    #[serde(default)]
    pub oauth_managed: bool,
    /// OAuth token URL (for refresh)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oauth_token_url: Option<String>,
    /// AES-GCM encrypted OAuth access token (base64)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oauth_access_token_encrypted: Option<String>,
    /// AES-GCM encrypted OAuth refresh token (base64)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oauth_refresh_token_encrypted: Option<String>,
    /// Token expiry time (ISO 8601)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oauth_token_expires_at: Option<String>,
    /// AES-GCM encrypted OAuth client ID (base64)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oauth_client_id_encrypted: Option<String>,
    /// AES-GCM encrypted OAuth client secret (base64)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oauth_client_secret_encrypted: Option<String>,
    /// OAuth scopes (space-separated)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oauth_scopes: Option<String>,
    /// Token endpoint auth method: "client_secret_post" | "client_secret_basic"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oauth_token_endpoint_auth_method: Option<String>,
    /// Alternate OAuth client ID parameter name (e.g. "client_key" for TikTok)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub oauth_client_id_param_name: Option<String>,
}

impl CredentialConfig {
    /// Create a header credential config with default OAuth fields.
    pub fn new_header(
        header_name: String,
        header_value_encrypted: Option<String>,
        target_url: Option<String>,
    ) -> Self {
        Self {
            injection_method: "header".to_string(),
            target_url,
            header_name: Some(header_name),
            header_value_encrypted,
            param_name: None,
            param_value_encrypted: None,
            oauth_managed: false,
            oauth_token_url: None,
            oauth_access_token_encrypted: None,
            oauth_refresh_token_encrypted: None,
            oauth_token_expires_at: None,
            oauth_client_id_encrypted: None,
            oauth_client_secret_encrypted: None,
            oauth_scopes: None,
            oauth_token_endpoint_auth_method: None,
            oauth_client_id_param_name: None,
        }
    }

    /// Create a query_param credential config with default OAuth fields.
    pub fn new_query_param(
        param_name: String,
        param_value_encrypted: Option<String>,
        target_url: Option<String>,
    ) -> Self {
        Self {
            injection_method: "query_param".to_string(),
            target_url,
            header_name: None,
            header_value_encrypted: None,
            param_name: Some(param_name),
            param_value_encrypted,
            oauth_managed: false,
            oauth_token_url: None,
            oauth_access_token_encrypted: None,
            oauth_refresh_token_encrypted: None,
            oauth_token_expires_at: None,
            oauth_client_id_encrypted: None,
            oauth_client_secret_encrypted: None,
            oauth_scopes: None,
            oauth_token_endpoint_auth_method: None,
            oauth_client_id_param_name: None,
        }
    }

    /// Create a path_prefix credential config.
    /// Reuses `header_name` for the prefix and `header_value_encrypted` for
    /// the credential, keeping the TOML schema flat.
    pub fn new_path_prefix(
        prefix: String,
        credential_encrypted: Option<String>,
        target_url: Option<String>,
    ) -> Self {
        Self {
            injection_method: "path_prefix".to_string(),
            target_url,
            header_name: Some(prefix),
            header_value_encrypted: credential_encrypted,
            param_name: None,
            param_value_encrypted: None,
            oauth_managed: false,
            oauth_token_url: None,
            oauth_access_token_encrypted: None,
            oauth_refresh_token_encrypted: None,
            oauth_token_expires_at: None,
            oauth_client_id_encrypted: None,
            oauth_client_secret_encrypted: None,
            oauth_scopes: None,
            oauth_token_endpoint_auth_method: None,
            oauth_client_id_param_name: None,
        }
    }

    /// Create a no-auth placeholder config. The entry exists so the
    /// node can resolve `target_url` and accept proxy requests, but
    /// no credential gets injected. Used when a server-held service
    /// is downgraded to `auth_method: "none"` — we cannot drop the
    /// entry outright because `proxy_executor` 502s with
    /// "No credentials configured" before it even considers
    /// `base_url` (thirty-third-round Codex P1).
    pub fn new_no_auth(target_url: Option<String>) -> Self {
        Self {
            injection_method: "none".to_string(),
            target_url,
            header_name: None,
            header_value_encrypted: None,
            param_name: None,
            param_value_encrypted: None,
            oauth_managed: false,
            oauth_token_url: None,
            oauth_access_token_encrypted: None,
            oauth_refresh_token_encrypted: None,
            oauth_token_expires_at: None,
            oauth_client_id_encrypted: None,
            oauth_client_secret_encrypted: None,
            oauth_scopes: None,
            oauth_token_endpoint_auth_method: None,
            oauth_client_id_param_name: None,
        }
    }
}

impl NodeConfig {
    /// Create a new config after registration (no credentials yet).
    pub fn new(server_url: String, node_id: String, storage_backend: String) -> Self {
        Self {
            server: ServerConfig { url: server_url },
            node: NodeSection {
                id: node_id,
                auth_token_encrypted: String::new(),
            },
            signing: SigningConfig::default(),
            ssh: SshConfig::default(),
            storage_backend,
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

    /// Add a header-based credential for a service (file backend only).
    #[cfg(test)]
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
            CredentialConfig::new_header(header_name.to_string(), Some(encrypted_value), None),
        );
        Ok(())
    }

    /// Add a query-param-based credential for a service (file backend only).
    #[cfg(test)]
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
            CredentialConfig::new_query_param(param_name.to_string(), Some(encrypted_value), None),
        );
        Ok(())
    }

    /// Remove a credential for a service (file backend only).
    #[cfg(test)]
    pub fn remove_credential(&mut self, service_slug: &str) -> Result<()> {
        if self.credentials.remove(service_slug).is_none() {
            return Err(Error::Config(format!(
                "No credential found for service '{service_slug}'"
            )));
        }
        Ok(())
    }

    /// Three-state resolution for `target_url` on an incoming
    /// `credential_update` frame:
    ///
    /// - `Some("url")`: set target to that URL.
    /// - `Some("")`: explicit clear — drop the stored URL so the node
    ///   falls back to its local config. The NyxID backend emits this
    ///   when a service's `endpoint_url` is reset to `""` (switching
    ///   back to node-local target resolution).
    /// - `None`: field omitted — preserve whatever URL is already
    ///   stored. Used on pure credential rotations where the backend
    ///   isn't touching the endpoint URL; without this fallback,
    ///   `self.credentials.insert` would replace the whole
    ///   `CredentialConfig` and silently clear the node-local
    ///   downstream URL that services like HA Supervisor depend on.
    fn resolve_target_url(&self, service_slug: &str, incoming: Option<&str>) -> Option<String> {
        match incoming {
            Some("") => None,
            Some(url) => Some(url.to_string()),
            None => self
                .credentials
                .get(service_slug)
                .and_then(|c| c.target_url.clone()),
        }
    }

    /// Add a header credential using the configured secret backend.
    pub fn add_header_credential_via(
        &mut self,
        service_slug: &str,
        header_name: &str,
        header_value: &str,
        target_url: Option<&str>,
        backend: &SecretBackend,
    ) -> Result<()> {
        let encrypted = backend.store_credential_value(service_slug, header_value)?;
        let resolved_target_url = self.resolve_target_url(service_slug, target_url);
        self.credentials.insert(
            service_slug.to_string(),
            CredentialConfig::new_header(header_name.to_string(), encrypted, resolved_target_url),
        );
        Ok(())
    }

    /// Add a query-param credential using the configured secret backend.
    pub fn add_query_param_credential_via(
        &mut self,
        service_slug: &str,
        param_name: &str,
        param_value: &str,
        target_url: Option<&str>,
        backend: &SecretBackend,
    ) -> Result<()> {
        let encrypted = backend.store_credential_value(service_slug, param_value)?;
        let resolved_target_url = self.resolve_target_url(service_slug, target_url);
        self.credentials.insert(
            service_slug.to_string(),
            CredentialConfig::new_query_param(
                param_name.to_string(),
                encrypted,
                resolved_target_url,
            ),
        );
        Ok(())
    }

    /// Add a path-prefix credential using the configured secret backend.
    pub fn add_path_prefix_credential_via(
        &mut self,
        service_slug: &str,
        prefix: &str,
        credential: &str,
        target_url: Option<&str>,
        backend: &SecretBackend,
    ) -> Result<()> {
        let encrypted = backend.store_credential_value(service_slug, credential)?;
        let resolved_target_url = self.resolve_target_url(service_slug, target_url);
        self.credentials.insert(
            service_slug.to_string(),
            CredentialConfig::new_path_prefix(prefix.to_string(), encrypted, resolved_target_url),
        );
        Ok(())
    }

    /// Remove a credential, also cleaning up from the secret backend.
    pub fn remove_credential_via(
        &mut self,
        service_slug: &str,
        backend: &SecretBackend,
    ) -> Result<()> {
        if self.credentials.remove(service_slug).is_none() {
            return Err(Error::Config(format!(
                "No credential found for service '{service_slug}'"
            )));
        }
        backend.delete_credential(service_slug)?;
        Ok(())
    }

    /// Set a no-auth placeholder entry for a service: preserves (or
    /// resolves) the target URL but drops any previously-stored
    /// secret. Used when NyxID pushes a `credential_update` after the
    /// user downgrades `auth_method` to `none` on a server-held
    /// service. Cleans up the secret-backend entry if one was present
    /// so stale material doesn't linger (thirty-third-round Codex P1).
    pub fn set_no_auth_via(
        &mut self,
        service_slug: &str,
        target_url: Option<&str>,
        backend: &SecretBackend,
    ) -> Result<()> {
        let resolved_target_url = self.resolve_target_url(service_slug, target_url);
        let had_secret = self.credentials.get(service_slug).is_some_and(|c| {
            c.header_value_encrypted.is_some() || c.param_value_encrypted.is_some()
        });
        self.credentials.insert(
            service_slug.to_string(),
            CredentialConfig::new_no_auth(resolved_target_url),
        );
        if had_secret {
            // Best-effort: a stale secret left behind wouldn't be
            // injected (the new entry is injection_method="none"),
            // but cleaning up keeps the backend tidy.
            let _ = backend.delete_credential(service_slug);
        }
        Ok(())
    }
}

/// Resolve the config directory path.
/// If a custom path is provided, use it directly.
/// Otherwise, default to `~/.nyxid-node/`.
pub fn resolve_config_dir(custom_path: Option<&str>) -> PathBuf {
    // None profile always passes validation, safe to unwrap
    resolve_config_dir_with_profile(custom_path, None)
        .expect("resolve_config_dir with None profile should never fail")
}

/// Resolve the config directory with profile support.
/// `None` or `Some("default")` = `~/.nyxid-node/`
/// `Some(name)` = `~/.nyxid-node/profiles/{name}/`
///
/// Returns an error if the profile name fails validation (path traversal prevention).
pub fn resolve_config_dir_with_profile(
    custom_path: Option<&str>,
    profile: Option<&str>,
) -> Result<PathBuf> {
    if let Some(path) = custom_path {
        return Ok(PathBuf::from(path));
    }

    let base = directories::BaseDirs::new()
        .map(|dirs| dirs.home_dir().join(".nyxid-node"))
        .unwrap_or_else(|| PathBuf::from(".nyxid-node"));

    match profile {
        None | Some("default") => Ok(base),
        Some(name) => {
            validate_node_profile_name(name)?;
            Ok(base.join("profiles").join(name))
        }
    }
}

/// Validate a node profile name: 1-64 characters, alphanumeric + hyphens + underscores only.
/// This prevents path traversal attacks (e.g. `../../etc`, `foo/bar`).
fn validate_node_profile_name(name: &str) -> Result<()> {
    if name.is_empty() || name.len() > 64 {
        return Err(Error::Validation(
            "Profile name must be 1-64 characters".into(),
        ));
    }
    if !name
        .chars()
        .all(|c| c.is_alphanumeric() || c == '-' || c == '_')
    {
        return Err(Error::Validation(format!(
            "Profile name must contain only alphanumeric characters, hyphens, \
             and underscores (got '{name}')"
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_dir_default_profile() {
        let dir_none = resolve_config_dir_with_profile(None, None).unwrap();
        let dir_default = resolve_config_dir_with_profile(None, Some("default")).unwrap();
        assert_eq!(dir_none, dir_default);
    }

    #[test]
    fn config_dir_named_profile() {
        let dir = resolve_config_dir_with_profile(None, Some("test-agent")).unwrap();
        assert!(dir.to_string_lossy().contains("profiles/test-agent"));
    }

    #[test]
    fn config_dir_custom_path_ignores_profile() {
        let dir = resolve_config_dir_with_profile(Some("/tmp/custom"), Some("agent")).unwrap();
        assert_eq!(dir, PathBuf::from("/tmp/custom"));
    }

    #[test]
    fn config_dir_rejects_path_traversal() {
        let result = resolve_config_dir_with_profile(None, Some("../../etc"));
        assert!(result.is_err());
    }

    #[test]
    fn config_dir_rejects_slash_in_profile() {
        let result = resolve_config_dir_with_profile(None, Some("foo/bar"));
        assert!(result.is_err());
    }

    #[test]
    fn roundtrip_config() {
        let dir = tempfile::tempdir().unwrap();
        let enc = LocalEncryption::load_or_generate(dir.path()).unwrap();

        let mut config = NodeConfig::new(
            "wss://example.com/api/v1/nodes/ws".to_string(),
            "test-node-id".to_string(),
            "file".to_string(),
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

        let mut config =
            NodeConfig::new("wss://x".to_string(), "n".to_string(), "file".to_string());
        config.set_auth_token("tok", &enc).unwrap();
        config
            .add_header_credential("svc", "Auth", "val", &enc)
            .unwrap();
        assert!(config.credentials.contains_key("svc"));

        config.remove_credential("svc").unwrap();
        assert!(!config.credentials.contains_key("svc"));

        assert!(config.remove_credential("nonexistent").is_err());
    }

    #[test]
    fn deserialize_defaults_ssh_settings_for_existing_configs() {
        let config: NodeConfig = toml::from_str(
            r#"
                [server]
                url = "wss://example.com/api/v1/nodes/ws"

                [node]
                id = "test-node-id"
                auth_token_encrypted = "ciphertext"
            "#,
        )
        .unwrap();

        assert_eq!(config.ssh.max_tunnels, 10);
        assert_eq!(config.ssh.io_timeout_secs, 3600);
        assert!(config.ssh.allowed_targets.is_empty());
    }
}

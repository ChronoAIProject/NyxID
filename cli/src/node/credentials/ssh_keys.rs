use std::fmt;

use chrono::{DateTime, Utc};
use zeroize::Zeroizing;

use crate::node::config::{NodeConfig, SshAlgorithmPreferences, SshKeyConfig};
use crate::node::error::{Error, Result};
use crate::node::secret_backend::SecretBackend;

pub const SSH_PRIVATE_KEY_SUFFIX: &str = "private_key";
pub const SSH_PASSPHRASE_SUFFIX: &str = "passphrase";

pub struct SshKeyEntry {
    pub service_slug: String,
    pub principal: String,
    pub private_key_pem: Zeroizing<String>,
    pub passphrase: Option<Zeroizing<String>>,
    pub target_host: String,
    pub target_port: u16,
    pub host_key_sha256: Option<String>,
    pub algorithms: Option<SshAlgorithmPreferences>,
    pub created_at: DateTime<Utc>,
}

pub struct NewSshKeyEntry {
    pub service_slug: String,
    pub principal: String,
    pub private_key_pem: Zeroizing<String>,
    pub passphrase: Option<Zeroizing<String>>,
    pub target_host: String,
    pub target_port: u16,
    pub host_key_sha256: Option<String>,
    pub algorithms: Option<SshAlgorithmPreferences>,
}

impl fmt::Debug for SshKeyEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SshKeyEntry")
            .field("service_slug", &self.service_slug)
            .field("principal", &self.principal)
            .field("private_key_pem", &"<redacted>")
            .field(
                "passphrase",
                &self.passphrase.as_ref().map(|_| "<redacted>"),
            )
            .field("target_host", &self.target_host)
            .field("target_port", &self.target_port)
            .field("host_key_sha256", &self.host_key_sha256)
            .field("algorithms", &self.algorithms)
            .field("created_at", &self.created_at)
            .finish()
    }
}

impl fmt::Debug for NewSshKeyEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("NewSshKeyEntry")
            .field("service_slug", &self.service_slug)
            .field("principal", &self.principal)
            .field("private_key_pem", &"<redacted>")
            .field(
                "passphrase",
                &self.passphrase.as_ref().map(|_| "<redacted>"),
            )
            .field("target_host", &self.target_host)
            .field("target_port", &self.target_port)
            .field("host_key_sha256", &self.host_key_sha256)
            .field("algorithms", &self.algorithms)
            .finish()
    }
}

pub fn find<'a>(
    entries: &'a [SshKeyEntry],
    service_slug: &str,
    principal: &str,
) -> Option<&'a SshKeyEntry> {
    entries
        .iter()
        .find(|entry| entry.service_slug == service_slug && entry.principal == principal)
}

pub fn config_find<'a>(
    config: &'a NodeConfig,
    service_slug: &str,
    principal: &str,
) -> Option<&'a SshKeyConfig> {
    config
        .ssh_keys
        .iter()
        .find(|entry| entry.service_slug == service_slug && entry.principal == principal)
}

pub fn principals_for_service(config: &NodeConfig, service_slug: &str) -> Vec<String> {
    let mut principals = config
        .ssh_keys
        .iter()
        .filter(|entry| entry.service_slug == service_slug)
        .map(|entry| entry.principal.clone())
        .collect::<Vec<_>>();
    principals.sort();
    principals.dedup();
    principals
}

pub fn load_entries(config: &NodeConfig, backend: &SecretBackend) -> Result<Vec<SshKeyEntry>> {
    config
        .ssh_keys
        .iter()
        .map(|entry| load_entry(entry, backend))
        .collect()
}

pub fn load_entry(config: &SshKeyConfig, backend: &SecretBackend) -> Result<SshKeyEntry> {
    let private_key = backend.load_credential_value(
        &secret_key_name(
            &config.service_slug,
            &config.principal,
            SSH_PRIVATE_KEY_SUFFIX,
        ),
        config.private_key_pem_encrypted.as_deref(),
    )?;

    let passphrase = match &config.passphrase_encrypted {
        Some(encrypted) => Some(Zeroizing::new(backend.load_credential_value(
            &secret_key_name(
                &config.service_slug,
                &config.principal,
                SSH_PASSPHRASE_SUFFIX,
            ),
            Some(encrypted),
        )?)),
        None => None,
    };

    let created_at = chrono::DateTime::parse_from_rfc3339(&config.created_at)
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(|_| Utc::now());

    Ok(SshKeyEntry {
        service_slug: config.service_slug.clone(),
        principal: config.principal.clone(),
        private_key_pem: Zeroizing::new(private_key),
        passphrase,
        target_host: config.target_host.clone(),
        target_port: config.target_port,
        host_key_sha256: config.host_key_sha256.clone(),
        algorithms: config.algorithms.clone(),
        created_at,
    })
}

pub fn add_entry(
    config: &mut NodeConfig,
    backend: &SecretBackend,
    entry: NewSshKeyEntry,
) -> Result<()> {
    validate_key_selector(&entry.service_slug, &entry.principal)?;
    validate_target(&entry.target_host, entry.target_port)?;
    let algorithms = entry.algorithms.filter(|prefs| !prefs.is_empty());
    if let Some(prefs) = &algorithms {
        prefs.validate()?;
    }

    if config_find(config, &entry.service_slug, &entry.principal).is_some() {
        return Err(Error::Validation(format!(
            "SSH key already exists for service '{}' principal '{}'",
            entry.service_slug, entry.principal
        )));
    }

    let private_key_pem_encrypted = backend.store_credential_value(
        &secret_key_name(
            &entry.service_slug,
            &entry.principal,
            SSH_PRIVATE_KEY_SUFFIX,
        ),
        entry.private_key_pem.as_str(),
    )?;

    let passphrase_encrypted = match entry.passphrase.as_ref() {
        Some(passphrase) => Some(backend.store_credential_value(
            &secret_key_name(&entry.service_slug, &entry.principal, SSH_PASSPHRASE_SUFFIX),
            passphrase.as_str(),
        )?),
        None => None,
    }
    .flatten();

    config.ssh_keys.push(SshKeyConfig {
        service_slug: entry.service_slug,
        principal: entry.principal,
        private_key_pem_encrypted,
        passphrase_encrypted,
        target_host: entry.target_host,
        target_port: entry.target_port,
        host_key_sha256: entry.host_key_sha256,
        algorithms,
        created_at: Utc::now().to_rfc3339(),
    });
    config
        .ssh_keys
        .sort_by(|a, b| (&a.service_slug, &a.principal).cmp(&(&b.service_slug, &b.principal)));

    Ok(())
}

pub fn remove_entry(
    config: &mut NodeConfig,
    backend: &SecretBackend,
    service_slug: &str,
    principal: &str,
) -> Result<SshKeyConfig> {
    let Some(index) = config
        .ssh_keys
        .iter()
        .position(|entry| entry.service_slug == service_slug && entry.principal == principal)
    else {
        return Err(Error::Config(format!(
            "No SSH key found for service '{service_slug}' principal '{principal}'"
        )));
    };

    let removed = config.ssh_keys.remove(index);
    backend.delete_credential(&secret_key_name(
        service_slug,
        principal,
        SSH_PRIVATE_KEY_SUFFIX,
    ))?;
    backend.delete_credential(&secret_key_name(
        service_slug,
        principal,
        SSH_PASSPHRASE_SUFFIX,
    ))?;
    Ok(removed)
}

pub fn secret_key_name(service_slug: &str, principal: &str, suffix: &str) -> String {
    format!("ssh-key/{service_slug}/{principal}/{suffix}")
}

fn validate_key_selector(service_slug: &str, principal: &str) -> Result<()> {
    if service_slug.trim().is_empty() {
        return Err(Error::Validation("Service slug is required".to_string()));
    }
    if principal.trim().is_empty() {
        return Err(Error::Validation("SSH principal is required".to_string()));
    }
    Ok(())
}

fn validate_target(target_host: &str, target_port: u16) -> Result<()> {
    if target_host.trim().is_empty() {
        return Err(Error::Validation("Target host is required".to_string()));
    }
    if target_port == 0 {
        return Err(Error::Validation(
            "Target port must be non-zero".to_string(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::encryption::LocalEncryption;

    fn file_backend(dir: &tempfile::TempDir) -> SecretBackend {
        SecretBackend::File(LocalEncryption::load_or_generate(dir.path()).unwrap())
    }

    fn test_entry(service_slug: &str, principal: &str) -> NewSshKeyEntry {
        NewSshKeyEntry {
            service_slug: service_slug.to_string(),
            principal: principal.to_string(),
            private_key_pem: Zeroizing::new(
                "-----BEGIN OPENSSH PRIVATE KEY-----\ntest\n-----END OPENSSH PRIVATE KEY-----\n"
                    .to_string(),
            ),
            passphrase: None,
            target_host: "10.0.0.1".to_string(),
            target_port: 22,
            host_key_sha256: None,
            algorithms: None,
        }
    }

    #[test]
    fn find_matches_service_slug_and_principal() {
        let entries = vec![
            SshKeyEntry {
                service_slug: "routeros".to_string(),
                principal: "nyxid-ro".to_string(),
                private_key_pem: Zeroizing::new("ro-key".to_string()),
                passphrase: None,
                target_host: "10.0.0.1".to_string(),
                target_port: 22,
                host_key_sha256: None,
                algorithms: None,
                created_at: Utc::now(),
            },
            SshKeyEntry {
                service_slug: "routeros".to_string(),
                principal: "nyxid-admin".to_string(),
                private_key_pem: Zeroizing::new("admin-key".to_string()),
                passphrase: None,
                target_host: "10.0.0.1".to_string(),
                target_port: 22,
                host_key_sha256: None,
                algorithms: None,
                created_at: Utc::now(),
            },
        ];

        let found = find(&entries, "routeros", "nyxid-admin").unwrap();
        assert_eq!(found.private_key_pem.as_str(), "admin-key");
        assert!(find(&entries, "routeros", "missing").is_none());
    }

    #[test]
    fn add_rejects_duplicate_service_principal_pair() {
        let dir = tempfile::tempdir().unwrap();
        let backend = file_backend(&dir);
        let mut config = NodeConfig::new(
            "wss://example.test/api/v1/nodes/ws".to_string(),
            "node-1".to_string(),
            "file".to_string(),
        );

        add_entry(&mut config, &backend, test_entry("routeros", "nyxid-ro")).unwrap();
        let err = add_entry(&mut config, &backend, test_entry("routeros", "nyxid-ro")).unwrap_err();

        assert!(matches!(err, Error::Validation(message) if message.contains("already exists")));
    }

    #[test]
    fn add_accepts_duplicate_service_with_different_principal() {
        let dir = tempfile::tempdir().unwrap();
        let backend = file_backend(&dir);
        let mut config = NodeConfig::new(
            "wss://example.test/api/v1/nodes/ws".to_string(),
            "node-1".to_string(),
            "file".to_string(),
        );

        add_entry(&mut config, &backend, test_entry("routeros", "nyxid-ro")).unwrap();
        add_entry(&mut config, &backend, test_entry("routeros", "nyxid-admin")).unwrap();

        assert_eq!(
            principals_for_service(&config, "routeros"),
            vec!["nyxid-admin".to_string(), "nyxid-ro".to_string()]
        );
    }

    #[test]
    fn ssh_key_toml_roundtrip_without_algorithms_keeps_config_clean() {
        let config = SshKeyConfig {
            service_slug: "routeros".to_string(),
            principal: "admin".to_string(),
            private_key_pem_encrypted: Some("encrypted-key".to_string()),
            passphrase_encrypted: None,
            target_host: "10.0.0.1".to_string(),
            target_port: 22,
            host_key_sha256: None,
            algorithms: None,
            created_at: "2026-01-01T00:00:00Z".to_string(),
        };

        let encoded = toml::to_string(&config).unwrap();
        assert!(!encoded.contains("algorithms"));

        let decoded: SshKeyConfig = toml::from_str(&encoded).unwrap();
        assert!(decoded.algorithms.is_none());
    }

    #[test]
    fn ssh_key_toml_roundtrip_with_algorithms_preserves_lists() {
        let algorithms = SshAlgorithmPreferences {
            kex: Some(vec!["diffie-hellman-group-exchange-sha256".to_string()]),
            host_key: Some(vec!["rsa-sha2-256".to_string(), "ssh-rsa".to_string()]),
            cipher: Some(vec!["aes256-ctr".to_string()]),
            mac: Some(vec!["hmac-sha2-256".to_string()]),
        };
        let config = SshKeyConfig {
            service_slug: "routeros".to_string(),
            principal: "admin".to_string(),
            private_key_pem_encrypted: Some("encrypted-key".to_string()),
            passphrase_encrypted: None,
            target_host: "10.0.0.1".to_string(),
            target_port: 22,
            host_key_sha256: Some("SHA256:abc123".to_string()),
            algorithms: Some(algorithms.clone()),
            created_at: "2026-01-01T00:00:00Z".to_string(),
        };

        let encoded = toml::to_string(&config).unwrap();
        assert!(encoded.contains("[algorithms]"));

        let decoded: SshKeyConfig = toml::from_str(&encoded).unwrap();
        assert_eq!(decoded.algorithms, Some(algorithms));
    }
}

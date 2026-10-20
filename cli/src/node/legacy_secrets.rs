//! Read-only interpretation of legacy keychain custody. Loading never creates,
//! consolidates or deletes entries, and never falls back from a present vault.
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use zeroize::Zeroizing;

use super::config::{KeychainFormat, NodeConfig};
use super::error::{Error, Result};
use super::keychain::{KeychainBackend, credential_key};
use super::secret_fields::{self, SecretField};
use super::secret_transaction::ObjectRef;
use super::storage::MAX_CONFIG_BYTES;

fn invalid() -> Error {
    Error::Keychain("Authoritative legacy node secret storage is unavailable or invalid".into())
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct LegacyVault {
    #[serde(default)]
    auth_token: Option<Zeroizing<String>>,
    #[serde(default)]
    signing_secret: Option<Zeroizing<String>>,
    #[serde(default)]
    credentials: BTreeMap<String, Zeroizing<String>>,
    #[serde(default)]
    pending_crypto_keys: BTreeMap<String, Zeroizing<String>>,
}
impl LegacyVault {
    fn get(&self, field: &SecretField) -> Option<&Zeroizing<String>> {
        match field {
            SecretField::AuthToken => self.auth_token.as_ref(),
            SecretField::SigningSecret => self.signing_secret.as_ref(),
            SecretField::PendingPrivate { pending_id } => self.pending_crypto_keys.get(pending_id),
            _ => self.credentials.get(&field.legacy_key()),
        }
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum LegacySource {
    Vault,
    Individual,
}
impl std::fmt::Debug for LegacySource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("LegacySource([REDACTED])")
    }
}

/// All legacy fields from a single authoritative interpretation. A subsequent
/// health check must read again; this object never substitutes for that check.
pub(crate) struct LegacySnapshot {
    pub(crate) source: Option<LegacySource>,
    values: BTreeMap<SecretField, Option<Zeroizing<String>>>,
}
impl std::fmt::Debug for LegacySnapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("LegacySnapshot { secrets_and_locations: [REDACTED] }")
    }
}
impl LegacySnapshot {
    pub(crate) fn read(config: &NodeConfig, backend: &KeychainBackend) -> Result<Self> {
        Self::read_anchored(config, backend, None)
    }

    /// A live runtime keeps the initially selected custody source. Losing its
    /// vault cannot make stale individual entries authoritative on the next
    /// health check. Backend/format transitions require a stopped runtime.
    pub(crate) fn reread(&self, config: &NodeConfig, backend: &KeychainBackend) -> Result<Self> {
        Self::read_anchored(config, backend, Some(&self.source))
    }

    fn read_anchored(
        config: &NodeConfig,
        backend: &KeychainBackend,
        anchor: Option<&Option<LegacySource>>,
    ) -> Result<Self> {
        if config.storage_backend != "keychain" || config.node.id != backend.owner_id() {
            return Err(invalid());
        }
        // The same contract applies to direct readers and publication/GC.
        // Unmarked or mixed refs must fail before any legacy backend read.
        super::secret_transaction::live_references(config)?;
        if config.keychain_format == Some(KeychainFormat::ImmutableV1) {
            if anchor.is_some_and(|source| source.is_some()) {
                return Err(invalid());
            }
            return Ok(Self {
                source: None,
                values: BTreeMap::new(),
            });
        }
        secret_fields::validate_legacy_namespace(config)?;
        let mut fields = Vec::new();
        for (field, required) in secret_fields::fields(config)? {
            if field
                .encoded(config)?
                .map(ObjectRef::parse)
                .transpose()?
                .flatten()
                .is_none()
            {
                // Only empty/absent legacy keychain markers are supported.
                if field
                    .encoded(config)?
                    .is_some_and(|value| !value.is_empty())
                {
                    return Err(invalid());
                }
                fields.push((field, required));
            }
        }
        if fields.is_empty() {
            if anchor.is_some_and(|source| source.is_some()) {
                return Err(invalid());
            }
            return Ok(Self {
                source: None,
                values: BTreeMap::new(),
            });
        }
        let raw = backend
            .get_optional("vault")
            .map_err(|_| invalid())?
            .map(Zeroizing::new);
        let vault = raw
            .as_ref()
            .map(|json| {
                if json.len() as u64 > MAX_CONFIG_BYTES {
                    return Err(invalid());
                }
                serde_json::from_str::<LegacyVault>(json).map_err(|_| invalid())
            })
            .transpose()?;
        let source = if vault.is_some() {
            LegacySource::Vault
        } else {
            LegacySource::Individual
        };
        if anchor.is_some_and(|expected| expected.as_ref() != Some(&source)) {
            return Err(invalid());
        }
        let mut values = BTreeMap::new();
        for (field, required) in fields {
            let value = if let Some(vault) = &vault {
                // A missing vault member never falls back to a stale individual.
                vault.get(&field).cloned()
            } else {
                let key = match &field {
                    SecretField::AuthToken | SecretField::SigningSecret => Some(field.legacy_key()),
                    // Pending crypto keys were introduced in the vault format;
                    // no speculative individual namespace is authoritative.
                    SecretField::PendingPrivate { .. } => None,
                    _ => Some(credential_key(&field.legacy_key())),
                };
                key.map(|key| backend.get_optional(&key).map_err(|_| invalid()))
                    .transpose()?
                    .flatten()
                    .map(Zeroizing::new)
            };
            if required && value.is_none() {
                return Err(invalid());
            }
            values.insert(field, value);
        }
        Ok(Self {
            source: Some(source),
            values,
        })
    }
    /// Refuse to delete a vault containing unmodelled secrets. Shadowed legacy
    /// entries are inventoried separately; their exact bytes are fenced before
    /// retirement and never become authoritative snapshot values.
    pub(crate) fn retirement_keys(
        config: &NodeConfig,
        backend: &KeychainBackend,
    ) -> Result<Vec<String>> {
        let fields = secret_fields::fields(config)?;
        let has_signing = fields
            .iter()
            .any(|(field, _)| matches!(field, SecretField::SigningSecret));
        let mut keys = vec!["vault".into()];
        let mut credential_keys = std::collections::BTreeSet::new();
        let mut pending_keys = std::collections::BTreeSet::new();
        for (field, _) in fields {
            match &field {
                SecretField::AuthToken | SecretField::SigningSecret => {
                    keys.push(field.legacy_key())
                }
                SecretField::PendingPrivate { pending_id } => {
                    pending_keys.insert(pending_id.clone());
                }
                _ => {
                    credential_keys.insert(field.legacy_key());
                    keys.push(credential_key(&field.legacy_key()));
                }
            }
        }
        if let Some(raw) = backend.get_optional("vault").map_err(|_| invalid())? {
            let raw = Zeroizing::new(raw);
            let vault: LegacyVault = serde_json::from_str(&raw).map_err(|_| invalid())?;
            if (vault.signing_secret.is_some() && !has_signing)
                || vault
                    .credentials
                    .keys()
                    .any(|key| !credential_keys.contains(key))
                || vault
                    .pending_crypto_keys
                    .keys()
                    .any(|key| !pending_keys.contains(key))
            {
                return Err(Error::Config("node_migration_unmodelled_legacy_secret: retain the legacy vault and reconcile its credential inventory before migration".into()));
            }
        }
        Ok(keys)
    }

    pub(crate) fn get(&self, field: &SecretField) -> Result<Option<Zeroizing<String>>> {
        self.values.get(field).cloned().ok_or_else(invalid)
    }
}

#[cfg(test)]
mod tests {
    use super::super::config::{CredentialConfig, SshKeyConfig};
    use super::super::secret_fields::CredentialPart;
    use super::*;

    fn fixture() -> (NodeConfig, KeychainBackend) {
        let mut config = NodeConfig::new(
            "wss://example.invalid".into(),
            "node".into(),
            "keychain".into(),
        );
        let mut oauth = CredentialConfig::new_header("Authorization".into(), None, None);
        oauth.oauth_managed = true;
        config.credentials.insert("oauth".into(), oauth);
        config
            .credentials
            .insert("no-auth".into(), CredentialConfig::new_no_auth(None));
        config.ssh_keys.push(SshKeyConfig {
            service_slug: "ssh".into(),
            principal: "alice".into(),
            private_key_pem_encrypted: None,
            passphrase_encrypted: None,
            target_host: "example.invalid".into(),
            target_port: 22,
            host_key_sha256: Some("SHA256:example".into()),
            algorithms: None,
            created_at: "2026-01-01T00:00:00Z".into(),
        });
        let backend = KeychainBackend::new_mock("node");
        (config, backend)
    }
    fn oauth(part: CredentialPart) -> SecretField {
        SecretField::Credential {
            slug: "oauth".into(),
            part,
        }
    }
    fn seed_individual(config: &NodeConfig, backend: &KeychainBackend) {
        for (field, _) in secret_fields::fields(config).unwrap() {
            let key = match field {
                SecretField::AuthToken | SecretField::SigningSecret => field.legacy_key(),
                _ => credential_key(&field.legacy_key()),
            };
            backend.set(&key, "synthetic-secret").unwrap();
        }
    }

    #[test]
    fn individual_read_discovers_absent_optional_markers_without_any_migration() {
        let (config, backend) = fixture();
        seed_individual(&config, &backend);
        let snapshot = LegacySnapshot::read(&config, &backend).unwrap();
        assert!(matches!(snapshot.source, Some(LegacySource::Individual)));
        assert!(
            snapshot
                .get(&oauth(CredentialPart::OAuthRefresh))
                .unwrap()
                .is_some()
        );
        assert!(
            snapshot
                .get(&oauth(CredentialPart::OAuthClientSecret))
                .unwrap()
                .is_some()
        );
        assert!(
            snapshot
                .get(&SecretField::SshPassphrase {
                    service: "ssh".into(),
                    principal: "alice".into()
                })
                .unwrap()
                .is_some()
        );
        assert!(backend.get_optional("vault").unwrap().is_none());
        assert_eq!(backend.get("auth_token").unwrap(), "synthetic-secret");
        assert!(!format!("{snapshot:?}").contains("synthetic-secret"));
    }

    #[test]
    fn present_vault_is_authoritative_and_never_falls_back_on_partial_or_corrupt_data() {
        let (config, backend) = fixture();
        seed_individual(&config, &backend);
        for json in [
            "not json",
            "{}",
            r#"{"auth_token":"vault-auth","future_secrets":{}}"#,
        ] {
            backend.set("vault", json).unwrap();
            assert!(LegacySnapshot::read(&config, &backend).is_err());
            assert_eq!(backend.get("vault").unwrap(), json);
            assert_eq!(backend.get("auth_token").unwrap(), "synthetic-secret");
        }
    }

    #[test]
    fn optional_empty_and_absent_are_preserved_in_both_legacy_formats() {
        let (mut config, backend) = fixture();
        config.ssh_keys.clear();
        backend.set("vault", r#"{"auth_token":"vault-auth","credentials":{"oauth":"header","oauth:oauth_access":"access","oauth:oauth_cid":"client","oauth:oauth_refresh":""}}"#).unwrap();
        let snapshot = LegacySnapshot::read(&config, &backend).unwrap();
        assert!(matches!(snapshot.source, Some(LegacySource::Vault)));
        assert_eq!(
            snapshot
                .get(&oauth(CredentialPart::OAuthRefresh))
                .unwrap()
                .unwrap()
                .as_str(),
            ""
        );
        assert!(
            snapshot
                .get(&oauth(CredentialPart::OAuthClientSecret))
                .unwrap()
                .is_none()
        );
        backend.delete("vault").unwrap();
        seed_individual(&config, &backend);
        backend.set("cred/oauth:oauth_refresh", "").unwrap();
        backend.delete("cred/oauth:oauth_csecret").unwrap();
        let snapshot = LegacySnapshot::read(&config, &backend).unwrap();
        assert_eq!(
            snapshot
                .get(&oauth(CredentialPart::OAuthRefresh))
                .unwrap()
                .unwrap()
                .as_str(),
            ""
        );
        assert!(
            snapshot
                .get(&oauth(CredentialPart::OAuthClientSecret))
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn disappearance_or_service_error_refuses_a_fresh_read_without_using_previous_snapshot() {
        let (config, backend) = fixture();
        seed_individual(&config, &backend);
        let _previous = LegacySnapshot::read(&config, &backend).unwrap();
        backend.delete("auth_token").unwrap();
        assert!(LegacySnapshot::read(&config, &backend).is_err());
        let unavailable = KeychainBackend::new_failing_mock("node", "sensitive-object-name");
        let error = LegacySnapshot::read(&config, &unavailable)
            .unwrap_err()
            .to_string();
        assert!(!error.contains("sensitive-object-name"));
    }

    #[test]
    fn anchored_vault_loss_never_promotes_stale_individual_entries() {
        let (mut config, backend) = fixture();
        config.ssh_keys.clear();
        seed_individual(&config, &backend);
        backend.set("vault", r#"{"auth_token":"vault-auth","credentials":{"oauth":"header","oauth:oauth_access":"access","oauth:oauth_cid":"client"}}"#).unwrap();
        let anchor = LegacySnapshot::read(&config, &backend).unwrap();
        assert!(matches!(anchor.source, Some(LegacySource::Vault)));
        assert!(anchor.reread(&config, &backend).is_ok());
        backend.delete("vault").unwrap();
        assert!(anchor.reread(&config, &backend).is_err());
        assert!(backend.get_optional("vault").unwrap().is_none());
        // First start on a genuine individual-only configuration remains
        // supported; that initial discovery cannot replace the runtime anchor.
        let individual = LegacySnapshot::read(&config, &backend).unwrap();
        assert!(matches!(individual.source, Some(LegacySource::Individual)));
        assert!(individual.reread(&config, &backend).is_ok());
    }

    #[test]
    fn modern_optional_absence_and_namespace_collisions_never_read_legacy_storage() {
        let (mut config, _) = fixture();
        config.credentials.insert(
            "oauth:oauth_access".into(),
            CredentialConfig::new_header("Authorization".into(), None, None),
        );
        assert!(secret_fields::validate_legacy_namespace(&config).is_err());
        // A faulty store proves the all-modern path performs no legacy read,
        // even for optional fields whose old keychain writer returned None.
        let unavailable = KeychainBackend::new_failing_mock("node", "must-not-be-read");
        for (field, required) in secret_fields::fields(&config).unwrap() {
            if required {
                field
                    .replace(
                        &mut config,
                        Some(format!(
                            "nyxid-key:v1:{}:{}",
                            uuid::Uuid::new_v4(),
                            uuid::Uuid::new_v4(),
                        )),
                    )
                    .unwrap();
            }
        }
        config.keychain_format = Some(KeychainFormat::ImmutableV1);
        let modern = LegacySnapshot::read(&config, &unavailable).unwrap();
        assert!(modern.source.is_none());
        assert!(modern.values.is_empty());
        assert!(modern.reread(&config, &unavailable).is_ok());
        assert!(
            config.credentials["oauth"]
                .oauth_refresh_token_encrypted
                .is_none()
        );
        assert!(config.ssh_keys[0].passphrase_encrypted.is_none());
    }

    #[test]
    fn direct_legacy_reader_refuses_mixed_and_all_ref_unmarked_configs() {
        let (mut config, backend) = fixture();
        config.ssh_keys.clear();
        backend.set("vault", r#"{"auth_token":"vault-auth","credentials":{"oauth":"header","oauth:oauth_access":"access","oauth:oauth_cid":"client"}}"#).unwrap();
        config
            .credentials
            .get_mut("oauth")
            .unwrap()
            .header_value_encrypted = Some(format!(
            "nyxid-key:v1:{}:{}",
            uuid::Uuid::new_v4(),
            uuid::Uuid::new_v4(),
        ));
        assert!(LegacySnapshot::read(&config, &backend).is_err());
        for (field, _) in secret_fields::fields(&config).unwrap() {
            field
                .replace(
                    &mut config,
                    Some(format!(
                        "nyxid-key:v1:{}:{}",
                        uuid::Uuid::new_v4(),
                        uuid::Uuid::new_v4(),
                    )),
                )
                .unwrap();
        }
        assert!(LegacySnapshot::read(&config, &backend).is_err());
        config.keychain_format = Some(KeychainFormat::ImmutableV1);
        assert!(
            LegacySnapshot::read(&config, &backend)
                .unwrap()
                .source
                .is_none()
        );
    }

    #[test]
    fn full_legacy_snapshot_publishes_refs_once_preserving_every_optional_presence() {
        use super::super::config::PendingCryptoKeyConfig;
        use super::super::secret_transaction::{Attempt, ObjectStore};
        let (mut config, backend) = fixture();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        config.signing.shared_secret_encrypted = Some(String::new());
        config.credentials.insert(
            "query".into(),
            CredentialConfig::new_query_param("key".into(), None, None),
        );
        config.pending_crypto_keys.insert(
            "pending".into(),
            PendingCryptoKeyConfig {
                version: "v1".into(),
                service_slug: "query".into(),
                injection_method: "query_param".into(),
                field_name: "key".into(),
                target_url: None,
                expires_at: "2026-01-01T00:00:00Z".into(),
                public_key: "synthetic-public".into(),
                private_key_encrypted: None,
            },
        );
        backend.set("vault", r#"{"auth_token":"auth","signing_secret":"signing","credentials":{"oauth":"header","oauth:oauth_access":"access","oauth:oauth_cid":"client","oauth:oauth_refresh":"","query":"query-value","ssh-key/ssh/alice/private_key":"private","ssh-key/ssh/alice/passphrase":""},"pending_crypto_keys":{"pending":"pending-private"}}"#).unwrap();
        let legacy_json = backend.get("vault").unwrap();
        config.save(&path).unwrap();
        let mut config = NodeConfig::load(&path).unwrap();
        let legacy = LegacySnapshot::read(&config, &backend).unwrap();
        let attempt = Attempt::begin_in(
            &super::super::storage::Directory::open(path.parent().unwrap()).unwrap(),
            &path,
            config.publication.snapshot(),
            "node",
        )
        .unwrap();
        let mut present = 0;
        for (field, _) in secret_fields::fields(&config).unwrap() {
            let value = legacy.get(&field).unwrap();
            let encoded = value
                .as_ref()
                .map(|v| attempt.stage(field.clone(), v, &backend))
                .transpose()
                .unwrap();
            if encoded.is_some() {
                present += 1;
            }
            field.replace(&mut config, encoded).unwrap();
        }
        config.keychain_format = Some(KeychainFormat::ImmutableV1);
        config.save(&path).unwrap();
        let loaded = NodeConfig::load(&path).unwrap();
        let refs = super::super::secret_transaction::live_references(&loaded).unwrap();
        assert_eq!(present, 10);
        assert_eq!(refs.len(), present);
        for (reference, field) in refs {
            let actual = ObjectStore::get(&backend, reference).unwrap();
            let expected = legacy.get(&field).unwrap();
            assert_eq!(
                actual.as_deref().map(String::as_str),
                expected.as_deref().map(String::as_str)
            );
        }
        assert!(
            loaded.credentials["oauth"]
                .oauth_client_secret_encrypted
                .is_none()
        );
        assert!(
            loaded.credentials["oauth"]
                .oauth_refresh_token_encrypted
                .is_some()
        );
        assert!(loaded.ssh_keys[0].passphrase_encrypted.is_some());
        // Source cleanup is a separate commit-aware phase; this publication
        // has neither consolidated nor removed authoritative legacy material.
        assert_eq!(backend.get("vault").unwrap(), legacy_json);
        assert!(legacy.reread(&loaded, &backend).is_err());
        assert!(
            LegacySnapshot::read(&loaded, &backend)
                .unwrap()
                .source
                .is_none()
        );
    }
}

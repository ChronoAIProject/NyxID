//! Typed locations for every secret persisted by a node. References and
//! ciphertext are never emitted by Debug. Enumeration performs no OS effect.
use serde::{Deserialize, Serialize};

use super::config::NodeConfig;
use super::error::{Error, Result};

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum CredentialPart {
    Header,
    Parameter,
    OAuthAccess,
    OAuthRefresh,
    OAuthClientId,
    OAuthClientSecret,
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum SecretField {
    AuthToken,
    SigningSecret,
    Credential { slug: String, part: CredentialPart },
    SshPrivate { service: String, principal: String },
    SshPassphrase { service: String, principal: String },
    PendingPrivate { pending_id: String },
}

impl std::fmt::Debug for SecretField {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("SecretField { location: [REDACTED] }")
    }
}

fn missing() -> Error {
    Error::Config("Node secret location no longer exists".into())
}

impl SecretField {
    /// Legacy key only; immutable storage always uses an independently allocated
    /// reference, so logical names cannot overwrite another generation's value.
    pub(crate) fn legacy_key(&self) -> String {
        match self {
            Self::AuthToken => "auth_token".into(),
            Self::SigningSecret => "signing_secret".into(),
            Self::Credential { slug, part } => match part {
                CredentialPart::Header | CredentialPart::Parameter => slug.clone(),
                CredentialPart::OAuthAccess => format!("{slug}:oauth_access"),
                CredentialPart::OAuthRefresh => format!("{slug}:oauth_refresh"),
                CredentialPart::OAuthClientId => format!("{slug}:oauth_cid"),
                CredentialPart::OAuthClientSecret => format!("{slug}:oauth_csecret"),
            },
            Self::SshPrivate { service, principal } => {
                format!("ssh-key/{service}/{principal}/private_key")
            }
            Self::SshPassphrase { service, principal } => {
                format!("ssh-key/{service}/{principal}/passphrase")
            }
            Self::PendingPrivate { pending_id } => pending_id.clone(),
        }
    }

    pub(crate) fn encoded<'a>(&self, config: &'a NodeConfig) -> Result<Option<&'a str>> {
        Ok(match self {
            Self::AuthToken => Some(config.node.auth_token_encrypted.as_str()),
            Self::SigningSecret => config.signing.shared_secret_encrypted.as_deref(),
            Self::Credential { slug, part } => {
                let credential = config.credentials.get(slug).ok_or_else(missing)?;
                match part {
                    CredentialPart::Header => credential.header_value_encrypted.as_deref(),
                    CredentialPart::Parameter => credential.param_value_encrypted.as_deref(),
                    CredentialPart::OAuthAccess => {
                        credential.oauth_access_token_encrypted.as_deref()
                    }
                    CredentialPart::OAuthRefresh => {
                        credential.oauth_refresh_token_encrypted.as_deref()
                    }
                    CredentialPart::OAuthClientId => {
                        credential.oauth_client_id_encrypted.as_deref()
                    }
                    CredentialPart::OAuthClientSecret => {
                        credential.oauth_client_secret_encrypted.as_deref()
                    }
                }
            }
            Self::SshPrivate { service, principal }
            | Self::SshPassphrase { service, principal } => {
                let key = config
                    .ssh_keys
                    .iter()
                    .find(|key| &key.service_slug == service && &key.principal == principal)
                    .ok_or_else(missing)?;
                if matches!(self, Self::SshPrivate { .. }) {
                    key.private_key_pem_encrypted.as_deref()
                } else {
                    key.passphrase_encrypted.as_deref()
                }
            }
            Self::PendingPrivate { pending_id } => config
                .pending_crypto_keys
                .get(pending_id)
                .ok_or_else(missing)?
                .private_key_encrypted
                .as_deref(),
        })
    }

    pub(crate) fn replace(&self, config: &mut NodeConfig, value: Option<String>) -> Result<()> {
        match self {
            Self::AuthToken => config.node.auth_token_encrypted = value.ok_or_else(missing)?,
            Self::SigningSecret => config.signing.shared_secret_encrypted = value,
            Self::Credential { slug, part } => {
                let credential = config.credentials.get_mut(slug).ok_or_else(missing)?;
                let slot = match part {
                    CredentialPart::Header => &mut credential.header_value_encrypted,
                    CredentialPart::Parameter => &mut credential.param_value_encrypted,
                    CredentialPart::OAuthAccess => &mut credential.oauth_access_token_encrypted,
                    CredentialPart::OAuthRefresh => &mut credential.oauth_refresh_token_encrypted,
                    CredentialPart::OAuthClientId => &mut credential.oauth_client_id_encrypted,
                    CredentialPart::OAuthClientSecret => {
                        &mut credential.oauth_client_secret_encrypted
                    }
                };
                *slot = value;
            }
            Self::SshPrivate { service, principal }
            | Self::SshPassphrase { service, principal } => {
                let key = config
                    .ssh_keys
                    .iter_mut()
                    .find(|key| &key.service_slug == service && &key.principal == principal)
                    .ok_or_else(missing)?;
                if matches!(self, Self::SshPrivate { .. }) {
                    key.private_key_pem_encrypted = value;
                } else {
                    key.passphrase_encrypted = value;
                }
            }
            Self::PendingPrivate { pending_id } => {
                config
                    .pending_crypto_keys
                    .get_mut(pending_id)
                    .ok_or_else(missing)?
                    .private_key_encrypted = value
            }
        }
        Ok(())
    }
}

/// Required logical slots can have no encoded value in legacy keychain
/// configurations. Optional slots are included even when absent so the legacy
/// reader can discover values whose old writer lost the presence marker.
/// File readers interpret absent optional slots as absent, without generation.
pub(crate) fn fields(config: &NodeConfig) -> Result<Vec<(SecretField, bool)>> {
    let mut output = vec![(SecretField::AuthToken, true)];
    if config.signing.shared_secret_encrypted.is_some() {
        output.push((SecretField::SigningSecret, true));
    }
    for (slug, credential) in &config.credentials {
        let main = match credential.injection_method.as_str() {
            "header" | "path_prefix" | "aws_sigv4" | "ifttt_webhook" => {
                Some(CredentialPart::Header)
            }
            "query_param" => Some(CredentialPart::Parameter),
            "none"
                if credential.header_value_encrypted.is_none()
                    && credential.param_value_encrypted.is_none()
                    && !credential.oauth_managed
                    && credential.oauth_access_token_encrypted.is_none()
                    && credential.oauth_refresh_token_encrypted.is_none()
                    && credential.oauth_client_id_encrypted.is_none()
                    && credential.oauth_client_secret_encrypted.is_none() =>
            {
                None
            }
            _ => {
                return Err(Error::Config(
                    "Cannot enumerate secrets for an invalid node credential".into(),
                ));
            }
        };
        // Refuse contradictory inactive main slots. Otherwise liveness could
        // silently overlook a reference stored in the unused slot.
        if (matches!(main, Some(CredentialPart::Header))
            && credential.param_value_encrypted.is_some())
            || (matches!(main, Some(CredentialPart::Parameter))
                && credential.header_value_encrypted.is_some())
        {
            return Err(Error::Config(
                "Conflicting node credential secret slots".into(),
            ));
        }
        if let Some(part) = main {
            output.push((
                SecretField::Credential {
                    slug: slug.clone(),
                    part,
                },
                true,
            ));
        }
        for part in [
            CredentialPart::OAuthAccess,
            CredentialPart::OAuthRefresh,
            CredentialPart::OAuthClientId,
            CredentialPart::OAuthClientSecret,
        ] {
            let field = SecretField::Credential {
                slug: slug.clone(),
                part,
            };
            if credential.oauth_managed || field.encoded(config)?.is_some() {
                let required = credential.oauth_managed
                    && matches!(
                        part,
                        CredentialPart::OAuthAccess | CredentialPart::OAuthClientId
                    );
                output.push((field, required));
            }
        }
    }
    for key in &config.ssh_keys {
        output.push((
            SecretField::SshPrivate {
                service: key.service_slug.clone(),
                principal: key.principal.clone(),
            },
            true,
        ));
        output.push((
            SecretField::SshPassphrase {
                service: key.service_slug.clone(),
                principal: key.principal.clone(),
            },
            false,
        ));
    }
    for pending_id in config.pending_crypto_keys.keys() {
        output.push((
            SecretField::PendingPrivate {
                pending_id: pending_id.clone(),
            },
            true,
        ));
    }
    let unique: std::collections::BTreeSet<_> = output.iter().map(|(field, _)| field).collect();
    if unique.len() != output.len() {
        return Err(Error::Config(
            "Duplicate node secret locations are not supported".into(),
        ));
    }
    Ok(output)
}

/// Legacy keychain credentials share a flat namespace. Auth/signing and pending
/// use separate entries/maps; HTTP, OAuth and SSH all share the credential map.
/// Refuse a collision before reading or migrating a value with unclear custody.
pub(crate) fn validate_legacy_namespace(config: &NodeConfig) -> Result<()> {
    let mut keys = std::collections::BTreeSet::new();
    for (field, _) in fields(config)? {
        if matches!(
            field,
            SecretField::Credential { .. }
                | SecretField::SshPrivate { .. }
                | SecretField::SshPassphrase { .. }
        ) && !keys.insert(field.legacy_key())
        {
            return Err(Error::Config(
                "Ambiguous legacy node credential namespace".into(),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::config::{CredentialConfig, PendingCryptoKeyConfig, SshKeyConfig};

    fn config() -> NodeConfig {
        let mut config = NodeConfig::new(
            "wss://example.invalid/node".into(),
            "node".into(),
            "file".into(),
        );
        config.node.auth_token_encrypted = "auth-ciphertext".into();
        config.signing.shared_secret_encrypted = Some("signing-ciphertext".into());
        let mut oauth = CredentialConfig::new_header(
            "Authorization".into(),
            Some("header-ciphertext".into()),
            None,
        );
        oauth.oauth_managed = true;
        oauth.oauth_access_token_encrypted = Some("access-ciphertext".into());
        oauth.oauth_refresh_token_encrypted = Some("refresh-ciphertext".into());
        oauth.oauth_client_id_encrypted = Some("client-id-ciphertext".into());
        oauth.oauth_client_secret_encrypted = Some("client-secret-ciphertext".into());
        config.credentials.insert("oauth".into(), oauth);
        config.credentials.insert(
            "query".into(),
            CredentialConfig::new_query_param("key".into(), Some("query-ciphertext".into()), None),
        );
        config
            .credentials
            .insert("no-auth".into(), CredentialConfig::new_no_auth(None));
        config.ssh_keys.push(SshKeyConfig {
            service_slug: "ssh".into(),
            principal: "user".into(),
            private_key_pem_encrypted: Some("ssh-ciphertext".into()),
            passphrase_encrypted: Some("passphrase-ciphertext".into()),
            target_host: "example.invalid".into(),
            target_port: 22,
            host_key_sha256: None,
            algorithms: None,
            created_at: "2026-01-01T00:00:00Z".into(),
        });
        config.pending_crypto_keys.insert(
            "pending".into(),
            PendingCryptoKeyConfig {
                version: "v1".into(),
                service_slug: "service".into(),
                injection_method: "header".into(),
                field_name: "Authorization".into(),
                target_url: None,
                expires_at: "2026-01-01T00:00:00Z".into(),
                public_key: "public".into(),
                private_key_encrypted: Some("pending-ciphertext".into()),
            },
        );
        config
    }

    #[test]
    fn every_secret_slot_round_trips_without_touching_metadata() {
        let mut config = config();
        let slots = fields(&config).unwrap();
        assert_eq!(slots.len(), 11);
        let expected = slots
            .iter()
            .map(|(field, _)| {
                (
                    field.clone(),
                    field.encoded(&config).unwrap().map(str::to_owned),
                )
            })
            .collect::<Vec<_>>();
        for (field, _) in &slots {
            field
                .replace(&mut config, Some("new-reference".into()))
                .unwrap();
        }
        for (field, _) in &slots {
            assert_eq!(field.encoded(&config).unwrap(), Some("new-reference"));
        }
        for (field, value) in expected {
            field.replace(&mut config, value).unwrap();
        }
        assert_eq!(
            toml::to_string(&config).unwrap(),
            toml::to_string(&self::config()).unwrap()
        );
        validate_legacy_namespace(&config).unwrap();
    }

    #[test]
    fn optional_absence_and_empty_presence_remain_distinct() {
        let mut config = config();
        config
            .credentials
            .get_mut("oauth")
            .unwrap()
            .oauth_refresh_token_encrypted = None;
        config
            .credentials
            .get_mut("oauth")
            .unwrap()
            .oauth_client_secret_encrypted = Some(String::new());
        config.ssh_keys[0].passphrase_encrypted = None;
        let slots = fields(&config).unwrap();
        let absent = slots
            .iter()
            .filter(|(field, required)| !required && field.encoded(&config).unwrap().is_none())
            .count();
        assert_eq!(absent, 2);
        let field = SecretField::Credential {
            slug: "oauth".into(),
            part: CredentialPart::OAuthClientSecret,
        };
        assert_eq!(field.encoded(&config).unwrap(), Some(""));
        field.replace(&mut config, None).unwrap();
        assert_eq!(field.encoded(&config).unwrap(), None);
    }

    #[test]
    fn no_auth_has_no_secret_and_invalid_or_conflicting_slots_refuse() {
        let mut config =
            NodeConfig::new("wss://example.invalid".into(), "node".into(), "file".into());
        config
            .credentials
            .insert("none".into(), CredentialConfig::new_no_auth(None));
        assert_eq!(fields(&config).unwrap().len(), 1);
        config
            .credentials
            .get_mut("none")
            .unwrap()
            .oauth_client_secret_encrypted = Some("orphan".into());
        assert!(fields(&config).is_err());
        config.credentials.insert(
            "none".into(),
            CredentialConfig::new_header("Authorization".into(), None, None),
        );
        config
            .credentials
            .get_mut("none")
            .unwrap()
            .param_value_encrypted = Some("inactive".into());
        assert!(fields(&config).is_err());
        config
            .credentials
            .get_mut("none")
            .unwrap()
            .param_value_encrypted = None;
        config.credentials.get_mut("none").unwrap().injection_method = "unknown".into();
        assert!(fields(&config).is_err());
    }

    #[test]
    fn duplicate_selectors_and_ambiguous_legacy_names_refuse() {
        let mut config = config();
        config.ssh_keys.push(config.ssh_keys[0].clone());
        assert!(fields(&config).is_err());
        config.ssh_keys.pop();
        config.credentials.insert(
            "oauth:oauth_access".into(),
            CredentialConfig::new_header("X-Key".into(), None, None),
        );
        assert!(fields(&config).is_ok());
        assert!(validate_legacy_namespace(&config).is_err());
    }

    #[test]
    fn locations_redact_debug_and_missing_targets_refuse() {
        let mut config = config();
        let field = SecretField::Credential {
            slug: "private-selector".into(),
            part: CredentialPart::OAuthAccess,
        };
        assert!(!format!("{field:?}").contains("private-selector"));
        assert!(field.encoded(&config).is_err());
        assert!(field.replace(&mut config, Some("value".into())).is_err());
    }
}

use std::collections::HashMap;
use std::sync::Arc;

use zeroize::Zeroizing;

use crate::config::NodeConfig;
#[cfg(test)]
use crate::encryption::LocalEncryption;
use crate::error::{Error, Result};
use crate::secret_backend::SecretBackend;

/// Type-safe credential injection, avoiding empty-string placeholders.
#[derive(Clone)]
pub enum CredentialInjection {
    Header {
        name: String,
        value: Zeroizing<String>,
    },
    QueryParam {
        name: String,
        value: Zeroizing<String>,
    },
}

/// A single service's decrypted credential.
#[derive(Clone)]
pub struct ServiceCredential {
    pub injection: CredentialInjection,
}

impl ServiceCredential {
    /// The injection method as a string ("header" or "query_param").
    pub fn injection_method(&self) -> &str {
        match &self.injection {
            CredentialInjection::Header { .. } => "header",
            CredentialInjection::QueryParam { .. } => "query_param",
        }
    }

    /// Display name of what is being injected (for `credentials list`).
    pub fn target_name(&self) -> &str {
        match &self.injection {
            CredentialInjection::Header { name, .. } => name,
            CredentialInjection::QueryParam { name, .. } => name,
        }
    }

    /// For header injection: returns (header_name, header_value).
    pub fn header(&self) -> Option<(&str, &str)> {
        match &self.injection {
            CredentialInjection::Header { name, value } => Some((name, value)),
            _ => None,
        }
    }

    /// For query_param injection: returns (param_name, param_value).
    pub fn query_param(&self) -> Option<(&str, &str)> {
        match &self.injection {
            CredentialInjection::QueryParam { name, value } => Some((name, value)),
            _ => None,
        }
    }
}

/// In-memory credential store loaded from the config file.
/// All values are decrypted at load time and held in memory.
#[derive(Clone)]
pub struct CredentialStore {
    credentials: Arc<HashMap<String, ServiceCredential>>,
}

impl CredentialStore {
    /// Load credentials from config, decrypting each encrypted value (file backend only).
    #[cfg(test)]
    pub fn from_config(config: &NodeConfig, enc: &LocalEncryption) -> Result<Self> {
        let mut map = HashMap::new();

        for (slug, cred_config) in &config.credentials {
            match cred_config.injection_method.as_str() {
                "header" => {
                    let header_name = cred_config
                        .header_name
                        .as_deref()
                        .unwrap_or("Authorization");
                    let encrypted = cred_config.header_value_encrypted.as_deref().ok_or_else(|| {
                        Error::Config(format!(
                            "Credential '{slug}' has header injection but no header_value_encrypted"
                        ))
                    })?;
                    let header_value = enc.decrypt(encrypted)?;

                    map.insert(
                        slug.clone(),
                        ServiceCredential {
                            injection: CredentialInjection::Header {
                                name: header_name.to_string(),
                                value: Zeroizing::new(header_value),
                            },
                        },
                    );
                }
                "query_param" => {
                    let param_name = cred_config.param_name.as_deref().ok_or_else(|| {
                        Error::Config(format!(
                            "Credential '{slug}' has query_param injection but no param_name"
                        ))
                    })?;
                    let encrypted = cred_config.param_value_encrypted.as_deref().ok_or_else(|| {
                        Error::Config(format!(
                            "Credential '{slug}' has query_param injection but no param_value_encrypted"
                        ))
                    })?;
                    let param_value = enc.decrypt(encrypted)?;

                    map.insert(
                        slug.clone(),
                        ServiceCredential {
                            injection: CredentialInjection::QueryParam {
                                name: param_name.to_string(),
                                value: Zeroizing::new(param_value),
                            },
                        },
                    );
                }
                other => {
                    return Err(Error::Config(format!(
                        "Unknown injection method '{other}' for credential '{slug}'"
                    )));
                }
            }
        }

        Ok(Self {
            credentials: Arc::new(map),
        })
    }

    /// Load credentials using the unified secret backend.
    pub fn from_config_with_backend(config: &NodeConfig, backend: &SecretBackend) -> Result<Self> {
        let mut map = HashMap::new();

        for (slug, cred_config) in &config.credentials {
            match cred_config.injection_method.as_str() {
                "header" => {
                    let header_name = cred_config
                        .header_name
                        .as_deref()
                        .unwrap_or("Authorization");
                    let value = backend.load_credential_value(
                        slug,
                        cred_config.header_value_encrypted.as_deref(),
                    )?;
                    map.insert(
                        slug.clone(),
                        ServiceCredential {
                            injection: CredentialInjection::Header {
                                name: header_name.to_string(),
                                value: Zeroizing::new(value),
                            },
                        },
                    );
                }
                "query_param" => {
                    let param_name = cred_config.param_name.as_deref().ok_or_else(|| {
                        Error::Config(format!(
                            "Credential '{slug}' has query_param injection but no param_name"
                        ))
                    })?;
                    let value = backend.load_credential_value(
                        slug,
                        cred_config.param_value_encrypted.as_deref(),
                    )?;
                    map.insert(
                        slug.clone(),
                        ServiceCredential {
                            injection: CredentialInjection::QueryParam {
                                name: param_name.to_string(),
                                value: Zeroizing::new(value),
                            },
                        },
                    );
                }
                other => {
                    return Err(Error::Config(format!(
                        "Unknown injection method '{other}' for credential '{slug}'"
                    )));
                }
            }
        }

        Ok(Self {
            credentials: Arc::new(map),
        })
    }

    /// Get credential for a service slug.
    pub fn get(&self, service_slug: &str) -> Option<&ServiceCredential> {
        self.credentials.get(service_slug)
    }

    /// Number of configured credentials.
    pub fn count(&self) -> usize {
        self.credentials.len()
    }

    /// Sorted list of service slugs.
    pub fn service_slugs(&self) -> Vec<String> {
        let mut slugs: Vec<String> = self.credentials.keys().cloned().collect();
        slugs.sort();
        slugs
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::NodeConfig;

    #[test]
    fn load_from_config() {
        let dir = tempfile::tempdir().unwrap();
        let enc = LocalEncryption::load_or_generate(dir.path()).unwrap();

        let mut config =
            NodeConfig::new("wss://x".to_string(), "n".to_string(), "file".to_string());
        config.set_auth_token("tok", &enc).unwrap();
        config
            .add_header_credential("openai", "Authorization", "Bearer sk-test", &enc)
            .unwrap();
        config
            .add_query_param_credential("stripe", "api_key", "sk_live_123", &enc)
            .unwrap();

        let store = CredentialStore::from_config(&config, &enc).unwrap();
        assert_eq!(store.count(), 2);

        let openai = store.get("openai").unwrap();
        assert_eq!(openai.injection_method(), "header");
        let (hdr_name, hdr_value) = openai.header().unwrap();
        assert_eq!(hdr_name, "Authorization");
        assert_eq!(hdr_value, "Bearer sk-test");

        let stripe = store.get("stripe").unwrap();
        assert_eq!(stripe.injection_method(), "query_param");
        let (param_name, param_value) = stripe.query_param().unwrap();
        assert_eq!(param_name, "api_key");
        assert_eq!(param_value, "sk_live_123");
    }

    #[test]
    fn empty_store() {
        let dir = tempfile::tempdir().unwrap();
        let enc = LocalEncryption::load_or_generate(dir.path()).unwrap();

        let mut config =
            NodeConfig::new("wss://x".to_string(), "n".to_string(), "file".to_string());
        config.set_auth_token("tok", &enc).unwrap();

        let store = CredentialStore::from_config(&config, &enc).unwrap();
        assert_eq!(store.count(), 0);
        assert!(store.get("nonexistent").is_none());
    }
}

use std::collections::HashMap;
use std::sync::Arc;

use zeroize::Zeroizing;

use super::config::NodeConfig;
#[cfg(test)]
use super::encryption::LocalEncryption;
use super::error::{Error, Result};
use super::secret_backend::SecretBackend;

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
    /// Path prefix injection: prepend `/{prefix}{credential}` to the URL path.
    /// Used by Telegram Bot API (`/bot<token>/sendMessage`).
    PathPrefix {
        prefix: String,
        credential: Zeroizing<String>,
    },
    /// No credential injection. Entry exists so the node can resolve
    /// `target_url` and accept proxy requests for this slug, but no
    /// header/query-param/path-prefix injection is performed. Used by
    /// server-held services that have been downgraded to
    /// `auth_method: "none"` — removing the entry outright would make
    /// `proxy_executor` 502 with "No credentials configured" before it
    /// even looks at `base_url`, breaking the downstream call
    /// (thirty-third-round Codex P1).
    NoAuth,
    /// AWS SigV4 — the encrypted blob is the JSON credential consumed by
    /// `nyxid_cloud_auth::aws_sigv4`: `{access_key_id, secret_access_key,
    /// region, service}`. The node agent signs the outgoing request
    /// locally so the access key never crosses NyxID. NyxID#716.
    AwsSigv4 { credential_json: Zeroizing<String> },
    /// GCP service-account JWT-bearer OAuth — the encrypted blob is the
    /// raw service-account JSON key file content. The node mints + caches
    /// access tokens locally; the SA key never crosses NyxID. NyxID#716.
    GcpServiceAccount { credential_json: Zeroizing<String> },
}

/// A single service's decrypted credential.
#[derive(Clone)]
pub struct ServiceCredential {
    pub injection: CredentialInjection,
    /// Local target URL for this service (used when NyxID sends empty base_url).
    pub target_url: Option<String>,
}

impl ServiceCredential {
    /// The injection method as a string ("header", "query_param",
    /// "path_prefix", "aws_sigv4", "gcp_service_account", or "none").
    pub fn injection_method(&self) -> &str {
        match &self.injection {
            CredentialInjection::Header { .. } => "header",
            CredentialInjection::QueryParam { .. } => "query_param",
            CredentialInjection::PathPrefix { .. } => "path_prefix",
            CredentialInjection::AwsSigv4 { .. } => "aws_sigv4",
            CredentialInjection::GcpServiceAccount { .. } => "gcp_service_account",
            CredentialInjection::NoAuth => "none",
        }
    }

    /// Display name of what is being injected (for `credentials list`).
    pub fn target_name(&self) -> &str {
        match &self.injection {
            CredentialInjection::Header { name, .. } => name,
            CredentialInjection::QueryParam { name, .. } => name,
            CredentialInjection::PathPrefix { prefix, .. } => prefix,
            CredentialInjection::AwsSigv4 { .. } => "(AWS SigV4)",
            CredentialInjection::GcpServiceAccount { .. } => "(GCP SA token)",
            CredentialInjection::NoAuth => "(no auth)",
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

    /// For path_prefix injection: returns (prefix, credential).
    pub fn path_prefix(&self) -> Option<(&str, &str)> {
        match &self.injection {
            CredentialInjection::PathPrefix {
                prefix, credential, ..
            } => Some((prefix, credential)),
            _ => None,
        }
    }

    /// Raw credential value for WebSocket auth-frame injection.
    pub fn raw_credential(&self) -> Option<&str> {
        match &self.injection {
            CredentialInjection::Header { value, .. } => Some(value),
            CredentialInjection::QueryParam { value, .. } => Some(value),
            CredentialInjection::PathPrefix { credential, .. } => Some(credential),
            // SigV4 / GCP SA credentials are structured JSON blobs, not
            // raw token strings — surfacing them via WS frame templates
            // would leak the underlying secret material verbatim. Return
            // `None` so the WS frame injector skips substitution for
            // these auth methods.
            CredentialInjection::AwsSigv4 { .. } => None,
            CredentialInjection::GcpServiceAccount { .. } => None,
            CredentialInjection::NoAuth => None,
        }
    }

    /// For AWS SigV4 injection: returns the JSON credential blob.
    pub fn aws_sigv4_credential(&self) -> Option<&str> {
        match &self.injection {
            CredentialInjection::AwsSigv4 { credential_json } => Some(credential_json),
            _ => None,
        }
    }

    /// For GCP service-account injection: returns the raw SA JSON blob.
    pub fn gcp_service_account_credential(&self) -> Option<&str> {
        match &self.injection {
            CredentialInjection::GcpServiceAccount { credential_json } => Some(credential_json),
            _ => None,
        }
    }

    /// Local target URL for this service.
    pub fn target_url(&self) -> Option<&str> {
        self.target_url.as_deref()
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
                            target_url: cred_config.target_url.clone(),
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
                            target_url: cred_config.target_url.clone(),
                        },
                    );
                }
                "path_prefix" => {
                    let prefix = cred_config.header_name.as_deref().unwrap_or("bot");
                    let encrypted = cred_config.header_value_encrypted.as_deref().ok_or_else(|| {
                        Error::Config(format!(
                            "Credential '{slug}' has path_prefix injection but no header_value_encrypted"
                        ))
                    })?;
                    let credential = enc.decrypt(encrypted)?;

                    map.insert(
                        slug.clone(),
                        ServiceCredential {
                            injection: CredentialInjection::PathPrefix {
                                prefix: prefix.to_string(),
                                credential: Zeroizing::new(credential),
                            },
                            target_url: cred_config.target_url.clone(),
                        },
                    );
                }
                "none" => {
                    // No-auth placeholder: entry exists for target_url
                    // resolution but no credential gets injected.
                    map.insert(
                        slug.clone(),
                        ServiceCredential {
                            injection: CredentialInjection::NoAuth,
                            target_url: cred_config.target_url.clone(),
                        },
                    );
                }
                "aws_sigv4" => {
                    let encrypted = cred_config
                        .header_value_encrypted
                        .as_deref()
                        .ok_or_else(|| {
                            Error::Config(format!(
                                "Credential '{slug}' has aws_sigv4 injection but no header_value_encrypted"
                            ))
                        })?;
                    let json = enc.decrypt(encrypted)?;
                    map.insert(
                        slug.clone(),
                        ServiceCredential {
                            injection: CredentialInjection::AwsSigv4 {
                                credential_json: Zeroizing::new(json),
                            },
                            target_url: cred_config.target_url.clone(),
                        },
                    );
                }
                "gcp_service_account" => {
                    let encrypted = cred_config
                        .header_value_encrypted
                        .as_deref()
                        .ok_or_else(|| {
                            Error::Config(format!(
                                "Credential '{slug}' has gcp_service_account injection but no header_value_encrypted"
                            ))
                        })?;
                    let json = enc.decrypt(encrypted)?;
                    map.insert(
                        slug.clone(),
                        ServiceCredential {
                            injection: CredentialInjection::GcpServiceAccount {
                                credential_json: Zeroizing::new(json),
                            },
                            target_url: cred_config.target_url.clone(),
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
                            target_url: cred_config.target_url.clone(),
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
                            target_url: cred_config.target_url.clone(),
                        },
                    );
                }
                "path_prefix" => {
                    let prefix = cred_config.header_name.as_deref().unwrap_or("bot");
                    let value = backend.load_credential_value(
                        slug,
                        cred_config.header_value_encrypted.as_deref(),
                    )?;
                    map.insert(
                        slug.clone(),
                        ServiceCredential {
                            injection: CredentialInjection::PathPrefix {
                                prefix: prefix.to_string(),
                                credential: Zeroizing::new(value),
                            },
                            target_url: cred_config.target_url.clone(),
                        },
                    );
                }
                "none" => {
                    map.insert(
                        slug.clone(),
                        ServiceCredential {
                            injection: CredentialInjection::NoAuth,
                            target_url: cred_config.target_url.clone(),
                        },
                    );
                }
                "aws_sigv4" => {
                    let json = backend.load_credential_value(
                        slug,
                        cred_config.header_value_encrypted.as_deref(),
                    )?;
                    map.insert(
                        slug.clone(),
                        ServiceCredential {
                            injection: CredentialInjection::AwsSigv4 {
                                credential_json: Zeroizing::new(json),
                            },
                            target_url: cred_config.target_url.clone(),
                        },
                    );
                }
                "gcp_service_account" => {
                    let json = backend.load_credential_value(
                        slug,
                        cred_config.header_value_encrypted.as_deref(),
                    )?;
                    map.insert(
                        slug.clone(),
                        ServiceCredential {
                            injection: CredentialInjection::GcpServiceAccount {
                                credential_json: Zeroizing::new(json),
                            },
                            target_url: cred_config.target_url.clone(),
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

/// Thread-safe, hot-reloadable credential handle.
///
/// The sender half is held by the background reload task.
/// The receiver half is cloned into each connection and proxy handler.
/// `snapshot()` returns the current `CredentialStore` via a cheap `Arc` clone.
#[derive(Clone)]
pub struct SharedCredentials {
    rx: tokio::sync::watch::Receiver<CredentialStore>,
}

pub struct SharedCredentialsSender {
    tx: tokio::sync::watch::Sender<CredentialStore>,
}

impl SharedCredentials {
    pub fn new(initial: CredentialStore) -> (SharedCredentialsSender, Self) {
        let (tx, rx) = tokio::sync::watch::channel(initial);
        (SharedCredentialsSender { tx }, Self { rx })
    }

    /// Get a snapshot of the current credentials (cheap Arc clone).
    pub fn snapshot(&self) -> CredentialStore {
        self.rx.borrow().clone()
    }
}

impl SharedCredentialsSender {
    /// Atomically replace the credential store.
    pub fn update(&self, new_store: CredentialStore) {
        let _ = self.tx.send(new_store);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::node::config::NodeConfig;

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

    /// AWS SigV4 + GCP service-account credentials round-trip through
    /// config → encryption → load → typed lookup. The blob is opaque to
    /// the store; the proxy_executor is responsible for JSON-parsing it
    /// at request time.
    #[test]
    fn load_cloud_billing_credentials() {
        use crate::node::config::CredentialConfig;
        let dir = tempfile::tempdir().unwrap();
        let enc = LocalEncryption::load_or_generate(dir.path()).unwrap();

        let mut config =
            NodeConfig::new("wss://x".to_string(), "n".to_string(), "file".to_string());
        config.set_auth_token("tok", &enc).unwrap();

        let aws_blob = r#"{"access_key_id":"AKIA","secret_access_key":"sec","region":"us-east-1","service":"ce"}"#;
        let aws_enc = enc.encrypt(aws_blob).unwrap();
        config.credentials.insert(
            "aws-cost-explorer".to_string(),
            CredentialConfig::new_aws_sigv4(
                Some(aws_enc),
                Some("https://ce.us-east-1.amazonaws.com".into()),
            ),
        );

        let gcp_blob = r#"{"type":"service_account","client_email":"sa@p.iam.gserviceaccount.com","private_key":"x","project_id":"p","private_key_id":"k"}"#;
        let gcp_enc = enc.encrypt(gcp_blob).unwrap();
        config.credentials.insert(
            "gcp-cloud-billing".to_string(),
            CredentialConfig::new_gcp_service_account(
                Some(gcp_enc),
                Some("https://cloudbilling.googleapis.com".into()),
            ),
        );

        let store = CredentialStore::from_config(&config, &enc).unwrap();
        assert_eq!(store.count(), 2);

        let aws = store.get("aws-cost-explorer").unwrap();
        assert_eq!(aws.injection_method(), "aws_sigv4");
        assert_eq!(aws.aws_sigv4_credential().unwrap(), aws_blob);
        // SigV4 credentials must not be exposed via the WS-frame helper.
        assert!(aws.raw_credential().is_none());

        let gcp = store.get("gcp-cloud-billing").unwrap();
        assert_eq!(gcp.injection_method(), "gcp_service_account");
        assert_eq!(gcp.gcp_service_account_credential().unwrap(), gcp_blob);
        assert!(gcp.raw_credential().is_none());
    }
}

//! GCP service-account auth.
//!
//! Wraps the upstream `gcp_auth` crate so the rest of NyxID consumes a
//! stable, narrow API: validate the credential JSON the user stored,
//! return a cached access token for the requested scope, invalidate
//! the cache when an upstream returns 401/403.
//!
//! Why upstream and not hand-rolled:
//!
//! - `gcp_auth` builds + signs the JWT-bearer assertion against
//!   Google's hardcoded canonical token endpoint, refusing any
//!   user-supplied `token_uri`. That's exactly the SSRF guard we had
//!   to implement manually (Codex review BLOCKER 7).
//! - `gcp_auth::CustomServiceAccount` handles single-flight token
//!   minting, expiry tracking, and refresh-before-expiry internally,
//!   so we drop ~250 LOC of `tokio::sync::Mutex` + `DashMap` plumbing.
//! - The crate picks up `GOOGLE_APPLICATION_CREDENTIALS` and the GCE
//!   metadata server for free, which is useful for node operators
//!   running on a VM with workload identity even though we don't
//!   wire that path through today.

use std::sync::Arc;

use dashmap::DashMap;
use gcp_auth::{CustomServiceAccount, TokenProvider};
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::error::{CloudAuthError, CloudAuthResult};

/// Default OAuth scope for proxying GCP Cloud Billing + BigQuery requests.
///
/// Cloud Billing API accepts the broad `cloud-platform` scope; BigQuery
/// requires the more specific `bigquery` scope. We request both so a
/// single cached token can serve either downstream — the alternative
/// is two separate cache entries per credential, and these requests
/// share the same service account anyway.
pub const DEFAULT_GCP_SCOPES: &[&str] = &[
    "https://www.googleapis.com/auth/cloud-platform",
    "https://www.googleapis.com/auth/bigquery",
];

/// Decoded GCP service-account credential.
///
/// We keep our own shallow parser (rather than letting `gcp_auth`
/// handle it end-to-end) so the proxy can reject malformed credentials
/// early — at the API boundary where we can produce a useful error
/// — instead of inside the lazily-initialized token provider.
#[derive(Debug, Clone, Deserialize)]
pub struct GcpServiceAccountKey {
    #[serde(rename = "type")]
    pub key_type: String,
    pub project_id: String,
    pub private_key_id: String,
    pub private_key: String,
    pub client_email: String,
    #[serde(default)]
    pub token_uri: Option<String>,
}

impl GcpServiceAccountKey {
    pub fn from_json(raw: &str) -> CloudAuthResult<Self> {
        let trimmed = raw.trim();
        let key: Self = serde_json::from_str(trimmed).map_err(|e| {
            CloudAuthError::InvalidCredential(format!(
                "gcp_service_account credential must be the raw service-account JSON: {}",
                e
            ))
        })?;
        if key.key_type != "service_account" {
            return Err(CloudAuthError::InvalidCredential(format!(
                "expected type=service_account, got '{}'",
                key.key_type
            )));
        }
        if key.client_email.is_empty() || key.private_key.is_empty() {
            return Err(CloudAuthError::InvalidCredential(
                "service account JSON missing client_email or private_key".to_string(),
            ));
        }
        Ok(key)
    }
}

/// Thread-safe access-token cache.
///
/// Holds one [`CustomServiceAccount`] per credential JSON; `gcp_auth`
/// itself caches the minted token under each provider and refreshes
/// before expiry, so we only need to keep providers around long
/// enough to amortize the JSON-parse + private-key load cost.
///
/// Cache key = `sha256(credential_json)` so identical credentials
/// (e.g. the same SA used by two services) share one provider, and
/// rotating the credential produces a new entry instead of stomping
/// the old one mid-request.
#[derive(Clone, Default)]
pub struct GcpTokenCache {
    providers: Arc<DashMap<String, Arc<CustomServiceAccount>>>,
}

impl GcpTokenCache {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns a non-expired access token for `(credential_json, scopes)`.
    ///
    /// Looks up the provider for `credential_json`, builds it on miss,
    /// then asks `gcp_auth` for a token covering `scopes`. The crate
    /// handles JWT signing, the token-endpoint POST, expiry tracking,
    /// and refresh-before-stale internally — we just hand the bytes
    /// to `bearer_auth`.
    ///
    /// `client` is accepted for API compatibility with the previous
    /// hand-rolled implementation; `gcp_auth` uses its own reqwest /
    /// hyper client internally.
    pub async fn access_token(
        &self,
        _client: &reqwest::Client,
        credential_json: &str,
        scopes: &[&str],
    ) -> CloudAuthResult<Arc<str>> {
        let provider = self.get_or_init_provider(credential_json)?;
        let token = provider
            .token(scopes)
            .await
            .map_err(|e| CloudAuthError::TokenMint(format!("gcp_auth: {e}")))?;
        Ok(Arc::from(token.as_str()))
    }

    /// Drops any cached provider for the given credential, forcing the
    /// next request to re-parse the JSON and re-mint a token. Wire
    /// this into the 401/403 path so a revoked SA key doesn't keep
    /// serving stale tokens until natural expiry (Codex review REC 8).
    pub fn invalidate(&self, credential_json: &str, _scopes: &[&str]) {
        let key = cache_key(credential_json);
        self.providers.remove(&key);
    }

    fn get_or_init_provider(
        &self,
        credential_json: &str,
    ) -> CloudAuthResult<Arc<CustomServiceAccount>> {
        let key = cache_key(credential_json);
        if let Some(existing) = self.providers.get(&key) {
            return Ok(existing.clone());
        }
        // Validate our own JSON shape first so a malformed credential
        // surfaces a useful error here instead of as a generic
        // gcp_auth parse failure.
        let _ = GcpServiceAccountKey::from_json(credential_json)?;
        // `CustomServiceAccount::from_json` ignores `token_uri` from
        // the input; it always POSTs to `oauth2.googleapis.com/token`.
        // That's the SSRF guard from BLOCKER 7 — get it free now.
        let provider = CustomServiceAccount::from_json(credential_json)
            .map_err(|e| CloudAuthError::InvalidCredential(format!("gcp_auth load: {e}")))?;
        let arc = Arc::new(provider);
        self.providers.insert(key, arc.clone());
        Ok(arc)
    }
}

fn cache_key(credential_json: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(credential_json.as_bytes());
    hex::encode(hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE_SA: &str = r#"{
        "type": "service_account",
        "project_id": "test-project",
        "private_key_id": "abc123",
        "private_key": "-----BEGIN RSA PRIVATE KEY-----\nMIIEpAIBAAKCAQEAxx==\n-----END RSA PRIVATE KEY-----\n",
        "client_email": "sa@test-project.iam.gserviceaccount.com",
        "client_id": "112233"
    }"#;

    #[test]
    fn parses_service_account_json() {
        let key = GcpServiceAccountKey::from_json(SAMPLE_SA).expect("parse");
        assert_eq!(key.client_email, "sa@test-project.iam.gserviceaccount.com");
        assert_eq!(key.project_id, "test-project");
    }

    #[test]
    fn rejects_non_service_account_type() {
        let raw = SAMPLE_SA.replace("service_account", "user_account");
        let err = GcpServiceAccountKey::from_json(&raw).unwrap_err();
        assert!(matches!(err, CloudAuthError::InvalidCredential(_)));
    }

    #[test]
    fn rejects_missing_private_key() {
        let raw = r#"{"type":"service_account","project_id":"p","private_key_id":"k","private_key":"","client_email":"sa@p.iam.gserviceaccount.com"}"#;
        let err = GcpServiceAccountKey::from_json(raw).unwrap_err();
        assert!(matches!(err, CloudAuthError::InvalidCredential(_)));
    }

    #[test]
    fn cache_key_is_deterministic() {
        assert_eq!(cache_key("creds"), cache_key("creds"));
        assert_ne!(cache_key("creds-a"), cache_key("creds-b"));
    }
}

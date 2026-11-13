//! Credential material never leaves this module except as an in-memory bearer.
use crate::errors::{AppError, AppResult};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::{Mutex, Semaphore};
use zeroize::Zeroizing;

const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";
const SCOPE: &str = "https://www.googleapis.com/auth/cloud-platform";
const MAX_ENTRIES: usize = 128;

fn unavailable() -> AppError {
    AppError::ServicePoolInfrastructureUnavailable
}

/// RFC 6750 token syntax also prevents accidentally forwarding a structured
/// service-account credential (and its private key) as a direct provider key.
pub(super) fn validate_bearer(value: &str) -> AppResult<()> {
    let token = value.trim_end_matches('=');
    if value.len() > 8192
        || token.is_empty()
        || !token
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-._~+/".contains(&b))
    {
        return Err(unavailable());
    }
    Ok(())
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ServiceAccount {
    #[serde(rename = "type")]
    kind: String,
    project_id: String,
    private_key_id: String,
    private_key: Zeroizing<String>,
    client_email: String,
    client_id: String,
    auth_uri: Option<String>,
    token_uri: Option<String>,
    auth_provider_x509_cert_url: Option<String>,
    client_x509_cert_url: Option<String>,
    universe_domain: Option<String>,
}
impl ServiceAccount {
    fn parse(raw: &str) -> AppResult<Self> {
        let key: Self = serde_json::from_str(raw).map_err(|_| unavailable())?;
        if key.kind != "service_account"
            || key.project_id.is_empty()
            || key.client_id.is_empty()
            || key.private_key_id.is_empty()
            || key.private_key_id.len() > 256
            || !key.client_email.ends_with(".iam.gserviceaccount.com")
            || key.client_email.len() > 320
            || key.token_uri.as_deref().is_some_and(|v| v != TOKEN_URL)
            || key
                .auth_uri
                .as_deref()
                .is_some_and(|v| v != "https://accounts.google.com/o/oauth2/auth")
            || key
                .auth_provider_x509_cert_url
                .as_deref()
                .is_some_and(|v| v != "https://www.googleapis.com/oauth2/v1/certs")
            || key
                .universe_domain
                .as_deref()
                .is_some_and(|v| v != "googleapis.com")
            || key.client_x509_cert_url.as_deref().is_some_and(|v| {
                !v.starts_with("https://www.googleapis.com/robot/v1/metadata/x509/")
            })
        {
            return Err(unavailable());
        }
        // Optional certificate URLs are validated metadata, never fetched.
        jsonwebtoken::EncodingKey::from_rsa_pem(key.private_key.as_bytes())
            .map_err(|_| unavailable())?;
        Ok(key)
    }
}

/// Save-time validation shares the executor's closed parser, without minting a token.
/// Errors contain no supplied data, including parser diagnostics or key fragments.
pub(crate) fn validate_saved_credential(raw: &str, service_account: bool) -> AppResult<()> {
    let result = if service_account {
        ServiceAccount::parse(raw).map(|_| ())
    } else {
        validate_bearer(raw)
    };
    result.map_err(|_| AppError::ValidationError(if service_account {
        "Invalid Google Cloud service-account JSON; use a service_account key with the standard Google endpoints and a valid RSA private key"
    } else {
        "Invalid provider API key"
    }.into()))
}

struct Cached {
    token: Zeroizing<String>,
    refresh_at: Instant,
}
#[derive(Default)]
struct Entry {
    cached: Option<Cached>,
    failed_until: Option<Instant>,
}

#[async_trait]
pub(super) trait TokenProvider: Send + Sync {
    async fn bearer(&self, identity: &str, credential: &str) -> AppResult<Zeroizing<String>>;
}

pub(super) struct ServiceAccountTokens {
    client: reqwest::Client,
    endpoint: String,
    entries: Mutex<HashMap<String, Arc<Mutex<Entry>>>>,
    requests: Semaphore,
}
impl ServiceAccountTokens {
    pub fn new(client: reqwest::Client) -> Self {
        Self {
            client,
            endpoint: TOKEN_URL.into(),
            entries: Mutex::new(HashMap::new()),
            requests: Semaphore::new(8),
        }
    }
    #[cfg(test)]
    pub fn with_endpoint(client: reqwest::Client, endpoint: String) -> Self {
        Self {
            endpoint,
            ..Self::new(client)
        }
    }
    async fn mint(&self, credential: &str) -> AppResult<Cached> {
        let key = ServiceAccount::parse(credential)?;
        #[derive(Serialize)]
        struct Claims<'a> {
            iss: &'a str,
            scope: &'a str,
            aud: &'a str,
            iat: i64,
            exp: i64,
        }
        let now = chrono::Utc::now().timestamp();
        let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256);
        header.kid = Some(key.private_key_id.clone());
        let assertion = Zeroizing::new(
            jsonwebtoken::encode(
                &header,
                &Claims {
                    iss: &key.client_email,
                    scope: SCOPE,
                    aud: TOKEN_URL,
                    iat: now,
                    exp: now + 3600,
                },
                &jsonwebtoken::EncodingKey::from_rsa_pem(key.private_key.as_bytes())
                    .map_err(|_| unavailable())?,
            )
            .map_err(|_| unavailable())?,
        );
        let _permit = self.requests.acquire().await.map_err(|_| unavailable())?;
        let started = Instant::now();
        let response = self
            .client
            .post(&self.endpoint)
            .timeout(Duration::from_secs(10))
            .form(&[
                ("grant_type", "urn:ietf:params:oauth:grant-type:jwt-bearer"),
                ("assertion", assertion.as_str()),
            ])
            .send()
            .await
            .map_err(|_| unavailable())?;
        if !response.status().is_success() {
            return Err(unavailable());
        }
        let bytes = super::bounded_body(response, 16_384).await?;
        #[derive(Deserialize)]
        struct Token {
            access_token: Zeroizing<String>,
            token_type: String,
            expires_in: u64,
        }
        let token: Token = serde_json::from_slice(&bytes).map_err(|_| unavailable())?;
        if token.token_type != "Bearer"
            || !(120..=3600).contains(&token.expires_in)
            || validate_bearer(&token.access_token).is_err()
        {
            return Err(unavailable());
        }
        // Expiry measured from BEFORE the request; bounded early-refresh jitter.
        let skew = 60 + rand::random::<u64>() % 30;
        Ok(Cached {
            token: token.access_token,
            refresh_at: started + Duration::from_secs(token.expires_in - skew),
        })
    }
}
#[async_trait]
impl TokenProvider for ServiceAccountTokens {
    async fn bearer(&self, identity: &str, credential: &str) -> AppResult<Zeroizing<String>> {
        let entry = {
            let mut entries = self.entries.lock().await;
            if !entries.contains_key(identity) && entries.len() >= MAX_ENTRIES {
                // Evict only idle entries: never create two simultaneous mints for a key.
                let idle = entries
                    .iter()
                    .find(|(_, v)| Arc::strong_count(v) == 1)
                    .map(|(k, _)| k.clone());
                if let Some(idle) = idle {
                    entries.remove(&idle);
                } else {
                    return Err(unavailable());
                }
            }
            entries.entry(identity.into()).or_default().clone()
        };
        let mut entry = entry.lock().await;
        if let Some(cached) = &entry.cached
            && cached.refresh_at > Instant::now()
        {
            return Ok(cached.token.clone());
        }
        if entry.failed_until.is_some_and(|t| t > Instant::now()) {
            return Err(unavailable());
        }
        match self.mint(credential).await {
            Ok(cached) => {
                let token = cached.token.clone();
                entry.cached = Some(cached);
                entry.failed_until = None;
                Ok(token)
            }
            Err(error) => {
                entry.cached = None;
                entry.failed_until = Some(Instant::now() + Duration::from_secs(2));
                Err(error)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };
    fn key() -> String {
        super::super::tests::sa()
    }
    #[test]
    fn saved_credentials_share_the_runtime_parser_without_provider_calls() {
        assert!(validate_saved_credential(&key(), true).is_ok());
        assert!(validate_saved_credential("sk-test-fixture", false).is_ok());
        assert!(validate_saved_credential(&key(), false).is_err());
        for (field, value) in [
            ("type", serde_json::json!("external_account")),
            (
                "credential_source",
                serde_json::json!({"file":"secret-path"}),
            ),
            ("token_uri", serde_json::json!("https://private.invalid")),
            ("private_key", serde_json::json!("secret-material")),
        ] {
            let mut value_key: serde_json::Value = serde_json::from_str(&key()).unwrap();
            value_key[field] = value;
            let error = validate_saved_credential(&value_key.to_string(), true).unwrap_err();
            assert!(matches!(error, AppError::ValidationError(_)));
            assert!(!error.to_string().contains("secret-"));
            assert!(!error.to_string().contains("private.invalid"));
        }
    }
    #[test]
    fn bearer_cannot_contain_a_service_account_or_header_material() {
        for value in [
            key(),
            "Bearer secret".into(),
            "secret\r\nx-key: x".into(),
            "".into(),
        ] {
            assert!(validate_bearer(&value).is_err());
        }
        assert!(validate_bearer("sk-fixture_A.1~+/==").is_ok());
        assert!(validate_bearer("bad=padding").is_err());
    }
    #[tokio::test]
    async fn token_singleflight_refresh_rotation_and_failure_are_bounded() {
        let server = MockServer::start().await;
        Mock::given(method("POST")).and(path("/token")).respond_with(ResponseTemplate::new(200).set_delay(Duration::from_millis(20)).set_body_json(json!({"access_token":"test-token","token_type":"Bearer","expires_in":3600}))).expect(3).mount(&server).await;
        let provider = ServiceAccountTokens::with_endpoint(
            reqwest::Client::new(),
            format!("{}/token", server.uri()),
        );
        let raw = key();
        let results =
            futures::future::join_all((0..16).map(|_| provider.bearer("identity-v1", &raw))).await;
        assert!(results.iter().all(Result::is_ok));
        let entry = provider.entries.lock().await["identity-v1"].clone();
        entry.lock().await.cached.as_mut().unwrap().refresh_at = Instant::now();
        assert!(provider.bearer("identity-v1", &raw).await.is_ok());
        assert!(provider.bearer("identity-v2", &raw).await.is_ok());
        use rsa::pkcs8::{DecodePrivateKey, EncodePublicKey};
        let private =
            rsa::RsaPrivateKey::from_pkcs8_pem(crate::test_utils::TEST_GCP_SA_PRIVATE_KEY).unwrap();
        let public = private
            .to_public_key()
            .to_public_key_pem(Default::default())
            .unwrap();
        for request in server.received_requests().await.unwrap() {
            let form: HashMap<String, String> = url::form_urlencoded::parse(&request.body)
                .into_owned()
                .collect();
            assert_eq!(
                form["grant_type"],
                "urn:ietf:params:oauth:grant-type:jwt-bearer"
            );
            let mut validation = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::RS256);
            validation.set_audience(&[TOKEN_URL]);
            let decoded = jsonwebtoken::decode::<serde_json::Value>(
                &form["assertion"],
                &jsonwebtoken::DecodingKey::from_rsa_pem(public.as_bytes()).unwrap(),
                &validation,
            )
            .unwrap();
            assert_eq!(decoded.claims["scope"], SCOPE);
            assert_eq!(
                decoded.claims["iss"],
                "fixture@private-project.iam.gserviceaccount.com"
            );
            assert_eq!(
                decoded.claims["exp"].as_i64().unwrap() - decoded.claims["iat"].as_i64().unwrap(),
                3600
            );
            assert!(!form.contains_key("private_key"));
        }
        server.verify().await;
        server.reset().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(503))
            .expect(1)
            .mount(&server)
            .await;
        for _ in 0..3 {
            assert!(provider.bearer("failure", &raw).await.is_err());
        }
        server.verify().await;
    }
    #[tokio::test]
    async fn credential_urls_types_and_extra_sources_never_trigger_network() {
        let server = MockServer::start().await;
        let provider = ServiceAccountTokens::with_endpoint(
            reqwest::Client::new(),
            format!("{}/token", server.uri()),
        );
        for (i, (field, value)) in [
            ("type", json!("external_account")),
            ("token_uri", json!("http://127.0.0.1/token")),
            ("credential_source", json!({"file":"/tmp/key"})),
            ("executable", json!({"command":"anything"})),
            ("universe_domain", json!("evil.example")),
        ]
        .into_iter()
        .enumerate()
        {
            let mut raw: serde_json::Value = serde_json::from_str(&key()).unwrap();
            raw[field] = value;
            assert!(
                provider
                    .bearer(&i.to_string(), &raw.to_string())
                    .await
                    .is_err()
            );
        }
        assert!(server.received_requests().await.unwrap().is_empty());
    }
}

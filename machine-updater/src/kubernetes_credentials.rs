//! Closed credential sources for trusted Kubernetes clients. These credentials
//! never authenticate a Machine's bootstrap identity or enter its workload.
use anyhow::{Result, anyhow, ensure};
use reqwest::{
    Client, RequestBuilder, Url,
    header::{AUTHORIZATION, HeaderValue},
};
use serde::Deserialize;
use std::{
    fmt,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::{io::AsyncReadExt, sync::Mutex, time::Instant};
use zeroize::Zeroizing;

const METADATA_ENDPOINT: &str =
    "http://metadata.google.internal/computeMetadata/v1/instance/service-accounts/default/token";
const MAX_METADATA_RESPONSE: usize = 64 * 1024;
const MAX_METADATA_TOKEN: usize = 16 * 1024;
const MAX_FILE_TOKEN: usize = 64 * 1024;
const ACQUISITION_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone)]
pub struct KubernetesCredentials {
    source: Source,
}
#[derive(Clone)]
enum Source {
    Static(Zeroizing<String>),
    TokenFile(PathBuf),
    GoogleMetadata(Arc<Metadata>),
}
impl fmt::Debug for KubernetesCredentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mode = match self.source {
            Source::Static(_) => "static",
            Source::TokenFile(_) => "token_file",
            Source::GoogleMetadata(_) => "google_metadata",
        };
        f.debug_struct("KubernetesCredentials")
            .field("mode", &mode)
            .field("credential", &"[REDACTED]")
            .finish()
    }
}
struct Metadata {
    client: Client,
    endpoint: Url,
    acquisition_timeout: Duration,
    failure_cooldown: Duration,
    state: Mutex<MetadataState>,
}
#[derive(Default)]
struct MetadataState {
    cached: Option<CachedToken>,
    retry_after: Option<Instant>,
}
struct CachedToken {
    token: Zeroizing<String>,
    refresh_at: Instant,
}
#[derive(Deserialize)]
struct MetadataResponse {
    access_token: Zeroizing<String>,
    expires_in: u64,
}
fn metadata_error() -> anyhow::Error {
    anyhow!("Google metadata Kubernetes credential unavailable or invalid")
}
fn file_error() -> anyhow::Error {
    anyhow!("Kubernetes credential file unavailable or invalid")
}
fn valid_token(token: &str, max: usize) -> bool {
    !token.is_empty() && token.len() <= max && token.bytes().all(|b| b.is_ascii_graphic())
}

impl KubernetesCredentials {
    pub fn static_token(token: String) -> Self {
        Self {
            source: Source::Static(Zeroizing::new(token)),
        }
    }

    pub fn token_file(path: PathBuf) -> Result<Self> {
        ensure!(
            path.is_absolute(),
            "Kubernetes credential file requires an absolute path"
        );
        Ok(Self {
            source: Source::TokenFile(path),
        })
    }

    /// The fixed endpoint is a trusted backend capability, never caller input.
    pub fn google_metadata() -> Result<Self> {
        Self::metadata(
            METADATA_ENDPOINT.parse().expect("constant metadata URL"),
            Duration::from_secs(3),
            ACQUISITION_TIMEOUT,
            Duration::from_secs(1),
        )
    }

    fn metadata(
        endpoint: Url,
        request_timeout: Duration,
        acquisition_timeout: Duration,
        failure_cooldown: Duration,
    ) -> Result<Self> {
        let client = Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(1))
            .timeout(request_timeout)
            .build()
            .map_err(|_| metadata_error())?;
        Ok(Self {
            source: Source::GoogleMetadata(Arc::new(Metadata {
                client,
                endpoint,
                acquisition_timeout,
                failure_cooldown,
                state: Mutex::new(MetadataState::default()),
            })),
        })
    }

    /// Local fixture injection only; not available in production builds and
    /// never wired to production environment configuration.
    #[cfg(any(test, feature = "test-support"))]
    pub fn metadata_for_test(
        endpoint: Url,
        request_timeout: Duration,
        acquisition_timeout: Duration,
        failure_cooldown: Duration,
    ) -> Result<Self> {
        ensure!(
            endpoint.scheme() == "http"
                && matches!(endpoint.host_str(), Some("127.0.0.1" | "localhost")),
            "metadata fixture must be loopback"
        );
        Self::metadata(
            endpoint,
            request_timeout,
            acquisition_timeout,
            failure_cooldown,
        )
    }

    /// Configuration-only readiness. Metadata acquisition is intentionally
    /// deferred to the existing asynchronous API probes and actual requests.
    pub fn credential_file_available(&self) -> bool {
        match &self.source {
            Source::TokenFile(path) => open_protected_file(path).is_ok(),
            _ => true,
        }
    }

    pub async fn authorize(&self, request: RequestBuilder) -> Result<RequestBuilder> {
        let token = match &self.source {
            Source::Static(token) => token.clone(),
            Source::TokenFile(path) => read_file(path).await?,
            Source::GoogleMetadata(metadata) => metadata.token().await?,
        };
        // Explicitly sensitive even when a caller inspects a built Request.
        let value = Zeroizing::new(format!("Bearer {}", token.as_str()));
        let mut value =
            HeaderValue::from_str(&value).map_err(|_| anyhow!("Kubernetes credential invalid"))?;
        value.set_sensitive(true);
        Ok(request.header(AUTHORIZATION, value))
    }
}

fn open_protected_file(path: &Path) -> Result<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true);
    // Reject FIFO/devices without blocking; projected symlinks remain valid.
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NONBLOCK);
    }
    let file = options.open(path).map_err(|_| file_error())?;
    let metadata = file.metadata().map_err(|_| file_error())?;
    ensure!(
        metadata.is_file() && metadata.len() > 0 && metadata.len() <= MAX_FILE_TOKEN as u64,
        "Kubernetes credential file unavailable or invalid"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        ensure!(
            metadata.permissions().mode() & 0o022 == 0,
            "Kubernetes credential file unavailable or invalid"
        );
    }
    Ok(file)
}
async fn read_file(path: &Path) -> Result<Zeroizing<String>> {
    let file = tokio::fs::File::from_std(open_protected_file(path)?);
    let mut bytes = Zeroizing::new(Vec::new());
    tokio::time::timeout(
        ACQUISITION_TIMEOUT,
        file.take(MAX_FILE_TOKEN as u64 + 1).read_to_end(&mut bytes),
    )
    .await
    .map_err(|_| file_error())?
    .map_err(|_| file_error())?;
    ensure!(
        bytes.len() <= MAX_FILE_TOKEN,
        "Kubernetes credential file unavailable or invalid"
    );
    let token = std::str::from_utf8(&bytes)
        .map_err(|_| file_error())?
        .trim();
    ensure!(
        valid_token(token, MAX_FILE_TOKEN),
        "Kubernetes credential file unavailable or invalid"
    );
    Ok(Zeroizing::new(token.to_owned()))
}

impl Metadata {
    async fn token(&self) -> Result<Zeroizing<String>> {
        let deadline = Instant::now() + self.acquisition_timeout;
        // Recheck after taking the lock: both successful refreshes and failures
        // are shared by all consumers, including newly constructed clients.
        let mut state = tokio::time::timeout_at(deadline, self.state.lock())
            .await
            .map_err(|_| metadata_error())?;
        let now = Instant::now();
        if let Some(cached) = &state.cached
            && now < cached.refresh_at
        {
            return Ok(cached.token.clone());
        }
        state.cached = None; // Never return stale credentials on refresh failure.
        if state.retry_after.is_some_and(|retry| now < retry) {
            return Err(metadata_error());
        }
        // Also coalesce if the caller cancels acquisition while the HTTP request
        // is in flight. A subsequent caller may retry after the short cooldown.
        state.retry_after = Some(now + self.failure_cooldown);
        let fetched = tokio::time::timeout_at(deadline, self.fetch())
            .await
            .map_err(|_| metadata_error())
            .and_then(|r| r);
        match fetched {
            Ok(cached) => {
                let token = cached.token.clone();
                state.cached = Some(cached);
                state.retry_after = None;
                Ok(token)
            }
            Err(_) => {
                state.retry_after = Some(Instant::now() + self.failure_cooldown);
                Err(metadata_error())
            }
        }
    }
    async fn fetch(&self) -> Result<CachedToken> {
        let started = Instant::now(); // Conservatively charge network time to TTL.
        let mut response = self
            .client
            .get(self.endpoint.clone())
            .header("Metadata-Flavor", "Google")
            .send()
            .await
            .map_err(|_| metadata_error())?;
        ensure!(
            response.status() == reqwest::StatusCode::OK,
            "Google metadata Kubernetes credential unavailable or invalid"
        );
        if response
            .content_length()
            .is_some_and(|n| n > MAX_METADATA_RESPONSE as u64)
        {
            return Err(metadata_error());
        }
        let mut bytes = Zeroizing::new(Vec::new());
        while let Some(chunk) = response.chunk().await.map_err(|_| metadata_error())? {
            if chunk.len() > MAX_METADATA_RESPONSE - bytes.len() {
                return Err(metadata_error());
            }
            bytes.extend_from_slice(&chunk);
        }
        let response: MetadataResponse =
            serde_json::from_slice(&bytes).map_err(|_| metadata_error())?;
        if !valid_token(&response.access_token, MAX_METADATA_TOKEN)
            || !(1..=86400).contains(&response.expires_in)
        {
            return Err(metadata_error());
        }
        let lifetime = Duration::from_secs(response.expires_in);
        let refresh_at = started
            .checked_add(lifetime - std::cmp::min(Duration::from_secs(60), lifetime / 10))
            .ok_or_else(metadata_error)?;
        if Instant::now() >= refresh_at {
            return Err(metadata_error());
        }
        Ok(CachedToken {
            token: response.access_token,
            refresh_at,
        })
    }
}

#[cfg(test)]
#[path = "kubernetes_credentials_tests.rs"]
mod tests;

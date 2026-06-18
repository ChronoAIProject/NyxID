use anyhow::{Context, Result, bail};
use reqwest::Client;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

/// User-Agent string sent on all CLI HTTP requests.
pub const CLI_USER_AGENT: &str = concat!("nyxid-cli/", env!("CARGO_PKG_VERSION"));

#[derive(Debug, Clone, Serialize)]
pub struct AuthDeviceRequestBody {
    pub client_label: Option<String>,
    pub client_user_agent: Option<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct AuthDeviceRequestResponse {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub verification_uri_complete: String,
    pub expires_in: u64,
    pub interval: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct AuthDevicePollBody {
    pub device_code: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct AuthDevicePollResponse {
    pub access_token: String,
    pub refresh_token: String,
    pub token_type: String,
    pub expires_in: u64,
}

#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
pub struct ErrorEnvelope {
    pub error: String,
    pub error_code: i64,
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthDeviceRequestOutcome {
    Created(AuthDeviceRequestResponse),
    NotSupported,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthDevicePollOutcome {
    Delivered(AuthDevicePollResponse),
    Error(ErrorEnvelope),
}

pub fn build_cli_http_client(profile: Option<&str>) -> Result<Client> {
    // Attach `X-NyxID-Client: cli` + `X-NyxID-Client-Version` ONLY when
    // BOTH conditions are true:
    //   (a) the operator has configured a telemetry DSN (or opted into
    //       the community share-back), AND
    //   (b) the user on this machine has actually consented (CLI
    //       config / `NYXID_TELEMETRY=on` / `DO_NOT_TRACK` — see
    //       `crate::telemetry::consent::resolve_consent`).
    //
    // Historically this function checked only (a), which meant that a
    // user who had run `nyxid telemetry disable` — or set
    // `DO_NOT_TRACK=1` — still produced surface-tagged headers on every
    // request, and the backend emitted `surface="cli"` telemetry events
    // for their traffic anyway. Local opt-out was partial theater.
    // Tracked in `docs/TELEMETRY_CONSENT_FIX.md` §8.3.
    //
    // The `profile` argument is kept on the signature for future per-
    // profile features, but consent is read against the DEFAULT profile
    // regardless. Rationale: the only built-in consent editor today is
    // `nyxid telemetry enable|disable`, which writes to
    // `~/.nyxid/config.toml` (default profile). The main.rs first-run
    // prompt also writes there. Reading profile-specific consent here
    // would mean `nyxid --profile dev some-command` silently lost its
    // headers until the user manually edited `~/.nyxid/profiles/dev/
    // config.toml` — a footgun that treats consent as per-profile when
    // no editor UI treats it that way. Consent is user-global in v1;
    // making it per-profile is a separate feature that also needs a
    // `nyxid telemetry --profile ...` editor path.
    let telemetry_configured = std::env::var("NYXID_TELEMETRY_DSN")
        .ok()
        .is_some_and(|s| !s.is_empty())
        || std::env::var("NYXID_SHARE_ANALYTICS")
            .ok()
            .is_some_and(|v| {
                matches!(v.to_ascii_lowercase().as_str(), "true" | "1" | "yes" | "on")
            });

    // `resolve_consent_preferring_profile` honors the default profile
    // (where v1 persists user choice) but also falls back to any
    // explicit per-profile consent from older releases, so upgrading
    // doesn't silently override a historical opt-out on a named
    // profile.
    let user_consented =
        crate::telemetry::consent::resolve_consent_preferring_profile(profile).enabled;

    let mut builder = Client::builder()
        .user_agent(CLI_USER_AGENT)
        .connect_timeout(std::time::Duration::from_secs(10));

    if telemetry_configured && user_consented {
        let mut default_headers = reqwest::header::HeaderMap::new();
        default_headers.insert(
            reqwest::header::HeaderName::from_static("x-nyxid-client"),
            reqwest::header::HeaderValue::from_static("cli"),
        );
        default_headers.insert(
            reqwest::header::HeaderName::from_static("x-nyxid-client-version"),
            reqwest::header::HeaderValue::from_static(env!("CARGO_PKG_VERSION")),
        );
        builder = builder.default_headers(default_headers);
    }

    builder.build().context("Failed to build HTTP client")
}

pub async fn auth_device_request(
    base_url: &str,
    body: &AuthDeviceRequestBody,
    profile: Option<&str>,
) -> Result<AuthDeviceRequestOutcome> {
    let base = format!("{}/api/v1", base_url.trim_end_matches('/'));
    let url = format!("{base}/auth/device/request");
    let client = build_cli_http_client(profile)?;

    let resp = client
        .post(&url)
        .json(body)
        .send()
        .await
        .context("POST /auth/device/request failed")?;

    if resp.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(AuthDeviceRequestOutcome::NotSupported);
    }

    if resp.status().is_success() {
        let body = resp
            .json::<AuthDeviceRequestResponse>()
            .await
            .context("Failed to parse response from /auth/device/request")?;
        return Ok(AuthDeviceRequestOutcome::Created(body));
    }

    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    bail!("/auth/device/request failed (HTTP {status}): {body}");
}

pub async fn auth_device_poll(
    base_url: &str,
    body: &AuthDevicePollBody,
    profile: Option<&str>,
) -> Result<AuthDevicePollOutcome> {
    let base = format!("{}/api/v1", base_url.trim_end_matches('/'));
    let url = format!("{base}/auth/device/poll");
    let client = build_cli_http_client(profile)?;

    let resp = client
        .post(&url)
        .json(body)
        .send()
        .await
        .context("POST /auth/device/poll failed")?;

    if resp.status().is_success() {
        let body = resp
            .json::<AuthDevicePollResponse>()
            .await
            .context("Failed to parse response from /auth/device/poll")?;
        return Ok(AuthDevicePollOutcome::Delivered(body));
    }

    let status = resp.status();
    let body = resp.text().await.unwrap_or_default();
    match serde_json::from_str::<ErrorEnvelope>(&body) {
        Ok(error) => Ok(AuthDevicePollOutcome::Error(error)),
        Err(_) => bail!("/auth/device/poll failed (HTTP {status}): {body}"),
    }
}

pub struct ApiClient {
    client: Client,
    base_url: String,
    access_token: String,
    profile: Option<String>,
    /// When true, 401 responses are never retried with a session-refreshed
    /// token. Used by flows like `channel-event push` that authenticate with
    /// a specific API key and must not silently fall back to the user's
    /// saved session access token on the `profile`.
    refresh_disabled: bool,
}

impl ApiClient {
    pub fn new(base_url: &str, access_token: String) -> Result<Self> {
        Self::new_with_profile(base_url, access_token, None)
    }

    pub fn new_with_profile(
        base_url: &str,
        access_token: String,
        profile: Option<String>,
    ) -> Result<Self> {
        let client = build_cli_http_client(profile.as_deref())?;

        Ok(Self {
            client,
            base_url: format!("{}/api/v1", base_url.trim_end_matches('/')),
            access_token,
            profile,
            refresh_disabled: false,
        })
    }

    pub fn from_auth(auth: &crate::cli::AuthArgs) -> Result<Self> {
        let base_url = auth.resolved_base_url()?;
        let token = crate::auth::resolve_access_token(auth)?;
        Self::new_with_profile(&base_url, token, auth.profile.clone())
    }

    /// Build a client after validating the saved session up front.
    ///
    /// Runs [`crate::auth::ensure_session`] first: a no-op when the access
    /// token is still valid (local `exp` check, no network), a silent refresh
    /// when it's expired, or a prompt/clean-error when the session is dead.
    /// Only then does it build the client. Use this for session-authenticated
    /// commands so a dead session is handled before the command does anything
    /// user-visible, instead of surfacing a raw 401 mid-operation.
    ///
    /// Do NOT use for flows that authenticate with a non-session credential
    /// (e.g. an agent API key passed via env) -- those should keep
    /// [`Self::from_auth`] / [`Self::without_token_refresh`], since
    /// `ensure_session` would have nothing to validate there anyway.
    pub async fn from_auth_checked(auth: &crate::cli::AuthArgs) -> Result<Self> {
        crate::auth::ensure_session(auth).await?;
        Self::from_auth(auth)
    }

    /// Disable the automatic 401 → session-refresh retry path.
    ///
    /// Returns `self` as a fluent builder. Use this when the caller is
    /// authenticating with a specific API key (e.g. an agent `nyxid_ag_...`
    /// key for `POST /channel-events/{id}`) and a 401 should surface the
    /// real error instead of silently retrying with the saved session
    /// access token for the profile.
    pub fn without_token_refresh(mut self) -> Self {
        self.refresh_disabled = true;
        self
    }

    pub fn base_url_root(&self) -> &str {
        self.base_url
            .strip_suffix("/api/v1")
            .unwrap_or(&self.base_url)
    }

    /// Attempt to refresh the access token using the saved refresh token.
    /// Returns `true` if the token was refreshed successfully. Delegates the
    /// wire protocol to [`crate::auth::exchange_refresh_token`] (the single
    /// source of truth) and owns only the token I/O: read the saved refresh
    /// token, persist the rotated pair, and update this client's in-memory copy.
    async fn try_refresh_token(&mut self) -> bool {
        if self.refresh_disabled {
            return false;
        }
        let profile = self.profile.as_deref();
        let refresh_token = match crate::auth::read_saved_refresh_token_for(profile) {
            Some(rt) => rt,
            None => return false,
        };

        match crate::auth::exchange_refresh_token(
            &self.client,
            self.base_url_root(),
            &refresh_token,
        )
        .await
        {
            crate::auth::RefreshExchange::Renewed {
                access_token,
                refresh_token,
            } => {
                if crate::auth::save_tokens_for(profile, &access_token, Some(&refresh_token))
                    .is_err()
                {
                    return false;
                }
                self.access_token = access_token;
                true
            }
            crate::auth::RefreshExchange::Unauthorized
            | crate::auth::RefreshExchange::Network(_) => false,
        }
    }

    pub async fn get<T: DeserializeOwned>(&mut self, path: &str) -> Result<T> {
        let url = format!("{}{path}", self.base_url);
        let resp = self
            .client
            .get(&url)
            .bearer_auth(&self.access_token)
            .send()
            .await
            .with_context(|| format!("GET {path} failed"))?;

        if resp.status() == reqwest::StatusCode::UNAUTHORIZED && self.try_refresh_token().await {
            let resp = self
                .client
                .get(&url)
                .bearer_auth(&self.access_token)
                .send()
                .await
                .with_context(|| format!("GET {path} failed (retry)"))?;
            return Self::handle_response(resp, path).await;
        }

        Self::handle_response(resp, path).await
    }

    pub async fn get_value(&mut self, path: &str) -> Result<serde_json::Value> {
        self.get(path).await
    }

    /// GET that returns `Ok(None)` on 404 instead of an error.
    pub async fn get_optional<T: DeserializeOwned>(&mut self, path: &str) -> Result<Option<T>> {
        let url = format!("{}{path}", self.base_url);
        let mut resp = self
            .client
            .get(&url)
            .bearer_auth(&self.access_token)
            .send()
            .await
            .with_context(|| format!("GET {path} failed"))?;

        if resp.status() == reqwest::StatusCode::UNAUTHORIZED && self.try_refresh_token().await {
            resp = self
                .client
                .get(&url)
                .bearer_auth(&self.access_token)
                .send()
                .await
                .with_context(|| format!("GET {path} failed (retry)"))?;
        }

        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        Self::handle_response(resp, path).await.map(Some)
    }

    pub async fn post<T: DeserializeOwned, B: Serialize>(
        &mut self,
        path: &str,
        body: &B,
    ) -> Result<T> {
        let url = format!("{}{path}", self.base_url);
        let resp = self
            .client
            .post(&url)
            .bearer_auth(&self.access_token)
            .json(body)
            .send()
            .await
            .with_context(|| format!("POST {path} failed"))?;

        if resp.status() == reqwest::StatusCode::UNAUTHORIZED && self.try_refresh_token().await {
            let resp = self
                .client
                .post(&url)
                .bearer_auth(&self.access_token)
                .json(body)
                .send()
                .await
                .with_context(|| format!("POST {path} failed (retry)"))?;
            return Self::handle_response(resp, path).await;
        }

        Self::handle_response(resp, path).await
    }

    pub async fn put<T: DeserializeOwned, B: Serialize>(
        &mut self,
        path: &str,
        body: &B,
    ) -> Result<T> {
        let url = format!("{}{path}", self.base_url);
        let resp = self
            .client
            .put(&url)
            .bearer_auth(&self.access_token)
            .json(body)
            .send()
            .await
            .with_context(|| format!("PUT {path} failed"))?;

        if resp.status() == reqwest::StatusCode::UNAUTHORIZED && self.try_refresh_token().await {
            let resp = self
                .client
                .put(&url)
                .bearer_auth(&self.access_token)
                .json(body)
                .send()
                .await
                .with_context(|| format!("PUT {path} failed (retry)"))?;
            return Self::handle_response(resp, path).await;
        }

        Self::handle_response(resp, path).await
    }

    #[allow(dead_code)]
    pub async fn patch<T: DeserializeOwned, B: Serialize>(
        &mut self,
        path: &str,
        body: &B,
    ) -> Result<T> {
        let url = format!("{}{path}", self.base_url);
        let resp = self
            .client
            .patch(&url)
            .bearer_auth(&self.access_token)
            .json(body)
            .send()
            .await
            .with_context(|| format!("PATCH {path} failed"))?;

        if resp.status() == reqwest::StatusCode::UNAUTHORIZED && self.try_refresh_token().await {
            let resp = self
                .client
                .patch(&url)
                .bearer_auth(&self.access_token)
                .json(body)
                .send()
                .await
                .with_context(|| format!("PATCH {path} failed (retry)"))?;
            return Self::handle_response(resp, path).await;
        }

        Self::handle_response(resp, path).await
    }

    pub async fn delete_empty(&mut self, path: &str) -> Result<()> {
        let url = format!("{}{path}", self.base_url);
        let resp = self
            .client
            .delete(&url)
            .bearer_auth(&self.access_token)
            .send()
            .await
            .with_context(|| format!("DELETE {path} failed"))?;

        if resp.status() == reqwest::StatusCode::UNAUTHORIZED && self.try_refresh_token().await {
            let resp = self
                .client
                .delete(&url)
                .bearer_auth(&self.access_token)
                .send()
                .await
                .with_context(|| format!("DELETE {path} failed (retry)"))?;
            if resp.status().is_success() {
                return Ok(());
            }
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            bail!("DELETE {path} failed (HTTP {status}): {body}");
        }

        if resp.status().is_success() {
            Ok(())
        } else {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            bail!("DELETE {path} failed (HTTP {status}): {body}");
        }
    }

    pub async fn delete<T: DeserializeOwned>(&mut self, path: &str) -> Result<T> {
        let url = format!("{}{path}", self.base_url);
        let resp = self
            .client
            .delete(&url)
            .bearer_auth(&self.access_token)
            .send()
            .await
            .with_context(|| format!("DELETE {path} failed"))?;

        if resp.status() == reqwest::StatusCode::UNAUTHORIZED && self.try_refresh_token().await {
            let resp = self
                .client
                .delete(&url)
                .bearer_auth(&self.access_token)
                .send()
                .await
                .with_context(|| format!("DELETE {path} failed (retry)"))?;
            return Self::handle_response(resp, path).await;
        }

        Self::handle_response(resp, path).await
    }

    #[allow(dead_code)]
    pub async fn post_empty<B: Serialize>(&mut self, path: &str, body: &B) -> Result<()> {
        let url = format!("{}{path}", self.base_url);
        let resp = self
            .client
            .post(&url)
            .bearer_auth(&self.access_token)
            .json(body)
            .send()
            .await
            .with_context(|| format!("POST {path} failed"))?;

        if resp.status() == reqwest::StatusCode::UNAUTHORIZED && self.try_refresh_token().await {
            let resp = self
                .client
                .post(&url)
                .bearer_auth(&self.access_token)
                .json(body)
                .send()
                .await
                .with_context(|| format!("POST {path} failed (retry)"))?;
            if resp.status().is_success() {
                return Ok(());
            }
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            bail!("POST {path} failed (HTTP {status}): {body}");
        }

        if resp.status().is_success() {
            Ok(())
        } else {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            bail!("POST {path} failed (HTTP {status}): {body}");
        }
    }

    pub async fn proxy_request(
        &mut self,
        method: &str,
        path: &str,
        headers: &[(String, String)],
        body: Option<&[u8]>,
    ) -> Result<reqwest::Response> {
        let url = format!("{}{path}", self.base_url);
        let method_parsed = reqwest::Method::from_bytes(method.to_uppercase().as_bytes())
            .with_context(|| format!("Invalid HTTP method: {method}"))?;

        // Strip any client-supplied Authorization header. The NyxID access
        // token must be the sole Authorization header on the wire -- letting a
        // second one through creates duplicate headers that Cloudflare rejects
        // at the edge with 400 Bad Request. Downstream service credentials
        // come from the service configuration, not from client headers.
        let filtered_headers: Vec<(String, String)> = headers
            .iter()
            .filter(|(k, _)| {
                if k.eq_ignore_ascii_case("authorization") {
                    eprintln!(
                        "warning: -H 'Authorization: ...' is reserved for NyxID auth and has been dropped. \
                         Downstream credentials come from the service configuration."
                    );
                    false
                } else {
                    true
                }
            })
            .cloned()
            .collect();

        let has_content_type = filtered_headers
            .iter()
            .any(|(k, _)| k.eq_ignore_ascii_case("content-type"));

        let client = self.client.clone();
        let body_vec = body.map(|b| b.to_vec());
        let build_req = |token: &str| {
            let mut req = client
                .request(method_parsed.clone(), &url)
                .bearer_auth(token);
            for (k, v) in &filtered_headers {
                req = req.header(k.as_str(), v.as_str());
            }
            if let Some(ref b) = body_vec {
                if !has_content_type {
                    // Default to application/octet-stream for raw bytes;
                    // callers sending JSON should pass -H 'Content-Type: application/json'.
                    req = req.header("content-type", "application/octet-stream");
                }
                req = req.body(b.clone());
            }
            req
        };

        let original_token = self.access_token.clone();

        let resp = build_req(&original_token)
            .send()
            .await
            .with_context(|| format!("Proxy request to {path} failed"))?;

        if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
            // Drain the auth error body before refreshing so reqwest can reuse
            // the connection for the refresh + retry sequence.
            let _ = resp.bytes().await;

            if self.try_refresh_token().await {
                return build_req(&self.access_token)
                    .send()
                    .await
                    .with_context(|| format!("Proxy request to {path} failed (retry)"));
            }

            // If refresh failed, replay the original unauthorized request so
            // callers still receive the backend's 401 payload.
            return build_req(&original_token)
                .send()
                .await
                .with_context(|| format!("Proxy request to {path} failed (unauthorized replay)"));
        }

        Ok(resp)
    }

    async fn handle_response<T: DeserializeOwned>(
        resp: reqwest::Response,
        path: &str,
    ) -> Result<T> {
        if resp.status().is_success() {
            resp.json()
                .await
                .with_context(|| format!("Failed to parse response from {path}"))
        } else {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            bail!("{path} failed (HTTP {status}): {body}");
        }
    }
}

/// Make an unauthenticated POST request and return parsed JSON.
pub async fn anonymous_post(
    base_url: &str,
    path: &str,
    body: &impl Serialize,
    profile: Option<&str>,
) -> Result<serde_json::Value> {
    let base = format!("{}/api/v1", base_url.trim_end_matches('/'));
    let url = format!("{base}{path}");
    let client = build_cli_http_client(profile)?;

    let resp = client
        .post(&url)
        .json(body)
        .send()
        .await
        .with_context(|| format!("POST {path} failed"))?;

    if resp.status().is_success() {
        resp.json()
            .await
            .with_context(|| format!("Failed to parse response from {path}"))
    } else {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        bail!("{path} failed (HTTP {status}): {body}");
    }
}

/// Make an unauthenticated POST request that returns no body.
pub async fn anonymous_post_empty(
    base_url: &str,
    path: &str,
    body: &impl Serialize,
    profile: Option<&str>,
) -> Result<()> {
    let base = format!("{}/api/v1", base_url.trim_end_matches('/'));
    let url = format!("{base}{path}");
    let client = build_cli_http_client(profile)?;

    let resp = client
        .post(&url)
        .json(body)
        .send()
        .await
        .with_context(|| format!("POST {path} failed"))?;

    if resp.status().is_success() {
        Ok(())
    } else {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        bail!("{path} failed (HTTP {status}): {body}");
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::io;
    use std::sync::Mutex;

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};

    use super::ApiClient;

    fn env_lock() -> &'static Mutex<()> {
        crate::test_support::env_lock()
    }

    struct HomeGuard {
        previous: Option<String>,
    }

    impl HomeGuard {
        fn set(path: &std::path::Path) -> Self {
            let previous = std::env::var("HOME").ok();
            // SAFETY: these tests serialize HOME mutations with `env_lock`.
            unsafe {
                std::env::set_var("HOME", path);
            }
            Self { previous }
        }
    }

    impl Drop for HomeGuard {
        fn drop(&mut self) {
            // SAFETY: these tests serialize HOME mutations with `env_lock`.
            unsafe {
                if let Some(previous) = &self.previous {
                    std::env::set_var("HOME", previous);
                } else {
                    std::env::remove_var("HOME");
                }
            }
        }
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)] // intentional: serialises HOME mutations across tests
    async fn proxy_request_refreshes_expired_token_and_retries() {
        let _env_guard = env_lock().lock().expect("env lock");
        let temp = tempfile::tempdir().expect("temp dir");
        let _home = HomeGuard::set(temp.path());

        crate::auth::save_tokens("expired-access-token", Some("valid-refresh-token"))
            .expect("save initial tokens");

        let base_url = start_proxy_refresh_test_server()
            .await
            .expect("test server");
        let mut api =
            ApiClient::new(&base_url, "expired-access-token".to_string()).expect("api client");

        let resp = api
            .proxy_request("GET", "/proxy/s/home-assistant/api/", &[], None)
            .await
            .expect("proxy response");

        assert_eq!(resp.status(), reqwest::StatusCode::OK);
        assert_eq!(
            resp.text().await.expect("response body"),
            r#"{"message":"API running."}"#
        );
        assert_eq!(
            crate::auth::read_saved_token().as_deref(),
            Some("fresh-access-token")
        );
        assert_eq!(
            crate::auth::read_saved_refresh_token().as_deref(),
            Some("fresh-refresh-token")
        );
    }

    async fn start_proxy_refresh_test_server() -> io::Result<String> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;

        tokio::spawn(async move {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };

            handle_proxy_refresh_sequence(&mut socket)
                .await
                .expect("proxy refresh sequence");
        });

        Ok(format!("http://{addr}"))
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)] // intentional: serialises HOME mutations across tests
    async fn proxy_request_strips_client_authorization_header() {
        let _env_guard = env_lock().lock().expect("env lock");
        let temp = tempfile::tempdir().expect("temp dir");
        let _home = HomeGuard::set(temp.path());

        crate::auth::save_tokens("nyxid-access-token", Some("nyxid-refresh-token"))
            .expect("save initial tokens");

        let base_url = start_auth_capture_server().await.expect("test server");
        let mut api =
            ApiClient::new(&base_url, "nyxid-access-token".to_string()).expect("api client");

        // User attempts to pass a downstream bearer token via -H. The CLI
        // must strip it so only the NyxID access token reaches the server;
        // otherwise Cloudflare 400s on duplicate Authorization headers.
        let user_headers = vec![(
            "Authorization".to_string(),
            "Bearer user-provided-token".to_string(),
        )];

        let resp = api
            .proxy_request("GET", "/proxy/s/api-lark-bot/ping", &user_headers, None)
            .await
            .expect("proxy response");

        assert_eq!(resp.status(), reqwest::StatusCode::OK);
    }

    async fn start_auth_capture_server() -> io::Result<String> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let addr = listener.local_addr()?;

        tokio::spawn(async move {
            let Ok((mut socket, _)) = listener.accept().await else {
                return;
            };

            let req = read_request(&mut socket).await.expect("read proxy request");

            // The server must see exactly one Authorization header, and it
            // must be the NyxID access token -- not the user-supplied one.
            assert_eq!(req.authorization_count, 1, "duplicate Authorization");
            assert_eq!(
                req.headers.get("authorization").map(String::as_str),
                Some("Bearer nyxid-access-token"),
                "wrong Authorization header forwarded"
            );

            write_response(&mut socket, 200, "OK", r#"{"ok":true}"#)
                .await
                .expect("write response");
        });

        Ok(format!("http://{addr}"))
    }

    async fn handle_proxy_refresh_sequence(socket: &mut TcpStream) -> io::Result<()> {
        let first = read_request(socket).await?;
        assert_eq!(first.method, "GET");
        assert_eq!(first.path, "/api/v1/proxy/s/home-assistant/api/");
        assert_eq!(
            first.headers.get("authorization").map(String::as_str),
            Some("Bearer expired-access-token")
        );
        write_response(
            socket,
            401,
            "Unauthorized",
            r#"{"error":"token_expired","error_code":2001,"message":"Token expired"}"#,
        )
        .await?;

        let second = read_request(socket).await?;
        assert_eq!(second.method, "POST");
        assert_eq!(second.path, "/api/v1/auth/refresh");
        assert!(
            second
                .body
                .contains(r#""refresh_token":"valid-refresh-token""#),
            "refresh body: {}",
            second.body
        );
        write_response(
            socket,
            200,
            "OK",
            r#"{"access_token":"fresh-access-token","expires_in":900,"refresh_token":"fresh-refresh-token"}"#,
        )
        .await?;

        let third = read_request(socket).await?;
        assert_eq!(third.method, "GET");
        assert_eq!(third.path, "/api/v1/proxy/s/home-assistant/api/");
        assert_eq!(
            third.headers.get("authorization").map(String::as_str),
            Some("Bearer fresh-access-token")
        );
        write_response(socket, 200, "OK", r#"{"message":"API running."}"#).await
    }

    struct ParsedRequest {
        method: String,
        path: String,
        headers: HashMap<String, String>,
        authorization_count: usize,
        body: String,
    }

    async fn read_request(socket: &mut TcpStream) -> io::Result<ParsedRequest> {
        let mut buf = Vec::new();
        let header_end = loop {
            let mut chunk = [0u8; 1024];
            let n = socket.read(&mut chunk).await?;
            if n == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "socket closed before request completed",
                ));
            }
            buf.extend_from_slice(&chunk[..n]);
            if let Some(end) = find_header_end(&buf) {
                break end;
            }
        };

        let header_text = String::from_utf8_lossy(&buf[..header_end]);
        let mut lines = header_text.split("\r\n");
        let request_line = lines
            .next()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing request line"))?;
        let mut request_parts = request_line.split_whitespace();
        let method = request_parts
            .next()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing method"))?
            .to_string();
        let path = request_parts
            .next()
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing path"))?
            .to_string();

        let mut headers = HashMap::new();
        let mut content_length = 0usize;
        let mut authorization_count = 0usize;
        for line in lines {
            if line.is_empty() {
                continue;
            }
            let Some((name, value)) = line.split_once(':') else {
                continue;
            };
            let name = name.trim().to_ascii_lowercase();
            let value = value.trim().to_string();
            if name == "content-length" {
                content_length = value.parse().unwrap_or(0);
            }
            if name == "authorization" {
                authorization_count += 1;
            }
            headers.insert(name, value);
        }

        while buf.len() - header_end < content_length {
            let mut chunk = vec![0u8; content_length - (buf.len() - header_end)];
            let n = socket.read(&mut chunk).await?;
            if n == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "socket closed before request body completed",
                ));
            }
            buf.extend_from_slice(&chunk[..n]);
        }

        let body =
            String::from_utf8_lossy(&buf[header_end..header_end + content_length]).to_string();

        Ok(ParsedRequest {
            method,
            path,
            headers,
            authorization_count,
            body,
        })
    }

    fn find_header_end(buf: &[u8]) -> Option<usize> {
        buf.windows(4)
            .position(|window| window == b"\r\n\r\n")
            .map(|idx| idx + 4)
    }

    async fn write_response(
        socket: &mut TcpStream,
        status: u16,
        reason: &str,
        body: &str,
    ) -> io::Result<()> {
        let response = format!(
            "HTTP/1.1 {status} {reason}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: keep-alive\r\n\r\n{body}",
            body.len()
        );
        socket.write_all(response.as_bytes()).await
    }
}

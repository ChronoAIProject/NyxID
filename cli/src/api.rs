use anyhow::{Context, Result, bail};
use reqwest::Client;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

/// User-Agent string sent on all CLI HTTP requests.
pub const CLI_USER_AGENT: &str = concat!("nyxid-cli/", env!("CARGO_PKG_VERSION"));

pub fn device_login_user_agent() -> String {
    format!(
        "{} ({}; {})",
        CLI_USER_AGENT,
        std::env::consts::OS,
        std::env::consts::ARCH
    )
}

#[cfg(test)]
mod device_login_user_agent_tests {
    use super::{CLI_USER_AGENT, device_login_user_agent};

    #[test]
    fn device_login_user_agent_adds_os_and_arch_without_changing_the_base_ua() {
        let expected = format!(
            "{} ({}; {})",
            CLI_USER_AGENT,
            std::env::consts::OS,
            std::env::consts::ARCH
        );
        assert_eq!(device_login_user_agent(), expected);
        assert_eq!(
            CLI_USER_AGENT,
            concat!("nyxid-cli/", env!("CARGO_PKG_VERSION"))
        );
    }
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct ErrorEnvelope {
    pub error: String,
    pub error_code: i64,
    pub message: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
}

#[derive(Debug, thiserror::Error)]
#[error("{request} failed (HTTP {status}): {body}")]
pub struct ApiError {
    request: String,
    status: reqwest::StatusCode,
    body: String,
    response: Option<ErrorEnvelope>,
    body_json: Option<serde_json::Value>,
}

impl ApiError {
    pub(crate) fn new(
        request: impl Into<String>,
        status: reqwest::StatusCode,
        body: String,
    ) -> Self {
        let body_json = serde_json::from_str::<serde_json::Value>(&body).ok();
        let response = body_json
            .as_ref()
            .and_then(|value| serde_json::from_value(value.clone()).ok());
        Self {
            request: request.into(),
            status,
            body,
            response,
            body_json,
        }
    }

    pub fn status(&self) -> reqwest::StatusCode {
        self.status
    }

    pub fn response(&self) -> Option<&ErrorEnvelope> {
        self.response.as_ref()
    }

    pub fn body_json(&self) -> Option<&serde_json::Value> {
        self.body_json.as_ref()
    }

    pub(crate) fn request_path(&self) -> &str {
        self.request
            .split_whitespace()
            .last()
            .unwrap_or("")
            .split('?')
            .next()
            .unwrap_or("")
    }
}

pub fn build_cli_http_client(profile: Option<&str>) -> Result<Client> {
    build_cli_http_client_with_redirect(profile, reqwest::redirect::Policy::limited(10))
}

pub fn build_credential_http_client(profile: Option<&str>) -> Result<Client> {
    build_cli_http_client_with_redirect(profile, reqwest::redirect::Policy::none())
}

fn build_cli_http_client_with_redirect(
    profile: Option<&str>,
    redirect: reqwest::redirect::Policy,
) -> Result<Client> {
    cli_http_client_builder(profile)?
        .redirect(redirect)
        .build()
        .context("Failed to build HTTP client")
}

fn cli_http_client_builder(profile: Option<&str>) -> Result<reqwest::ClientBuilder> {
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

    let mut builder = crate::tls::client_builder()?
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

    Ok(builder)
}

fn credential_request_url(address: &str) -> Result<url::Url> {
    let address =
        url::Url::parse(address).map_err(|_| anyhow::anyhow!("Invalid credential request URL"))?;
    if address.host().is_none()
        || !address.username().is_empty()
        || address.password().is_some()
        || address.fragment().is_some()
    {
        bail!("Credential request URL must have a host without userinfo or a fragment");
    }
    if address.scheme() != "https"
        && !(address.scheme() == "http"
            && matches!(
                address.host_str(),
                Some("localhost" | "127.0.0.1" | "[::1]")
            ))
    {
        bail!(
            "Credential request requires HTTPS; HTTP is allowed only for localhost, 127.0.0.1, or [::1]"
        );
    }
    Ok(address)
}

/// Build a client for a credential-bearing API or authentication request.
/// Remote destinations require HTTPS; local development bypasses proxies.
pub(crate) fn build_credential_http_client_for_url(
    profile: Option<&str>,
    address: &str,
) -> Result<Client> {
    credential_http_client_builder(profile, address)?
        .build()
        .context("Failed to build credential request client")
}

fn credential_http_client_builder(
    profile: Option<&str>,
    address: &str,
) -> Result<reqwest::ClientBuilder> {
    let address = credential_request_url(address)?;
    let local_http = address.scheme() == "http";
    let mut builder = cli_http_client_builder(profile)?
        .https_only(!local_http)
        .redirect(reqwest::redirect::Policy::none());
    if local_http {
        // Match the existing provider OAuth policy: local credentials bypass
        // proxies, localhost is pinned, and no redirect can leave this origin.
        builder = builder
            .no_proxy()
            .resolve("localhost", std::net::SocketAddr::from(([127, 0, 0, 1], 0)));
    }
    Ok(builder)
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
    write_refresh_disabled: bool,
    agent_key_auth: bool,
    login_generation: Option<String>,
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
        let client = build_credential_http_client_for_url(profile.as_deref(), base_url)?;
        let agent_key_auth = crate::auth::agent_key::is_agent_key_profile(profile.as_deref())
            && crate::auth::read_saved_token_for(profile.as_deref()).as_deref()
                == Some(access_token.as_str());
        let login_generation = crate::auth::login_generation(profile.as_deref());

        Ok(Self {
            client,
            base_url: format!("{}/api/v1", base_url.trim_end_matches('/')),
            access_token,
            profile,
            refresh_disabled: agent_key_auth,
            write_refresh_disabled: false,
            agent_key_auth,
            login_generation,
        })
    }

    pub fn from_auth(auth: &crate::cli::AuthArgs) -> Result<Self> {
        if let Some(resolved) = crate::auth::resolve_access_token_override(auth) {
            let base_url = auth.resolved_base_url()?;
            return Ok(Self {
                client: build_credential_http_client_for_url(auth.profile.as_deref(), &base_url)?,
                base_url: format!("{}/api/v1", base_url.trim_end_matches('/')),
                access_token: resolved.token,
                profile: auth.profile.clone(),
                refresh_disabled: true,
                write_refresh_disabled: false,
                agent_key_auth: false,
                login_generation: None,
            });
        }
        let _lock = crate::auth::acquire_refresh_lock(auth.profile.as_deref())?;
        let base_url = auth.resolved_base_url()?;
        let resolved = crate::auth::resolve_access_token_with_source(auth)?;
        let allow_refresh = resolved.source.allows_session_refresh();
        if allow_refresh || resolved.source == crate::auth::AccessTokenSource::SavedAgentKey {
            crate::auth::validate_profile_destination(auth.profile.as_deref(), &base_url)?;
        }
        let mut client = Self::new_with_profile(&base_url, resolved.token, auth.profile.clone())?;
        client.refresh_disabled = !allow_refresh;
        client.agent_key_auth = resolved.source == crate::auth::AccessTokenSource::SavedAgentKey;
        Ok(client)
    }

    /// Bind the hidden-release command's reads, token refresh and writes to
    /// the same HTTPS/loopback transport before making any request. A saved
    /// session renews on 401 through this client; caller-selected keys never do.
    pub fn from_auth_with_credential_transport(auth: &crate::cli::AuthArgs) -> Result<Self> {
        let mut api = Self::from_auth(auth)?;
        api.require_credential_transport()?;
        Ok(api)
    }

    /// Install HTTPS/exact-loopback and no-redirect guarantees before reading
    /// or transmitting a caller-supplied downstream credential.
    pub fn require_credential_transport(&mut self) -> Result<()> {
        self.client =
            build_credential_http_client_for_url(self.profile.as_deref(), self.base_url_root())?;
        Ok(())
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
    /// Caller-selected credentials are recognized by [`Self::from_auth`] and
    /// are never eligible for saved-session refresh.
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

    /// Preserve read refresh while refusing automatic replay of mutations.
    pub fn without_write_refresh(mut self) -> Self {
        self.write_refresh_disabled = true;
        self
    }

    pub fn for_credential_transfer(mut self) -> Result<Self> {
        self.client =
            credential_http_client_builder(self.profile.as_deref(), self.base_url_root())?
                .timeout(std::time::Duration::from_secs(75))
                .build()
                .context("Failed to build credential transfer client")?;
        self.refresh_disabled = true;
        Ok(self)
    }

    pub fn base_url_root(&self) -> &str {
        self.base_url
            .strip_suffix("/api/v1")
            .unwrap_or(&self.base_url)
    }

    pub fn is_agent_key_auth(&self) -> bool {
        self.agent_key_auth
    }

    fn reject_agent_key_unauthorized(&self, response: &reqwest::Response) -> Result<()> {
        if self.agent_key_auth && response.status() == reqwest::StatusCode::UNAUTHORIZED {
            return Err(crate::auth::ReauthRequired {
                code: "agent_key_rejected",
                message: crate::auth::agent_key::REJECTED_MESSAGE.into(),
            }
            .into());
        }
        Ok(())
    }

    /// Attempt to refresh the access token using the saved refresh token.
    /// Returns `true` if the token was refreshed successfully. Delegates the
    /// wire protocol to [`crate::auth::exchange_refresh_token`] (the single
    /// source of truth) and owns only the token I/O: read the saved refresh
    /// token, persist the rotated pair, and update this client's in-memory copy.
    async fn try_refresh_token(&mut self) -> bool {
        let client = self.client.clone();
        self.try_refresh_token_with_client(&client).await
    }

    async fn try_refresh_token_with_client(&mut self, client: &Client) -> bool {
        if self.refresh_disabled {
            return false;
        }
        let profile = self.profile.as_deref();
        // Same cross-process discipline as the preflight: lock, then prefer a
        // token another process already rotated over presenting a stale one.
        let Ok(_lock) = crate::auth::acquire_refresh_lock(profile) else {
            return false;
        };
        if self.login_generation != crate::auth::login_generation(profile)
            || crate::auth::validate_profile_destination(profile, self.base_url_root()).is_err()
        {
            return false;
        }
        if crate::auth::agent_key::is_agent_key_profile(profile) {
            return false;
        }
        if let Some(access_token) = crate::auth::fresh_access_token_on_disk(profile)
            && access_token != self.access_token
        {
            self.access_token = access_token;
            return true;
        }
        let refresh_token = match crate::auth::read_saved_refresh_token_for(profile) {
            Some(rt) => rt,
            None => return false,
        };

        match crate::auth::exchange_refresh_token(client, self.base_url_root(), &refresh_token)
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
            crate::auth::RefreshExchange::Rejected(_)
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

        self.reject_agent_key_unauthorized(&resp)?;
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

        self.reject_agent_key_unauthorized(&resp)?;
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

        self.reject_agent_key_unauthorized(&resp)?;
        if resp.status() == reqwest::StatusCode::UNAUTHORIZED
            && !self.write_refresh_disabled
            && self.try_refresh_token().await
        {
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

        self.reject_agent_key_unauthorized(&resp)?;
        if resp.status() == reqwest::StatusCode::UNAUTHORIZED
            && !self.write_refresh_disabled
            && self.try_refresh_token().await
        {
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

        self.reject_agent_key_unauthorized(&resp)?;
        if resp.status() == reqwest::StatusCode::UNAUTHORIZED
            && !self.write_refresh_disabled
            && self.try_refresh_token().await
        {
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
        self.delete_empty_with_headers(path, reqwest::header::HeaderMap::new())
            .await
    }

    pub async fn delete_empty_with_headers(
        &mut self,
        path: &str,
        headers: reqwest::header::HeaderMap,
    ) -> Result<()> {
        let url = format!("{}{path}", self.base_url);
        let resp = self
            .client
            .delete(&url)
            .bearer_auth(&self.access_token)
            .headers(headers.clone())
            .send()
            .await
            .with_context(|| format!("DELETE {path} failed"))?;

        self.reject_agent_key_unauthorized(&resp)?;
        if resp.status() == reqwest::StatusCode::UNAUTHORIZED
            && !self.write_refresh_disabled
            && self.try_refresh_token().await
        {
            let resp = self
                .client
                .delete(&url)
                .bearer_auth(&self.access_token)
                .headers(headers)
                .send()
                .await
                .with_context(|| format!("DELETE {path} failed (retry)"))?;
            if resp.status().is_success() {
                return Ok(());
            }
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(ApiError::new(format!("DELETE {path}"), status, body).into());
        }

        if resp.status().is_success() {
            Ok(())
        } else {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            Err(ApiError::new(format!("DELETE {path}"), status, body).into())
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

        self.reject_agent_key_unauthorized(&resp)?;
        if resp.status() == reqwest::StatusCode::UNAUTHORIZED
            && !self.write_refresh_disabled
            && self.try_refresh_token().await
        {
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

    pub async fn delete_with_body<T: DeserializeOwned, B: Serialize>(
        &mut self,
        path: &str,
        body: &B,
    ) -> Result<T> {
        let url = format!("{}{path}", self.base_url);
        let client = build_credential_http_client_for_url(self.profile.as_deref(), &url)?;
        let resp = client
            .delete(url.clone())
            .bearer_auth(&self.access_token)
            .json(body)
            .send()
            .await
            .with_context(|| format!("DELETE {path} failed"))?;
        self.reject_agent_key_unauthorized(&resp)?;
        if resp.status() == reqwest::StatusCode::UNAUTHORIZED
            && self.try_refresh_token_with_client(&client).await
        {
            let resp = client
                .delete(url)
                .bearer_auth(&self.access_token)
                .json(body)
                .send()
                .await
                .with_context(|| format!("DELETE {path} failed (retry)"))?;
            return Self::handle_response(resp, path).await;
        }
        Self::handle_response(resp, path).await
    }

    /// Delete an item whose API returns either legacy 204 or a JSON result.
    pub async fn delete_optional<T: DeserializeOwned>(&mut self, path: &str) -> Result<Option<T>> {
        let url = format!("{}{path}", self.base_url);
        let mut resp = self
            .client
            .delete(&url)
            .bearer_auth(&self.access_token)
            .send()
            .await
            .with_context(|| format!("DELETE {path} failed"))?;

        self.reject_agent_key_unauthorized(&resp)?;
        if resp.status() == reqwest::StatusCode::UNAUTHORIZED
            && !self.write_refresh_disabled
            && self.try_refresh_token().await
        {
            resp = self
                .client
                .delete(&url)
                .bearer_auth(&self.access_token)
                .send()
                .await
                .with_context(|| format!("DELETE {path} failed (retry)"))?;
        }
        if resp.status() == reqwest::StatusCode::NO_CONTENT {
            return Ok(None);
        }
        Self::handle_response(resp, path).await.map(Some)
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

        self.reject_agent_key_unauthorized(&resp)?;
        if resp.status() == reqwest::StatusCode::UNAUTHORIZED
            && !self.write_refresh_disabled
            && self.try_refresh_token().await
        {
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
            return Err(ApiError::new(format!("POST {path}"), status, body).into());
        }

        if resp.status().is_success() {
            Ok(())
        } else {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            Err(ApiError::new(format!("POST {path}"), status, body).into())
        }
    }

    /// Streamable HTTP MCP uses the root /mcp route. Calls never replay on 401.
    pub(crate) async fn mcp_request(
        &mut self,
        session: Option<&str>,
        version: &str,
        body: &serde_json::Value,
        read: bool,
    ) -> Result<reqwest::Response> {
        let response = self.mcp_request_once(session, version, body).await?;
        self.reject_agent_key_unauthorized(&response)?;
        if response.status() == reqwest::StatusCode::UNAUTHORIZED
            && read
            && self.try_refresh_token().await
        {
            return self.mcp_request_once(session, version, body).await;
        }
        Ok(response)
    }

    async fn mcp_request_once(
        &self,
        session: Option<&str>,
        version: &str,
        body: &serde_json::Value,
    ) -> Result<reqwest::Response> {
        let mut request = self
            .client
            .post(format!("{}/mcp", self.base_url_root()))
            .bearer_auth(&self.access_token)
            .header("Accept", "application/json, text/event-stream")
            .header("MCP-Protocol-Version", version)
            .json(body);
        if let Some(session) = session {
            request = request.header("Mcp-Session-Id", session);
        }
        request
            .send()
            .await
            .context("MCP request failed; inspect execution state before retrying a call")
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

        self.reject_agent_key_unauthorized(&resp)?;
        if resp.status() == reqwest::StatusCode::UNAUTHORIZED && !self.refresh_disabled {
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
            Err(ApiError::new(path, status, body).into())
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
    let client = build_credential_http_client_for_url(profile, &url)?;

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
        Err(ApiError::new(path, status, body).into())
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
    let client = build_credential_http_client_for_url(profile, &url)?;

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
        Err(ApiError::new(path, status, body).into())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;
    use std::ffi::OsString;
    use std::io;
    use std::sync::Mutex;

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::{TcpListener, TcpStream};
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::{ApiClient, ApiError};
    use crate::cli::{AuthArgs, OutputFormat};

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

    struct EnvGuard {
        previous: Vec<(&'static str, Option<OsString>)>,
    }

    impl EnvGuard {
        fn set(values: &[(&'static str, Option<&str>)]) -> Self {
            let previous = values
                .iter()
                .map(|(name, _)| (*name, std::env::var_os(name)))
                .collect();
            // SAFETY: tests using EnvGuard hold the process-wide env_lock.
            unsafe {
                for (name, value) in values {
                    match value {
                        Some(value) => std::env::set_var(name, value),
                        None => std::env::remove_var(name),
                    }
                }
            }
            Self { previous }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            // SAFETY: tests using EnvGuard hold the process-wide env_lock.
            unsafe {
                for (name, value) in &self.previous {
                    match value {
                        Some(value) => std::env::set_var(name, value),
                        None => std::env::remove_var(name),
                    }
                }
            }
        }
    }

    fn auth_args(base_url: String) -> AuthArgs {
        AuthArgs {
            base_url: Some(base_url),
            access_token: None,
            access_token_env: "NYXID_ACCESS_TOKEN".to_string(),
            profile: None,
            output: OutputFormat::Json,
        }
    }

    #[test]
    fn credential_request_urls_allow_https_and_only_exact_loopback_http() {
        for address in [
            "https://service.example/api/v1/service-preferences/hidden",
            "http://localhost:3001/api/v1/service-preferences/hidden",
            "http://127.0.0.1:3001/api/v1/service-preferences/hidden",
            "http://[::1]:3001/api/v1/service-preferences/hidden",
        ] {
            assert!(super::credential_request_url(address).is_ok(), "{address}");
            assert!(
                ApiClient::new(address, uuid::Uuid::new_v4().to_string()).is_ok(),
                "{address}"
            );
        }
        for address in [
            "http://service.example/hidden",
            "http://192.168.1.1/hidden",
            "http://127.0.0.2/hidden",
            "http://localhost.service.example/hidden",
            "http://127.0.0.1.service.example/hidden",
            "http://[::ffff:127.0.0.1]/hidden",
            "https://user:dummy@service.example/hidden",
            "http://localhost@service.example/hidden",
            "https://service.example/hidden#fragment",
            "file:///hidden",
            "ftp://localhost/hidden",
            "/api/v1/hidden",
            "//service.example/hidden",
            "service.example/hidden",
        ] {
            assert!(super::credential_request_url(address).is_err(), "{address}");
            assert!(
                ApiClient::new(address, uuid::Uuid::new_v4().to_string()).is_err(),
                "{address}"
            );
        }
    }

    #[tokio::test]
    async fn explicit_credentials_cannot_bypass_transport_validation() {
        for address in [
            "http://service.example",
            "http://127.0.0.2",
            "https://user:dummy@service.example",
        ] {
            let mut auth = auth_args(address.to_string());
            auth.access_token = Some(uuid::Uuid::new_v4().to_string());
            assert!(ApiClient::from_auth(&auth).is_err(), "{address}");
            assert!(
                ApiClient::from_auth_checked(&auth).await.is_err(),
                "{address}"
            );
        }
    }

    #[tokio::test]
    async fn api_paths_cannot_override_the_validated_destination() {
        let origin = MockServer::start().await;
        let target = MockServer::start().await;
        let paths = [
            "/transport-fixture".to_string(),
            "relative-fixture".to_string(),
            format!("//{}/transport-fixture", target.address()),
            format!("{}/transport-fixture", target.uri()),
            format!("/../../..//{}/transport-fixture", target.address()),
        ];
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
            .expect(paths.len() as u64)
            .mount(&origin)
            .await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
            .expect(0)
            .mount(&target)
            .await;
        let mut api = ApiClient::new(
            &format!("{}/prefix", origin.uri()),
            uuid::Uuid::new_v4().to_string(),
        )
        .unwrap();
        for route in paths {
            api.get_value(&route).await.unwrap();
        }
        origin.verify().await;
        target.verify().await;
    }

    #[tokio::test]
    async fn api_and_credential_transfer_refuse_body_preserving_redirects() {
        let origin = MockServer::start().await;
        let target = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/api/v1/transport-fixture"))
            .respond_with(ResponseTemplate::new(307).insert_header("Location", target.uri()))
            .expect(2)
            .mount(&origin)
            .await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
            .expect(0)
            .mount(&target)
            .await;
        for transfer in [false, true] {
            let mut api = ApiClient::new(&origin.uri(), uuid::Uuid::new_v4().to_string()).unwrap();
            if transfer {
                api = api.for_credential_transfer().unwrap();
            }
            let error = api
                .post::<serde_json::Value, _>(
                    "/transport-fixture",
                    &serde_json::json!({
                        "credential": uuid::Uuid::new_v4().to_string(),
                    }),
                )
                .await
                .expect_err("credential-bearing requests must not redirect");
            assert_eq!(
                error.downcast_ref::<ApiError>().unwrap().status(),
                reqwest::StatusCode::TEMPORARY_REDIRECT
            );
        }
        origin.verify().await;
        target.verify().await;
    }

    #[tokio::test]
    async fn loopback_api_and_credential_transfer_bypass_environment_proxies() {
        const CHILD_ORIGIN: &str = "NYXID_TEST_LOOPBACK_PROXY_ORIGIN";
        const TEST_NAME: &str =
            "api::tests::loopback_api_and_credential_transfer_bypass_environment_proxies";

        if let Some(address) = std::env::var_os(CHILD_ORIGIN) {
            let address = address.into_string().expect("child origin must be UTF-8");
            for transfer in [false, true] {
                let mut api = ApiClient::new(&address, uuid::Uuid::new_v4().to_string()).unwrap();
                if transfer {
                    api = api.for_credential_transfer().unwrap();
                }
                assert_eq!(
                    api.get_value("/transport-fixture").await.unwrap(),
                    serde_json::json!({"direct": true})
                );
            }
            return;
        }

        let origin = MockServer::start().await;
        let proxy = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/transport-fixture"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"direct": true})),
            )
            .expect(2)
            .mount(&origin)
            .await;
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(502))
            .expect(0)
            .mount(&proxy)
            .await;

        // Other HTTP tests read proxy variables without taking env_lock. Set
        // them only in a child running this exact test, so parallel clients
        // cannot accidentally send their requests to this test's proxy.
        let home = tempfile::tempdir().expect("child home");
        let address = origin.uri().replace("127.0.0.1", "localhost");
        let mut child = tokio::process::Command::new(std::env::current_exe().unwrap());
        child
            .args(["--exact", TEST_NAME, "--nocapture", "--test-threads=1"])
            .env(CHILD_ORIGIN, &address)
            .env("HOME", home.path())
            .env("HTTP_PROXY", proxy.uri())
            .env("http_proxy", proxy.uri())
            .env("HTTPS_PROXY", proxy.uri())
            .env("https_proxy", proxy.uri())
            .env("ALL_PROXY", proxy.uri())
            .env("all_proxy", proxy.uri())
            .env("NO_PROXY", "")
            .env("no_proxy", "")
            .stdin(std::process::Stdio::null())
            .kill_on_drop(true);
        let output = tokio::time::timeout(std::time::Duration::from_secs(30), child.output())
            .await
            .expect("proxy isolation child timed out")
            .expect("failed to run proxy isolation child");
        assert!(
            output.status.success(),
            "proxy isolation child failed: {}\n{}\n{}",
            output.status,
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        origin.verify().await;
        proxy.verify().await;
        assert!(
            origin
                .received_requests()
                .await
                .unwrap()
                .iter()
                .all(|request| {
                    request
                        .headers
                        .get("authorization")
                        .and_then(|value| value.to_str().ok())
                        .is_some_and(|value| value.starts_with("Bearer ") && value.len() > 7)
                })
        );
        assert!(proxy.received_requests().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn anonymous_auth_posts_reject_remote_cleartext_before_sending() {
        let body = serde_json::json!({});
        let error = super::anonymous_post(
            "http://cleartext-fixture.invalid",
            "/auth/register",
            &body,
            None,
        )
        .await
        .expect_err("registration requires secure transport");
        assert!(error.to_string().contains("requires HTTPS"));
        let error = super::anonymous_post_empty(
            "http://cleartext-fixture.invalid",
            "/auth/reset-password",
            &body,
            None,
        )
        .await
        .expect_err("password reset requires secure transport");
        assert!(error.to_string().contains("requires HTTPS"));
    }

    #[tokio::test]
    async fn non_success_response_preserves_typed_error_details() {
        let server = MockServer::start().await;
        let body = serde_json::json!({
            "error": "grant_cascade_confirmation_required",
            "error_code": 11500,
            "message": "Grant cascade confirmation required",
            "details": {
                "provider_name": "GitHub",
                "siblings": [{ "name": "GitHub Issues" }],
                "unaffected_other_app": [],
                "token_scope_available": true
            }
        });
        Mock::given(method("DELETE"))
            .and(path("/api/v1/keys/service-1"))
            .respond_with(ResponseTemplate::new(409).set_body_json(&body))
            .mount(&server)
            .await;

        let mut api = ApiClient::new(&server.uri(), "test-token".to_string()).unwrap();
        let error = api
            .delete_empty("/keys/service-1")
            .await
            .expect_err("409 should be an error");
        let api_error = error.downcast_ref::<ApiError>().expect("typed API error");

        assert_eq!(api_error.status(), reqwest::StatusCode::CONFLICT);
        assert_eq!(
            api_error.response().map(|response| response.error_code),
            Some(11500)
        );
        assert_eq!(
            api_error
                .response()
                .and_then(|response| response.details.as_ref())
                .and_then(|details| details["provider_name"].as_str()),
            Some("GitHub")
        );
        assert_eq!(api_error.body_json(), Some(&body));
    }

    async fn assert_override_is_fail_closed(auth: &AuthArgs, expected_token: &str) {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/proxy/s/test/ping"))
            .and(header("authorization", format!("Bearer {expected_token}")))
            .respond_with(ResponseTemplate::new(401).set_body_string("rejected override"))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/v1/auth/refresh"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token": "stored-session-replacement",
                "refresh_token": "rotated-refresh-token"
            })))
            .expect(0)
            .mount(&server)
            .await;

        let mut auth = auth.clone();
        auth.base_url = Some(server.uri());
        let mut api = ApiClient::from_auth_checked(&auth)
            .await
            .expect("caller-selected credential should build a client");
        let resp = api
            .proxy_request("GET", "/proxy/s/test/ping", &[], None)
            .await
            .expect("401 response should be returned without identity substitution");

        assert_eq!(resp.status(), reqwest::StatusCode::UNAUTHORIZED);
        assert_eq!(
            resp.text().await.expect("response body"),
            "rejected override"
        );
        server.verify().await;
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)] // intentional: serialises HOME/env mutations across tests
    async fn caller_selected_credentials_never_refresh_or_switch_identity() {
        let _env_guard = env_lock().lock().unwrap_or_else(|err| err.into_inner());
        let temp = tempfile::tempdir().expect("temp dir");
        let _home = HomeGuard::set(temp.path());
        let _env = EnvGuard::set(&[
            ("NYXID_ACCESS_TOKEN", Some("default-env-token")),
            ("NYXID_API_KEY", Some("api-key-token")),
            ("NYXID_TEST_SELECTED_TOKEN", Some("named-env-token")),
        ]);

        crate::auth::save_tokens("stored-access-token", Some("stored-refresh-token"))
            .expect("save stored session");

        let mut explicit = auth_args(String::new());
        explicit.access_token = Some("explicit-token".to_string());
        assert_override_is_fail_closed(&explicit, "explicit-token").await;

        let mut named_env = auth_args(String::new());
        named_env.access_token_env = "NYXID_TEST_SELECTED_TOKEN".to_string();
        assert_override_is_fail_closed(&named_env, "named-env-token").await;

        let default_env = auth_args(String::new());
        assert_override_is_fail_closed(&default_env, "default-env-token").await;

        // SAFETY: serialized by env_lock and restored by EnvGuard.
        unsafe {
            std::env::remove_var("NYXID_ACCESS_TOKEN");
        }
        let api_key_alias = auth_args(String::new());
        assert_override_is_fail_closed(&api_key_alias, "api-key-token").await;
    }

    #[tokio::test]
    #[allow(clippy::await_holding_lock)] // intentional: serialises HOME mutations across tests
    async fn proxy_request_refreshes_expired_token_and_retries() {
        let _env_guard = env_lock().lock().unwrap_or_else(|err| err.into_inner());
        let temp = tempfile::tempdir().expect("temp dir");
        let _home = HomeGuard::set(temp.path());

        crate::auth::save_tokens("expired-access-token", Some("valid-refresh-token"))
            .expect("save initial tokens");

        let base_url = start_proxy_refresh_test_server()
            .await
            .expect("test server");
        let _env = EnvGuard::set(&[("NYXID_TEST_SAVED_SESSION_TOKEN", None)]);
        let mut auth = auth_args(base_url);
        auth.access_token_env = "NYXID_TEST_SAVED_SESSION_TOKEN".to_string();
        let mut api = ApiClient::from_auth_checked(&auth)
            .await
            .expect("saved-session client");

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
        let _env_guard = env_lock().lock().unwrap_or_else(|err| err.into_inner());
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

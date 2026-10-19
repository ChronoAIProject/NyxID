use std::time::Duration;

use async_trait::async_trait;
use reqwest::{Client, redirect::Policy as RedirectPolicy};
use url::Url;
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::{Error, MAX_RESPONSE_BYTES, Request, Response, Transport};

/// Server-held upstream authority. Each instance is pinned to one NyxID
/// catalog service AND one exact UserService connection. No caller headers or
/// caller-selected URLs are accepted. Deliberately has no Debug implementation.
pub struct NyxIdTransport {
    client: Client,
    base_url: Url,
    service_id: Uuid,
    connection_id: Uuid,
    token: Zeroizing<String>,
}

impl NyxIdTransport {
    pub fn new(
        base_url: &str,
        service_id: Uuid,
        connection_id: Uuid,
        token: Zeroizing<String>,
    ) -> Result<Self, Error> {
        let base_url = Url::parse(base_url).map_err(|_| Error::Policy("invalid NyxID URL"))?;
        let loopback = base_url.host_str().is_some_and(|host| {
            host == "localhost"
                || host
                    .parse::<std::net::IpAddr>()
                    .is_ok_and(|ip| ip.is_loopback())
        });
        if !(base_url.scheme() == "https" || base_url.scheme() == "http" && loopback)
            || base_url.host_str().is_none()
            || !base_url.username().is_empty()
            || base_url.password().is_some()
            || base_url.query().is_some()
            || base_url.fragment().is_some()
            || base_url.path() != "/"
        {
            return Err(Error::Policy(
                "NyxID URL must be an HTTPS origin (HTTP allowed only on loopback)",
            ));
        }
        if token.is_empty() || token.chars().any(char::is_control) {
            return Err(Error::Policy("upstream key is missing or malformed"));
        }
        let client = Client::builder()
            .redirect(RedirectPolicy::none())
            .connect_timeout(Duration::from_secs(5))
            .timeout(Duration::from_secs(15))
            .no_proxy()
            .build()
            .map_err(|_| Error::Upstream)?;
        Ok(Self {
            client,
            base_url,
            service_id,
            connection_id,
            token,
        })
    }
}

#[async_trait]
impl Transport for NyxIdTransport {
    async fn send(&self, request: &Request) -> Result<Response, Error> {
        if !request.path.starts_with("/drive/v3/files")
            || request.path.contains(['%', '?', '#', '\\'])
            || request.path.split('/').any(|s| s == "." || s == "..")
            || request.query.contains_key("_nyxid_via")
        {
            return Err(Error::Denied("invalid prepared request"));
        }
        let mut url = self.base_url.clone();
        url.set_path(&format!(
            "/api/v1/proxy/{}{}",
            self.service_id, request.path
        ));
        url.query_pairs_mut()
            .extend_pairs(&request.query)
            .append_pair("_nyxid_via", &self.connection_id.to_string());
        let mut builder = self
            .client
            .request(request.method.clone(), url)
            .bearer_auth(self.token.as_str())
            .header("Accept-Encoding", "identity");
        if !request.body.is_empty() {
            builder = builder
                .header("Content-Type", "application/json")
                .body(request.body.clone());
        }
        let mut response = builder.send().await.map_err(|_| Error::Upstream)?;
        if response.status().is_redirection() {
            return Err(Error::Upstream);
        }
        if response
            .headers()
            .get("content-encoding")
            .is_some_and(|v| v != "identity")
        {
            return Err(Error::Upstream);
        }
        if response
            .content_length()
            .is_some_and(|n| n > MAX_RESPONSE_BYTES as u64)
        {
            return Err(Error::ResponseTooLarge);
        }
        let status = response.status().as_u16();
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("application/octet-stream")
            .to_owned();
        let mut body = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| Error::Upstream)? {
            if body.len().saturating_add(chunk.len()) > MAX_RESPONSE_BYTES {
                return Err(Error::ResponseTooLarge);
            }
            body.extend_from_slice(&chunk);
        }
        Ok(Response {
            status,
            content_type,
            body: body.into(),
        })
    }
}

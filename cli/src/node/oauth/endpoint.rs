use url::Url;

use crate::node::error::{Error, Result};

/// A validated destination and a client that enforces its transport policy.
pub(super) struct ProviderEndpoint {
    address: Url,
    client: reqwest::Client,
}

impl ProviderEndpoint {
    pub(super) fn parse_address(address: &str) -> Result<Url> {
        let address = Url::parse(address)
            .map_err(|_| Error::Config("OAuth endpoint must be a valid URL".into()))?;
        if address.host().is_none()
            || !address.username().is_empty()
            || address.password().is_some()
            || address.fragment().is_some()
        {
            return Err(Error::Config(
                "OAuth endpoint must have a host and must not contain userinfo or a fragment"
                    .into(),
            ));
        }
        if address.scheme() != "https"
            && !(address.scheme() == "http"
                && matches!(
                    address.host_str(),
                    Some("localhost" | "127.0.0.1" | "[::1]")
                ))
        {
            return Err(Error::Config(
                "OAuth endpoint must use HTTPS; HTTP is allowed only for localhost, 127.0.0.1, or [::1]".into(),
            ));
        }
        Ok(address)
    }

    pub(super) fn new(address: &str) -> Result<Self> {
        let address = Self::parse_address(address)?;
        let local_http = address.scheme() == "http";
        let mut builder = reqwest::Client::builder()
            .https_only(!local_http)
            .redirect(reqwest::redirect::Policy::none())
            .timeout(std::time::Duration::from_secs(30));
        if local_http {
            // A local development secret must never travel through an HTTP proxy
            // or a DNS/hosts override for localhost.
            builder = builder
                .no_proxy()
                .resolve("localhost", std::net::SocketAddr::from(([127, 0, 0, 1], 0)));
        }
        let client = builder
            .build()
            .map_err(|_| Error::Config("Failed to build OAuth endpoint client".into()))?;
        Ok(Self { address, client })
    }

    pub(super) fn post(&self) -> reqwest::RequestBuilder {
        self.client.post(self.address.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    #[test]
    fn endpoints_require_https_except_exact_loopback_hosts() {
        for address in [
            "https://provider.example/token",
            "https://provider.example/revoke",
            "http://localhost:1234/token",
            "http://127.0.0.1:1234/token",
            "http://[::1]:1234/revoke",
        ] {
            assert!(ProviderEndpoint::new(address).is_ok(), "{address}");
        }
        for address in [
            "http://provider.example/token",
            "http://192.168.1.1/token",
            "http://127.0.0.2/token",
            "http://localhost.provider.example/token",
            "http://127.0.0.1.provider.example/token",
            "http://[::ffff:127.0.0.1]/token",
            "ftp://localhost/token",
            "file:///token",
            "https://user:secret@provider.example/token",
            "http://localhost@provider.example/token",
            "https://provider.example/token#fragment",
        ] {
            assert!(ProviderEndpoint::new(address).is_err(), "{address}");
        }
    }

    #[tokio::test]
    async fn localhost_is_pinned_and_redirects_do_not_receive_credentials() {
        let origin = MockServer::start().await;
        let target = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(307).insert_header("location", target.uri()))
            .expect(1)
            .mount(&origin)
            .await;
        let address = format!("http://localhost:{}/token", origin.address().port());
        let response = ProviderEndpoint::new(&address)
            .unwrap()
            .post()
            .basic_auth("test-client", Some("test-secret"))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::TEMPORARY_REDIRECT);
        assert!(target.received_requests().await.unwrap().is_empty());
        origin.verify().await;
    }
}

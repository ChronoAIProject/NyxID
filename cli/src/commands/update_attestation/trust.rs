//! Sigstore's public constructor hides its TUF HTTP client. Supply our transport
//! to tough directly, retaining TUF signatures, expiry, hashes and length checks.
use anyhow::{Context, Result};
use futures::TryStreamExt;
use sigstore::trust::sigstore::SigstoreTrustRoot;
use tough::{IntoVec, Transport, TransportError, TransportErrorKind};

const ROOT: &[u8] = include_bytes!("../../../resources/sigstore-root.json");
const METADATA: &str = "https://tuf-repo-cdn.sigstore.dev/";
const TARGETS: &str = "https://tuf-repo-cdn.sigstore.dev/targets/";

pub(super) async fn load() -> Result<SigstoreTrustRoot> {
    // Match tough's timeouts and default headers. In particular, never reuse
    // github_client: it may carry a GitHub token in its default Authorization.
    let client = crate::tls::client_builder()?
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(30))
        .build()?;
    load_from(client, METADATA.parse()?, TARGETS.parse()?).await
}

pub(super) async fn load_github(client: reqwest::Client) -> Result<SigstoreTrustRoot> {
    let base = "https://raw.githubusercontent.com/sigstore/root-signing/main/";
    let repository = tough::RepositoryLoader::new(
        &ROOT,
        format!("{base}metadata/").parse()?,
        format!("{base}targets/").parse()?,
    )
    .transport(GithubTransport(CliTransport(client)))
    .expiration_enforcement(tough::ExpirationEnforcement::Safe)
    .load()
    .await?;
    let bytes = repository
        .read_target(&tough::TargetName::new("trusted_root.json")?)
        .await?
        .context("Missing authenticated trust root")?
        .into_vec()
        .await?;
    SigstoreTrustRoot::from_trusted_root_json_unchecked(&bytes)
        .context("Invalid authenticated trust root")
}

#[derive(Clone, Debug)]
struct GithubTransport(CliTransport);
#[tough::async_trait]
impl Transport for GithubTransport {
    async fn fetch(
        &self,
        mut url: url::Url,
    ) -> Result<
        futures::stream::BoxStream<'static, Result<tough::Bytes, TransportError>>,
        TransportError,
    > {
        // The Git mirror stores versioned roots in root_history and current
        // metadata/targets without consistent-snapshot prefixes. tough still
        // checks every version, length, hash, expiry and signature.
        let path = url.path().to_owned();
        let file = path.rsplit('/').next().unwrap_or_default();
        if path.contains("/metadata/") && file.ends_with(".root.json") {
            url.set_path(&path.replace("/metadata/", "/metadata/root_history/"));
        } else if path.contains("/metadata/") {
            if let Some((prefix, rest)) = file.split_once('.')
                && prefix.bytes().all(|b| b.is_ascii_digit())
            {
                url.set_path(&format!("{}{rest}", &path[..path.len() - file.len()]));
            }
        } else if let Some((prefix, rest)) = file.split_once('.')
            && prefix.len() == 64
            && prefix.bytes().all(|b| b.is_ascii_hexdigit())
        {
            url.set_path(&format!("{}{rest}", &path[..path.len() - file.len()]));
        }
        self.0.fetch(url).await
    }
}

async fn load_from(
    client: reqwest::Client,
    metadata: url::Url,
    targets: url::Url,
) -> Result<SigstoreTrustRoot> {
    let repository = tough::RepositoryLoader::new(&ROOT, metadata, targets)
        .transport(CliTransport(client))
        .expiration_enforcement(tough::ExpirationEnforcement::Safe)
        .load()
        .await
        .context("Failed to verify Sigstore TUF metadata")?;
    let target = tough::TargetName::new("trusted_root.json")?;
    let data = repository
        .read_target(&target)
        .await?
        .context("Sigstore TUF repository has no trusted_root.json")?
        .into_vec()
        .await?;
    // The unchecked constructor only parses: tough has authenticated these bytes
    // against the signed target metadata above, rooted in the embedded TUF root.
    SigstoreTrustRoot::from_trusted_root_json_unchecked(&data)
        .context("Invalid authenticated Sigstore trust root")
}

#[derive(Clone, Debug)]
struct CliTransport(reqwest::Client);

#[tough::async_trait]
impl Transport for CliTransport {
    async fn fetch(
        &self,
        url: url::Url,
    ) -> Result<
        futures::stream::BoxStream<'static, Result<tough::Bytes, TransportError>>,
        TransportError,
    > {
        let mut download = Download {
            client: self.0.clone(),
            url,
            stream: None,
            next_byte: 0,
            accepts_ranges: false,
            retries: 0,
        };
        download.open().await?;
        Ok(Box::pin(futures::stream::try_unfold(
            download,
            |mut download| async move {
                loop {
                    match download
                        .stream
                        .as_mut()
                        .expect("open download")
                        .try_next()
                        .await
                    {
                        Ok(Some(bytes)) => {
                            download.next_byte += bytes.len();
                            return Ok(Some((bytes, download)));
                        }
                        Ok(None) => return Ok(None),
                        Err(error) => {
                            download.retry(error).await?;
                            download.open().await?;
                        }
                    }
                }
            },
        )))
    }
}

// Preserve tough 0.21's four retries, 100ms backoff multiplied by 1.5 (capped
// at 1s), and range-based recovery. TUF still bounds and authenticates the stream.
struct Download {
    client: reqwest::Client,
    url: url::Url,
    stream: Option<futures::stream::BoxStream<'static, reqwest::Result<tough::Bytes>>>,
    next_byte: usize,
    accepts_ranges: bool,
    retries: u32,
}

impl Download {
    async fn open(&mut self) -> Result<(), TransportError> {
        loop {
            let mut request = self.client.get(self.url.clone());
            if self.next_byte > 0 {
                request =
                    request.header(reqwest::header::RANGE, format!("bytes={}-", self.next_byte));
            }
            match request
                .send()
                .await
                .and_then(reqwest::Response::error_for_status)
            {
                Ok(response) => {
                    if self.next_byte > 0
                        && response.status() != reqwest::StatusCode::PARTIAL_CONTENT
                    {
                        return Err(TransportError::new(
                            TransportErrorKind::Other,
                            self.url.as_str(),
                        ));
                    }
                    self.accepts_ranges |= response
                        .headers()
                        .get(reqwest::header::ACCEPT_RANGES)
                        .and_then(|value| value.to_str().ok())
                        .is_some_and(|value| value.contains("bytes"));
                    self.stream = Some(Box::pin(response.bytes_stream()));
                    return Ok(());
                }
                Err(error) => self.retry(error).await?,
            }
        }
    }

    async fn retry(&mut self, error: reqwest::Error) -> Result<(), TransportError> {
        let missing = matches!(error.status().map(|s| s.as_u16()), Some(403 | 404 | 410));
        let retryable = error.is_timeout()
            || error.is_request()
            || error
                .status()
                .is_some_and(|status| status.is_server_error());
        if missing
            || !retryable
            || self.retries == 4
            || (self.next_byte > 0 && !self.accepts_ranges)
        {
            return Err(TransportError::new_with_cause(
                if missing {
                    TransportErrorKind::FileNotFound
                } else {
                    TransportErrorKind::Other
                },
                self.url.as_str(),
                error.without_url(),
            ));
        }
        let wait = std::time::Duration::from_millis(100)
            .mul_f32(1.5_f32.powi(self.retries as i32))
            .min(std::time::Duration::from_secs(1));
        self.retries += 1;
        tokio::time::sleep(wait).await;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    #[ignore = "Live GitHub TUF mirror; explicit release validation"]
    async fn github_tuf_mirror_verifies_root_rotation_expiry_and_target_hash() {
        let client = reqwest::Client::builder()
            .https_only(true)
            .no_proxy()
            .user_agent("nyxid-update-validation")
            .build()
            .unwrap();
        load_github(client).await.unwrap();
    }

    #[tokio::test]
    async fn tuf_loader_rejects_unauthenticated_metadata_before_loading_trust_root() {
        use wiremock::{
            Mock, MockServer, ResponseTemplate,
            matchers::{method, path},
        };
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"signed": {}, "signatures": []})),
            )
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(path("/targets/trusted_root.json"))
            .respond_with(ResponseTemplate::new(200).set_body_string("untrusted"))
            .expect(0)
            .mount(&server)
            .await;
        let client = crate::tls::client_builder()
            .unwrap()
            .no_proxy()
            .build()
            .unwrap();
        assert!(
            load_from(
                client,
                format!("{}/", server.uri()).parse().unwrap(),
                format!("{}/targets/", server.uri()).parse().unwrap()
            )
            .await
            .is_err()
        );
        server.verify().await;
    }

    #[tokio::test]
    async fn tuf_transport_uses_cli_ca_roots() {
        let server = crate::tls::tests::Server::start("localhost", false).await;
        let ca = server.ca_file();
        let trust = crate::tls::tests::configured(Some(ca.path().into()));
        let transport = CliTransport(crate::tls::tests::client(&trust));
        assert_eq!(
            transport
                .fetch(server.url().parse().unwrap())
                .await
                .unwrap()
                .into_vec()
                .await
                .unwrap(),
            b"{}"
        );
        let untrusted = CliTransport(crate::tls::tests::client(&crate::tls::tests::configured(
            None,
        )));
        assert!(
            untrusted
                .fetch(server.url().parse().unwrap())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn tuf_transport_preserves_missing_metadata_statuses() {
        use wiremock::{Mock, MockServer, ResponseTemplate};
        for status in [403, 404, 410, 502] {
            let server = MockServer::start().await;
            Mock::given(wiremock::matchers::method("GET"))
                .respond_with(ResponseTemplate::new(status))
                .mount(&server)
                .await;
            let transport = CliTransport(
                crate::tls::client_builder()
                    .unwrap()
                    .no_proxy()
                    .build()
                    .unwrap(),
            );
            let error = match transport.fetch(server.uri().parse().unwrap()).await {
                Ok(_) => panic!("expected HTTP error"),
                Err(error) => error,
            };
            assert!(matches!(error.kind(), TransportErrorKind::FileNotFound) == (status != 502));
        }
    }
    #[tokio::test]
    async fn tuf_transport_retries_server_errors_with_a_bounded_budget() {
        use std::sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        };
        use wiremock::{Mock, MockServer, ResponseTemplate};
        for permanent in [false, true] {
            let server = MockServer::start().await;
            let count = Arc::new(AtomicUsize::new(0));
            Mock::given(wiremock::matchers::method("GET"))
                .respond_with({
                    let count = count.clone();
                    move |_: &wiremock::Request| {
                        if count.fetch_add(1, Ordering::SeqCst) == 0 || permanent {
                            ResponseTemplate::new(503)
                        } else {
                            ResponseTemplate::new(200).set_body_string("verified downstream")
                        }
                    }
                })
                .mount(&server)
                .await;
            let transport = CliTransport(
                crate::tls::client_builder()
                    .unwrap()
                    .no_proxy()
                    .build()
                    .unwrap(),
            );
            let result = transport.fetch(server.uri().parse().unwrap()).await;
            if permanent {
                assert!(result.is_err());
                assert_eq!(count.load(Ordering::SeqCst), 5);
            } else {
                assert_eq!(
                    result.unwrap().into_vec().await.unwrap(),
                    b"verified downstream"
                );
                assert_eq!(count.load(Ordering::SeqCst), 2);
            }
        }
    }

    #[tokio::test]
    async fn tuf_transport_resumes_timed_out_stream_at_the_next_byte() {
        use tokio::{
            io::{AsyncReadExt, AsyncWriteExt},
            net::TcpListener,
        };
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut first, _) = listener.accept().await.unwrap();
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                request.push(first.read_u8().await.unwrap());
            }
            first
                .write_all(
                    b"HTTP/1.1 200 OK\r\nContent-Length: 6\r\nAccept-Ranges: bytes\r\n\r\nab",
                )
                .await
                .unwrap();
            // Keep the first body open until the client's request timeout fires.
            let (mut next, _) = listener.accept().await.unwrap();
            request.clear();
            while !request.ends_with(b"\r\n\r\n") {
                request.push(next.read_u8().await.unwrap());
            }
            assert!(
                String::from_utf8(request.clone())
                    .unwrap()
                    .to_ascii_lowercase()
                    .contains("range: bytes=2-")
            );
            next.write_all(b"HTTP/1.1 206 Partial Content\r\nContent-Length: 4\r\nContent-Range: bytes 2-5/6\r\n\r\ncd").await.unwrap();
            // A partial response need not repeat Accept-Ranges. A second timeout
            // must retain the original range capability and accumulated offset.
            let (mut last, _) = listener.accept().await.unwrap();
            request.clear();
            while !request.ends_with(b"\r\n\r\n") {
                request.push(last.read_u8().await.unwrap());
            }
            assert!(
                String::from_utf8(request)
                    .unwrap()
                    .to_ascii_lowercase()
                    .contains("range: bytes=4-")
            );
            last.write_all(b"HTTP/1.1 206 Partial Content\r\nContent-Length: 2\r\nContent-Range: bytes 4-5/6\r\n\r\nef").await.unwrap();
        });
        let transport = CliTransport(
            crate::tls::client_builder()
                .unwrap()
                .no_proxy()
                .timeout(std::time::Duration::from_millis(200))
                .build()
                .unwrap(),
        );
        assert_eq!(
            transport
                .fetch(format!("http://{address}/target").parse().unwrap())
                .await
                .unwrap()
                .into_vec()
                .await
                .unwrap(),
            b"abcdef"
        );
        server.await.unwrap();
    }
}

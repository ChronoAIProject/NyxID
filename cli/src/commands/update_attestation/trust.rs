//! Sigstore's public constructor hides its TUF HTTP client. Supply our transport
//! to tough directly, retaining TUF signatures, expiry, hashes and length checks.
use anyhow::{Context, Result};
use futures::TryStreamExt;
use sigstore::trust::sigstore::SigstoreTrustRoot;
use tough::{IntoVec, Transport, TransportError, TransportErrorKind};

#[path = "cache.rs"]
mod cache;

/// Only constructed after TUF authentication (or from pinned test fixtures).
/// Retired log keys remain available for historical verification. The upstream
/// accessor filters them by wall-clock time rather than the signed event time.
#[derive(Debug)]
pub(super) struct Root {
    fulcio: Vec<Authority>,
    rekor: Vec<LogKey>,
    ctfe: Vec<LogKey>,
}

#[derive(Debug)]
struct Authority {
    certs: Vec<Vec<u8>>,
    start: Option<chrono::DateTime<chrono::Utc>>,
    end: Option<chrono::DateTime<chrono::Utc>>,
}

type ValidityInterval = (
    Option<chrono::DateTime<chrono::Utc>>,
    Option<chrono::DateTime<chrono::Utc>>,
);

fn validity(value: &serde_json::Value) -> Result<ValidityInterval> {
    anyhow::ensure!(
        value.is_null() || value.is_object(),
        "Invalid trust validity interval"
    );
    let date = |field: &str| -> Result<Option<chrono::DateTime<chrono::Utc>>> {
        match &value[field] {
            serde_json::Value::Null => Ok(None),
            serde_json::Value::String(s) => Ok(Some(
                chrono::DateTime::parse_from_rfc3339(s)?.with_timezone(&chrono::Utc),
            )),
            _ => anyhow::bail!("Invalid trust validity timestamp"),
        }
    };
    let (start, end) = (date("start")?, date("end")?);
    anyhow::ensure!(
        start.zip(end).is_none_or(|(s, e)| s <= e),
        "Reversed trust validity interval"
    );
    Ok((start, end))
}

#[derive(Debug)]
pub(super) struct LogKey {
    pub id: String,
    pub der: Vec<u8>,
    pub origin: String,
    start: Option<chrono::DateTime<chrono::Utc>>,
    end: Option<chrono::DateTime<chrono::Utc>>,
}

impl LogKey {
    pub fn check_time(&self, seconds: u64) -> Result<()> {
        let time = i64::try_from(seconds)
            .ok()
            .and_then(|s| chrono::DateTime::from_timestamp(s, 0))
            .context("Log timestamp out of range")?;
        self.check_datetime(time)
    }

    pub fn check_millis(&self, millis: u64) -> Result<()> {
        let time = i64::try_from(millis)
            .ok()
            .and_then(chrono::DateTime::from_timestamp_millis)
            .context("Log timestamp out of range")?;
        self.check_datetime(time)
    }

    fn check_datetime(&self, time: chrono::DateTime<chrono::Utc>) -> Result<()> {
        anyhow::ensure!(
            self.start.is_none_or(|s| time >= s) && self.end.is_none_or(|e| time <= e),
            "Log key was not valid at signed event time"
        );
        Ok(())
    }
}

impl Root {
    pub(super) fn from_authenticated_json(bytes: &[u8]) -> Result<Self> {
        use base64::Engine;
        let value: serde_json::Value = serde_json::from_slice(bytes)?;
        let logs = |field: &str| -> Result<Vec<LogKey>> {
            value[field]
                .as_array()
                .context("Trust root missing log list")?
                .iter()
                .map(|log| {
                    let decode = |v: &serde_json::Value| -> Result<Vec<u8>> {
                        Ok(base64::engine::general_purpose::STANDARD
                            .decode(v.as_str().context("Missing log key bytes")?)?)
                    };
                    let der = decode(&log["publicKey"]["rawBytes"])?;
                    let id = hex::encode(decode(&log["logId"]["keyId"])?);
                    // TUF binds each format-specific ID to its key. Rekor v2 uses
                    // a note-key ID, which is not SHA256(SPKI); its presence must
                    // not invalidate a trust root also containing supported v1 logs.
                    anyhow::ensure!(id.len() == 64, "Invalid log key ID length");
                    let url: url::Url = log["baseUrl"]
                        .as_str()
                        .context("Missing log origin")?
                        .parse()?;
                    let (start, end) = validity(&log["publicKey"]["validFor"])?;
                    Ok(LogKey {
                        id,
                        der,
                        origin: url.host_str().context("Missing log hostname")?.to_owned(),
                        start,
                        end,
                    })
                })
                .collect()
        };
        // Also retain upstream schema validation after TUF target authentication.
        SigstoreTrustRoot::from_trusted_root_json_unchecked(bytes)?;
        let fulcio = value["certificateAuthorities"]
            .as_array()
            .context("Missing Fulcio authorities")?
            .iter()
            .map(|ca| {
                let (start, end) = validity(&ca["validFor"])?;
                let certs = ca["certChain"]["certificates"]
                    .as_array()
                    .context("Missing Fulcio chain")?
                    .iter()
                    .map(|cert| {
                        Ok(base64::engine::general_purpose::STANDARD.decode(
                            cert["rawBytes"]
                                .as_str()
                                .context("Missing Fulcio certificate")?,
                        )?)
                    })
                    .collect::<Result<Vec<_>>>()?;
                Ok(Authority { certs, start, end })
            })
            .collect::<Result<Vec<_>>>()?;
        Ok(Self {
            fulcio,
            rekor: logs("tlogs")?,
            ctfe: logs("ctlogs")?,
        })
    }

    pub fn rekor_key(&self, id: &str) -> Result<&LogKey> {
        self.rekor
            .iter()
            .find(|k| k.id == id)
            .context("Unknown Rekor log key")
    }

    pub fn ctfe_key(&self, id: &str) -> Result<&LogKey> {
        self.ctfe
            .iter()
            .find(|k| k.id == id)
            .context("Unknown CT log key")
    }

    pub fn fulcio_certs(
        &self,
        signing_time: pki_types::UnixTime,
    ) -> Result<Vec<pki_types::CertificateDer<'_>>> {
        use x509_cert::der::Decode;
        let seconds = signing_time.as_secs();
        let time = i64::try_from(seconds)
            .ok()
            .and_then(|s| chrono::DateTime::from_timestamp(s, 0))
            .context("Signing time out of range")?;
        let mut certs = Vec::new();
        for ca in &self.fulcio {
            if ca.start.is_some_and(|s| time < s) || ca.end.is_some_and(|e| time > e) {
                continue;
            }
            for der in &ca.certs {
                let cert = x509_cert::Certificate::from_der(der)?;
                let validity = cert.tbs_certificate.validity;
                // webpki deliberately does not validate trust-anchor expiry.
                if seconds >= validity.not_before.to_unix_duration().as_secs()
                    && seconds <= validity.not_after.to_unix_duration().as_secs()
                {
                    certs.push(pki_types::CertificateDer::from(der.as_slice()));
                }
            }
        }
        anyhow::ensure!(
            !certs.is_empty(),
            "No Fulcio authority valid at authenticated signing time"
        );
        Ok(certs)
    }
}

const ROOT: &[u8] = include_bytes!("../../../resources/sigstore-root.json");
const METADATA: &str = "https://tuf-repo-cdn.sigstore.dev/";
const TARGETS: &str = "https://tuf-repo-cdn.sigstore.dev/targets/";

pub(super) fn prepare_datastore(path: &std::path::Path) -> Result<()> {
    cache::prepare_datastore(path)
}

pub(super) async fn load() -> Result<Root> {
    // Match tough's timeouts and default headers. In particular, never reuse
    // github_client: it may carry a GitHub token in its default Authorization.
    let client = crate::tls::client_builder()?
        .connect_timeout(std::time::Duration::from_secs(10))
        .timeout(std::time::Duration::from_secs(30))
        .build()?;
    load_from(
        client,
        METADATA.parse()?,
        TARGETS.parse()?,
        &cache::native_path()?,
    )
    .await
}

pub(super) async fn load_github(
    client: reqwest::Client,
    datastore: Option<&std::path::Path>,
) -> Result<Root> {
    let base = "https://raw.githubusercontent.com/sigstore/root-signing/main/";
    let native;
    let datastore = match datastore {
        Some(path) => path,
        None => {
            native = cache::native_path()?;
            &native
        }
    };
    let repository = cache::load(
        ROOT,
        GithubTransport(CliTransport(client)),
        format!("{base}metadata/").parse()?,
        format!("{base}targets/").parse()?,
        datastore,
    )
    .await?;
    let bytes = repository
        .read_target(&tough::TargetName::new("trusted_root.json")?)
        .await?
        .context("Missing authenticated trust root")?
        .into_vec()
        .await?;
    Root::from_authenticated_json(&bytes).context("Invalid authenticated trust root")
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
    datastore: &std::path::Path,
) -> Result<Root> {
    let repository = cache::load(ROOT, CliTransport(client), metadata, targets, datastore)
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
    Root::from_authenticated_json(&data).context("Invalid authenticated Sigstore trust root")
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

    #[test]
    fn fulcio_authority_trust_and_certificate_validity_use_signed_time() {
        let mut value: serde_json::Value = serde_json::from_slice(include_bytes!(
            "../../../tests/fixtures/sigstore/trusted_root.json"
        ))
        .unwrap();
        let seconds = 1_792_042_636;
        let date = chrono::DateTime::from_timestamp(seconds, 0)
            .unwrap()
            .to_rfc3339();
        for ca in value["certificateAuthorities"].as_array_mut().unwrap() {
            ca["validFor"] = serde_json::json!({"start":date,"end":date});
        }
        let root = Root::from_authenticated_json(&serde_json::to_vec(&value).unwrap()).unwrap();
        let time = |s| pki_types::UnixTime::since_unix_epoch(std::time::Duration::from_secs(s));
        assert!(!root.fulcio_certs(time(seconds as u64)).unwrap().is_empty());
        assert!(root.fulcio_certs(time(seconds as u64 - 1)).is_err());
        assert!(root.fulcio_certs(time(seconds as u64 + 1)).is_err());
        // With no configured interval, expired anchors still cannot authorize
        // signatures beyond their own X.509 validity (webpki skips this check).
        for ca in value["certificateAuthorities"].as_array_mut().unwrap() {
            ca["validFor"] = serde_json::Value::Null;
        }
        let root = Root::from_authenticated_json(&serde_json::to_vec(&value).unwrap()).unwrap();
        assert!(root.fulcio_certs(time(4_102_444_800)).is_err());
    }

    #[test]
    fn historical_log_keys_are_checked_at_authenticated_event_time() {
        let mut value: serde_json::Value = serde_json::from_slice(include_bytes!(
            "../../../tests/fixtures/sigstore/trusted_root.json"
        ))
        .unwrap();
        value["tlogs"][0]["publicKey"]["validFor"] =
            serde_json::json!({"start": "2020-01-01T00:00:00Z", "end": "2021-01-01T00:00:00Z"});
        let root = Root::from_authenticated_json(&serde_json::to_vec(&value).unwrap()).unwrap();
        let key = &root.rekor[0];
        key.check_time(1_600_000_000).unwrap();
        assert!(key.check_time(1_500_000_000).is_err());
        assert!(key.check_time(1_700_000_000).is_err());
        assert!(key.check_millis(1_609_459_200_001).is_err());
        value["tlogs"][0]["publicKey"]["validFor"]["end"] = 0.into();
        assert!(Root::from_authenticated_json(&serde_json::to_vec(&value).unwrap()).is_err());
    }

    #[tokio::test]
    #[ignore = "Live GitHub TUF mirror; explicit release validation"]
    async fn github_tuf_mirror_verifies_root_rotation_expiry_and_target_hash() {
        let client = reqwest::Client::builder()
            .https_only(true)
            .no_proxy()
            .user_agent("nyxid-update-validation")
            .build()
            .unwrap();
        let state = cache::test_directory();
        load_github(client.clone(), Some(state.path()))
            .await
            .unwrap();
        load_github(client, Some(state.path())).await.unwrap();
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
        let state = cache::test_directory();
        assert!(
            load_from(
                client,
                format!("{}/", server.uri()).parse().unwrap(),
                format!("{}/targets/", server.uri()).parse().unwrap(),
                state.path()
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

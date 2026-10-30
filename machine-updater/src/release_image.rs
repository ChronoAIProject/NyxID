use anyhow::{Context, Result, ensure};
use sha2::{Digest, Sha256};
use std::{
    future::Future,
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use tokio::sync::Mutex;

const REPOSITORY: &str = "chronoaiproject/nyxid/nyxid-machine-updater";
const MANIFEST_LIMIT: usize = 4 * 1024 * 1024;
const TOKEN_LIMIT: usize = 64 * 1024;

#[derive(serde::Deserialize)]
struct RegistryToken {
    // Never include this response or its fields in diagnostic output.
    token: zeroize::Zeroizing<String>,
}

async fn manifest_digest(
    client: &reqwest::Client,
    registry: &str,
    version: &str,
) -> Result<String> {
    nyxid_machine::update::version(version).map_err(anyhow::Error::msg)?;
    let response = client
        .get(format!("{registry}/token"))
        .query(&[
            ("service", "ghcr.io"),
            ("scope", &format!("repository:{REPOSITORY}:pull")),
        ])
        .send()
        .await?
        .error_for_status()?;
    let token: RegistryToken =
        crate::update_attestation::read_json_bounded(response, TOKEN_LIMIT).await?;
    ensure!(!token.token.is_empty(), "Registry token unavailable");
    let response = client.get(format!("{registry}/v2/{REPOSITORY}/manifests/{version}"))
        .bearer_auth(token.token.as_str())
        .header("accept", "application/vnd.oci.image.index.v1+json, application/vnd.docker.distribution.manifest.list.v2+json, application/vnd.oci.image.manifest.v1+json, application/vnd.docker.distribution.manifest.v2+json")
        .send().await?.error_for_status()?;
    let claimed = response
        .headers()
        .get("docker-content-digest")
        .and_then(|v| v.to_str().ok())
        .context("Registry digest unavailable")?
        .to_owned();
    let mut response = response;
    ensure!(
        response
            .content_length()
            .is_none_or(|n| n <= MANIFEST_LIMIT as u64),
        "Manifest exceeds limit"
    );
    let mut body = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        ensure!(
            body.len() + chunk.len() <= MANIFEST_LIMIT,
            "Manifest exceeds limit"
        );
        body.extend_from_slice(&chunk);
    }
    let digest = format!("sha256:{:x}", Sha256::digest(&body));
    ensure!(digest == claimed, "Manifest digest mismatch");
    let manifest: serde_json::Value = serde_json::from_slice(&body)?;
    ensure!(manifest["schemaVersion"] == 2, "Unsupported image manifest");
    Ok(digest)
}

async fn resolve_verified(version: &str, datastore: &Path) -> Result<String> {
    ensure!(
        datastore.is_absolute(),
        "Server trust state requires an absolute persistent path"
    );
    let client = crate::tls::client_builder()?
        .user_agent("NyxID-release-verifier")
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .build()?;
    let digest = manifest_digest(&client, "https://ghcr.io", version).await?;
    crate::update_attestation::verify_image_attestation(
        &client,
        &digest[7..],
        version,
        Some(datastore),
    )
    .await.inspect_err(|error| {
        tracing::warn!(stage = "verify_updater_image", reason = %error,
            "Official updater verification failed; trust storage must remain private, writable and durable");
    })?;
    Ok(format!("{}@{digest}", nyxid_machine::update::UPDATER_IMAGE))
}

struct Entry {
    version: String,
    image: Option<String>,
    expires: Instant,
}
pub struct VerifiedUpdater {
    datastore: PathBuf,
    cache: Mutex<Option<Entry>>,
}
impl VerifiedUpdater {
    /// The server must provision this private durable directory independently
    /// of HOME and retain it across restarts. There is no ephemeral fallback.
    pub fn new(datastore: PathBuf) -> Result<Self> {
        ensure!(
            datastore.is_absolute(),
            "Server trust state requires an absolute persistent path"
        );
        crate::update_attestation::prepare_tuf_datastore(&datastore)
            .context("Server trust storage must be a writable private durable directory")?;
        Ok(Self {
            datastore,
            cache: Mutex::new(None),
        })
    }
    /// Container deployments override this with a persistent service volume.
    /// Native servers use a separate private user cache, never /var/lib by default.
    pub fn for_server() -> Result<Self> {
        Self::new(server_path(
            std::env::var_os("NYXID_SERVER_TUF_STATE_DIR"),
            std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }),
        )?)
    }
    pub async fn image(&self, version: &str) -> Option<String> {
        self.cached(version, || resolve_verified(version, &self.datastore))
            .await
    }
    async fn cached<F, Fut>(&self, version: &str, resolve: F) -> Option<String>
    where
        F: FnOnce() -> Fut,
        Fut: Future<Output = Result<String>>,
    {
        // Single flight, release keyed, positive cache one hour, transient failure
        // thirty seconds. An unsuccessful verification never exposes a tag.
        let mut cache = self.cache.lock().await;
        if let Some(entry) = cache.as_ref()
            && entry.version == version
            && entry.expires > Instant::now()
        {
            return entry.image.clone();
        }
        let image = tokio::time::timeout(Duration::from_secs(45), resolve())
            .await
            .ok()
            .and_then(Result::ok);
        let ttl = if image.is_some() { 3600 } else { 30 };
        *cache = Some(Entry {
            version: version.into(),
            image: image.clone(),
            expires: Instant::now() + Duration::from_secs(ttl),
        });
        image
    }
}

fn server_path(
    configured: Option<std::ffi::OsString>,
    home: Option<std::ffi::OsString>,
) -> Result<PathBuf> {
    let path = match configured {
        Some(path) => PathBuf::from(path),
        None => {
            PathBuf::from(home.context("No home directory; configure NYXID_SERVER_TUF_STATE_DIR")?)
                .join(".nyxid/server-tuf")
        }
    };
    ensure!(
        path.is_absolute(),
        "NYXID_SERVER_TUF_STATE_DIR must be an absolute persistent path"
    );
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{header, method, path},
    };
    #[tokio::test]
    async fn registry_resolution_hashes_the_manifest_and_uses_pull_only_auth() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/token"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"token":"fixture"})),
            )
            .mount(&server)
            .await;
        let body = r#"{"schemaVersion":2,"manifests":[]}"#;
        let digest = format!("sha256:{:x}", Sha256::digest(body));
        Mock::given(method("GET"))
            .and(path(format!("/v2/{REPOSITORY}/manifests/0.41.0")))
            .and(header("authorization", "Bearer fixture"))
            .respond_with(
                ResponseTemplate::new(200)
                    .insert_header("docker-content-digest", digest.as_str())
                    .set_body_string(body),
            )
            .mount(&server)
            .await;
        assert_eq!(
            manifest_digest(&reqwest::Client::new(), &server.uri(), "0.41.0")
                .await
                .unwrap(),
            digest
        );
    }
    #[tokio::test]
    async fn only_verified_images_are_cached_and_failures_never_fall_back_to_tags() {
        let directory = tempfile::tempdir().unwrap();
        let cache = VerifiedUpdater::new(directory.path().join("tuf")).unwrap();
        assert!(
            cache
                .cached("0.41.0", || async { anyhow::bail!("failed attestation") })
                .await
                .is_none()
        );
        assert!(
            cache
                .cached("0.41.0", || async {
                    panic!("failure cache must prevent repeated requests")
                })
                .await
                .is_none()
        );
        let image = format!(
            "{}@sha256:{}",
            nyxid_machine::update::UPDATER_IMAGE,
            "ab".repeat(32)
        );
        assert_eq!(
            cache.cached("0.42.0", || async { Ok(image.clone()) }).await,
            Some(image.clone())
        );
        assert_eq!(
            cache
                .cached("0.42.0", || async {
                    panic!("verified cache must be reused")
                })
                .await,
            Some(image)
        );
    }

    #[tokio::test]
    async fn oversized_registry_token_is_rejected_before_manifest_request() {
        let server = MockServer::start().await;
        Mock::given(path("/token"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({"token":"x".repeat(TOKEN_LIMIT)})),
            )
            .mount(&server)
            .await;
        let error = manifest_digest(&reqwest::Client::new(), &server.uri(), "0.41.0")
            .await
            .unwrap_err();
        assert!(format!("{error:#}").contains("JSON response exceeds size limit"));
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn server_requires_an_explicit_absolute_trust_path_without_native_fallback() {
        let error = resolve_verified("0.41.0", Path::new("relative-state"))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("absolute persistent path"));
        assert!(VerifiedUpdater::new(PathBuf::new()).is_err());
        let file = tempfile::NamedTempFile::new().unwrap();
        assert!(VerifiedUpdater::new(file.path().to_owned()).is_err());
        assert!(server_path(Some("relative-state".into()), Some("/fixture/home".into())).is_err());
        assert_eq!(
            server_path(None, Some("/fixture/home".into())).unwrap(),
            PathBuf::from("/fixture/home/.nyxid/server-tuf")
        );
        assert_eq!(
            server_path(Some("/private/volume/tuf".into()), None).unwrap(),
            PathBuf::from("/private/volume/tuf")
        );
    }
}

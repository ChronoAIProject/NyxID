use anyhow::{Context, Result, ensure};
use futures::TryStreamExt;
use sha2::{Digest, Sha256};
use std::{
    future::Future,
    time::{Duration, Instant},
};
use tokio::sync::Mutex;

const REPOSITORY: &str = "chronoaiproject/nyxid/nyxid-machine-updater";
const MANIFEST_LIMIT: usize = 4 * 1024 * 1024;

async fn manifest_digest(
    client: &reqwest::Client,
    registry: &str,
    version: &str,
) -> Result<String> {
    nyxid_machine::update::version(version).map_err(anyhow::Error::msg)?;
    let token: serde_json::Value = client
        .get(format!("{registry}/token"))
        .query(&[
            ("service", "ghcr.io"),
            ("scope", &format!("repository:{REPOSITORY}:pull")),
        ])
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let token = token["token"]
        .as_str()
        .context("Registry token unavailable")?;
    let response = client.get(format!("{registry}/v2/{REPOSITORY}/manifests/{version}"))
        .bearer_auth(token)
        .header("accept", "application/vnd.oci.image.index.v1+json, application/vnd.docker.distribution.manifest.list.v2+json, application/vnd.oci.image.manifest.v1+json, application/vnd.docker.distribution.manifest.v2+json")
        .send().await?.error_for_status()?;
    let claimed = response
        .headers()
        .get("docker-content-digest")
        .and_then(|v| v.to_str().ok())
        .context("Registry digest unavailable")?
        .to_owned();
    let mut body = Vec::new();
    let mut stream = response.bytes_stream();
    while let Some(chunk) = stream.try_next().await? {
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

async fn resolve_verified(version: &str) -> Result<String> {
    let client = crate::tls::client_builder()?
        .user_agent("NyxID-release-verifier")
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(30))
        .build()?;
    let digest = manifest_digest(&client, "https://ghcr.io", version).await?;
    crate::update_attestation::verify_image_attestation(&client, &digest[7..], version, None)
        .await?;
    Ok(format!("{}@{digest}", nyxid_machine::update::UPDATER_IMAGE))
}

struct Entry {
    version: String,
    image: Option<String>,
    expires: Instant,
}
#[derive(Default)]
pub struct VerifiedUpdater {
    cache: Mutex<Option<Entry>>,
}
impl VerifiedUpdater {
    pub async fn image(&self, version: &str) -> Option<String> {
        self.cached(version, || resolve_verified(version)).await
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
        let cache = VerifiedUpdater::default();
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
}

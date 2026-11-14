//! Trusted release identities. Client strings never select a network destination.
use super::*;
use crate::models::client_release_registry::{
    COLLECTION_NAME as RELEASES, ClientRelease, ReleaseSource,
};
use serde::Deserialize;
use std::{
    collections::HashMap,
    sync::{Arc, LazyLock},
    time::Instant,
};

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegisterRelease {
    pub platform: Platform,
    pub app_version: String,
    pub app_build: String,
    #[serde(default)]
    pub artifact_id: Option<String>,
}
impl RegisterRelease {
    pub fn from_report(report: &CrashReportV1) -> Self {
        Self {
            platform: report.platform,
            app_version: report.app_version.clone(),
            app_build: report.app_build.clone(),
            artifact_id: report.artifact_id.clone(),
        }
    }
    pub fn validate(&self) -> AppResult<()> {
        if !validation::valid_release(
            &self.app_version,
            &self.app_build,
            self.artifact_id.as_deref(),
        ) || (self.platform == Platform::Web && self.artifact_id.is_none())
        {
            return Err(AppError::ValidationError("Invalid release identity".into()));
        }
        Ok(())
    }
    fn filter(&self) -> Document {
        doc! {
            "platform": self.platform.as_str(),
            "app_version": &self.app_version,
            "app_build": &self.app_build,
            "artifact_id": &self.artifact_id
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Registration {
    Registered,
    AlreadyRegistered,
}
impl Registration {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Registered => "registered",
            Self::AlreadyRegistered => "already_registered",
        }
    }
}
pub async fn register(
    db: &Database,
    identity: &RegisterRelease,
    source: ReleaseSource,
    actor: Option<&str>,
) -> AppResult<Registration> {
    identity.validate()?;
    let row = ClientRelease {
        id: uuid::Uuid::new_v4().to_string(),
        platform: identity.platform,
        app_version: identity.app_version.clone(),
        app_build: identity.app_build.clone(),
        artifact_id: identity.artifact_id.clone(),
        source,
        registered_at: Utc::now(),
        registered_by: actor.map(str::to_owned),
    };
    let result = db
        .collection::<ClientRelease>(RELEASES)
        .update_one(
            identity.filter(),
            doc! {
                "$setOnInsert": bson::to_document(&row)
                    .map_err(|_| AppError::Internal("Release serialization failed".into()))?
            },
        )
        .upsert(true)
        .await;
    let result = match result {
        Ok(result) => result,
        Err(error)
            if matches!(error.kind.as_ref(),
                mongodb::error::ErrorKind::Command(command) if command.code == 11000
            ) || matches!(error.kind.as_ref(),
                mongodb::error::ErrorKind::Write(mongodb::error::WriteFailure::WriteError(write))
                    if write.code == 11000
            ) =>
        {
            return Ok(Registration::AlreadyRegistered);
        }
        Err(error) => return Err(error.into()),
    };
    Ok(if result.upserted_id.is_some() {
        Registration::Registered
    } else {
        Registration::AlreadyRegistered
    })
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    artifact_id: String,
    app_version: String,
    app_build: String,
}
impl Manifest {
    fn matches(&self, identity: &RegisterRelease) -> bool {
        Some(self.artifact_id.as_str()) == identity.artifact_id.as_deref()
            && self.app_version == identity.app_version
            && self.app_build == identity.app_build
    }
}
fn manifest_urls(config: &crate::config::AppConfig) -> Vec<url::Url> {
    std::iter::once(&config.frontend_url)
        .chain(config.domain_aliases.frontend_urls.iter())
        .filter_map(|origin| {
            let mut url = url::Url::parse(origin).ok()?;
            if url.host_str().is_none()
                || !url.username().is_empty()
                || url.password().is_some()
                || (url.scheme() != "https"
                    && !(config.environment == "development" && url.scheme() == "http"))
            {
                return None;
            }
            url.set_path("/crash-build.json");
            url.set_query(None);
            url.set_fragment(None);
            Some(url)
        })
        .collect()
}
async fn fetch_manifest(url: url::Url) -> Result<Manifest, logging::ManifestFailure> {
    use logging::ManifestFailure::*;
    let client = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(3))
        .no_proxy()
        .referer(false)
        .build()
        .map_err(|_| Transport)?;
    // Public build metadata only: no report, user, cookies or authorization.
    let mut response = client.get(url).send().await.map_err(|_| Transport)?;
    if !response.status().is_success() {
        return Err(Status);
    }
    if response.content_length().is_some_and(|n| n > 4096) {
        return Err(Oversize);
    }
    let mut data = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| Transport)? {
        if data.len() + chunk.len() > 4096 {
            return Err(Oversize);
        }
        data.extend_from_slice(&chunk);
    }
    let manifest: Manifest = serde_json::from_slice(&data).map_err(|_| InvalidJson)?;
    if !validation::valid_release(
        &manifest.app_version,
        &manifest.app_build,
        Some(&manifest.artifact_id),
    ) {
        return Err(InvalidJson);
    }
    Ok(manifest)
}
struct CachedManifest {
    fetched_at: Instant,
    manifest: Option<Manifest>,
}
type Gate = Arc<tokio::sync::Mutex<Option<CachedManifest>>>;
static FLIGHTS: LazyLock<std::sync::Mutex<HashMap<String, Gate>>> = LazyLock::new(Default::default);
async fn cached_manifest(url: url::Url, identity: Option<&RegisterRelease>) -> Option<Manifest> {
    let key = url.origin().ascii_serialization();
    let gate = {
        let mut gates = FLIGHTS.lock().expect("manifest flights");
        if gates.len() >= 1024 {
            gates.retain(|_, g| {
                Arc::strong_count(g) > 1
                    || g.try_lock().map_or(true, |cached| {
                        cached
                            .as_ref()
                            .is_some_and(|v| v.fetched_at.elapsed() < Duration::from_secs(60))
                    })
            });
        }
        if gates.len() >= 1024 && !gates.contains_key(&key) {
            return None;
        }
        gates.entry(key).or_default().clone()
    };
    let mut cached = gate.lock().await;
    if let Some(value) = cached
        .as_ref()
        .filter(|v| v.fetched_at.elapsed() < Duration::from_secs(60))
    {
        return value.manifest.clone();
    }
    // Cache even cancellation as a negative result. The gate stays locked for
    // the bounded fetch so all distinct report identities share one request.
    *cached = Some(CachedManifest {
        fetched_at: Instant::now(),
        manifest: None,
    });
    let manifest = match fetch_manifest(url.clone()).await {
        Ok(manifest) => {
            if identity.is_some_and(|identity| !manifest.matches(identity)) {
                logging::manifest_failure(
                    logging::ManifestFailure::Mismatch,
                    url.host_str().unwrap_or_default(),
                );
            }
            Some(manifest)
        }
        Err(reason) => {
            logging::manifest_failure(reason, url.host_str().unwrap_or_default());
            None
        }
    };
    *cached = Some(CachedManifest {
        fetched_at: Instant::now(),
        manifest: manifest.clone(),
    });
    manifest
}
pub async fn auto_register_web(
    db: &Database,
    config: &crate::config::AppConfig,
    identity: &RegisterRelease,
) {
    if identity.platform != Platform::Web || identity.validate().is_err() {
        return;
    }
    match db
        .collection::<ClientRelease>(RELEASES)
        .find_one(identity.filter())
        .await
    {
        Ok(None) => {}
        // A database failure is not authority to perform outbound I/O.
        _ => return,
    }
    for url in manifest_urls(config) {
        if let Some(manifest) = cached_manifest(url, Some(identity)).await
            && manifest.matches(identity)
        {
            let _ = register(db, identity, ReleaseSource::FrontendManifest, None).await;
            return;
        }
    }
}
// The initial tick registers enrolled live releases before any crash arrives.
// Each refresh shares the hardened, single-flight origin cache with ingestion.
const MANIFEST_REFRESH_INTERVAL: Duration = Duration::from_secs(15 * 60);
pub async fn run_manifest_refresh(
    db: Database,
    config: Arc<crate::config::AppConfig>,
    shutdown: super::super::shutdown::Runtime,
) {
    run_manifest_refresh_every(db, config, shutdown, MANIFEST_REFRESH_INTERVAL).await;
}
async fn run_manifest_refresh_every(
    db: Database,
    config: Arc<crate::config::AppConfig>,
    shutdown: super::super::shutdown::Runtime,
    cadence: Duration,
) {
    let mut interval = tokio::time::interval(cadence);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    while let Some(_work) = shutdown.tick(&mut interval).await {
        refresh_live_manifests(&db, &config, &shutdown).await;
    }
}
async fn refresh_live_manifests(
    db: &Database,
    config: &crate::config::AppConfig,
    shutdown: &super::super::shutdown::Runtime,
) {
    if shutdown.is_draining()
        || !matches!(
            admin::rollout(db).await,
            Ok(admin::Rollout::Global | admin::Rollout::Cohort)
        )
    {
        return;
    }
    for url in manifest_urls(config) {
        if shutdown.is_draining() {
            break;
        }
        if let Some(manifest) = cached_manifest(url, None).await {
            let identity = RegisterRelease {
                platform: Platform::Web,
                app_version: manifest.app_version,
                app_build: manifest.app_build,
                artifact_id: Some(manifest.artifact_id),
            };
            // A known release needs no write; a lookup failure grants no authority.
            if matches!(
                db.collection::<ClientRelease>(RELEASES)
                    .find_one(identity.filter())
                    .await,
                Ok(None)
            ) {
                let _ = register(db, &identity, ReleaseSource::FrontendManifest, None).await;
            }
        }
    }
}
#[cfg(test)]
#[path = "releases_review_tests.rs"]
mod review_tests;
/// A web identity binds its artifact; store binaries bind platform/version/build.
/// Verification is query-time so registration requires no report backfill.
pub fn verification_lookup() -> Document {
    doc! {
        "$lookup": {
            "from": RELEASES,
            "let": {
                "platform": "$report.platform",
                "version": "$report.app_version",
                "build": "$report.app_build",
                "artifact": "$report.artifact_id"
            },
            "pipeline": [{
                "$match": {
                    "$expr": {
                        "$and": [
                            {"$eq": ["$platform", "$$platform"]},
                            {"$eq": ["$app_version", "$$version"]},
                            {"$eq": ["$app_build", "$$build"]},
                            {"$or": [
                                {"$ne": ["$$platform", "web"]},
                                {"$eq": ["$artifact_id", "$$artifact"]}
                            ]}
                        ]
                    }
                }
            }, {"$limit": 1}, {"$project": {"_id": 1}}],
            "as": "registered_release"
        }
    }
}
pub async fn ensure_indexes(db: &Database) -> Result<(), mongodb::error::Error> {
    db.collection::<ClientRelease>(RELEASES)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"platform":1,"app_version":1,"app_build":1,"artifact_id":1})
                .options(
                    IndexOptions::builder()
                        .name("client_release_identity".to_owned())
                        .unique(true)
                        .build(),
                )
                .build(),
        )
        .await?;
    // Exercise decoding here only through typed models in callers.
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiremock::{
        Mock, MockServer, ResponseTemplate,
        matchers::{method, path},
    };
    #[tokio::test]
    async fn web_manifest_is_bounded_strict_single_flight_and_never_uses_report_urls() {
        let (db, _, _) = super::super::tests::account("crash_manifest").await;
        let server = MockServer::builder().start().await;
        let r = super::super::tests::report();
        let mut config = crate::test_utils::test_app_state(db.clone()).config;
        config.environment = "development".into();
        config.frontend_url = server.uri();
        config.domain_aliases.frontend_urls.clear();
        Mock::given(method("GET")).and(path("/crash-build.json")).respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"artifact_id":r.artifact_id,"app_version":r.app_version,"app_build":r.app_build}))).expect(1).mount(&server).await;
        let identity = RegisterRelease::from_report(&r);
        futures::future::join_all((0..8).map(|_| auto_register_web(&db, &config, &identity))).await;
        let rows: Vec<_> = db
            .collection::<ClientRelease>(RELEASES)
            .find(doc! {})
            .await
            .unwrap()
            .try_collect()
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert!(matches!(rows[0].source, ReleaseSource::FrontendManifest));
        assert!(rows[0].registered_by.is_none());
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests[0].url.path(), "/crash-build.json");
        assert!(requests[0].url.query().is_none());
        assert!(requests[0].body.is_empty());
        for h in ["cookie", "authorization", "user-agent"] {
            assert!(requests[0].headers.get(h).is_none());
        }
        let mut contaminated = r.clone();
        contaminated.app_build = "https://evil.test/private".into();
        auto_register_web(&db, &config, &RegisterRelease::from_report(&contaminated)).await; // Closed identity validation rejects it before I/O.
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
        config.environment = "production".into();
        assert!(manifest_urls(&config).is_empty());
    }
    #[tokio::test]
    async fn mismatch_unknown_fields_oversize_and_redirect_are_unverified_and_negative_cached() {
        for mode in ["mismatch", "extra", "oversize", "redirect"] {
            let (db, _, _) = super::super::tests::account("crash_manifest_refused").await;
            let server = MockServer::builder().start().await;
            let target = MockServer::builder().start().await;
            Mock::given(method("GET"))
                .respond_with(ResponseTemplate::new(200))
                .expect(0)
                .mount(&target)
                .await;
            let r = super::super::tests::report();
            let mut config = crate::test_utils::test_app_state(db.clone()).config;
            config.environment = "development".into();
            config.frontend_url = server.uri();
            config.domain_aliases.frontend_urls.clear();
            let mut manifest = serde_json::json!({"artifact_id":r.artifact_id,"app_version":r.app_version,"app_build":r.app_build});
            let response = match mode {
                "redirect" => ResponseTemplate::new(302)
                    .insert_header("Location", format!("{}/secret", target.uri())),
                "oversize" => ResponseTemplate::new(200).set_body_string("x".repeat(4097)),
                "extra" => {
                    manifest["message"] = serde_json::json!("SECRET_SENTINEL");
                    ResponseTemplate::new(200).set_body_json(manifest)
                }
                _ => {
                    manifest["app_build"] = serde_json::json!("mismatch");
                    ResponseTemplate::new(200).set_body_json(manifest)
                }
            };
            Mock::given(method("GET"))
                .and(path("/crash-build.json"))
                .respond_with(response)
                .expect(1)
                .mount(&server)
                .await;
            auto_register_web(&db, &config, &RegisterRelease::from_report(&r)).await;
            auto_register_web(&db, &config, &RegisterRelease::from_report(&r)).await;
            assert_eq!(
                db.collection::<ClientRelease>(RELEASES)
                    .count_documents(doc! {})
                    .await
                    .unwrap(),
                0,
                "{mode}"
            );
        }
    }
    use futures::TryStreamExt;
}

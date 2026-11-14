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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegisterRelease {
    pub platform: Platform,
    pub app_version: String,
    pub app_build: String,
    #[serde(default)]
    pub artifact_id: Option<String>,
}
impl RegisterRelease {
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
        doc! {"platform": self.platform.as_str(), "app_version": &self.app_version, "app_build": &self.app_build, "artifact_id": &self.artifact_id}
    }
}
pub async fn register(
    db: &Database,
    identity: &RegisterRelease,
    source: ReleaseSource,
    actor: Option<&str>,
) -> AppResult<()> {
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
    db.collection::<ClientRelease>(RELEASES).update_one(identity.filter(), doc! {"$setOnInsert": bson::to_document(&row).map_err(|_| AppError::Internal("Release serialization failed".into()))?}).upsert(true).await?;
    Ok(())
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    artifact_id: String,
    app_version: String,
    app_build: String,
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
async fn fetch_manifest(client: &reqwest::Client, url: url::Url) -> Option<Manifest> {
    // Public build metadata only: no report, user, cookies or authorization.
    let mut response = client.get(url).send().await.ok()?;
    if !response.status().is_success() || response.content_length().is_some_and(|n| n > 4096) {
        return None;
    }
    let mut data = Vec::new();
    while let Some(chunk) = response.chunk().await.ok()? {
        if data.len() + chunk.len() > 4096 {
            return None;
        }
        data.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&data).ok()
}
type Gate = Arc<tokio::sync::Mutex<Option<Instant>>>;
static FLIGHTS: LazyLock<std::sync::Mutex<HashMap<String, Gate>>> = LazyLock::new(Default::default);
pub async fn auto_register_web(
    db: &Database,
    config: &crate::config::AppConfig,
    report: &CrashReportV1,
) {
    if report.platform != Platform::Web {
        return;
    }
    let identity = RegisterRelease {
        platform: report.platform,
        app_version: report.app_version.clone(),
        app_build: report.app_build.clone(),
        artifact_id: report.artifact_id.clone(),
    };
    if identity.validate().is_err() {
        return;
    }
    let Ok(Some(_)) = db
        .collection::<ClientRelease>(RELEASES)
        .find_one(identity.filter())
        .await
    else {
        let key = format!(
            "{}:{}:{}",
            db.name(),
            config.frontend_url,
            report.artifact_id.as_deref().unwrap_or_default()
        );
        let gate = {
            let mut gates = FLIGHTS.lock().expect("manifest flights");
            // Evict only idle entries; active waiters keep their single-flight identity.
            if gates.len() >= 1024 {
                gates.retain(|_, g| {
                    Arc::strong_count(g) > 1
                        || g.try_lock().map_or(true, |at| {
                            at.is_some_and(|at| at.elapsed() < Duration::from_secs(60))
                        })
                });
            }
            if gates.len() >= 1024 && !gates.contains_key(&key) {
                return;
            }
            gates.entry(key).or_default().clone()
        };
        let _ = tokio::time::timeout(Duration::from_secs(3), async {
            let mut failure = gate.lock().await;
            if failure.is_some_and(|at| at.elapsed() < Duration::from_secs(60)) {
                return;
            }
            if db
                .collection::<ClientRelease>(RELEASES)
                .find_one(identity.filter())
                .await
                .ok()
                .flatten()
                .is_some()
            {
                return;
            }
            // Set before I/O so cancellation/timeout is negative-cached too.
            *failure = Some(Instant::now());
            let Ok(client) = reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(3))
                .no_proxy()
                .referer(false)
                .build()
            else {
                return;
            };
            for url in manifest_urls(config) {
                if let Some(manifest) = fetch_manifest(&client, url).await
                    && manifest.artifact_id == report.artifact_id.as_deref().unwrap_or_default()
                    && manifest.app_version == report.app_version
                    && manifest.app_build == report.app_build
                    && register(db, &identity, ReleaseSource::FrontendManifest, None)
                        .await
                        .is_ok()
                {
                    return;
                }
            }
            *failure = Some(Instant::now());
        })
        .await;
        return;
    };
}
/// A web identity binds its artifact; store binaries bind platform/version/build.
/// Verification is query-time so registration requires no report backfill.
pub fn verification_lookup() -> Document {
    doc! {"$lookup": {"from": RELEASES, "let": {"platform": "$report.platform", "version": "$report.app_version", "build": "$report.app_build", "artifact": "$report.artifact_id"}, "pipeline": [
        {"$match": {"$expr": {"$and": [
            {"$eq": ["$platform", "$$platform"]}, {"$eq": ["$app_version", "$$version"]}, {"$eq": ["$app_build", "$$build"]},
            {"$or": [{"$ne": ["$$platform", "web"]}, {"$eq": ["$artifact_id", "$$artifact"]}]}
        ]}}}, {"$limit": 1}, {"$project": {"_id":1}}
    ], "as": "registered_release"}}
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
        let server = MockServer::start().await;
        let r = super::super::tests::report();
        let mut config = crate::test_utils::test_app_state(db.clone()).config;
        config.environment = "development".into();
        config.frontend_url = server.uri();
        config.domain_aliases.frontend_urls.clear();
        Mock::given(method("GET")).and(path("/crash-build.json")).respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"artifact_id":r.artifact_id,"app_version":r.app_version,"app_build":r.app_build}))).expect(1).mount(&server).await;
        futures::future::join_all((0..8).map(|_| auto_register_web(&db, &config, &r))).await;
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
        auto_register_web(&db, &config, &contaminated).await; // Closed identity validation rejects it before I/O.
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
        config.environment = "production".into();
        assert!(manifest_urls(&config).is_empty());
    }
    #[tokio::test]
    async fn mismatch_unknown_fields_oversize_and_redirect_are_unverified_and_negative_cached() {
        for mode in ["mismatch", "extra", "oversize", "redirect"] {
            let (db, _, _) = super::super::tests::account("crash_manifest_refused").await;
            let server = MockServer::start().await;
            let target = MockServer::start().await;
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
            auto_register_web(&db, &config, &r).await;
            auto_register_web(&db, &config, &r).await;
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

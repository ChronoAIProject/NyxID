use super::*;
use futures::TryStreamExt;
use tracing::instrument::WithSubscriber;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

fn identity() -> RegisterRelease {
    RegisterRelease::from_report(&super::super::tests::report())
}
fn manifest(identity: &RegisterRelease) -> serde_json::Value {
    serde_json::json!({"artifact_id":identity.artifact_id,"app_version":identity.app_version,"app_build":identity.app_build})
}

#[tokio::test]
async fn distinct_unknown_artifacts_share_one_fetch_per_origin_and_cached_positive_manifest() {
    let (db, _, _) = super::super::tests::account("crash_origin_cache").await;
    let primary = MockServer::builder().start().await;
    let alias = MockServer::builder().start().await;
    let known = [identity(), identity()];
    for (server, release) in [(&primary, &known[0]), (&alias, &known[1])] {
        Mock::given(method("GET"))
            .and(path("/crash-build.json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(manifest(release)))
            .expect(1)
            .mount(server)
            .await;
    }
    let mut config = crate::test_utils::test_app_state(db.clone()).config;
    config.environment = "development".into();
    config.frontend_url = primary.uri();
    // Duplicate aliases for an origin still share the same fetch.
    config.domain_aliases.frontend_urls = vec![alias.uri(), primary.uri()];
    let unknown: Vec<_> = (0..32).map(|_| identity()).collect();
    futures::future::join_all(unknown.iter().map(|r| auto_register_web(&db, &config, r))).await;
    assert_eq!(
        db.collection::<ClientRelease>(RELEASES)
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    for release in &known {
        auto_register_web(&db, &config, release).await;
    }
    assert_eq!(
        db.collection::<ClientRelease>(RELEASES)
            .count_documents(doc! {})
            .await
            .unwrap(),
        2
    );
    assert_eq!(primary.received_requests().await.unwrap().len(), 1);
    assert_eq!(alias.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn failed_manifest_fetch_logs_exactly_one_closed_warning_and_caches_distinct_artifacts() {
    use logging::ManifestFailure::*;
    let private = "SECRET_SENTINEL person@example.test Bearer token /private/container";
    for reason in [Transport, Status, Oversize, InvalidJson, Mismatch] {
        let server = MockServer::builder().start().await;
        // A listening socket with no HTTP response exercises the transport timeout.
        let stalled = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let known = identity();
        let response = match reason {
            Transport => ResponseTemplate::new(200),
            Status => ResponseTemplate::new(500).set_body_string(private),
            Oversize => ResponseTemplate::new(200).set_body_string(private.repeat(100)),
            InvalidJson => {
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"message":private}))
            }
            Mismatch => ResponseTemplate::new(200).set_body_json(manifest(&known)),
        };
        Mock::given(method("GET"))
            .and(path("/crash-build.json"))
            .respond_with(response)
            .expect(if reason == Transport { 0 } else { 1 })
            .mount(&server)
            .await;
        let origin = if reason == Transport {
            format!("http://{}", stalled.local_addr().unwrap())
        } else {
            server.uri()
        };
        let mut config = crate::test_utils::test_app_state(
            crate::test_utils::connect_test_database("crash_fetch_logs")
                .await
                .unwrap(),
        )
        .config;
        config.environment = "development".into();
        config.frontend_url = format!("{origin}/PRIVATE_PATH?token=PRIVATE_QUERY");
        config.domain_aliases.frontend_urls.clear();
        let url = manifest_urls(&config).pop().unwrap();
        let capture = logging::tests::Capture::default();
        let unknown: Vec<_> = (0..8).map(|_| identity()).collect();
        async {
            for release in &unknown {
                let _ = cached_manifest(url.clone(), Some(release)).await;
            }
        }
        .with_subscriber(capture.subscriber())
        .await;
        let events: Vec<_> = capture
            .events()
            .into_iter()
            .filter(|e| e["fields"]["message"] == "Crash release manifest fetch failed")
            .collect();
        assert_eq!(events.len(), 1, "{}", capture.text());
        assert_eq!(events[0]["level"], "WARN");
        assert_eq!(
            events[0]["fields"],
            serde_json::json!({"message":"Crash release manifest fetch failed","reason":reason.as_str(),"host":"127.0.0.1"})
        );
        let emitted = events[0].to_string();
        for sentinel in [
            private,
            "SECRET_SENTINEL",
            "PRIVATE_PATH",
            "PRIVATE_QUERY",
            "crash-build.json",
            known.artifact_id.as_deref().unwrap(),
            unknown[0].artifact_id.as_deref().unwrap(),
        ] {
            assert!(!emitted.contains(sentinel));
        }
    }
}

#[tokio::test]
async fn origin_manifest_cache_expires_positive_and_negative_results_after_sixty_seconds() {
    for status in [200, 503] {
        let server = MockServer::builder().start().await;
        let release = identity();
        Mock::given(method("GET"))
            .respond_with(ResponseTemplate::new(status).set_body_json(manifest(&release)))
            .expect(2)
            .mount(&server)
            .await;
        let url = url::Url::parse(&format!("{}/crash-build.json", server.uri())).unwrap();
        let _ = cached_manifest(url.clone(), Some(&release)).await;
        let _ = cached_manifest(url.clone(), Some(&release)).await;
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
        let gate = FLIGHTS
            .lock()
            .unwrap()
            .get(&url.origin().ascii_serialization())
            .unwrap()
            .clone();
        gate.lock().await.as_mut().unwrap().fetched_at = Instant::now() - Duration::from_secs(61);
        let _ = cached_manifest(url, Some(&release)).await;
        assert_eq!(server.received_requests().await.unwrap().len(), 2);
    }
}

#[tokio::test]
async fn crash_concurrent_release_registration_is_idempotent() {
    let (db, _, _) = super::super::tests::account("crash_release_race").await;
    let release = identity();
    let results = futures::future::join_all(
        (0..32).map(|_| register(&db, &release, ReleaseSource::Admin, None)),
    )
    .await;
    let outcomes: Vec<_> = results.into_iter().map(Result::unwrap).collect();
    assert_eq!(
        outcomes
            .iter()
            .filter(|r| **r == Registration::Registered)
            .count(),
        1
    );
    assert_eq!(
        outcomes
            .iter()
            .filter(|r| **r == Registration::AlreadyRegistered)
            .count(),
        31
    );
    assert_eq!(
        db.collection::<ClientRelease>(RELEASES)
            .count_documents(doc! {})
            .await
            .unwrap(),
        1
    );
}

#[tokio::test]
async fn crash_live_manifest_registers_at_startup_refreshes_periodically_and_stops_on_drain() {
    use crate::services::shutdown::Runtime;
    let (db, _, _) = super::super::tests::account("crash_live_manifest").await;
    let server = MockServer::builder().start().await;
    let mut config = crate::test_utils::test_app_state(db.clone()).config;
    config.environment = "development".into();
    config.frontend_url = server.uri();
    config.domain_aliases.frontend_urls.clear();
    let first = identity();
    let second = identity();
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(manifest(&first)))
        .expect(1)
        .mount(&server)
        .await;
    let runtime = Runtime::default();
    let worker = runtime.spawn_worker(run_manifest_refresh_every(
        db.clone(),
        Arc::new(config.clone()),
        runtime.clone(),
        Duration::from_millis(20),
    ));
    async fn wait_registered(db: &Database, identity: &RegisterRelease) {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if db
                    .collection::<ClientRelease>(RELEASES)
                    .find_one(identity.filter())
                    .await
                    .unwrap()
                    .is_some()
                {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
    }
    wait_registered(&db, &first).await;
    server.verify().await;
    server.reset().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(manifest(&second)))
        .expect(1)
        .mount(&server)
        .await;
    let key = url::Url::parse(&server.uri())
        .unwrap()
        .origin()
        .ascii_serialization();
    let gate = FLIGHTS.lock().unwrap().get(&key).unwrap().clone();
    gate.lock().await.as_mut().unwrap().fetched_at = Instant::now() - Duration::from_secs(61);
    wait_registered(&db, &second).await;
    runtime.begin_drain();
    tokio::time::timeout(Duration::from_secs(5), worker)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(runtime.tracked_units(), 0);
    assert_eq!(runtime.counts().background, 0);
    assert_eq!(MANIFEST_REFRESH_INTERVAL, Duration::from_secs(900));
    let rows: Vec<_> = db
        .collection::<ClientRelease>(RELEASES)
        .find(doc! {})
        .await
        .unwrap()
        .try_collect()
        .await
        .unwrap();
    assert_eq!(rows.len(), 2);
    assert!(
        rows.iter()
            .all(|r| matches!(r.source, ReleaseSource::FrontendManifest)
                && r.registered_by.is_none())
    );
}

#[tokio::test]
async fn crash_live_manifest_invalid_or_unreachable_registers_nothing_and_logs_closed_reason() {
    use crate::services::shutdown::Runtime;
    for mode in ["invalid_json", "transport"] {
        let (db, _, _) = super::super::tests::account("crash_live_manifest_failure").await;
        let server = MockServer::builder().start().await;
        let origin = if mode == "transport" {
            // Port zero is never a listening HTTP endpoint; no free-port reuse race.
            "http://127.0.0.1:0".to_owned()
        } else {
            Mock::given(method("GET"))
                .respond_with(ResponseTemplate::new(200).set_body_string("SECRET_SENTINEL"))
                .expect(1)
                .mount(&server)
                .await;
            server.uri()
        };
        let mut config = crate::test_utils::test_app_state(db.clone()).config;
        config.environment = "development".into();
        config.frontend_url = origin;
        config.domain_aliases.frontend_urls.clear();
        let capture = logging::tests::Capture::default();
        refresh_live_manifests(&db, &config, &Runtime::default())
            .with_subscriber(capture.subscriber())
            .await;
        assert_eq!(
            db.collection::<ClientRelease>(RELEASES)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );
        let events = capture.events();
        assert_eq!(events.len(), 1);
        assert_eq!(
            events[0]["fields"],
            serde_json::json!({
                "message": "Crash release manifest fetch failed", "reason": mode,
                "host": "127.0.0.1"
            })
        );
        assert!(!capture.text().contains("SECRET_SENTINEL"));
    }
}

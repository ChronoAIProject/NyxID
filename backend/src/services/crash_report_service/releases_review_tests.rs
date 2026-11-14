use super::*;
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
                let _ = cached_manifest(url.clone(), release).await;
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
        let _ = cached_manifest(url.clone(), &release).await;
        let _ = cached_manifest(url.clone(), &release).await;
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
        let gate = FLIGHTS
            .lock()
            .unwrap()
            .get(&url.origin().ascii_serialization())
            .unwrap()
            .clone();
        gate.lock().await.as_mut().unwrap().fetched_at = Instant::now() - Duration::from_secs(61);
        let _ = cached_manifest(url, &release).await;
        assert_eq!(server.received_requests().await.unwrap().len(), 2);
    }
}

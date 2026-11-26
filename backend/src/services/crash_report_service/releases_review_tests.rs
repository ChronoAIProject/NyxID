use super::*;
use crate::services::{
    feature_flag_service::{self as flags, FlagTarget},
    security_readiness_service as readiness,
};
use futures::TryStreamExt;
use mongodb::event::{EventHandler, command::CommandEvent};
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

async fn enable_manifest_rollout(db: &Database, actor: &str, target: FlagTarget) {
    readiness::ready_test_replica(db).await;
    flags::set_platform_override(db, FLAG, &target, true, actor)
        .await
        .unwrap();
    // Rollout checks need no heartbeat; stop fixture writes before monitoring passes.
    readiness::stop_test_replica(db).await;
}

#[tokio::test]
async fn crash_default_off_footprint_has_no_export_or_network_commands() {
    use crate::{
        models::feature_flag_override::{
            COLLECTION_NAME as OVERRIDES, FeatureFlagOverride, FlagTargetKind,
        },
        services::{crash_report_export, shutdown::Runtime},
        test_utils::connect_test_database_with_command_handler,
    };
    for killed in [false, true] {
        let commands = Arc::new(std::sync::Mutex::new(Vec::new()));
        let observed = commands.clone();
        let db = connect_test_database_with_command_handler(
            "crash_default_off_footprint",
            EventHandler::callback(move |event| {
                if let CommandEvent::Started(event) = event {
                    observed
                        .lock()
                        .unwrap()
                        .push((event.command_name, event.command));
                }
            }),
        )
        .await
        .unwrap();
        super::super::ensure_indexes(&db).await.unwrap();
        if killed {
            let now = Utc::now();
            db.collection::<FeatureFlagOverride>(OVERRIDES)
                .insert_one(FeatureFlagOverride {
                    id: uuid::Uuid::new_v4().to_string(),
                    org_user_id: None,
                    definition_id: None,
                    flag_key: FLAG.into(),
                    target_kind: FlagTargetKind::Global,
                    target_key: None,
                    enabled: false,
                    created_at: now,
                    updated_at: now,
                    updated_by: uuid::Uuid::new_v4().to_string(),
                })
                .await
                .unwrap();
        }
        assert_eq!(
            admin::rollout(&db).await.unwrap(),
            if killed {
                admin::Rollout::Killed
            } else {
                admin::Rollout::Off
            }
        );
        let server = MockServer::builder().start().await;
        Mock::given(method("GET"))
            .and(path("/crash-build.json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(manifest(&identity())))
            .expect(0)
            .mount(&server)
            .await;
        let mut config = crate::test_utils::test_app_state(db.clone()).config;
        config.environment = "development".into();
        config.frontend_url = server.uri();
        config.domain_aliases.frontend_urls.clear();
        let runtime = Runtime::default();

        commands.lock().unwrap().clear();
        tokio::time::timeout(
            Duration::from_secs(1),
            crash_report_export::worker::run(
                db.clone(),
                Arc::new(crash_report_export::CrashConfig::default()),
                runtime.clone(),
            ),
        )
        .await
        .expect("an unconfigured exporter returns before claiming work");
        assert!(commands.lock().unwrap().is_empty(), "killed={killed}");

        refresh_live_manifests(&db, &config, &runtime).await;
        let seen = commands.lock().unwrap().clone();
        assert_eq!(seen.len(), 1, "killed={killed}: {seen:?}");
        assert_eq!(seen[0].0, "find");
        assert_eq!(seen[0].1.get_str("find"), Ok(OVERRIDES));
        assert_eq!(
            seen[0].1.get_document("filter"),
            Ok(&doc! {"flag_key": FLAG, "definition_id": null})
        );
        assert!(server.received_requests().await.unwrap().is_empty());
        server.verify().await;
        db.drop().await.unwrap();
    }
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
    let (db, actor, _) = super::super::tests::account("crash_live_manifest").await;
    enable_manifest_rollout(&db, &actor, FlagTarget::Global).await;
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
        let (db, actor, _) = super::super::tests::account("crash_live_manifest_failure").await;
        enable_manifest_rollout(&db, &actor, FlagTarget::Global).await;
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
        let events: Vec<_> = capture
            .events()
            .into_iter()
            .filter(|event| event["fields"]["message"] == "Crash release manifest fetch failed")
            .collect();
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

#[tokio::test]
async fn crash_live_manifest_off_and_killed_skip_io_and_enabled_rollouts_write_only_once() {
    use crate::{
        models::user::{User, UserType},
        services::shutdown::Runtime,
        test_utils::{connect_test_database_with_command_handler, test_user},
    };
    for mode in ["off", "killed", "global", "cohort"] {
        let commands = Arc::new(std::sync::Mutex::new(Vec::<Document>::new()));
        let observed = commands.clone();
        let db = connect_test_database_with_command_handler(
            "crash_manifest_rollout",
            EventHandler::callback(move |event| {
                if let CommandEvent::Started(event) = event {
                    observed.lock().unwrap().push(event.command);
                }
            }),
        )
        .await
        .unwrap();
        super::super::ensure_indexes(&db).await.unwrap();
        let actor = uuid::Uuid::new_v4().to_string();
        db.collection::<User>(crate::models::user::COLLECTION_NAME)
            .insert_one(test_user(&actor, UserType::Person))
            .await
            .unwrap();
        match mode {
            "global" => enable_manifest_rollout(&db, &actor, FlagTarget::Global).await,
            "cohort" | "killed" => {
                enable_manifest_rollout(&db, &actor, FlagTarget::User(actor.clone())).await;
                if mode == "killed" {
                    flags::set_platform_override(&db, FLAG, &FlagTarget::Global, false, &actor)
                        .await
                        .unwrap();
                }
            }
            _ => {}
        }
        let enabled = matches!(mode, "global" | "cohort");
        let server = MockServer::builder().start().await;
        let release = identity();
        Mock::given(method("GET"))
            .and(path("/crash-build.json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(manifest(&release)))
            .expect(if enabled { 2 } else { 0 })
            .mount(&server)
            .await;
        let mut config = crate::test_utils::test_app_state(db.clone()).config;
        config.environment = "development".into();
        config.frontend_url = server.uri();
        config.domain_aliases.frontend_urls.clear();
        for pass in 0..2 {
            if enabled && pass == 1 {
                let key = url::Url::parse(&server.uri())
                    .unwrap()
                    .origin()
                    .ascii_serialization();
                let gate = FLIGHTS.lock().unwrap().get(&key).unwrap().clone();
                gate.lock().await.as_mut().unwrap().fetched_at =
                    Instant::now() - Duration::from_secs(61);
            }
            commands.lock().unwrap().clear();
            refresh_live_manifests(&db, &config, &Runtime::default()).await;
            let seen = commands.lock().unwrap().clone();
            let writes = seen
                .iter()
                .filter(|command| {
                    ["insert", "update", "delete", "findAndModify", "bulkWrite"]
                        .iter()
                        .any(|key| command.contains_key(key))
                })
                .collect::<Vec<_>>();
            assert_eq!(
                writes.len(),
                usize::from(enabled && pass == 0),
                "{mode}: {seen:?}"
            );
            if let Some(write) = writes.first() {
                assert_eq!(write.get_str("update"), Ok(RELEASES));
            }
            assert_eq!(
                seen.iter()
                    .filter(|command| command.get_str("find") == Ok(RELEASES))
                    .count(),
                usize::from(enabled),
                "{mode}: {seen:?}"
            );
        }
        assert_eq!(
            db.collection::<ClientRelease>(RELEASES)
                .count_documents(doc! {})
                .await
                .unwrap(),
            u64::from(enabled)
        );
        assert_eq!(
            server.received_requests().await.unwrap().len(),
            if enabled { 2 } else { 0 }
        );
        server.verify().await;
    }
}

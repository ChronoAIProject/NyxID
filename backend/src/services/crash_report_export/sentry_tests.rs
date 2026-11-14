fn test_exporter(dsn: &str) -> Result<SentryExporter, &'static str> {
    SentryExporter::new(&crate::config::CrashDestinationConfig::for_test(dsn)?)
}
use super::*;
use crate::services::crash_report_service::tests::report;
use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};
#[test]
fn startup_dsn_validation_rejects_paths_queries_credentials_and_insecure_hosts() {
    for dsn in [
        "http://key@example.test/1",
        "https://key:secret@example.test/1",
        "https://key@example.test/a/1",
        "https://key@example.test/1?token=secret",
        "https://key@example.test/1#secret",
        "https://example.test/1",
        "https://key@example.test/",
    ] {
        assert!(crate::config::CrashDestinationConfig::parse(dsn).is_err());
    }
    assert!(
        crate::config::CrashDestinationConfig::parse("https://publickey@example.test/123").is_ok()
    );
}
#[tokio::test]
async fn exported_bytes_and_headers_have_no_personal_sentinels_or_request_context() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    let exporter = test_exporter(&(server.uri().replace("http://", "http://publickey@") + "/123"));
    let exporter = exporter.unwrap();
    // Contaminate fields belonging to local ownership and export bookkeeping.
    // The transport must receive only the closed report projection.
    use crate::models::client_error_report::{ClientErrorReport, Export, ExportStatus};
    let private =
        "SECRET_SENTINEL Bearer private person@example.test /Users/private eyJhbGci.token";
    let r = report();
    let now = chrono::Utc::now();
    let row = ClientErrorReport {
        id: private.into(),
        user_id: private.into(),
        event_id: r.event_id.clone(),
        consent_revision: private.into(),
        report: r,
        environment: "test".into(),
        severity: "error".into(),
        fingerprint: "local-only".into(),
        received_at: now,
        expires_at: now + chrono::Duration::days(30),
        export: Export {
            status: ExportStatus::Dispatching,
            destination_id: Some(private.into()),
            attempts: 1,
            lease_id: Some(private.into()),
            lease_until: Some(now),
            next_attempt_at: None,
            deadline: now,
            last_error_code: Some(private.into()),
            exported_at: None,
        },
    };
    let safe = SanitizedReport {
        report: &row.report,
        environment: &row.environment,
        severity: &row.severity,
    };
    assert_eq!(exporter.submit(&safe).await, SubmitOutcome::Exported);
    let requests = server.received_requests().await.unwrap();
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    let bytes = String::from_utf8(request.body.clone()).unwrap();
    for sentinel in [
        "SECRET_SENTINEL",
        "Bearer ",
        "person@example.test",
        "/Users/private",
        "eyJhbGci.token",
        "user_id",
        "account_id",
        "consent_revision",
        "server_name",
        "breadcrumbs",
        "\"request\"",
        "\"user\"",
        "\"value\"",
        "registers",
        "message",
        "customData",
    ] {
        assert!(!bytes.contains(sentinel), "{sentinel}");
    }
    assert!(!format!("{:?}", request.headers).contains("SECRET_SENTINEL"));
    for header in [
        "authorization",
        "cookie",
        "user-agent",
        "x-forwarded-for",
        "x-real-ip",
        "x-nyxid-analytics-account",
    ] {
        assert!(!request.headers.contains_key(header));
    }
    assert_eq!(request.url.path(), "/api/123/envelope/");
    assert!(request.url.query().is_none());
    assert_eq!(
        request.headers.get("x-sentry-auth").unwrap(),
        "Sentry sentry_version=7,sentry_key=publickey"
    );
}

#[tokio::test]
async fn native_export_uses_image_mapping_and_relative_offsets_without_absolute_addresses() {
    use crate::models::client_error_report::{NativeFrame, NativeImage, NativeSignal};
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    let exporter =
        test_exporter(&(server.uri().replace("http://", "http://publickey@") + "/456")).unwrap();
    let mut r = report();
    r.platform = Platform::Ios;
    r.kind = Kind::NativeCrash;
    r.native_signal = Some(NativeSignal::Abrt);
    r.exception_class = crate::models::client_error_report::ExceptionClass::Unknown;
    r.js_frames.clear();
    r.runtime = None;
    r.native_images = vec![NativeImage {
        uuid: uuid::Uuid::new_v4().to_string(),
        name: "NyxIDMobile".into(),
        size: 4096,
    }];
    r.native_frames = vec![
        NativeFrame {
            image: Some(0),
            offset: 32,
        },
        NativeFrame {
            image: Some(0),
            offset: 64,
        },
    ];
    crate::services::crash_report_service::validation::validate(&r, chrono::Utc::now()).unwrap();
    assert_eq!(
        exporter
            .submit(&SanitizedReport {
                report: &r,
                environment: "test",
                severity: "fatal",
            })
            .await,
        SubmitOutcome::Exported
    );
    let requests = server.received_requests().await.unwrap();
    let text = std::str::from_utf8(&requests[0].body).unwrap();
    let event: serde_json::Value = serde_json::from_str(text.lines().nth(2).unwrap()).unwrap();
    assert_eq!(
        event["debug_meta"]["images"][0]["debug_id"],
        r.native_images[0].uuid
    );
    assert_eq!(event["debug_meta"]["images"][0]["image_addr"], "0x0");
    assert_eq!(event["debug_meta"]["images"][0]["code_file"], "NyxIDMobile");
    assert_eq!(
        event["exception"]["values"][0]["stacktrace"]["frames"][0]["instruction_addr"],
        "0x40"
    );
    assert_eq!(
        event["exception"]["values"][0]["stacktrace"]["frames"][0]["addr_mode"],
        "rel:0"
    );
    assert_eq!(event["exception"]["values"][0]["type"], "SIGABRT");
    assert!(event["exception"]["values"][0].get("value").is_none());
    for field in ["user", "request", "breadcrumbs", "extra", "server_name"] {
        assert!(event.get(field).is_none());
    }
}
#[tokio::test]
async fn retry_unknown_rejection_and_connect_outcomes_are_distinct() {
    for (status, outcome) in [
        (200, SubmitOutcome::Exported),
        (
            429,
            SubmitOutcome::Retry {
                after: 20,
                code: "provider_retry",
            },
        ),
        (
            503,
            SubmitOutcome::Retry {
                after: 20,
                code: "provider_retry",
            },
        ),
        (403, SubmitOutcome::Failed),
        (500, SubmitOutcome::Unknown),
        (302, SubmitOutcome::Unknown),
    ] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .respond_with(ResponseTemplate::new(status).insert_header("Retry-After", "20"))
            .mount(&server)
            .await;
        let exporter =
            test_exporter(&(server.uri().replace("http://", "http://publickey@") + "/1")).unwrap();
        let r = report();
        assert_eq!(
            exporter
                .submit(&SanitizedReport {
                    report: &r,
                    environment: "test",
                    severity: "error",
                })
                .await,
            outcome
        );
    }
    let exporter = test_exporter("http://publickey@127.0.0.1:1/1").unwrap();
    let r = report();
    assert!(matches!(
        exporter
            .submit(&SanitizedReport {
                report: &r,
                environment: "test",
                severity: "error",
            })
            .await,
        SubmitOutcome::Retry {
            code: "connect_failed",
            ..
        }
    ));
}

#[tokio::test]
async fn timeout_after_dispatch_is_unknown_and_never_a_connect_retry() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_delay(Duration::from_secs(11)))
        .mount(&server)
        .await;
    let exporter =
        test_exporter(&(server.uri().replace("http://", "http://publickey@") + "/1")).unwrap();
    let r = report();
    assert_eq!(
        exporter
            .submit(&SanitizedReport {
                report: &r,
                environment: "test",
                severity: "error",
            })
            .await,
        SubmitOutcome::Unknown
    );
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[test]
fn sentry_uses_symbolicated_grouping_platform_releases_and_standard_contexts() {
    use crate::models::client_error_report::{Platform, Runtime, RuntimeName};
    for platform in [Platform::Web, Platform::Ios, Platform::Android] {
        let mut report = report();
        report.platform = platform;
        report.runtime = if platform == Platform::Web {
            Some(Runtime {
                name: RuntimeName::Chrome,
                major: 123,
            })
        } else {
            None
        };
        let bytes = envelope(&SanitizedReport {
            report: &report,
            environment: "test",
            severity: "error",
        });
        let text = String::from_utf8(bytes).unwrap();
        let event: serde_json::Value = serde_json::from_str(text.lines().nth(2).unwrap()).unwrap();
        assert!(event.get("fingerprint").is_none());
        if platform == Platform::Web {
            assert!(event["release"].as_str().unwrap().starts_with("nyxid-web@"));
            assert!(event["contexts"]["os"].get("name").is_none());
            assert_eq!(event["contexts"]["browser"]["name"], "chrome");
            assert!(event["contexts"].get("runtime").is_none());
        } else {
            assert!(
                event["release"]
                    .as_str()
                    .unwrap()
                    .starts_with(&format!("nyxid-{}@", platform.as_str()))
            );
            assert_eq!(
                event["contexts"]["os"]["name"],
                if platform == Platform::Ios {
                    "iOS"
                } else {
                    "Android"
                }
            );
            assert!(event["contexts"].get("runtime").is_none());
            assert!(event["contexts"].get("browser").is_none());
        }
    }
}

#[test]
fn every_javascript_and_cocoa_event_explicitly_disables_vendor_ip_inference() {
    for native in [false, true] {
        let mut r = report();
        if native {
            r.platform = Platform::Ios;
            r.kind = Kind::NativeCrash;
            r.native_signal = Some(crate::models::client_error_report::NativeSignal::Segv);
            r.js_frames.clear();
            r.runtime = None;
        }
        let bytes = envelope(&SanitizedReport {
            report: &r,
            environment: "test",
            severity: "error",
        });
        let event: Value =
            serde_json::from_slice(bytes.split(|b| *b == b'\n').nth(2).unwrap()).unwrap();
        assert_eq!(event["sdk"]["settings"]["infer_ip"], "never");
        assert_eq!(event["sdk"]["version"], env!("CARGO_PKG_VERSION"));
        assert!(event.get("user").is_none());
    }
}

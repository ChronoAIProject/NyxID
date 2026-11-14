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
        assert!(SentryExporter::new(dsn).is_err());
    }
    assert!(SentryExporter::new("https://publickey@example.test/123").is_ok());
}
#[tokio::test]
async fn exported_bytes_and_headers_have_no_personal_sentinels_or_request_context() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    let exporter = SentryExporter::parse(
        &(server.uri().replace("http://", "http://publickey@") + "/123"),
        true,
    );
    let exporter = exporter.unwrap();
    let r = report();
    let safe = SanitizedReport {
        report: &r,
        environment: "test",
        severity: "error",
        fingerprint: "abc",
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
        let exporter = SentryExporter::parse(
            &(server.uri().replace("http://", "http://publickey@") + "/1"),
            true,
        )
        .unwrap();
        let r = report();
        assert_eq!(
            exporter
                .submit(&SanitizedReport {
                    report: &r,
                    environment: "test",
                    severity: "error",
                    fingerprint: "abc"
                })
                .await,
            outcome
        );
    }
    let exporter = SentryExporter::parse("http://publickey@127.0.0.1:1/1", true).unwrap();
    let r = report();
    assert!(matches!(
        exporter
            .submit(&SanitizedReport {
                report: &r,
                environment: "test",
                severity: "error",
                fingerprint: "abc"
            })
            .await,
        SubmitOutcome::Retry {
            code: "connect_failed",
            ..
        }
    ));
}

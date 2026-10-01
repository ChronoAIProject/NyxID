use super::*;

#[test]
fn poll_deadline_preserves_fractional_expiry_and_handles_unbounded_intervals() {
    let now = "2026-09-09T01:02:03.123456789Z"
        .parse::<DateTime<Utc>>()
        .unwrap();
    let expires_at = now + Duration::seconds(130) + Duration::nanoseconds(123);
    assert_eq!(
        next_poll_time(now, 125, expires_at),
        now + Duration::seconds(125)
    );
    for interval in [131, i64::MAX as u64, u64::MAX] {
        assert_eq!(next_poll_time(now, interval, expires_at), expires_at);
    }
    let expires_at = now + Duration::milliseconds(750);
    assert_eq!(next_poll_time(now, 5, expires_at), expires_at);
    assert_eq!(next_poll_time(expires_at, 5, expires_at), expires_at);

    let monotonic_expiry = Instant::now() + std::time::Duration::from_millis(750);
    assert_eq!(next_poll_deadline(5, monotonic_expiry), monotonic_expiry);
    assert_eq!(
        next_poll_deadline(u64::MAX, monotonic_expiry),
        monotonic_expiry
    );
}

#[test]
fn destination_normalization_and_error_contract() {
    assert_eq!(
        normalized_destination("https://EXAMPLE.com:443/").unwrap(),
        "https://example.com"
    );
    for invalid in [
        "file:///tmp/server",
        "https://user@example.com",
        "https://example.com?secret=x",
        "https://example.com/#x",
    ] {
        assert!(normalized_destination(invalid).is_err());
    }
    let errors = [
        LoginError::Pending,
        LoginError::Denied,
        LoginError::Expired,
        LoginError::Delivered,
        LoginError::RateLimited,
        LoginError::Busy,
        LoginError::Missing,
        LoginError::DestinationMismatch,
        LoginError::InvalidCode,
        LoginError::Unavailable,
        LoginError::Unsupported,
        LoginError::Storage,
    ];
    let codes: std::collections::HashSet<_> = errors.iter().map(|e| e.exit_code()).collect();
    let names: std::collections::HashSet<_> = errors.iter().map(|e| e.code()).collect();
    assert_eq!(codes.len(), errors.len());
    assert_eq!(names.len(), errors.len());
}

#[test]
fn request_lock_serializes_processes_and_releases_on_drop() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("request.json");
    let lock = lock_request(&path).unwrap();
    assert_eq!(
        lock_request(&path)
            .unwrap_err()
            .downcast_ref::<LoginError>(),
        Some(&LoginError::Busy)
    );
    drop(lock);
    assert!(lock_request(&path).is_ok());
}

#[test]
fn pending_file_is_protected_and_terminal_states_erase_poll_secret() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("request.json");
    let mut pending = PendingLogin {
        version: 1,
        request_id: uuid::Uuid::new_v4().to_string(),
        next_poll_at: None,
        base_url: "https://example.com".into(),
        profile: Some("agent".into()),
        flow: "device".into(),
        device_code: "private-poll-secret".into(),
        expires_at: Utc::now() + Duration::minutes(10),
        interval: 5,
        state: "pending".into(),
    };
    save_pending(&path, &pending).unwrap();
    assert_eq!(
        load_pending(&path).unwrap().device_code,
        "private-poll-secret"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
    assert!(finish_error(&path, &mut pending, LoginError::Denied).is_err());
    assert!(
        !fs::read_to_string(path)
            .unwrap()
            .contains("private-poll-secret")
    );
}

#[test]
fn login_failure_without_diagnostic_keeps_legacy_json_and_exit_codes() {
    for kind in [
        LoginError::Unavailable,
        LoginError::Storage,
        LoginError::Pending,
        LoginError::Denied,
    ] {
        let failure = LoginFailure::from(kind);
        assert_eq!(failure.json(), kind.json());
        assert_eq!(failure.text(), kind.to_string());
        assert_eq!(failure.kind.exit_code(), kind.exit_code());
    }
}

#[tokio::test]
async fn malformed_challenge_json_does_not_echo_device_code_values() {
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};
    let server = MockServer::start().await;
    Mock::given(method("GET")).respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
        "device_code":"POLL_SECRET", "user_code":"ABCD-EFGH", "verification_uri":"https://example.com/login", "expires_in":"TOKEN_SECRET", "interval":5
    }))).mount(&server).await;
    let response = credential_client(None)
        .unwrap()
        .get(server.uri())
        .send()
        .await
        .unwrap();
    let failure = match response_json::<Challenge>(response).await {
        Ok(_) => panic!("invalid challenge accepted"),
        Err(error) => error,
    };
    assert_eq!(failure.diagnostic.as_ref().unwrap().stage, Stage::Response);
    for text in [failure.text(), failure.json().to_string()] {
        assert!(!text.contains("TOKEN_SECRET"));
        assert!(!text.contains("POLL_SECRET"));
    }
}

#[tokio::test]
async fn non_json_rate_limit_response_preserves_rate_limited_code() {
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(
            ResponseTemplate::new(429)
                .insert_header("Retry-After", "20")
                .set_body_string("not JSON"),
        )
        .mount(&server)
        .await;
    let response = credential_client(None)
        .unwrap()
        .get(server.uri())
        .send()
        .await
        .unwrap();
    let (error, interval) = poll_error(response).await;
    assert_eq!(error.kind, LoginError::RateLimited);
    assert_eq!(interval, Some(20));
    assert!(error.diagnostic.is_none());
    assert_eq!(
        error.json().to_string(),
        LoginError::RateLimited.json().to_string()
    );
}

#[tokio::test]
async fn normal_login_outcomes_keep_legacy_json_without_diagnostics() {
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};
    for (code, kind) in [
        (11202, LoginError::Pending),
        (11903, LoginError::Pending),
        (11204, LoginError::Denied),
        (11904, LoginError::Denied),
        (11201, LoginError::Expired),
        (11905, LoginError::Delivered),
        (11206, LoginError::RateLimited),
        (12000, LoginError::InvalidCode),
        (12004, LoginError::RateLimited),
    ] {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(400).set_body_json(
                    serde_json::json!({"error_code":code,"message":"SERVER_SECRET"}),
                ),
            )
            .mount(&server)
            .await;
        let response = credential_client(None)
            .unwrap()
            .get(server.uri())
            .send()
            .await
            .unwrap();
        let error = response_error(response).await;
        assert_eq!(error.kind, kind);
        assert!(error.diagnostic.is_none());
        let legacy = match kind {
            LoginError::Pending => {
                r#"{"error":{"code":"login_pending","message":"Login is awaiting approval; resume this request later."}}"#
            }
            LoginError::Denied => {
                r#"{"error":{"code":"login_denied","message":"Login was denied. Start a new request to try again."}}"#
            }
            LoginError::Expired => {
                r#"{"error":{"code":"login_expired","message":"Login expired. Start a new request."}}"#
            }
            LoginError::Delivered => {
                r#"{"error":{"code":"login_already_delivered","message":"This login has already been delivered. Check the saved profile with whoami."}}"#
            }
            LoginError::RateLimited => {
                r#"{"error":{"code":"login_rate_limited","message":"Login is rate limited. Wait before retrying this request."}}"#
            }
            LoginError::InvalidCode => {
                r#"{"error":{"code":"login_code_invalid","message":"The login code is invalid or cancelled."}}"#
            }
            _ => panic!("unexpected normal outcome"),
        };
        assert_eq!(error.json().to_string(), legacy);
        assert_eq!(error.text(), kind.to_string());
    }
}

#[tokio::test]
async fn unavailable_response_exposes_only_numeric_server_error_code() {
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};
    for code in [
        serde_json::json!(99999),
        serde_json::json!("SERVER_SECRET"),
        serde_json::Value::Null,
    ] {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(503).set_body_json(
                    serde_json::json!({"error_code":code,"message":"SERVER_SECRET"}),
                ),
            )
            .mount(&server)
            .await;
        let response = credential_client(None)
            .unwrap()
            .get(server.uri())
            .send()
            .await
            .unwrap();
        let error = response_error(response).await;
        assert_eq!(error.kind, LoginError::Unavailable);
        assert_eq!(
            error.diagnostic.as_ref().unwrap().server_error_code,
            code.as_i64()
        );
        assert!(
            error
                .diagnostic
                .as_ref()
                .unwrap()
                .hint
                .contains("--base-url")
        );
        assert!(!error.json().to_string().contains("SERVER_SECRET"));
    }
}

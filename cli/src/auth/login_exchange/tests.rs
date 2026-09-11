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

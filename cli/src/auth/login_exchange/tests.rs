use super::*;

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

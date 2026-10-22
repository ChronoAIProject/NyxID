use super::*;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

pub(crate) struct Home {
    _dir: tempfile::TempDir,
    previous: Option<std::ffi::OsString>,
    _guard: std::sync::MutexGuard<'static, ()>,
}
impl Home {
    pub(crate) fn new() -> Self {
        let guard = crate::test_support::env_lock()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir = tempfile::tempdir().unwrap();
        let previous = std::env::var_os("HOME");
        // SAFETY: all environment mutations are serialized by env_lock.
        unsafe {
            std::env::set_var("HOME", dir.path());
        }
        Self {
            _dir: dir,
            previous,
            _guard: guard,
        }
    }
}
impl Drop for Home {
    fn drop(&mut self) {
        // SAFETY: the environment lock is retained until after restoration.
        unsafe {
            match &self.previous {
                Some(value) => std::env::set_var("HOME", value),
                None => std::env::remove_var("HOME"),
            }
        }
    }
}
pub(crate) fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}
pub(crate) fn jwt(exp: i64) -> String {
    use base64::Engine;
    format!(
        "e30.{}.sig",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(
            serde_json::to_vec(
                &serde_json::json!({"iat": exp - 604800, "exp": exp, "sub": "user"})
            )
            .unwrap()
        )
    )
}
pub(crate) fn args(base: &str) -> AuthArgs {
    AuthArgs {
        base_url: Some(base.into()),
        access_token: None,
        access_token_env: "NYXID_REFRESH_1992_UNUSED".into(),
        profile: Some("refresh-1992".into()),
        output: crate::cli::OutputFormat::Json,
    }
}
fn seed(auth: &AuthArgs, access: &str, refresh: &str) {
    save_base_url_for(auth.profile.as_deref(), auth.base_url.as_deref().unwrap()).unwrap();
    save_tokens_for(auth.profile.as_deref(), access, Some(refresh)).unwrap();
}
#[test]
fn classifier_precedence_and_unrecognised_expiry_are_honest() {
    use RefreshRejectionReason::*;
    for (key, text, expected) in [
        ("token_expired", "Session has been revoked", TokenExpired),
        ("unauthorized", "Session expired or revoked", SessionEnded),
        (
            "unauthorized",
            "Refresh token has expired or been revoked",
            SessionEnded,
        ),
        (
            "unauthorized",
            "Unauthorized: Session has been revoked",
            Revoked,
        ),
        ("unauthorized", "Refresh token has been revoked", Revoked),
        ("unauthorized", "Session has ended", SessionEnded),
        ("unauthorized", "Session has expired", SessionExpired),
        ("unauthorized", "Refresh token has expired", TokenExpired),
        ("unauthorized", "Refresh token expired", TokenExpired),
        ("unauthorized", "Refresh token not found", NotFound),
        (
            "conflict",
            "Service access changed or expired; authorize again",
            Other,
        ),
        ("unknown", "Something unusual expired yesterday", Other),
    ] {
        let rejection =
            RefreshRejection::classify(Some(&serde_json::json!({"error":key,"message":text})));
        assert_eq!(rejection.reason, expected, "{text}");
        if expected == Other {
            assert!(rejection.headline().contains(text));
        }
    }
}
#[tokio::test]
async fn retry_uses_original_token_for_server_conflict_timeout_and_malformed_success() {
    for first in [
        ResponseTemplate::new(500),
        ResponseTemplate::new(409).set_body_json(
            serde_json::json!({"message":format!("Conflict: {ROTATION_CONFLICT_MESSAGE}")}),
        ),
        ResponseTemplate::new(200).set_body_string("{"),
        ResponseTemplate::new(200).set_delay(std::time::Duration::from_millis(150)),
    ] {
        let server = MockServer::start().await;
        Mock::given(path("/api/v1/auth/refresh"))
            .respond_with(first)
            .up_to_n_times(1)
            .with_priority(1)
            .mount(&server)
            .await;
        Mock::given(path("/api/v1/auth/refresh"))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                serde_json::json!({"access_token":"new-access","refresh_token":"new-refresh"}),
            ))
            .with_priority(2)
            .mount(&server)
            .await;
        let outcome = refresh_with_retry(
            &reqwest::Client::new(),
            &server.uri(),
            "original",
            std::time::Duration::from_secs(5),
            std::time::Duration::from_millis(100),
        )
        .await;
        assert!(matches!(outcome, RefreshExchange::Renewed { .. }));
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 2);
        for request in requests {
            assert_eq!(
                request.body_json::<serde_json::Value>().unwrap()["refresh_token"],
                "original"
            );
        }
    }
}
#[tokio::test]
async fn malformed_transient_response_preserves_retry_after_within_the_total_budget() {
    for status in [408, 429, 503] {
        let server = MockServer::start().await;
        Mock::given(path("/api/v1/auth/refresh"))
            .respond_with(
                ResponseTemplate::new(status)
                    .set_body_string("{")
                    .insert_header("Retry-After", "9999"),
            )
            .expect(1)
            .mount(&server)
            .await;
        let outcome = refresh_with_retry(
            &reqwest::Client::new(),
            &server.uri(),
            "original",
            std::time::Duration::from_secs(5),
            std::time::Duration::from_secs(1),
        )
        .await;
        assert!(matches!(outcome, RefreshExchange::Network(_)));
        server.verify().await;
    }
}
#[tokio::test]
async fn terminal_responses_are_never_retried_and_transients_exhaust_as_unavailable() {
    for (status, body, terminal) in [
        (
            401,
            serde_json::json!({"message":"Session has expired"}),
            true,
        ),
        (
            403,
            serde_json::json!({"message":"Service access changed or expired"}),
            true,
        ),
        (
            409,
            serde_json::json!({"message":"Service access changed or expired"}),
            true,
        ),
        (
            409,
            serde_json::json!({"message":ROTATION_CONFLICT_MESSAGE}),
            false,
        ),
        (
            503,
            serde_json::json!({"error":"oauth_authority_unavailable"}),
            false,
        ),
    ] {
        let server = MockServer::start().await;
        Mock::given(path("/api/v1/auth/refresh"))
            .respond_with(
                ResponseTemplate::new(status)
                    .set_body_json(body)
                    .insert_header("Retry-After", "9999"),
            )
            .expect(1)
            .mount(&server)
            .await;
        let start = tokio::time::Instant::now();
        let outcome = refresh_with_retry(
            &reqwest::Client::new(),
            &server.uri(),
            "original",
            std::time::Duration::from_millis(200),
            std::time::Duration::from_millis(100),
        )
        .await;
        assert!(start.elapsed() < std::time::Duration::from_secs(1));
        assert_eq!(matches!(outcome, RefreshExchange::Rejected(_)), terminal);
        if !terminal {
            let RefreshExchange::Network(error) = outcome else {
                panic!("expected transient")
            };
            assert!(
                InFlightRefreshOutcome::Unavailable(error)
                    .access_token()
                    .unwrap_err()
                    .is::<RefreshUnavailable>()
            );
        }
    }
    let server = MockServer::start().await;
    Mock::given(path("/api/v1/auth/refresh"))
        .respond_with(ResponseTemplate::new(401).set_body_string("not json"))
        .expect(1)
        .mount(&server)
        .await;
    assert!(matches!(
        exchange_refresh_token(&reqwest::Client::new(), &server.uri(), "original").await,
        RefreshExchange::Rejected(_)
    ));
}
#[test]
fn persistence_reports_each_stage_and_preserves_refresh_first_recovery() {
    let _home = Home::new();
    runtime().block_on(async {
        let auth = args("http://localhost:1234");
        let profile = auth.profile.as_deref();
        let old = jwt(Utc::now().timestamp() + 3600);
        seed(&auth, &old, "old-refresh");
        let refresh_path = refresh_token_file_path_for(profile).unwrap();
        std::fs::remove_file(&refresh_path).unwrap();
        std::fs::create_dir(&refresh_path).unwrap();
        let failed = persist_refresh_pair(profile, "new-access", "new-refresh").await;
        assert_eq!(failed.stage, PersistenceStage::Nothing);
        let error = failed.check(false).unwrap_err();
        assert!(
            error.is::<RefreshUnavailable>()
                && error
                    .to_string()
                    .contains("Ordinary commands will not repair")
        );
        assert_eq!(read_saved_token_for(profile).unwrap(), old);
        std::fs::remove_dir(refresh_path).unwrap();
        let access_path = token_file_path_for(profile).unwrap();
        std::fs::remove_file(&access_path).unwrap();
        std::fs::create_dir(&access_path).unwrap();
        let failed = persist_refresh_pair(profile, "new-access", "new-refresh").await;
        assert_eq!(failed.stage, PersistenceStage::Refresh);
        assert!(failed.check(true).unwrap_err().is::<RefreshUnavailable>());
        assert_eq!(
            read_saved_refresh_token_for(profile).unwrap(),
            "new-refresh"
        );
        assert!(
            persist_refresh_pair(profile, "new-access", "new-refresh")
                .await
                .check(false)
                .is_ok()
        );
        std::fs::remove_dir(access_path).unwrap();
        let metadata = user_id_file_path_for(profile).unwrap();
        std::fs::remove_file(&metadata).unwrap();
        std::fs::create_dir(&metadata).unwrap();
        let failed = persist_refresh_pair(profile, &old, "new-refresh").await;
        assert_eq!(failed.stage, PersistenceStage::Credentials);
        failed.check(true).unwrap();
        assert_eq!(read_saved_token_for(profile).unwrap(), old);
    });
}
#[test]
fn checked_client_and_ssh_preflight_stop_after_access_write_failure_without_second_exchange() {
    let _home = Home::new();
    runtime().block_on(async {
        for checked_client in [true, false] {
            let server = MockServer::start().await; let auth = args(&server.uri());
            let profile = auth.profile.as_deref();
            let dir = token_dir_for_profile(profile).unwrap(); std::fs::create_dir_all(&dir).unwrap();
            let access = token_file_path_for(profile).unwrap();
            if access.is_dir() { std::fs::remove_dir(&access).unwrap(); }
            seed(&auth, &jwt(1), "original");
            Mock::given(method("POST")).and(path("/api/v1/auth/refresh")).respond_with(move |_: &wiremock::Request| {
                std::fs::remove_file(&access).unwrap(); std::fs::create_dir(&access).unwrap();
                ResponseTemplate::new(200).set_body_json(serde_json::json!({"access_token":"new-access","refresh_token":"new-refresh"}))
            }).expect(1).mount(&server).await;
            Mock::given(path("/api/v1/ssh/certificates")).respond_with(ResponseTemplate::new(500)).expect(0).mount(&server).await;
            let result = if checked_client { crate::api::ApiClient::from_auth_checked(&auth).await.map(|_| ()) }
                else { crate::commands::ssh::run(crate::cli::SshCli { command: crate::cli::SshCommand::IssueCert {
                    auth:auth.clone(), service_id:"service".into(), public_key_file:"unused-public-key".into(), principal:"user".into(), certificate_file:"unused-cert".into(), ca_public_key_file:None,
                }}).await };
            assert!(result.unwrap_err().is::<RefreshUnavailable>());
            assert_eq!(read_saved_refresh_token_for(profile).as_deref(), Some("new-refresh"));
        }
    });
}
#[test]
fn captured_refresh_serializes_current_thread_callers_and_fences_replaced_login() {
    let _home = Home::new();
    runtime().block_on(async {
        let server = MockServer::start().await;
        let auth = args(&server.uri());
        let profile = auth.profile.as_deref();
        seed(&auth, "old-access", "original");
        let new = jwt(Utc::now().timestamp() + 3600);
        Mock::given(path("/api/v1/auth/refresh"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_delay(std::time::Duration::from_millis(50))
                    .set_body_json(
                        serde_json::json!({"access_token":new,"refresh_token":"new-refresh"}),
                    ),
            )
            .expect(1)
            .mount(&server)
            .await;
        let client = reqwest::Client::new();
        let destination = server.uri();
        let (a, b) = tokio::join!(
            refresh_captured_session(&client, profile, &destination, None, "old-access", true),
            refresh_captured_session(&client, profile, &destination, None, "old-access", true)
        );
        assert!(matches!(a, InFlightRefreshOutcome::Refreshed(_)));
        assert!(matches!(b, InFlightRefreshOutcome::Adopted(_)));
        for (destination, generation, eligible) in [
            (server.uri(), None, false),
            ("http://different.test".into(), None, true),
            (server.uri(), Some("replaced"), true),
        ] {
            assert!(matches!(
                refresh_captured_session(
                    &client,
                    profile,
                    &destination,
                    generation,
                    "old-access",
                    eligible
                )
                .await,
                InFlightRefreshOutcome::Skipped(_)
            ));
        }
    });
}

#[test]
fn late_grace_reuse_with_fresh_access_reports_unsaved_refresh_without_promising_grace_time() {
    let _home = Home::new();
    runtime().block_on(async {
        let server = MockServer::start().await;
        let auth = args(&server.uri());
        let profile = auth.profile.as_deref();
        let fresh = jwt(Utc::now().timestamp() + 3600);
        seed(&auth, &fresh, "original");
        // Grace can return the same successor 119 seconds after its rotation;
        // receiving it does not restart the server's two-minute recovery window.
        let successor = jwt(Utc::now().timestamp() + 604800 - 119);
        Mock::given(path("/api/v1/auth/refresh"))
            .respond_with(ResponseTemplate::new(200).set_body_json(
                serde_json::json!({"access_token":"new-access","refresh_token":successor}),
            ))
            .expect(1)
            .mount(&server)
            .await;
        let error = TOKEN_WRITE_FAILURE
            .scope(
                Some(refresh_token_file_path_for(profile).unwrap()),
                force_refresh_session(&auth),
            )
            .await
            .unwrap_err();
        assert!(error.is::<RefreshUnavailable>());
        assert!(
            error
                .to_string()
                .contains("immediately run `nyxid session refresh`")
        );
        assert!(!error.to_string().contains("2 minutes"));
        assert_eq!(read_saved_token_for(profile).unwrap(), fresh);
        assert_eq!(read_saved_refresh_token_for(profile).unwrap(), "original");
        ensure_session(&auth).await.unwrap(); // Fresh ordinary commands do not repair the lost successor.
        server.verify().await;
    });
}
#[test]
fn saved_successor_survives_access_write_failure_and_the_next_invocation_renews_it() {
    let _home = Home::new();
    runtime().block_on(async {
        let server=MockServer::start().await;let auth=args(&server.uri());let profile=auth.profile.as_deref();
        let old=jwt(1);seed(&auth,&old,"original");
        Mock::given(path("/api/v1/auth/refresh")).respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"access_token":"new-access","refresh_token":"successor"}))).expect(1).mount(&server).await;
        let error=TOKEN_WRITE_FAILURE.scope(Some(token_file_path_for(profile).unwrap()),crate::api::ApiClient::from_auth_checked(&auth)).await.err().unwrap();
        assert!(error.is::<RefreshUnavailable>());server.verify().await;
        assert_eq!(read_saved_token_for(profile).unwrap(),old);assert_eq!(read_saved_refresh_token_for(profile).unwrap(),"successor");
        server.reset().await;
        Mock::given(path("/api/v1/auth/refresh")).and(wiremock::matchers::body_json(serde_json::json!({"refresh_token":"successor"})))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"access_token":jwt(Utc::now().timestamp()+3600),"refresh_token":"next-successor"}))).expect(1).mount(&server).await;
        crate::api::ApiClient::from_auth_checked(&auth).await.unwrap();
        assert_eq!(read_saved_refresh_token_for(profile).unwrap(),"next-successor");
    });
}

pub(crate) fn save_destination(profile: Option<&str>, destination: &str) -> Result<()> {
    save_base_url_for(profile, destination)
}

#[test]
fn unreadable_refresh_credentials_are_retryable_in_every_refresh_path() {
    let _home = Home::new();
    runtime().block_on(async {
        let server = MockServer::start().await;
        let auth = args(&server.uri());
        let profile = auth.profile.as_deref();
        let access = jwt(1);
        seed(&auth, &access, "original");
        let refresh_path = refresh_token_file_path_for(profile).unwrap();
        std::fs::remove_file(&refresh_path).unwrap();
        std::fs::create_dir(&refresh_path).unwrap();
        Mock::given(path("/api/v1/auth/refresh"))
            .respond_with(ResponseTemplate::new(500))
            .expect(0)
            .mount(&server)
            .await;
        let captured = refresh_captured_session(
            &reqwest::Client::new(),
            profile,
            &server.uri(),
            None,
            &access,
            true,
        )
        .await
        .access_token()
        .unwrap_err();
        let preflight = crate::api::ApiClient::from_auth_checked(&auth)
            .await
            .err()
            .unwrap();
        let explicit = force_refresh_session(&auth).await.unwrap_err();
        for error in [captured, preflight, explicit] {
            assert!(error.is::<RefreshUnavailable>(), "{error}");
            let rendered: serde_json::Value =
                serde_json::from_str(&crate::error_format::render_error(&error, true)).unwrap();
            assert_eq!(rendered["exit_code"], 4);
        }
        assert_eq!(
            read_saved_token_for(profile).as_deref(),
            Some(access.as_str())
        );
        assert!(refresh_path.is_dir());
        server.verify().await;
    });
}

#[test]
fn captured_refresh_storage_faults_never_adopt_or_exchange_credentials() {
    let _home = Home::new();
    runtime().block_on(async {
        let server = MockServer::start().await;
        let auth = args(&server.uri());
        let profile = auth.profile.as_deref();
        let access = jwt(1);
        seed(&auth, &access, "original");
        Mock::given(path("/api/v1/auth/refresh"))
            .respond_with(ResponseTemplate::new(500))
            .expect(0)
            .mount(&server)
            .await;
        for name in [
            TOKEN_FILE_NAME,
            "login_generation",
            BASE_URL_FILE_NAME,
            "auth_kind",
        ] {
            let file = token_dir_for_profile(profile).unwrap().join(name);
            let original = std::fs::read(&file).ok();
            if original.is_some() {
                std::fs::remove_file(&file).unwrap();
            }
            std::fs::create_dir(&file).unwrap();
            let error = refresh_captured_session(
                &reqwest::Client::new(),
                profile,
                &server.uri(),
                None,
                &access,
                true,
            )
            .await
            .access_token()
            .unwrap_err();
            assert!(error.is::<RefreshUnavailable>(), "{name}: {error}");
            std::fs::remove_dir(&file).unwrap();
            if let Some(original) = original {
                std::fs::write(&file, original).unwrap();
            }
        }
        assert_eq!(
            read_saved_token_for(profile).as_deref(),
            Some(access.as_str())
        );
        assert_eq!(
            read_saved_refresh_token_for(profile).as_deref(),
            Some("original")
        );
        server.verify().await;
    });
}
pub(crate) fn replace_login(
    profile: Option<&str>,
    write: impl FnOnce() -> Result<()>,
) -> Result<()> {
    replace_login_files(profile, write)
}

#[test]
fn synchronous_profile_lock_waits_for_a_holder_then_succeeds() {
    let _home = Home::new();
    let holder = acquire_refresh_lock(None).unwrap();
    let release = std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(200));
        drop(holder);
    });
    let started = std::time::Instant::now();
    let acquired = acquire_refresh_lock(None).unwrap();
    assert!(started.elapsed() >= std::time::Duration::from_millis(150));
    drop(acquired);
    release.join().unwrap();
}

#[test]
fn synchronous_profile_lock_reports_a_bounded_timeout() {
    let _home = Home::new();
    let _holder = acquire_refresh_lock(None).unwrap();
    let result = acquire_refresh_lock_with_timeout(None, std::time::Duration::from_millis(50));
    let error = result
        .err()
        .expect("the holder still owns the profile lock");
    assert!(error.is::<ProfileLockTimeout>());
    assert_eq!(
        error.to_string(),
        "Profile refresh lock timed out; retry later"
    );
}

#[test]
fn destination_configuration_errors_are_not_retryable_refresh_failures() {
    let _home = Home::new();
    runtime().block_on(async {
        let server = MockServer::start().await;
        Mock::given(path("/api/v1/auth/refresh"))
            .respond_with(ResponseTemplate::new(500))
            .expect(0)
            .mount(&server)
            .await;
        let mut auth = args(&server.uri());
        seed(&auth, &jwt(1), &jwt(Utc::now().timestamp() + 3600));
        for invalid in [false, true] {
            auth.base_url = Some(if invalid {
                "not a URL".into()
            } else {
                "https://another.example".into()
            });
            let error = ensure_session(&auth).await.unwrap_err();
            assert!(error.is::<ProfileDestinationError>());
            assert!(!error.is::<RefreshUnavailable>());
            let error = force_refresh_session(&auth).await.unwrap_err();
            assert!(error.is::<ProfileDestinationError>());
            assert!(!error.is::<RefreshUnavailable>());
            let outcome = refresh_captured_session(
                &reqwest::Client::new(),
                auth.profile.as_deref(),
                auth.base_url.as_deref().unwrap(),
                None,
                "old-access",
                true,
            )
            .await;
            if invalid {
                let error = outcome.access_token().unwrap_err();
                assert!(error.is::<ProfileDestinationError>());
                assert!(!error.is::<RefreshUnavailable>());
            } else {
                assert!(matches!(
                    outcome,
                    InFlightRefreshOutcome::Skipped("login_or_destination_changed")
                ));
            }
        }
        auth.base_url = Some(server.uri());
        save_base_url_for(auth.profile.as_deref(), "invalid saved URL").unwrap();
        let error = force_refresh_session(&auth).await.unwrap_err();
        assert!(error.is::<ProfileDestinationError>());
        server.verify().await;
    });
}

#[test]
fn saved_profile_helpers_share_auth_kind_destination_and_jwt_timestamps() {
    let _home = Home::new();
    runtime().block_on(async {
        let auth = args("https://nyxid.example/");
        seed(&auth, &jwt(1), &jwt(604801));
        let snapshot = snapshot_saved_profile(&auth).await.unwrap();
        assert!(!snapshot.agent);
        assert_eq!(
            jwt_iat_from_token(snapshot.refresh.as_deref().unwrap())
                .unwrap()
                .timestamp(),
            1
        );
        assert!(jwt_iat_from_token("opaque").is_none());
        assert!(
            validate_profile_destination(auth.profile.as_deref(), "https://nyxid.example").is_ok()
        );
        let dir = token_dir_for_profile(auth.profile.as_deref()).unwrap();
        std::fs::write(dir.join(agent_key::AUTH_KIND_FILE), "agent_key").unwrap();
        std::fs::write(dir.join(agent_key::TOKEN_FILE), "agent-credential").unwrap();
        let snapshot = snapshot_saved_profile(&auth).await.unwrap();
        assert!(snapshot.agent);
        assert_eq!(snapshot.access.as_deref(), Some("agent-credential"));
        std::fs::write(dir.join(agent_key::AUTH_KIND_FILE), [0xff]).unwrap();
        assert!(agent_key::read_auth_kind(auth.profile.as_deref()).is_err());
        let error = snapshot_saved_profile(&auth).await.err().unwrap();
        assert_eq!(error.code, "profile_storage_error");
    });
}

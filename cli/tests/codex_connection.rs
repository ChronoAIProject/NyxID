use serde_json::{Value, json};
use std::{path::Path, process::Output};
use tokio::process::Command;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{body_json, method, path},
};

const SECRET: &str = "sk-fixture-local-only-never-real";
const ACCOUNT: &str = "44a0a7a8-1d73-4d80-b49f-598a6c082211";
const EMAIL: &str = "fixture@example.test";
const CONNECTION: &str = "3d6e467d-e8ec-4b62-a02b-7d12c83b04ad";

fn status(connected: bool) -> Value {
    json!({"account_id":ACCOUNT,"account_email":EMAIL,
        "provider_id":"a98a2ba1-993e-4dd2-aaba-bf66265aa913","provider_slug":"openai",
        "connection":connected.then(||json!({"id":CONNECTION,"state_version":2})),
        "status":if connected {"saved"} else {"not_connected"},"service_id":null})
}

async fn run(home: &Path, source: &Path, base: &str, extra: &[&str]) -> Output {
    run_with_format(home, source, base, "json", extra).await
}

async fn run_with_format(
    home: &Path,
    source: &Path,
    base: &str,
    format: &str,
    extra: &[&str],
) -> Output {
    tokio::time::timeout(
        std::time::Duration::from_secs(10),
        Command::new(env!("CARGO_BIN_EXE_nyxid"))
            .args([
                "provider",
                "connect-codex",
                "--base-url",
                base,
                "--access-token",
                "fixture-account-token",
                "--output",
                format,
            ])
            .args(extra)
            .env("HOME", home)
            .env("CODEX_HOME", source)
            .env("CI", "1")
            .env("NYXID_SKIP_SKILL_SELF_HEAL", "1")
            .env("NYXID_NO_UPDATE_CHECK", "1")
            .env_remove("NYXID_TELEMETRY_DSN")
            .env_remove("NYXID_SHARE_ANALYTICS")
            .env_remove("NYXID_PROFILE")
            .env_remove("NYXID_API_KEY")
            .env_remove("NYXID_ACCESS_TOKEN")
            .stdin(std::process::Stdio::null())
            .kill_on_drop(true)
            .output(),
    )
    .await
    .unwrap()
    .unwrap()
}

fn assert_redacted(out: &Output) {
    assert!(!String::from_utf8_lossy(&out.stdout).contains(SECRET));
    assert!(!String::from_utf8_lossy(&out.stderr).contains(SECRET));
}

#[tokio::test]
async fn consent_reports_keep_identifiers_on_stdout_in_table_and_json_modes() {
    for format in ["table", "json"] {
        let home = tempfile::tempdir().unwrap();
        let source = tempfile::tempdir().unwrap();
        let server = MockServer::start().await;
        let instance = server.uri();
        Mock::given(method("GET"))
            .and(path("/api/v1/providers/codex-connection"))
            .respond_with(ResponseTemplate::new(200).set_body_json(status(true)))
            .expect(3)
            .mount(&server)
            .await;

        let assert_report = |out: &Output, outcome: &str, state: &str, message: &str| {
            let stdout = String::from_utf8_lossy(&out.stdout);
            let stderr = String::from_utf8_lossy(&out.stderr);
            assert!(out.status.success(), "{stderr}");
            assert_redacted(out);
            for identifier in [EMAIL, ACCOUNT, CONNECTION, instance.as_str()] {
                assert!(
                    stdout.contains(identifier),
                    "missing consent detail: {stdout}"
                );
                assert!(
                    !stderr.contains(identifier),
                    "identifier in diagnostics: {stderr}"
                );
            }
            if format == "json" {
                let result: Value = serde_json::from_slice(&out.stdout).unwrap();
                let mut expected = status(true);
                expected["status"] = state.into();
                assert_eq!(result["outcome"], outcome);
                assert_eq!(result["instance"], instance);
                assert_eq!(result["connection"], expected);
                assert!(result["message"].as_str().unwrap().contains(message));
            } else {
                assert!(stdout.contains(message));
                assert!(stdout.contains(&format!("Instance: {instance}")));
                assert!(stdout.contains(&format!("Account: {EMAIL} ({ACCOUNT})")));
                assert!(stdout.contains(&format!("Connection: {CONNECTION} (version 2)")));
                assert!(stdout.contains(&format!("State: {state}")));
            }
        };

        // The source is unavailable until both levels of consent have been reviewed.
        let review = run_with_format(home.path(), source.path(), &instance, format, &[]).await;
        assert_report(
            &review,
            "consent_required",
            "saved",
            "--approve-instance and --approve-account",
        );
        let replacement = run_with_format(
            home.path(),
            source.path(),
            &instance,
            format,
            &[
                "--approve-instance",
                &instance,
                "--approve-account",
                ACCOUNT,
            ],
        )
        .await;
        assert_report(
            &replacement,
            "replacement_consent_required",
            "saved",
            "--replace-connection and --replace-version",
        );
        let requests = server.received_requests().await.unwrap();
        assert_eq!(requests.len(), 2);
        assert!(requests.iter().all(|request| request.method == "GET"));

        let bytes = json!({"OPENAI_API_KEY":SECRET,"auth_mode":"apikey"}).to_string();
        std::fs::write(source.path().join("auth.json"), &bytes).unwrap();
        Mock::given(method("POST"))
            .and(path("/api/v1/providers/codex-connection"))
            .and(body_json(json!({
                "account_id":ACCOUNT,"api_key":SECRET,
                "expected_connection":{"id":CONNECTION,"state_version":2}
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(status(true)))
            .expect(1)
            .mount(&server)
            .await;
        let mut verified = status(true);
        verified["status"] = "usable".into();
        Mock::given(method("POST"))
            .and(path("/api/v1/providers/codex-connection/verify"))
            .and(body_json(json!({
                "connection":{"id":CONNECTION,"state_version":2},
                "model":"gpt-4.1-mini"
            })))
            .respond_with(ResponseTemplate::new(200).set_body_json(verified))
            .expect(1)
            .mount(&server)
            .await;
        let approved = run_with_format(
            home.path(),
            source.path(),
            &instance,
            format,
            &[
                "--approve-instance",
                &instance,
                "--approve-account",
                ACCOUNT,
                "--replace-connection",
                CONNECTION,
                "--replace-version",
                "2",
            ],
        )
        .await;
        assert_report(
            &approved,
            "verification_finished",
            "usable",
            "Connection status checked.",
        );
        assert_eq!(
            std::fs::read_to_string(source.path().join("auth.json")).unwrap(),
            bytes
        );
    }
}

#[tokio::test]
async fn skip_and_review_require_no_saved_home_or_credential_source() {
    let home = tempfile::NamedTempFile::new().unwrap();
    let server = MockServer::start().await;
    let skip = run(
        home.path(),
        Path::new("/missing-fixture-codex"),
        &server.uri(),
        &["--skip"],
    )
    .await;
    assert!(skip.status.success());
    assert!(server.received_requests().await.unwrap().is_empty());
    Mock::given(method("GET"))
        .and(path("/api/v1/providers/codex-connection"))
        .respond_with(ResponseTemplate::new(200).set_body_json(status(false)))
        .expect(1)
        .mount(&server)
        .await;
    let review = run(
        home.path(),
        Path::new("/missing-fixture-codex"),
        &server.uri(),
        &[],
    )
    .await;
    assert!(
        review.status.success(),
        "{}",
        String::from_utf8_lossy(&review.stderr)
    );
    assert_eq!(
        serde_json::from_slice::<Value>(&review.stdout).unwrap()["outcome"],
        "consent_required"
    );
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn approved_custom_file_upload_is_minimal_redacted_and_preserves_source() {
    let home = tempfile::tempdir().unwrap();
    let source = tempfile::tempdir().unwrap();
    let bytes = json!({"OPENAI_API_KEY":SECRET,"auth_mode":"apikey","unrelated":"do-not-upload"})
        .to_string();
    std::fs::write(source.path().join("auth.json"), &bytes).unwrap();
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/providers/codex-connection"))
        .respond_with(ResponseTemplate::new(200).set_body_json(status(false)))
        .expect(1)
        .mount(&server)
        .await;
    let mut saved = status(true);
    saved["account_email"] = SECRET.into();
    Mock::given(method("POST"))
        .and(path("/api/v1/providers/codex-connection"))
        .and(body_json(
            json!({"account_id":ACCOUNT,"api_key":SECRET,"expected_connection":null}),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(saved))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/providers/codex-connection/verify"))
        .respond_with(ResponseTemplate::new(502).set_body_string(SECRET))
        .expect(1)
        .mount(&server)
        .await;
    let out = run(
        home.path(),
        source.path(),
        &server.uri(),
        &[
            "--approve-instance",
            &server.uri(),
            "--approve-account",
            ACCOUNT,
        ],
    )
    .await;
    assert!(out.status.success());
    assert_redacted(&out);
    assert_eq!(
        serde_json::from_slice::<Value>(&out.stdout).unwrap()["outcome"],
        "saved_verification_pending"
    );
    assert_eq!(
        std::fs::read_to_string(source.path().join("auth.json")).unwrap(),
        bytes
    );
}

#[tokio::test]
async fn replacement_requires_the_reviewed_version_before_opening_credentials() {
    let home = tempfile::tempdir().unwrap();
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/providers/codex-connection"))
        .respond_with(ResponseTemplate::new(200).set_body_json(status(true)))
        .expect(1)
        .mount(&server)
        .await;
    let out = run(
        home.path(),
        Path::new("/missing-fixture-codex"),
        &server.uri(),
        &[
            "--approve-instance",
            &server.uri(),
            "--approve-account",
            ACCOUNT,
            "--replace-connection",
            CONNECTION,
            "--replace-version",
            "1",
        ],
    )
    .await;
    assert!(out.status.success());
    assert_eq!(
        serde_json::from_slice::<Value>(&out.stdout).unwrap()["outcome"],
        "replacement_consent_required"
    );
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn upload_redirects_and_reflected_errors_never_forward_or_print_credentials() {
    for code in [307, 308, 400] {
        let home = tempfile::tempdir().unwrap();
        let source = tempfile::tempdir().unwrap();
        std::fs::write(
            source.path().join("auth.json"),
            json!({"OPENAI_API_KEY":SECRET}).to_string(),
        )
        .unwrap();
        let server = MockServer::start().await;
        let other = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/api/v1/providers/codex-connection"))
            .respond_with(ResponseTemplate::new(200).set_body_json(status(false)))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/api/v1/providers/codex-connection"))
            .respond_with(
                ResponseTemplate::new(code)
                    .insert_header("location", other.uri())
                    .set_body_string(SECRET),
            )
            .expect(1)
            .mount(&server)
            .await;
        let out = run(
            home.path(),
            source.path(),
            &server.uri(),
            &[
                "--approve-instance",
                &server.uri(),
                "--approve-account",
                ACCOUNT,
            ],
        )
        .await;
        assert!(!out.status.success());
        assert_redacted(&out);
        assert!(other.received_requests().await.unwrap().is_empty());
    }
}

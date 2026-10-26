use std::process::Command;
use wiremock::{Mock, MockServer, ResponseTemplate, matchers::path};

#[test]
fn status_help_documents_lock_storage_and_configuration_exit_codes() {
    let output = Command::new(env!("CARGO_BIN_EXE_nyxid"))
        .args(["session", "status", "--help"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let help = String::from_utf8(output.stdout).unwrap();
    for text in [
        "4 = profile_lock_timeout",
        "1 = storage or configuration errors",
        "profile_storage_error",
        "profile_destination_mismatch",
    ] {
        assert!(help.contains(text), "missing {text}: {help}");
    }
}

#[tokio::test]
async fn status_process_prints_one_json_envelope_on_exit_three_and_never_refreshes() {
    use base64::Engine;
    let server = MockServer::start().await;
    Mock::given(path("/api/v1/auth/refresh"))
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&server)
        .await;
    let home = tempfile::tempdir().unwrap();
    let dir = home.path().join(".nyxid");
    std::fs::create_dir_all(&dir).unwrap();
    let access = format!(
        "e30.{}.sig",
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(
            serde_json::to_vec(&serde_json::json!({"exp":chrono::Utc::now().timestamp()+3600}))
                .unwrap()
        )
    );
    std::fs::write(dir.join("access_token"), &access).unwrap();
    std::fs::write(dir.join("refresh_token"), "e30.eyJpYXQiOjAsImV4cCI6MX0.sig").unwrap();
    Mock::given(path("/api/v1/sessions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
        .expect(1)
        .mount(&server)
        .await;
    let base = server.uri();
    let path = home.path().to_path_buf();
    let output = tokio::task::spawn_blocking(move || {
        Command::new(env!("CARGO_BIN_EXE_nyxid"))
            .args([
                "session",
                "status",
                "--output",
                "json",
                "--base-url",
                &base,
                "--access-token",
                "ignored",
            ])
            .env("HOME", path)
            .env_remove("NYXID_URL")
            .env("NYXID_TELEMETRY", "off")
            .output()
            .unwrap()
    })
    .await
    .unwrap();
    assert_eq!(
        output.status.code(),
        Some(3),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(report["error"], "refresh_token_expired");
    assert_eq!(report["server_check"], "no_current_session");
    assert_eq!(report["credential_overrides_ignored"], true);
    assert!(!String::from_utf8_lossy(&output.stderr).contains("refresh_token_expired"));
}

#[test]
fn status_table_process_prints_a_human_readable_view_without_json() {
    let home = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_nyxid"))
        .args(["session", "status", "--output", "table"])
        .env("HOME", home.path())
        .env_remove("NYXID_URL")
        .env("NYXID_TELEMETRY", "off")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(3));
    assert!(output.stdout.is_empty());
    let table = String::from_utf8(output.stderr).unwrap();
    assert!(table.contains("Profile"));
    assert!(table.contains("no_credentials"));
    assert!(table.contains("refresh_token_not_found"));
    assert!(table.contains("Run `nyxid login`"));
    assert!(!table.contains("\"local_state\":"));
}

#[test]
fn status_configuration_failure_prints_one_json_envelope_with_exit_one() {
    for destination in ["https://different.example", "invalid URL"] {
        let home = tempfile::tempdir().unwrap();
        let dir = home.path().join(".nyxid");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("base_url"), "https://saved.example").unwrap();
        let output = Command::new(env!("CARGO_BIN_EXE_nyxid"))
            .args([
                "session",
                "status",
                "--output",
                "json",
                "--base-url",
                destination,
            ])
            .env("HOME", home.path())
            .env_remove("NYXID_URL")
            .env("NYXID_TELEMETRY", "off")
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(1));
        let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(report["error"], "profile_destination_mismatch");
        assert_eq!(report["local_state"], "configuration_error");
        assert!(!String::from_utf8_lossy(&output.stderr).contains("profile_destination_mismatch"));
    }
}

use std::process::Command;
use wiremock::{Mock, MockServer, ResponseTemplate, matchers::path};

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

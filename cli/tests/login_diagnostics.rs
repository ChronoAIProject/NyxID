use serde_json::{Value, json};
use std::{path::Path, process::Output};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{method, path},
};

async fn run(home: &Path, base: &str, args: &[&str], ca: Option<&Path>) -> Output {
    let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_nyxid"));
    command
        .args(["login", "--base-url", base])
        .args(args)
        .env("HOME", home)
        .env("CI", "1")
        .env("DO_NOT_TRACK", "1")
        .env("NYXID_NO_UPDATE_CHECK", "1")
        .env("NYXID_SKIP_SKILL_SELF_HEAL", "1")
        .env_remove("NYXID_PROFILE")
        .env_remove("NYXID_URL")
        .env_remove("NYXID_CA_CERT")
        .env_remove("SSL_CERT_FILE")
        .env_remove("SSL_CERT_DIR")
        .env_remove("HTTPS_PROXY")
        .env_remove("HTTP_PROXY")
        .env_remove("ALL_PROXY")
        .env_remove("https_proxy")
        .env_remove("http_proxy")
        .env_remove("all_proxy")
        .env_remove("NYXID_LOGIN_NO_DEVICE_FALLBACK")
        .kill_on_drop(true);
    if let Some(ca) = ca {
        command.env("NYXID_CA_CERT", ca);
    }
    tokio::time::timeout(std::time::Duration::from_secs(15), command.output())
        .await
        .unwrap()
        .unwrap()
}
fn json_error(output: &Output, stage: &str, exit: i32) -> Value {
    assert_eq!(
        output.status.code(),
        Some(exit),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let value: Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(value["error"]["diagnostic"]["stage"], stage, "{value}");
    for secret in ["POLL_SECRET", "TOKEN_SECRET", "CODE_SECRET", "PROXY_SECRET"] {
        assert!(!String::from_utf8_lossy(&output.stdout).contains(secret));
        assert!(!String::from_utf8_lossy(&output.stderr).contains(secret));
    }
    value
}

#[tokio::test]
async fn device_and_agent_request_response_diagnostics_preserve_error_contract() {
    for flow in ["device/v2", "agent-key"] {
        for (status, body) in [
            (200, "not JSON POLL_SECRET"),
            (
                503,
                r#"{"device_code":"POLL_SECRET","message":"TOKEN_SECRET"}"#,
            ),
        ] {
            let server = MockServer::start().await;
            let home = tempfile::tempdir().unwrap();
            Mock::given(method("POST"))
                .and(path(format!("/api/v1/auth/{flow}/request")))
                .respond_with(ResponseTemplate::new(status).set_body_string(body))
                .mount(&server)
                .await;
            let mode = if flow == "agent-key" {
                "--agent-key"
            } else {
                "--device"
            };
            let output = run(
                home.path(),
                &server.uri(),
                &[mode, "--no-wait", "--output", "json"],
                None,
            )
            .await;
            let value = json_error(&output, "response", 19);
            assert_eq!(value["error"]["diagnostic"]["http_status"], status);
            let mut compatible = value;
            compatible["error"]
                .as_object_mut()
                .unwrap()
                .remove("diagnostic");
            assert_eq!(
                compatible,
                json!({"error":{"code":"login_unavailable","message":"Login could not complete. Check connectivity and resume later."}})
            );
            let text = run(home.path(), &server.uri(), &[mode, "--no-wait"], None).await;
            let text = String::from_utf8_lossy(&text.stderr);
            assert!(
                text.starts_with(
                    "Login could not complete. Check connectivity and resume later.\n"
                ),
                "{text}"
            );
            assert!(text.lines().any(|line| line == "  stage: response"));
            assert!(text.lines().any(|line| line.starts_with("  hint:")));
            assert!(!text.contains("POLL_SECRET"));
            assert!(!text.contains("TOKEN_SECRET"));
        }
    }
}

#[tokio::test]
async fn invalid_verification_urls_report_validation_without_server_secrets() {
    for uri in [
        "javascript:POLL_SECRET",
        "https://user:POLL_SECRET@example.com/login",
        "garbage POLL_SECRET",
    ] {
        let server = MockServer::start().await;
        let home = tempfile::tempdir().unwrap();
        Mock::given(path("/api/v1/auth/device/v2/request")).respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "device_code":"POLL_SECRET", "user_code":"ABCD-EFGH", "verification_uri":uri, "expires_in":600, "interval":5
        }))).mount(&server).await;
        json_error(
            &run(
                home.path(),
                &server.uri(),
                &["--device", "--no-wait", "--output", "json"],
                None,
            )
            .await,
            "validation",
            19,
        );
    }
}

#[tokio::test]
async fn ca_configuration_errors_report_config_without_network_io() {
    let home = tempfile::tempdir().unwrap();
    let missing = home.path().join("missing.pem");
    let value = json_error(
        &run(
            home.path(),
            "https://127.0.0.1:1",
            &["--device", "--no-wait", "--output", "json"],
            Some(&missing),
        )
        .await,
        "config",
        19,
    );
    assert!(
        value["error"]["diagnostic"]["hint"]
            .as_str()
            .unwrap()
            .contains("NYXID_CA_CERT")
    );
    assert!(value.to_string().contains("missing.pem"));
}

#[tokio::test]
async fn redeem_storage_and_invalid_delivery_have_safe_diagnostics() {
    for (delivery, stage, code) in [
        (
            json!({"access_token":"TOKEN_SECRET","refresh_token":"TOKEN_SECRET"}),
            "storage",
            21,
        ),
        (
            json!({"device_code":"POLL_SECRET","access_token":42}),
            "response",
            19,
        ),
    ] {
        let server = MockServer::start().await;
        let home = tempfile::tempdir().unwrap();
        std::fs::write(home.path().join(".nyxid"), "blocks profile directory").unwrap();
        Mock::given(path("/api/v1/auth/login-code/redeem"))
            .respond_with(ResponseTemplate::new(200).set_body_json(delivery))
            .mount(&server)
            .await;
        json_error(
            &run(
                home.path(),
                &server.uri(),
                &["--code", "CODE_SECRET", "--output", "json"],
                None,
            )
            .await,
            stage,
            code,
        );
    }
}

#[tokio::test]
async fn pending_record_storage_failure_reports_storage() {
    let server = MockServer::start().await;
    let home = tempfile::tempdir().unwrap();
    std::fs::write(home.path().join(".nyxid"), "blocks profile directory").unwrap();
    Mock::given(path("/api/v1/auth/device/v2/request")).respond_with(ResponseTemplate::new(200).set_body_json(json!({
        "device_code":"POLL_SECRET", "user_code":"ABCD-EFGH", "verification_uri":"https://example.com/login", "expires_in":600, "interval":5
    }))).mount(&server).await;
    json_error(
        &run(
            home.path(),
            &server.uri(),
            &["--device", "--no-wait", "--output", "json"],
            None,
        )
        .await,
        "storage",
        21,
    );
}

#[tokio::test]
async fn resume_poll_reports_status_and_preserves_secret_local_state() {
    for flow in ["device/v2", "agent-key"] {
        let server = MockServer::start().await;
        let home = tempfile::tempdir().unwrap();
        Mock::given(path(format!("/api/v1/auth/{flow}/request"))).respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "device_code":"POLL_SECRET", "user_code":"ABCD-EFGH", "verification_uri":"https://example.com/login", "expires_in":600, "interval":5
        }))).mount(&server).await;
        Mock::given(path(format!("/api/v1/auth/{flow}/poll")))
            .respond_with(
                ResponseTemplate::new(502).set_body_json(json!({"message":"POLL_SECRET"})),
            )
            .mount(&server)
            .await;
        let mode = if flow == "agent-key" {
            "--agent-key"
        } else {
            "--device"
        };
        let started = run(
            home.path(),
            &server.uri(),
            &[mode, "--no-wait", "--output", "json"],
            None,
        )
        .await;
        assert!(started.status.success());
        let started: Value = serde_json::from_slice(&started.stdout).unwrap();
        let id = started["request_id"].as_str().unwrap();
        let value = json_error(
            &run(
                home.path(),
                &server.uri(),
                &["resume", id, "--once", "--output", "json"],
                None,
            )
            .await,
            "response",
            19,
        );
        assert_eq!(value["error"]["diagnostic"]["http_status"], 502);
        let pending: Value = serde_json::from_slice(
            &std::fs::read(home.path().join(format!(".nyxid/pending-logins/{id}.json"))).unwrap(),
        )
        .unwrap();
        assert_eq!(pending["state"], "pending");
        assert_eq!(pending["device_code"], "POLL_SECRET");
    }
}

#[tokio::test]
async fn login_rejects_base_url_queries_without_echoing_secrets() {
    let home = tempfile::tempdir().unwrap();
    let output = run(
        home.path(),
        "https://example.com/?token=TOKEN_SECRET",
        &["--device", "--no-wait", "--output", "json"],
        None,
    )
    .await;
    assert_eq!(output.status.code(), Some(17));
    assert!(!String::from_utf8_lossy(&output.stdout).contains("TOKEN_SECRET"));
    assert!(!String::from_utf8_lossy(&output.stderr).contains("TOKEN_SECRET"));
}

use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use serde_json::{Value, json};
use std::{path::Path, process::Output, time::Duration};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{body_json, header, method, path},
};

fn jwt(exp: i64) -> String {
    format!(
        "{}.{}.{}",
        URL_SAFE_NO_PAD.encode(b"{}"),
        URL_SAFE_NO_PAD
            .encode(json!({"sub":uuid::Uuid::new_v4().to_string(),"exp":exp}).to_string()),
        uuid::Uuid::new_v4()
    )
}
fn seed(home: &Path, server: &MockServer, access: &str, refresh: &str) {
    let dir = home.join(".nyxid/profiles/refresh-test");
    std::fs::create_dir_all(&dir).unwrap();
    for (file, value) in [
        ("access_token", access),
        ("refresh_token", refresh),
        ("base_url", &server.uri()),
    ] {
        std::fs::write(dir.join(file), value).unwrap();
    }
}
async fn run(home: &Path, server: &MockServer, args: &[&str]) -> Output {
    let mut cmd = tokio::process::Command::new(env!("CARGO_BIN_EXE_nyxid"));
    cmd.args(args)
        .args(["--profile", "refresh-test", "--base-url", &server.uri()])
        .env("HOME", home)
        .env("CI", "1")
        .env("DO_NOT_TRACK", "1")
        .env("NYXID_NO_UPDATE_CHECK", "1")
        .env("NYXID_SKIP_SKILL_SELF_HEAL", "1")
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true);
    for variable in [
        "NYXID_ACCESS_TOKEN",
        "NYXID_API_KEY",
        "NYXID_PROFILE",
        "NYXID_URL",
        "NYXID_BASE_URL",
        "NYXID_TELEMETRY_DSN",
    ] {
        cmd.env_remove(variable);
    }
    tokio::time::timeout(Duration::from_secs(30), cmd.output())
        .await
        .expect("CLI must not prompt or hang")
        .unwrap()
}
fn output_json(output: &Output) -> Value {
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[tokio::test]
async fn session_status_and_list_json_expose_expiries_without_tokens() {
    let server = MockServer::start().await;
    let home = tempfile::tempdir().unwrap();
    let now = chrono::Utc::now();
    let access = jwt(now.timestamp() + 3600);
    let refresh = jwt(now.timestamp() + 604800);
    seed(home.path(), &server, &access, &refresh);
    let expiry = (now + chrono::Duration::days(2)).to_rfc3339();
    let refresh_expiry = (now + chrono::Duration::days(7)).to_rfc3339();
    Mock::given(method("GET")).and(path("/api/v1/sessions"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([{"id":"session-metadata", "is_current":true, "expires_at":expiry, "session_expires_at":expiry, "refresh_expires_at":refresh_expiry}]))).mount(&server).await;
    for subcommand in ["status", "list"] {
        let output = run(
            home.path(),
            &server,
            &["session", subcommand, "--output", "json"],
        )
        .await;
        let body = output_json(&output);
        let row = if subcommand == "list" {
            &body[0]
        } else {
            &body
        };
        assert_eq!(row["session_expires_at"], expiry);
        assert_eq!(row["refresh_expires_at"], refresh_expiry);
        assert!(
            row["expiry_warning"]
                .as_str()
                .unwrap()
                .contains("absolute expiry")
        );
        let text = String::from_utf8_lossy(&output.stdout);
        assert!(!text.contains(&access));
        assert!(!text.contains(&refresh));
        let output = run(home.path(), &server, &["session", subcommand]).await;
        assert!(output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("--agent-key"));
    }
}

#[tokio::test]
async fn concurrent_oracle_commands_share_one_locked_refresh_per_profile() {
    let server = MockServer::start().await;
    let home = tempfile::tempdir().unwrap();
    let now = chrono::Utc::now().timestamp();
    let old = jwt(now - 100);
    let refresh = jwt(now + 604800);
    let new = jwt(now + 3600);
    let next = jwt(now + 604800);
    seed(home.path(), &server, &old, &refresh);
    Mock::given(method("POST"))
        .and(path("/api/v1/auth/refresh"))
        .and(body_json(json!({"refresh_token":refresh})))
        .respond_with(
            ResponseTemplate::new(200)
                .set_delay(Duration::from_millis(250))
                .set_body_json(json!({"access_token":new,"refresh_token":next,"expires_in":3600})),
        )
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/v1/oracle/pools"))
        .and(header("authorization", format!("Bearer {new}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!([])))
        .expect(8)
        .mount(&server)
        .await;
    let outputs = futures::future::join_all((0..8).map(|_| {
        run(
            home.path(),
            &server,
            &["oracle", "pool", "list", "--output", "json"],
        )
    }))
    .await;
    for output in outputs {
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let stored = std::fs::read_to_string(
        home.path()
            .join(".nyxid/profiles/refresh-test/refresh_token"),
    )
    .unwrap();
    assert!(stored == next);
}

#[tokio::test]
async fn structured_refresh_rejections_keep_exit_three_and_require_explicit_login() {
    for (reason, code) in [
        ("session_expired", "session_expired"),
        ("session_revoked", "session_revoked"),
        ("refresh_reused", "refresh_token_reused"),
        ("refresh_revoked", "refresh_token_revoked"),
    ] {
        let server = MockServer::start().await;
        let home = tempfile::tempdir().unwrap();
        let now = chrono::Utc::now().timestamp();
        seed(home.path(), &server, &jwt(now - 100), &jwt(now + 604800));
        Mock::given(method("POST")).and(path("/api/v1/auth/refresh"))
            .respond_with(ResponseTemplate::new(401).set_body_json(json!({"error":"unauthorized","error_code":1001,"message":"Unauthorized: Session expired or revoked","details":{"reason":reason}})))
            .expect(1).mount(&server).await;
        let output = run(
            home.path(),
            &server,
            &["oracle", "pool", "list", "--output", "json"],
        )
        .await;
        assert_eq!(output.status.code(), Some(3));
        let body: Value = serde_json::from_slice(&output.stderr).unwrap();
        assert_eq!(body["error"], code);
        assert_eq!(body["action"], "login");
        assert!(
            !body["message"]
                .as_str()
                .unwrap()
                .contains("refresh token has expired")
        );
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }
}

#[tokio::test]
async fn in_flight_401_keeps_structured_refresh_reason_and_never_retries_other_identity() {
    let server = MockServer::start().await;
    let home = tempfile::tempdir().unwrap();
    let now = chrono::Utc::now().timestamp();
    seed(home.path(), &server, &jwt(now + 3600), &jwt(now + 604800));
    Mock::given(method("GET")).and(path("/api/v1/oracle/pools"))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({"error":"unauthorized","error_code":1001,"message":"Unauthorized: Session expired or revoked"})))
        .expect(1).mount(&server).await;
    Mock::given(method("POST")).and(path("/api/v1/auth/refresh"))
        .respond_with(ResponseTemplate::new(401).set_body_json(json!({"error":"unauthorized","error_code":1001,"message":"Unauthorized: Session expired or revoked","details":{"reason":"session_expired"}})))
        .expect(1).mount(&server).await;
    let output = run(
        home.path(),
        &server,
        &["oracle", "pool", "list", "--output", "json"],
    )
    .await;
    assert_eq!(output.status.code(), Some(3));
    let body: Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(body["error"], "session_expired");
    assert_eq!(server.received_requests().await.unwrap().len(), 2);
}

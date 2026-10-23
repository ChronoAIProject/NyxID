use base64::Engine;
use std::path::Path;
use wiremock::{Mock, MockServer, ResponseTemplate};

fn seed(home: &Path, destination: &str) {
    let dir = home.join(".nyxid");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("base_url"), destination).unwrap();
    for (file, expiry) in [
        ("access_token", 1),
        ("refresh_token", chrono::Utc::now().timestamp() + 3600),
    ] {
        let token = format!(
            "e30.{}.sig",
            base64::engine::general_purpose::URL_SAFE_NO_PAD
                .encode(serde_json::to_vec(&serde_json::json!({"exp": expiry})).unwrap())
        );
        std::fs::write(dir.join(file), token).unwrap();
    }
}

async fn run(home: &Path, destination: &str, subcommand: &str, ca: Option<&Path>) {
    let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_nyxid"));
    command
        .args([
            "session",
            subcommand,
            "--base-url",
            destination,
            "--output",
            "json",
        ])
        .env("HOME", home)
        .env("CI", "1")
        .env_remove("NYXID_URL")
        .env_remove("NYXID_PROFILE")
        .env_remove("NYXID_ACCESS_TOKEN")
        .env_remove("NYXID_API_KEY")
        .env_remove("NYXID_CA_CERT")
        .env_remove("SSL_CERT_FILE")
        .env_remove("SSL_CERT_DIR")
        .env("DO_NOT_TRACK", "1")
        .env("NYXID_NO_UPDATE_CHECK", "1")
        .env("NYXID_SKIP_SKILL_SELF_HEAL", "1")
        .kill_on_drop(true);
    if let Some(ca) = ca {
        command.env("NYXID_CA_CERT", ca);
    }
    let output = tokio::time::timeout(std::time::Duration::from_secs(10), command.output())
        .await
        .expect("deterministic configuration errors must fail without refresh retries")
        .unwrap();
    assert_eq!(output.status.code(), Some(1), "{subcommand}: {output:?}");
    let error = String::from_utf8_lossy(&output.stderr);
    assert!(!error.contains("refresh_unavailable"), "{error}");
    if ca.is_some() {
        assert!(error.contains("NYXID_CA_CERT"), "{error}");
    } else if subcommand == "status" {
        let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(report["error"], "profile_destination_mismatch");
    } else {
        assert!(error.contains("Invalid profile destination"), "{error}");
    }
}

#[tokio::test]
async fn disallowed_matching_http_destination_exits_one_before_preflight_or_explicit_refresh() {
    let server = MockServer::start().await;
    Mock::given(wiremock::matchers::any())
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&server)
        .await;
    for destination in [
        "http://remote.example".to_owned(),
        server.uri().replace("127.0.0.1", "0.0.0.0"),
    ] {
        let home = tempfile::tempdir().unwrap();
        seed(home.path(), &destination);
        for subcommand in ["list", "refresh", "status"] {
            run(home.path(), &destination, subcommand, None).await;
        }
    }
    assert!(server.received_requests().await.unwrap().is_empty());
}

#[tokio::test]
async fn invalid_tls_configuration_exits_one_before_preflight_or_explicit_refresh() {
    let server = MockServer::start().await;
    Mock::given(wiremock::matchers::any())
        .respond_with(ResponseTemplate::new(500))
        .expect(0)
        .mount(&server)
        .await;
    let home = tempfile::tempdir().unwrap();
    seed(home.path(), &server.uri());
    let ca = home.path().join("missing-ca.pem");
    for subcommand in ["list", "refresh"] {
        run(home.path(), &server.uri(), subcommand, Some(&ca)).await;
    }
    assert!(server.received_requests().await.unwrap().is_empty());
}

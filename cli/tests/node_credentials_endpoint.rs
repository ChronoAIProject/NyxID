//! #2126: migration is explicit; the established local Add flow stays intact.
use std::{path::PathBuf, process::Stdio, time::Duration};

use serde_json::json;
use tokio::{io::AsyncWriteExt, process::Command};
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{body_json, header, method, path},
};

struct Harness {
    home: tempfile::TempDir,
    config_dir: PathBuf,
    node_id: String,
}

impl Harness {
    async fn new(server: &MockServer) -> Self {
        let home = tempfile::tempdir().unwrap();
        let config_dir = home.path().join("node");
        std::fs::create_dir(&config_dir).unwrap();
        let node_id = uuid::Uuid::new_v4().to_string();
        let config = json!({
            "server": {"url": format!("{}/api/v1/nodes/ws", server.uri().replacen("http://", "ws://", 1))},
            "node": {"id": node_id, "auth_token_encrypted": ""},
            "storage_backend": "file",
        });
        std::fs::write(
            config_dir.join("config.toml"),
            toml::to_string(&config).unwrap(),
        )
        .unwrap();
        let harness = Self {
            home,
            config_dir,
            node_id,
        };
        // Seed a real encrypted local credential without an owner login.
        let output = harness
            .command("add")
            .args([
                "--service",
                "monitor",
                "--url",
                "http://old-monitor",
                "--header",
                "Authorization",
                "--value",
            ])
            .arg(uuid::Uuid::new_v4().to_string())
            .output()
            .await
            .unwrap();
        assert!(
            output.status.success(),
            "local credential setup must succeed"
        );
        harness
    }

    fn command(&self, action: &str) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_nyxid"));
        command
            .args(["node", "credentials", "--config"])
            .arg(&self.config_dir)
            .arg(action)
            .env_clear()
            .env("HOME", self.home.path())
            .env("XDG_CONFIG_HOME", self.home.path())
            .env("DO_NOT_TRACK", "1")
            .env("NYXID_NO_UPDATE_CHECK", "1")
            .env("NYXID_SKIP_SKILL_SELF_HEAL", "1")
            .current_dir(self.home.path())
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        command
    }

    fn config_bytes(&self) -> Vec<u8> {
        std::fs::read(self.config_dir.join("config.toml")).unwrap()
    }
}

#[tokio::test]
async fn add_url_with_existing_credential_still_prompts_without_owner_login_or_network() {
    let server = MockServer::start().await;
    let harness = Harness::new(&server).await;
    let before = harness.config_bytes();
    let mut child = harness
        .command("add")
        .args(["--service", "monitor", "--url", "http://new-monitor"])
        .stdin(Stdio::piped())
        .spawn()
        .unwrap();
    // Stop at the first interactive prompt, before any secret prompt. This
    // exercises the exact old invocation without needing a terminal or secret IO.
    child
        .stdin
        .take()
        .unwrap()
        .write_all(b"invalid-auth-method\n")
        .await
        .unwrap();
    let output = tokio::time::timeout(Duration::from_secs(30), child.wait_with_output())
        .await
        .unwrap()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("Auth method (header/query_param)"));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("Invalid choice 'invalid-auth-method'")
    );
    assert!(server.received_requests().await.unwrap().is_empty());
    assert!(
        before == harness.config_bytes(),
        "no config write before completing the prompt"
    );
}

#[tokio::test]
async fn set_url_uses_owner_auth_and_metadata_only_put_without_local_save_including_clear() {
    let server = MockServer::start().await;
    let harness = Harness::new(&server).await;
    let before = harness.config_bytes();
    let owner_token = uuid::Uuid::new_v4().to_string();
    Mock::given(method("GET"))
        .and(path("/api/v1/keys"))
        .and(header("authorization", format!("Bearer {owner_token}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"keys": [{
            "id": "existing-service", "slug": "monitor", "node_id": harness.node_id,
            "credential_type": "node_managed", "endpoint_url": "http://old-monitor",
        }]})))
        .expect(2)
        .mount(&server)
        .await;
    for (target, status) in [("http://new-monitor", "offline"), ("", "synced")] {
        Mock::given(method("PUT")).and(path("/api/v1/keys/existing-service"))
            .and(header("authorization", format!("Bearer {owner_token}")))
            .and(body_json(json!({"endpoint_url": target})))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "endpoint_url": target,
                "node_target_sync": [{"service_id": "existing-service", "status": status, "target_revision": 1}],
            }))).expect(1).mount(&server).await;
        let output = tokio::time::timeout(
            Duration::from_secs(30),
            harness
                .command("set-url")
                .args(["--service", "monitor", "--url", target])
                .env("NYXID_ACCESS_TOKEN", &owner_token)
                .output(),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(output.status.success(), "set-url must succeed");
        assert!(String::from_utf8_lossy(&output.stdout).contains("local credential preserved"));
        if status == "offline" {
            assert!(String::from_utf8_lossy(&output.stderr).contains("node target sync offline"));
        }
        assert!(
            before == harness.config_bytes(),
            "only the node push may save local metadata"
        );
    }
}

#[tokio::test]
async fn set_url_validates_locally_before_owner_auth_or_network() {
    let server = MockServer::start().await;
    let harness = Harness::new(&server).await;
    let before = harness.config_bytes();
    let output = harness
        .command("set-url")
        .args(["--service", "monitor", "--url", "ftp://new-monitor"])
        .output()
        .await
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Target URL must be HTTP(S) or SSH"));
    assert!(server.received_requests().await.unwrap().is_empty());
    assert!(
        before == harness.config_bytes(),
        "invalid target must not alter the config"
    );
}

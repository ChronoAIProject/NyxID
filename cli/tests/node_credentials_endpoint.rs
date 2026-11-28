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
    credential_value: String,
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
            credential_value: uuid::Uuid::new_v4().to_string(),
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
            .arg(&harness.credential_value)
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
        let mut command = self.node_command("credentials");
        command.arg(action);
        command
    }

    fn node_command(&self, action: &str) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_nyxid"));
        command
            .args(["node", action, "--config"])
            .arg(&self.config_dir)
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
async fn agent_reports_legacy_credential_count_and_warns_once_at_start() {
    let server = MockServer::start().await;
    let harness = Harness::new(&server).await;
    let mut config: serde_json::Value =
        toml::from_str(std::str::from_utf8(&harness.config_bytes()).unwrap()).unwrap();
    let bound = config["credentials"]["monitor"].clone();
    // The encrypted fixture value also serves as the node's local test token.
    config["node"]["auth_token_encrypted"] = bound["header_value_encrypted"].clone();
    config["credentials"]["empty"] = bound.clone();
    config["credentials"]["empty"]["destination_binding"]["destinations"] = json!([]);
    let mut legacy = bound;
    legacy
        .as_object_mut()
        .unwrap()
        .remove("destination_binding");

    for count in [0, 2] {
        if count == 2 {
            config["credentials"]["legacy-one"] = legacy.clone();
            config["credentials"]["legacy-two"] = legacy.clone();
        }
        std::fs::write(
            harness.config_dir.join("config.toml"),
            toml::to_string(&config).unwrap(),
        )
        .unwrap();
        let status = harness.node_command("agent-status").output().await.unwrap();
        assert!(status.status.success());
        let label = format!("{count} legacy (unbound)");
        assert!(String::from_utf8(status.stdout).unwrap().contains(&label));

        // Wait through a reconnect to catch warnings emitted on each connection.
        let connections = Mock::given(path("/api/v1/nodes/ws"))
            .respond_with(ResponseTemplate::new(503))
            .expect(2..)
            .mount_as_scoped(&server)
            .await;
        let mut agent = harness.node_command("start").spawn().unwrap();
        tokio::time::timeout(Duration::from_secs(30), connections.wait_until_satisfied())
            .await
            .expect("agent must retry the local connection");
        agent.start_kill().unwrap();
        let output = agent.wait_with_output().await.unwrap();
        let log = format!(
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(!log.contains(&harness.credential_value));
        assert!(!log.contains(legacy["header_value_encrypted"].as_str().unwrap()));
        let warnings: Vec<_> = log
            .lines()
            .filter(|line| line.contains("legacy (unbound)"))
            .collect();
        if count == 0 {
            assert!(warnings.is_empty());
        } else {
            let [warning] = warnings.as_slice() else {
                panic!("expected one legacy credential warning")
            };
            for text in [
                "WARN",
                &label,
                "nyxid node credentials list",
                "nyxid node credentials rebind --service <slug> --url <base>",
            ] {
                assert!(warning.contains(text));
            }
        }
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

#[tokio::test]
async fn local_rebind_replaces_the_list_without_owner_login_or_secret_access() {
    let server = MockServer::start().await;
    let harness = Harness::new(&server).await;
    let original: toml::Value =
        toml::from_str(std::str::from_utf8(&harness.config_bytes()).unwrap()).unwrap();
    // Install the representation written by an older binary.
    let mut legacy = original.clone();
    legacy["credentials"]["monitor"]
        .as_table_mut()
        .unwrap()
        .remove("destination_binding");
    std::fs::write(
        harness.config_dir.join("config.toml"),
        toml::to_string(&legacy).unwrap(),
    )
    .unwrap();
    let list = harness.command("list").output().await.unwrap();
    assert!(list.status.success());
    assert!(
        String::from_utf8(list.stdout)
            .unwrap()
            .contains("legacy (unbound)")
    );
    let result = harness
        .command("rebind")
        .args([
            "--service",
            "monitor",
            "--url",
            "https://new.example/api",
            "--url",
            "https://editor.example",
        ])
        .output()
        .await
        .unwrap();
    assert!(result.status.success());
    let list = harness.command("list").output().await.unwrap();
    assert!(list.status.success());
    let list = String::from_utf8(list.stdout).unwrap();
    assert!(list.contains("https://editor.example/") && list.contains("https://new.example/api"));
    let saved: toml::Value =
        toml::from_str(std::str::from_utf8(&harness.config_bytes()).unwrap()).unwrap();
    for field in ["target_url", "header_value_encrypted"] {
        assert!(
            saved["credentials"]["monitor"][field] == original["credentials"]["monitor"][field]
        );
    }
    assert!(server.received_requests().await.unwrap().is_empty());
}

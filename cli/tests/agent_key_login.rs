use serde_json::{Value, json};
use std::{path::Path, process::Output};
use tokio::process::Command;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{body_partial_json, header, method, path},
};

const SECRET: &str = "nyxid_ag_0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn identity() -> Value {
    json!({"api_key": {"id": "key-id", "name": "Home Agent", "key_prefix": "nyxid_ag_01234567", "owner_type": "personal", "owner_id": "owner-id", "owner_name": "Human", "scopes": "read proxy", "allow_all_services": false, "allow_all_nodes": false, "allowed_service_ids": ["service-id"], "allowed_node_ids": [], "expires_at": null, "rate_limit_per_second": 10, "rate_limit_burst": 20, "platform": "generic", "created_now": false}, "credential_id": "credential-id", "credential_expires_at": null, "label": "workstation"})
}

fn command(home: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_nyxid"));
    command
        .env("HOME", home)
        .env("CI", "1")
        .env("NYXID_NO_UPDATE_CHECK", "1")
        .env("NYXID_SKIP_SKILL_SELF_HEAL", "1")
        .env_remove("NYXID_API_KEY")
        .env_remove("NYXID_ACCESS_TOKEN")
        .env_remove("NYXID_PROFILE")
        .env_remove("NYXID_BASE_URL")
        .env_remove("NYXID_TELEMETRY_DSN")
        .env_remove("NYXID_SHARE_ANALYTICS")
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true);
    command
}

async fn run(home: &Path, args: &[&str]) -> Output {
    tokio::time::timeout(
        std::time::Duration::from_secs(45),
        command(home).args(args).output(),
    )
    .await
    .expect("CLI timed out or prompted")
    .expect("run CLI")
}

fn output_text(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn profile(home: &Path) -> std::path::PathBuf {
    home.join(".nyxid/profiles/home-agent")
}

fn seed_profile(home: &Path, server: &MockServer) {
    let dir = profile(home);
    std::fs::create_dir_all(&dir).unwrap();
    for (file, content) in [
        ("auth_kind", "agent_key".to_string()),
        ("token", SECRET.to_string()),
        ("agent_key.json", identity().to_string()),
        ("base_url", server.uri()),
    ] {
        std::fs::write(dir.join(file), content).unwrap();
    }
}

async fn mount_request(server: &MockServer) {
    Mock::given(method("POST")).and(path("/api/v1/auth/agent-key/request")).and(body_partial_json(json!({"requested_profile": "home-agent"})))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"device_code": "nyx_akl_requester-only", "user_code": "ABCD-EFGH", "verification_uri": format!("{}/login/agent-key", server.uri()), "expires_in": 600, "interval": 5}))).expect(1).mount(server).await;
}

#[tokio::test]
async fn full_login_stores_only_restricted_credential_and_safe_metadata() {
    let server = MockServer::start().await;
    let home = tempfile::tempdir().unwrap();
    let dir = profile(home.path());
    std::fs::create_dir_all(&dir).unwrap();
    for file in ["access_token", "refresh_token", "user_id"] {
        std::fs::write(dir.join(file), "stale-human-session").unwrap();
    }
    mount_request(&server).await;
    let mut delivery = identity();
    delivery["credential"] = json!(SECRET);
    Mock::given(method("POST"))
        .and(path("/api/v1/auth/agent-key/poll"))
        .and(body_partial_json(
            json!({"device_code": "nyx_akl_requester-only"}),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(delivery))
        .expect(1)
        .mount(&server)
        .await;
    let output = run(
        home.path(),
        &[
            "login",
            "--agent-key",
            "--profile",
            "home-agent",
            "--base-url",
            &server.uri(),
        ],
    )
    .await;
    assert!(output.status.success(), "{}", output_text(&output));
    let text = output_text(&output);
    assert!(text.contains("Authentication: Agent Key"));
    assert!(text.contains("ABCD-EFGH"));
    assert!(!text.contains("user_code="));
    assert!(!text.contains(SECRET));
    assert!(!text.contains("nyx_akl_requester-only"));
    assert_eq!(std::fs::read_to_string(dir.join("token")).unwrap(), SECRET);
    assert_eq!(
        std::fs::read_to_string(dir.join("auth_kind")).unwrap(),
        "agent_key"
    );
    let metadata = std::fs::read_to_string(dir.join("agent_key.json")).unwrap();
    assert!(!metadata.contains(SECRET));
    assert_eq!(
        serde_json::from_str::<Value>(&metadata).unwrap()["api_key"]["id"],
        "key-id"
    );
    for file in ["access_token", "refresh_token", "user_id"] {
        assert!(!dir.join(file).exists());
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        for file in ["token", "auth_kind", "agent_key.json"] {
            assert_eq!(
                std::fs::metadata(dir.join(file))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o600
            );
        }
    }
}

#[tokio::test]
async fn pending_and_slow_down_retry_with_the_server_interval() {
    let server = MockServer::start().await;
    let home = tempfile::tempdir().unwrap();
    mount_request(&server).await;
    let polls = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let received = polls.clone();
    Mock::given(method("POST"))
        .and(path("/api/v1/auth/agent-key/poll"))
        .respond_with(move |_: &wiremock::Request| {
            let mut times = received.lock().unwrap();
            times.push(std::time::Instant::now());
            match times.len() {
                1 | 2 => ResponseTemplate::new(400).set_body_json(json!({
                    "error_code": if times.len() == 1 {11902} else {11903},
                    "error": "waiting", "message": "Waiting",
                })),
                _ => {
                    let mut delivery = identity();
                    delivery["credential"] = json!(SECRET);
                    ResponseTemplate::new(200).set_body_json(delivery)
                }
            }
        })
        .expect(3)
        .mount(&server)
        .await;
    let output = run(
        home.path(),
        &[
            "login",
            "--agent-key",
            "--profile",
            "home-agent",
            "--base-url",
            &server.uri(),
        ],
    )
    .await;
    assert!(output.status.success(), "{}", output_text(&output));
    let times = polls.lock().unwrap();
    assert_eq!(times.len(), 3);
    assert!(times[1].duration_since(times[0]) >= std::time::Duration::from_secs(5));
    assert!(times[2].duration_since(times[1]) >= std::time::Duration::from_secs(10));
    assert_eq!(
        std::fs::read_to_string(profile(home.path()).join("token")).unwrap(),
        SECRET
    );
    assert!(!output_text(&output).contains(SECRET));
}

#[tokio::test]
async fn unsupported_backend_never_falls_back_to_an_account_session() {
    let server = MockServer::start().await;
    let home = tempfile::tempdir().unwrap();
    Mock::given(method("POST"))
        .and(path("/api/v1/auth/agent-key/request"))
        .respond_with(ResponseTemplate::new(404))
        .expect(1)
        .mount(&server)
        .await;
    let output = run(
        home.path(),
        &[
            "login",
            "--agent-key",
            "--profile",
            "home-agent",
            "--base-url",
            &server.uri(),
        ],
    )
    .await;
    assert!(!output.status.success());
    assert!(output_text(&output).contains("This NyxID backend doesn't support Agent Key login"));
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
    assert!(!profile(home.path()).join("access_token").exists());
}

#[tokio::test]
async fn every_terminal_poll_error_exits_without_storing_a_session() {
    let cases = [11900, 11901, 11904, 11905, 11906, 11907, 11908, 11909];
    futures::future::join_all(cases.into_iter().map(|code| async move {
        let server = MockServer::start().await;
        let home = tempfile::tempdir().unwrap();
        mount_request(&server).await;
        Mock::given(method("POST"))
            .and(path("/api/v1/auth/agent-key/poll"))
            .respond_with(ResponseTemplate::new(400).set_body_json(
                json!({"error_code": code, "error": "agent_key_error", "message": "Failure"}),
            ))
            .expect(1)
            .mount(&server)
            .await;
        let output = run(
            home.path(),
            &[
                "login",
                "--agent-key",
                "--profile",
                "home-agent",
                "--base-url",
                &server.uri(),
            ],
        )
        .await;
        assert!(!output.status.success(), "error {code}");
        assert!(!profile(home.path()).join("token").exists());
        assert_eq!(server.received_requests().await.unwrap().len(), 2);
    }))
    .await;
}

#[tokio::test]
async fn whoami_status_and_rejected_credentials_never_refresh_or_prompt() {
    let server = MockServer::start().await;
    let home = tempfile::tempdir().unwrap();
    seed_profile(home.path(), &server);
    Mock::given(method("GET"))
        .and(path("/api/v1/auth/agent-key/self"))
        .and(header("Authorization", format!("Bearer {SECRET}")))
        .respond_with(ResponseTemplate::new(200).set_body_json(identity()))
        .expect(3)
        .mount(&server)
        .await;
    for command in ["whoami", "status"] {
        let output = run(
            home.path(),
            &[
                command,
                "--profile",
                "home-agent",
                "--base-url",
                &server.uri(),
            ],
        )
        .await;
        assert!(output.status.success(), "{}", output_text(&output));
        assert!(output_text(&output).contains("Authentication: Agent Key"));
        assert!(!output_text(&output).contains(SECRET));
    }
    let json_output = run(
        home.path(),
        &[
            "whoami",
            "--profile",
            "home-agent",
            "--base-url",
            &server.uri(),
            "--output",
            "json",
        ],
    )
    .await;
    assert_eq!(
        serde_json::from_slice::<Value>(&json_output.stdout).unwrap()["auth"]["kind"],
        "agent_key"
    );
    server.verify().await;
    server.reset().await;
    std::fs::write(
        profile(home.path()).join("refresh_token"),
        "stale-refresh-token",
    )
    .unwrap();
    Mock::given(method("GET"))
        .and(path("/api/v1/auth/agent-key/self"))
        .respond_with(ResponseTemplate::new(401).set_body_json(
            json!({"error_code": 1001, "error": "unauthorized", "message": "revoked"}),
        ))
        .expect(1)
        .mount(&server)
        .await;
    let rejected = run(
        home.path(),
        &[
            "whoami",
            "--profile",
            "home-agent",
            "--base-url",
            &server.uri(),
        ],
    )
    .await;
    assert!(!rejected.status.success());
    assert!(output_text(&rejected).contains("Your Agent Key credential was rejected"));
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
    let refresh = run(
        home.path(),
        &[
            "session",
            "refresh",
            "--profile",
            "home-agent",
            "--base-url",
            &server.uri(),
        ],
    )
    .await;
    assert_eq!(refresh.status.code(), Some(3));
    assert!(output_text(&refresh).contains("Agent Key sessions do not refresh"));
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

#[tokio::test]
async fn logout_revokes_then_clears_locally_even_when_server_rejects() {
    for status in [200, 500] {
        let server = MockServer::start().await;
        let home = tempfile::tempdir().unwrap();
        seed_profile(home.path(), &server);
        Mock::given(method("DELETE"))
            .and(path("/api/v1/auth/agent-key/self"))
            .and(header("Authorization", format!("Bearer {SECRET}")))
            .respond_with(ResponseTemplate::new(status).set_body_json(json!({"ok": true})))
            .expect(1)
            .mount(&server)
            .await;
        let output = run(
            home.path(),
            &[
                "logout",
                "--profile",
                "home-agent",
                "--base-url",
                &server.uri(),
            ],
        )
        .await;
        assert!(output.status.success(), "{}", output_text(&output));
        assert!(output_text(&output).contains(if status == 200 {
            "revoked on the server"
        } else {
            "could not be confirmed"
        }));
        for file in ["token", "auth_kind", "agent_key.json"] {
            assert!(!profile(home.path()).join(file).exists());
        }
    }
}

#[tokio::test]
async fn explicit_credentials_override_saved_agent_key_profile() {
    let server = MockServer::start().await;
    let home = tempfile::tempdir().unwrap();
    seed_profile(home.path(), &server);
    Mock::given(method("GET"))
        .and(path("/api/v1/users/me"))
        .and(header("Authorization", "Bearer explicit-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"id": "explicit-user"})))
        .expect(1)
        .mount(&server)
        .await;
    let output = run(
        home.path(),
        &[
            "whoami",
            "--profile",
            "home-agent",
            "--base-url",
            &server.uri(),
            "--access-token",
            "explicit-token",
            "--output",
            "json",
        ],
    )
    .await;
    assert!(output.status.success(), "{}", output_text(&output));
    assert_eq!(
        serde_json::from_slice::<Value>(&output.stdout).unwrap()["id"],
        "explicit-user"
    );
}

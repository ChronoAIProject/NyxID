use serde_json::{Value, json};
use std::{path::Path, process::Output};
use tokio::process::Command;
use wiremock::{
    Mock, MockServer, ResponseTemplate,
    matchers::{body_partial_json, method, path},
};

const POLL_SECRET: &str = "nyx_adc_fixture-poller-only";
const KEY_SECRET: &str =
    "nyxid_ag_0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

#[tokio::test]
async fn plain_login_environment_opt_out_reaches_browser_config_before_any_device_request() {
    let server = MockServer::start().await;
    let home = tempfile::tempdir().unwrap();
    Mock::given(method("GET"))
        .and(path("/api/v1/public/config"))
        .respond_with(ResponseTemplate::new(503))
        .expect(1)
        .mount(&server)
        .await;
    let output = Command::new(env!("CARGO_BIN_EXE_nyxid"))
        .args(["login", "--base-url", &server.uri()])
        .env("HOME", home.path())
        .env("CI", "1")
        .env("NYXID_LOGIN_NO_DEVICE_FALLBACK", "1")
        .env("NYXID_NO_UPDATE_CHECK", "1")
        .env("NYXID_SKIP_SKILL_SELF_HEAL", "1")
        .env_remove("NYXID_TELEMETRY_DSN")
        .env_remove("NYXID_SHARE_ANALYTICS")
        .stdin(std::process::Stdio::null())
        .output()
        .await
        .unwrap();
    assert!(!output.status.success());
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
    assert!(!String::from_utf8_lossy(&output.stderr).contains("Detected CI"));
}

async fn run(home: &Path, args: &[&str]) -> Output {
    tokio::time::timeout(
        std::time::Duration::from_secs(10),
        Command::new(env!("CARGO_BIN_EXE_nyxid"))
            .args(args)
            .env("HOME", home)
            .env("CI", "1")
            .env("NYXID_NO_UPDATE_CHECK", "1")
            .env("NYXID_SKIP_SKILL_SELF_HEAL", "1")
            .env_remove("NYXID_URL")
            .env_remove("NYXID_PROFILE")
            .env_remove("NYXID_API_KEY")
            .env_remove("NYXID_ACCESS_TOKEN")
            .env_remove("NYXID_TELEMETRY_DSN")
            .env_remove("NYXID_SHARE_ANALYTICS")
            .stdin(std::process::Stdio::null())
            .kill_on_drop(true)
            .output(),
    )
    .await
    .expect("CLI blocked or launched an interactive flow")
    .expect("run CLI")
}

fn output_json(output: &Output) -> Value {
    serde_json::from_slice(&output.stdout).unwrap_or_else(|_| {
        panic!(
            "stdout was not JSON: {} / {}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

#[tokio::test]
async fn in_flight_account_request_never_retries_credentials_from_a_replaced_login() {
    for same_instance in [false, true] {
        let first = MockServer::start().await;
        let other = MockServer::start().await;
        let second = if same_instance { &first } else { &other };
        let home = tempfile::tempdir().unwrap();
        for (server, code, token) in [
            (&first, "AAAA-BBBB", "fixture-original-token"),
            (second, "CCCC-DDDD", "fixture-replacement-token"),
        ] {
            Mock::given(method("POST"))
                .and(path("/api/v1/auth/login-code/redeem"))
                .and(body_partial_json(json!({"code":code})))
                .respond_with(
                    ResponseTemplate::new(200).set_body_json(json!({"auth_kind":"account_session",
                    "access_token":token,"refresh_token":format!("{token}-refresh")})),
                )
                .expect(1)
                .mount(server)
                .await;
        }
        let login = run(
            home.path(),
            &[
                "login",
                "--code",
                "AAAA-BBBB",
                "--base-url",
                &first.uri(),
                "--output",
                "json",
            ],
        )
        .await;
        assert!(login.status.success());
        Mock::given(method("GET"))
            .and(path("/api/v1/users/me"))
            .respond_with(
                ResponseTemplate::new(401).set_delay(std::time::Duration::from_millis(750)),
            )
            .expect(1)
            .mount(&first)
            .await;
        let request = run(home.path(), &["whoami", "--output", "json"]);
        let replace = async {
            tokio::time::timeout(std::time::Duration::from_secs(3), async {
                loop {
                    if first
                        .received_requests()
                        .await
                        .unwrap()
                        .iter()
                        .any(|request| request.url.path() == "/api/v1/users/me")
                    {
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
            })
            .await
            .unwrap();
            let login = run(
                home.path(),
                &[
                    "login",
                    "--code",
                    "CCCC-DDDD",
                    "--base-url",
                    &second.uri(),
                    "--output",
                    "json",
                ],
            )
            .await;
            assert!(login.status.success());
        };
        let (request, ()) = tokio::join!(request, replace);
        assert!(!request.status.success());
        let requests = first.received_requests().await.unwrap();
        assert!(
            !requests
                .iter()
                .any(|r| r.url.path().ends_with("/auth/refresh"))
        );
        assert_eq!(
            requests
                .iter()
                .filter(|r| r.url.path() == "/api/v1/users/me")
                .count(),
            1
        );
        assert_eq!(
            std::fs::read_to_string(home.path().join(".nyxid/access_token")).unwrap(),
            "fixture-replacement-token"
        );
    }
}

async fn begin(server: &MockServer, home: &Path, mode: &str) -> String {
    let flow = if mode == "--agent-key" {
        "agent-key"
    } else {
        "device/v2"
    };
    Mock::given(method("POST")).and(path(format!("/api/v1/auth/{flow}/request")))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"device_code": POLL_SECRET,
            "user_code": "ABCD-EFGH", "verification_uri": format!("{}/login/{flow}?user_code=must-strip", server.uri()),
            "expires_in": 600, "interval": 5}))).expect(1).mount(server).await;
    let out = run(
        home,
        &[
            "login",
            mode,
            "--no-wait",
            "--output",
            "json",
            "--profile",
            "agent",
            "--base-url",
            &server.uri(),
        ],
    )
    .await;
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let challenge = output_json(&out);
    assert_eq!(challenge["user_code"], "ABCD-EFGH");
    assert!(
        !challenge["verification_uri"]
            .as_str()
            .unwrap()
            .contains('?')
    );
    assert!(!String::from_utf8_lossy(&out.stdout).contains(POLL_SECRET));
    assert!(!String::from_utf8_lossy(&out.stderr).contains(POLL_SECRET));
    assert_eq!(
        server.received_requests().await.unwrap().len(),
        1,
        "no-wait polled"
    );
    challenge["request_id"].as_str().unwrap().to_string()
}

#[tokio::test]
async fn logout_never_removes_a_concurrent_replacement_login() {
    for restricted in [false, true] {
        let server = MockServer::start().await;
        let home = tempfile::tempdir().unwrap();
        for (code, delivery) in [
            (
                "AAAA-BBBB",
                if restricted {
                    key_delivery()
                } else {
                    json!({"auth_kind":"account_session","access_token":"fixture-original","refresh_token":"fixture-original-refresh"})
                },
            ),
            (
                "CCCC-DDDD",
                json!({"auth_kind":"account_session","access_token":"fixture-new","refresh_token":"fixture-new-refresh"}),
            ),
        ] {
            Mock::given(method("POST"))
                .and(path("/api/v1/auth/login-code/redeem"))
                .and(body_partial_json(json!({"code":code})))
                .respond_with(ResponseTemplate::new(200).set_body_json(delivery))
                .expect(1)
                .mount(&server)
                .await;
        }
        assert!(
            run(
                home.path(),
                &["login", "--code", "AAAA-BBBB", "--base-url", &server.uri()]
            )
            .await
            .status
            .success()
        );
        let logout_path = if restricted {
            "/api/v1/auth/agent-key/self"
        } else {
            "/api/v1/auth/logout"
        };
        Mock::given(method(if restricted { "DELETE" } else { "POST" }))
            .and(path(logout_path))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(json!({"ok":true}))
                    .set_delay(std::time::Duration::from_millis(750)),
            )
            .expect(1)
            .mount(&server)
            .await;
        let logout = run(home.path(), &["logout"]);
        let replace = async {
            tokio::time::timeout(std::time::Duration::from_secs(3), async {
                loop {
                    if server
                        .received_requests()
                        .await
                        .unwrap()
                        .iter()
                        .any(|request| request.url.path() == logout_path)
                    {
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
            })
            .await
            .unwrap();
            assert!(
                run(
                    home.path(),
                    &["login", "--code", "CCCC-DDDD", "--base-url", &server.uri()]
                )
                .await
                .status
                .success()
            );
        };
        let (logout, ()) = tokio::join!(logout, replace);
        assert!(
            logout.status.success(),
            "{}",
            String::from_utf8_lossy(&logout.stderr)
        );
        assert_eq!(
            std::fs::read_to_string(home.path().join(".nyxid/access_token")).unwrap(),
            "fixture-new"
        );
        assert_eq!(
            std::fs::read_to_string(home.path().join(".nyxid/refresh_token")).unwrap(),
            "fixture-new-refresh"
        );
    }
}

#[tokio::test]
async fn logout_rejects_unmatched_destination_without_forwarding_saved_credentials() {
    let server = MockServer::start().await;
    let other = MockServer::start().await;
    let home = tempfile::tempdir().unwrap();
    Mock::given(method("POST"))
        .and(path("/api/v1/auth/login-code/redeem"))
        .respond_with(ResponseTemplate::new(200).set_body_json(key_delivery()))
        .expect(1)
        .mount(&server)
        .await;
    assert!(
        run(
            home.path(),
            &["login", "--code", "AAAA-BBBB", "--base-url", &server.uri()]
        )
        .await
        .status
        .success()
    );
    assert!(
        !run(home.path(), &["logout", "--base-url", &other.uri()])
            .await
            .status
            .success()
    );
    assert!(other.received_requests().await.unwrap().is_empty());
    assert_eq!(
        std::fs::read_to_string(home.path().join(".nyxid/token")).unwrap(),
        KEY_SECRET
    );
}

fn key_delivery() -> Value {
    json!({"auth_kind": "agent_key", "credential": KEY_SECRET, "credential_id": "child", "credential_expires_at": null,
        "label": "fixture agent", "api_key": {"id": "parent", "name": "Restricted", "key_prefix": "nyxid_ag_fixture",
            "owner_type": "personal", "owner_id": "owner", "owner_name": "Fixture", "scopes": "read proxy",
            "allow_all_services": false, "allow_all_nodes": false, "allowed_service_ids": [], "allowed_node_ids": [],
            "expires_at": null, "rate_limit_per_second": 5, "rate_limit_burst": 10, "platform": null}})
}

#[tokio::test]
async fn no_wait_resume_stores_either_grant_and_repeats_never_poll_twice() {
    for (mode, restricted) in [
        ("--device", false),
        ("--device", true),
        ("--agent-key", true),
    ] {
        let server = MockServer::start().await;
        let home = tempfile::tempdir().unwrap();
        let id = begin(&server, home.path(), mode).await;
        let flow = if mode == "--agent-key" {
            "agent-key"
        } else {
            "device/v2"
        };
        Mock::given(method("POST")).and(path(format!("/api/v1/auth/{flow}/poll")))
            .and(body_partial_json(json!({"device_code": POLL_SECRET})))
            .respond_with(ResponseTemplate::new(200).set_body_json(if restricted { key_delivery() } else {
                json!({"auth_kind":"account_session", "access_token":"fixture-account-access", "refresh_token":"fixture-account-refresh"})
            })).expect(1).mount(&server).await;
        let out = run(
            home.path(),
            &["login", "resume", &id, "--once", "--output", "json"],
        )
        .await;
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(
            output_json(&out)["auth"]["auth_kind"],
            if restricted {
                "agent_key"
            } else {
                "account_session"
            }
        );
        for secret in [
            POLL_SECRET,
            KEY_SECRET,
            "fixture-account-access",
            "fixture-account-refresh",
        ] {
            assert!(!String::from_utf8_lossy(&out.stdout).contains(secret));
            assert!(!String::from_utf8_lossy(&out.stderr).contains(secret));
        }
        let profile = home.path().join(".nyxid/profiles/agent");
        if restricted {
            assert_eq!(
                std::fs::read_to_string(profile.join("auth_kind")).unwrap(),
                "agent_key"
            );
            assert_eq!(
                std::fs::read_to_string(profile.join("token")).unwrap(),
                KEY_SECRET
            );
            assert!(!profile.join("refresh_token").exists());
        } else {
            assert_eq!(
                std::fs::read_to_string(profile.join("refresh_token")).unwrap(),
                "fixture-account-refresh"
            );
        }
        let again = run(
            home.path(),
            &["login", "resume", &id, "--once", "--output", "json"],
        )
        .await;
        assert_eq!(again.status.code(), Some(13));
        assert_eq!(
            output_json(&again)["error"]["code"],
            "login_already_delivered"
        );
        assert!(
            !std::fs::read_to_string(home.path().join(format!(".nyxid/pending-logins/{id}.json")))
                .unwrap()
                .contains(POLL_SECRET)
        );
    }
}

#[tokio::test]
async fn resume_states_are_distinct_and_destination_checks_precede_network() {
    for (code, expected_exit, expected_code) in [
        (11202, 10, "login_pending"),
        (11204, 11, "login_denied"),
        (11201, 12, "login_expired"),
        (11205, 13, "login_already_delivered"),
        (11206, 14, "login_rate_limited"),
    ] {
        let server = MockServer::start().await;
        let home = tempfile::tempdir().unwrap();
        let id = begin(&server, home.path(), "--device").await;
        let wrong = run(
            home.path(),
            &[
                "login",
                "resume",
                &id,
                "--profile",
                "other",
                "--once",
                "--output",
                "json",
            ],
        )
        .await;
        assert_eq!(wrong.status.code(), Some(17));
        let wrong = run(
            home.path(),
            &[
                "login",
                "resume",
                &id,
                "--base-url",
                "http://127.0.0.1:1",
                "--once",
                "--output",
                "json",
            ],
        )
        .await;
        assert_eq!(wrong.status.code(), Some(17));
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
        Mock::given(method("POST"))
            .and(path("/api/v1/auth/device/v2/poll"))
            .respond_with(
                ResponseTemplate::new(400)
                    .set_body_json(json!({"error_code": code, "message": POLL_SECRET})),
            )
            .expect(1)
            .mount(&server)
            .await;
        let out = run(
            home.path(),
            &["login", "resume", &id, "--once", "--output", "json"],
        )
        .await;
        assert_eq!(out.status.code(), Some(expected_exit));
        assert_eq!(output_json(&out)["error"]["code"], expected_code);
        assert!(!String::from_utf8_lossy(&out.stdout).contains(POLL_SECRET));
        assert!(!String::from_utf8_lossy(&out.stderr).contains(POLL_SECRET));
    }
}

#[tokio::test]
async fn explicit_restricted_resume_rejects_account_delivery() {
    let server = MockServer::start().await;
    let home = tempfile::tempdir().unwrap();
    let id = begin(&server, home.path(), "--agent-key").await;
    Mock::given(method("POST"))
        .and(path("/api/v1/auth/agent-key/poll"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "access_token": "unrequested-access", "refresh_token": "unrequested-refresh"
        })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v1/auth/logout"))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;
    let out = run(
        home.path(),
        &["login", "resume", &id, "--once", "--output", "json"],
    )
    .await;
    assert_eq!(out.status.code(), Some(19));
    assert!(!home.path().join(".nyxid/profiles/agent/token").exists());
    assert!(
        !home
            .path()
            .join(".nyxid/profiles/agent/refresh_token")
            .exists()
    );
    assert!(!String::from_utf8_lossy(&out.stdout).contains("unrequested"));
    assert!(!String::from_utf8_lossy(&out.stderr).contains("unrequested"));
}

#[tokio::test]
async fn resume_persists_server_slowdown_above_sixty_seconds() {
    let server = MockServer::start().await;
    let home = tempfile::tempdir().unwrap();
    let id = begin(&server, home.path(), "--device").await;
    Mock::given(method("POST"))
        .and(path("/api/v1/auth/device/v2/poll"))
        .respond_with(ResponseTemplate::new(429).set_body_json(json!({
            "error_code": 11203, "interval": 125
        })))
        .expect(1)
        .mount(&server)
        .await;
    let out = run(
        home.path(),
        &["login", "resume", &id, "--once", "--output", "json"],
    )
    .await;
    assert_eq!(out.status.code(), Some(10));
    let pending: Value = serde_json::from_slice(
        &std::fs::read(home.path().join(format!(".nyxid/pending-logins/{id}.json"))).unwrap(),
    )
    .unwrap();
    assert_eq!(pending["interval"], 125);
    assert_eq!(pending["state"], "pending");
}

#[tokio::test]
async fn final_resume_record_failure_reports_one_truthful_success() {
    let server = MockServer::start().await;
    let home = tempfile::tempdir().unwrap();
    let id = begin(&server, home.path(), "--device").await;
    let pending_path = home.path().join(format!(".nyxid/pending-logins/{id}.json"));
    Mock::given(method("POST"))
        .and(path("/api/v1/auth/device/v2/poll"))
        .respond_with(move |_: &wiremock::Request| {
            std::fs::remove_file(&pending_path).unwrap();
            std::fs::create_dir(&pending_path).unwrap();
            ResponseTemplate::new(200).set_body_json(
                json!({"access_token":"saved-access", "refresh_token":"saved-refresh"}),
            )
        })
        .expect(1)
        .mount(&server)
        .await;
    let result = run(
        home.path(),
        &["login", "resume", &id, "--once", "--output", "json"],
    )
    .await;
    assert!(result.status.success());
    let status = output_json(&result);
    assert_eq!(status["status"], "authenticated");
    assert_eq!(status["resume_state_saved"], false);
    assert_eq!(
        std::fs::read_to_string(home.path().join(".nyxid/profiles/agent/refresh_token")).unwrap(),
        "saved-refresh"
    );
    assert!(!String::from_utf8_lossy(&result.stdout).contains("saved-access"));
}

#[tokio::test]
async fn resume_directory_and_lock_errors_have_stable_json_storage_error() {
    for blocker in ["pending-logins", "lock"] {
        let home = tempfile::tempdir().unwrap();
        let id = "11111111-2222-4333-8444-555555555555";
        let nyxid = home.path().join(".nyxid");
        std::fs::create_dir(&nyxid).unwrap();
        if blocker == "pending-logins" {
            std::fs::write(nyxid.join(blocker), "fixture").unwrap();
        } else {
            std::fs::create_dir_all(nyxid.join(format!("pending-logins/{id}.lock"))).unwrap();
        }
        let result = run(
            home.path(),
            &["login", "resume", id, "--once", "--output", "json"],
        )
        .await;
        assert_eq!(result.status.code(), Some(21));
        assert_eq!(
            output_json(&result)["error"]["code"],
            "login_storage_failed"
        );
    }
}

#[tokio::test]
async fn failed_destination_save_clears_partial_credentials_and_revokes_either_grant() {
    for restricted in [false, true] {
        let server = MockServer::start().await;
        let home = tempfile::tempdir().unwrap();
        let id = begin(&server, home.path(), "--device").await;
        let profile = home.path().join(".nyxid/profiles/agent");
        std::fs::create_dir_all(profile.join("base_url")).unwrap();
        std::fs::write(profile.join("access_token"), "old-account-token").unwrap();
        std::fs::write(profile.join("token"), "old-agent-token").unwrap();
        Mock::given(method("POST"))
            .and(path("/api/v1/auth/device/v2/poll"))
            .respond_with(ResponseTemplate::new(200).set_body_json(if restricted {
                key_delivery()
            } else {
                json!({"access_token":"new-access", "refresh_token":"new-refresh"})
            }))
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method(if restricted { "DELETE" } else { "POST" }))
            .and(path(if restricted {
                "/api/v1/auth/agent-key/self"
            } else {
                "/api/v1/auth/logout"
            }))
            .and(wiremock::matchers::header(
                "authorization",
                if restricted {
                    format!("Bearer {KEY_SECRET}")
                } else {
                    "Bearer new-access".into()
                },
            ))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(&server)
            .await;
        let output = run(
            home.path(),
            &["login", "resume", &id, "--once", "--output", "json"],
        )
        .await;
        assert_eq!(output.status.code(), Some(21));
        assert_eq!(
            output_json(&output)["error"]["code"],
            "login_storage_failed"
        );
        for name in [
            "token",
            "access_token",
            "refresh_token",
            "auth_kind",
            "agent_key.json",
        ] {
            assert!(
                !profile.join(name).exists(),
                "partial credential remained: {name}"
            );
        }
        for secret in ["new-access", "new-refresh", KEY_SECRET] {
            assert!(!String::from_utf8_lossy(&output.stdout).contains(secret));
            assert!(!String::from_utf8_lossy(&output.stderr).contains(secret));
        }
    }
}

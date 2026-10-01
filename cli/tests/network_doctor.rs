use std::process::Command;

#[test]
fn doctor_reports_tls_configuration_and_redacts_proxy_urls_in_both_formats() {
    let home = tempfile::tempdir().unwrap();
    for json in [false, true] {
        let mut command = Command::new(env!("CARGO_BIN_EXE_nyxid"));
        command.arg("doctor");
        if json {
            command.arg("--json");
        }
        let output = command
            .env("HOME", home.path())
            .env("DO_NOT_TRACK", "1")
            .env("NYXID_NO_UPDATE_CHECK", "1")
            .env("NYXID_SKIP_SKILL_SELF_HEAL", "1")
            .env("NYXID_CA_CERT", home.path().join("missing-ca.pem"))
            .env_remove("SSL_CERT_FILE")
            .env_remove("SSL_CERT_DIR")
            .env(
                "HTTPS_PROXY",
                "http://user:PROXY_SECRET@proxy.example:8123/path?TOKEN_SECRET#fragment",
            )
            .env("NO_PROXY", "localhost,.example.com,127.0.0.1")
            .output()
            .unwrap();
        assert!(!output.status.success());
        let stdout = String::from_utf8_lossy(&output.stdout);
        for text in [&stdout, &String::from_utf8_lossy(&output.stderr)] {
            assert!(!text.contains("PROXY_SECRET"));
            assert!(!text.contains("TOKEN_SECRET"));
            assert!(!text.contains("user:"));
        }
        assert!(stdout.contains("Network / TLS"));
        assert!(stdout.contains("Bundled Mozilla"));
        assert!(stdout.contains("OS store"));
        assert!(stdout.contains("missing-ca.pem"));
        assert!(stdout.contains("http://proxy.example:8123"));
        assert!(stdout.contains("localhost,.example.com,127.0.0.1"));
        if json {
            let report: serde_json::Value = serde_json::from_str(&stdout).unwrap();
            let github = report["sections"]
                .as_array()
                .unwrap()
                .iter()
                .find(|s| s["title"] == "GitHub Releases")
                .unwrap();
            assert_eq!(github["rows"][0]["diagnostic"]["stage"], "config");
        } else {
            assert!(stdout.lines().any(|line| line == "      stage: config"));
            assert!(!stdout.lines().any(|line| line.starts_with("  stage:")));
        }
    }
}

#[test]
fn root_and_login_help_discover_ca_and_proxy_configuration() {
    for args in [vec!["--help"], vec!["login", "--help"]] {
        let output = Command::new(env!("CARGO_BIN_EXE_nyxid"))
            .args(args)
            .output()
            .unwrap();
        assert!(output.status.success());
        let help = String::from_utf8_lossy(&output.stdout);
        for variable in [
            "NYXID_CA_CERT",
            "SSL_CERT_FILE",
            "SSL_CERT_DIR",
            "HTTPS_PROXY",
            "NO_PROXY",
        ] {
            assert!(help.contains(variable), "{help}");
        }
    }
}

#[tokio::test]
async fn node_start_rejects_permanent_ca_configuration_before_reconnecting() {
    let home = tempfile::tempdir().unwrap();
    let output = tokio::time::timeout(
        std::time::Duration::from_secs(3),
        tokio::process::Command::new(env!("CARGO_BIN_EXE_nyxid"))
            .args(["node", "start"])
            .env("HOME", home.path())
            .env("NYXID_CA_CERT", home.path().join("missing.pem"))
            .env("DO_NOT_TRACK", "1")
            .env("NYXID_NO_UPDATE_CHECK", "1")
            .env("NYXID_SKIP_SKILL_SELF_HEAL", "1")
            .kill_on_drop(true)
            .output(),
    )
    .await
    .expect("node must fail immediately, not reconnect forever")
    .unwrap();
    assert!(!output.status.success());
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("NYXID_CA_CERT"), "{stderr}");
    assert!(stderr.contains("missing.pem"));
    assert!(!stderr.contains("reconnecting"));
}

#[test]
fn auto_update_enable_validates_ca_configuration_before_installing_scheduler() {
    let home = tempfile::tempdir().unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_nyxid"))
        .args(["update", "auto", "enable"])
        .env("HOME", home.path())
        .env("NYXID_INSTALL_ROOT", home.path().join("versions"))
        .env("NYXID_CA_CERT", home.path().join("missing.pem"))
        .env("DO_NOT_TRACK", "1")
        .env("NYXID_NO_UPDATE_CHECK", "1")
        .env("NYXID_SKIP_SKILL_SELF_HEAL", "1")
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("NYXID_CA_CERT"));
    assert!(!home.path().join("versions/.auto-update.json").exists());
    assert!(
        !home
            .path()
            .join("Library/LaunchAgents/dev.nyxid.update.plist")
            .exists()
    );
    assert!(
        !home
            .path()
            .join(".config/systemd/user/nyxid-update.service")
            .exists()
    );
}

#[tokio::test]
async fn doctor_uses_selected_profile_or_explicit_base_and_empty_ca_values_mean_unset() {
    use wiremock::{Mock, MockServer, ResponseTemplate, matchers::path};
    let saved = MockServer::start().await;
    let explicit = MockServer::start().await;
    for server in [&saved, &explicit] {
        Mock::given(path("/health"))
            .respond_with(ResponseTemplate::new(200))
            .expect(1)
            .mount(server)
            .await;
    }
    let home = tempfile::tempdir().unwrap();
    let profile = home.path().join(".nyxid/profiles/dot");
    std::fs::create_dir_all(&profile).unwrap();
    std::fs::write(profile.join("base_url"), saved.uri()).unwrap();
    let mut os_roots = None;
    for empty in [false, true] {
        let mut command = tokio::process::Command::new(env!("CARGO_BIN_EXE_nyxid"));
        command
            .args(["doctor", "--profile", "dot", "--json"])
            .env("HOME", home.path())
            .env("DO_NOT_TRACK", "1")
            .env("NYXID_NO_UPDATE_CHECK", "1")
            .env("NYXID_SKIP_SKILL_SELF_HEAL", "1")
            .env("HTTPS_PROXY", "http://127.0.0.1:1")
            .env("HTTP_PROXY", "http://127.0.0.1:1")
            .env("ALL_PROXY", "http://127.0.0.1:1")
            .env("NO_PROXY", "localhost,127.0.0.1")
            .env_remove("https_proxy")
            .env_remove("http_proxy")
            .env_remove("all_proxy")
            .env_remove("no_proxy")
            .kill_on_drop(true);
        for name in ["NYXID_CA_CERT", "SSL_CERT_FILE", "SSL_CERT_DIR"] {
            if empty {
                command.env(name, "");
            } else {
                command.env_remove(name);
            }
        }
        if empty {
            command.args(["--base-url", &explicit.uri()]);
        }
        let output = tokio::time::timeout(std::time::Duration::from_secs(10), command.output())
            .await
            .unwrap()
            .unwrap();
        let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        let sections = report["sections"].as_array().unwrap();
        let api = sections
            .iter()
            .find(|section| section["title"] == "NyxID API")
            .unwrap();
        assert_eq!(api["rows"][0]["status"], "pass", "{report}");
        assert!(
            api["rows"][0]["detail"]
                .as_str()
                .unwrap()
                .contains(&if empty { explicit.uri() } else { saved.uri() })
        );
        let network = sections
            .iter()
            .find(|section| section["title"] == "Network / TLS")
            .unwrap();
        let rows = network["rows"].as_array().unwrap();
        assert!(rows.iter().all(|row| row["status"] != "fail"));
        let native = rows.iter().find(|row| row["label"] == "OS store").unwrap()["detail"].clone();
        if empty {
            assert_eq!(os_roots.as_ref().unwrap(), &native);
        } else {
            os_roots = Some(native);
        }
        assert!(
            !rows
                .iter()
                .any(|row| row["label"] == "SSL_CERT_FILE" || row["label"] == "SSL_CERT_DIR")
        );
    }
}

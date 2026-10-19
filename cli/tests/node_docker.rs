#![cfg(unix)]

use std::{os::unix::fs::PermissionsExt, process::Command};

struct DockerHarness {
    root: tempfile::TempDir,
}

impl DockerHarness {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("bin")).unwrap();
        let docker = root.path().join("bin/docker");
        std::fs::write(
            &docker,
            "#!/bin/sh\nprintf '%s\\0' \"$@\" >> \"$DOCKER_CALL_LOG\"\nprintf '\\n' >> \"$DOCKER_CALL_LOG\"\n",
        )
        .unwrap();
        std::fs::set_permissions(&docker, std::fs::Permissions::from_mode(0o755)).unwrap();
        let config = root.path().join(".nyxid-node");
        std::fs::create_dir(&config).unwrap();
        std::fs::write(config.join("config.toml"), "").unwrap();
        Self { root }
    }

    fn command(&self, action: &str) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_nyxid"));
        command
            .args(["node", "docker", action])
            .env_clear()
            .env("HOME", self.root.path())
            .env("PATH", self.root.path().join("bin"))
            .env("DOCKER_CALL_LOG", self.root.path().join("docker.log"))
            .env("DO_NOT_TRACK", "1")
            .env("NYXID_NO_UPDATE_CHECK", "1")
            .env("NYXID_SKIP_SKILL_SELF_HEAL", "1")
            .current_dir(self.root.path());
        command
    }

    fn calls(&self) -> Vec<Vec<String>> {
        std::fs::read_to_string(self.root.path().join("docker.log"))
            .unwrap()
            .lines()
            .map(|line| {
                line.trim_end_matches('\0')
                    .split('\0')
                    .map(String::from)
                    .collect()
            })
            .collect()
    }

    fn certificate(&self, relative: &str) -> std::path::PathBuf {
        let path = self.root.path().join(relative);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let certificate = rcgen::generate_simple_self_signed(vec!["localhost".into()]).unwrap();
        std::fs::write(&path, certificate.cert.pem()).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
        path
    }
}

#[test]
fn docker_start_and_restart_reject_invalid_ca_before_container_changes() {
    for action in ["start", "restart"] {
        for name in ["NYXID_CA_CERT", "SSL_CERT_FILE", "SSL_CERT_DIR"] {
            let harness = DockerHarness::new();
            let output = harness
                .command(action)
                .env(name, "missing-ca")
                .output()
                .unwrap();
            assert!(!output.status.success());
            let stderr = String::from_utf8_lossy(&output.stderr);
            assert!(stderr.contains(name), "{stderr}");
            assert!(stderr.contains("missing-ca"), "{stderr}");
            assert_eq!(harness.calls(), vec![vec!["--version"]]);
        }
    }
}

#[test]
fn docker_start_and_restart_reject_ambiguous_mounts_before_container_changes() {
    for action in ["start", "restart"] {
        let harness = DockerHarness::new();
        let path = harness.certificate("company,ca.pem");
        let output = harness
            .command(action)
            .env("NYXID_CA_CERT", path)
            .output()
            .unwrap();
        assert!(!output.status.success());
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(stderr.contains("NYXID_CA_CERT"), "{stderr}");
        assert!(stderr.contains("cannot be mounted by Docker"), "{stderr}");
        assert_eq!(harness.calls(), vec![vec!["--version"]]);
    }
}

#[test]
fn docker_start_and_restart_forward_valid_ca_mounts_without_proxy_environment() {
    for action in ["start", "restart"] {
        let harness = DockerHarness::new();
        harness.certificate("company ca:2026.pem");
        harness.certificate("system ca.pem");
        harness.certificate("first certs/ca.pem");
        harness.certificate("second certs/ca.pem");
        let output = harness
            .command(action)
            .env("NYXID_CA_CERT", "company ca:2026.pem")
            .env("SSL_CERT_FILE", "system ca.pem")
            .env("SSL_CERT_DIR", "first certs:second certs")
            .env("HTTPS_PROXY", "http://user:PROXY_SECRET@proxy.example:8123")
            .env("HTTP_PROXY", "http://proxy.example:8123")
            .env("ALL_PROXY", "http://proxy.example:8123")
            .env("NO_PROXY", "private.example")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let calls = harness.calls();
        assert_eq!(calls[0], ["--version"]);
        assert_eq!(calls[1], ["image", "inspect", "nyxid-node:latest"]);
        if action == "restart" {
            assert_eq!(calls[2], ["stop", "nyxid-node"]);
        }
        let run = calls.last().unwrap();
        assert_eq!(run[0], "run");
        assert_eq!(run.last().unwrap(), "nyxid-node:latest");
        for (relative, target) in [
            ("company ca:2026.pem", "/etc/nyxid/tls/ca.pem"),
            ("system ca.pem", "/etc/nyxid/tls/system.pem"),
            ("first certs", "/etc/nyxid/tls/certs/0"),
            ("second certs", "/etc/nyxid/tls/certs/1"),
        ] {
            let mount = format!(
                "type=bind,source={},target={target},readonly",
                harness
                    .root
                    .path()
                    .canonicalize()
                    .unwrap()
                    .join(relative)
                    .display()
            );
            assert!(
                run.windows(2).any(|args| args == ["--mount", &mount]),
                "{run:?}"
            );
        }
        let environment: Vec<_> = run
            .windows(2)
            .filter(|args| args[0] == "-e")
            .map(|args| args[1].as_str())
            .collect();
        assert_eq!(
            environment,
            [
                "NYXID_CA_CERT=/etc/nyxid/tls/ca.pem",
                "SSL_CERT_FILE=/etc/nyxid/tls/system.pem",
                "SSL_CERT_DIR=/etc/nyxid/tls/certs/0:/etc/nyxid/tls/certs/1",
            ]
        );
        assert!(
            run.iter()
                .all(|arg| !arg.contains("PROXY") && !arg.contains("private.example"))
        );
    }
}

#[test]
fn docker_start_with_empty_ca_values_uses_original_run_arguments() {
    let harness = DockerHarness::new();
    let output = harness
        .command("start")
        .env("NYXID_CA_CERT", "")
        .env("SSL_CERT_FILE", "")
        .env("SSL_CERT_DIR", "")
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let config = harness.root.path().join(".nyxid-node");
    assert_eq!(
        harness.calls().last().unwrap(),
        &[
            "run",
            "-d",
            "--name",
            "nyxid-node",
            "--restart",
            "unless-stopped",
            "-v",
            &format!("{}:/app/config:rw", config.display()),
            "nyxid-node:latest",
        ]
    );
}

#[test]
fn machine_docker_restart_validates_ca_before_touching_existing_container() {
    for action in ["start", "restart"] {
        let harness = DockerHarness::new();
        let output = harness
            .command(action)
            .arg("--machine")
            .env("NYXID_CA_CERT", "missing-ca")
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("NYXID_CA_CERT"));
        assert_eq!(harness.calls(), vec![vec!["--version"]]);
    }
    let harness = DockerHarness::new();
    let output = harness
        .command("restart")
        .arg("--machine")
        .output()
        .unwrap();
    assert!(output.status.success());
    assert_eq!(
        harness.calls().last().unwrap(),
        &["restart", "nyxid-node-machine"]
    );
}

#[test]
fn machine_docker_creation_preserves_ca_mounts_and_sandbox_profile() {
    let harness = DockerHarness::new();
    let docker = harness.root.path().join("bin/docker");
    std::fs::write(&docker,
        "#!/bin/sh\nprintf '%s\\0' \"$@\" >> \"$DOCKER_CALL_LOG\"\nprintf '\\n' >> \"$DOCKER_CALL_LOG\"\n[ \"$1\" != inspect ]\n",
    ).unwrap();
    let path = harness.certificate("company.pem");
    let output = harness
        .command("start")
        .arg("--machine")
        .env("NYXID_CA_CERT", &path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let calls = harness.calls();
    let run = calls.last().unwrap();
    assert_eq!(
        run.last().unwrap(),
        "ghcr.io/chronoaiproject/nyxid/nyxid-node-machine:latest"
    );
    assert!(
        run.windows(2)
            .any(|args| args == ["-e", "NYXID_CA_CERT=/etc/nyxid/tls/ca.pem"])
    );
    let mount = format!(
        "type=bind,source={},target=/etc/nyxid/tls/ca.pem,readonly",
        path.display()
    );
    assert!(run.windows(2).any(|args| args == ["--mount", &mount]));
    assert!(
        run.windows(2)
            .any(|args| args[0] == "--security-opt" && args[1].starts_with("seccomp="))
    );
    assert!(
        run.iter()
            .any(|arg| arg == "nyxid-node-machine-state:/var/lib/nyxid-machine")
    );
}

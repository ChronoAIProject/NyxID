use std::process::{Command, Output};
use tempfile::tempdir;

fn rekey(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_nyxid"))
        .args(["node", "rekey"])
        .args(args)
        .env("NYXID_REKEY_FIXTURE", "test-input")
        .env_remove("NYXID_REKEY_ABSENT")
        .output()
        .unwrap()
}

#[test]
fn rekey_validates_both_sources_before_storage_and_redacts_errors() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("token");
    std::fs::write(&path, "test-input\r\n").unwrap();
    let config = dir.path().to_str().unwrap();
    for args in [
        vec![
            "--auth-token-file",
            path.to_str().unwrap(),
            "--signing-secret-env",
            "NYXID_REKEY_ABSENT",
        ],
        vec![
            "--auth-token-env",
            "NYXID_REKEY_FIXTURE",
            "--signing-secret-env",
            "NYXID_REKEY_ABSENT",
        ],
        vec![
            "--auth-token",
            "test-input",
            "--signing-secret-env",
            "NYXID_REKEY_ABSENT",
        ],
    ] {
        let literal = args[0] == "--auth-token";
        let output = rekey(&[args.as_slice(), &["--config", config]].concat());
        let err = String::from_utf8(output.stderr).unwrap();
        assert!(!output.status.success());
        assert!(err.contains("Cannot read --signing-secret-env"), "{err}");
        assert_eq!(err.contains("process listings"), literal);
        assert!(!err.contains("test-input"));
        assert!(!dir.path().join("config.toml").exists());
    }
}

#[test]
fn rekey_refuses_conflicting_sources_for_either_secret() {
    for flag in ["--auth-token", "--signing-secret"] {
        let output = rekey(&[
            flag,
            "test-input",
            &format!("{flag}-env"),
            "NYXID_REKEY_FIXTURE",
        ]);
        assert_eq!(output.status.code(), Some(2));
        assert!(
            !String::from_utf8(output.stderr)
                .unwrap()
                .contains("test-input")
        );
    }
}

#[test]
fn rekey_accepts_file_and_named_environment_together() {
    let dir = tempdir().unwrap();
    let config = dir.path().join("config.toml");
    std::fs::write(
        &config,
        "[server]\nurl='http://localhost'\n[node]\nid='fixture'\nauth_token_encrypted=''\n",
    )
    .unwrap();
    let path = dir.path().join("signing-secret");
    std::fs::write(&path, "file-input\r\n").unwrap();
    let output = rekey(&[
        "--config",
        dir.path().to_str().unwrap(),
        "--auth-token-env",
        "NYXID_REKEY_FIXTURE",
        "--signing-secret-file",
        path.to_str().unwrap(),
    ]);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let saved = std::fs::read_to_string(config).unwrap();
    assert!(saved.contains("shared_secret_encrypted"));
    assert!(!saved.contains("file-input") && !saved.contains("test-input"));
    assert!(output.stderr.is_empty());
}

use super::*;

const KEY: &str = "sk-fixture-not-a-live-credential";

#[test]
fn only_api_key_file_storage_is_imported_without_modifying_source() {
    let home = tempfile::tempdir().unwrap();
    let source = serde_json::json!({"OPENAI_API_KEY":KEY,"auth_mode":"apikey"}).to_string();
    std::fs::write(home.path().join("auth.json"), &source).unwrap();
    assert_eq!(read_api_key(home.path()).unwrap().as_str(), KEY);
    for mode in ["keyring", "auto", "unsupported"] {
        std::fs::write(
            home.path().join("config.toml"),
            format!("cli_auth_credentials_store = {mode:?}"),
        )
        .unwrap();
        assert!(read_api_key(home.path()).is_err());
    }
    std::fs::write(
        home.path().join("config.toml"),
        "cli_auth_credentials_store = 'file'",
    )
    .unwrap();
    assert_eq!(read_api_key(home.path()).unwrap().as_str(), KEY);
    assert_eq!(
        std::fs::read_to_string(home.path().join("auth.json")).unwrap(),
        source
    );
    std::fs::write(
        home.path().join("auth.json"),
        serde_json::json!({"tokens":{"access_token":"fixture-oauth"},"OPENAI_API_KEY":KEY})
            .to_string(),
    )
    .unwrap();
    assert_eq!(
        read_api_key(home.path()).unwrap_err(),
        "separate_authorization_required"
    );
}

#[cfg(unix)]
#[test]
fn broken_or_symlinked_configuration_never_defaults_to_file_import() {
    let home = tempfile::tempdir().unwrap();
    std::fs::write(
        home.path().join("auth.json"),
        serde_json::json!({"OPENAI_API_KEY":KEY}).to_string(),
    )
    .unwrap();
    std::os::unix::fs::symlink(home.path().join("missing"), home.path().join("config.toml"))
        .unwrap();
    assert_eq!(read_api_key(home.path()).unwrap_err(), "source_unsupported");
}

#[test]
fn bounded_source_rejects_directories_large_files_and_invalid_json() {
    let home = tempfile::tempdir().unwrap();
    assert_eq!(read_api_key(home.path()).unwrap_err(), "source_unavailable");
    std::fs::write(
        home.path().join("auth.json"),
        vec![b'x'; MAX_SOURCE_BYTES as usize + 1],
    )
    .unwrap();
    assert_eq!(read_api_key(home.path()).unwrap_err(), "source_unsupported");
    std::fs::write(home.path().join("auth.json"), "{}").unwrap();
    assert_eq!(read_api_key(home.path()).unwrap_err(), "source_unsupported");
    assert_eq!(read_source(home.path()).unwrap_err(), "source_unsupported");
}

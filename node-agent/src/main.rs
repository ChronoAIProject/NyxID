mod cli;
mod config;
mod credential_store;
mod encryption;
mod error;
mod keychain;
mod metrics;
mod proxy_executor;
mod secret_backend;
mod signing;
mod ws_client;

use std::path::Path;

use clap::Parser;
use tracing_subscriber::EnvFilter;

use std::time::Duration;

use crate::cli::{Cli, Commands, CredentialCommands};
use crate::config::NodeConfig;
use crate::credential_store::{CredentialStore, SharedCredentials, SharedCredentialsSender};
use crate::error::Result;
use crate::secret_backend::SecretBackend;

#[tokio::main]
async fn main() {
    let cli = Cli::parse();

    let log_level = cli.log_level.as_deref().unwrap_or("info");
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(log_level)),
        )
        .init();

    if let Err(e) = run(cli).await {
        tracing::error!(error = %e, "Fatal error");
        std::process::exit(1);
    }
}

async fn run(cli: Cli) -> Result<()> {
    match cli.command {
        Commands::Register {
            token,
            url,
            config: config_path,
            keychain,
        } => cmd_register(&token, url.as_deref(), config_path.as_deref(), keychain).await,
        Commands::Start {
            config: config_path,
        } => cmd_start(config_path.as_deref()).await,
        Commands::Status {
            config: config_path,
        } => cmd_status(config_path.as_deref()),
        Commands::Rekey {
            auth_token,
            signing_secret,
            config: config_path,
        } => cmd_rekey(&auth_token, &signing_secret, config_path.as_deref()),
        Commands::Credentials {
            command,
            config: config_path,
        } => cmd_credentials(command, config_path.as_deref()),
        Commands::Migrate {
            to,
            config: config_path,
        } => cmd_migrate(&to, config_path.as_deref()),
        Commands::Version => {
            println!("nyxid-node {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
    }
}

async fn cmd_register(
    token: &str,
    url: Option<&str>,
    config_path: Option<&str>,
    use_keychain: bool,
) -> Result<()> {
    let config_dir = config::resolve_config_dir(config_path);
    std::fs::create_dir_all(&config_dir)?;
    let backend_name = if use_keychain { "keychain" } else { "file" };

    SecretBackend::preflight(backend_name, &config_dir)?;

    // M2: Use ws:// for localhost (wss:// requires TLS not available in dev)
    let ws_url = url.unwrap_or("ws://localhost:3001/api/v1/nodes/ws");

    tracing::info!(url = %ws_url, "Registering node...");

    let (node_id, auth_token, signing_secret) = ws_client::register_node(ws_url, token).await?;

    tracing::info!(node_id = %node_id, "Registration successful");

    let backend = SecretBackend::new(backend_name, &node_id, &config_dir)?;

    let mut config = NodeConfig::new(ws_url.to_string(), node_id, backend_name.to_string());
    backend.store_auth_token(&mut config, &auth_token)?;
    if let Some(secret) = signing_secret {
        backend.store_signing_secret(&mut config, &secret)?;
    }

    let config_file = config_dir.join("config.toml");
    config.save(&config_file)?;

    tracing::info!(path = %config_file.display(), "Configuration saved");
    println!("Node registered successfully.");
    println!("  Node ID:  {}", config.node.id);
    println!("  Storage:  {backend_name}");
    println!("  Config:   {}", config_file.display());
    println!();
    println!("Start the agent with:");
    println!("  nyxid-node start");

    Ok(())
}

async fn cmd_start(config_path: Option<&str>) -> Result<()> {
    let config_dir = config::resolve_config_dir(config_path);
    let config_file = config_dir.join("config.toml");

    let config = NodeConfig::load(&config_file)?;
    let backend = SecretBackend::from_config(&config, &config_dir)?;

    let auth_token = backend.load_auth_token(&config)?;
    let signing_secret = backend.load_signing_secret(&config)?;
    let credentials = CredentialStore::from_config_with_backend(&config, &backend)?;

    let (cred_sender, shared_creds) = SharedCredentials::new(credentials);

    tracing::info!(
        node_id = %config.node.id,
        server = %config.server.url,
        storage = %config.storage_backend,
        credentials = shared_creds.snapshot().count(),
        "Starting node agent"
    );

    // Spawn background task that reloads credentials when config file changes
    let reload_handle = tokio::spawn(credential_reload_loop(
        config_file,
        config_dir,
        cred_sender,
        Duration::from_secs(5),
    ));

    ws_client::run_with_shutdown(config, auth_token, signing_secret, shared_creds).await;

    reload_handle.abort();
    Ok(())
}

/// Poll the config file mtime and reload credentials when it changes.
async fn credential_reload_loop(
    config_file: std::path::PathBuf,
    config_dir: std::path::PathBuf,
    sender: SharedCredentialsSender,
    interval: Duration,
) {
    let mut last_modified = std::fs::metadata(&config_file)
        .and_then(|m| m.modified())
        .ok();

    loop {
        tokio::time::sleep(interval).await;

        let current_modified = match std::fs::metadata(&config_file).and_then(|m| m.modified()) {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!(error = %e, "Failed to stat config file for credential reload");
                continue;
            }
        };

        if Some(current_modified) == last_modified {
            continue;
        }

        last_modified = Some(current_modified);

        let config = match NodeConfig::load(&config_file) {
            Ok(c) => c,
            Err(e) => {
                tracing::error!(error = %e, "Failed to reload config, keeping existing credentials");
                continue;
            }
        };

        let backend = match SecretBackend::from_config(&config, &config_dir) {
            Ok(b) => b,
            Err(e) => {
                tracing::error!(error = %e, "Failed to init secret backend, keeping existing credentials");
                continue;
            }
        };

        match CredentialStore::from_config_with_backend(&config, &backend) {
            Ok(new_store) => {
                let count = new_store.count();
                sender.update(new_store);
                tracing::info!(credentials = count, "Credentials reloaded from config");
            }
            Err(e) => {
                tracing::error!(error = %e, "Failed to reload credentials, keeping existing");
            }
        }
    }
}

fn cmd_status(config_path: Option<&str>) -> Result<()> {
    let config_dir = config::resolve_config_dir(config_path);
    let config_file = config_dir.join("config.toml");

    let config = NodeConfig::load(&config_file)?;
    let backend = SecretBackend::from_config(&config, &config_dir)?;
    let credentials = CredentialStore::from_config_with_backend(&config, &backend)?;

    println!("Node Status");
    println!("  Node ID:     {}", config.node.id);
    println!("  Server:      {}", config.server.url);
    println!("  Storage:     {}", config.storage_backend);
    println!("  Credentials: {} configured", credentials.count());

    for slug in credentials.service_slugs() {
        println!("    - {slug}");
    }

    Ok(())
}

fn cmd_rekey(auth_token: &str, signing_secret: &str, config_path: Option<&str>) -> Result<()> {
    let config_dir = config::resolve_config_dir(config_path);
    let config_file = config_dir.join("config.toml");

    let mut config = NodeConfig::load(&config_file)?;
    let backend = SecretBackend::from_config(&config, &config_dir)?;

    backend.store_auth_token(&mut config, auth_token)?;
    backend.store_signing_secret(&mut config, signing_secret)?;
    config.save(&config_file)?;

    println!("Node credentials updated.");
    println!("Restart the agent to reconnect with the rotated credentials.");
    Ok(())
}

fn cmd_migrate(target_backend: &str, config_path: Option<&str>) -> Result<()> {
    if target_backend != "keychain" && target_backend != "file" {
        return Err(crate::error::Error::Validation(
            "Target must be 'keychain' or 'file'".to_string(),
        ));
    }

    let config_dir = config::resolve_config_dir(config_path);
    let config_file = config_dir.join("config.toml");

    let mut config = NodeConfig::load(&config_file)?;
    let source_backend = config.storage_backend.clone();

    if config.storage_backend == target_backend {
        println!("Already using '{target_backend}' storage. Nothing to migrate.");
        return Ok(());
    }

    let source = SecretBackend::from_config(&config, &config_dir)?;
    let target = SecretBackend::new(target_backend, &config.node.id, &config_dir)?;
    let report = migrate_config(&mut config, &source, &target, target_backend, &config_file)?;

    println!("Migrated from '{source_backend}' to '{target_backend}'.");
    println!("Restart the agent to use the new storage backend.");
    for warning in report.cleanup_warnings {
        eprintln!("Warning: {warning}");
    }
    Ok(())
}

#[derive(Debug, Default)]
struct MigrationReport {
    cleanup_warnings: Vec<String>,
}

fn migrate_config(
    config: &mut NodeConfig,
    source: &SecretBackend,
    target: &SecretBackend,
    target_backend: &str,
    config_file: &Path,
) -> Result<MigrationReport> {
    let auth_token = source.load_auth_token(config)?;
    let signing_secret = source.load_signing_secret(config)?;

    let mut credential_values = Vec::new();
    for (slug, cred_config) in &config.credentials {
        let value = source.load_credential_value(
            slug,
            cred_config
                .header_value_encrypted
                .as_deref()
                .or(cred_config.param_value_encrypted.as_deref()),
        )?;
        credential_values.push((slug.clone(), cred_config.injection_method.clone(), value));
    }

    let mut updated = config.clone();
    target.store_auth_token(&mut updated, &auth_token)?;
    if let Some(ref secret) = signing_secret {
        target.store_signing_secret(&mut updated, secret)?;
    }

    for (slug, injection_method, value) in &credential_values {
        let encrypted = target.store_credential_value(slug, value)?;
        if let Some(cred_config) = updated.credentials.get_mut(slug) {
            match injection_method.as_str() {
                "header" => cred_config.header_value_encrypted = encrypted,
                "query_param" => cred_config.param_value_encrypted = encrypted,
                _ => {}
            }
        }
    }

    updated.storage_backend = target_backend.to_string();
    if let Err(err) = updated.save(config_file) {
        rollback_target_secrets(target, &credential_values);
        return Err(err);
    }

    let cleanup_warnings = cleanup_source_secrets(source, &credential_values);
    *config = updated;
    Ok(MigrationReport { cleanup_warnings })
}

fn rollback_target_secrets(target: &SecretBackend, credential_values: &[(String, String, String)]) {
    let _ = target.delete_auth_token();
    let _ = target.delete_signing_secret();
    for (slug, _, _) in credential_values {
        let _ = target.delete_credential(slug);
    }
}

fn cleanup_source_secrets(
    source: &SecretBackend,
    credential_values: &[(String, String, String)],
) -> Vec<String> {
    let mut warnings = Vec::new();

    if let Err(err) = source.delete_auth_token() {
        warnings.push(format!("Failed to remove old auth token: {err}"));
    }
    if let Err(err) = source.delete_signing_secret() {
        warnings.push(format!("Failed to remove old signing secret: {err}"));
    }
    for (slug, _, _) in credential_values {
        if let Err(err) = source.delete_credential(slug) {
            warnings.push(format!(
                "Failed to remove old credential '{slug}' from the previous backend: {err}"
            ));
        }
    }

    warnings
}

fn cmd_credentials(command: CredentialCommands, config_path: Option<&str>) -> Result<()> {
    let config_dir = config::resolve_config_dir(config_path);
    let config_file = config_dir.join("config.toml");

    match command {
        CredentialCommands::Add {
            service,
            header,
            query_param,
        } => {
            let mut config = NodeConfig::load(&config_file)?;
            let backend = SecretBackend::from_config(&config, &config_dir)?;

            if let Some(header_val) = header {
                let (name, value) = parse_header(&header_val)?;
                config.add_header_credential_via(&service, &name, &value, &backend)?;
            } else if let Some(qp_val) = query_param {
                let (name, value) = parse_query_param(&qp_val)?;
                config.add_query_param_credential_via(&service, &name, &value, &backend)?;
            } else {
                return Err(crate::error::Error::Validation(
                    "Either --header or --query-param must be provided".to_string(),
                ));
            }

            config.save(&config_file)?;
            println!("Credential added for service '{service}'.");
            Ok(())
        }
        CredentialCommands::List => {
            let config = NodeConfig::load(&config_file)?;
            let backend = SecretBackend::from_config(&config, &config_dir)?;
            let creds = CredentialStore::from_config_with_backend(&config, &backend)?;

            if creds.count() == 0 {
                println!("No credentials configured.");
            } else {
                println!("Configured credentials:");
                for slug in creds.service_slugs() {
                    if let Some(cred) = creds.get(&slug) {
                        println!(
                            "  {slug}: {} ({})",
                            cred.injection_method(),
                            cred.target_name()
                        );
                    }
                }
            }
            Ok(())
        }
        CredentialCommands::Remove { service } => {
            let mut config = NodeConfig::load(&config_file)?;
            let backend = SecretBackend::from_config(&config, &config_dir)?;
            config.remove_credential_via(&service, &backend)?;
            config.save(&config_file)?;
            println!("Credential removed for service '{service}'.");
            Ok(())
        }
    }
}

fn parse_header(header: &str) -> Result<(String, String)> {
    let (name, value) = header.split_once(':').ok_or_else(|| {
        crate::error::Error::Validation(
            "Header must be in 'Name: value' format (e.g., 'Authorization: Bearer sk-...')"
                .to_string(),
        )
    })?;
    Ok((name.trim().to_string(), value.trim().to_string()))
}

fn parse_query_param(param: &str) -> Result<(String, String)> {
    let (name, value) = param.split_once('=').ok_or_else(|| {
        crate::error::Error::Validation(
            "Query param must be in 'name=value' format (e.g., 'api_key=sk-...')".to_string(),
        )
    })?;
    Ok((name.to_string(), value.to_string()))
}

#[cfg(test)]
mod tests {
    use std::fs;

    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    use super::*;
    use crate::encryption::LocalEncryption;
    use crate::error::Error;

    #[test]
    fn missing_keychain_signing_secret_fails_closed() {
        let backend = SecretBackend::new_mock_keychain("node-1");
        let mut config = NodeConfig::new(
            "wss://example.com/api/v1/nodes/ws".to_string(),
            "node-1".to_string(),
            "keychain".to_string(),
        );

        backend
            .store_auth_token(&mut config, "nyx_nauth_test")
            .unwrap();
        config.signing.shared_secret_encrypted = Some(String::new());

        let err = backend.load_signing_secret(&config).unwrap_err();
        assert!(matches!(err, Error::Keychain(_)));
    }

    #[test]
    fn migrate_keychain_to_file_cleans_up_source_secrets() {
        let dir = tempfile::tempdir().unwrap();
        let source = SecretBackend::new_mock_keychain("node-1");
        let target = SecretBackend::File(LocalEncryption::load_or_generate(dir.path()).unwrap());

        let mut config = NodeConfig::new(
            "wss://example.com/api/v1/nodes/ws".to_string(),
            "node-1".to_string(),
            "keychain".to_string(),
        );
        source
            .store_auth_token(&mut config, "nyx_nauth_test")
            .unwrap();
        source
            .store_signing_secret(&mut config, "00112233445566778899aabbccddeeff")
            .unwrap();
        config
            .add_header_credential_via("openai", "Authorization", "Bearer sk-test", &source)
            .unwrap();

        let config_file = dir.path().join("config.toml");
        let report = migrate_config(&mut config, &source, &target, "file", &config_file).unwrap();
        assert!(report.cleanup_warnings.is_empty());
        assert_eq!(config.storage_backend, "file");

        let loaded = NodeConfig::load(&config_file).unwrap();
        let file_backend = SecretBackend::from_config(&loaded, dir.path()).unwrap();
        assert_eq!(
            file_backend.load_auth_token(&loaded).unwrap(),
            "nyx_nauth_test"
        );
        assert_eq!(
            file_backend.load_signing_secret(&loaded).unwrap(),
            Some("00112233445566778899aabbccddeeff".to_string())
        );
        assert_eq!(
            file_backend
                .load_credential_value(
                    "openai",
                    loaded.credentials["openai"]
                        .header_value_encrypted
                        .as_deref(),
                )
                .unwrap(),
            "Bearer sk-test"
        );

        assert!(source.load_auth_token(&config).is_err());
        assert!(source.load_signing_secret(&config).is_err());
        assert!(
            source
                .load_credential_value(
                    "openai",
                    config.credentials["openai"]
                        .header_value_encrypted
                        .as_deref(),
                )
                .is_err()
        );
    }

    #[cfg(unix)]
    #[test]
    fn migrate_preserves_source_secrets_when_save_fails() {
        let dir = tempfile::tempdir().unwrap();
        let source = SecretBackend::new_mock_keychain("node-1");
        let target = SecretBackend::File(LocalEncryption::load_or_generate(dir.path()).unwrap());

        let mut config = NodeConfig::new(
            "wss://example.com/api/v1/nodes/ws".to_string(),
            "node-1".to_string(),
            "keychain".to_string(),
        );
        source
            .store_auth_token(&mut config, "nyx_nauth_test")
            .unwrap();
        source
            .store_signing_secret(&mut config, "00112233445566778899aabbccddeeff")
            .unwrap();
        config
            .add_header_credential_via("openai", "Authorization", "Bearer sk-test", &source)
            .unwrap();

        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o500)).unwrap();

        let config_file = dir.path().join("config.toml");
        let result = migrate_config(&mut config, &source, &target, "file", &config_file);

        fs::set_permissions(dir.path(), fs::Permissions::from_mode(0o700)).unwrap();

        assert!(result.is_err());
        assert_eq!(config.storage_backend, "keychain");
        assert_eq!(source.load_auth_token(&config).unwrap(), "nyx_nauth_test");
        assert_eq!(
            source.load_signing_secret(&config).unwrap(),
            Some("00112233445566778899aabbccddeeff".to_string())
        );
        assert_eq!(
            source
                .load_credential_value(
                    "openai",
                    config.credentials["openai"]
                        .header_value_encrypted
                        .as_deref(),
                )
                .unwrap(),
            "Bearer sk-test"
        );
    }

    #[test]
    fn cleanup_source_secrets_removes_auth_signing_and_credentials() {
        let backend = SecretBackend::new_mock_keychain("node-1");
        let mut config = NodeConfig::new(
            "wss://example.com/api/v1/nodes/ws".to_string(),
            "node-1".to_string(),
            "keychain".to_string(),
        );
        backend
            .store_auth_token(&mut config, "nyx_nauth_test")
            .unwrap();
        backend
            .store_signing_secret(&mut config, "00112233445566778899aabbccddeeff")
            .unwrap();
        config
            .add_header_credential_via("openai", "Authorization", "Bearer sk-test", &backend)
            .unwrap();

        let warnings = cleanup_source_secrets(
            &backend,
            &[(
                "openai".to_string(),
                "header".to_string(),
                "Bearer sk-test".to_string(),
            )],
        );

        assert!(warnings.is_empty());
        // After cleanup, vault fields should be cleared
        assert!(backend.load_auth_token(&config).is_err());
        assert!(backend.load_signing_secret(&config).is_err());
        assert!(backend.load_credential_value("openai", None).is_err());
    }
}

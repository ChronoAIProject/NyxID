//! Node agent command dispatch -- extracted from the standalone nyxid-node binary.
//!
//! All functions here are called from `cli/src/commands/node.rs` when the user
//! runs `nyxid node <agent-subcommand>`.

use std::path::Path;
use std::time::Duration;

use base64::Engine as _;
use serde::Deserialize;

use super::config::{self, NodeConfig, SshAlgorithmPreferences};
use super::credential_store::{CredentialStore, SharedCredentials, SharedCredentialsSender};
use super::credentials::ssh_keys::{self, NewSshKeyEntry};
use super::error::{Error, Result};
use super::oauth;
use super::secret_backend::SecretBackend;
use super::ssh_node_exec;
use super::ws_client;

use crate::cli::CredentialSecretFormat;
use zeroize::Zeroizing;

#[derive(Clone, Debug, Deserialize)]
struct PendingCredentialMetadata {
    id: String,
    service_slug: String,
    injection_method: String,
    field_name: String,
    target_url: Option<String>,
    label: Option<String>,
    created_at: String,
    expires_at: String,
}

#[derive(Debug, Deserialize)]
struct PendingCredentialListResponse {
    pending_credentials: Vec<PendingCredentialMetadata>,
}

// ---------------------------------------------------------------------------
// Top-level commands
// ---------------------------------------------------------------------------

pub async fn cmd_register(
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

    let mut node_config = NodeConfig::new(ws_url.to_string(), node_id, backend_name.to_string());
    backend.store_auth_token(&mut node_config, &auth_token)?;
    if let Some(secret) = signing_secret {
        backend.store_signing_secret(&mut node_config, &secret)?;
    }

    let config_file = config_dir.join("config.toml");
    node_config.save(&config_file)?;

    tracing::info!(path = %config_file.display(), "Configuration saved");
    println!("Node registered successfully.");
    println!("  Node ID:  {}", node_config.node.id);
    println!("  Storage:  {backend_name}");
    println!("  Config:   {}", config_file.display());
    println!();
    println!("Start the agent with:");
    println!("  nyxid node start");

    Ok(())
}

pub async fn cmd_start(config_path: Option<&str>, log_level: Option<&str>) -> Result<()> {
    init_tracing(log_level);

    let config_dir = config::resolve_config_dir(config_path);
    let config_file = config_dir.join("config.toml");

    let node_config = NodeConfig::load(&config_file)?;
    let backend = SecretBackend::from_config(&node_config, &config_dir)?;

    let auth_token = backend.load_auth_token(&node_config)?;
    let signing_secret = backend.load_signing_secret(&node_config)?;
    let credentials = CredentialStore::from_config_with_backend(&node_config, &backend)?;

    let (cred_sender, shared_creds) = SharedCredentials::new(credentials);
    let cred_sender = std::sync::Arc::new(cred_sender);

    let backend = std::sync::Arc::new(backend);

    tracing::info!(
        node_id = %node_config.node.id,
        server = %node_config.server.url,
        storage = %node_config.storage_backend,
        credentials = shared_creds.snapshot().count(),
        "Starting node agent"
    );

    // Spawn background task that reloads credentials when config file changes
    let reload_handle = tokio::spawn(credential_reload_loop(
        config_file.clone(),
        cred_sender.clone(),
        Duration::from_secs(5),
        std::sync::Arc::clone(&backend),
    ));

    // Spawn OAuth refresh loop (check every 60 seconds)
    let refresh_handle = tokio::spawn(oauth_refresh_loop(
        config_file.clone(),
        Duration::from_secs(60),
        std::sync::Arc::clone(&backend),
    ));

    ws_client::run_with_shutdown(
        node_config,
        config_file,
        auth_token,
        signing_secret,
        shared_creds,
        cred_sender,
        backend,
    )
    .await;

    reload_handle.abort();
    refresh_handle.abort();
    Ok(())
}

pub fn cmd_status(config_path: Option<&str>) -> Result<()> {
    let config_dir = config::resolve_config_dir(config_path);
    let config_file = config_dir.join("config.toml");

    let node_config = NodeConfig::load(&config_file)?;
    let backend = SecretBackend::from_config(&node_config, &config_dir)?;
    let credentials = CredentialStore::from_config_with_backend(&node_config, &backend)?;

    println!("Node Status");
    println!("  Node ID:     {}", node_config.node.id);
    println!("  Server:      {}", node_config.server.url);
    println!("  Storage:     {}", node_config.storage_backend);
    println!("  Credentials: {} configured", credentials.count());

    for slug in credentials.service_slugs() {
        println!("    - {slug}");
    }

    Ok(())
}

pub fn cmd_rekey(auth_token: &str, signing_secret: &str, config_path: Option<&str>) -> Result<()> {
    let config_dir = config::resolve_config_dir(config_path);
    let config_file = config_dir.join("config.toml");

    let mut node_config = NodeConfig::load(&config_file)?;
    let backend = SecretBackend::from_config(&node_config, &config_dir)?;

    backend.store_auth_token(&mut node_config, auth_token)?;
    backend.store_signing_secret(&mut node_config, signing_secret)?;
    node_config.save(&config_file)?;

    println!("Node credentials updated.");
    println!("Restart the agent to reconnect with the rotated credentials.");
    Ok(())
}

pub fn cmd_version() {
    println!("nyxid node agent {}", env!("CARGO_PKG_VERSION"));
}

pub fn cmd_migrate(target_backend: &str, config_path: Option<&str>) -> Result<()> {
    if target_backend != "keychain" && target_backend != "file" {
        return Err(super::error::Error::Validation(
            "Target must be 'keychain' or 'file'".to_string(),
        ));
    }

    let config_dir = config::resolve_config_dir(config_path);
    let config_file = config_dir.join("config.toml");

    let mut node_config = NodeConfig::load(&config_file)?;
    let source_backend = node_config.storage_backend.clone();

    if node_config.storage_backend == target_backend {
        println!("Already using '{target_backend}' storage. Nothing to migrate.");
        return Ok(());
    }

    let source = SecretBackend::from_config(&node_config, &config_dir)?;
    let target = SecretBackend::new(target_backend, &node_config.node.id, &config_dir)?;
    let report = migrate_config(
        &mut node_config,
        &source,
        &target,
        target_backend,
        &config_file,
    )?;

    println!("Migrated from '{source_backend}' to '{target_backend}'.");
    println!("Restart the agent to use the new storage backend.");
    for warning in report.cleanup_warnings {
        eprintln!("Warning: {warning}");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Credentials subcommands
// ---------------------------------------------------------------------------

pub async fn cmd_credentials(
    command: crate::cli::NodeCredentialCommands,
    config_path: Option<&str>,
) -> Result<()> {
    use crate::cli::NodeCredentialCommands;

    let config_dir = config::resolve_config_dir(config_path);
    let config_file = config_dir.join("config.toml");

    match command {
        NodeCredentialCommands::Add {
            service: raw_service,
            url,
            header,
            query_param,
            secret_format,
            value,
        } => {
            let service = raw_service.to_lowercase();
            let mut node_config = NodeConfig::load(&config_file)?;
            let backend = SecretBackend::from_config(&node_config, &config_dir)?;

            if let Some(header_name) = header {
                // Support legacy inline format "Name: value" for backwards compat
                if header_name.contains(':') {
                    if value.is_some() {
                        return Err(super::error::Error::Validation(
                            "Use either --header Name with a prompted/inline secret, or the legacy --header 'Name: value' form"
                                .to_string(),
                        ));
                    }
                    if secret_format != CredentialSecretFormat::Raw {
                        return Err(super::error::Error::Validation(
                            "Legacy 'Name: value' input cannot be combined with --secret-format"
                                .to_string(),
                        ));
                    }
                    let (name, val) = parse_header(&header_name)?;
                    node_config.add_header_credential_via(
                        &service,
                        &name,
                        &val,
                        url.as_deref(),
                        &backend,
                    )?;
                } else {
                    let secret = read_secret_value(
                        value,
                        &format!("Enter value for header '{header_name}'"),
                    )?;
                    let secret = format_secret_value(secret, secret_format)?;
                    node_config.add_header_credential_via(
                        &service,
                        &header_name,
                        &secret,
                        url.as_deref(),
                        &backend,
                    )?;
                }
            } else if let Some(param_name) = query_param {
                // Support legacy inline format "name=value" for backwards compat
                if param_name.contains('=') {
                    if value.is_some() {
                        return Err(super::error::Error::Validation(
                            "Use either --query-param name with a prompted/inline secret, or the legacy --query-param 'name=value' form"
                                .to_string(),
                        ));
                    }
                    if secret_format != CredentialSecretFormat::Raw {
                        return Err(super::error::Error::Validation(
                            "Legacy 'name=value' input cannot be combined with --secret-format"
                                .to_string(),
                        ));
                    }
                    let (name, val) = parse_query_param(&param_name)?;
                    node_config.add_query_param_credential_via(
                        &service,
                        &name,
                        &val,
                        url.as_deref(),
                        &backend,
                    )?;
                } else {
                    if secret_format != CredentialSecretFormat::Raw {
                        return Err(super::error::Error::Validation(
                            "--secret-format bearer/basic is only supported with --header"
                                .to_string(),
                        ));
                    }
                    let secret = read_secret_value(
                        value,
                        &format!("Enter value for query param '{param_name}'"),
                    )?;
                    node_config.add_query_param_credential_via(
                        &service,
                        &param_name,
                        &secret,
                        url.as_deref(),
                        &backend,
                    )?;
                }
            } else {
                // Interactive mode: prompt for all values
                let method = prompt_choice("Auth method", &["header", "query_param"], "header")?;

                if method == "header" {
                    let header_name = prompt_string("Header name", "Authorization")?;

                    let fmt_str =
                        prompt_choice("Secret format", &["raw", "bearer", "basic"], "raw")?;
                    let fmt = match fmt_str.as_str() {
                        "bearer" => CredentialSecretFormat::Bearer,
                        "basic" => CredentialSecretFormat::Basic,
                        _ => CredentialSecretFormat::Raw,
                    };

                    let secret = read_secret_value(
                        value,
                        &format!("Enter value for header '{header_name}'"),
                    )?;
                    let secret = format_secret_value(secret, fmt)?;

                    let effective_url = match url {
                        Some(u) => Some(u),
                        None => {
                            let input = prompt_string_optional(
                                "Endpoint URL (optional, press Enter to skip)",
                            )?;
                            if input.is_empty() { None } else { Some(input) }
                        }
                    };

                    node_config.add_header_credential_via(
                        &service,
                        &header_name,
                        &secret,
                        effective_url.as_deref(),
                        &backend,
                    )?;
                } else {
                    let param_name = prompt_string("Query param name", "api_key")?;

                    let secret = read_secret_value(
                        value,
                        &format!("Enter value for query param '{param_name}'"),
                    )?;

                    let effective_url = match url {
                        Some(u) => Some(u),
                        None => {
                            let input = prompt_string_optional(
                                "Endpoint URL (optional, press Enter to skip)",
                            )?;
                            if input.is_empty() { None } else { Some(input) }
                        }
                    };

                    node_config.add_query_param_credential_via(
                        &service,
                        &param_name,
                        &secret,
                        effective_url.as_deref(),
                        &backend,
                    )?;
                }
            }

            node_config.save(&config_file)?;
            println!("Credential added for service '{service}'.");
            Ok(())
        }
        NodeCredentialCommands::Setup {
            service,
            additional_scopes,
            api_url,
            access_token,
        } => {
            let additional_scopes = normalize_cli_scopes(&additional_scopes);
            cmd_credentials_setup(
                &config_file,
                &config_dir,
                &service,
                &additional_scopes,
                api_url.as_deref(),
                access_token.as_deref(),
            )
            .await
        }

        NodeCredentialCommands::List => {
            let node_config = NodeConfig::load(&config_file)?;
            let backend = SecretBackend::from_config(&node_config, &config_dir)?;
            let creds = CredentialStore::from_config_with_backend(&node_config, &backend)?;

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
        NodeCredentialCommands::Pending => {
            let node_config = NodeConfig::load(&config_file)?;
            let backend = SecretBackend::from_config(&node_config, &config_dir)?;
            let pending = fetch_pending_credentials_for_node(&node_config, &backend).await?;

            if pending.is_empty() {
                println!("No pending credentials.");
                return Ok(());
            }

            println!("Pending credentials:");
            for item in pending {
                println!(
                    "  {}: {} {} (age {}, expires {})",
                    item.service_slug,
                    item.injection_method,
                    item.field_name,
                    format_pending_age(&item.created_at),
                    item.expires_at
                );
                if let Some(label) = item.label.as_deref().filter(|label| !label.is_empty()) {
                    println!("    Label: {label}");
                }
                if let Some(url) = item.target_url.as_deref().filter(|url| !url.is_empty()) {
                    println!("    Verify target URL before accepting: {url}");
                }
            }
            Ok(())
        }
        NodeCredentialCommands::Accept { slug, value_env } => {
            let service = slug.to_lowercase();
            let mut node_config = NodeConfig::load(&config_file)?;
            let backend = SecretBackend::from_config(&node_config, &config_dir)?;
            let pending = find_pending_credential(&node_config, &backend, &service).await?;

            if let Some(url) = pending.target_url.as_deref().filter(|url| !url.is_empty()) {
                println!("Verify target URL before accepting: {url}");
            }

            let secret = match value_env {
                Some(env_name) => {
                    let value = std::env::var(&env_name).map_err(|_| {
                        super::error::Error::Validation(format!(
                            "Environment variable '{env_name}' is not set"
                        ))
                    })?;
                    read_secret_value(Some(value), "credential value")?
                }
                None => prompt_secret(&format!("Enter credential value for '{service}'"))?,
            };

            store_pending_credential_locally(&mut node_config, &backend, &pending, &secret)?;
            node_config.save(&config_file)?;

            let mut api = node_agent_api_client(&node_config, &backend)?;
            api.post_empty(
                &format!("/node-agent/pending-credentials/{}/consume", pending.id),
                &serde_json::json!({}),
            )
            .await
            .map_err(|error| {
                super::error::Error::Config(format!("Failed to mark credential consumed: {error}"))
            })?;

            println!(
                "Credential accepted for '{}' ({}).",
                pending.service_slug, pending.injection_method
            );
            Ok(())
        }
        NodeCredentialCommands::Decline { slug, reason } => {
            let service = slug.to_lowercase();
            let node_config = NodeConfig::load(&config_file)?;
            let backend = SecretBackend::from_config(&node_config, &config_dir)?;
            let pending = find_pending_credential(&node_config, &backend, &service).await?;
            let mut api = node_agent_api_client(&node_config, &backend)?;
            api.post_empty(
                &format!("/node-agent/pending-credentials/{}/decline", pending.id),
                &serde_json::json!({ "reason": reason }),
            )
            .await
            .map_err(|error| {
                super::error::Error::Config(format!("Failed to decline credential: {error}"))
            })?;

            println!("Credential push declined for '{}'.", pending.service_slug);
            Ok(())
        }
        NodeCredentialCommands::AddOauth {
            service: raw_service,
            from_catalog,
            client_id,
            client_secret,
            authorization_url,
            token_url,
            device_code_url,
            scopes,
            additional_scopes,
            url,
            api_url,
            access_token,
        } => {
            let service = raw_service.to_lowercase();
            let additional_scopes = normalize_cli_scopes(&additional_scopes);
            cmd_credentials_add_oauth(
                &config_file,
                &config_dir,
                &service,
                from_catalog,
                client_id,
                client_secret,
                authorization_url,
                token_url,
                device_code_url,
                scopes,
                &additional_scopes,
                url,
                api_url,
                access_token,
            )
            .await
        }
        NodeCredentialCommands::Remove {
            service: raw_service,
        } => {
            let service = raw_service.to_lowercase();
            let mut node_config = NodeConfig::load(&config_file)?;
            let backend = SecretBackend::from_config(&node_config, &config_dir)?;
            node_config.remove_credential_via(&service, &backend)?;
            node_config.save(&config_file)?;
            println!("Credential removed for service '{service}'.");
            Ok(())
        }
    }
}

pub async fn cmd_ssh_credentials(
    command: crate::cli::NodeSshCredentialCommands,
    config_path: Option<&str>,
) -> Result<()> {
    use crate::cli::NodeSshCredentialCommands;

    init_cli_tracing();

    let config_dir = config::resolve_config_dir(config_path);
    let config_file = config_dir.join("config.toml");

    match command {
        NodeSshCredentialCommands::Add {
            service,
            principal,
            key_file,
            host,
            port,
            passphrase_env,
            no_pin_host_key,
            kex,
            host_key,
            cipher,
            mac,
        } => {
            let service = service.to_lowercase();
            let algorithms = algorithm_preferences_from_cli(kex, host_key, cipher, mac)?;
            let mut private_key_pem =
                Zeroizing::new(std::fs::read_to_string(&key_file).map_err(|error| {
                    super::error::Error::Config(format!(
                        "Failed to read key file {}: {error}",
                        key_file.display()
                    ))
                })?);
            if private_key_pem.trim().is_empty() {
                return Err(super::error::Error::Validation(
                    "Private key file is empty".to_string(),
                ));
            }
            if !private_key_pem.ends_with('\n') {
                private_key_pem.push('\n');
            }

            let passphrase = match passphrase_env {
                Some(env) => {
                    let value = std::env::var(&env).map_err(|_| {
                        super::error::Error::Validation(format!(
                            "Environment variable '{env}' is not set"
                        ))
                    })?;
                    if value.is_empty() {
                        return Err(super::error::Error::Validation(format!(
                            "Environment variable '{env}' is empty"
                        )));
                    }
                    Some(Zeroizing::new(value))
                }
                None => None,
            };

            let host_key_sha256 = if no_pin_host_key {
                None
            } else {
                let fingerprint =
                    ssh_node_exec::scan_host_key_sha256(&host, port, 10, algorithms.as_ref())
                        .await
                        .map_err(|error| super::error::Error::Config(error.message))?;
                Some(fingerprint)
            };

            let mut node_config = NodeConfig::load(&config_file)?;
            let backend = SecretBackend::from_config(&node_config, &config_dir)?;
            ssh_keys::add_entry(
                &mut node_config,
                &backend,
                NewSshKeyEntry {
                    service_slug: service.clone(),
                    principal: principal.clone(),
                    private_key_pem,
                    passphrase,
                    target_host: host.clone(),
                    target_port: port,
                    host_key_sha256: host_key_sha256.clone(),
                    algorithms,
                },
            )?;
            node_config.save(&config_file)?;

            println!("SSH key added for service '{service}' principal '{principal}'.");
            if let Some(fingerprint) = host_key_sha256 {
                println!("Pinned host key: {fingerprint}");
            } else {
                println!("Host key pinning disabled for this entry.");
            }
            Ok(())
        }
        NodeSshCredentialCommands::SetAlgos {
            service,
            principal,
            kex,
            host_key,
            cipher,
            mac,
            reset_kex,
            reset_host_key,
            reset_cipher,
            reset_mac,
            reset_all,
        } => {
            validate_algorithm_reset_args(AlgorithmResetArgs {
                kex: &kex,
                host_key: &host_key,
                cipher: &cipher,
                mac: &mac,
                reset_kex,
                reset_host_key,
                reset_cipher,
                reset_mac,
                reset_all,
            })?;

            let service = service.to_lowercase();
            let mut node_config = NodeConfig::load(&config_file)?;
            let entry = node_config
                .ssh_keys
                .iter_mut()
                .find(|entry| entry.service_slug == service && entry.principal == principal)
                .ok_or_else(|| {
                    Error::Config(format!(
                        "No SSH key found for service '{service}' principal '{principal}'"
                    ))
                })?;

            if reset_all {
                entry.algorithms = None;
            } else {
                let mut algorithms = entry.algorithms.clone().unwrap_or_default();
                if reset_kex {
                    algorithms.kex = None;
                }
                if reset_host_key {
                    algorithms.host_key = None;
                }
                if reset_cipher {
                    algorithms.cipher = None;
                }
                if reset_mac {
                    algorithms.mac = None;
                }
                if !kex.is_empty() {
                    algorithms.kex = Some(kex);
                }
                if !host_key.is_empty() {
                    algorithms.host_key = Some(host_key);
                }
                if !cipher.is_empty() {
                    algorithms.cipher = Some(cipher);
                }
                if !mac.is_empty() {
                    algorithms.mac = Some(mac);
                }
                entry.algorithms = normalize_algorithm_preferences(algorithms)?;
            }

            let algorithms = entry.algorithms.clone();
            node_config.save(&config_file)?;
            println!(
                "SSH algorithm allowlists updated for service '{service}' principal '{principal}'."
            );
            print_algorithm_allowlists(algorithms.as_ref());
            Ok(())
        }
        NodeSshCredentialCommands::List { service } => {
            let node_config = NodeConfig::load(&config_file)?;
            let mut entries = node_config.ssh_keys.clone();
            if let Some(service) = service.as_deref() {
                entries.retain(|entry| entry.service_slug == service);
            }
            entries.sort_by(|a, b| {
                (&a.service_slug, &a.principal).cmp(&(&b.service_slug, &b.principal))
            });
            if entries.is_empty() {
                println!("No SSH node-key credentials configured.");
            } else {
                println!("SSH node-key credentials:");
                for entry in entries {
                    let pin = entry.host_key_sha256.as_deref().unwrap_or("unpinned");
                    println!(
                        "  {} / {} -> {}:{} ({})",
                        entry.service_slug,
                        entry.principal,
                        entry.target_host,
                        entry.target_port,
                        pin
                    );
                }
            }
            Ok(())
        }
        NodeSshCredentialCommands::Show { service, principal } => {
            let node_config = NodeConfig::load(&config_file)?;
            let entry =
                ssh_keys::config_find(&node_config, &service, &principal).ok_or_else(|| {
                    super::error::Error::Config(format!(
                        "No SSH key found for service '{service}' principal '{principal}'"
                    ))
                })?;
            println!("Service:    {}", entry.service_slug);
            println!("Principal:  {}", entry.principal);
            println!("Target:     {}:{}", entry.target_host, entry.target_port);
            println!(
                "Host key:   {}",
                entry.host_key_sha256.as_deref().unwrap_or("unpinned")
            );
            print_algorithm_allowlists(entry.algorithms.as_ref());
            println!("Created at: {}", entry.created_at);
            println!("Key:        <redacted>");
            println!(
                "Passphrase: {}",
                if entry.passphrase_encrypted.is_some() {
                    "<redacted>"
                } else {
                    "none"
                }
            );
            Ok(())
        }
        NodeSshCredentialCommands::Remove { service, principal } => {
            let mut node_config = NodeConfig::load(&config_file)?;
            let backend = SecretBackend::from_config(&node_config, &config_dir)?;
            ssh_keys::remove_entry(&mut node_config, &backend, &service, &principal)?;
            node_config.save(&config_file)?;
            println!("SSH key removed for service '{service}' principal '{principal}'.");
            Ok(())
        }
        NodeSshCredentialCommands::Test { service, principal } => {
            let node_config = NodeConfig::load(&config_file)?;
            let backend = SecretBackend::from_config(&node_config, &config_dir)?;
            let stored =
                ssh_keys::config_find(&node_config, &service, &principal).ok_or_else(|| {
                    super::error::Error::Config(format!(
                        "No SSH key found for service '{service}' principal '{principal}'"
                    ))
                })?;
            let entry = ssh_keys::load_entry(stored, &backend)?;
            let result = ssh_node_exec::test_connection(entry, 10).await;
            match result {
                Ok(()) => {
                    println!(
                        "SSH key test succeeded for service '{service}' principal '{principal}'."
                    );
                    Ok(())
                }
                Err(error) => Err(super::error::Error::Config(error.message)),
            }
        }
        NodeSshCredentialCommands::Prune { stale, auth } => {
            if !stale {
                return Err(super::error::Error::Validation(
                    "Only --stale pruning is supported".to_string(),
                ));
            }
            let mut node_config = NodeConfig::load(&config_file)?;
            let backend = SecretBackend::from_config(&node_config, &config_dir)?;
            let mut api = crate::api::ApiClient::from_auth(&auth)
                .map_err(|error| super::error::Error::Config(error.to_string()))?;
            let resp: serde_json::Value = api
                .get("/keys")
                .await
                .map_err(|error| super::error::Error::Config(error.to_string()))?;
            let node_key_slugs = resp
                .get("keys")
                .and_then(|v| v.as_array())
                .into_iter()
                .flatten()
                .filter(|item| {
                    item.get("ssh_auth_mode").and_then(|v| v.as_str()) == Some("node_key")
                })
                .filter_map(|item| {
                    item.get("slug")
                        .or_else(|| item.get("service_slug"))
                        .and_then(|v| v.as_str())
                        .map(str::to_string)
                })
                .collect::<std::collections::BTreeSet<_>>();

            let stale_entries = node_config
                .ssh_keys
                .iter()
                .filter(|entry| !node_key_slugs.contains(&entry.service_slug))
                .map(|entry| (entry.service_slug.clone(), entry.principal.clone()))
                .collect::<Vec<_>>();
            for (service, principal) in &stale_entries {
                let _ = ssh_keys::remove_entry(&mut node_config, &backend, service, principal)?;
            }
            node_config.save(&config_file)?;
            println!("Pruned {} stale SSH key entries.", stale_entries.len());
            Ok(())
        }
    }
}

fn algorithm_preferences_from_cli(
    kex: Vec<String>,
    host_key: Vec<String>,
    cipher: Vec<String>,
    mac: Vec<String>,
) -> Result<Option<SshAlgorithmPreferences>> {
    normalize_algorithm_preferences(SshAlgorithmPreferences {
        kex: non_empty_algorithm_list(kex),
        host_key: non_empty_algorithm_list(host_key),
        cipher: non_empty_algorithm_list(cipher),
        mac: non_empty_algorithm_list(mac),
    })
}

fn normalize_algorithm_preferences(
    algorithms: SshAlgorithmPreferences,
) -> Result<Option<SshAlgorithmPreferences>> {
    if algorithms.is_empty() {
        return Ok(None);
    }
    algorithms.validate()?;
    Ok(Some(algorithms))
}

fn non_empty_algorithm_list(values: Vec<String>) -> Option<Vec<String>> {
    if values.is_empty() {
        None
    } else {
        Some(values)
    }
}

struct AlgorithmResetArgs<'a> {
    kex: &'a [String],
    host_key: &'a [String],
    cipher: &'a [String],
    mac: &'a [String],
    reset_kex: bool,
    reset_host_key: bool,
    reset_cipher: bool,
    reset_mac: bool,
    reset_all: bool,
}

fn validate_algorithm_reset_args(args: AlgorithmResetArgs<'_>) -> Result<()> {
    let any_list = !args.kex.is_empty()
        || !args.host_key.is_empty()
        || !args.cipher.is_empty()
        || !args.mac.is_empty();
    let any_reset = args.reset_kex
        || args.reset_host_key
        || args.reset_cipher
        || args.reset_mac
        || args.reset_all;
    if !any_list && !any_reset {
        return Err(Error::Validation(
            "set-algos requires at least one of --kex, --host-key, --cipher, --mac, \
             --reset-kex, --reset-host-key, --reset-cipher, --reset-mac, or --reset-all"
                .to_string(),
        ));
    }
    if args.reset_all && any_list {
        return Err(Error::Validation(
            "--reset-all cannot be combined with algorithm allowlists".to_string(),
        ));
    }
    if args.reset_kex && !args.kex.is_empty() {
        return Err(Error::Validation(
            "--kex cannot be combined with --reset-kex".to_string(),
        ));
    }
    if args.reset_host_key && !args.host_key.is_empty() {
        return Err(Error::Validation(
            "--host-key cannot be combined with --reset-host-key".to_string(),
        ));
    }
    if args.reset_cipher && !args.cipher.is_empty() {
        return Err(Error::Validation(
            "--cipher cannot be combined with --reset-cipher".to_string(),
        ));
    }
    if args.reset_mac && !args.mac.is_empty() {
        return Err(Error::Validation(
            "--mac cannot be combined with --reset-mac".to_string(),
        ));
    }
    Ok(())
}

fn print_algorithm_allowlists(algorithms: Option<&SshAlgorithmPreferences>) {
    println!("Algorithms:");
    println!(
        "  KEX:       {}",
        format_algorithm_allowlist(algorithms.and_then(|prefs| prefs.kex.as_deref()))
    );
    println!(
        "  Host key:  {}",
        format_algorithm_allowlist(algorithms.and_then(|prefs| prefs.host_key.as_deref()))
    );
    println!(
        "  Cipher:    {}",
        format_algorithm_allowlist(algorithms.and_then(|prefs| prefs.cipher.as_deref()))
    );
    println!(
        "  MAC:       {}",
        format_algorithm_allowlist(algorithms.and_then(|prefs| prefs.mac.as_deref()))
    );
}

fn format_algorithm_allowlist(list: Option<&[String]>) -> String {
    match list {
        Some(values) if !values.is_empty() => values.join(", "),
        Some(_) => "<empty>".to_string(),
        None => "default".to_string(),
    }
}

// ---------------------------------------------------------------------------
// OpenClaw subcommands
// ---------------------------------------------------------------------------

const OPENCLAW_SERVICE_SLUG: &str = "llm-openclaw";

pub async fn cmd_openclaw(
    command: crate::cli::NodeOpenClawCommands,
    config_path: Option<&str>,
) -> Result<()> {
    use crate::cli::NodeOpenClawCommands;

    let config_dir = config::resolve_config_dir(config_path);
    let config_file = config_dir.join("config.toml");

    match command {
        NodeOpenClawCommands::Connect {
            url,
            token,
            api_url,
            access_token,
        } => {
            cmd_openclaw_connect(
                &config_file,
                &config_dir,
                &url,
                token,
                api_url,
                access_token,
            )
            .await
        }
        NodeOpenClawCommands::Status => cmd_openclaw_status(&config_file, &config_dir),
        NodeOpenClawCommands::Disconnect => cmd_openclaw_disconnect(&config_file, &config_dir),
    }
}

// ---------------------------------------------------------------------------
// Tracing init
// ---------------------------------------------------------------------------

fn init_tracing(log_level: Option<&str>) {
    use tracing_subscriber::EnvFilter;

    let level = log_level.unwrap_or("info");
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(level)),
        )
        .try_init();
}

fn init_cli_tracing() {
    use tracing_subscriber::EnvFilter;

    if std::env::var_os("RUST_LOG").is_none() {
        return;
    }
    let _ = tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(EnvFilter::from_default_env())
        .try_init();
}

// ---------------------------------------------------------------------------
// Internal helpers (from node-agent main.rs)
// ---------------------------------------------------------------------------

/// Poll the config file mtime and reload credentials when it changes.
async fn credential_reload_loop(
    config_file: std::path::PathBuf,
    sender: std::sync::Arc<SharedCredentialsSender>,
    interval: Duration,
    backend: std::sync::Arc<SecretBackend>,
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

        let node_config = match NodeConfig::load(&config_file) {
            Ok(c) => c,
            Err(e) => {
                tracing::error!(error = %e, "Failed to reload config, keeping existing credentials");
                continue;
            }
        };

        // Refresh in-memory vault cache so newly-added keychain secrets are visible.
        if let Err(e) = backend.refresh() {
            tracing::error!(error = %e, "Failed to refresh secret backend, keeping existing");
            continue;
        }

        match CredentialStore::from_config_with_backend(&node_config, &backend) {
            Ok(new_store) => {
                // Only mark as processed after successful reload so that a
                // transient failure (e.g. keychain vault not yet flushed by
                // another process) retries on the next tick.
                last_modified = Some(current_modified);
                let count = new_store.count();
                sender.update(new_store);
                tracing::info!(credentials = count, "Credentials reloaded from config");
            }
            Err(e) => {
                tracing::error!(error = %e, "Failed to reload credentials, will retry");
            }
        }
    }
}

/// Background task that refreshes OAuth tokens before they expire.
async fn oauth_refresh_loop(
    config_file: std::path::PathBuf,
    interval: Duration,
    backend: std::sync::Arc<SecretBackend>,
) {
    loop {
        tokio::time::sleep(interval).await;

        let node_config = match NodeConfig::load(&config_file) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let mut config_changed = false;
        let mut updated_config = node_config.clone();

        for (slug, cred) in &node_config.credentials {
            if !cred.oauth_managed {
                continue;
            }

            // Check if token expires within 5 minutes
            let needs_refresh = match &cred.oauth_token_expires_at {
                Some(expires_str) => match chrono::DateTime::parse_from_rfc3339(expires_str) {
                    Ok(expires) => {
                        let now = chrono::Utc::now();
                        let buffer = chrono::Duration::minutes(5);
                        expires.with_timezone(&chrono::Utc) - buffer < now
                    }
                    Err(_) => false,
                },
                None => false,
            };

            if !needs_refresh {
                continue;
            }

            // Load refresh token
            let refresh_tok = match &cred.oauth_refresh_token_encrypted {
                Some(enc) => {
                    match backend
                        .load_credential_value(&format!("{slug}:oauth_refresh"), Some(enc.as_str()))
                    {
                        Ok(t) => t,
                        Err(_) => continue,
                    }
                }
                None => continue,
            };

            let client_id = match &cred.oauth_client_id_encrypted {
                Some(enc) => {
                    match backend
                        .load_credential_value(&format!("{slug}:oauth_cid"), Some(enc.as_str()))
                    {
                        Ok(t) => t,
                        Err(_) => continue,
                    }
                }
                None => continue,
            };

            let client_secret = cred.oauth_client_secret_encrypted.as_ref().and_then(|enc| {
                backend
                    .load_credential_value(&format!("{slug}:oauth_csecret"), Some(enc.as_str()))
                    .ok()
            });

            let token_url = match &cred.oauth_token_url {
                Some(url) => url.as_str(),
                None => continue,
            };

            let auth_method = cred
                .oauth_token_endpoint_auth_method
                .as_deref()
                .unwrap_or("client_secret_post");

            // Attempt refresh
            match oauth::refresh_token(
                token_url,
                &client_id,
                client_secret.as_deref(),
                &refresh_tok,
                auth_method,
                cred.oauth_client_id_param_name.as_deref(),
            )
            .await
            {
                Ok(new_token) => {
                    tracing::info!(service = %slug, "OAuth token refreshed");

                    let header_value = format!(
                        "{} {}",
                        new_token.token_type.as_deref().unwrap_or("Bearer"),
                        new_token.access_token
                    );

                    if let Some(cred_mut) = updated_config.credentials.get_mut(slug) {
                        // Update header value
                        cred_mut.header_value_encrypted = backend
                            .store_credential_value(slug, &header_value)
                            .ok()
                            .flatten();

                        // Update OAuth tokens
                        cred_mut.oauth_access_token_encrypted = backend
                            .store_credential_value(
                                &format!("{slug}:oauth_access"),
                                &new_token.access_token,
                            )
                            .ok()
                            .flatten();

                        if let Some(ref rt) = new_token.refresh_token {
                            cred_mut.oauth_refresh_token_encrypted = backend
                                .store_credential_value(&format!("{slug}:oauth_refresh"), rt)
                                .ok()
                                .flatten();
                        }

                        cred_mut.oauth_token_expires_at = new_token.expires_in.map(|secs| {
                            (chrono::Utc::now() + chrono::Duration::seconds(secs as i64))
                                .to_rfc3339()
                        });

                        config_changed = true;
                    }
                }
                Err(e) => {
                    tracing::warn!(service = %slug, error = %e, "OAuth token refresh failed");
                }
            }
        }

        if config_changed && let Err(e) = updated_config.save(&config_file) {
            tracing::error!(error = %e, "Failed to save config after OAuth refresh");
        }
    }
}

fn api_base_url_from_node_ws_url(ws_url: &str) -> String {
    let http_url = if let Some(rest) = ws_url.strip_prefix("ws://") {
        format!("http://{rest}")
    } else if let Some(rest) = ws_url.strip_prefix("wss://") {
        format!("https://{rest}")
    } else {
        ws_url.to_string()
    };

    http_url
        .trim_end_matches('/')
        .strip_suffix("/api/v1/nodes/ws")
        .unwrap_or_else(|| http_url.trim_end_matches('/'))
        .to_string()
}

fn node_agent_api_client(
    node_config: &NodeConfig,
    backend: &SecretBackend,
) -> Result<crate::api::ApiClient> {
    let auth_token = backend.load_auth_token(node_config)?;
    let base_api_url = api_base_url_from_node_ws_url(&node_config.server.url);
    crate::api::ApiClient::new(&base_api_url, auth_token)
        .map(crate::api::ApiClient::without_token_refresh)
        .map_err(|error| {
            super::error::Error::Config(format!("Failed to create node-agent API client: {error}"))
        })
}

async fn fetch_pending_credentials_for_node(
    node_config: &NodeConfig,
    backend: &SecretBackend,
) -> Result<Vec<PendingCredentialMetadata>> {
    let mut api = node_agent_api_client(node_config, backend)?;
    let response: PendingCredentialListResponse = api
        .get("/node-agent/pending-credentials")
        .await
        .map_err(|error| {
            super::error::Error::Config(format!("Failed to list pending credentials: {error}"))
        })?;
    Ok(response.pending_credentials)
}

async fn find_pending_credential(
    node_config: &NodeConfig,
    backend: &SecretBackend,
    service_slug: &str,
) -> Result<PendingCredentialMetadata> {
    let pending = fetch_pending_credentials_for_node(node_config, backend).await?;
    pending
        .into_iter()
        .find(|item| item.service_slug == service_slug)
        .ok_or_else(|| {
            super::error::Error::Config(format!(
                "No pending credential found for service '{service_slug}'"
            ))
        })
}

fn store_pending_credential_locally(
    node_config: &mut NodeConfig,
    backend: &SecretBackend,
    pending: &PendingCredentialMetadata,
    secret: &str,
) -> Result<()> {
    match pending.injection_method.as_str() {
        "header" => node_config.add_header_credential_via(
            &pending.service_slug,
            &pending.field_name,
            secret,
            pending.target_url.as_deref(),
            backend,
        ),
        "query-param" => node_config.add_query_param_credential_via(
            &pending.service_slug,
            &pending.field_name,
            secret,
            pending.target_url.as_deref(),
            backend,
        ),
        "path-prefix" => node_config.add_path_prefix_credential_via(
            &pending.service_slug,
            &pending.field_name,
            secret,
            pending.target_url.as_deref(),
            backend,
        ),
        other => Err(super::error::Error::Validation(format!(
            "Unsupported injection method '{other}'"
        ))),
    }
}

fn format_pending_age(created_at: &str) -> String {
    let Ok(created) = chrono::DateTime::parse_from_rfc3339(created_at) else {
        return "-".to_string();
    };
    let age = chrono::Utc::now().signed_duration_since(created.with_timezone(&chrono::Utc));
    let seconds = age.num_seconds().max(0);
    if seconds < 60 {
        format!("{seconds}s")
    } else if seconds < 3_600 {
        format!("{}m", seconds / 60)
    } else if seconds < 86_400 {
        format!("{}h", seconds / 3_600)
    } else {
        format!("{}d", seconds / 86_400)
    }
}

fn read_secret_value(value: Option<String>, prompt: &str) -> Result<String> {
    let value = match value {
        Some(v) => v,
        None => prompt_secret(prompt)?,
    };
    if value.is_empty() {
        return Err(super::error::Error::Validation(
            "Secret value must not be empty".to_string(),
        ));
    }
    Ok(value)
}

fn format_secret_value(secret: String, secret_format: CredentialSecretFormat) -> Result<String> {
    match secret_format {
        CredentialSecretFormat::Raw => Ok(secret),
        CredentialSecretFormat::Bearer => {
            if secret.starts_with("Bearer ") {
                Ok(secret)
            } else {
                Ok(format!("Bearer {secret}"))
            }
        }
        CredentialSecretFormat::Basic => {
            if secret.starts_with("Basic ") {
                Ok(secret)
            } else if secret.contains(':') {
                let encoded = base64::engine::general_purpose::STANDARD.encode(secret);
                Ok(format!("Basic {encoded}"))
            } else {
                Err(super::error::Error::Validation(
                    "Basic auth secret must be in 'username:password' format".to_string(),
                ))
            }
        }
    }
}

fn prompt_secret(prompt: &str) -> Result<String> {
    let value = rpassword::prompt_password(format!("{prompt}: "))
        .map_err(|e| super::error::Error::Validation(format!("Failed to read secret: {e}")))?;
    read_secret_value(Some(value), prompt)
}

/// Normalize repeated `--scope` CLI inputs into a trimmed, deduped list.
///
/// Accepts comma- or whitespace-separated values within a single entry and
/// across multiple entries, e.g. `--scope a,b --scope "c d"`. Matches the
/// behavior of `nyxid service add --scope` so the two paths feel identical.
fn normalize_cli_scopes(raw: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for entry in raw {
        for piece in entry.split(|c: char| c == ',' || c.is_whitespace()) {
            let trimmed = piece.trim();
            if !trimmed.is_empty() && !out.iter().any(|existing| existing == trimmed) {
                out.push(trimmed.to_string());
            }
        }
    }
    out
}

/// Merge default OAuth scopes with user-supplied additional scopes, preserving
/// the order of `defaults` and appending any extras that are not already
/// present. Dedup is case-sensitive (RFC 6749 §3.3).
fn merge_oauth_scopes(defaults: &[String], additional: &[String]) -> Vec<String> {
    let mut merged: Vec<String> = defaults.to_vec();
    for scope in additional {
        if !merged.iter().any(|existing| existing == scope) {
            merged.push(scope.clone());
        }
    }
    merged
}

fn parse_header(header: &str) -> Result<(String, String)> {
    let (name, value) = header.split_once(':').ok_or_else(|| {
        super::error::Error::Validation(
            "Header must be in 'Name: value' format (e.g., 'Authorization: Bearer sk-...')"
                .to_string(),
        )
    })?;
    Ok((name.trim().to_string(), value.trim().to_string()))
}

fn parse_query_param(param: &str) -> Result<(String, String)> {
    let (name, value) = param.split_once('=').ok_or_else(|| {
        super::error::Error::Validation(
            "Query param must be in 'name=value' format (e.g., 'api_key=sk-...')".to_string(),
        )
    })?;
    Ok((name.to_string(), value.to_string()))
}

fn prompt_string(label: &str, default: &str) -> Result<String> {
    use std::io::Write;
    print!("{label} [{default}]: ");
    std::io::stdout()
        .flush()
        .map_err(|e| super::error::Error::Validation(format!("Failed to flush stdout: {e}")))?;
    let mut input = String::new();
    std::io::stdin()
        .read_line(&mut input)
        .map_err(|e| super::error::Error::Validation(format!("Failed to read input: {e}")))?;
    let trimmed = input.trim();
    Ok(if trimmed.is_empty() {
        default.to_string()
    } else {
        trimmed.to_string()
    })
}

fn prompt_string_optional(label: &str) -> Result<String> {
    use std::io::Write;
    print!("{label}: ");
    std::io::stdout()
        .flush()
        .map_err(|e| super::error::Error::Validation(format!("Failed to flush stdout: {e}")))?;
    let mut input = String::new();
    std::io::stdin()
        .read_line(&mut input)
        .map_err(|e| super::error::Error::Validation(format!("Failed to read input: {e}")))?;
    Ok(input.trim().to_string())
}

fn prompt_choice(label: &str, options: &[&str], default: &str) -> Result<String> {
    use std::io::Write;
    let options_str = options.join("/");
    print!("{label} ({options_str}) [{default}]: ");
    std::io::stdout()
        .flush()
        .map_err(|e| super::error::Error::Validation(format!("Failed to flush stdout: {e}")))?;
    let mut input = String::new();
    std::io::stdin()
        .read_line(&mut input)
        .map_err(|e| super::error::Error::Validation(format!("Failed to read input: {e}")))?;
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Ok(default.to_string());
    }
    if options.contains(&trimmed) {
        Ok(trimmed.to_string())
    } else {
        Err(super::error::Error::Validation(format!(
            "Invalid choice '{trimmed}', expected one of: {options_str}"
        )))
    }
}

// ---------------------------------------------------------------------------
// Migration helpers
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
struct MigrationReport {
    cleanup_warnings: Vec<String>,
}

fn migrate_config(
    node_config: &mut NodeConfig,
    source: &SecretBackend,
    target: &SecretBackend,
    target_backend: &str,
    config_file: &Path,
) -> Result<MigrationReport> {
    let auth_token = source.load_auth_token(node_config)?;
    let signing_secret = source.load_signing_secret(node_config)?;

    let mut credential_values = Vec::new();
    for (slug, cred_config) in &node_config.credentials {
        let value = source.load_credential_value(
            slug,
            cred_config
                .header_value_encrypted
                .as_deref()
                .or(cred_config.param_value_encrypted.as_deref()),
        )?;
        credential_values.push((slug.clone(), cred_config.injection_method.clone(), value));
    }

    let mut updated = node_config.clone();
    target.store_auth_token(&mut updated, &auth_token)?;
    if let Some(ref secret) = signing_secret {
        target.store_signing_secret(&mut updated, secret)?;
    }

    for (slug, injection_method, value) in &credential_values {
        let encrypted = target.store_credential_value(slug, value)?;
        if let Some(cred_config) = updated.credentials.get_mut(slug) {
            match injection_method.as_str() {
                // path_prefix and the two cloud-billing methods all
                // store the encrypted credential on
                // `header_value_encrypted` (Codex review REC 10). The
                // pre-NyxID#716 migration code only knew about
                // `"header"` and `"query_param"`, so a backend switch
                // from file → keychain (or vice versa) on a node with
                // a path-prefix or cloud-billing credential would
                // silently drop the encrypted-payload pointer.
                "header" | "path_prefix" | "aws_sigv4" | "gcp_service_account" => {
                    cred_config.header_value_encrypted = encrypted;
                }
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
    *node_config = updated;
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

// ---------------------------------------------------------------------------
// OpenClaw helpers
// ---------------------------------------------------------------------------

async fn cmd_openclaw_connect(
    config_file: &Path,
    config_dir: &Path,
    gateway_url: &str,
    token: Option<String>,
    api_url: Option<String>,
    access_token: Option<String>,
) -> Result<()> {
    // Validate gateway URL
    if !gateway_url.starts_with("http://") && !gateway_url.starts_with("https://") {
        return Err(super::error::Error::Validation(
            "Gateway URL must start with http:// or https://".to_string(),
        ));
    }

    // Get bearer token (prompt if not provided)
    let bearer_token = match token {
        Some(t) => t,
        None => {
            println!("Enter your OpenClaw gateway bearer token (OPENCLAW_GATEWAY_TOKEN):");
            prompt_secret("Bearer token")?
        }
    };

    // 1. Store credential locally on the node
    let mut node_config = NodeConfig::load(config_file)?;
    let backend = SecretBackend::from_config(&node_config, config_dir)?;

    let header_value = format!("Bearer {bearer_token}");
    node_config.add_header_credential_via(
        OPENCLAW_SERVICE_SLUG,
        "Authorization",
        &header_value,
        Some(gateway_url),
        &backend,
    )?;
    node_config.save(config_file)?;
    println!("Local credential stored for '{OPENCLAW_SERVICE_SLUG}'.");

    // 2. Create the NyxID-side node-routed AI Service (if an access token is available).
    let nyxid_token = access_token
        .or_else(crate::auth::read_saved_token)
        .or_else(|| std::env::var("NYXID_ACCESS_TOKEN").ok())
        .filter(|s| !s.is_empty());

    let base_api_url = api_url.unwrap_or_else(|| {
        // Derive HTTP API URL from the WS URL in config
        let ws_url = &node_config.server.url;
        ws_url
            .replace("ws://", "http://")
            .replace("wss://", "https://")
            .replace("/api/v1/nodes/ws", "")
    });

    if let Some(ref token) = nyxid_token {
        match crate::api::ApiClient::new(&base_api_url, token.clone()) {
            Ok(mut api) => {
                let existing_keys = api.get_value("/keys").await;
                let already_registered = existing_keys
                    .ok()
                    .and_then(|value| value["keys"].as_array().cloned())
                    .is_some_and(|keys| {
                        keys.iter().any(|key| {
                            key["slug"].as_str() == Some(OPENCLAW_SERVICE_SLUG)
                                && key["node_id"].as_str() == Some(node_config.node.id.as_str())
                                && key["is_active"].as_bool().unwrap_or(true)
                        })
                    });

                if already_registered {
                    println!("NyxID AI Service already exists for '{OPENCLAW_SERVICE_SLUG}'.");
                } else {
                    let create_result = api
                        .post::<serde_json::Value, _>(
                            "/keys",
                            &serde_json::json!({
                                "service_slug": OPENCLAW_SERVICE_SLUG,
                                "label": "OpenClaw",
                                "node_id": node_config.node.id,
                            }),
                        )
                        .await;

                    match create_result {
                        Ok(result) => {
                            let slug = result["slug"].as_str().unwrap_or(OPENCLAW_SERVICE_SLUG);
                            println!("NyxID AI Service created: {slug}");
                        }
                        Err(e) => {
                            println!("Warning: Could not create NyxID AI Service: {e}");
                        }
                    }
                }
            }
            Err(e) => {
                println!("Warning: Could not create API client for NyxID sync: {e}");
            }
        }
    } else {
        println!(
            "No NyxID access token provided (--access-token, NYXID_ACCESS_TOKEN env var, or `nyxid login`)."
        );
        println!(
            "Local credential stored. To complete setup, create or route the OpenClaw AI Service to this node in NyxID."
        );
    }

    println!();
    println!("OpenClaw connected at {gateway_url}");
    println!("Start the node agent with: nyxid node start");
    Ok(())
}

fn cmd_openclaw_status(config_file: &Path, config_dir: &Path) -> Result<()> {
    let node_config = NodeConfig::load(config_file)?;
    let backend = SecretBackend::from_config(&node_config, config_dir)?;
    let creds = CredentialStore::from_config_with_backend(&node_config, &backend)?;

    if let Some(cred) = creds.get(OPENCLAW_SERVICE_SLUG) {
        println!("OpenClaw: connected");
        println!(
            "  Injection: {} ({})",
            cred.injection_method(),
            cred.target_name()
        );
    } else {
        println!("OpenClaw: not connected");
        println!("  Run 'nyxid node openclaw connect --url <gateway-url>' to connect.");
    }

    Ok(())
}

fn cmd_openclaw_disconnect(config_file: &Path, config_dir: &Path) -> Result<()> {
    let mut node_config = NodeConfig::load(config_file)?;
    let backend = SecretBackend::from_config(&node_config, config_dir)?;
    node_config.remove_credential_via(OPENCLAW_SERVICE_SLUG, &backend)?;
    node_config.save(config_file)?;
    println!("OpenClaw credentials removed from node.");
    println!("Note: To fully disconnect, also remove the binding in the NyxID web UI.");
    Ok(())
}

// ---------------------------------------------------------------------------
// Credential setup (auto from catalog)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
struct ExistingServiceMatch {
    id: String,
    endpoint_url: String,
    /// `true` when the service already points at the requested node.
    already_on_node: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum BackendRegistrationChange {
    None,
    Created {
        id: String,
    },
    UpdatedEndpoint {
        id: String,
        previous_endpoint_url: String,
    },
    AssignedNode {
        id: String,
    },
}

/// Find an existing service that matches by slug (or catalog_service_slug).
/// Prefers a match that is already on the requested node; falls back to an
/// unassigned service (no node_id) so that `credentials setup` can adopt it.
fn find_existing_service(
    keys_response: &serde_json::Value,
    service_slug: &str,
    node_id: &str,
) -> Option<ExistingServiceMatch> {
    let keys = keys_response["keys"].as_array()?;

    let matches_slug = |entry: &serde_json::Value| -> bool {
        entry["catalog_service_slug"]
            .as_str()
            .map(|slug| slug == service_slug)
            .unwrap_or_else(|| entry["slug"].as_str() == Some(service_slug))
    };

    // Prefer: same node_id
    if let Some(entry) = keys
        .iter()
        .find(|e| e["node_id"].as_str() == Some(node_id) && matches_slug(e))
    {
        let id = entry["id"].as_str().or(entry["_id"].as_str())?;
        return Some(ExistingServiceMatch {
            id: id.to_string(),
            endpoint_url: entry["endpoint_url"].as_str().unwrap_or("").to_string(),
            already_on_node: true,
        });
    }

    // Fallback: slug matches but no node assigned yet
    if let Some(entry) = keys
        .iter()
        .find(|e| e["node_id"].as_str().is_none_or(|n| n.is_empty()) && matches_slug(e))
    {
        let id = entry["id"].as_str().or(entry["_id"].as_str())?;
        return Some(ExistingServiceMatch {
            id: id.to_string(),
            endpoint_url: entry["endpoint_url"].as_str().unwrap_or("").to_string(),
            already_on_node: false,
        });
    }

    None
}

async fn prepare_backend_service_registration(
    api: &mut crate::api::ApiClient,
    service_slug: &str,
    label: &str,
    node_id: &str,
    target_url: Option<&str>,
) -> Result<BackendRegistrationChange> {
    let keys_response = api.get_value("/keys").await.map_err(|e| {
        super::error::Error::Validation(format!(
            "Failed to list existing services before registering '{service_slug}': {e}"
        ))
    })?;

    if let Some(existing) = find_existing_service(&keys_response, service_slug, node_id) {
        if existing.already_on_node {
            println!("Service '{service_slug}' already registered in backend for node {node_id}.");
        } else {
            // Service exists but is not yet routed via a node -- assign it.
            println!("Routing service '{service_slug}' through node {node_id}...");
            api.put::<serde_json::Value, _>(
                &format!("/keys/{}", existing.id),
                &serde_json::json!({ "node_id": node_id }),
            )
            .await
            .map_err(|e| {
                super::error::Error::Validation(format!(
                    "Failed to assign node to service '{service_slug}': {e}"
                ))
            })?;
            println!("Service routed through node.");
        }

        if let Some(url) = target_url.filter(|url| *url != existing.endpoint_url) {
            println!("Updating backend endpoint URL for '{service_slug}'...");
            api.put::<serde_json::Value, _>(
                &format!("/keys/{}", existing.id),
                &serde_json::json!({ "endpoint_url": url }),
            )
            .await
            .map_err(|e| {
                super::error::Error::Validation(format!(
                    "Failed to update backend endpoint URL for '{service_slug}': {e}"
                ))
            })?;
            println!("Backend endpoint updated.");
            return Ok(BackendRegistrationChange::UpdatedEndpoint {
                id: existing.id,
                previous_endpoint_url: existing.endpoint_url,
            });
        }

        return if existing.already_on_node {
            Ok(BackendRegistrationChange::None)
        } else {
            Ok(BackendRegistrationChange::AssignedNode { id: existing.id })
        };
    }

    println!("Registering service '{service_slug}' in backend (node_id: {node_id})...");
    let mut body = serde_json::json!({
        "service_slug": service_slug,
        "label": label,
        "node_id": node_id,
    });
    if let Some(url) = target_url {
        body["endpoint_url"] = serde_json::Value::String(url.to_string());
    }

    let response: serde_json::Value = api.post("/keys", &body).await.map_err(|e| {
        super::error::Error::Validation(format!(
            "Failed to register service '{service_slug}' in backend: {e}"
        ))
    })?;
    let id = response["id"]
        .as_str()
        .or(response["_id"].as_str())
        .ok_or_else(|| {
            super::error::Error::Validation(format!(
                "Backend registration for '{service_slug}' succeeded without returning a service ID"
            ))
        })?;
    println!("Service registered in backend.");

    Ok(BackendRegistrationChange::Created { id: id.to_string() })
}

async fn rollback_backend_service_registration(
    api: &mut crate::api::ApiClient,
    service_slug: &str,
    change: &BackendRegistrationChange,
) -> Result<()> {
    match change {
        BackendRegistrationChange::None => Ok(()),
        BackendRegistrationChange::Created { id } => {
            eprintln!("Rolling back backend service registration for '{service_slug}'...");
            api.delete_empty(&format!("/keys/{id}"))
                .await
                .map_err(|e| {
                    super::error::Error::Validation(format!(
                        "Failed to roll back backend registration for '{service_slug}': {e}"
                    ))
                })?;
            Ok(())
        }
        BackendRegistrationChange::AssignedNode { id } => {
            eprintln!("Removing node assignment for '{service_slug}'...");
            api.put::<serde_json::Value, _>(
                &format!("/keys/{id}"),
                // Backend treats "" as clear, null as leave-unchanged.
                &serde_json::json!({ "node_id": "" }),
            )
            .await
            .map_err(|e| {
                super::error::Error::Validation(format!(
                    "Failed to remove node assignment for '{service_slug}': {e}"
                ))
            })?;
            Ok(())
        }
        BackendRegistrationChange::UpdatedEndpoint {
            id,
            previous_endpoint_url,
        } => {
            eprintln!("Restoring previous backend endpoint URL for '{service_slug}'...");
            api.put::<serde_json::Value, _>(
                &format!("/keys/{id}"),
                &serde_json::json!({ "endpoint_url": previous_endpoint_url }),
            )
            .await
            .map_err(|e| {
                super::error::Error::Validation(format!(
                    "Failed to restore backend endpoint URL for '{service_slug}': {e}"
                ))
            })?;
            Ok(())
        }
    }
}

async fn cmd_credentials_setup(
    config_file: &Path,
    config_dir: &Path,
    raw_service: &str,
    additional_scopes: &[String],
    api_url: Option<&str>,
    access_token: Option<&str>,
) -> Result<()> {
    let service = raw_service.to_lowercase();
    let service = service.as_str();
    let node_config = NodeConfig::load(config_file)?;

    // Resolve API URL from config
    let base_api_url = api_url.map(|s| s.to_string()).unwrap_or_else(|| {
        node_config
            .server
            .url
            .replace("ws://", "http://")
            .replace("wss://", "https://")
            .replace("/api/v1/nodes/ws", "")
    });

    // Use ApiClient for auto token refresh
    let token_str = access_token
        .map(|s| s.to_string())
        .or_else(crate::auth::read_saved_token)
        .or_else(|| std::env::var("NYXID_ACCESS_TOKEN").ok())
        .ok_or_else(|| {
            super::error::Error::Validation(
                "No access token. Run `nyxid login` first, set NYXID_ACCESS_TOKEN, or pass --access-token".to_string(),
            )
        })?;

    let mut api = crate::api::ApiClient::new(&base_api_url, token_str.clone()).map_err(|e| {
        super::error::Error::Validation(format!("Failed to create API client: {e}"))
    })?;

    // Try catalog first; fall back to the user's existing keys for custom
    // endpoints or suffixed catalog slugs (e.g. "openai-2").
    let catalog_entry: Option<serde_json::Value> = api
        .get_optional(&format!("/catalog/{service}"))
        .await
        .map_err(|e| {
            super::error::Error::Validation(format!(
                "Failed to fetch catalog entry for '{service}': {e}"
            ))
        })?;

    // If the slug wasn't found in the catalog, look it up in the user's keys.
    // For suffixed slugs the keys entry may have a catalog_service_slug we can
    // use to fetch the original catalog entry for OAuth URLs / instructions.
    let (entry, from_catalog) = if let Some(entry) = catalog_entry {
        (entry, true)
    } else {
        let keys_response: serde_json::Value = api.get_value("/keys").await.map_err(|e| {
            super::error::Error::Validation(format!(
                "Failed to list keys while resolving '{service}': {e}"
            ))
        })?;
        let matched = keys_response["keys"]
            .as_array()
            .and_then(|keys| keys.iter().find(|k| k["slug"].as_str() == Some(service)))
            .cloned();
        match matched {
            Some(key_entry) => {
                // If the key was provisioned from a catalog service, try to
                // fetch the original catalog entry for extra metadata.
                if let Some(cat_slug) = key_entry["catalog_service_slug"].as_str() {
                    if let Some(cat) = api
                        .get_optional::<serde_json::Value>(&format!("/catalog/{cat_slug}"))
                        .await
                        .unwrap_or(None)
                    {
                        // Merge: catalog provides OAuth URLs / instructions,
                        // key entry overrides auth_method, auth_key_name, etc.
                        let mut merged = cat;
                        for field in ["auth_method", "auth_key_name"] {
                            if let Some(v) = key_entry[field].as_str() {
                                merged[field] = serde_json::Value::String(v.to_string());
                            }
                        }
                        (merged, true)
                    } else {
                        (key_entry, false)
                    }
                } else {
                    (key_entry, false)
                }
            }
            None => {
                return Err(super::error::Error::Validation(format!(
                    "Service '{service}' not found in catalog or your keys. \
                     Create it first via the web UI or `nyxid keys create`."
                )));
            }
        }
    };

    let provider_type = entry["provider_type"]
        .as_str()
        .or_else(|| {
            // Keys entries use "credential_type" instead of "provider_type"
            match entry["credential_type"].as_str() {
                Some("oauth2") => Some("oauth2"),
                _ => None,
            }
        })
        .unwrap_or("api_key");
    let credential_mode = entry["credential_mode"].as_str().unwrap_or("admin");
    let auth_method = entry["auth_method"].as_str().unwrap_or("bearer");
    let auth_key_name = entry["auth_key_name"].as_str().unwrap_or("Authorization");
    let default_url = entry["base_url"]
        .as_str()
        .or_else(|| entry["endpoint_url"].as_str())
        .unwrap_or("");
    let requires_gw = entry["requires_gateway_url"].as_bool().unwrap_or(false);
    let svc_name = entry["name"]
        .as_str()
        .or_else(|| entry["label"].as_str())
        .unwrap_or(service);

    println!("Setting up credentials for: {svc_name} ({service})");
    println!("  Provider type:   {provider_type}");
    if from_catalog {
        println!("  Credential mode: {credential_mode}");
    }
    println!("  Auth method:     {auth_method}");
    println!();

    // Resolve target URL early so it can be used for backend registration.
    let target_url = if requires_gw {
        println!("This service requires your instance URL.");
        eprint!("Enter your instance URL: ");
        std::io::Write::flush(&mut std::io::stderr())?;
        let mut url = String::new();
        std::io::stdin().read_line(&mut url)?;
        let url = url.trim().to_string();
        if url.is_empty() {
            return Err(super::error::Error::Validation(
                "Instance URL is required for this service".to_string(),
            ));
        }
        if !url.starts_with("http://") && !url.starts_with("https://") && !url.starts_with("ssh://")
        {
            return Err(super::error::Error::Validation(
                "Instance URL must start with http://, https://, or ssh://".to_string(),
            ));
        }
        Some(url)
    } else if !default_url.is_empty() && !default_url.contains(".invalid") {
        Some(default_url.to_string())
    } else {
        None
    };

    let node_id = &node_config.node.id;
    let backend_change = prepare_backend_service_registration(
        &mut api,
        service,
        svc_name,
        node_id,
        target_url.as_deref(),
    )
    .await?;
    println!();

    let setup_result = match provider_type {
        "oauth2" | "device_code" => {
            println!("This service requires OAuth authentication.");
            if credential_mode == "user" || credential_mode == "both" {
                println!(
                    "You need to provide your own OAuth app credentials (client_id and client_secret)."
                );
                println!();
            }
            println!("Running OAuth flow from the node...");
            println!();

            // Delegate to add-oauth with --from-catalog. Pass through any
            // additional scopes the caller supplied so they get merged with
            // the catalog's default_scopes.
            cmd_credentials_add_oauth(
                config_file,
                config_dir,
                service,
                from_catalog,
                None, // client_id (will be prompted if needed)
                None, // client_secret
                None,
                None,
                None,
                None, // scopes (legacy replacement) -- not used in setup path
                additional_scopes,
                target_url,
                Some(base_api_url),
                Some(token_str.clone()),
            )
            .await
        }
        _ => {
            if !additional_scopes.is_empty() {
                eprintln!(
                    "warning: --scope has no effect on API-key services (scopes apply to OAuth flows)"
                );
            }

            let is_cloud_billing = matches!(auth_method, "aws_sigv4" | "gcp_service_account");
            if is_cloud_billing {
                println!(
                    "This service uses {auth_method}. Paste the JSON credential payload \
                     (Ctrl-D / EOF when done):"
                );
            } else {
                println!("This service requires an API key / bearer token.");
            }
            if let Some(ref url) = entry["api_key_url"].as_str() {
                println!("  Get your API key at: {url}");
            }
            if let Some(ref instructions) = entry["api_key_instructions"].as_str() {
                println!("  {instructions}");
            }
            println!();

            // Cloud-billing credentials are multi-line JSON blobs and
            // can't be entered safely through rpassword (which masks
            // input and strips newlines). Read from stdin until EOF so
            // the user can paste the full SA JSON / access-key JSON.
            // NyxID#716 + Codex review BLOCKER 6.
            let secret = if is_cloud_billing {
                use std::io::Read;
                let mut buf = String::new();
                std::io::stdin().read_to_string(&mut buf).map_err(|e| {
                    super::error::Error::Validation(format!(
                        "Failed to read credential JSON from stdin: {e}"
                    ))
                })?;
                buf.trim().to_string()
            } else {
                rpassword::prompt_password("Enter credential (hidden): ")?
            };
            if secret.is_empty() {
                return Err(super::error::Error::Validation(
                    "Credential is required".to_string(),
                ));
            }

            let mut node_config = NodeConfig::load(config_file)?;
            let backend = SecretBackend::from_config(&node_config, config_dir)?;

            // Store credential using the injection method matching the auth method
            if auth_method == "path" {
                node_config.add_path_prefix_credential_via(
                    service,
                    auth_key_name,
                    &secret,
                    target_url.as_deref(),
                    &backend,
                )?;
            } else if auth_method == "query" {
                node_config.add_query_param_credential_via(
                    service,
                    auth_key_name,
                    &secret,
                    target_url.as_deref(),
                    &backend,
                )?;
            } else if auth_method == "aws_sigv4" {
                node_config.add_aws_sigv4_credential_via(
                    service,
                    &secret,
                    target_url.as_deref(),
                    &backend,
                )?;
            } else if auth_method == "gcp_service_account" {
                node_config.add_gcp_service_account_credential_via(
                    service,
                    &secret,
                    target_url.as_deref(),
                    &backend,
                )?;
            } else {
                // Format credential based on auth method
                let header_value = if auth_method == "bearer" {
                    format!("Bearer {secret}")
                } else {
                    secret
                };
                node_config.add_header_credential_via(
                    service,
                    auth_key_name,
                    &header_value,
                    target_url.as_deref(),
                    &backend,
                )?;
            }

            node_config.save(config_file)?;
            println!("Credential added for service '{service}'.");
            if let Some(url) = target_url {
                println!("  Target URL: {url}");
            }
            println!("  Auth: {auth_method} / {auth_key_name}");
            Ok(())
        }
    };

    if let Err(error) = setup_result {
        if let Err(rollback_error) =
            rollback_backend_service_registration(&mut api, service, &backend_change).await
        {
            eprintln!(
                "Warning: backend rollback for service '{service}' failed after local setup error: {rollback_error}"
            );
        }
        return Err(error);
    }

    Ok(())
}

#[allow(clippy::too_many_arguments)]
async fn cmd_credentials_add_oauth(
    config_file: &Path,
    config_dir: &Path,
    service: &str,
    from_catalog: bool,
    client_id: Option<String>,
    client_secret: Option<String>,
    authorization_url: Option<String>,
    token_url: Option<String>,
    device_code_url: Option<String>,
    scopes: Option<String>,
    additional_scopes: &[String],
    target_url: Option<String>,
    api_url: Option<String>,
    access_token: Option<String>,
) -> Result<()> {
    let mut node_config = NodeConfig::load(config_file)?;
    let backend = SecretBackend::from_config(&node_config, config_dir)?;

    // 1. Get OAuth config
    let oauth_config = if from_catalog {
        let base_api_url = api_url.unwrap_or_else(|| {
            node_config
                .server
                .url
                .replace("ws://", "http://")
                .replace("wss://", "https://")
                .replace("/api/v1/nodes/ws", "")
        });
        let token = access_token
            .or_else(crate::auth::read_saved_token)
            .or_else(|| std::env::var("NYXID_ACCESS_TOKEN").ok())
            .filter(|s| !s.is_empty())
            .ok_or_else(|| {
                super::error::Error::Validation(
                    "No access token. Run `nyxid login` first, set NYXID_ACCESS_TOKEN, or pass --access-token".to_string(),
                )
            })?;
        let mut api = crate::api::ApiClient::new(&base_api_url, token).map_err(|e| {
            super::error::Error::Validation(format!("Failed to create API client: {e}"))
        })?;
        let entry: serde_json::Value =
            api.get(&format!("/catalog/{service}")).await.map_err(|e| {
                super::error::Error::Validation(format!(
                    "Failed to fetch catalog entry for '{service}': {e}"
                ))
            })?;

        oauth::oauth_config_from_catalog_value(&entry)?
    } else {
        let tok_url = token_url.ok_or_else(|| {
            super::error::Error::Validation(
                "--token-url is required when not using --from-catalog".to_string(),
            )
        })?;
        oauth::OAuthConfig {
            authorization_url,
            token_url: tok_url,
            device_code_url,
            device_verification_url: None,
            device_token_url: None,
            default_scopes: scopes
                .as_deref()
                .map(|s| s.split_whitespace().map(String::from).collect())
                .unwrap_or_default(),
            supports_pkce: false,
            device_code_format: "rfc8628".to_string(),
            token_endpoint_auth_method: "client_secret_post".to_string(),
            extra_auth_params: None,
            oauth_client_id: None,
            client_id_param_name: None,
        }
    };

    // 2. Get client credentials
    let cid = match client_id.or_else(|| oauth_config.oauth_client_id.clone()) {
        Some(id) => id,
        None => {
            let entered = prompt_string_optional("OAuth Client ID")?;
            if entered.is_empty() {
                return Err(super::error::Error::Validation(
                    "OAuth client ID is required".to_string(),
                ));
            }
            entered
        }
    };
    let csecret = match client_secret {
        Some(s) => Some(s),
        None => {
            println!("Enter OAuth Client Secret (press Enter to skip for public clients):");
            match rpassword::prompt_password("Client Secret: ") {
                Ok(s) if s.is_empty() => None,
                Ok(s) => Some(s),
                Err(_) => None,
            }
        }
    };

    // 3. Determine scopes
    //
    // `scopes` (--scopes) is the legacy power-user escape hatch: if set, it
    // replaces the catalog's default_scopes entirely. `additional_scopes`
    // (--scope, repeatable) is the additive path from issue #181: any extras
    // are merged on top of whichever base scope set we ended up using.
    //
    // Backward-compat: when the caller supplied no additional scopes we take
    // the exact pre-feature code path (single `unwrap_or_else`) so any edge-
    // case whitespace / empty-string behavior of the legacy `--scopes` flag
    // is preserved byte-for-byte. Only the new `--scope` path goes through
    // the split + merge logic.
    let final_scopes = if additional_scopes.is_empty() {
        scopes.unwrap_or_else(|| oauth_config.default_scopes.join(" "))
    } else {
        // OpenAI-format device code providers do not accept a `scope` field,
        // so reject additional scopes for them explicitly (mirrors the backend
        // `ensure_additional_scopes_supported` check).
        if oauth_config.device_code_format == "openai" {
            return Err(super::error::Error::Validation(
                "This provider's device code endpoint does not accept additional OAuth scopes \
                 (OpenAI-format device code providers ignore the `scope` parameter). \
                 Remove --scope and try again."
                    .to_string(),
            ));
        }

        let base_scopes: Vec<String> = match scopes.as_deref() {
            Some(s) if !s.trim().is_empty() => s.split_whitespace().map(String::from).collect(),
            _ => oauth_config.default_scopes.clone(),
        };
        merge_oauth_scopes(&base_scopes, additional_scopes).join(" ")
    };

    // 4. Run the OAuth flow
    let token_response = if oauth_config.device_code_url.is_some() {
        oauth::run_device_code_flow(&oauth_config, &cid, csecret.as_deref(), &final_scopes).await?
    } else {
        oauth::run_authorization_code_flow(&oauth_config, &cid, csecret.as_deref(), &final_scopes)
            .await?
    };

    // 5. Store tokens locally
    let header_value = format!(
        "{} {}",
        token_response.token_type.as_deref().unwrap_or("Bearer"),
        token_response.access_token
    );

    let expires_at = token_response
        .expires_in
        .map(|secs| (chrono::Utc::now() + chrono::Duration::seconds(secs as i64)).to_rfc3339());

    // Store the header credential (for immediate use by proxy_executor)
    node_config.add_header_credential_via(
        service,
        "Authorization",
        &header_value,
        target_url.as_deref(),
        &backend,
    )?;

    // Store OAuth metadata for refresh
    if let Some(cred) = node_config.credentials.get_mut(service) {
        cred.oauth_managed = true;
        cred.oauth_token_url = Some(oauth_config.token_url.clone());
        cred.oauth_access_token_encrypted = backend.store_credential_value(
            &format!("{service}:oauth_access"),
            &token_response.access_token,
        )?;
        if let Some(ref rt) = token_response.refresh_token {
            cred.oauth_refresh_token_encrypted =
                backend.store_credential_value(&format!("{service}:oauth_refresh"), rt)?;
        }
        cred.oauth_token_expires_at = expires_at;
        cred.oauth_client_id_encrypted =
            backend.store_credential_value(&format!("{service}:oauth_cid"), &cid)?;
        if let Some(ref cs) = csecret {
            cred.oauth_client_secret_encrypted =
                backend.store_credential_value(&format!("{service}:oauth_csecret"), cs)?;
        }
        cred.oauth_scopes = if final_scopes.is_empty() {
            None
        } else {
            Some(final_scopes)
        };
        cred.oauth_token_endpoint_auth_method = Some(oauth_config.token_endpoint_auth_method);
        cred.oauth_client_id_param_name = oauth_config.client_id_param_name;
    }

    node_config.save(config_file)?;
    println!("OAuth credential stored for service '{service}'.");
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::super::encryption::LocalEncryption;
    use super::super::error::Error;
    use super::*;

    #[test]
    fn normalize_cli_scopes_splits_mixed_separators() {
        let input = vec![
            "contact:contact.base:readonly,contact:department.base:readonly".to_string(),
            "attendance:record:read scope.with.dots".to_string(),
            "  ".to_string(), // whitespace-only entry is ignored
            "contact:contact.base:readonly".to_string(), // duplicate is deduped
        ];
        let out = normalize_cli_scopes(&input);
        assert_eq!(
            out,
            vec![
                "contact:contact.base:readonly".to_string(),
                "contact:department.base:readonly".to_string(),
                "attendance:record:read".to_string(),
                "scope.with.dots".to_string(),
            ]
        );
    }

    #[test]
    fn normalize_cli_scopes_returns_empty_for_empty_input() {
        assert!(normalize_cli_scopes(&[]).is_empty());
        assert!(normalize_cli_scopes(&["".to_string(), "  ,  ".to_string()]).is_empty());
    }

    #[test]
    fn merge_oauth_scopes_preserves_defaults_and_appends_extras() {
        let defaults = vec!["openid".to_string(), "email".to_string()];
        let extras = vec![
            "profile".to_string(),
            "email".to_string(), // dedup
            "offline_access".to_string(),
        ];
        let merged = merge_oauth_scopes(&defaults, &extras);
        assert_eq!(
            merged,
            vec![
                "openid".to_string(),
                "email".to_string(),
                "profile".to_string(),
                "offline_access".to_string(),
            ]
        );
    }

    #[test]
    fn merge_oauth_scopes_handles_no_defaults_and_no_extras() {
        assert!(merge_oauth_scopes(&[], &[]).is_empty());
        assert_eq!(
            merge_oauth_scopes(&["a".to_string()], &[]),
            vec!["a".to_string()]
        );
        assert_eq!(
            merge_oauth_scopes(&[], &["b".to_string()]),
            vec!["b".to_string()]
        );
    }

    #[test]
    fn find_existing_service_matches_catalog_slug_on_same_node() {
        let keys_response = json!({
            "keys": [
                {
                    "id": "direct-1",
                    "slug": "llm-openai",
                    "node_id": null,
                    "endpoint_url": "https://api.openai.com/v1"
                },
                {
                    "id": "node-1",
                    "slug": "llm-openai-2",
                    "catalog_service_slug": "llm-openai",
                    "node_id": "node-a",
                    "endpoint_url": "https://gateway.example.com/v1"
                },
                {
                    "id": "node-2",
                    "slug": "llm-openai-3",
                    "catalog_service_slug": "llm-openai",
                    "node_id": "node-b",
                    "endpoint_url": "https://other.example.com/v1"
                }
            ]
        });

        let existing = find_existing_service(&keys_response, "llm-openai", "node-a").unwrap();
        assert_eq!(existing.id, "node-1");
        assert!(existing.already_on_node);
        assert_eq!(existing.endpoint_url, "https://gateway.example.com/v1");
    }

    #[test]
    fn find_existing_service_falls_back_to_unassigned_service() {
        let keys_response = json!({
            "keys": [
                {
                    "id": "direct-1",
                    "slug": "llm-openai",
                    "node_id": null,
                    "endpoint_url": "https://api.openai.com/v1"
                },
                {
                    "id": "node-2",
                    "slug": "llm-openai-2",
                    "catalog_service_slug": "llm-openai",
                    "node_id": "node-b",
                    "endpoint_url": "https://other.example.com/v1"
                }
            ]
        });

        // No service on node-a, but "direct-1" has no node and matches slug
        let existing = find_existing_service(&keys_response, "llm-openai", "node-a").unwrap();
        assert_eq!(existing.id, "direct-1");
        assert!(!existing.already_on_node);
    }

    #[test]
    fn find_existing_service_falls_back_to_user_slug_when_needed() {
        let keys_response = json!({
            "keys": [
                {
                    "id": "node-1",
                    "slug": "llm-openai",
                    "node_id": "node-a",
                    "endpoint_url": ""
                }
            ]
        });

        let existing = find_existing_service(&keys_response, "llm-openai", "node-a").unwrap();
        assert_eq!(existing.id, "node-1");
        assert!(existing.already_on_node);
    }

    #[test]
    fn find_existing_service_matches_custom_endpoint_by_slug() {
        let keys_response = json!({
            "keys": [
                {
                    "id": "custom-1",
                    "slug": "testing-gh4t",
                    "node_id": null,
                    "endpoint_url": "https://my-api.example.com"
                }
            ]
        });

        let existing = find_existing_service(&keys_response, "testing-gh4t", "node-a").unwrap();
        assert_eq!(existing.id, "custom-1");
        assert!(!existing.already_on_node);
    }

    #[test]
    fn format_secret_value_supports_raw_bearer_and_basic() {
        assert_eq!(
            format_secret_value("sk-test".to_string(), CredentialSecretFormat::Raw).unwrap(),
            "sk-test"
        );
        assert_eq!(
            format_secret_value("sk-test".to_string(), CredentialSecretFormat::Bearer).unwrap(),
            "Bearer sk-test"
        );
        assert_eq!(
            format_secret_value("user:pass".to_string(), CredentialSecretFormat::Basic).unwrap(),
            "Basic dXNlcjpwYXNz"
        );
    }

    #[test]
    fn format_secret_value_accepts_existing_bearer_and_basic_prefixes() {
        assert_eq!(
            format_secret_value("Bearer sk-test".to_string(), CredentialSecretFormat::Bearer,)
                .unwrap(),
            "Bearer sk-test"
        );
        assert_eq!(
            format_secret_value(
                "Basic dXNlcjpwYXNz".to_string(),
                CredentialSecretFormat::Basic,
            )
            .unwrap(),
            "Basic dXNlcjpwYXNz"
        );
    }

    #[test]
    fn basic_secret_requires_username_password_pair() {
        let err = format_secret_value("token-only".to_string(), CredentialSecretFormat::Basic)
            .unwrap_err();
        assert!(matches!(err, Error::Validation(_)));
    }

    #[test]
    fn api_base_url_from_node_ws_url_derives_http_api_root() {
        assert_eq!(
            api_base_url_from_node_ws_url("ws://localhost:3001/api/v1/nodes/ws"),
            "http://localhost:3001"
        );
        assert_eq!(
            api_base_url_from_node_ws_url("wss://auth.example.com/api/v1/nodes/ws"),
            "https://auth.example.com"
        );
    }

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
            .add_header_credential_via("openai", "Authorization", "Bearer sk-test", None, &source)
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
        use std::fs;
        use std::os::unix::fs::PermissionsExt;

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
            .add_header_credential_via("openai", "Authorization", "Bearer sk-test", None, &source)
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
            .add_header_credential_via("openai", "Authorization", "Bearer sk-test", None, &backend)
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
        assert!(backend.load_auth_token(&config).is_err());
        assert!(backend.load_signing_secret(&config).is_err());
        assert!(backend.load_credential_value("openai", None).is_err());
    }
}

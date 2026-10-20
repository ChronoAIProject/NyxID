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
use super::ssh_host_key_store::{
    CertHostKeySource, CertHostKeyStore, CertHostKeyStoreError, canonical_sha256_fingerprint,
};
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
    let backend_name = if use_keychain { "keychain" } else { "file" };
    let custody = super::registration::RegistrationCustody::acquire(&config_dir, backend_name)?;
    custody.preflight()?;

    // M2: Use ws:// for localhost (wss:// requires TLS not available in dev)
    let ws_url = url.unwrap_or("ws://localhost:3001/api/v1/nodes/ws");

    tracing::info!(url = %ws_url, "Registering node...");

    let (node_id, auth_token, signing_secret) = ws_client::register_node(ws_url, token).await?;

    tracing::info!(node_id = %node_id, "Registration successful");

    let auth_token = Zeroizing::new(auth_token);
    let signing_secret = signing_secret.map(Zeroizing::new);
    let config_file = custody.config_file().to_owned();
    let keychain = super::keychain::KeychainBackend::new(&node_id);
    let mut node_config = NodeConfig::new(ws_url.to_string(), node_id, backend_name.to_string());
    let operation = custody.into_operation(&node_config, keychain)?;
    operation.prepare(&mut node_config)?;
    node_config.node.auth_token_encrypted =
        operation.store(super::secret_fields::SecretField::AuthToken, &auth_token)?;
    if let Some(secret) = signing_secret {
        node_config.signing.shared_secret_encrypted =
            Some(operation.store(super::secret_fields::SecretField::SigningSecret, &secret)?);
    }
    operation.publish(&node_config, &config_file)?;

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
    // Invalid explicit trust configuration is permanent for this process.
    crate::tls::shared_config()?;

    let config_dir = config::resolve_config_dir(config_path);
    let config_file = config_dir.join("config.toml");

    let directory = super::storage::Directory::open(&config_dir)?;
    // Acquire before reading secrets and retain through the runtime and OAuth
    // shutdown. OS inventory also refuses this process until it exits; a free
    // lease alone never proves that detached or legacy workers have stopped.
    let _runtime_lease = directory.runtime_lease(false)?;
    let node_config = NodeConfig::load_in(&directory, &config_file)?;
    let backend = SecretBackend::reader_in(&node_config, &config_file, directory)?;
    oauth::storage::recover_pending(&backend)?;
    let (node_config, backend) = backend.current_reader()?;

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

    // Spawn OAuth refresh loop (check every 60 seconds)
    let (refresh_shutdown, refresh_stopped) = tokio::sync::oneshot::channel();
    let refresh_handle = tokio::spawn(oauth_refresh_loop(
        Duration::from_secs(60),
        std::sync::Arc::clone(&backend),
        refresh_stopped,
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

    // Stop further ticks without cancelling an in-flight provider request or
    // dropping returned material that is awaiting durable storage repair.
    let _ = refresh_shutdown.send(());
    refresh_handle
        .await
        .map_err(|_| Error::Config("oauth_refresh_task_failed".into()))?;
    Ok(())
}

pub fn cmd_status(config_path: Option<&str>) -> Result<()> {
    let config_dir = config::resolve_config_dir(config_path);
    status_to_writer(&config_dir, None, &mut std::io::stdout().lock())
}

fn status_to_writer(
    config_dir: &Path,
    keychain: Option<super::keychain::KeychainBackend>,
    output: &mut impl std::io::Write,
) -> Result<()> {
    let node_config = NodeConfig::load(&config_dir.join("config.toml"))?;
    let backend = match keychain {
        Some(keychain) => SecretBackend::reader_with_keychain(&node_config, config_dir, keychain)?,
        None => SecretBackend::reader(&node_config, config_dir)?,
    };
    let credentials = CredentialStore::from_config_with_backend(&node_config, &backend)?;

    writeln!(output, "Node Status")?;
    writeln!(output, "  Node ID:     {}", node_config.node.id)?;
    writeln!(output, "  Server:      {}", node_config.server.url)?;
    writeln!(output, "  Storage:     {}", node_config.storage_backend)?;
    writeln!(output, "  Credentials: {} configured", credentials.count())?;
    for slug in credentials.service_slugs() {
        writeln!(output, "    - {slug}")?;
    }
    Ok(())
}

pub fn cmd_rekey(auth_token: &str, signing_secret: &str, config_path: Option<&str>) -> Result<()> {
    let config_dir = config::resolve_config_dir(config_path);
    let config_file = config_dir.join("config.toml");

    let mut node_config = NodeConfig::load(&config_file)?;
    let backend = SecretBackend::writer(&mut node_config, &config_dir)?;

    backend.store_auth_token(&mut node_config, auth_token)?;
    backend.store_signing_secret(&mut node_config, signing_secret)?;
    backend.publish(&node_config, &config_file)?;

    println!("Node credentials updated.");
    println!("Restart the agent to reconnect with the rotated credentials.");
    Ok(())
}

pub fn cmd_version() {
    println!("nyxid node agent {}", env!("CARGO_PKG_VERSION"));
}

pub(crate) fn cmd_migrate(
    target_backend: &str,
    config_path: Option<&str>,
    dry_run: bool,
    status: bool,
    cutover: super::daemon::Cutover<'_>,
) -> Result<()> {
    super::migration::command_with_cutover(target_backend, config_path, dry_run, status, cutover)
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
            let mut node_config = load_node_config_for_credentials(&config_file, &config_dir)?;
            let backend = SecretBackend::writer(&mut node_config, &config_dir)?;

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

            backend.publish(&node_config, &config_file)?;
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
            let node_config = load_node_config_for_credentials(&config_file, &config_dir)?;
            let backend = SecretBackend::reader(&node_config, &config_dir)?;
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
            let node_config = load_node_config_for_credentials(&config_file, &config_dir)?;
            let backend = SecretBackend::reader(&node_config, &config_dir)?;
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
            let mut node_config = load_node_config_for_credentials(&config_file, &config_dir)?;
            let backend = SecretBackend::writer(&mut node_config, &config_dir)?;
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
            backend.publish(&node_config, &config_file)?;

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
            let node_config = load_node_config_for_credentials(&config_file, &config_dir)?;
            let backend = SecretBackend::reader(&node_config, &config_dir)?;
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
            let mut node_config = load_node_config_for_credentials(&config_file, &config_dir)?;
            let backend = SecretBackend::writer(&mut node_config, &config_dir)?;
            node_config.remove_credential_via(&service, &backend)?;
            backend.publish(&node_config, &config_file)?;
            println!("Credential removed for service '{service}'.");
            Ok(())
        }
    }
}

fn load_node_config_for_credentials(config_file: &Path, config_dir: &Path) -> Result<NodeConfig> {
    if !config_file.exists() {
        return Err(Error::Config(format!(
            "No local node agent config found at {}. Run this command on the machine where the node agent is installed (SSH there first); `nyxid node credentials` is node-side only, not available on the user-side CLI.",
            config_dir.display()
        )));
    }

    NodeConfig::load(config_file)
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
            let backend = SecretBackend::writer(&mut node_config, &config_dir)?;
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
            backend.publish(&node_config, &config_file)?;

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
            let backend = SecretBackend::writer(&mut node_config, &config_dir)?;
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
            backend.publish(&node_config, &config_file)?;
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
            let backend = SecretBackend::writer(&mut node_config, &config_dir)?;
            ssh_keys::remove_entry(&mut node_config, &backend, &service, &principal)?;
            backend.publish(&node_config, &config_file)?;
            println!("SSH key removed for service '{service}' principal '{principal}'.");
            Ok(())
        }
        NodeSshCredentialCommands::Test { service, principal } => {
            let node_config = NodeConfig::load(&config_file)?;
            let backend = SecretBackend::reader(&node_config, &config_dir)?;
            let stored =
                ssh_keys::config_find(&node_config, &service, &principal).ok_or_else(|| {
                    super::error::Error::Config(format!(
                        "No SSH key found for service '{service}' principal '{principal}'"
                    ))
                })?;
            let entry = ssh_keys::load_entry(&node_config, stored, &backend)?;
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
            let backend = SecretBackend::writer(&mut node_config, &config_dir)?;
            let mut api = crate::api::ApiClient::from_auth_checked(&auth)
                .await
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
            backend.publish(&node_config, &config_file)?;
            println!("Pruned {} stale SSH key entries.", stale_entries.len());
            Ok(())
        }
    }
}

pub fn cmd_node_ssh(command: crate::cli::NodeSshCommands, config_path: Option<&str>) -> Result<()> {
    use crate::cli::NodeSshCommands;

    init_cli_tracing();

    match command {
        NodeSshCommands::CertHostKey { command } => cmd_ssh_cert_host_key(command, config_path),
    }
}

fn cmd_ssh_cert_host_key(
    command: crate::cli::NodeSshCertHostKeyCommands,
    config_path: Option<&str>,
) -> Result<()> {
    use crate::cli::NodeSshCertHostKeyCommands;

    let config_dir = config::resolve_config_dir(config_path);
    let mut store =
        CertHostKeyStore::load(&config_dir).map_err(cert_host_key_store_error_to_node_error)?;

    match command {
        NodeSshCertHostKeyCommands::List => {
            let entries = store.entries();
            if entries.is_empty() {
                println!("No cert-mode SSH host keys pinned.");
            } else {
                println!("Cert-mode SSH host keys:");
                for entry in entries {
                    println!(
                        "  {}:{} {} source={} created={} updated={}",
                        entry.host,
                        entry.port,
                        entry.host_key_sha256,
                        cert_host_key_source_label(&entry.source),
                        entry.created_at,
                        entry.updated_at
                    );
                }
            }
            Ok(())
        }
        NodeSshCertHostKeyCommands::Pin {
            host,
            port,
            sha256_fingerprint,
        } => {
            let fingerprint = canonical_sha256_fingerprint(&sha256_fingerprint)
                .map_err(cert_host_key_store_error_to_node_error)?;
            store
                .set_explicit_pin(&host, port, &fingerprint)
                .map_err(cert_host_key_store_error_to_node_error)?;
            let entry = store.get(&host, port).ok_or_else(|| {
                Error::Config("Explicit SSH host-key pin was not persisted".to_string())
            })?;
            println!(
                "Pinned cert-mode SSH host key for {}:{}.",
                entry.host, entry.port
            );
            println!("Fingerprint: {}", entry.host_key_sha256);
            println!("Source:      explicit");
            println!("Applies immediately to running node agents.");
            Ok(())
        }
        NodeSshCertHostKeyCommands::Forget { host, port } => {
            match store
                .forget(&host, port)
                .map_err(cert_host_key_store_error_to_node_error)?
            {
                Some(entry) => {
                    println!(
                        "Forgot cert-mode SSH host key for {}:{}.",
                        entry.host, entry.port
                    );
                    println!("Previous fingerprint: {}", entry.host_key_sha256);
                    println!(
                        "Next cert-mode connection will trust-on-first-use and persist the observed host key."
                    );
                    println!("Applies immediately to running node agents.");
                }
                None => {
                    println!("No cert-mode SSH host key pin found for {host}:{port}.");
                }
            }
            Ok(())
        }
    }
}

fn cert_host_key_store_error_to_node_error(error: CertHostKeyStoreError) -> Error {
    match error {
        CertHostKeyStoreError::InvalidFingerprint => Error::Validation(
            "Invalid SSH host-key fingerprint; expected a SHA256 fingerprint such as SHA256:AbCd... without MD5 or key material"
                .to_string(),
        ),
        other => Error::Config(other.to_string()),
    }
}

fn cert_host_key_source_label(source: &CertHostKeySource) -> &'static str {
    match source {
        CertHostKeySource::Tofu => "tofu",
        CertHostKeySource::Explicit => "explicit",
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

/// Revalidate custody even if config bytes/mtime did not change. A failure
/// empties cached credentials and terminates the serving runtime.
pub(super) async fn credential_reload_loop(
    sender: std::sync::Arc<SharedCredentialsSender>,
    interval: Duration,
    backend: std::sync::Arc<SecretBackend>,
) -> Result<()> {
    loop {
        tokio::time::sleep(interval).await;
        reload_credentials(&backend, &sender)?;
    }
}

pub(super) fn reload_credentials(
    backend: &SecretBackend,
    sender: &SharedCredentialsSender,
) -> Result<()> {
    let loaded = (|| {
        let (config, reader) = backend.current_reader()?;
        let credentials = CredentialStore::from_config_with_backend(&config, &reader)?;
        reader.refresh()?;
        Ok(credentials)
    })();
    match loaded {
        Ok(credentials) => {
            sender.update(credentials);
            Ok(())
        }
        Err(error) => {
            sender.update(CredentialStore::default());
            Err(error)
        }
    }
}

/// Background task that refreshes OAuth tokens before they expire.
async fn oauth_refresh_loop(
    interval: Duration,
    backend: std::sync::Arc<SecretBackend>,
    mut shutdown: tokio::sync::oneshot::Receiver<()>,
) {
    loop {
        tokio::select! {
            biased;
            _ = &mut shutdown => return,
            () = tokio::time::sleep(interval) => {}
        }
        if oauth_refresh_once(&backend).await.is_err() {
            // Provider/transport bodies, URLs and key-service details never
            // reach diagnostics. Recovery remains durable and blocks replay.
            tracing::error!(
                reason = "oauth_refresh_or_recovery_failed",
                "Node OAuth refresh requires storage recovery or manual reauthorization"
            );
        }
    }
}

pub(super) async fn oauth_refresh_once(backend: &SecretBackend) -> Result<()> {
    use oauth::storage::{Plan, Rotation};
    oauth::storage::recover_pending(backend)?;
    let (config, _) = backend.current_reader()?;
    let services: Vec<_> = config
        .credentials
        .iter()
        .filter(|(_, credential)| credential.oauth_managed)
        .map(|(service, _)| service.clone())
        .collect();
    for service in services {
        // Each provider await has its own original generation and lease.
        let (config, reader) = backend.current_reader()?;
        let credential = config
            .credentials
            .get(&service)
            .ok_or_else(|| Error::Config("oauth_credential_changed".into()))?;
        let Some(expires_at) = &credential.oauth_token_expires_at else {
            continue;
        };
        let expires = chrono::DateTime::parse_from_rfc3339(expires_at)
            .map_err(|_| Error::Config("oauth_expiry_invalid".into()))?;
        if expires.with_timezone(&chrono::Utc) - chrono::Duration::minutes(5) >= chrono::Utc::now()
        {
            continue;
        }
        let required = |value: Option<Zeroizing<String>>| {
            value.ok_or_else(|| Error::Config("oauth_refresh_configuration_incomplete".into()))
        };
        use super::secret_fields::{CredentialPart, SecretField};
        let field = |part| SecretField::Credential {
            slug: service.clone(),
            part,
        };
        let refresh = required(reader.load_field(&config, &field(CredentialPart::OAuthRefresh))?)?;
        let client_id =
            required(reader.load_field(&config, &field(CredentialPart::OAuthClientId))?)?;
        let client_secret =
            reader.load_field(&config, &field(CredentialPart::OAuthClientSecret))?;
        let url = credential
            .oauth_token_url
            .as_deref()
            .ok_or_else(|| Error::Config("oauth_token_endpoint_missing".into()))?;
        let mut rotation = Rotation::open(&reader, &service)?;
        rotation.recover()?;
        rotation.dispatch()?;
        let token = oauth::refresh_token(
            url,
            &client_id,
            client_secret.as_ref().map(|value| value.as_str()),
            &refresh,
            credential
                .oauth_token_endpoint_auth_method
                .as_deref()
                .unwrap_or("client_secret_post"),
            credential.oauth_client_id_param_name.as_deref(),
            &credential.oauth_request_options,
        )
        .await?;
        rotation.retain_durable(Plan::Refresh, token).await?;
        rotation.recover()?;
        tracing::info!(
            reason = "oauth_refresh_published",
            "Node OAuth token durably published"
        );
    }
    Ok(())
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
        "ifttt-webhook" | "ifttt_webhook" => node_config.add_ifttt_credential_via(
            &pending.service_slug,
            secret,
            pending.target_url.as_deref(),
            backend,
        ),
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

fn resolve_oauth_scopes(
    config: &oauth::OAuthConfig,
    scopes: Option<&str>,
    additional_scopes: &[String],
) -> Result<String> {
    if !config.request_options.supports_oauth_scopes {
        if scopes.is_some_and(|value| !value.trim().is_empty()) || !additional_scopes.is_empty() {
            return Err(super::error::Error::Validation(
                "This provider does not accept OAuth scopes. Remove --scope and --scopes.".into(),
            ));
        }
        return Ok(String::new());
    }
    // Preserve the legacy --scopes string exactly when no additive scopes exist.
    if additional_scopes.is_empty() {
        return Ok(scopes
            .map(str::to_string)
            .unwrap_or_else(|| config.default_scopes.join(" ")));
    }
    if config.device_code_format == "openai" {
        return Err(super::error::Error::Validation(
            "This provider's device code endpoint does not accept additional OAuth scopes \
             (OpenAI-format device code providers ignore the `scope` parameter). \
             Remove --scope and try again."
                .into(),
        ));
    }
    let base_scopes = match scopes {
        Some(value) if !value.trim().is_empty() => {
            value.split_whitespace().map(String::from).collect()
        }
        _ => config.default_scopes.clone(),
    };
    Ok(merge_oauth_scopes(&base_scopes, additional_scopes).join(" "))
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
    let backend = SecretBackend::writer(&mut node_config, config_dir)?;

    let header_value = format!("Bearer {bearer_token}");
    node_config.add_header_credential_via(
        OPENCLAW_SERVICE_SLUG,
        "Authorization",
        &header_value,
        Some(gateway_url),
        &backend,
    )?;
    backend.publish(&node_config, config_file)?;
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
    let backend = SecretBackend::reader(&node_config, config_dir)?;
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
    let backend = SecretBackend::writer(&mut node_config, config_dir)?;
    node_config.remove_credential_via(OPENCLAW_SERVICE_SLUG, &backend)?;
    backend.publish(&node_config, config_file)?;
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
    let node_config = load_node_config_for_credentials(config_file, config_dir)?;

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

            let is_cloud_billing = auth_method == "aws_sigv4";
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
            // the user can paste the full access-key JSON.
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

            let mut node_config = load_node_config_for_credentials(config_file, config_dir)?;
            let backend = SecretBackend::writer(&mut node_config, config_dir)?;

            // Store credential using the injection method matching the auth method
            if auth_method == "ifttt_webhook" {
                node_config.add_ifttt_credential_via(
                    service,
                    &secret,
                    target_url.as_deref(),
                    &backend,
                )?;
            } else if auth_method == "path" {
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

            backend.publish(&node_config, config_file)?;
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
    let client_secret = client_secret.map(Zeroizing::new);
    let node_config = load_node_config_for_credentials(config_file, config_dir)?;
    let backend = SecretBackend::reader(&node_config, config_dir)?;
    let mut rotation = oauth::storage::Rotation::open(&backend, service)?;
    if rotation.recover_for_login()? {
        println!("Retained OAuth credential recovered; no provider request repeated.");
        return Ok(());
    }

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
            request_options: Default::default(),
        }
    };

    let final_scopes = resolve_oauth_scopes(&oauth_config, scopes.as_deref(), additional_scopes)?;

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
                Ok(s) => Some(Zeroizing::new(s)),
                Err(_) => return Err(Error::Config("oauth_client_secret_input_failed".into())),
            }
        }
    };

    let cid = Zeroizing::new(cid);
    let plan = oauth::storage::Plan::login(
        &oauth_config,
        &cid,
        csecret.as_ref().map(|value| value.as_str()),
        &final_scopes,
        target_url.as_deref(),
    );
    // Persist replay protection and independent recovery custody first.
    rotation.dispatch()?;
    let token_response = if oauth_config.device_code_url.is_some() {
        oauth::run_device_code_flow(
            &oauth_config,
            &cid,
            csecret.as_ref().map(|value| value.as_str()),
            &final_scopes,
        )
        .await?
    } else {
        oauth::run_authorization_code_flow(
            &oauth_config,
            &cid,
            csecret.as_ref().map(|value| value.as_str()),
            &final_scopes,
        )
        .await?
    };

    rotation.retain_durable(plan, token_response).await?;
    rotation.recover()?;
    println!("OAuth credential stored for service '{service}'.");
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    fn read_snapshot(
        config: &super::super::config::NodeConfig,
        path: &std::path::Path,
        keychain: Option<&super::super::keychain::KeychainBackend>,
        previous: Option<&super::super::legacy_secrets::LegacySnapshot>,
    ) -> super::super::error::Result<super::super::secret_snapshot::SecretSnapshot> {
        let directory = super::super::storage::Directory::open(path.parent().unwrap())?;
        super::super::secret_snapshot::SecretSnapshot::read_in(
            &directory, config, path, keychain, previous, None,
        )
    }

    use serde_json::json;

    use super::super::encryption::LocalEncryption;
    use super::super::error::Error;
    use super::*;

    use std::collections::BTreeMap;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::super::config::{CredentialConfig, PendingCryptoKeyConfig, SshKeyConfig};
    use super::super::credential_store::CredentialStore;
    use super::super::credentials::{pending_key_store::PendingCredentialKeyStore, ssh_keys};
    use super::super::keychain::KeychainBackend;
    use super::super::secret_fields::{self, CredentialPart, SecretField};
    use super::super::secret_transaction::Attempt;
    use super::super::storage::Directory;

    #[test]
    fn startup_refuses_migration_lease_before_config_or_secret_reads() {
        let _serial = crate::test_support::env_lock().lock().unwrap();
        let root = tempfile::tempdir().unwrap();
        let directory = Directory::open(root.path()).unwrap();
        let lease = directory.runtime_lease(true).unwrap();
        let before = directory.names().unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let error = runtime
            .block_on(cmd_start(root.path().to_str(), Some("error")))
            .unwrap_err()
            .to_string();
        assert!(error.contains("node_storage_runtime_busy"));
        assert_eq!(directory.names().unwrap(), before);
        assert!(!root.path().join(".keyfile").exists());
        assert!(!root.path().join("config.toml").exists());
        drop(lease);
        assert!(directory.runtime_lease(false).is_ok());
    }

    struct ModernFixture {
        directory: tempfile::TempDir,
        config: NodeConfig,
        store: KeychainBackend,
        expected: BTreeMap<SecretField, Option<Zeroizing<String>>>,
    }

    fn modern_fixture() -> ModernFixture {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.toml");
        let encryption = LocalEncryption::initialize(directory.path()).unwrap();
        let mut config =
            NodeConfig::new("wss://example.invalid".into(), "node".into(), "file".into());
        config.signing.shared_secret_encrypted = Some(String::new());
        let mut credential = CredentialConfig::new_header("Authorization".into(), None, None);
        credential.oauth_managed = true;
        config.credentials.insert("oauth".into(), credential);
        config.credentials.insert(
            "query".into(),
            CredentialConfig::new_query_param("key".into(), None, None),
        );
        config
            .credentials
            .insert("none".into(), CredentialConfig::new_no_auth(None));
        config.ssh_keys.push(SshKeyConfig {
            service_slug: "ssh".into(),
            principal: "alice".into(),
            private_key_pem_encrypted: None,
            passphrase_encrypted: None,
            target_host: "example.invalid".into(),
            target_port: 22,
            host_key_sha256: None,
            algorithms: None,
            created_at: "2026-01-01T00:00:00Z".into(),
        });
        config.pending_crypto_keys.insert(
            "pending".into(),
            PendingCryptoKeyConfig {
                version: nyxid_crypto::VERSION_V1.into(),
                service_slug: "oauth".into(),
                injection_method: "header".into(),
                field_name: "Authorization".into(),
                target_url: None,
                expires_at: "2026-10-20T00:00:00Z".into(),
                public_key: "synthetic-public".into(),
                private_key_encrypted: None,
            },
        );
        let mut expected = BTreeMap::new();
        for (field, _) in secret_fields::fields(&config).unwrap() {
            let value = match &field {
                SecretField::Credential {
                    part: CredentialPart::OAuthRefresh,
                    ..
                }
                | SecretField::SshPassphrase { .. } => None,
                SecretField::Credential {
                    part: CredentialPart::OAuthClientSecret,
                    ..
                } => Some(Zeroizing::new(String::new())),
                SecretField::PendingPrivate { .. } => Some(nyxid_crypto::encode_private_key_b64u(
                    &Zeroizing::new([49; 32]),
                )),
                _ => Some(Zeroizing::new(format!(
                    "synthetic:{}",
                    serde_json::to_string(&field).unwrap()
                ))),
            };
            let encoded = value
                .as_ref()
                .map(|value| encryption.encrypt(value).unwrap());
            field.replace(&mut config, encoded).unwrap();
            expected.insert(field, value);
        }
        config.save(&path).unwrap();
        let source = NodeConfig::load(&path).unwrap();
        let snapshot = read_snapshot(&source, &path, None, None).unwrap();
        let store = KeychainBackend::new_mock("node");
        let attempt = Attempt::begin_in(
            &super::super::storage::Directory::open(path.parent().unwrap()).unwrap(),
            &path,
            source.publication.snapshot(),
            "node",
        )
        .unwrap();
        let modern = snapshot
            .project_keychain(&source, &attempt, &store)
            .unwrap();
        modern.save(&path).unwrap();
        drop(attempt);
        ModernFixture {
            directory,
            config: NodeConfig::load(&path).unwrap(),
            store,
            expected,
        }
    }

    #[tokio::test]
    async fn oauth_refresh_publishes_keychain_tokens_and_does_not_repeat_provider_request() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(method("POST")).and(path("/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "access_token": "returned-access-SENTINEL", "refresh_token": "returned-refresh-SENTINEL", "expires_in": 3600
            }))).expect(1).mount(&server).await;
        let fixture = modern_fixture();
        let path = fixture.directory.path().join("config.toml");
        let mut config = fixture.config.clone();
        let writer = SecretBackend::writer_with_keychain(
            &mut config,
            fixture.directory.path(),
            fixture.store.clone(),
        )
        .unwrap();
        let credential = config.credentials.get_mut("oauth").unwrap();
        credential.oauth_refresh_token_encrypted = writer
            .store_field(
                SecretField::Credential {
                    slug: "oauth".into(),
                    part: CredentialPart::OAuthRefresh,
                },
                "original-refresh",
            )
            .unwrap();
        credential.oauth_token_url = Some(format!("{}/token", server.uri()));
        credential.oauth_token_expires_at = Some("2000-01-01T00:00:00Z".into());
        writer.publish(&config, &path).unwrap();
        let config = NodeConfig::load(&path).unwrap();
        let reader = SecretBackend::reader_with_keychain(
            &config,
            fixture.directory.path(),
            fixture.store.clone(),
        )
        .unwrap();
        oauth_refresh_once(&reader).await.unwrap();
        oauth_refresh_once(&reader).await.unwrap();
        let (config, reader) = reader.current_reader().unwrap();
        for (part, expected) in [
            (CredentialPart::OAuthAccess, "returned-access-SENTINEL"),
            (CredentialPart::OAuthRefresh, "returned-refresh-SENTINEL"),
        ] {
            assert!(
                reader
                    .require_field(
                        &config,
                        &SecretField::Credential {
                            slug: "oauth".into(),
                            part
                        }
                    )
                    .unwrap()
                    .as_str()
                    == expected
            );
        }
        server.verify().await;
    }

    #[tokio::test]
    async fn oauth_shutdown_waits_for_returned_token_durability_without_another_request() {
        use wiremock::matchers::{method, path as match_path};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        let fixture = modern_fixture();
        let path = fixture.directory.path().join("config.toml");
        let mut config = fixture.config.clone();
        let writer = SecretBackend::writer_with_keychain(
            &mut config,
            fixture.directory.path(),
            fixture.store.clone(),
        )
        .unwrap();
        let credential = config.credentials.get_mut("oauth").unwrap();
        credential.oauth_refresh_token_encrypted = writer
            .store_field(
                SecretField::Credential {
                    slug: "oauth".into(),
                    part: CredentialPart::OAuthRefresh,
                },
                "original-refresh",
            )
            .unwrap();
        credential.oauth_token_url = Some(format!("{}/token", server.uri()));
        credential.oauth_token_expires_at = Some("2000-01-01T00:00:00Z".into());
        writer.publish(&config, &path).unwrap();
        let before = std::fs::read(&path).unwrap();
        let config = NodeConfig::load(&path).unwrap();
        let reader = SecretBackend::reader_with_keychain(
            &config,
            fixture.directory.path(),
            fixture.store.clone(),
        )
        .unwrap();
        let failed_record = Arc::new(std::sync::Mutex::new(None));
        let ready = Arc::new(tokio::sync::Notify::new());
        let response_ready = Arc::clone(&ready);
        let response_record = Arc::clone(&failed_record);
        let directory = fixture.directory.path().to_path_buf();
        Mock::given(method("POST"))
            .and(match_path("/token"))
            .respond_with(move |_: &wiremock::Request| {
                let record = std::fs::read_dir(&directory)
                    .unwrap()
                    .map(|entry| entry.unwrap().path())
                    .find(|path| {
                        path.file_name().unwrap().to_str().unwrap().starts_with(".nyxid-storage-oauth-")
                            && path.extension().is_some_and(|extension| extension == "json")
                    })
                    .unwrap();
                std::fs::remove_file(&record).unwrap();
                std::fs::create_dir(&record).unwrap();
                *response_record.lock().unwrap() = Some(record);
                response_ready.notify_one();
                ResponseTemplate::new(200).set_body_json(json!({
                    "access_token": "returned-access-SENTINEL", "refresh_token": "returned-refresh-SENTINEL", "expires_in": 3600
                }))
            })
            .expect(1)
            .mount(&server)
            .await;
        let (shutdown, stopped) = tokio::sync::oneshot::channel();
        let mut refresh = tokio::spawn(oauth_refresh_loop(
            Duration::from_millis(1),
            Arc::new(reader),
            stopped,
        ));
        tokio::time::timeout(Duration::from_secs(5), ready.notified())
            .await
            .unwrap();
        shutdown.send(()).unwrap();
        assert!(
            tokio::time::timeout(Duration::from_millis(30), &mut refresh)
                .await
                .is_err()
        );
        assert_eq!(std::fs::read(&path).unwrap(), before);
        let record = failed_record.lock().unwrap().take().unwrap();
        std::fs::remove_dir(record).unwrap();
        tokio::time::timeout(Duration::from_secs(5), refresh)
            .await
            .unwrap()
            .unwrap();
        let config = NodeConfig::load(&path).unwrap();
        let reader = SecretBackend::reader_with_keychain(
            &config,
            fixture.directory.path(),
            fixture.store.clone(),
        )
        .unwrap();
        assert!(
            reader
                .require_field(
                    &config,
                    &SecretField::Credential {
                        slug: "oauth".into(),
                        part: CredentialPart::OAuthRefresh
                    }
                )
                .unwrap()
                .as_str()
                == "returned-refresh-SENTINEL"
        );
        server.verify().await;
    }

    #[tokio::test]
    async fn oauth_login_command_recovers_returned_tokens_before_any_catalog_or_provider_call() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.toml");
        let file = LocalEncryption::initialize(directory.path()).unwrap();
        let mut config =
            NodeConfig::new("wss://example.invalid".into(), "node".into(), "file".into());
        config.set_auth_token("synthetic-auth", &file).unwrap();
        config.save(&path).unwrap();
        let config = NodeConfig::load(&path).unwrap();
        let reader = SecretBackend::reader(&config, directory.path()).unwrap();
        let mut rotation = oauth::storage::Rotation::open(&reader, "new-oauth").unwrap();
        rotation.dispatch().unwrap();
        let options = oauth::oauth_config_from_catalog_value(
            &json!({"token_url": "https://provider.invalid/token"}),
        )
        .unwrap();
        let plan = oauth::storage::Plan::login(
            &options,
            "synthetic-client",
            Some("client-secret-SENTINEL"),
            "read",
            Some("https://provider.invalid"),
        );
        let token = serde_json::from_value(json!({"access_token": "returned-access-SENTINEL", "refresh_token": "returned-refresh-SENTINEL", "expires_in": 3600})).unwrap();
        rotation.retain(plan, token).unwrap();
        drop(rotation);
        // Every input needed to start a new flow is deliberately absent. A
        // command that recontacts catalog/provider would fail these inputs.
        cmd_credentials_add_oauth(
            &path,
            directory.path(),
            "new-oauth",
            true,
            None,
            None,
            None,
            None,
            None,
            None,
            &[],
            None,
            None,
            None,
        )
        .await
        .unwrap();
        let config = NodeConfig::load(&path).unwrap();
        let reader = SecretBackend::reader(&config, directory.path()).unwrap();
        assert!(
            reader
                .require_field(
                    &config,
                    &SecretField::Credential {
                        slug: "new-oauth".into(),
                        part: CredentialPart::OAuthClientSecret
                    }
                )
                .unwrap()
                .as_str()
                == "client-secret-SENTINEL"
        );
        assert!(
            reader
                .require_field(
                    &config,
                    &SecretField::Credential {
                        slug: "new-oauth".into(),
                        part: CredentialPart::Header
                    }
                )
                .unwrap()
                .as_str()
                == "Bearer returned-access-SENTINEL"
        );
    }

    #[tokio::test]
    async fn oauth_login_command_publishes_a_real_device_flow_and_preserves_optional_absence() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(method("POST")).and(path("/device"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "device_code": "synthetic-device", "user_code": "TEST", "expires_in": 30, "interval": 1,
                "verification_uri": "https://provider.invalid/device"
            }))).expect(1).mount(&server).await;
        Mock::given(method("POST"))
            .and(path("/token"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "access_token": "returned-access-SENTINEL", "expires_in": 3600
            })))
            .expect(1)
            .mount(&server)
            .await;
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.toml");
        let file = LocalEncryption::initialize(directory.path()).unwrap();
        let mut config =
            NodeConfig::new("wss://example.invalid".into(), "node".into(), "file".into());
        config.set_auth_token("synthetic-auth", &file).unwrap();
        config.save(&path).unwrap();
        cmd_credentials_add_oauth(
            &path,
            directory.path(),
            "new-oauth",
            false,
            Some("synthetic-client".into()),
            Some("client-secret-SENTINEL".into()),
            None,
            Some(format!("{}/token", server.uri())),
            Some(format!("{}/device", server.uri())),
            None,
            &[],
            None,
            None,
            None,
        )
        .await
        .unwrap();
        let config = NodeConfig::load(&path).unwrap();
        let reader = SecretBackend::reader(&config, directory.path()).unwrap();
        assert!(config.credentials["new-oauth"].oauth_managed);
        assert!(
            reader
                .load_field(
                    &config,
                    &SecretField::Credential {
                        slug: "new-oauth".into(),
                        part: CredentialPart::OAuthRefresh
                    }
                )
                .unwrap()
                .is_none()
        );
        assert!(
            reader
                .require_field(
                    &config,
                    &SecretField::Credential {
                        slug: "new-oauth".into(),
                        part: CredentialPart::Header
                    }
                )
                .unwrap()
                .as_str()
                == "Bearer returned-access-SENTINEL"
        );
        server.verify().await;
    }

    #[test]
    fn typed_writer_replaces_and_retires_only_the_removed_objects() {
        use super::super::secret_transaction::{ObjectRef, ObjectStore};
        let fixture = modern_fixture();
        let path = fixture.directory.path().join("config.toml");
        let mut config = fixture.config.clone();
        let field = SecretField::Credential {
            slug: "oauth".into(),
            part: CredentialPart::Header,
        };
        let original = ObjectRef::parse(field.encoded(&config).unwrap().unwrap())
            .unwrap()
            .unwrap();
        let writer = SecretBackend::writer_with_keychain(
            &mut config,
            fixture.directory.path(),
            fixture.store.clone(),
        )
        .unwrap();
        // A logical name cannot masquerade as a typed field, including slugs
        // that happen to resemble an OAuth or SSH suffix.
        assert!(
            writer
                .store_credential_value("oauth:oauth_refresh", "never-staged")
                .is_err()
        );
        config
            .credentials
            .get_mut("oauth")
            .unwrap()
            .header_value_encrypted = writer
            .store_field(field.clone(), "synthetic-replacement")
            .unwrap();
        assert_eq!(
            writer.require_field(&config, &field).unwrap().as_str(),
            "synthetic-replacement"
        );
        assert!(
            ObjectStore::get(&fixture.store, original)
                .unwrap()
                .is_some()
        );
        writer.publish(&config, &path).unwrap();
        assert!(
            ObjectStore::get(&fixture.store, original)
                .unwrap()
                .is_none()
        );
        assert!(writer.publish(&config, &path).is_err());
        let mut loaded = NodeConfig::load(&path).unwrap();
        let reader = SecretBackend::reader_with_keychain(
            &loaded,
            fixture.directory.path(),
            fixture.store.clone(),
        )
        .unwrap();
        for (location, expected) in &fixture.expected {
            if location != &field {
                assert!(reader.load_field(&loaded, location).unwrap() == *expected);
            }
        }
        assert_eq!(
            reader.require_field(&loaded, &field).unwrap().as_str(),
            "synthetic-replacement"
        );
        let replacement = ObjectRef::parse(field.encoded(&loaded).unwrap().unwrap())
            .unwrap()
            .unwrap();
        let remover = SecretBackend::writer_with_keychain(
            &mut loaded,
            fixture.directory.path(),
            fixture.store.clone(),
        )
        .unwrap();
        loaded.remove_credential_via("oauth", &remover).unwrap();
        assert!(
            ObjectStore::get(&fixture.store, replacement)
                .unwrap()
                .is_some()
        );
        remover.publish(&loaded, &path).unwrap();
        assert!(
            ObjectStore::get(&fixture.store, replacement)
                .unwrap()
                .is_none()
        );
        let loaded = NodeConfig::load(&path).unwrap();
        let reader =
            SecretBackend::reader_with_keychain(&loaded, fixture.directory.path(), fixture.store)
                .unwrap();
        for (location, expected) in &fixture.expected {
            if !matches!(location, SecretField::Credential { slug, .. } if slug == "oauth") {
                assert!(reader.load_field(&loaded, location).unwrap() == *expected);
            }
        }
    }

    #[test]
    fn stale_writer_never_retires_winner_or_loser_before_durable_recovery() {
        use super::super::secret_transaction::{ObjectRef, ObjectStore};
        let fixture = modern_fixture();
        let path = fixture.directory.path().join("config.toml");
        let mut loser = fixture.config.clone();
        let writer = SecretBackend::writer_with_keychain(
            &mut loser,
            fixture.directory.path(),
            fixture.store.clone(),
        )
        .unwrap();
        writer
            .store_auth_token(&mut loser, "synthetic-losing-auth")
            .unwrap();
        let staged = ObjectRef::parse(&loser.node.auth_token_encrypted)
            .unwrap()
            .unwrap();
        let mut winner = fixture.config.clone();
        winner.server.url = "wss://metadata-winner.invalid".into();
        winner.save(&path).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert!(writer.publish(&loser, &path).is_err());
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        assert!(ObjectStore::get(&fixture.store, staged).unwrap().is_some());
        drop(writer);
        let mut current = NodeConfig::load(&path).unwrap();
        let next = SecretBackend::writer_with_keychain(
            &mut current,
            fixture.directory.path(),
            fixture.store.clone(),
        )
        .unwrap();
        next.publish(&current, &path).unwrap();
        assert!(ObjectStore::get(&fixture.store, staged).unwrap().is_none());
        let current = NodeConfig::load(&path).unwrap();
        let reader =
            SecretBackend::reader_with_keychain(&current, fixture.directory.path(), fixture.store)
                .unwrap();
        for (field, expected) in &fixture.expected {
            assert!(reader.load_field(&current, field).unwrap() == *expected);
        }
    }

    #[test]
    fn ordinary_writer_refuses_legacy_keychain_before_conversion_or_key_service_access() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.toml");
        NodeConfig::new(
            "wss://example.invalid".into(),
            "node".into(),
            "keychain".into(),
        )
        .save(&path)
        .unwrap();
        let mut config = NodeConfig::load(&path).unwrap();
        let before = std::fs::read(&path).unwrap();
        let names = Directory::open(directory.path()).unwrap().names().unwrap();
        let store = KeychainBackend::new_failing_mock("node", "key service must not be touched");
        let error = SecretBackend::writer_with_keychain(&mut config, directory.path(), store)
            .err()
            .unwrap();
        assert!(error.to_string().contains("stopped-daemon migration"));
        assert!(
            error
                .to_string()
                .contains("nyxid node migrate --to keychain")
        );
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert_eq!(
            Directory::open(directory.path()).unwrap().names().unwrap(),
            names
        );
    }

    #[test]
    fn runtime_reload_tracks_generations_and_invalidates_deleted_keychain_objects() {
        use super::super::secret_transaction::{ObjectRef, ObjectStore};
        let fixture = modern_fixture();
        let path = fixture.directory.path().join("config.toml");
        let runtime = SecretBackend::reader_with_keychain(
            &fixture.config,
            fixture.directory.path(),
            fixture.store.clone(),
        )
        .unwrap();
        let initial = CredentialStore::from_config_with_backend(&fixture.config, &runtime).unwrap();
        let (sender, credentials) = SharedCredentials::new(initial);
        let (mut config, writer) = runtime.current_writer().unwrap();
        config
            .add_header_credential_via("new", "X-Key", "synthetic-new", None, &writer)
            .unwrap();
        writer.publish(&config, &path).unwrap();
        reload_credentials(&runtime, &sender).unwrap();
        assert_eq!(
            credentials
                .snapshot()
                .get("new")
                .unwrap()
                .header()
                .unwrap()
                .1,
            "synthetic-new"
        );
        let config = NodeConfig::load(&path).unwrap();
        let before = std::fs::read(&path).unwrap();
        let modified = std::fs::metadata(&path).unwrap().modified().unwrap();
        let reference = ObjectRef::parse(
            config.credentials["new"]
                .header_value_encrypted
                .as_deref()
                .unwrap(),
        )
        .unwrap()
        .unwrap();
        ObjectStore::delete(&fixture.store, reference).unwrap();
        assert!(reload_credentials(&runtime, &sender).is_err());
        assert_eq!(credentials.snapshot().count(), 0);
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert_eq!(
            std::fs::metadata(&path).unwrap().modified().unwrap(),
            modified
        );
    }

    #[test]
    fn runtime_file_custody_or_identity_changes_clear_cached_credentials() {
        for change in [
            "missing-key",
            "replacement-key",
            "backend",
            "owner",
            "server",
            "auth",
            "signing",
        ] {
            let directory = tempfile::tempdir().unwrap();
            let path = directory.path().join("config.toml");
            let file =
                super::super::encryption::LocalEncryption::initialize(directory.path()).unwrap();
            let mut config =
                NodeConfig::new("wss://example.invalid".into(), "node".into(), "file".into());
            config.set_auth_token("synthetic-auth", &file).unwrap();
            config
                .add_header_credential("service", "X-Key", "synthetic-key", &file)
                .unwrap();
            config.save(&path).unwrap();
            let config = NodeConfig::load(&path).unwrap();
            let runtime = SecretBackend::reader(&config, directory.path()).unwrap();
            let initial = CredentialStore::from_config_with_backend(&config, &runtime).unwrap();
            let (sender, credentials) = SharedCredentials::new(initial);
            let mut changed = config.clone();
            match change {
                "missing-key" => std::fs::remove_file(directory.path().join(".keyfile")).unwrap(),
                "replacement-key" => {
                    std::fs::write(directory.path().join(".keyfile"), [82; 32]).unwrap()
                }
                "backend" => {
                    changed.storage_backend = "keychain".into();
                    changed.node.auth_token_encrypted.clear();
                    changed.credentials.clear();
                    changed.save(&path).unwrap();
                }
                "owner" => {
                    changed.node.id = "other".into();
                    changed.save(&path).unwrap();
                }
                "server" => {
                    changed.server.url = "wss://other.invalid".into();
                    changed.save(&path).unwrap();
                }
                "auth" => {
                    changed.set_auth_token("synthetic-rotated", &file).unwrap();
                    changed.save(&path).unwrap();
                }
                "signing" => {
                    changed
                        .set_signing_secret("synthetic-signing", &file)
                        .unwrap();
                    changed.save(&path).unwrap();
                }
                _ => unreachable!(),
            }
            assert!(reload_credentials(&runtime, &sender).is_err(), "{change}");
            assert_eq!(credentials.snapshot().count(), 0, "{change}");
            assert!(runtime.current_writer().is_err(), "{change}");
            if change == "missing-key" {
                assert!(!directory.path().join(".keyfile").exists());
            }
        }
    }

    #[tokio::test]
    async fn custody_monitor_terminates_and_clears_cached_credentials_without_config_change() {
        let fixture = modern_fixture();
        let runtime = SecretBackend::reader_with_keychain(
            &fixture.config,
            fixture.directory.path(),
            fixture.store.clone(),
        )
        .unwrap();
        let initial = CredentialStore::from_config_with_backend(&fixture.config, &runtime).unwrap();
        let (sender, credentials) = SharedCredentials::new(initial);
        let reference = super::super::secret_transaction::ObjectRef::parse(
            &fixture.config.node.auth_token_encrypted,
        )
        .unwrap()
        .unwrap();
        super::super::secret_transaction::ObjectStore::delete(&fixture.store, reference).unwrap();
        let outcome = tokio::time::timeout(
            Duration::from_secs(1),
            credential_reload_loop(
                std::sync::Arc::new(sender),
                Duration::from_millis(1),
                std::sync::Arc::new(runtime),
            ),
        )
        .await
        .unwrap();
        assert!(outcome.is_err());
        assert_eq!(credentials.snapshot().count(), 0);
    }

    #[test]
    fn ssh_and_pending_writers_keep_typed_locations_until_publication() {
        use super::super::credentials::crypto::PendingCredentialCryptoMetadata;
        use super::super::secret_transaction::{ObjectRef, ObjectStore};
        let fixture = modern_fixture();
        let path = fixture.directory.path().join("config.toml");
        let mut config = fixture.config.clone();
        let writer = SecretBackend::writer_with_keychain(
            &mut config,
            fixture.directory.path(),
            fixture.store.clone(),
        )
        .unwrap();
        ssh_keys::add_entry(
            &mut config,
            &writer,
            NewSshKeyEntry {
                service_slug: "other-ssh".into(),
                principal: "bob".into(),
                private_key_pem: Zeroizing::new("synthetic-private-key".into()),
                passphrase: Some(Zeroizing::new("synthetic-passphrase".into())),
                target_host: "example.invalid".into(),
                target_port: 2222,
                host_key_sha256: None,
                algorithms: None,
            },
        )
        .unwrap();
        let loaded = ssh_keys::load_entry(
            &config,
            ssh_keys::config_find(&config, "other-ssh", "bob").unwrap(),
            &writer,
        )
        .unwrap();
        assert_eq!(loaded.private_key_pem.as_str(), "synthetic-private-key");
        assert_eq!(loaded.passphrase.unwrap().as_str(), "synthetic-passphrase");
        let metadata = PendingCredentialCryptoMetadata {
            pending_id: "new-pending".into(),
            node_id: "node".into(),
            service_slug: "oauth".into(),
            injection_method: "header".into(),
            field_name: "Authorization".into(),
            target_url: None,
            expires_at: "2099-01-01T00:00:00Z".into(),
            version: nyxid_crypto::VERSION_V1.into(),
        };
        let mut pending = PendingCredentialKeyStore::new(&mut config, &writer);
        let first = pending.create_or_load(&metadata).unwrap();
        let private = pending.load_private_key("new-pending").unwrap();
        assert_eq!(pending.create_or_load(&metadata).unwrap(), first);
        writer.publish(&config, &path).unwrap();
        let mut config = NodeConfig::load(&path).unwrap();
        let private_ref = ObjectRef::parse(
            config.pending_crypto_keys["new-pending"]
                .private_key_encrypted
                .as_ref()
                .unwrap(),
        )
        .unwrap()
        .unwrap();
        let writer = SecretBackend::writer_with_keychain(
            &mut config,
            fixture.directory.path(),
            fixture.store.clone(),
        )
        .unwrap();
        let mut pending = PendingCredentialKeyStore::new(&mut config, &writer);
        assert!(*pending.load_private_key("new-pending").unwrap() == *private);
        assert!(pending.delete("new-pending").unwrap());
        ssh_keys::remove_entry(&mut config, &writer, "other-ssh", "bob").unwrap();
        assert!(
            ObjectStore::get(&fixture.store, private_ref)
                .unwrap()
                .is_some()
        );
        writer.publish(&config, &path).unwrap();
        assert!(
            ObjectStore::get(&fixture.store, private_ref)
                .unwrap()
                .is_none()
        );
        let config = NodeConfig::load(&path).unwrap();
        let reader =
            SecretBackend::reader_with_keychain(&config, fixture.directory.path(), fixture.store)
                .unwrap();
        for (field, expected) in &fixture.expected {
            assert!(reader.load_field(&config, field).unwrap() == *expected);
        }
    }

    #[tokio::test]
    async fn file_command_writers_rekey_add_replace_remove_and_refuse_missing_custody() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.toml");
        let custom = directory.path().to_str();
        let file = LocalEncryption::initialize(directory.path()).unwrap();
        let mut config =
            NodeConfig::new("wss://example.invalid".into(), "node".into(), "file".into());
        config.set_auth_token("synthetic-old-auth", &file).unwrap();
        config.save(&path).unwrap();
        cmd_rekey("synthetic-new-auth", "synthetic-new-signing", custom).unwrap();
        for query in [false, true] {
            cmd_credentials(
                crate::cli::NodeCredentialCommands::Add {
                    service: "new-service".into(),
                    url: Some("https://example.invalid".into()),
                    header: if query {
                        None
                    } else {
                        Some("Authorization".into())
                    },
                    query_param: if query { Some("api_key".into()) } else { None },
                    secret_format: CredentialSecretFormat::Raw,
                    value: Some("synthetic-command-value".into()),
                },
                custom,
            )
            .await
            .unwrap();
            let loaded = NodeConfig::load(&path).unwrap();
            let reader = SecretBackend::reader(&loaded, directory.path()).unwrap();
            assert_eq!(
                reader.load_auth_token(&loaded).unwrap(),
                "synthetic-new-auth"
            );
            assert_eq!(
                reader.load_signing_secret(&loaded).unwrap().as_deref(),
                Some("synthetic-new-signing")
            );
            let field = SecretField::Credential {
                slug: "new-service".into(),
                part: if query {
                    CredentialPart::Parameter
                } else {
                    CredentialPart::Header
                },
            };
            assert_eq!(
                reader.require_field(&loaded, &field).unwrap().as_str(),
                "synthetic-command-value"
            );
        }
        cmd_credentials(
            crate::cli::NodeCredentialCommands::Remove {
                service: "new-service".into(),
            },
            custom,
        )
        .await
        .unwrap();
        assert!(NodeConfig::load(&path).unwrap().credentials.is_empty());
        let before = std::fs::read(&path).unwrap();
        std::fs::remove_file(directory.path().join(".keyfile")).unwrap();
        assert!(
            cmd_rekey(
                "synthetic-must-not-publish",
                "synthetic-must-not-publish",
                custom
            )
            .is_err()
        );
        assert!(!directory.path().join(".keyfile").exists());
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }

    #[test]
    fn typed_reader_reopens_every_modern_slot_without_legacy_discovery_or_mutation() {
        let fixture = modern_fixture();
        // Neither corrupt legacy custody nor stale optional values may be read.
        fixture.store.set("vault", "invalid legacy vault").unwrap();
        fixture
            .store
            .set("cred/oauth:oauth_refresh", "stale-refresh")
            .unwrap();
        let names = Directory::open(fixture.directory.path())
            .unwrap()
            .names()
            .unwrap();
        let bytes = std::fs::read(fixture.directory.path().join("config.toml")).unwrap();
        let legacy_reads = Arc::new(AtomicUsize::new(0));
        let probe = legacy_reads.clone();
        let store = fixture.store.clone().with_read_probe(move |key| {
            if !key.starts_with("object/v1/") {
                probe.fetch_add(1, Ordering::SeqCst);
            }
        });
        let reader =
            SecretBackend::reader_with_keychain(&fixture.config, fixture.directory.path(), store)
                .unwrap();
        for (field, expected) in &fixture.expected {
            assert!(reader.load_field(&fixture.config, field).unwrap() == *expected);
        }
        let credentials =
            CredentialStore::from_config_with_backend(&fixture.config, &reader).unwrap();
        assert_eq!(credentials.count(), 3);
        assert!(
            credentials.get("oauth").unwrap().header().unwrap().1
                == fixture.expected[&SecretField::Credential {
                    slug: "oauth".into(),
                    part: CredentialPart::Header
                }]
                    .as_ref()
                    .unwrap()
                    .as_str()
        );
        let ssh = ssh_keys::load_entries(&fixture.config, &reader).unwrap();
        assert!(
            ssh[0].private_key_pem.as_str()
                == fixture.expected[&SecretField::SshPrivate {
                    service: "ssh".into(),
                    principal: "alice".into()
                }]
                    .as_ref()
                    .unwrap()
                    .as_str()
        );
        assert!(ssh[0].passphrase.is_none());
        let mut config = fixture.config.clone();
        let pending = PendingCredentialKeyStore::new(&mut config, &reader)
            .load_private_key("pending")
            .unwrap();
        assert!(*pending == [49; 32]);
        assert_eq!(legacy_reads.load(Ordering::SeqCst), 0);
        assert!(std::fs::read(fixture.directory.path().join("config.toml")).unwrap() == bytes);
        assert_eq!(
            Directory::open(fixture.directory.path())
                .unwrap()
                .names()
                .unwrap(),
            names
        );
        assert!(
            reader
                .store_credential_value("oauth", "never-staged")
                .is_err()
        );
        assert!(reader.delete_credential("oauth").is_err());
        assert!(SecretBackend::from_config(&fixture.config, fixture.directory.path()).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn all_keychain_graph_and_object_health_reads_hold_the_publication_lock() {
        use std::os::fd::AsRawFd;
        let fixture = modern_fixture();
        let lock = fixture.directory.path().join(".storage-writer.lock");
        let calls = Arc::new(AtomicUsize::new(0));
        let probe = calls.clone();
        let store = fixture.store.clone().with_read_probe(move |_| {
            let file = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&lock)
                .unwrap();
            assert_eq!(
                unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) },
                -1
            );
            assert_eq!(
                std::io::Error::last_os_error().raw_os_error(),
                Some(libc::EWOULDBLOCK)
            );
            probe.fetch_add(1, Ordering::SeqCst);
        });
        let reader =
            SecretBackend::reader_with_keychain(&fixture.config, fixture.directory.path(), store)
                .unwrap();
        reader.refresh().unwrap();
        reader.load_auth_token(&fixture.config).unwrap();
        assert!(calls.load(Ordering::SeqCst) > 0);
    }

    #[test]
    fn reader_rejects_forged_projection_before_keychain_effects() {
        let fixture = modern_fixture();
        let calls = Arc::new(AtomicUsize::new(0));
        let probe = calls.clone();
        let store = fixture.store.clone().with_read_probe(move |_| {
            probe.fetch_add(1, Ordering::SeqCst);
        });
        let reader =
            SecretBackend::reader_with_keychain(&fixture.config, fixture.directory.path(), store)
                .unwrap();
        calls.store(0, Ordering::SeqCst);
        let mut forged = fixture.config.clone();
        forged
            .credentials
            .get_mut("query")
            .unwrap()
            .oauth_client_secret_encrypted = Some(String::new());
        assert!(reader.load_auth_token(&forged).is_err());
        assert_eq!(calls.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn legacy_snapshot_reader_never_consolidates_or_promotes_stale_custody() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let mut config = NodeConfig::new(
            "wss://example.invalid".into(),
            "node".into(),
            "keychain".into(),
        );
        config.credentials.insert(
            "service".into(),
            CredentialConfig::new_header("Authorization".into(), None, None),
        );
        config.save(&path).unwrap();
        let config = NodeConfig::load(&path).unwrap();
        let store = KeychainBackend::new_mock("node");
        let vault = r#"{"auth_token":"synthetic-vault-auth","credentials":{"service":"synthetic-vault-credential"}}"#;
        store.set("vault", vault).unwrap();
        store.set("auth_token", "synthetic-stale-auth").unwrap();
        store
            .set("cred/service", "synthetic-stale-credential")
            .unwrap();
        let bytes = std::fs::read(&path).unwrap();
        let names = Directory::open(dir.path()).unwrap().names().unwrap();
        let reader =
            SecretBackend::reader_with_keychain(&config, dir.path(), store.clone()).unwrap();
        assert!(reader.load_auth_token(&config).unwrap() == "synthetic-vault-auth");
        reader.refresh().unwrap();
        assert!(store.get("vault").unwrap() == vault);
        assert!(store.get("auth_token").unwrap() == "synthetic-stale-auth");
        assert!(std::fs::read(&path).unwrap() == bytes);
        assert_eq!(Directory::open(dir.path()).unwrap().names().unwrap(), names);
        let initial = CredentialStore::from_config_with_backend(&config, &reader).unwrap();
        let (sender, credentials) = SharedCredentials::new(initial);
        reload_credentials(&reader, &sender).unwrap();
        assert_eq!(credentials.snapshot().count(), 1);
        store.delete("vault").unwrap();
        assert!(reader.load_auth_token(&config).is_err());
        assert!(reader.refresh().is_err());
        assert!(store.get_optional("vault").unwrap().is_none());
        assert!(store.get("auth_token").unwrap() == "synthetic-stale-auth");
        assert!(reload_credentials(&reader, &sender).is_err());
        assert_eq!(credentials.snapshot().count(), 0);
    }

    #[test]
    fn file_reader_checks_cached_key_health_and_full_projection_without_generation() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        let enc = LocalEncryption::initialize(dir.path()).unwrap();
        let mut config =
            NodeConfig::new("wss://example.invalid".into(), "node".into(), "file".into());
        config.set_auth_token("synthetic-auth", &enc).unwrap();
        config
            .add_header_credential("service", "Authorization", "synthetic-header", &enc)
            .unwrap();
        config.save(&path).unwrap();
        let config = NodeConfig::load(&path).unwrap();
        let reader = SecretBackend::reader_with_keychain(
            &config,
            dir.path(),
            KeychainBackend::new_failing_mock("node", "must not touch keychain"),
        )
        .unwrap();
        assert!(reader.load_signing_secret(&config).unwrap().is_none());
        let mut forged = config.clone();
        forged.node.auth_token_encrypted = config.credentials["service"]
            .header_value_encrypted
            .clone()
            .unwrap();
        assert!(reader.load_auth_token(&forged).is_err());
        std::fs::remove_file(dir.path().join(".keyfile")).unwrap();
        assert!(reader.load_auth_token(&config).is_err());
        assert!(reader.refresh().is_err());
        assert!(!dir.path().join(".keyfile").exists());
        LocalEncryption::initialize(dir.path()).unwrap();
        assert!(reader.refresh().is_err());
    }

    #[test]
    fn reader_keeps_pinned_directory_after_path_retarget_and_never_uses_a_cached_object() {
        let fixture = modern_fixture();
        let reader = SecretBackend::reader_with_keychain(
            &fixture.config,
            fixture.directory.path(),
            fixture.store.clone(),
        )
        .unwrap();
        let parent = tempfile::tempdir().unwrap();
        let moved = parent.path().join("moved");
        std::fs::rename(fixture.directory.path(), &moved).unwrap();
        Directory::create_private(fixture.directory.path()).unwrap();
        assert!(reader.load_auth_token(&fixture.config).is_ok());
        let encoded = fixture
            .config
            .node
            .auth_token_encrypted
            .strip_prefix("nyxid-key:v1:")
            .unwrap();
        let (attempt, object) = encoded.split_once(':').unwrap();
        fixture
            .store
            .delete(&format!("object/v1/{attempt}/{object}"))
            .unwrap();
        assert!(reader.load_auth_token(&fixture.config).is_err());
        assert!(reader.refresh().is_err());
        assert_eq!(
            std::fs::read_dir(fixture.directory.path()).unwrap().count(),
            0
        );
    }

    const KEY_A: &str = "SHA256:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
    const KEY_B: &str = "SHA256://////////////////////////////////////////8";

    #[test]
    fn status_command_uses_strict_file_and_immutable_readers_without_secret_output() {
        use super::super::keychain::KeychainBackend;
        use super::super::registration::RegistrationCustody;
        use super::super::secret_fields::SecretField;
        for backend in ["file", "keychain"] {
            let dir = tempfile::tempdir().unwrap();
            let mut config = NodeConfig::new(
                "wss://example.invalid".into(),
                "node".into(),
                backend.into(),
            );
            let store = KeychainBackend::new_mock("node");
            let operation = RegistrationCustody::acquire(dir.path(), backend)
                .unwrap()
                .into_operation(&config, store.clone())
                .unwrap();
            operation.prepare(&mut config).unwrap();
            config.node.auth_token_encrypted = operation
                .store(SecretField::AuthToken, "synthetic-status-auth")
                .unwrap();
            operation
                .publish(&config, &dir.path().join("config.toml"))
                .unwrap();
            drop(operation);
            let before = std::fs::read(dir.path().join("config.toml")).unwrap();
            let mut output = Vec::new();
            status_to_writer(dir.path(), Some(store), &mut output).unwrap();
            let output = String::from_utf8(output).unwrap();
            assert!(output.contains("Credentials: 0 configured"));
            assert!(!output.contains("synthetic-status-auth"));
            assert!(!output.contains("nyxid-key:"));
            assert!(std::fs::read(dir.path().join("config.toml")).unwrap() == before);
            if backend == "file" {
                std::fs::remove_file(dir.path().join(".keyfile")).unwrap();
                assert!(cmd_status(dir.path().to_str()).is_err());
                assert!(!dir.path().join(".keyfile").exists());
            }
        }
    }

    #[tokio::test]
    async fn registration_command_existing_config_refuses_before_network_or_key_effects() {
        let dir = tempfile::tempdir().unwrap();
        let directory = super::super::storage::Directory::open(dir.path()).unwrap();
        directory
            .publish(
                std::ffi::OsStr::new("config.toml"),
                b"existing-config-without-key",
                false,
            )
            .unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("ws://{}", listener.local_addr().unwrap());
        let result = cmd_register(
            "synthetic-unused-registration",
            Some(&url),
            dir.path().to_str(),
            false,
        )
        .await;
        assert!(matches!(result, Err(Error::Config(_))));
        assert_eq!(
            directory.names().unwrap(),
            [std::ffi::OsString::from("config.toml")]
        );
        assert_eq!(
            std::fs::read(dir.path().join("config.toml")).unwrap(),
            b"existing-config-without-key"
        );
        assert!(
            tokio::time::timeout(Duration::from_millis(40), listener.accept())
                .await
                .is_err()
        );
    }

    #[tokio::test]
    async fn registration_command_retains_custody_across_await_and_path_retarget() {
        use futures::{SinkExt, StreamExt};
        use tokio_tungstenite::tungstenite::Message;
        for retarget in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let profile = dir.path().join("profile");
            let moved = dir.path().join("moved");
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let url = format!("ws://{}", listener.local_addr().unwrap());
            let (seen_tx, seen_rx) = tokio::sync::oneshot::channel();
            let (release_tx, release_rx) = tokio::sync::oneshot::channel();
            let server = tokio::spawn(async move {
                let (stream, _) = listener.accept().await.unwrap();
                let mut socket = tokio_tungstenite::accept_async(stream).await.unwrap();
                let message = socket.next().await.unwrap().unwrap();
                let parsed: serde_json::Value =
                    serde_json::from_str(message.to_text().unwrap()).unwrap();
                assert_eq!(parsed["type"], "register");
                assert!(parsed["token"] == "synthetic-registration");
                seen_tx.send(()).unwrap();
                release_rx.await.unwrap();
                socket.send(Message::Text(json!({"type":"register_ok", "node_id":"synthetic-node", "auth_token":"synthetic-command-auth", "signing_secret":"synthetic-command-signing"}).to_string().into())).await.unwrap();
                let _ = socket.next().await;
            });
            let first_path = profile.to_string_lossy().into_owned();
            let first_url = url.clone();
            let first = tokio::spawn(async move {
                cmd_register(
                    "synthetic-registration",
                    Some(&first_url),
                    Some(&first_path),
                    false,
                )
                .await
            });
            tokio::time::timeout(Duration::from_secs(5), seen_rx)
                .await
                .unwrap()
                .unwrap();
            assert!(!profile.join(".keyfile").exists());
            let authoritative = if retarget {
                std::fs::rename(&profile, &moved).unwrap();
                super::super::storage::Directory::create_private(&profile).unwrap();
                &moved
            } else {
                &profile
            };
            let second = cmd_register(
                "synthetic-unused-competitor",
                Some(&url),
                authoritative.to_str(),
                false,
            )
            .await;
            assert!(
                matches!(second, Err(Error::Io(error)) if error.kind() == std::io::ErrorKind::WouldBlock)
            );
            release_tx.send(()).unwrap();
            tokio::time::timeout(Duration::from_secs(5), first)
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            tokio::time::timeout(Duration::from_secs(5), server)
                .await
                .unwrap()
                .unwrap();
            if retarget {
                assert_eq!(std::fs::read_dir(&profile).unwrap().count(), 0);
            }
            let loaded = NodeConfig::load(&authoritative.join("config.toml")).unwrap();
            let encryption = LocalEncryption::open_existing(authoritative).unwrap();
            assert!(loaded.decrypt_auth_token(&encryption).unwrap() == "synthetic-command-auth");
            assert!(
                loaded
                    .decrypt_signing_secret(&encryption)
                    .unwrap()
                    .as_deref()
                    == Some("synthetic-command-signing")
            );
            let installed = std::fs::read(authoritative.join("config.toml")).unwrap();
            assert!(
                cmd_register(
                    "synthetic-unused-retry",
                    Some(&url),
                    authoritative.to_str(),
                    false
                )
                .await
                .is_err()
            );
            assert_eq!(
                std::fs::read(authoritative.join("config.toml")).unwrap(),
                installed
            );
        }
    }

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
    fn scopeless_node_oauth_rejects_cli_scopes_but_ignores_stale_defaults() {
        let config = oauth::oauth_config_from_catalog_value(&serde_json::json!({
            "token_url": "https://api.notion.com/v1/oauth/token",
            "supports_oauth_scopes": false,
            "default_scopes": ["stale-default"]
        }))
        .unwrap();
        assert_eq!(resolve_oauth_scopes(&config, None, &[]).unwrap(), "");
        assert!(resolve_oauth_scopes(&config, Some("read"), &[]).is_err());
        assert!(resolve_oauth_scopes(&config, None, &["read".into()]).is_err());
        let legacy = oauth::oauth_config_from_catalog_value(&serde_json::json!({
            "token_url": "https://example.com/token", "default_scopes": ["read"]
        }))
        .unwrap();
        assert_eq!(
            resolve_oauth_scopes(&legacy, None, &["write".into()]).unwrap(),
            "read write"
        );
        assert_eq!(
            resolve_oauth_scopes(&legacy, Some("  custom  "), &[]).unwrap(),
            "  custom  "
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
    fn cert_host_key_cli_list_pin_forget_round_trips_and_retofus() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().to_string_lossy();

        cmd_node_ssh(
            crate::cli::NodeSshCommands::CertHostKey {
                command: crate::cli::NodeSshCertHostKeyCommands::List,
            },
            Some(config_path.as_ref()),
        )
        .unwrap();

        cmd_node_ssh(
            crate::cli::NodeSshCommands::CertHostKey {
                command: crate::cli::NodeSshCertHostKeyCommands::Pin {
                    host: "SSH.Example.".to_string(),
                    port: 2222,
                    sha256_fingerprint: "sha256:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=="
                        .to_string(),
                },
            },
            Some(config_path.as_ref()),
        )
        .unwrap();

        let store = CertHostKeyStore::load(dir.path()).unwrap();
        let entry = store.get("ssh.example", 2222).unwrap();
        assert_eq!(entry.host, "ssh.example");
        assert_eq!(entry.host_key_sha256, KEY_A);
        assert_eq!(entry.source, CertHostKeySource::Explicit);

        cmd_node_ssh(
            crate::cli::NodeSshCommands::CertHostKey {
                command: crate::cli::NodeSshCertHostKeyCommands::Forget {
                    host: "[ssh.example]".to_string(),
                    port: 2222,
                },
            },
            Some(config_path.as_ref()),
        )
        .unwrap();

        let mut store = CertHostKeyStore::load(dir.path()).unwrap();
        assert!(store.get("ssh.example", 2222).is_none());
        assert_eq!(
            store
                .ensure_matches_or_pin("ssh.example", 2222, KEY_B)
                .unwrap(),
            super::super::ssh_host_key_store::HostKeyDecision::PinnedNew
        );
        let entry = store.get("ssh.example", 2222).unwrap();
        assert_eq!(entry.host_key_sha256, KEY_B);
        assert_eq!(entry.source, CertHostKeySource::Tofu);
    }

    #[test]
    fn cert_host_key_cli_pin_rejects_bad_fingerprint() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().to_string_lossy();

        let err = cmd_node_ssh(
            crate::cli::NodeSshCommands::CertHostKey {
                command: crate::cli::NodeSshCertHostKeyCommands::Pin {
                    host: "ssh.example".to_string(),
                    port: 22,
                    sha256_fingerprint: "not-a-sha256-fingerprint".to_string(),
                },
            },
            Some(config_path.as_ref()),
        )
        .unwrap_err();

        assert!(matches!(err, Error::Validation(_)));
        let store = CertHostKeyStore::load(dir.path()).unwrap();
        assert!(store.entries().is_empty());
    }

    #[test]
    fn cert_host_key_cli_explicit_pin_is_enforced_and_not_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let config_path = dir.path().to_string_lossy();

        cmd_node_ssh(
            crate::cli::NodeSshCommands::CertHostKey {
                command: crate::cli::NodeSshCertHostKeyCommands::Pin {
                    host: "ssh.example".to_string(),
                    port: 22,
                    sha256_fingerprint: KEY_A.to_string(),
                },
            },
            Some(config_path.as_ref()),
        )
        .unwrap();

        let mut store = CertHostKeyStore::load(dir.path()).unwrap();
        let err = store
            .ensure_matches_or_pin("ssh.example", 22, KEY_B)
            .unwrap_err();

        assert!(matches!(
            err,
            CertHostKeyStoreError::HostKeyMismatch { expected, observed }
                if expected == KEY_A && observed == KEY_B
        ));
        let reloaded = CertHostKeyStore::load(dir.path()).unwrap();
        let entry = reloaded.get("ssh.example", 22).unwrap();
        assert_eq!(entry.host_key_sha256, KEY_A);
        assert_eq!(entry.source, CertHostKeySource::Explicit);
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
        let store = super::super::keychain::KeychainBackend::new_mock("node-1");
        let mut config = NodeConfig::new(
            "wss://example.com/api/v1/nodes/ws".into(),
            "node-1".into(),
            "keychain".into(),
        );
        store.set("auth_token", "nyx_nauth_test").unwrap();
        store
            .set("signing_secret", "00112233445566778899aabbccddeeff")
            .unwrap();
        store.set("cred/openai", "Bearer sk-test").unwrap();
        config.signing.shared_secret_encrypted = Some(String::new());
        config.credentials.insert(
            "openai".into(),
            super::super::config::CredentialConfig::new_header("Authorization".into(), None, None),
        );
        let config_file = dir.path().join("config.toml");
        config.save(&config_file).unwrap();
        super::super::migration::test_migrate(&config_file, "file", &store).unwrap();
        let loaded = NodeConfig::load(&config_file).unwrap();
        assert_eq!(loaded.storage_backend, "file");
        let file_backend =
            SecretBackend::reader_with_keychain(&loaded, dir.path(), store.clone()).unwrap();
        assert_eq!(
            file_backend.load_auth_token(&loaded).unwrap(),
            "nyx_nauth_test"
        );
        assert_eq!(
            file_backend.load_signing_secret(&loaded).unwrap(),
            Some("00112233445566778899aabbccddeeff".into())
        );
        assert!(
            file_backend
                .require_field(
                    &loaded,
                    &super::super::secret_fields::SecretField::Credential {
                        slug: "openai".into(),
                        part: super::super::secret_fields::CredentialPart::Header,
                    }
                )
                .unwrap()
                .as_str()
                == "Bearer sk-test"
        );
        // Check actual source custody, rather than a stale in-memory vault.
        for name in ["auth_token", "signing_secret", "cred/openai"] {
            assert!(store.get_optional(name).unwrap().is_none());
        }
    }

    #[cfg(unix)]
    #[test]
    fn migrate_preserves_source_secrets_when_save_fails() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let store = super::super::keychain::KeychainBackend::new_mock("node-1");
        let mut config = NodeConfig::new(
            "wss://example.com/api/v1/nodes/ws".into(),
            "node-1".into(),
            "keychain".into(),
        );
        store.set("auth_token", "nyx_nauth_test").unwrap();
        store
            .set("signing_secret", "00112233445566778899aabbccddeeff")
            .unwrap();
        store.set("cred/openai", "Bearer sk-test").unwrap();
        config.signing.shared_secret_encrypted = Some(String::new());
        config.credentials.insert(
            "openai".into(),
            super::super::config::CredentialConfig::new_header("Authorization".into(), None, None),
        );
        let path = dir.path().join("config.toml");
        config.save(&path).unwrap();
        let before = std::fs::read(&path).unwrap();
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o500)).unwrap();
        let result = super::super::migration::test_migrate(&path, "file", &store);
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
        assert!(result.is_err());
        assert_eq!(std::fs::read(&path).unwrap(), before);
        assert_eq!(NodeConfig::load(&path).unwrap().storage_backend, "keychain");
        assert_eq!(store.get("auth_token").unwrap(), "nyx_nauth_test");
        assert_eq!(
            store.get("signing_secret").unwrap(),
            "00112233445566778899aabbccddeeff"
        );
        assert_eq!(store.get("cred/openai").unwrap(), "Bearer sk-test");
    }

    #[test]
    fn parse_header_splits_on_colon() {
        let (name, value) = parse_header("Authorization: Bearer sk-test").unwrap();
        assert_eq!(name, "Authorization");
        assert_eq!(value, "Bearer sk-test");
    }

    #[test]
    fn parse_header_rejects_no_colon() {
        assert!(parse_header("NoColonHere").is_err());
    }

    #[test]
    fn parse_query_param_splits_on_equals() {
        let (name, value) = parse_query_param("api_key=sk-test").unwrap();
        assert_eq!(name, "api_key");
        assert_eq!(value, "sk-test");
    }

    #[test]
    fn parse_query_param_rejects_no_equals() {
        assert!(parse_query_param("noequals").is_err());
    }

    #[test]
    fn format_pending_age_formats_seconds() {
        let now = chrono::Utc::now();
        let ts = (now - chrono::Duration::seconds(45)).to_rfc3339();
        let age = format_pending_age(&ts);
        assert!(age.ends_with('s'));
    }

    #[test]
    fn format_pending_age_formats_minutes() {
        let now = chrono::Utc::now();
        let ts = (now - chrono::Duration::minutes(5)).to_rfc3339();
        let age = format_pending_age(&ts);
        assert!(age.ends_with('m'));
    }

    #[test]
    fn format_pending_age_formats_hours() {
        let now = chrono::Utc::now();
        let ts = (now - chrono::Duration::hours(3)).to_rfc3339();
        let age = format_pending_age(&ts);
        assert!(age.ends_with('h'));
    }

    #[test]
    fn format_pending_age_formats_days() {
        let now = chrono::Utc::now();
        let ts = (now - chrono::Duration::days(7)).to_rfc3339();
        let age = format_pending_age(&ts);
        assert!(age.ends_with('d'));
    }

    #[test]
    fn format_pending_age_handles_invalid_timestamp() {
        assert_eq!(format_pending_age("not-a-date"), "-");
    }

    #[test]
    fn read_secret_value_rejects_empty() {
        let result = read_secret_value(Some(String::new()), "test");
        assert!(result.is_err());
    }

    #[test]
    fn read_secret_value_accepts_nonempty() {
        let result = read_secret_value(Some("secret".into()), "test");
        assert_eq!(result.unwrap(), "secret");
    }

    #[test]
    fn format_algorithm_allowlist_none_shows_default() {
        assert_eq!(format_algorithm_allowlist(None), "default");
    }

    #[test]
    fn format_algorithm_allowlist_empty_shows_marker() {
        assert_eq!(format_algorithm_allowlist(Some(&[])), "<empty>");
    }

    #[test]
    fn format_algorithm_allowlist_values_joined() {
        let vals = vec!["a".to_string(), "b".to_string()];
        assert_eq!(format_algorithm_allowlist(Some(&vals)), "a, b");
    }

    #[test]
    fn non_empty_algorithm_list_empty_returns_none() {
        assert!(non_empty_algorithm_list(vec![]).is_none());
    }

    #[test]
    fn non_empty_algorithm_list_nonempty_returns_some() {
        assert!(non_empty_algorithm_list(vec!["aes128-ctr".to_string()]).is_some());
    }

    #[test]
    fn normalize_algorithm_preferences_all_none_returns_none() {
        let prefs = SshAlgorithmPreferences {
            kex: None,
            host_key: None,
            cipher: None,
            mac: None,
        };
        assert!(normalize_algorithm_preferences(prefs).unwrap().is_none());
    }

    #[test]
    fn validate_algorithm_reset_args_rejects_empty() {
        let args = AlgorithmResetArgs {
            kex: &[],
            host_key: &[],
            cipher: &[],
            mac: &[],
            reset_kex: false,
            reset_host_key: false,
            reset_cipher: false,
            reset_mac: false,
            reset_all: false,
        };
        assert!(validate_algorithm_reset_args(args).is_err());
    }

    #[test]
    fn validate_algorithm_reset_args_rejects_reset_all_with_list() {
        let kex = vec!["diffie-hellman".to_string()];
        let args = AlgorithmResetArgs {
            kex: &kex,
            host_key: &[],
            cipher: &[],
            mac: &[],
            reset_kex: false,
            reset_host_key: false,
            reset_cipher: false,
            reset_mac: false,
            reset_all: true,
        };
        assert!(validate_algorithm_reset_args(args).is_err());
    }

    #[test]
    fn validate_algorithm_reset_args_rejects_kex_with_reset_kex() {
        let kex = vec!["dh".to_string()];
        let args = AlgorithmResetArgs {
            kex: &kex,
            host_key: &[],
            cipher: &[],
            mac: &[],
            reset_kex: true,
            reset_host_key: false,
            reset_cipher: false,
            reset_mac: false,
            reset_all: false,
        };
        assert!(validate_algorithm_reset_args(args).is_err());
    }

    #[test]
    fn validate_algorithm_reset_args_accepts_reset_all_alone() {
        let args = AlgorithmResetArgs {
            kex: &[],
            host_key: &[],
            cipher: &[],
            mac: &[],
            reset_kex: false,
            reset_host_key: false,
            reset_cipher: false,
            reset_mac: false,
            reset_all: true,
        };
        assert!(validate_algorithm_reset_args(args).is_ok());
    }

    #[test]
    fn validate_algorithm_reset_args_rejects_host_key_with_reset_host_key() {
        let hk = vec!["rsa".to_string()];
        let args = AlgorithmResetArgs {
            kex: &[],
            host_key: &hk,
            cipher: &[],
            mac: &[],
            reset_kex: false,
            reset_host_key: true,
            reset_cipher: false,
            reset_mac: false,
            reset_all: false,
        };
        assert!(validate_algorithm_reset_args(args).is_err());
    }

    #[test]
    fn validate_algorithm_reset_args_rejects_cipher_with_reset_cipher() {
        let c = vec!["aes".to_string()];
        let args = AlgorithmResetArgs {
            kex: &[],
            host_key: &[],
            cipher: &c,
            mac: &[],
            reset_kex: false,
            reset_host_key: false,
            reset_cipher: true,
            reset_mac: false,
            reset_all: false,
        };
        assert!(validate_algorithm_reset_args(args).is_err());
    }

    #[test]
    fn validate_algorithm_reset_args_rejects_mac_with_reset_mac() {
        let m = vec!["hmac".to_string()];
        let args = AlgorithmResetArgs {
            kex: &[],
            host_key: &[],
            cipher: &[],
            mac: &m,
            reset_kex: false,
            reset_host_key: false,
            reset_cipher: false,
            reset_mac: true,
            reset_all: false,
        };
        assert!(validate_algorithm_reset_args(args).is_err());
    }

    #[test]
    fn store_pending_credential_locally_header_method() {
        let dir = tempfile::tempdir().unwrap();
        let backend = SecretBackend::File(LocalEncryption::load_or_generate(dir.path()).unwrap());
        let mut config =
            NodeConfig::new("ws://x/api/v1/nodes/ws".into(), "n1".into(), "file".into());
        let pending = PendingCredentialMetadata {
            id: "p1".into(),
            service_slug: "svc".into(),
            injection_method: "header".into(),
            field_name: "Authorization".into(),
            target_url: None,
            label: None,
            created_at: "2026-01-01T00:00:00Z".into(),
            expires_at: "2026-01-02T00:00:00Z".into(),
        };
        store_pending_credential_locally(&mut config, &backend, &pending, "Bearer sk-test")
            .unwrap();
        assert!(config.credentials.contains_key("svc"));
    }

    #[test]
    fn store_pending_credential_locally_query_param() {
        let dir = tempfile::tempdir().unwrap();
        let backend = SecretBackend::File(LocalEncryption::load_or_generate(dir.path()).unwrap());
        let mut config =
            NodeConfig::new("ws://x/api/v1/nodes/ws".into(), "n1".into(), "file".into());
        let pending = PendingCredentialMetadata {
            id: "p2".into(),
            service_slug: "svc2".into(),
            injection_method: "query-param".into(),
            field_name: "api_key".into(),
            target_url: None,
            label: None,
            created_at: "2026-01-01T00:00:00Z".into(),
            expires_at: "2026-01-02T00:00:00Z".into(),
        };
        store_pending_credential_locally(&mut config, &backend, &pending, "sk-test").unwrap();
        assert!(config.credentials.contains_key("svc2"));
    }

    #[test]
    fn store_pending_credential_locally_unsupported_method() {
        let dir = tempfile::tempdir().unwrap();
        let backend = SecretBackend::File(LocalEncryption::load_or_generate(dir.path()).unwrap());
        let mut config =
            NodeConfig::new("ws://x/api/v1/nodes/ws".into(), "n1".into(), "file".into());
        let pending = PendingCredentialMetadata {
            id: "p3".into(),
            service_slug: "svc3".into(),
            injection_method: "unknown".into(),
            field_name: "x".into(),
            target_url: None,
            label: None,
            created_at: "2026-01-01T00:00:00Z".into(),
            expires_at: "2026-01-02T00:00:00Z".into(),
        };
        assert!(store_pending_credential_locally(&mut config, &backend, &pending, "val").is_err());
    }

    #[test]
    fn store_pending_credential_locally_path_prefix() {
        let dir = tempfile::tempdir().unwrap();
        let backend = SecretBackend::File(LocalEncryption::load_or_generate(dir.path()).unwrap());
        let mut config =
            NodeConfig::new("ws://x/api/v1/nodes/ws".into(), "n1".into(), "file".into());
        let pending = PendingCredentialMetadata {
            id: "p4".into(),
            service_slug: "svc4".into(),
            injection_method: "path-prefix".into(),
            field_name: "v1".into(),
            target_url: None,
            label: None,
            created_at: "2026-01-01T00:00:00Z".into(),
            expires_at: "2026-01-02T00:00:00Z".into(),
        };
        store_pending_credential_locally(&mut config, &backend, &pending, "key123").unwrap();
        assert!(config.credentials.contains_key("svc4"));
    }

    #[test]
    fn legacy_crypto_none_accept_flow_unchanged() {
        let response: PendingCredentialListResponse = serde_json::from_value(json!({
            "pending_credentials": [{
                "id": "p-legacy",
                "service_slug": "svc",
                "injection_method": "header",
                "field_name": "Authorization",
                "target_url": null,
                "label": null,
                "created_at": "2026-01-01T00:00:00Z",
                "expires_at": "2026-01-02T00:00:00Z"
            }]
        }))
        .unwrap();
        let pending = response.pending_credentials.first().unwrap();
        let dir = tempfile::tempdir().unwrap();
        let backend = SecretBackend::File(LocalEncryption::load_or_generate(dir.path()).unwrap());
        let mut config =
            NodeConfig::new("ws://x/api/v1/nodes/ws".into(), "n1".into(), "file".into());

        store_pending_credential_locally(&mut config, &backend, pending, "Bearer sk-test").unwrap();

        let stored = config.credentials.get("svc").unwrap();
        assert_eq!(stored.injection_method, "header");
        assert!(config.pending_crypto_keys.is_empty());
    }

    #[test]
    fn find_existing_service_returns_none_when_no_keys() {
        let resp = json!({"keys": []});
        assert!(find_existing_service(&resp, "test", "node-a").is_none());
    }

    #[test]
    fn find_existing_service_returns_none_missing_keys_field() {
        let resp = json!({});
        assert!(find_existing_service(&resp, "test", "node-a").is_none());
    }

    #[test]
    fn api_base_url_from_node_ws_url_handles_no_ws_path() {
        assert_eq!(
            api_base_url_from_node_ws_url("https://example.com"),
            "https://example.com"
        );
    }

    #[test]
    fn api_base_url_from_node_ws_url_strips_trailing_slash() {
        assert_eq!(
            api_base_url_from_node_ws_url("ws://localhost:3001/api/v1/nodes/ws/"),
            "http://localhost:3001"
        );
    }

    #[test]
    fn cleanup_source_secrets_removes_auth_signing_and_credentials() {
        let dir = tempfile::tempdir().unwrap();
        let store = super::super::keychain::KeychainBackend::new_mock("node-1");
        let mut config = NodeConfig::new(
            "wss://example.com/api/v1/nodes/ws".to_string(),
            "node-1".to_string(),
            "keychain".to_string(),
        );
        store.set("auth_token", "nyx_nauth_test").unwrap();
        store
            .set("signing_secret", "00112233445566778899aabbccddeeff")
            .unwrap();
        store.set("cred/openai", "Bearer sk-test").unwrap();
        config.signing.shared_secret_encrypted = Some(String::new());
        config.credentials.insert(
            "openai".into(),
            super::super::config::CredentialConfig::new_header("Authorization".into(), None, None),
        );
        let path = dir.path().join("config.toml");
        config.save(&path).unwrap();
        super::super::migration::test_migrate(&path, "file", &store).unwrap();

        for name in ["auth_token", "signing_secret", "cred/openai"] {
            assert!(store.get_optional(name).unwrap().is_none());
        }
        let installed = NodeConfig::load(&path).unwrap();
        let reader = SecretBackend::reader_with_keychain(&installed, dir.path(), store).unwrap();
        assert_eq!(
            reader.load_auth_token(&installed).unwrap(),
            "nyx_nauth_test"
        );
        assert_eq!(
            reader.load_signing_secret(&installed).unwrap().as_deref(),
            Some("00112233445566778899aabbccddeeff")
        );
        assert!(
            reader
                .require_field(
                    &installed,
                    &super::super::secret_fields::SecretField::Credential {
                        slug: "openai".into(),
                        part: super::super::secret_fields::CredentialPart::Header,
                    }
                )
                .unwrap()
                .as_str()
                == "Bearer sk-test"
        );
    }
}

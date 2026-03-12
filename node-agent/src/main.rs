mod cli;
mod config;
mod credential_store;
mod encryption;
mod error;
mod metrics;
mod proxy_executor;
mod signing;
mod ws_client;

use clap::Parser;
use tracing_subscriber::EnvFilter;

use crate::cli::{Cli, Commands, CredentialCommands};
use crate::config::NodeConfig;
use crate::credential_store::CredentialStore;
use crate::encryption::LocalEncryption;
use crate::error::Result;

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
        } => {
            cmd_register(&token, url.as_deref(), config_path.as_deref()).await
        }
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
        Commands::Version => {
            println!("nyxid-node {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
    }
}

async fn cmd_register(token: &str, url: Option<&str>, config_path: Option<&str>) -> Result<()> {
    let config_dir = config::resolve_config_dir(config_path);
    std::fs::create_dir_all(&config_dir)?;

    let encryption = LocalEncryption::load_or_generate(&config_dir)?;

    // M2: Use ws:// for localhost (wss:// requires TLS not available in dev)
    let ws_url = url.unwrap_or("ws://localhost:3001/api/v1/nodes/ws");

    tracing::info!(url = %ws_url, "Registering node...");

    let (node_id, auth_token, signing_secret) =
        ws_client::register_node(ws_url, token).await?;

    tracing::info!(node_id = %node_id, "Registration successful");

    let mut config = NodeConfig::new(ws_url.to_string(), node_id);
    config.set_auth_token(&auth_token, &encryption)?;
    if let Some(secret) = signing_secret {
        config.set_signing_secret(&secret, &encryption)?;
    }

    let config_file = config_dir.join("config.toml");
    config.save(&config_file)?;

    tracing::info!(path = %config_file.display(), "Configuration saved");
    println!("Node registered successfully.");
    println!("  Node ID: {}", config.node.id);
    println!("  Config:  {}", config_file.display());
    println!();
    println!("Start the agent with:");
    println!("  nyxid-node start");

    Ok(())
}

async fn cmd_start(config_path: Option<&str>) -> Result<()> {
    let config_dir = config::resolve_config_dir(config_path);
    let config_file = config_dir.join("config.toml");

    let config = NodeConfig::load(&config_file)?;
    let encryption = LocalEncryption::load_or_generate(&config_dir)?;

    let auth_token = config.decrypt_auth_token(&encryption)?;
    let signing_secret = config.decrypt_signing_secret(&encryption)?;
    let credentials = CredentialStore::from_config(&config, &encryption)?;

    tracing::info!(
        node_id = %config.node.id,
        server = %config.server.url,
        credentials = credentials.count(),
        "Starting node agent"
    );

    ws_client::run_with_shutdown(config, auth_token, signing_secret, credentials).await;

    Ok(())
}

fn cmd_status(config_path: Option<&str>) -> Result<()> {
    let config_dir = config::resolve_config_dir(config_path);
    let config_file = config_dir.join("config.toml");

    let config = NodeConfig::load(&config_file)?;
    let encryption = LocalEncryption::load_or_generate(&config_dir)?;
    let credentials = CredentialStore::from_config(&config, &encryption)?;

    println!("Node Status");
    println!("  Node ID:     {}", config.node.id);
    println!("  Server:      {}", config.server.url);
    println!("  Credentials: {} configured", credentials.count());

    for slug in credentials.service_slugs() {
        println!("    - {slug}");
    }

    Ok(())
}

fn cmd_rekey(
    auth_token: &str,
    signing_secret: &str,
    config_path: Option<&str>,
) -> Result<()> {
    let config_dir = config::resolve_config_dir(config_path);
    let config_file = config_dir.join("config.toml");
    let encryption = LocalEncryption::load_or_generate(&config_dir)?;

    let mut config = NodeConfig::load(&config_file)?;
    config.set_auth_token(auth_token, &encryption)?;
    config.set_signing_secret(signing_secret, &encryption)?;
    config.save(&config_file)?;

    println!("Node credentials updated.");
    println!("Restart the agent to reconnect with the rotated credentials.");
    Ok(())
}

fn cmd_credentials(command: CredentialCommands, config_path: Option<&str>) -> Result<()> {
    let config_dir = config::resolve_config_dir(config_path);
    let config_file = config_dir.join("config.toml");
    let encryption = LocalEncryption::load_or_generate(&config_dir)?;

    match command {
        CredentialCommands::Add {
            service,
            header,
            query_param,
        } => {
            let mut config = NodeConfig::load(&config_file)?;

            if let Some(header_val) = header {
                let (name, value) = parse_header(&header_val)?;
                config.add_header_credential(&service, &name, &value, &encryption)?;
            } else if let Some(qp_val) = query_param {
                let (name, value) = parse_query_param(&qp_val)?;
                config.add_query_param_credential(&service, &name, &value, &encryption)?;
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
            let creds = CredentialStore::from_config(&config, &encryption)?;

            if creds.count() == 0 {
                println!("No credentials configured.");
            } else {
                println!("Configured credentials:");
                for slug in creds.service_slugs() {
                    if let Some(cred) = creds.get(&slug) {
                        println!("  {slug}: {} ({})", cred.injection_method(), cred.target_name());
                    }
                }
            }
            Ok(())
        }
        CredentialCommands::Remove { service } => {
            let mut config = NodeConfig::load(&config_file)?;
            config.remove_credential(&service)?;
            config.save(&config_file)?;
            println!("Credential removed for service '{service}'.");
            Ok(())
        }
    }
}

fn parse_header(header: &str) -> Result<(String, String)> {
    let (name, value) = header.split_once(':').ok_or_else(|| {
        crate::error::Error::Validation(
            "Header must be in 'Name: value' format (e.g., 'Authorization: Bearer sk-...')".to_string(),
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

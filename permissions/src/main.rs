use std::{net::SocketAddr, path::PathBuf, sync::Arc};

use clap::{Parser, Subcommand};
use nyxid_permissions::{
    Engine, Policy, Transport,
    drive::{DriveAdapter, example_policy},
    http_transport::NyxIdTransport,
    mock::MockDrive,
    parse_json, server,
};
use uuid::Uuid;
use zeroize::Zeroizing;

#[derive(Parser)]
#[command(about = "Local, experimental Google Drive permission proxy backed by NyxID")]
struct Args {
    #[arg(long, default_value = "127.0.0.1:4318")]
    listen: SocketAddr,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// In-memory Drive only; no account connection or Google calls.
    Demo {
        /// Optional policy override for exercising hooks and constraints.
        #[arg(long)]
        policy: Option<PathBuf>,
    },
    /// Forward permitted requests through an existing NyxID Google connection.
    Serve {
        #[arg(long)]
        policy: PathBuf,
        #[arg(long)]
        nyxid_url: String,
        /// Catalog service UUID (Google Drive or Workspace).
        #[arg(long)]
        service_id: Uuid,
        /// Exact UserService UUID used as NyxID's _nyxid_via selector.
        #[arg(long)]
        connection_id: Uuid,
    },
}

fn secret(name: &str) -> Result<Zeroizing<String>, Box<dyn std::error::Error>> {
    Ok(Zeroizing::new(std::env::var(name).map_err(|_| {
        format!("Set {name}; credentials are never accepted as command-line arguments")
    })?))
}

fn load_policy(path: PathBuf) -> Result<Policy, Box<dyn std::error::Error>> {
    if std::fs::metadata(&path)?.len() > 64 * 1024 {
        return Err("Policy file exceeds 64 KiB".into());
    }
    Ok(serde_json::from_value(parse_json(&std::fs::read(path)?)?)?)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    if !args.listen.ip().is_loopback() {
        return Err("The POC only listens on loopback".into());
    }
    let client_key = secret("NYXID_POC_CLIENT_KEY")?;
    let (policy, transport): (Policy, Arc<dyn Transport>) = match args.command {
        Command::Demo { policy } => (
            policy
                .map(load_policy)
                .transpose()?
                .unwrap_or_else(example_policy),
            Arc::new(MockDrive::default()),
        ),
        Command::Serve {
            policy,
            nyxid_url,
            service_id,
            connection_id,
        } => {
            let policy = load_policy(policy)?;
            let upstream_key = secret("NYXID_POC_UPSTREAM_KEY")?;
            if client_key.as_str() == upstream_key.as_str() {
                return Err("Client and upstream keys must be different".into());
            }
            (
                policy,
                Arc::new(NyxIdTransport::new(
                    &nyxid_url,
                    service_id,
                    connection_id,
                    upstream_key,
                )?),
            )
        }
    };
    let engine = Arc::new(Engine::new(policy, Arc::new(DriveAdapter), transport)?);
    let router = server::router(engine, client_key)?;
    let listener = tokio::net::TcpListener::bind(args.listen).await?;
    eprintln!(
        "Drive permission POC: http://{}; MCP: /mcp",
        listener.local_addr()?
    );
    axum::serve(listener, router)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}

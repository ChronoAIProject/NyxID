//! Deploy on an independently administered host, behind an authenticated TLS
//! ingress. This executable never connects to Mongo or accepts enrollment HTTP.
use std::{net::SocketAddr, path::PathBuf, sync::Arc};

use clap::{Parser, Subcommand};
use nyxid::{
    handlers::audit_witness::router,
    models::audit_witness::AuditLink,
    services::audit_witness::{ReferenceWitness, WitnessError, WitnessResult, storage},
};
use uuid::Uuid;

#[derive(Parser)]
#[command(about = "Independent append-only NyxID audit witness")]
struct Args {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Offline root/admin provisioning. All destination paths must be new.
    Enroll {
        #[arg(long)]
        admin_dir: PathBuf,
        #[arg(long)]
        journal_dir: PathBuf,
        #[arg(long)]
        token_output: PathBuf,
        #[arg(long)]
        sink_uid: u32,
        #[arg(long)]
        sink_gid: u32,
        #[arg(long)]
        chain_id: Uuid,
        #[arg(long)]
        key_version: u32,
        #[arg(long)]
        seq: u64,
        #[arg(long)]
        prev_hash: String,
        #[arg(long)]
        entry_hash: String,
    },
    /// Run as the unprivileged sink UID; a different UID owns enrollment.
    Serve {
        #[arg(long)]
        admin_dir: PathBuf,
        #[arg(long)]
        journal_dir: PathBuf,
        /// Loopback only; provide HTTPS on the independent host's TLS ingress.
        #[arg(long, default_value = "127.0.0.1:8091")]
        listen: SocketAddr,
    },
}

#[tokio::main]
async fn main() -> std::process::ExitCode {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
    match execute(Args::parse()).await {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("audit witness: {error}");
            std::process::ExitCode::FAILURE
        }
    }
}

async fn execute(args: Args) -> WitnessResult<()> {
    match args.command {
        Command::Enroll {
            admin_dir,
            journal_dir,
            token_output,
            sink_uid,
            sink_gid,
            chain_id,
            key_version,
            seq,
            prev_hash,
            entry_hash,
        } => {
            storage::enroll(
                &admin_dir,
                &journal_dir,
                &token_output,
                sink_uid,
                sink_gid,
                chain_id,
                AuditLink {
                    key_version,
                    seq,
                    prev_hash,
                    entry_hash,
                },
            )?;
            println!(
                "audit witness enrollment durable; distribute the public pin and append credential separately"
            );
        }
        Command::Serve {
            admin_dir,
            journal_dir,
            listen,
        } => {
            if !listen.ip().is_loopback() {
                return Err(WitnessError::Invalid);
            }
            let witness = Arc::new(ReferenceWitness::open(&admin_dir, &journal_dir)?);
            let listener = tokio::net::TcpListener::bind(listen)
                .await
                .map_err(|_| WitnessError::Unavailable)?;
            println!("audit witness ready");
            axum::serve(listener, router(witness))
                .with_graceful_shutdown(async {
                    let _ = tokio::signal::ctrl_c().await;
                })
                .await
                .map_err(|_| WitnessError::Unavailable)?;
        }
    }
    Ok(())
}

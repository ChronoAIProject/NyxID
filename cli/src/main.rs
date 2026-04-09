mod api;
mod auth;
mod cli;
mod commands;
pub mod node;

use anyhow::Result;
use clap::Parser;

use crate::cli::{Cli, Commands};

#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("Error: {e:#}");
        std::process::exit(1);
    }
}

async fn run() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Login(args) => auth::run_login(args).await,
        Commands::Logout(args) => {
            auth::run_logout(&args.resolved_base_url()?, args.profile.as_deref()).await
        }

        // C1-C4: Auth flows (unauthenticated)
        Commands::Register(args) => commands::auth_flows::run_register(args).await,
        Commands::VerifyEmail(args) => commands::auth_flows::run_verify_email(args).await,
        Commands::ForgotPassword(args) => commands::auth_flows::run_forgot_password(args).await,
        Commands::ResetPassword(args) => commands::auth_flows::run_reset_password(args).await,

        Commands::Whoami(auth) => {
            let mut api = api::ApiClient::from_auth(&auth)?;
            commands::whoami::run(&mut api, auth.output).await
        }
        Commands::Status(auth) => {
            let mut api = api::ApiClient::from_auth(&auth)?;
            commands::status::run(&mut api, auth.output).await
        }

        // C5, I1-I3: Profile
        Commands::Profile { command } => commands::profile::run(command).await,

        // C6: MFA
        Commands::Mfa { command } => commands::mfa::run(command).await,

        // C7: Sessions
        Commands::Session { command } => commands::session::run(command).await,

        Commands::Catalog { command } => commands::catalog::run(command).await,
        Commands::Service { command } => commands::service::run(command).await,
        Commands::ApiKey { command } => commands::api_key::run(command).await,
        Commands::Node { command } => commands::node::run(command).await,

        // C8-C10: Proxy
        Commands::Proxy { command } => commands::proxy::run(command).await,

        Commands::Ssh(ssh) => commands::ssh::run(ssh).await,
        Commands::Openclaw { command } => commands::openclaw::run(command).await,
        Commands::Mcp { command } => commands::mcp::run(command).await,

        // I11-I14: Notifications
        Commands::Notification { command } => commands::notification::run(command).await,

        // I15-I20: Approvals
        Commands::Approval { command } => commands::approval::run(command).await,

        // I24: Endpoints
        Commands::Endpoint { command } => commands::endpoint::run(command).await,

        // I25-I26: External keys
        Commands::ExternalKey { command } => commands::external_key::run(command).await,

        // AI skill setup
        Commands::AiSetup { command } => commands::ai_setup::run(command).await,

        // Self-update CLI + skills
        Commands::Update(args) => commands::update::run(args).await,

        // Channel bot relay
        Commands::ChannelBot { command } => commands::channel_bot::run(command).await,

        // HTTP Event Gateway (device events)
        Commands::ChannelEvent { command } => commands::channel_event::run(command).await,

        // Admin-only operations
        Commands::Admin { command } => commands::admin::run(command).await,
    }
}

mod api;
mod auth;
mod cli;
mod commands;
pub mod node;
mod wizard;

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
        Commands::Org { command } => commands::org::run(command).await,
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

        // Wizard v2 (experimental, see docs/CLI_WIZARD_V2.md)
        Commands::Wizard(args) => run_wizard(args).await,
    }
}

async fn run_wizard(args: cli::WizardArgs) -> Result<()> {
    // Resolve base URL + bearer from the shared AuthArgs so the wizard's
    // proxy can forward allowlisted requests to the NyxID backend. If the
    // user isn't logged in, this surfaces the usual "run nyxid login" error.
    let base_url = args.auth.resolved_base_url()?;
    let access_token = auth::resolve_access_token(&args.auth)?;
    let base_url_root = base_url.trim_end_matches('/').to_string();
    let proxy = wizard::ProxyContext {
        base_url_root,
        access_token,
    };

    match wizard::run_flow(&args.flow, proxy).await? {
        wizard::WizardOutcome::Completed(body) => {
            print_wizard_summary(&body, &base_url);
            Ok(())
        }
        wizard::WizardOutcome::Cancelled => {
            eprintln!("✗ Wizard cancelled. No service was created.");
            std::process::exit(1);
        }
        wizard::WizardOutcome::TimedOut => {
            eprintln!("✗ Wizard timed out. No service was created.");
            eprintln!("  Tip: for scripted use, pass a slug and --credential-env:");
            eprintln!("       nyxid service add <slug> --credential-env VAR --label <label>");
            std::process::exit(1);
        }
    }
}

/// Format the happy-path completion summary per docs/CLI_WIZARD_V2.md §3.2.
///
/// The body is whatever the browser posted to `/api/proxy/complete`. For the
/// `ai-key` flow it looks like: { flow, milestone, slug, label, proxy_url }.
/// `proxy_url` may be null if the backend didn't echo one; we synthesize a
/// sensible default from the CLI's own base_url.
fn print_wizard_summary(body: &serde_json::Value, base_url: &str) {
    let slug = body.get("slug").and_then(|v| v.as_str());
    let label = body.get("label").and_then(|v| v.as_str());
    let proxy_url = body.get("proxy_url").and_then(|v| v.as_str());

    match slug {
        Some(slug) => {
            let display_label = label.unwrap_or(slug);
            eprintln!("✓ Service '{display_label}' created.");
            eprintln!("  Slug:      {slug}");
            let rendered_url = match proxy_url {
                Some(u) => u.to_string(),
                None => format!(
                    "{}/api/v1/proxy/s/{}/",
                    base_url.trim_end_matches('/'),
                    slug
                ),
            };
            eprintln!("  Proxy URL: {rendered_url}");
            eprintln!();
            eprintln!("  Next:");
            eprintln!(
                "    curl {}<api-path> -H \"Authorization: Bearer $NYX_KEY\"",
                if rendered_url.ends_with('/') {
                    rendered_url.clone()
                } else {
                    format!("{rendered_url}/")
                }
            );
            eprintln!("  Example: append /v1/models for OpenAI-compatible providers.");
        }
        None => {
            // Wizard completed but didn't produce a slug. Could be a
            // placeholder-only flow (early-milestone builds) or the user
            // pressed Done on Step 1 with nothing selected. Print a minimal
            // acknowledgement so the exit isn't silent.
            eprintln!("✓ Wizard completed (no service created).");
        }
    }
}

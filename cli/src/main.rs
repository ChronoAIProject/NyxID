mod api;
mod auth;
mod cli;
mod commands;
mod error_format;
pub mod node;
pub mod org_resolver;
mod skill_self_heal;
mod telemetry;
#[cfg(test)]
mod test_support;
mod wizard;

use anyhow::Result;
use clap::Parser;

use crate::cli::{Cli, Commands};

#[tokio::main]
async fn main() {
    // `--output` is flattened into many subcommands, so this best-effort
    // pre-parse only controls final error formatting. Clap remains the source
    // of truth for command parsing and success-path output.
    let json_output_from_argv = error_format::detect_json_output_from_argv();

    // Wrap all work so we can emit one telemetry event with exit code
    // and duration after dispatch returns, regardless of success.
    let start = std::time::Instant::now();
    let cli = Cli::parse();
    let profile = extract_profile(&cli.command);

    // Telemetry is hard-off by default: if no DSN is configured and
    // share-back is not opted into, don't resolve consent and don't
    // prompt. This keeps default-off CLI behavior byte-identical to
    // the pre-telemetry build (no new prompts, no new files written).
    let telemetry_dsn_configured = std::env::var("NYXID_TELEMETRY_DSN")
        .ok()
        .is_some_and(|s| !s.is_empty())
        || std::env::var("NYXID_SHARE_ANALYTICS")
            .ok()
            .is_some_and(|v| {
                matches!(v.to_ascii_lowercase().as_str(), "true" | "1" | "yes" | "on")
            });

    let mut tele_client: Option<telemetry::TelemetryClient> = if telemetry_dsn_configured {
        // DSN is present. Resolve consent, prompt if first-run TTY,
        // then init. Prompt refusal never bails the command.
        //
        // Consent resolution honors any explicit per-profile choice
        // persisted by older releases (via `_preferring_profile`) but
        // otherwise defaults to the user-global (default profile)
        // config. That keeps migration safe: upgrading will not
        // silently override a historical opt-out on a named profile.
        // Going forward, only the default profile is written to — the
        // prompt and the `nyxid telemetry` editor both use `None`.
        //
        // `TelemetryClient::init` receives `profile` so the anon
        // distinct_id file is isolated per profile (identity, not
        // consent — different concern).
        let mut consent =
            telemetry::consent::resolve_consent_preferring_profile(profile.as_deref());
        let _ = telemetry::consent::prompt_if_needed_interactive(None, &mut consent);
        if consent.enabled {
            telemetry::TelemetryClient::init(profile.as_deref())
        } else {
            None
        }
    } else {
        None
    };

    let (group, sub) = command_names(&cli.command);

    // Best-effort: detect partially-installed skills (caused by older CLI
    // binaries that predate per-topic references) and refresh them before the
    // user's command runs. Failures here never block the command.
    skill_self_heal::maybe_self_heal(&cli.command).await;

    let result = run(cli).await;

    if let Some(client) = tele_client.as_mut() {
        client
            .track(telemetry::CliEvent::CommandInvoked {
                command_group: group,
                subcommand: sub,
                exit_code: if result.is_ok() { 0 } else { 1 },
                duration_ms: start.elapsed().as_millis() as u64,
                profile: profile.clone(),
                os: std::env::consts::OS,
                arch: std::env::consts::ARCH,
            })
            .await;
    }

    if let Err(e) = result {
        eprintln!("{}", error_format::render_error(&e, json_output_from_argv));
        std::process::exit(1);
    }
}

fn extract_profile(command: &Commands) -> Option<String> {
    // `AuthArgs` (profile-bearing struct) is flattened into many
    // subcommands; rather than enumerate all of them we peek at the
    // one path we care about — the login command is the primary place
    // profile is user-supplied. This is best-effort; telemetry tags
    // without profile are acceptable.
    match command {
        Commands::Login(args) => args.profile.clone(),
        _ => None,
    }
}

fn command_names(command: &Commands) -> (&'static str, &'static str) {
    match command {
        Commands::Login(_) => ("auth", "login"),
        Commands::Logout(_) => ("auth", "logout"),
        Commands::Register(_) => ("auth", "register"),
        Commands::VerifyEmail(_) => ("auth", "verify_email"),
        Commands::ForgotPassword(_) => ("auth", "forgot_password"),
        Commands::ResetPassword(_) => ("auth", "reset_password"),
        Commands::Whoami(_) => ("user", "whoami"),
        Commands::Status(_) => ("user", "status"),
        Commands::Profile { .. } => ("user", "profile"),
        Commands::Mfa { .. } => ("user", "mfa"),
        Commands::Session { .. } => ("user", "session"),
        Commands::Catalog { .. } => ("catalog", "subcommand"),
        Commands::Service { .. } => ("service", "subcommand"),
        Commands::Provider { .. } => ("provider", "subcommand"),
        Commands::ApiKey { .. } => ("api_key", "subcommand"),
        Commands::Org { .. } => ("org", "subcommand"),
        Commands::Node { .. } => ("node", "subcommand"),
        Commands::NodeCredential { .. } => ("node_credential", "subcommand"),
        Commands::Proxy { .. } => ("proxy", "subcommand"),
        Commands::Ssh(_) => ("ssh", "subcommand"),
        Commands::Openclaw { .. } => ("openclaw", "subcommand"),
        Commands::Mcp { .. } => ("mcp", "subcommand"),
        Commands::Notification { .. } => ("notification", "subcommand"),
        Commands::Oauth { .. } => ("oauth", "subcommand"),
        Commands::Approval { .. } => ("approval", "subcommand"),
        Commands::Endpoint { .. } => ("endpoint", "subcommand"),
        Commands::ExternalKey { .. } => ("external_key", "subcommand"),
        Commands::ServiceAccount { .. } => ("service_account", "subcommand"),
        Commands::DeveloperApp { .. } => ("developer_app", "subcommand"),
        Commands::AiSetup { .. } => ("ai_setup", "subcommand"),
        Commands::Update(_) => ("cli", "update"),
        Commands::ChannelBot { .. } => ("channel_bot", "subcommand"),
        Commands::ChannelEvent { .. } => ("channel_event", "subcommand"),
        Commands::Admin { .. } => ("admin", "subcommand"),
        Commands::Telemetry { .. } => ("telemetry", "subcommand"),
        Commands::Repo(_) => ("repo", "repo"),
        Commands::Pairing { .. } => ("pairing", "subcommand"),
        Commands::Info => ("repo", "info"),
    }
}

async fn run(cli: Cli) -> Result<()> {
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
        Commands::Provider { command } => commands::provider::run(command).await,
        Commands::ApiKey { command } => commands::api_key::run(command).await,
        Commands::Org { command } => commands::org::run(command).await,
        Commands::Node { command } => commands::node::run(command).await,
        Commands::NodeCredential { command } => commands::node_credential::run(command).await,

        // C8-C10: Proxy
        Commands::Proxy { command } => commands::proxy::run(command).await,

        Commands::Ssh(ssh) => commands::ssh::run(ssh).await,
        Commands::Openclaw { command } => commands::openclaw::run(command).await,
        Commands::Mcp { command } => commands::mcp::run(command).await,

        // I11-I14: Notifications
        Commands::Notification { command } => commands::notification::run(command).await,

        Commands::Oauth { command } => commands::oauth::run(command).await,

        // I15-I20: Approvals
        Commands::Approval { command } => commands::approval::run(command).await,

        // I24: Endpoints
        Commands::Endpoint { command } => commands::endpoint::run(command).await,

        // I25-I26: External keys
        Commands::ExternalKey { command } => commands::external_key::run(command).await,

        // SUP-030: Service accounts (admin + org-admin)
        Commands::ServiceAccount { command } => commands::service_account::run(command).await,

        // SUP-030: Developer OAuth apps (personal + org-admin)
        Commands::DeveloperApp { command } => commands::developer_app::run(command).await,

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

        // Telemetry (consent editor; also docs/TELEMETRY.md §3)
        Commands::Telemetry { command } => commands::telemetry::run(command, None).await,

        // Project links
        Commands::Repo(args) => commands::repo::run_repo(args).await,
        Commands::Pairing { command } => commands::pairing::run(command).await,
        Commands::Info => commands::repo::run_info().await,
    }
}

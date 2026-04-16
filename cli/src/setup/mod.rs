mod integrations;
mod proxy;
pub mod server;

use std::sync::Arc;

use anyhow::{Context, Result, bail};
use rand::RngCore;
use tokio::sync::Mutex;

use crate::api::ApiClient;
use crate::cli::SetupCommands;

use integrations::IntegrationAdapter;
use integrations::ha_addon::HaAddonAdapter;
use server::{CompletionResult, WizardState};

/// Generate a random hex state token for CSRF protection.
fn generate_csrf_state() -> String {
    let mut bytes = [0u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    hex::encode(bytes)
}

/// Entry point for `nyxid setup <subcommand>`.
pub async fn run(command: SetupCommands) -> Result<()> {
    // 1. Extract auth args and build integration adapter
    let (auth, integration): (_, Box<dyn IntegrationAdapter>) = match &command {
        SetupCommands::Ha { auth, ha_url, .. } => (
            auth.clone(),
            Box::new(HaAddonAdapter {
                ha_url: ha_url.clone(),
            }),
        ),
    };

    // 2. Require authentication
    let api = match ApiClient::from_auth(&auth) {
        Ok(api) => api,
        Err(_) => {
            eprintln!("Not logged in. Please log in first:");
            eprintln!("  nyxid login");
            bail!("Authentication required. Run `nyxid login` first.");
        }
    };

    let base_url = auth.resolved_base_url()?;

    // 3. Bind to an OS-assigned ephemeral port (port 0).
    //    Bind once with std, convert to tokio — no race condition.
    let std_listener = std::net::TcpListener::bind("127.0.0.1:0")
        .context("Failed to bind local setup wizard server")?;
    std_listener
        .set_nonblocking(true)
        .context("Failed to set listener to non-blocking")?;
    let listener = tokio::net::TcpListener::from_std(std_listener)
        .context("Failed to create async listener")?;
    let port = listener.local_addr()?.port();

    // 4. Build wizard state
    let csrf_state = generate_csrf_state();
    let (completion_tx, completion_rx) = tokio::sync::oneshot::channel();

    let state = Arc::new(WizardState {
        api_client: Mutex::new(api),
        base_url,
        integration,
        csrf_state,
        completion_tx: Mutex::new(Some(completion_tx)),
        port,
    });

    // 5. Run the wizard server (opens browser, waits for completion)
    let result: CompletionResult =
        server::run_wizard_server(state, listener, port, completion_rx).await?;

    // 6. Print non-sensitive result to terminal
    eprintln!();
    eprintln!("\u{2713} Setup complete!");
    if let Some(ref name) = result.service_name {
        eprintln!("  Name: {name}");
    }
    if let Some(ref slug) = result.slug {
        eprintln!("  Slug: {slug}");
    }
    eprintln!("  Integration: {}", result.integration_name);
    eprintln!("  No credentials were printed to this terminal.");

    Ok(())
}

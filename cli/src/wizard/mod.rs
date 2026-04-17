//! CLI wizard v2 — local browser-served UI for credential setup.
//!
//! See `docs/CLI_WIZARD_V2.md` for the full spec. This module is the M1
//! skeleton: bind an axum server to `127.0.0.1:0`, serve an embedded
//! static page, open the browser, and wait for a completion or cancel
//! signal from the page before exiting.

mod server;

use anyhow::{Result, anyhow};

/// Outcome of a wizard run, returned to the caller for shaping terminal output.
#[derive(Debug, Clone)]
pub enum WizardOutcome {
    Completed(serde_json::Value),
    Cancelled,
    TimedOut,
}

/// Run the named wizard flow. For M1 only `ai-key` is accepted and renders
/// the placeholder skeleton. Future milestones route to concrete flow
/// definitions via `flows/mod.rs`.
pub async fn run_flow(flow_id: &str) -> Result<WizardOutcome> {
    match flow_id {
        "ai-key" => server::run_skeleton().await,
        other => Err(anyhow!(
            "unknown wizard flow '{other}'. In v2.0 only 'ai-key' is supported."
        )),
    }
}

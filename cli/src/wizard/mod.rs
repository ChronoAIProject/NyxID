//! CLI wizard v2 — local browser-served UI for credential setup.
//!
//! See `docs/CLI_WIZARD_V2.md` for the full spec. The module runs a local
//! axum server that hosts a hand-rolled SPA and proxies a narrow,
//! method+path allowlist of requests to the NyxID backend with the user's
//! bearer token attached server-side.

mod server;

use anyhow::{Result, anyhow};

/// Runtime context the wizard needs to proxy to the NyxID backend.
///
/// The `base_url_root` is the user-facing NyxID origin (e.g.
/// `https://auth.nyxid.dev`) with no trailing slash and no `/api/v1`
/// suffix. `access_token` is the user's session bearer, loaded from
/// `~/.nyxid/` by `ApiClient::from_auth` and handed in here.
#[derive(Debug, Clone)]
pub struct ProxyContext {
    pub base_url_root: String,
    pub access_token: String,
}

/// Outcome of a wizard run, returned to the caller for shaping terminal output.
#[derive(Debug, Clone)]
pub enum WizardOutcome {
    Completed(serde_json::Value),
    Cancelled,
    TimedOut,
}

/// Run the named wizard flow. In v2.0 only `ai-key` is accepted. The
/// `proxy` argument carries the NyxID origin + bearer token so the wizard
/// can attach auth to the narrow allowlist of forwarded endpoints.
pub async fn run_flow(flow_id: &str, proxy: ProxyContext) -> Result<WizardOutcome> {
    match flow_id {
        "ai-key" => server::run_flow(server::FlowKind::AiKey, proxy).await,
        other => Err(anyhow!(
            "unknown wizard flow '{other}'. In v2.0 only 'ai-key' is supported."
        )),
    }
}

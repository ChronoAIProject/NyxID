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

/// Shared entry point for the `ai-key` wizard: resolves auth from the
/// standard `AuthArgs`, runs the flow, prints the §3.2 terminal summary
/// on success, and `process::exit(1)` on cancel/timeout.
///
/// Called by both `nyxid service add` (bare, interactive) and the
/// discoverability alias `nyxid wizard ai-key`.
pub async fn run_ai_key_wizard(auth: &crate::cli::AuthArgs) -> Result<()> {
    let base_url = auth.resolved_base_url()?;
    let access_token = crate::auth::resolve_access_token(auth)?;
    let base_url_root = base_url.trim_end_matches('/').to_string();
    let proxy = ProxyContext {
        base_url_root,
        access_token,
    };

    match run_flow("ai-key", proxy).await? {
        WizardOutcome::Completed(body) => {
            print_wizard_summary(&body, &base_url);
            Ok(())
        }
        WizardOutcome::Cancelled => {
            eprintln!("✗ Wizard cancelled. No service was created.");
            std::process::exit(1);
        }
        WizardOutcome::TimedOut => {
            eprintln!("✗ Wizard timed out. No service was created.");
            eprintln!("  Tip: for scripted use, pass a slug and --credential-env:");
            eprintln!("       nyxid service add <slug> --credential-env VAR --label <label>");
            std::process::exit(1);
        }
    }
}

/// Format the happy-path completion summary per docs/CLI_WIZARD_V2.md §3.2.
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
            eprintln!("✓ Wizard completed (no service created).");
        }
    }
}

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
/// `~/.nyxid/` by `ApiClient::from_auth` and handed in here. `profile`
/// is needed so the proxy can refresh the access token on 401 using
/// the saved refresh token for the correct profile.
#[derive(Debug, Clone)]
pub struct ProxyContext {
    pub base_url_root: String,
    pub access_token: String,
    pub profile: Option<String>,
}

/// CLI-supplied prefill for the wizard form. Any field set here is
/// encoded into the browser URL as a query parameter and picked up by
/// `wizard.js` on page load — it pre-selects the catalog card, jumps
/// to Step 2, and fills matching inputs.
#[derive(Debug, Clone, Default)]
pub struct WizardPrefill {
    pub slug: Option<String>,
    pub label: Option<String>,
    pub via_node: Option<String>,
    pub endpoint_url: Option<String>,
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
/// `prefill` seeds the browser UI with CLI-supplied values.
pub async fn run_flow(
    flow_id: &str,
    proxy: ProxyContext,
    prefill: WizardPrefill,
) -> Result<WizardOutcome> {
    match flow_id {
        "ai-key" => server::run_flow(server::FlowKind::AiKey, proxy, prefill).await,
        other => Err(anyhow!(
            "unknown wizard flow '{other}'. In v2.0 only 'ai-key' is supported."
        )),
    }
}

/// Shared entry point for the `ai-key` wizard: resolves auth from the
/// standard `AuthArgs`, runs the flow, prints the §3.2 terminal summary
/// on success, and `process::exit(1)` on cancel/timeout.
///
/// `prefill` carries any CLI-supplied values the user typed on the
/// command line (slug, label, via-node, endpoint-url) — the wizard
/// opens with those pre-selected/pre-filled.
pub async fn run_ai_key_wizard(auth: &crate::cli::AuthArgs, prefill: WizardPrefill) -> Result<()> {
    let base_url = auth.resolved_base_url()?;
    let access_token = crate::auth::resolve_access_token(auth)?;
    let base_url_root = base_url.trim_end_matches('/').to_string();
    let proxy = ProxyContext {
        base_url_root,
        access_token,
        profile: auth.profile.clone(),
    };

    match run_flow("ai-key", proxy, prefill).await? {
        WizardOutcome::Completed(body) => {
            attract_terminal("NyxID wizard complete");
            print_wizard_summary(&body, &base_url);
            Ok(())
        }
        WizardOutcome::Cancelled => {
            attract_terminal("NyxID wizard cancelled");
            eprintln!("✗ Wizard cancelled. No service was created.");
            std::process::exit(1);
        }
        WizardOutcome::TimedOut => {
            attract_terminal("NyxID wizard timed out");
            eprintln!("✗ Wizard timed out. No service was created.");
            eprintln!("  Tip: for scripted use, pass a slug and --credential-env:");
            eprintln!("       nyxid service add <slug> --credential-env VAR --label <label>");
            std::process::exit(1);
        }
    }
}

/// Ring a terminal bell + emit the OSC-9 notification sequence so the
/// terminal app (iTerm2, WezTerm, Kitty, and many others) pops a user
/// attention cue when the user's browser-side wizard completes. No-op
/// where the terminal ignores either sequence.
///
/// - `\x07`           BEL — dock/tab bounce, notification badge in most apps
/// - `ESC ] 9 ; … \x07` OSC-9 growl-style notification with the given title
///
/// We emit both unconditionally; the escape is short and harmless even on
/// terminals that don't recognise it.
fn attract_terminal(msg: &str) {
    if !std::io::IsTerminal::is_terminal(&std::io::stderr()) {
        return;
    }
    use std::io::Write;
    let mut err = std::io::stderr().lock();
    // BEL + OSC-9 notification + trailing BEL to close the OSC sequence.
    let _ = write!(err, "\x07\x1b]9;{msg}\x07");
    let _ = err.flush();
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

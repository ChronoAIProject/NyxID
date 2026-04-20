//! CLI wizard — local browser-served UI for credential setup and one-time
//! secret display ("DisplayOnce").
//!
//! v2 (`docs/CLI_WIZARD_V2.md`) shipped `nyxid service add` — the wizard
//! collects a secret in a local browser so it never enters the terminal /
//! LLM context.
//!
//! v3 (`docs/CLI_WIZARD_V3.md`) extends the same primitive to flows where
//! the BACKEND generates the secret and we have to display it exactly
//! once: `nyxid api-key rotate` and `nyxid node rotate-token`. These
//! reuse the v2 axum server, CSP, CSRF, Origin pin, heartbeat, and
//! placeholder-cleanup machinery, with three additions:
//!   - typed `RotationAckPayload` on the `/complete` body so a typo in
//!     the browser can't smuggle a secret through `serde_json::Value`,
//!   - field-allowlist printer (`print_rotation_summary`) that only ever
//!     reads `id` / `name` / `message`,
//!   - longer heartbeat-dead window for rotation flows so the user has
//!     time to copy the secret into a password manager.

mod server;

use anyhow::{Result, anyhow};
use serde::Deserialize;

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

/// CLI-supplied prefill for the ai-key wizard. Any field set here is
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

/// CLI-supplied prefill for rotation flows (`api-key rotate`,
/// `node rotate-token`). The CLI resolves any `id-or-name` argument
/// to a canonical id BEFORE constructing this so the browser only ever
/// sees the resolved id; `display_name` powers the confirm-rotate
/// panel ("Rotate API key 'foo'?").
#[derive(Debug, Clone, Default)]
pub struct RotatePrefill {
    pub resource_id: String,
    pub display_name: String,
}

/// Typed completion payload posted by the browser when the user clicks
/// "I have saved this — close" on the DisplayOnce panel. Two guards live
/// in this struct:
///   - `#[serde(deny_unknown_fields)]` rejects bodies that contain
///     anything beyond these two fields, so a buggy or compromised
///     wizard page can't smuggle `full_key`/`auth_token`/`signing_secret`
///     into the CLI's process via `/api/proxy/complete`.
///   - the `Debug` impl can only print fields the struct holds, so a
///     future `tracing::debug!("outcome: {:?}", outcome)` is safe.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RotationAckPayload {
    pub acknowledged: bool,
    pub resource_id: String,
}

/// Outcome of a wizard run, returned to the caller for shaping terminal
/// output. Variants are flow-specific so the leak surface stays narrow:
/// the ai-key flow keeps its existing untyped body (a slug+label+url
/// summary nobody calls a secret); rotation flows MUST go through the
/// typed `RotationAckPayload` so the printer never sees raw bytes from
/// the browser.
#[derive(Debug, Clone)]
pub enum WizardOutcome {
    AiKeyCompleted(serde_json::Value),
    RotationAcknowledged(RotationAckPayload),
    Cancelled,
    TimedOut,
}

/// Run the named wizard flow. Used by the legacy `service add` entry
/// (`flow_id = "ai-key"`). Rotation flows have their own typed entry
/// points (`run_api_key_rotate_wizard`, `run_node_rotate_token_wizard`)
/// because their prefill shape and outcome shape differ.
pub async fn run_flow(
    flow_id: &str,
    proxy: ProxyContext,
    prefill: WizardPrefill,
) -> Result<WizardOutcome> {
    match flow_id {
        "ai-key" => {
            server::run_flow(
                server::FlowKind::AiKey,
                proxy,
                server::PrefillData::AiKey(prefill),
            )
            .await
        }
        other => Err(anyhow!(
            "unknown wizard flow '{other}'. Use one of: ai-key (or call run_*_rotate_wizard for rotation flows)."
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
        WizardOutcome::AiKeyCompleted(body) => {
            attract_terminal("NyxID wizard complete");
            print_wizard_summary(&body, &base_url);
            Ok(())
        }
        WizardOutcome::RotationAcknowledged(_) => {
            // Defensive: rotation outcome can't reach the ai-key handler
            // (server::handle_complete dispatches by FlowKind), but if it
            // ever did we'd refuse to print anything from it.
            Err(anyhow!(
                "internal: ai-key wizard returned a rotation outcome (flow dispatch broken)"
            ))
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

/// Shared entry point for the `api-key rotate` wizard. Wizard mode is
/// gated by the caller (`cli/src/commands/api_key.rs` Rotate arm) using
/// `is_wizard_eligible`. The CLI resolves id-or-name → canonical id and
/// fetches the display name BEFORE handing off, so the browser opens
/// with all the metadata it needs.
pub async fn run_api_key_rotate_wizard(
    auth: &crate::cli::AuthArgs,
    prefill: RotatePrefill,
) -> Result<()> {
    run_rotation_wizard(
        auth,
        server::FlowKind::ApiKeyRotate,
        prefill,
        |display, id| {
            eprintln!("✓ API key '{display}' rotated. New value was shown in the browser.");
            eprintln!("  ID: {id}");
            eprintln!("  The previous key is now revoked.");
        },
        "API key",
        "nyxid api-key rotate <id>",
    )
    .await
}

/// Shared entry point for the `node rotate-token` wizard.
pub async fn run_node_rotate_token_wizard(
    auth: &crate::cli::AuthArgs,
    prefill: RotatePrefill,
) -> Result<()> {
    run_rotation_wizard(
        auth,
        server::FlowKind::NodeRotateToken,
        prefill,
        |display, id| {
            eprintln!(
                "✓ Node '{display}' token rotated. New auth token + signing secret were shown in the browser."
            );
            eprintln!("  ID: {id}");
            eprintln!("  Restart the node agent with the new credentials:");
            eprintln!("    nyxid node rekey --auth-token <token-from-browser> --signing-secret <hex-from-browser>");
            eprintln!("  The previous token is now revoked.");
        },
        "Node token",
        "nyxid node rotate-token <id>",
    )
    .await
}

/// Shared rotation wizard runner. Owns the shape that's common across
/// all DisplayOnce-shaped flows: build the proxy context, run the flow,
/// print a flow-specific success summary on ack, print rotation-aware
/// cancel/timeout summaries on the failure paths.
///
/// The `success_summary` closure is given the resolved `display_name`
/// and `resource_id` and is responsible for printing the post-rotation
/// terminal output. CRITICAL: this closure is the ONLY thing that
/// formats output for the user, and it MUST NOT read any field from the
/// `RotationAckPayload` other than `resource_id` (already passed as an
/// argument). The payload's `Debug` impl is also safe — `deny_unknown_fields`
/// guarantees only `acknowledged` + `resource_id` ever land in it.
async fn run_rotation_wizard(
    auth: &crate::cli::AuthArgs,
    flow_kind: server::FlowKind,
    prefill: RotatePrefill,
    success_summary: impl FnOnce(&str, &str),
    resource_label: &'static str,
    rerun_command: &'static str,
) -> Result<()> {
    let base_url = auth.resolved_base_url()?;
    let access_token = crate::auth::resolve_access_token(auth)?;
    let base_url_root = base_url.trim_end_matches('/').to_string();
    let proxy = ProxyContext {
        base_url_root,
        access_token,
        profile: auth.profile.clone(),
    };
    let display_name_for_summary = prefill.display_name.clone();

    let outcome = server::run_flow(flow_kind, proxy, server::PrefillData::Rotate(prefill)).await?;

    match outcome {
        WizardOutcome::RotationAcknowledged(ack) => {
            attract_terminal("NyxID wizard complete");
            // Field allowlist: ONLY `display_name_for_summary` (CLI-side
            // resolved before the wizard ran) + `ack.resource_id` (echoed
            // back from the browser, validated against the prefilled id).
            // Never `format!("{ack:?}")`, never `serde_json::to_string(&ack)`.
            success_summary(&display_name_for_summary, &ack.resource_id);
            Ok(())
        }
        WizardOutcome::AiKeyCompleted(_) => {
            // Defensive: server dispatch shouldn't produce this for a
            // rotation flow.
            Err(anyhow!(
                "internal: rotation wizard returned an ai-key outcome (flow dispatch broken)"
            ))
        }
        WizardOutcome::Cancelled => {
            attract_terminal("NyxID wizard cancelled");
            eprintln!("✗ Rotation cancelled.");
            eprintln!(
                "  If the new {resource_label} value was shown in the browser, the rotation already happened on the server."
            );
            eprintln!(
                "  If you saved it, you're done. If not, run `{rerun_command}` again to issue a fresh value."
            );
            std::process::exit(1);
        }
        WizardOutcome::TimedOut => {
            attract_terminal("NyxID wizard timed out");
            eprintln!("✗ Rotation wizard timed out.");
            eprintln!(
                "  If the new {resource_label} value was shown in the browser, the rotation already happened on the server."
            );
            eprintln!(
                "  If you didn't save it, run `{rerun_command}` again to issue a fresh value."
            );
            std::process::exit(1);
        }
    }
}

/// Returns true when the CLI is running somewhere we can reasonably
/// open a local browser for the wizard. False on SSH / explicit opt-out
/// / Linux without DISPLAY/WAYLAND, in which case the caller falls
/// through to the existing scripted (non-wizard) path.
///
/// Mirrors `cli::commands::service::is_headless_environment` (kept
/// private there to avoid widening the public surface) but inverts the
/// boolean — `true` means "wizard is fine to launch."
pub fn is_wizard_eligible() -> bool {
    if std::env::var_os("NYXID_NO_WIZARD").is_some() {
        return false;
    }
    if std::env::var_os("SSH_CONNECTION").is_some() || std::env::var_os("SSH_TTY").is_some() {
        return false;
    }
    #[cfg(target_os = "linux")]
    {
        if std::env::var_os("DISPLAY").is_none() && std::env::var_os("WAYLAND_DISPLAY").is_none() {
            return false;
        }
    }
    true
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

/// Format the happy-path completion summary per docs/CLI_WIZARD_V2.md §3.2
/// (ai-key flow only — rotation flows have their own per-flow summary
/// closure inside `run_rotation_wizard`).
///
/// Uses an EXPLICIT field allowlist (`slug`, `label`, `proxy_url`) — never
/// `to_string(&body)`, never `{:?}` on the body, never reads anything
/// shaped like a secret. This matches v2's existing behavior and is
/// preserved here so `WizardOutcome::AiKeyCompleted(Value)` stays narrow.
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

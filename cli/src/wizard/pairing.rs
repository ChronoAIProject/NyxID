//! Remote pairing client (Mode B wizard flow).
//!
//! When [`super::is_wizard_eligible`] returns false — SSH, no DISPLAY,
//! explicit opt-out — the CLI can't open a local browser, so instead
//! of spawning an axum server on `127.0.0.1` we hand the user a short
//! code + a pair URL on a NyxID-hosted page. The user opens that URL
//! on their phone or laptop, enters the code, and completes the same
//! DisplayOnce wizard on the frontend. The CLI polls the backend for
//! the typed ack and then returns a `WizardOutcome`.
//!
//! The ack payload shapes reuse the exact structs from `super` so the
//! per-flow printers don't care whether the wizard ran locally or
//! remotely.

use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::{
    AiKeyPairingAckPayload, ApiKeyCreateAckPayload, ApiKeyCreatePrefill,
    DeveloperAppCreateAckPayload, DeveloperAppCreatePrefill, MfaSetupAckPayload, MfaSetupPrefill,
    NodeRegisterAckPayload, NodeRegisterPrefill, RotatePrefill, RotationAckPayload,
    ServiceAccountCreateAckPayload, ServiceAccountCreatePrefill, WizardOutcome, WizardPrefill,
};

/// Which flow the CLI is pairing for. One variant per wizard kind
/// the frontend pair page supports. `AiKey` (service-add) produces a
/// user-service record and echoes back non-secret identifiers; the
/// four DisplayOnce variants mint a one-time secret server-side that
/// the browser displays and the CLI never sees.
#[derive(Debug, Clone, Copy)]
pub enum PairingFlow {
    AiKey,
    ApiKeyCreate,
    ApiKeyRotate,
    NodeRegisterToken,
    NodeRotateToken,
    ServiceAccountCreate,
    ServiceAccountRotateSecret,
    DeveloperAppCreate,
    DeveloperAppRotateSecret,
    MfaSetup,
}

impl PairingFlow {
    fn kind(self) -> &'static str {
        match self {
            Self::AiKey => "ai-key",
            Self::ApiKeyCreate => "api-key-create",
            Self::ApiKeyRotate => "api-key-rotate",
            Self::NodeRegisterToken => "node-register-token",
            Self::NodeRotateToken => "node-rotate-token",
            Self::ServiceAccountCreate => "service-account-create",
            Self::ServiceAccountRotateSecret => "service-account-rotate-secret",
            Self::DeveloperAppCreate => "developer-app-create",
            Self::DeveloperAppRotateSecret => "developer-app-rotate-secret",
            Self::MfaSetup => "mfa-setup",
        }
    }

    /// Reverse mapping used by `pairing resume` to rehydrate the
    /// flow type from the server-reported `kind` string. Returns
    /// `None` when the kind isn't recognized — the resume command
    /// prints an "upgrade CLI" hint in that case rather than blindly
    /// picking the wrong ack parser.
    pub fn from_kind_str(kind: &str) -> Option<Self> {
        match kind {
            "ai-key" => Some(Self::AiKey),
            "api-key-create" => Some(Self::ApiKeyCreate),
            "api-key-rotate" => Some(Self::ApiKeyRotate),
            "node-register-token" => Some(Self::NodeRegisterToken),
            "node-rotate-token" => Some(Self::NodeRotateToken),
            "service-account-create" => Some(Self::ServiceAccountCreate),
            "service-account-rotate-secret" => Some(Self::ServiceAccountRotateSecret),
            "developer-app-create" => Some(Self::DeveloperAppCreate),
            "developer-app-rotate-secret" => Some(Self::DeveloperAppRotateSecret),
            "mfa-setup" => Some(Self::MfaSetup),
            _ => None,
        }
    }
}

/// Shared URL builder used by both the wait-and-poll and no-wait
/// print paths. Centralized so the `?code=` query-param encoding
/// lives in one spot.
fn pair_url_with_code(resp: &CreateResp) -> String {
    if resp.pair_url.contains('?') {
        format!("{}&code={}", resp.pair_url, urlencoding_minimal(&resp.code))
    } else {
        format!("{}?code={}", resp.pair_url, urlencoding_minimal(&resp.code))
    }
}

/// Payload carried in the `POST /api/v1/cli-pairings` body. Matches
/// the backend's `CreatePairingRequest` shape.
#[derive(Debug, Serialize)]
struct CreateReq<'a> {
    kind: &'a str,
    prefill: Value,
}

/// Shape of `POST /api/v1/cli-pairings` response.
#[derive(Debug, Deserialize)]
struct CreateResp {
    id: String,
    code: String,
    pair_url: String,
    #[serde(default)]
    #[allow(dead_code)]
    poll_url: String,
    expires_at: String,
}

/// Shape of `GET /api/v1/cli-pairings/{id}/poll` response. Flattened
/// so `{status, kind, expires_at, ack?}` all live at the top level;
/// see `PollResponse` in the backend service. `kind` lets the resume
/// command dispatch per-kind summaries without a second round-trip.
#[derive(Debug, Deserialize)]
struct PollResp {
    #[serde(flatten)]
    status: PollStatus,
    /// Read by `resume_kind` so `pairing resume --output json` can
    /// distinguish api-key-rotate from node-rotate-token (both yield
    /// the same `WizardOutcome::RotationAcknowledged`).
    #[serde(default)]
    kind: String,
    /// Currently unread but surfaced by the backend for future UX
    /// that wants to display "expires at" in resume output.
    #[serde(default)]
    #[allow(dead_code)]
    expires_at: String,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum PollStatus {
    Pending,
    Claimed,
    Completed { ack: Value },
    Cancelled,
    Expired,
}

// ── poll cadence ─────────────────────────────────────────────────────

/// Starting poll interval. Short enough that a user who enters the
/// code + completes the wizard sees the CLI exit in a few seconds.
const POLL_MIN: Duration = Duration::from_secs(2);

/// Upper bound on interval when the server signals load. RFC 8628 §3.5
/// recommends backing off; we cap so we still notice completion
/// reasonably quickly.
const POLL_MAX: Duration = Duration::from_secs(10);

/// Hard ceiling on total wait time. Slightly longer than the default
/// server-side 900s TTL so the CLI surfaces the TTL-expired state
/// itself rather than hanging.
const OVERALL_TIMEOUT: Duration = Duration::from_secs(960);

/// Run the pairing flow for the given DisplayOnce wizard kind. Prints
/// the code + URL on stdout/stderr, polls until the user completes or
/// we hit a terminal state, and returns a `WizardOutcome` in the same
/// shape the local-server wizard produces.
pub async fn run_display_once_pairing(
    auth: &crate::cli::AuthArgs,
    flow: PairingFlow,
    prefill: Value,
) -> Result<WizardOutcome> {
    let mut api = crate::api::ApiClient::from_auth(auth)?;
    let created = create_pairing(&mut api, flow, prefill).await?;

    print_pairing_instructions(&created);

    let outcome = poll_until_terminal(&mut api, &created.id, flow).await;

    // Best-effort cancel ONLY on explicit user-initiated cancel
    // outcomes. We used to cancel on any non-success outcome
    // including `Err(...)`, but that meant a transient poll failure
    // (permanent auth error, 5 consecutive 5xx, etc.) would flip a
    // browser-completed pairing to Cancelled, causing the browser's
    // `/complete` to fail. Leave the record alone on errors; TTL
    // reclaims it.
    if matches!(
        outcome,
        Ok(WizardOutcome::Cancelled) | Ok(WizardOutcome::TimedOut)
    ) {
        let _: Result<Value> = api
            .post(
                &format!("/cli-pairings/{}/cancel", created.id),
                &serde_json::json!({}),
            )
            .await;
    }

    // When poll gives up with an error (auth expired, repeated
    // 5xx, network partition), the pairing record is still alive
    // on the server and the browser may yet complete. Surface
    // the resume command on stderr BEFORE returning so the user
    // has a concrete recovery path and doesn't re-run the
    // destructive create/rotate command — which would mint a
    // duplicate key/token/service while the original pairing is
    // still claimable. Written only to stderr so `--output json`
    // consumers on stdout are unaffected.
    if outcome.is_err() {
        let resume_hint = format_resume_command(auth, &created.id);
        eprintln!();
        eprintln!("  Polling stopped, but the pairing is still alive on the server.");
        eprintln!("  If the browser completes, pick up the result with:");
        eprintln!("    {resume_hint}");
        if auth.access_token.as_deref().is_some_and(|t| !t.is_empty()) {
            // Same rationale as `run_no_wait_pairing`: the resume
            // hint deliberately omits the literal bearer to keep
            // it out of transcripts, so `--access-token` callers
            // need a heads-up that the token must be re-supplied.
            eprintln!("  (This invocation used `--access-token`; re-supply it when resuming.)");
        }
        eprintln!();
    }

    outcome
}

/// `--no-wait` mode: create the pairing on the server, print the
/// code, pair URL, and resume hint, then exit without polling. The
/// calling agent can surface the URL to the user (who completes the
/// wizard in a browser) and later run `nyxid pairing resume <id>`
/// from a fresh invocation to pick up the result.
///
/// Always uses the remote-pairing transport even if a local browser
/// would otherwise be available — the local-server path requires the
/// CLI process to stay alive for the browser to post back, which
/// defeats the purpose of `--no-wait`.
pub async fn run_no_wait_pairing(
    auth: &crate::cli::AuthArgs,
    flow: PairingFlow,
    prefill: Value,
) -> Result<()> {
    let mut api = crate::api::ApiClient::from_auth(auth)?;
    let created = create_pairing(&mut api, flow, prefill).await?;

    // Build the resume hint preserving whichever `--profile` and
    // `--base-url` the user supplied on the current invocation.
    // Without these, a `--no-wait` started against a non-default
    // profile (e.g. `--profile staging`) would resume against the
    // default token store and the pairing would appear missing.
    let resume_hint = format_resume_command(auth, &created.id);
    // `--access-token <literal>` is deliberately NOT echoed in the
    // resume command (avoids leaking the bearer into transcripts
    // / PR comments), but a caller who authenticated ONLY via
    // that flag has no other credential source a fresh `resume`
    // invocation can pick up — it would fail with 401. Flag the
    // case so the stderr / JSON output can tell the caller to
    // re-supply the token when running resume.
    let access_token_required_on_resume =
        auth.access_token.as_deref().is_some_and(|t| !t.is_empty());

    eprintln!();
    eprintln!("  NyxID pairing created (--no-wait mode)");
    eprintln!("  ──────────────────────────────────────");
    eprintln!("  Complete the wizard on another device:");
    eprintln!();
    eprintln!("    1. Open:   {}", pair_url_with_code(&created));
    eprintln!("    2. Log in as yourself (if not already)");
    eprintln!("    3. Confirm the code:   {}", created.code);
    eprintln!();
    eprintln!("  To pick up the result later, run:");
    eprintln!("    {resume_hint}");
    if access_token_required_on_resume {
        eprintln!();
        eprintln!("  Note: this invocation used `--access-token`; the token is");
        eprintln!("  NOT echoed in the resume command. Re-supply it when");
        eprintln!("  resuming — either via `--access-token <value>`, via the");
        eprintln!("  env var named by `--access-token-env` (default");
        eprintln!("  NYXID_ACCESS_TOKEN), or by running `nyxid login` first.");
    }
    eprintln!();
    eprintln!("  Expires at: {}", created.expires_at);

    // Emit a machine-readable line on stdout so agents can parse the
    // pairing id without scraping stderr. `pair_url` carries the
    // `?code=ABCD-1234` query param already baked in — that way an
    // agent that forwards this payload to the user lands them on a
    // prefilled form instead of a blank claim screen (matches the
    // human-facing stderr output on line 200). The bare URL is
    // still available as `pair_url_base` for callers that want to
    // compose their own variant.
    let url_with_code = pair_url_with_code(&created);
    println!(
        "{}",
        serde_json::json!({
            "pairing_id": created.id,
            "code": created.code,
            "pair_url": url_with_code,
            "pair_url_base": created.pair_url,
            "expires_at": created.expires_at,
            // Ready-to-exec resume string that preserves the
            // caller's `--profile` / `--base-url` so agents can
            // forward it verbatim without reconstructing flags.
            "resume_cmd": resume_hint,
            // `true` when `resume_cmd` alone won't authenticate
            // successfully — the original invocation relied on
            // `--access-token`, and the bearer isn't echoed in
            // the hint. Agent wrappers should surface this to
            // the user (or re-pipe the token into the resume
            // call) instead of running `resume_cmd` blindly.
            "requires_access_token_on_resume": access_token_required_on_resume,
        })
    );
    Ok(())
}

/// Compose the exact `nyxid pairing resume ...` invocation a user
/// (or agent) should run to pick up a `--no-wait` pairing. Copies
/// the auth-context flags from the current invocation verbatim so
/// a non-default profile, an explicit `--base-url`, a non-default
/// token source, or `--output json` survives the handoff. Values
/// containing shell metacharacters are shell-quoted with a
/// conservative single-quote wrapper.
fn format_resume_command(auth: &crate::cli::AuthArgs, pairing_id: &str) -> String {
    let mut parts: Vec<String> = vec![
        "nyxid".into(),
        "pairing".into(),
        "resume".into(),
        pairing_id.to_string(),
    ];
    if let Some(profile) = auth.profile.as_deref()
        && !profile.is_empty()
    {
        parts.push("--profile".into());
        parts.push(shell_quote(profile));
    }
    if let Some(base_url) = auth.base_url.as_deref()
        && !base_url.is_empty()
    {
        parts.push("--base-url".into());
        parts.push(shell_quote(base_url));
    }
    // Preserve the token source, but NEVER echo the literal
    // bearer. `--no-wait` is targeted at agent wrappers whose
    // stdout/stderr commonly ends up in durable transcripts, PR
    // comments, or user-facing chat surfaces; embedding the raw
    // token in the printed command would promote a transient CLI
    // argument into a leaked credential. Callers who used
    // `--access-token` on the original invocation must re-supply
    // it out-of-band when running resume (environment, saved
    // session, or `--access-token` again). We carry forward
    // `--access-token-env <NAME>` since the env-var NAME itself
    // is not sensitive.
    if auth.access_token_env != "NYXID_ACCESS_TOKEN" && !auth.access_token_env.is_empty() {
        parts.push("--access-token-env".into());
        parts.push(shell_quote(&auth.access_token_env));
    }
    // Preserve `--output json` so agents that requested machine-
    // readable output get the same shape from `resume` — otherwise
    // the resume call would silently default back to the human
    // table format and break the consumer's parser.
    if matches!(auth.output, crate::cli::OutputFormat::Json) {
        parts.push("--output".into());
        parts.push("json".into());
    }
    parts.join(" ")
}

/// Cross-shell argument formatter for the printed `resume_cmd`.
///
/// The resume hint is copy-pasted into whatever shell the user happens
/// to run (POSIX sh/bash/zsh on macOS/Linux, `cmd.exe` / PowerShell on
/// Windows). Single-quote wrapping — the POSIX idiom — is NOT portable:
/// `cmd.exe` passes single quotes through literally, so a value like
/// `'staging'` becomes the literal string `'staging'` (including the
/// quotes) and the profile lookup goes looking for `'staging'` instead
/// of `staging`.
///
/// In practice the values that flow through here are constrained:
/// profile names (validated elsewhere as alphanumeric + `-_`), HTTPS
/// URLs (no shell metacharacters in normal use), and env-var names
/// (alphanumeric + `_`). The overwhelmingly common path is "no
/// quoting needed": we return the value unchanged whenever it lies
/// in a conservative safe set that both POSIX shells and `cmd.exe`
/// treat as a single literal argv token. For anything outside that
/// set we wrap in double quotes (the common subset between POSIX
/// and cmd.exe grouping) with a minimal escape for `"` and `\` —
/// good enough for the rare weirder URL without introducing shell-
/// specific quirks. Values that contain characters neither shell
/// can grouping-quote safely (unescaped newlines, raw control
/// chars, `$`-interpolation under POSIX, `%` under cmd.exe) fall
/// through as an unquoted best-effort; the copy-paste path is a
/// hint, not a contract, and operators are expected to use sane
/// profile/URL values.
fn shell_quote(value: &str) -> String {
    // Intersection of "safe unquoted" for POSIX sh and cmd.exe:
    //   POSIX sh safe: [A-Za-z0-9_./:@%+=-]
    //   cmd.exe safe:  no spaces or any of & | < > ( ) ^ " ; , = ! %
    // The `=` and `%` characters are legal in POSIX but special in
    // cmd.exe (argv split / variable expansion), so we drop them.
    // The `,` and `;` characters are legal in both as part of a
    // single argv token, but cmd.exe treats them as delimiters in
    // some contexts — safest to require quoting.
    fn is_cross_shell_safe(ch: char) -> bool {
        ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.' | '/' | ':' | '@' | '+')
    }

    if !value.is_empty() && value.chars().all(is_cross_shell_safe) {
        return value.to_string();
    }

    // Fallback: double-quote and escape `\` and `"`. This is the
    // common shape both POSIX sh and cmd.exe accept as a single
    // grouped argument. Values with `$` (POSIX interpolation) or `%`
    // (cmd.exe variable expansion) will still transform, but the
    // hint has always been best-effort and those characters don't
    // appear in our realistic input set (URL schemes, profile
    // names, env var names).
    let mut out = String::with_capacity(value.len() + 2);
    out.push('"');
    for ch in value.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            other => out.push(other),
        }
    }
    out.push('"');
    out
}

/// `nyxid pairing resume <id>`: poll an existing pairing record and
/// return its outcome once terminal. Used after `--no-wait` to pick
/// up the completed flow. The pairing's `kind` is read from the poll
/// response so we can dispatch to the right typed ack parser —
/// callers don't need to know what flow they're resuming.
/// Peek at the server-reported `kind` of a pairing record without
/// consuming the outcome. Used by `nyxid pairing resume
/// --output json` to distinguish `api-key-rotate` from
/// `node-rotate-token` — both produce `WizardOutcome::
/// RotationAcknowledged`, so without the kind string an agent
/// consumer can't tell which workflow finished. Returns the raw
/// string so future backend-added kinds flow through untouched.
pub async fn resume_kind(auth: &crate::cli::AuthArgs, pairing_id: &str) -> Result<String> {
    let mut api = crate::api::ApiClient::from_auth(auth)?;
    let path = format!("/cli-pairings/{pairing_id}/poll");
    let resp: PollResp = api
        .get(&path)
        .await
        .context("failed to read pairing kind")?;
    Ok(resp.kind)
}

pub async fn resume_pairing(
    auth: &crate::cli::AuthArgs,
    pairing_id: &str,
) -> Result<WizardOutcome> {
    let mut api = crate::api::ApiClient::from_auth(auth)?;
    let path = format!("/cli-pairings/{pairing_id}/poll");
    let initial: PollResp = api
        .get(&path)
        .await
        .context("failed to read pairing state")?;

    // Terminal states that don't need a kind-specific printer come
    // first — when the TTL sweeper has already removed the record,
    // the backend returns `{status: expired, kind: ""}`, and we
    // must NOT treat the empty kind as an "unsupported CLI version"
    // error. Cancelled is symmetric.
    match initial.status {
        PollStatus::Cancelled => return Ok(WizardOutcome::Cancelled),
        PollStatus::Expired => return Ok(WizardOutcome::TimedOut),
        _ => {}
    }

    // For non-terminal (pending/claimed) and Completed states we
    // need a recognizable kind to dispatch the right ack parser and
    // summary printer.
    let flow = PairingFlow::from_kind_str(&initial.kind).ok_or_else(|| {
        anyhow!(
            "pairing kind '{}' is not supported by this CLI version. Upgrade nyxid-cli.",
            initial.kind
        )
    })?;

    if let PollStatus::Completed { ack } = initial.status {
        return parse_ack(flow, ack);
    }

    poll_until_terminal(&mut api, pairing_id, flow).await
}

/// Centralized create-pairing call. Shared by the wait-and-poll and
/// no-wait paths so the request body + error wrapping stays single-
/// sourced.
async fn create_pairing(
    api: &mut crate::api::ApiClient,
    flow: PairingFlow,
    prefill: Value,
) -> Result<CreateResp> {
    api.post(
        "/cli-pairings",
        &CreateReq {
            kind: flow.kind(),
            prefill,
        },
    )
    .await
    .context("failed to create CLI pairing on server")
}

/// Main poll loop. Backs off 2s → 10s on consecutive `Pending`
/// responses (gentler load); resets to 2s on `Claimed` so completion
/// is detected quickly. Returns a `WizardOutcome` the caller can
/// match on.
///
/// Interrupt handling: a SIGINT (Ctrl-C) during the idle gap between
/// polls returns `WizardOutcome::Cancelled` so the wrapper can fire
/// the best-effort server-side cancel before the process exits. If
/// the signal arrives mid-HTTP-request the network call still
/// completes; that's intentional — tearing down reqwest mid-flight
/// occasionally leaks connections on macOS, and the extra ~2s
/// roundtrip isn't worth it.
async fn poll_until_terminal(
    api: &mut crate::api::ApiClient,
    id: &str,
    flow: PairingFlow,
) -> Result<WizardOutcome> {
    let path = format!("/cli-pairings/{id}/poll");
    let deadline = std::time::Instant::now() + OVERALL_TIMEOUT;
    let mut interval = POLL_MIN;
    // Count consecutive errors so a permanently-broken state (expired
    // `--access-token`, invalid `NYXID_ACCESS_TOKEN`, revoked refresh
    // token) surfaces as a real failure instead of silently retrying
    // for the full 16-minute OVERALL_TIMEOUT. Picked empirically:
    // transient network blips typically recover within 1–2 retries,
    // permanent auth failures look identical from the second call
    // onward.
    let mut consecutive_errors: u32 = 0;
    const MAX_CONSECUTIVE_ERRORS: u32 = 5;

    loop {
        if std::time::Instant::now() >= deadline {
            return Ok(WizardOutcome::TimedOut);
        }

        let resp: PollResp = match api.get(&path).await {
            Ok(r) => {
                consecutive_errors = 0;
                r
            }
            Err(e) => {
                consecutive_errors += 1;
                let is_auth = looks_like_auth_failure(&e);
                if is_auth || consecutive_errors >= MAX_CONSECUTIVE_ERRORS {
                    // Permanent-looking failure — stop polling and
                    // surface the real error. The caller's wrapper
                    // fires a best-effort cancel so the record
                    // doesn't sit in `Claimed` until TTL.
                    eprintln!();
                    eprintln!(
                        "✗ Pairing poll failed permanently after {consecutive_errors} error(s): {e}"
                    );
                    if is_auth {
                        eprintln!("  Your CLI's access token appears to be invalid or expired.");
                        eprintln!("  Re-run `nyxid login` and try again.");
                    }
                    return Err(e);
                }
                eprintln!(
                    "  ! poll failed ({consecutive_errors}/{MAX_CONSECUTIVE_ERRORS}): {e}. Retrying in {}s...",
                    interval.as_secs()
                );
                if sleep_or_interrupt(interval).await {
                    return Ok(WizardOutcome::Cancelled);
                }
                continue;
            }
        };

        match resp.status {
            PollStatus::Pending => {
                // Slow polling while nobody has entered the code;
                // cheaper on the server during long idle waits.
                interval = (interval + Duration::from_secs(2)).min(POLL_MAX);
            }
            PollStatus::Claimed => {
                // User has the wizard open — tighten the loop so we
                // notice completion within ~2s of the ack hitting
                // the server.
                interval = POLL_MIN;
            }
            PollStatus::Completed { ack } => {
                return parse_ack(flow, ack);
            }
            PollStatus::Cancelled => return Ok(WizardOutcome::Cancelled),
            PollStatus::Expired => return Ok(WizardOutcome::TimedOut),
        }
        if sleep_or_interrupt(interval).await {
            return Ok(WizardOutcome::Cancelled);
        }
    }
}

/// Best-effort detection of auth-failure errors from `ApiClient`. The
/// client already attempted a refresh once; a lingering 401/403 means
/// the access token is permanently bad in the CLI's config, and
/// polling any longer will just hit the same wall. We match on the
/// status-code fragment that `handle_response` bakes into its error
/// strings because `reqwest::Error`'s type is opaque through anyhow.
fn looks_like_auth_failure(err: &anyhow::Error) -> bool {
    let chain: Vec<String> = err.chain().map(|e| e.to_string()).collect();
    let text = chain.join(" ").to_lowercase();
    text.contains("401") || text.contains("403") || text.contains("unauthorized")
}

/// Sleep for `dur` OR wake early if the user hits Ctrl-C. Returns
/// `true` when interrupted, `false` when the sleep completed
/// naturally. The caller that sees `true` should treat the flow as
/// cancelled so the wrapper fires the server-side cancel.
async fn sleep_or_interrupt(dur: Duration) -> bool {
    tokio::select! {
        biased;
        _ = tokio::signal::ctrl_c() => {
            eprintln!();
            eprintln!("  ! Ctrl-C received — cancelling pairing...");
            true
        }
        _ = tokio::time::sleep(dur) => false,
    }
}

/// Convert the backend's opaque ack JSON into one of the typed
/// WizardOutcome variants. The exact shape depends on the flow, and
/// all variants use `deny_unknown_fields` on the underlying struct
/// (see super::mod.rs) so a buggy frontend can't smuggle a field past.
fn parse_ack(flow: PairingFlow, ack: Value) -> Result<WizardOutcome> {
    match flow {
        PairingFlow::AiKey => {
            let payload: AiKeyPairingAckPayload =
                serde_json::from_value(ack).context("invalid ai-key pairing ack from server")?;
            if !payload.acknowledged {
                return Err(anyhow!("ai-key pairing ack not acknowledged"));
            }
            Ok(WizardOutcome::AiKeyPaired(payload))
        }
        PairingFlow::ApiKeyCreate => {
            let payload: ApiKeyCreateAckPayload =
                serde_json::from_value(ack).context("invalid api-key-create ack from server")?;
            if !payload.acknowledged {
                return Err(anyhow!("api-key-create ack not acknowledged"));
            }
            Ok(WizardOutcome::ApiKeyCreateAcknowledged(payload))
        }
        PairingFlow::ApiKeyRotate
        | PairingFlow::NodeRotateToken
        | PairingFlow::ServiceAccountRotateSecret
        | PairingFlow::DeveloperAppRotateSecret => {
            let payload: RotationAckPayload =
                serde_json::from_value(ack).context("invalid rotation ack from server")?;
            if !payload.acknowledged {
                return Err(anyhow!("rotation ack not acknowledged"));
            }
            Ok(WizardOutcome::RotationAcknowledged(payload))
        }
        PairingFlow::NodeRegisterToken => {
            let payload: NodeRegisterAckPayload = serde_json::from_value(ack)
                .context("invalid node-register-token ack from server")?;
            if !payload.acknowledged {
                return Err(anyhow!("node-register-token ack not acknowledged"));
            }
            Ok(WizardOutcome::NodeRegisterAcknowledged(payload))
        }
        PairingFlow::ServiceAccountCreate => {
            let payload: ServiceAccountCreateAckPayload = serde_json::from_value(ack)
                .context("invalid service-account-create ack from server")?;
            if !payload.acknowledged {
                return Err(anyhow!("service-account-create ack not acknowledged"));
            }
            Ok(WizardOutcome::ServiceAccountCreateAcknowledged(payload))
        }
        PairingFlow::DeveloperAppCreate => {
            let payload: DeveloperAppCreateAckPayload = serde_json::from_value(ack)
                .context("invalid developer-app-create ack from server")?;
            if !payload.acknowledged {
                return Err(anyhow!("developer-app-create ack not acknowledged"));
            }
            Ok(WizardOutcome::DeveloperAppCreateAcknowledged(payload))
        }
        PairingFlow::MfaSetup => {
            let payload: MfaSetupAckPayload =
                serde_json::from_value(ack).context("invalid mfa-setup ack from server")?;
            if !payload.acknowledged {
                return Err(anyhow!("mfa-setup ack not acknowledged"));
            }
            Ok(WizardOutcome::MfaSetupAcknowledged(payload))
        }
    }
}

/// Render the code + pair URL to stderr in a shape that is:
///   - visible to a human operator,
///   - easy for an AI agent to parse and relay to its user, and
///   - emitted on stderr so it doesn't corrupt a scripted pipe on
///     stdout (the caller's JSON/table output still goes to stdout).
fn print_pairing_instructions(resp: &CreateResp) {
    // Embed the code as a `?code=` query param so users who click the
    // URL from a device where they're already logged in skip the "type
    // the code into the field" step. See `pair_url_with_code`.
    let url = pair_url_with_code(resp);

    eprintln!();
    eprintln!("  NyxID remote pairing required");
    eprintln!("  ──────────────────────────────");
    eprintln!("  This environment can't open a browser locally (SSH / container / no DISPLAY).");
    eprintln!("  Complete the wizard on another device:");
    eprintln!();
    eprintln!("    1. Open:   {url}");
    eprintln!("    2. Log in as yourself (if not already)");
    eprintln!(
        "    3. Confirm the code:   {}   (pre-filled from the URL; retype if it doesn't match)",
        resp.code
    );
    eprintln!();
    eprintln!("  Expires at: {}", resp.expires_at);
    eprintln!("  Waiting for the pairing to be completed...");
    eprintln!();
}

/// Minimal URL-safe encoding for the pairing code. The code uses only
/// Crockford-style alphanumerics + an optional dash, none of which need
/// percent-encoding per RFC 3986 — but we keep the helper explicit so
/// a future alphabet change doesn't silently break URL parsing.
fn urlencoding_minimal(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c.to_string()
            } else {
                format!("%{:02X}", c as u32)
            }
        })
        .collect()
}

// ── helpers exposed to command entry points ─────────────────────────

/// Serialize an `ApiKeyCreatePrefill` into the JSON shape the frontend
/// wizard expects. Kept here (not on the struct) because the CLI-side
/// struct is a snapshot for clap parsing; the on-wire shape is a
/// separate concern.
pub fn prefill_api_key_create(p: &ApiKeyCreatePrefill) -> Value {
    let mut obj = serde_json::Map::new();
    if let Some(v) = &p.name {
        obj.insert("name".into(), Value::String(v.clone()));
    }
    if let Some(v) = &p.platform {
        obj.insert("platform".into(), Value::String(v.clone()));
    }
    if let Some(v) = &p.scopes {
        obj.insert("scopes".into(), Value::String(v.clone()));
    }
    if let Some(v) = p.expires_in_days {
        obj.insert(
            "expires_in_days".into(),
            Value::Number(serde_json::Number::from(v)),
        );
    }
    if p.allow_all_services {
        obj.insert("allow_all_services".into(), Value::Bool(true));
    }
    if p.allow_all_nodes {
        obj.insert("allow_all_nodes".into(), Value::Bool(true));
    }
    if let Some(v) = &p.allowed_services_csv {
        obj.insert("allowed_services_csv".into(), Value::String(v.clone()));
    }
    if let Some(v) = &p.allowed_nodes_csv {
        obj.insert("allowed_nodes_csv".into(), Value::String(v.clone()));
    }
    if let Some(v) = &p.callback_url {
        obj.insert("callback_url".into(), Value::String(v.clone()));
    }
    if let Some(v) = &p.org_id {
        obj.insert("org_id".into(), Value::String(v.clone()));
    }
    Value::Object(obj)
}

pub fn prefill_rotate(p: &RotatePrefill) -> Value {
    serde_json::json!({
        "resource_id": p.resource_id,
        "display_name": p.display_name,
    })
}

/// Map a `WizardPrefill` (ai-key flow, local-server shape) onto the
/// JSON payload the frontend pair page expects. Mirrors the
/// `prefill_query` encoding the local server uses for its URL
/// query-string, so the frontend can consume both shapes identically.
pub fn prefill_ai_key(p: &WizardPrefill) -> Value {
    let mut obj = serde_json::Map::new();
    if let Some(v) = &p.slug {
        obj.insert("slug".into(), Value::String(v.clone()));
    }
    if let Some(v) = &p.label {
        obj.insert("label".into(), Value::String(v.clone()));
    }
    if let Some(v) = &p.via_node {
        obj.insert("via_node".into(), Value::String(v.clone()));
    }
    if let Some(v) = &p.org {
        obj.insert("org_id".into(), Value::String(v.clone()));
    }
    if let Some(v) = &p.endpoint_url {
        obj.insert("endpoint_url".into(), Value::String(v.clone()));
    }
    // Issue #414: custom-mode fields. Only emit `custom: true` when
    // it's set so existing catalog-flow pairing records stay
    // byte-identical (for the freshness test + any pairing-record
    // log consumers). The other three are only meaningful in custom
    // mode and follow the same `Some => emit` pattern as the
    // catalog-flow fields above.
    if p.custom {
        obj.insert("custom".into(), Value::Bool(true));
    }
    if let Some(v) = &p.custom_slug {
        obj.insert("custom_slug".into(), Value::String(v.clone()));
    }
    if let Some(v) = &p.auth_method {
        obj.insert("auth_method".into(), Value::String(v.clone()));
    }
    if let Some(v) = &p.auth_key_name {
        obj.insert("auth_key_name".into(), Value::String(v.clone()));
    }
    Value::Object(obj)
}

pub fn prefill_node_register(p: &NodeRegisterPrefill) -> Value {
    let mut obj = serde_json::Map::new();
    if let Some(v) = &p.name {
        obj.insert("name".into(), Value::String(v.clone()));
    }
    Value::Object(obj)
}

pub fn prefill_service_account_create(p: &ServiceAccountCreatePrefill) -> Value {
    let mut obj = serde_json::Map::new();
    if let Some(v) = &p.name {
        obj.insert("name".into(), Value::String(v.clone()));
    }
    if let Some(v) = &p.scopes {
        obj.insert("scopes".into(), Value::String(v.clone()));
    }
    if let Some(v) = &p.description {
        obj.insert("description".into(), Value::String(v.clone()));
    }
    if let Some(v) = p.rate_limit_override {
        obj.insert(
            "rate_limit_override".into(),
            Value::Number(serde_json::Number::from(v)),
        );
    }
    if let Some(v) = &p.role_ids_csv {
        obj.insert("role_ids_csv".into(), Value::String(v.clone()));
    }
    if let Some(v) = &p.org_id {
        obj.insert("org_id".into(), Value::String(v.clone()));
    }
    Value::Object(obj)
}

pub fn prefill_developer_app_create(p: &DeveloperAppCreatePrefill) -> Value {
    let mut obj = serde_json::Map::new();
    if let Some(v) = &p.name {
        obj.insert("name".into(), Value::String(v.clone()));
    }
    if !p.redirect_uris.is_empty() {
        let arr: Vec<Value> = p
            .redirect_uris
            .iter()
            .filter(|s| !s.is_empty())
            .map(|s| Value::String(s.clone()))
            .collect();
        if !arr.is_empty() {
            obj.insert("redirect_uris".into(), Value::Array(arr));
        }
    }
    if let Some(v) = &p.allowed_scopes {
        obj.insert("allowed_scopes".into(), Value::String(v.clone()));
    }
    if let Some(v) = &p.delegation_scopes {
        obj.insert("delegation_scopes".into(), Value::String(v.clone()));
    }
    if let Some(v) = p.broker_capability {
        obj.insert("broker_capability".into(), Value::Bool(v));
    }
    if let Some(v) = &p.org_id {
        obj.insert("org_id".into(), Value::String(v.clone()));
    }
    Value::Object(obj)
}

pub fn prefill_mfa_setup(_p: &MfaSetupPrefill) -> Value {
    Value::Object(serde_json::Map::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flow_kind_slugs_match_backend() {
        // These string literals must match the FlowKind slugs used on
        // the frontend / server dispatch. Keep in sync if either side
        // adds a flow.
        assert_eq!(PairingFlow::AiKey.kind(), "ai-key");
        assert_eq!(PairingFlow::ApiKeyCreate.kind(), "api-key-create");
        assert_eq!(PairingFlow::ApiKeyRotate.kind(), "api-key-rotate");
        assert_eq!(PairingFlow::NodeRegisterToken.kind(), "node-register-token");
        assert_eq!(PairingFlow::NodeRotateToken.kind(), "node-rotate-token");
        assert_eq!(
            PairingFlow::ServiceAccountCreate.kind(),
            "service-account-create"
        );
        assert_eq!(
            PairingFlow::ServiceAccountRotateSecret.kind(),
            "service-account-rotate-secret"
        );
        assert_eq!(
            PairingFlow::DeveloperAppCreate.kind(),
            "developer-app-create"
        );
        assert_eq!(
            PairingFlow::DeveloperAppRotateSecret.kind(),
            "developer-app-rotate-secret"
        );
        assert_eq!(PairingFlow::MfaSetup.kind(), "mfa-setup");
    }

    #[test]
    fn from_kind_str_round_trips_all_slugs() {
        for variant in [
            PairingFlow::AiKey,
            PairingFlow::ApiKeyCreate,
            PairingFlow::ApiKeyRotate,
            PairingFlow::NodeRegisterToken,
            PairingFlow::NodeRotateToken,
            PairingFlow::ServiceAccountCreate,
            PairingFlow::ServiceAccountRotateSecret,
            PairingFlow::DeveloperAppCreate,
            PairingFlow::DeveloperAppRotateSecret,
            PairingFlow::MfaSetup,
        ] {
            let slug = variant.kind();
            let back = PairingFlow::from_kind_str(slug)
                .unwrap_or_else(|| panic!("from_kind_str rejected its own slug: {slug}"));
            assert_eq!(back.kind(), slug, "round-trip slug mismatch");
        }
        assert!(PairingFlow::from_kind_str("unknown-kind").is_none());
    }

    #[test]
    fn parse_ack_service_account_create_rejects_unknown_fields() {
        let ack = serde_json::json!({
            "acknowledged": true,
            "service_account_id": "abc",
            "client_secret": "leak"
        });
        let outcome = parse_ack(PairingFlow::ServiceAccountCreate, ack);
        assert!(outcome.is_err(), "deny_unknown_fields should reject");
    }

    #[test]
    fn parse_ack_developer_app_create_rejects_unknown_fields() {
        let ack = serde_json::json!({
            "acknowledged": true,
            "developer_app_id": "abc",
            "client_secret": "leak"
        });
        let outcome = parse_ack(PairingFlow::DeveloperAppCreate, ack);
        assert!(outcome.is_err(), "deny_unknown_fields should reject");
    }

    #[test]
    fn parse_ack_mfa_setup_rejects_unknown_fields() {
        let ack = serde_json::json!({
            "acknowledged": true,
            "factor_id": "abc",
            "totp_secret": "leak"
        });
        let outcome = parse_ack(PairingFlow::MfaSetup, ack);
        assert!(outcome.is_err(), "deny_unknown_fields should reject");
    }

    #[test]
    fn rotation_ack_accepts_new_rotation_kinds() {
        for flow in [
            PairingFlow::ServiceAccountRotateSecret,
            PairingFlow::DeveloperAppRotateSecret,
        ] {
            let ack = serde_json::json!({
                "acknowledged": true,
                "resource_id": "abc",
            });
            let outcome = parse_ack(flow, ack);
            assert!(outcome.is_ok(), "rotation ack should be accepted");
        }
    }

    #[test]
    fn prefill_service_account_create_omits_none_fields() {
        let p = ServiceAccountCreatePrefill {
            name: Some("ci-bot".into()),
            ..Default::default()
        };
        let v = prefill_service_account_create(&p);
        let obj = v.as_object().expect("object");
        assert_eq!(obj.get("name").and_then(|x| x.as_str()), Some("ci-bot"));
        assert!(!obj.contains_key("scopes"));
        assert!(!obj.contains_key("description"));
        assert!(!obj.contains_key("rate_limit_override"));
    }

    #[test]
    fn prefill_developer_app_create_emits_redirect_uri_array() {
        let p = DeveloperAppCreatePrefill {
            name: Some("My App".into()),
            redirect_uris: vec!["https://app.example/cb".into()],
            ..Default::default()
        };
        let v = prefill_developer_app_create(&p);
        let obj = v.as_object().expect("object");
        let uris = obj.get("redirect_uris").and_then(|x| x.as_array()).unwrap();
        assert_eq!(uris.len(), 1);
        assert_eq!(uris[0].as_str(), Some("https://app.example/cb"));
    }

    #[test]
    fn prefill_mfa_setup_is_empty_object() {
        let v = prefill_mfa_setup(&MfaSetupPrefill::default());
        let obj = v.as_object().expect("object");
        assert!(obj.is_empty());
    }

    #[test]
    fn prefill_ai_key_omits_none_fields() {
        let p = WizardPrefill {
            slug: Some("llm-openai".into()),
            ..WizardPrefill::default()
        };
        let v = prefill_ai_key(&p);
        let obj = v.as_object().expect("object");
        assert_eq!(obj.get("slug").and_then(|x| x.as_str()), Some("llm-openai"));
        assert!(!obj.contains_key("label"));
        assert!(!obj.contains_key("via_node"));
        assert!(!obj.contains_key("org_id"));
        assert!(!obj.contains_key("endpoint_url"));
        // Issue #414: custom-mode fields default to absent. The
        // existing catalog flow's pairing records stay byte-identical
        // — important for the wizard freshness test and for any
        // pairing-record consumers that might pattern-match on shape.
        assert!(!obj.contains_key("custom"));
        assert!(!obj.contains_key("custom_slug"));
        assert!(!obj.contains_key("auth_method"));
        assert!(!obj.contains_key("auth_key_name"));
    }

    #[test]
    fn prefill_ai_key_emits_custom_mode_fields_when_set() {
        // Issue #414: when --custom is passed at the CLI, the wizard
        // SPA reads `custom: true` from the prefill JSON to skip the
        // catalog grid. The other custom-mode fields ride along with
        // the same Some=>emit semantics as the catalog-flow fields.
        let p = WizardPrefill {
            label: Some("Home Assistant".into()),
            via_node: Some("node-uuid".into()),
            endpoint_url: Some("http://homeassistant.local:8123".into()),
            custom: true,
            custom_slug: Some("home-assistant".into()),
            auth_method: Some("bearer".into()),
            auth_key_name: Some("Authorization".into()),
            ..WizardPrefill::default()
        };
        let v = prefill_ai_key(&p);
        let obj = v.as_object().expect("object");
        // catalog `slug` stays absent in custom mode (no catalog
        // entry); SPA differentiates by `custom: true`.
        assert!(!obj.contains_key("slug"));
        assert_eq!(obj.get("custom").and_then(|x| x.as_bool()), Some(true));
        assert_eq!(
            obj.get("custom_slug").and_then(|x| x.as_str()),
            Some("home-assistant")
        );
        assert_eq!(
            obj.get("auth_method").and_then(|x| x.as_str()),
            Some("bearer")
        );
        assert_eq!(
            obj.get("auth_key_name").and_then(|x| x.as_str()),
            Some("Authorization")
        );
        assert_eq!(
            obj.get("via_node").and_then(|x| x.as_str()),
            Some("node-uuid")
        );
        assert_eq!(
            obj.get("endpoint_url").and_then(|x| x.as_str()),
            Some("http://homeassistant.local:8123")
        );
    }

    #[test]
    fn prefill_ai_key_emits_org_id_when_set() {
        let with_org = WizardPrefill {
            org: Some("0a130a17-2624-4fbb-a69d-8ba51c99952a".into()),
            ..WizardPrefill::default()
        };
        let v = prefill_ai_key(&with_org);
        let obj = v.as_object().expect("object");
        assert_eq!(
            obj.get("org_id").and_then(|x| x.as_str()),
            Some("0a130a17-2624-4fbb-a69d-8ba51c99952a")
        );

        let without_org = prefill_ai_key(&WizardPrefill::default());
        let obj = without_org.as_object().expect("object");
        assert!(!obj.contains_key("org_id"));
    }

    #[test]
    fn prefill_ai_key_skips_custom_flag_when_false() {
        // Default state for the catalog flow (custom is bool, default
        // false). The serializer must not emit `custom: false` —
        // existing pairing records stay byte-identical.
        let p = WizardPrefill {
            slug: Some("llm-openai".into()),
            custom: false,
            ..WizardPrefill::default()
        };
        let v = prefill_ai_key(&p);
        let obj = v.as_object().expect("object");
        assert!(!obj.contains_key("custom"));
    }

    #[test]
    fn parse_ack_ai_key_rejects_unknown_fields() {
        // AiKeyPairingAckPayload uses deny_unknown_fields; a browser
        // that tries to slip a credential back through the ack is
        // rejected before the CLI ever sees the payload.
        let ack = serde_json::json!({
            "acknowledged": true,
            "service_id": "svc-id",
            "slug": "llm-openai",
            "label": "personal",
            "proxy_url": "https://auth.nyxid.dev/api/v1/proxy/s/llm-openai/",
            "credential": "sk-secret-leak"
        });
        let outcome = parse_ack(PairingFlow::AiKey, ack);
        assert!(outcome.is_err(), "deny_unknown_fields should reject");
    }

    #[test]
    fn looks_like_auth_failure_matches_common_shapes() {
        assert!(looks_like_auth_failure(&anyhow!(
            "GET /foo failed (HTTP 401 Unauthorized): ..."
        )));
        assert!(looks_like_auth_failure(&anyhow!(
            "GET /foo failed (HTTP 403 Forbidden): ..."
        )));
        assert!(looks_like_auth_failure(&anyhow!(
            "unauthorized: token expired"
        )));
        assert!(!looks_like_auth_failure(&anyhow!(
            "GET /foo failed (HTTP 500): internal server error"
        )));
        assert!(!looks_like_auth_failure(&anyhow!(
            "dns error: failed to resolve"
        )));
    }

    #[test]
    fn urlencoding_minimal_passes_alphanumerics() {
        assert_eq!(urlencoding_minimal("ABCD-1234"), "ABCD-1234");
        assert_eq!(urlencoding_minimal("abc9XYZ"), "abc9XYZ");
    }

    #[test]
    fn urlencoding_minimal_escapes_unexpected_chars() {
        // Defensive: if a future alphabet change introduces non-URL-safe
        // chars, the helper encodes them rather than silently corrupting
        // the query string.
        assert_eq!(urlencoding_minimal("a b"), "a%20b");
        assert_eq!(urlencoding_minimal("a&b"), "a%26b");
    }

    #[test]
    fn shell_quote_leaves_safe_values_bare() {
        // Safe values must emit unchanged so the hint works in both
        // POSIX sh/bash and Windows cmd.exe — cmd.exe passes single
        // quotes through literally, so `'staging'` would be treated
        // as the literal value `'staging'`, not `staging`.
        assert_eq!(shell_quote("staging"), "staging");
        assert_eq!(shell_quote("NYXID_ACCESS_TOKEN"), "NYXID_ACCESS_TOKEN");
        assert_eq!(
            shell_quote("https://auth.nyxid.dev"),
            "https://auth.nyxid.dev"
        );
    }

    #[test]
    fn shell_quote_wraps_unsafe_values_in_double_quotes() {
        // Double-quote is the grouping quote both POSIX sh and
        // cmd.exe understand. Single-quote wrapping would fail on
        // cmd.exe; double-quote works cross-shell for tokens
        // without `$` / `%`.
        assert_eq!(shell_quote("weird name"), "\"weird name\"");
        assert_eq!(shell_quote("a&b"), "\"a&b\"");
        assert_eq!(shell_quote("a\"b"), "\"a\\\"b\"");
        assert_eq!(shell_quote("a\\b"), "\"a\\\\b\"");
    }

    fn make_auth(
        profile: Option<&str>,
        base_url: Option<&str>,
        access_token: Option<&str>,
        access_token_env: &str,
        output: crate::cli::OutputFormat,
    ) -> crate::cli::AuthArgs {
        crate::cli::AuthArgs {
            base_url: base_url.map(str::to_owned),
            access_token: access_token.map(str::to_owned),
            access_token_env: access_token_env.to_string(),
            profile: profile.map(str::to_owned),
            output,
        }
    }

    #[test]
    fn resume_command_is_minimal_when_no_flags_customised() {
        // Default AuthArgs: no profile, no base-url override, default
        // env name, table output. The hint should collapse to just
        // the positional pairing id so wrappers don't have to strip
        // noise before re-running it.
        let auth = make_auth(
            None,
            None,
            None,
            "NYXID_ACCESS_TOKEN",
            crate::cli::OutputFormat::Table,
        );
        assert_eq!(
            format_resume_command(&auth, "pair_123"),
            "nyxid pairing resume pair_123"
        );
    }

    #[test]
    fn resume_command_propagates_profile_and_base_url() {
        let auth = make_auth(
            Some("staging"),
            Some("https://auth.staging.nyxid.dev"),
            None,
            "NYXID_ACCESS_TOKEN",
            crate::cli::OutputFormat::Table,
        );
        // No quoting on values made of safe chars — see `shell_quote`
        // for the cross-shell rationale (single-quotes would break
        // Windows cmd.exe, which treats them as literal).
        assert_eq!(
            format_resume_command(&auth, "pair_abc"),
            "nyxid pairing resume pair_abc --profile staging \
             --base-url https://auth.staging.nyxid.dev"
        );
    }

    #[test]
    fn resume_command_propagates_output_json() {
        let auth = make_auth(
            None,
            None,
            None,
            "NYXID_ACCESS_TOKEN",
            crate::cli::OutputFormat::Json,
        );
        assert_eq!(
            format_resume_command(&auth, "pair_xyz"),
            "nyxid pairing resume pair_xyz --output json"
        );
    }

    #[test]
    fn resume_command_propagates_non_default_token_env() {
        let auth = make_auth(
            None,
            None,
            None,
            "AGENT_BOT_TOKEN",
            crate::cli::OutputFormat::Table,
        );
        assert_eq!(
            format_resume_command(&auth, "pair_env"),
            "nyxid pairing resume pair_env --access-token-env AGENT_BOT_TOKEN"
        );
    }

    #[test]
    fn resume_command_never_echoes_literal_access_token() {
        // `--no-wait` output is typically captured in agent
        // transcripts; the resume hint must not turn a transient
        // CLI argument into a durable credential leak. Callers
        // must re-supply the token out-of-band when resuming.
        let auth = make_auth(
            None,
            None,
            Some("sk-liveliteral"),
            "NYXID_ACCESS_TOKEN",
            crate::cli::OutputFormat::Table,
        );
        let hint = format_resume_command(&auth, "pair_tok");
        assert!(
            !hint.contains("sk-liveliteral"),
            "resume hint must never contain the literal bearer token: {hint}"
        );
        assert!(
            !hint.contains("--access-token "),
            "resume hint must not include a literal --access-token flag: {hint}"
        );
    }

    #[test]
    fn parse_ack_ai_key_requires_acknowledged_true() {
        let ack = serde_json::json!({
            "acknowledged": false,
            "service_id": "x",
            "slug": "y",
            "label": "z",
            "proxy_url": "https://x"
        });
        let outcome = parse_ack(PairingFlow::AiKey, ack);
        assert!(outcome.is_err());
    }

    #[test]
    fn poll_resp_parses_all_states() {
        // The new wire shape wraps `status` alongside `kind` and
        // `expires_at`. Older responses (no kind/expires_at) should
        // still parse thanks to `#[serde(default)]`.
        let cases = vec![
            (
                r#"{"status":"pending","kind":"api-key-create","expires_at":"2026-04-21T00:00:00Z"}"#,
                "pending",
            ),
            (
                r#"{"status":"claimed","kind":"ai-key","expires_at":"2026-04-21T00:00:00Z"}"#,
                "claimed",
            ),
            (r#"{"status":"cancelled"}"#, "cancelled"),
            (r#"{"status":"expired"}"#, "expired"),
            (
                r#"{"status":"completed","ack":{"acknowledged":true,"api_key_id":"abc"},"kind":"api-key-create","expires_at":"2026-04-21T00:00:00Z"}"#,
                "completed",
            ),
        ];
        for (json, label) in cases {
            let parsed: PollResp =
                serde_json::from_str(json).unwrap_or_else(|e| panic!("parse {label}: {e}"));
            match (parsed.status, label) {
                (PollStatus::Pending, "pending") => (),
                (PollStatus::Claimed, "claimed") => (),
                (PollStatus::Cancelled, "cancelled") => (),
                (PollStatus::Expired, "expired") => (),
                (PollStatus::Completed { .. }, "completed") => (),
                (_, other) => panic!("unexpected variant for {other}"),
            }
        }
    }

    #[test]
    fn prefill_api_key_create_omits_none_fields() {
        let p = ApiKeyCreatePrefill {
            name: Some("agent".into()),
            platform: None,
            scopes: None,
            expires_in_days: None,
            allow_all_services: false,
            allow_all_nodes: false,
            allowed_services_csv: None,
            allowed_nodes_csv: None,
            callback_url: None,
            org_id: None,
        };
        let v = prefill_api_key_create(&p);
        let obj = v.as_object().expect("object");
        assert_eq!(obj.get("name").and_then(|x| x.as_str()), Some("agent"));
        assert!(!obj.contains_key("platform"));
        assert!(!obj.contains_key("scopes"));
    }

    #[test]
    fn prefill_rotate_has_required_fields() {
        let p = RotatePrefill {
            resource_id: "id".into(),
            display_name: "name".into(),
        };
        let v = prefill_rotate(&p);
        assert_eq!(v["resource_id"], "id");
        assert_eq!(v["display_name"], "name");
    }

    #[test]
    fn parse_ack_api_key_create_requires_acknowledged_true() {
        let ack = serde_json::json!({"acknowledged": false, "api_key_id": "x"});
        let outcome = parse_ack(PairingFlow::ApiKeyCreate, ack);
        assert!(outcome.is_err());
    }

    #[test]
    fn parse_ack_rejects_unknown_fields() {
        let ack = serde_json::json!({
            "acknowledged": true,
            "api_key_id": "abc",
            "raw_secret": "nyxid_ag_leaked"
        });
        let outcome = parse_ack(PairingFlow::ApiKeyCreate, ack);
        assert!(outcome.is_err(), "deny_unknown_fields should reject");
    }

    #[test]
    fn parse_ack_rotation_rejects_unacknowledged() {
        let ack = serde_json::json!({"acknowledged": false, "resource_id": "x"});
        assert!(parse_ack(PairingFlow::ApiKeyRotate, ack).is_err());
    }

    #[test]
    fn parse_ack_node_register_rejects_unacknowledged() {
        let ack = serde_json::json!({"acknowledged": false, "token_id": "x"});
        assert!(parse_ack(PairingFlow::NodeRegisterToken, ack).is_err());
    }

    #[test]
    fn parse_ack_service_account_rejects_unacknowledged() {
        let ack = serde_json::json!({"acknowledged": false, "service_account_id": "x"});
        assert!(parse_ack(PairingFlow::ServiceAccountCreate, ack).is_err());
    }

    #[test]
    fn parse_ack_developer_app_rejects_unacknowledged() {
        let ack = serde_json::json!({"acknowledged": false, "developer_app_id": "x"});
        assert!(parse_ack(PairingFlow::DeveloperAppCreate, ack).is_err());
    }

    #[test]
    fn parse_ack_mfa_rejects_unacknowledged() {
        let ack = serde_json::json!({"acknowledged": false, "factor_id": "x"});
        assert!(parse_ack(PairingFlow::MfaSetup, ack).is_err());
    }

    #[test]
    fn parse_ack_node_register_valid() {
        let ack = serde_json::json!({"acknowledged": true, "token_id": "tok-1"});
        let outcome = parse_ack(PairingFlow::NodeRegisterToken, ack).unwrap();
        assert!(matches!(
            outcome,
            WizardOutcome::NodeRegisterAcknowledged(_)
        ));
    }

    #[test]
    fn parse_ack_service_account_create_valid() {
        let ack = serde_json::json!({"acknowledged": true, "service_account_id": "sa-1"});
        let outcome = parse_ack(PairingFlow::ServiceAccountCreate, ack).unwrap();
        assert!(matches!(
            outcome,
            WizardOutcome::ServiceAccountCreateAcknowledged(_)
        ));
    }

    #[test]
    fn parse_ack_developer_app_create_valid() {
        let ack = serde_json::json!({"acknowledged": true, "developer_app_id": "app-1"});
        let outcome = parse_ack(PairingFlow::DeveloperAppCreate, ack).unwrap();
        assert!(matches!(
            outcome,
            WizardOutcome::DeveloperAppCreateAcknowledged(_)
        ));
    }

    #[test]
    fn parse_ack_mfa_setup_valid() {
        let ack = serde_json::json!({"acknowledged": true, "factor_id": "mfa-1"});
        let outcome = parse_ack(PairingFlow::MfaSetup, ack).unwrap();
        assert!(matches!(outcome, WizardOutcome::MfaSetupAcknowledged(_)));
    }

    #[test]
    fn parse_ack_api_key_create_valid() {
        let ack = serde_json::json!({"acknowledged": true, "api_key_id": "key-1"});
        let outcome = parse_ack(PairingFlow::ApiKeyCreate, ack).unwrap();
        assert!(matches!(
            outcome,
            WizardOutcome::ApiKeyCreateAcknowledged(_)
        ));
    }

    #[test]
    fn parse_ack_ai_key_valid() {
        let ack = serde_json::json!({"acknowledged": true, "service_id": "s1", "slug": "s", "label": "l"});
        let outcome = parse_ack(PairingFlow::AiKey, ack).unwrap();
        assert!(matches!(outcome, WizardOutcome::AiKeyPaired(_)));
    }

    #[test]
    fn parse_ack_node_rotate_valid() {
        let ack = serde_json::json!({"acknowledged": true, "resource_id": "n1"});
        let outcome = parse_ack(PairingFlow::NodeRotateToken, ack).unwrap();
        assert!(matches!(outcome, WizardOutcome::RotationAcknowledged(_)));
    }

    #[test]
    fn pair_url_with_code_appends_to_existing_query() {
        let resp = CreateResp {
            id: "id".into(),
            code: "AB-12".into(),
            pair_url: "https://x.com/pair?flow=ai".into(),
            poll_url: String::new(),
            expires_at: String::new(),
        };
        assert_eq!(
            pair_url_with_code(&resp),
            "https://x.com/pair?flow=ai&code=AB-12"
        );
    }

    #[test]
    fn pair_url_with_code_adds_query_when_none() {
        let resp = CreateResp {
            id: "id".into(),
            code: "CD-34".into(),
            pair_url: "https://x.com/pair".into(),
            poll_url: String::new(),
            expires_at: String::new(),
        };
        assert_eq!(pair_url_with_code(&resp), "https://x.com/pair?code=CD-34");
    }

    #[test]
    fn urlencoding_minimal_empty_string() {
        assert_eq!(urlencoding_minimal(""), "");
    }

    #[test]
    fn shell_quote_empty_string_wraps_in_quotes() {
        assert_eq!(shell_quote(""), "\"\"");
    }

    #[test]
    fn prefill_node_register_with_name() {
        let p = NodeRegisterPrefill {
            name: Some("my-node".into()),
        };
        let v = prefill_node_register(&p);
        assert_eq!(v["name"], "my-node");
    }

    #[test]
    fn prefill_node_register_without_name() {
        let p = NodeRegisterPrefill { name: None };
        let v = prefill_node_register(&p);
        assert!(v.as_object().unwrap().is_empty());
    }

    #[test]
    fn prefill_api_key_create_all_fields_set() {
        let p = ApiKeyCreatePrefill {
            name: Some("agent".into()),
            platform: Some("claude-code".into()),
            scopes: Some("proxy".into()),
            expires_in_days: Some(30),
            allow_all_services: true,
            allow_all_nodes: true,
            allowed_services_csv: Some("svc1,svc2".into()),
            allowed_nodes_csv: Some("n1".into()),
            callback_url: Some("https://cb.example.com".into()),
            org_id: Some("org-1".into()),
        };
        let v = prefill_api_key_create(&p);
        let obj = v.as_object().unwrap();
        assert_eq!(obj.len(), 10);
        assert_eq!(v["expires_in_days"], 30);
    }

    #[test]
    fn prefill_service_account_create_all_fields() {
        let p = ServiceAccountCreatePrefill {
            name: Some("bot".into()),
            scopes: Some("admin".into()),
            description: Some("CI bot".into()),
            rate_limit_override: Some(200),
            role_ids_csv: Some("r1,r2".into()),
            org_id: Some("org-x".into()),
        };
        let v = prefill_service_account_create(&p);
        assert_eq!(v.as_object().unwrap().len(), 6);
    }

    #[test]
    fn prefill_developer_app_create_filters_empty_uris() {
        let p = DeveloperAppCreatePrefill {
            redirect_uris: vec!["".into(), "https://a.com".into(), "".into()],
            ..Default::default()
        };
        let v = prefill_developer_app_create(&p);
        let uris = v["redirect_uris"].as_array().unwrap();
        assert_eq!(uris.len(), 1);
    }

    #[test]
    fn prefill_developer_app_create_no_uris() {
        let p = DeveloperAppCreatePrefill::default();
        let v = prefill_developer_app_create(&p);
        assert!(!v.as_object().unwrap().contains_key("redirect_uris"));
    }

    #[test]
    fn looks_like_auth_failure_handles_chained_errors() {
        let inner = anyhow::anyhow!("connection failed");
        let outer = inner.context("GET /foo failed (HTTP 401)");
        assert!(looks_like_auth_failure(&outer));
    }

    #[test]
    fn from_kind_str_returns_none_for_empty() {
        assert!(PairingFlow::from_kind_str("").is_none());
    }
}

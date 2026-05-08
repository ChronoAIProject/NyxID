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
//!
//! v3.1 (`docs/CLI_WIZARD_V3.md` §2) adds the create-side of DisplayOnce:
//! `nyxid node register-token` and `nyxid api-key create`. The backend
//! still generates the one-time secret, but the trigger is a *create*
//! call rather than a *rotate* call. The same narrow machinery applies:
//! per-flow typed ack payloads (`NodeRegisterAckPayload`,
//! `ApiKeyCreateAckPayload`), per-flow field-allowlist printer closures,
//! and the same 60 s heartbeat-dead window (generalized from
//! `is_rotation` → `is_display_once` on `FlowKind`).

mod server;

pub mod pairing;

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
///
/// Issue #414: the wizard also supports `--custom` mode where the
/// user is creating a custom (non-catalog) endpoint. In that case
/// the wizard's confirm panel skips the catalog grid and renders
/// the form directly with the prefilled definitional fields
/// (label, endpoint_url, auth_method, auth_key_name, via_node,
/// optional custom_slug). The user fills in the remaining required
/// fields (typically just the credential) and submits. Same
/// `POST /keys` body shape as the catalog flow with `custom: true`.
#[derive(Debug, Clone, Default)]
pub struct WizardPrefill {
    pub slug: Option<String>,
    pub label: Option<String>,
    pub via_node: Option<String>,
    /// Resolved org owner user id from `--org`, not the raw slug/name
    /// typed by the user. The frontend sends this as `target_org_id`.
    pub org: Option<String>,
    pub endpoint_url: Option<String>,
    /// `true` when the user passed `--custom`. The wizard SPA reads
    /// this and skips Step 1 (catalog grid), going straight to the
    /// custom-service form pre-populated with the other fields.
    pub custom: bool,
    /// `--slug <s>` override for custom services. Distinct from the
    /// catalog `slug` above, which selects a catalog entry. The user
    /// can supply this to choose their own proxy slug; otherwise the
    /// wizard auto-derives it from `label`.
    pub custom_slug: Option<String>,
    /// Auth method (bearer, header, query, path, basic, body,
    /// bot_bearer, none). Required for custom services since there's
    /// no catalog default to inherit.
    pub auth_method: Option<String>,
    /// Auth key name (e.g. Authorization, X-API-Key, app_secret).
    /// When unset, defaults are derived per auth_method by the SPA.
    pub auth_key_name: Option<String>,
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

/// CLI-supplied prefill for `nyxid node register-token`. No resource to
/// resolve up front — the backend mints a fresh token on confirm — so
/// the only field is the optional node name. When the user didn't pass
/// `--name` on the CLI, the wizard collects it in the browser instead
/// of via an opaque stdin prompt.
#[derive(Debug, Clone, Default)]
pub struct NodeRegisterPrefill {
    pub name: Option<String>,
}

/// CLI-supplied prefill for `nyxid api-key create`. Any field set here
/// is encoded into the browser URL so the wizard opens with those
/// values pre-populated. A user who types `nyxid api-key create --name
/// coding-agent --platform claude-code` gets a wizard pre-filled with
/// those two fields and the cursor on the "Create" button.
#[derive(Debug, Clone, Default)]
pub struct ApiKeyCreatePrefill {
    pub name: Option<String>,
    pub platform: Option<String>,
    pub scopes: Option<String>,
    pub expires_in_days: Option<u32>,
    pub allow_all_services: bool,
    pub allow_all_nodes: bool,
    pub allowed_services_csv: Option<String>,
    pub allowed_nodes_csv: Option<String>,
    pub callback_url: Option<String>,
    pub org_id: Option<String>,
}

/// CLI-supplied prefill for `nyxid service-account create`. Same
/// shape principle as `ApiKeyCreatePrefill` — every field surfaces
/// the corresponding CLI flag so the browser opens pre-populated.
#[derive(Debug, Clone, Default)]
pub struct ServiceAccountCreatePrefill {
    pub name: Option<String>,
    pub scopes: Option<String>,
    pub description: Option<String>,
    pub rate_limit_override: Option<u64>,
    pub role_ids_csv: Option<String>,
    pub org_id: Option<String>,
}

/// CLI-supplied prefill for `nyxid developer-app create`. The wizard
/// only fires for confidential clients (the public-client path
/// produces no `client_secret`, so the original terminal output has
/// nothing to leak). `client_type` is therefore always
/// "confidential" when this prefill is constructed.
#[derive(Debug, Clone, Default)]
pub struct DeveloperAppCreatePrefill {
    pub name: Option<String>,
    /// Repeated `--redirect-uri` flags. Required: at least one URI.
    /// Captured as a list so the wizard can render an editable
    /// multi-row input pre-populated with whatever the CLI passed.
    pub redirect_uris: Vec<String>,
    pub allowed_scopes: Option<String>,
    pub delegation_scopes: Option<String>,
    pub broker_capability: Option<bool>,
    pub org_id: Option<String>,
}

/// CLI-supplied prefill for `nyxid mfa setup`. There are no CLI
/// flags today, but the struct is kept for symmetry with the other
/// flows and to leave room for future fields (e.g. an explicit
/// factor name) without changing the wizard wire shape.
#[derive(Debug, Clone, Default)]
pub struct MfaSetupPrefill {}

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

/// Typed completion payload for `nyxid node register-token`. Same
/// `deny_unknown_fields` guard as `RotationAckPayload`: the browser
/// CANNOT smuggle the raw `nyx_nreg_...` token back through this path.
/// `token_id` is the server-issued UUID for the registration token
/// record (already visible to the user in the audit log and list
/// endpoint); echoing it is safe and lets the CLI summary reference
/// the row the user just created.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeRegisterAckPayload {
    pub acknowledged: bool,
    pub token_id: String,
}

/// Typed completion payload for `nyxid api-key create`. Shape and
/// guards mirror the other ack payloads. `api_key_id` is the server-
/// issued UUID for the created `ApiKey` record — non-secret, visible
/// via `GET /api/v1/api-keys`, safe to print.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ApiKeyCreateAckPayload {
    pub acknowledged: bool,
    pub api_key_id: String,
}

/// Typed completion payload for `nyxid service-account create`. The
/// raw `client_secret` lives only in the browser; `service_account_id`
/// is the non-secret UUID, safe to print in the terminal summary.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ServiceAccountCreateAckPayload {
    pub acknowledged: bool,
    pub service_account_id: String,
}

/// Typed completion payload for `nyxid developer-app create` for
/// confidential clients only (public clients never enter the wizard,
/// so an ack of this shape implies a `client_secret` was minted).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeveloperAppCreateAckPayload {
    pub acknowledged: bool,
    pub developer_app_id: String,
}

/// Typed completion payload for `nyxid mfa setup`. The wizard runs
/// the full enrollment in the browser (`POST /mfa/setup` →
/// `POST /mfa/confirm`); `factor_id` identifies the now-verified
/// `MfaFactor` row and is non-secret.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MfaSetupAckPayload {
    pub acknowledged: bool,
    pub factor_id: String,
}

/// Typed completion payload for the pairing-transport flavor of
/// `nyxid service add` (ai-key). Unlike the DisplayOnce flows, the
/// user's downstream credential never round-trips through the pairing
/// record — only non-secret identifiers: the new `UserService` id,
/// the slug, and the label.
///
/// Importantly, the final proxy URL is NOT carried in the ack. The
/// frontend runs on `FRONTEND_URL` and the proxy endpoint lives on
/// `BASE_URL` — so a browser-side `window.location.origin` would be
/// wrong on split-origin deployments. The CLI already knows its own
/// `base_url` from `AuthArgs` and builds the proxy URL from that
/// (see `print_wizard_summary` in this module) which is the single
/// source of truth.
///
/// Same `deny_unknown_fields` guard keeps the payload narrow against
/// a buggy browser page.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AiKeyPairingAckPayload {
    pub acknowledged: bool,
    /// Retained on the struct (not currently printed) so scripted
    /// callers and audit code can read the `UserService` id without
    /// re-querying. `deny_unknown_fields` requires explicit
    /// enumeration, so keeping it here also documents the wire shape.
    #[allow(dead_code)]
    pub service_id: String,
    pub slug: String,
    pub label: String,
}

/// Outcome of a wizard run, returned to the caller for shaping terminal
/// output. Variants are flow-specific so the leak surface stays narrow:
/// the ai-key flow keeps its existing untyped body (a slug+label+url
/// summary nobody calls a secret); DisplayOnce flows MUST go through a
/// typed per-flow ack payload so the printer never sees raw bytes from
/// the browser.
#[derive(Debug, Clone)]
pub enum WizardOutcome {
    AiKeyCompleted(serde_json::Value),
    /// Pairing-transport equivalent of `AiKeyCompleted`. Separate
    /// variant so the local-server and pairing paths keep distinct
    /// printers — the local server's body is an untyped `Value` built
    /// by the in-wizard SPA, while the pairing ack is the narrow
    /// typed payload above.
    AiKeyPaired(AiKeyPairingAckPayload),
    /// Shared by all rotation flows: `api-key-rotate`,
    /// `node-rotate-token`, `service-account-rotate-secret`,
    /// `developer-app-rotate-secret`. The CLI dispatches the
    /// per-flow success summary via `FlowKind` (see
    /// `run_rotation_wizard`); only the resource_id round-trips.
    RotationAcknowledged(RotationAckPayload),
    NodeRegisterAcknowledged(NodeRegisterAckPayload),
    ApiKeyCreateAcknowledged(ApiKeyCreateAckPayload),
    ServiceAccountCreateAcknowledged(ServiceAccountCreateAckPayload),
    DeveloperAppCreateAcknowledged(DeveloperAppCreateAckPayload),
    MfaSetupAcknowledged(MfaSetupAckPayload),
    Cancelled,
    TimedOut,
}

/// Best-effort fetch of a resource's display name AFTER a successful
/// create-flow ack. The ack itself only carries the resource UUID
/// (deny_unknown_fields keeps the payload narrow), but `nyxid …
/// pairing resume` and the live wizard summaries are friendlier when
/// they can echo the human-readable name. Falls through silently on
/// any failure — the caller renders the ID-only line in that case.
///
/// `path` is the API path beneath `/api/v1` (e.g.
/// `/admin/service-accounts/{id}`). `name_field` is the JSON key on
/// the response that holds the display string — service-account uses
/// `name`, developer-app uses `client_name`.
async fn try_fetch_display_name(
    auth: &crate::cli::AuthArgs,
    path: &str,
    name_field: &str,
) -> Option<String> {
    let mut api = crate::api::ApiClient::from_auth(auth).ok()?;
    let v: serde_json::Value = api.get(path).await.ok()?;
    v.get(name_field).and_then(|n| n.as_str()).map(String::from)
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
pub async fn run_ai_key_wizard(
    auth: &crate::cli::AuthArgs,
    prefill: WizardPrefill,
    no_wait: bool,
) -> Result<()> {
    if no_wait {
        let prefill_json = pairing::prefill_ai_key(&prefill);
        return pairing::run_no_wait_pairing(auth, pairing::PairingFlow::AiKey, prefill_json).await;
    }

    let base_url = auth.resolved_base_url()?;

    let outcome = if is_wizard_eligible() {
        let access_token = crate::auth::resolve_access_token(auth)?;
        let base_url_root = base_url.trim_end_matches('/').to_string();
        let proxy = ProxyContext {
            base_url_root,
            access_token,
            profile: auth.profile.clone(),
        };
        run_flow("ai-key", proxy, prefill).await?
    } else {
        let prefill_json = pairing::prefill_ai_key(&prefill);
        pairing::run_display_once_pairing(auth, pairing::PairingFlow::AiKey, prefill_json).await?
    };

    match outcome {
        WizardOutcome::AiKeyCompleted(body) => {
            attract_terminal("NyxID wizard complete");
            print_wizard_summary(&body, &base_url);
            Ok(())
        }
        WizardOutcome::AiKeyPaired(ack) => {
            attract_terminal("NyxID wizard complete");
            // Field allowlist: slug / label from the backend-created
            // UserService (non-secret by construction). `proxy_url`
            // is intentionally NOT in the ack — split-origin
            // deployments (FRONTEND_URL != BASE_URL) would produce
            // the wrong host. `print_wizard_summary` builds it from
            // the CLI's own `base_url`, which is authoritative.
            let pseudo_body = serde_json::json!({
                "slug": ack.slug,
                "label": ack.label,
            });
            print_wizard_summary(&pseudo_body, &base_url);
            Ok(())
        }
        WizardOutcome::RotationAcknowledged(_)
        | WizardOutcome::NodeRegisterAcknowledged(_)
        | WizardOutcome::ApiKeyCreateAcknowledged(_)
        | WizardOutcome::ServiceAccountCreateAcknowledged(_)
        | WizardOutcome::DeveloperAppCreateAcknowledged(_)
        | WizardOutcome::MfaSetupAcknowledged(_) => {
            // Defensive: a DisplayOnce outcome can't reach the ai-key
            // handler (server::handle_complete and the pairing client
            // dispatch by FlowKind / PairingFlow), but if it ever did
            // we'd refuse to print anything from it.
            Err(anyhow!(
                "internal: ai-key wizard returned a display-once outcome (flow dispatch broken)"
            ))
        }
        WizardOutcome::Cancelled => {
            attract_terminal("NyxID wizard cancelled");
            eprintln!("✗ Wizard cancelled before the CLI received a completed service.");
            eprintln!(
                "  If you clicked Connect service before closing the window, run `nyxid service list` to check whether it was created."
            );
            // The remote-pairing path may have been cancelled by the
            // web UI bouncing to the main Keys page for an
            // unsupported flow (OAuth/device-code in split-origin,
            // token-exchange, etc.). Hint at the Keys page so the
            // user knows where to finish.
            eprintln!(
                "  If this provider needs OAuth / multi-field setup, finish adding the service in the NyxID web UI under Keys."
            );
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

/// Shared entry point for the `nyxid node register-token` wizard.
/// Scripted / headless path lives in `commands::node` and stays
/// byte-identical to pre-wizard behavior.
///
/// When the environment can run a local browser, spins up the local
/// axum server; otherwise falls back to the remote pairing transport
/// ([`pairing::run_display_once_pairing`]) which emits a code + pair
/// URL and polls the backend for the ack. Either way the returned
/// `WizardOutcome` shape is the same, so the terminal output below
/// doesn't branch.
pub async fn run_node_register_token_wizard(
    auth: &crate::cli::AuthArgs,
    prefill: NodeRegisterPrefill,
    no_wait: bool,
) -> Result<()> {
    if no_wait {
        let prefill_json = pairing::prefill_node_register(&prefill);
        return pairing::run_no_wait_pairing(
            auth,
            pairing::PairingFlow::NodeRegisterToken,
            prefill_json,
        )
        .await;
    }

    let outcome = if is_wizard_eligible() {
        let base_url = auth.resolved_base_url()?;
        let access_token = crate::auth::resolve_access_token(auth)?;
        let base_url_root = base_url.trim_end_matches('/').to_string();
        let proxy = ProxyContext {
            base_url_root,
            access_token,
            profile: auth.profile.clone(),
        };
        server::run_flow(
            server::FlowKind::NodeRegisterToken,
            proxy,
            server::PrefillData::NodeRegister(prefill),
        )
        .await?
    } else {
        let prefill_json = pairing::prefill_node_register(&prefill);
        pairing::run_display_once_pairing(
            auth,
            pairing::PairingFlow::NodeRegisterToken,
            prefill_json,
        )
        .await?
    };

    match outcome {
        WizardOutcome::NodeRegisterAcknowledged(ack) => {
            attract_terminal("NyxID wizard complete");
            // Field allowlist: only `ack.token_id` (echoed from the
            // browser, validated UUID-ish server-side). Never
            // `format!("{ack:?}")`, never `serde_json::to_string(&ack)`.
            eprintln!("✓ Registration token generated. New value was shown in the browser.");
            eprintln!("  Token ID: {}", ack.token_id);
            eprintln!("  Register a node with:");
            eprintln!(
                "    nyxid node register --token <token-from-browser> --url ws://<server>/api/v1/nodes/ws"
            );
            Ok(())
        }
        WizardOutcome::Cancelled => {
            attract_terminal("NyxID wizard cancelled");
            eprintln!("✗ Registration token wizard cancelled.");
            eprintln!("  If the new token was shown in the browser, it was minted on the server.");
            eprintln!(
                "  If you saved it, you're done. If not, run `nyxid node register-token` again."
            );
            std::process::exit(1);
        }
        WizardOutcome::TimedOut => {
            attract_terminal("NyxID wizard timed out");
            eprintln!("✗ Registration token wizard timed out.");
            eprintln!("  If the new token was shown in the browser, it was minted on the server.");
            eprintln!(
                "  If you didn't save it, run `nyxid node register-token` again to issue a fresh token."
            );
            std::process::exit(1);
        }
        _ => Err(anyhow!(
            "internal: node-register-token wizard returned unexpected outcome"
        )),
    }
}

/// Shared entry point for the `nyxid api-key create` wizard. All CLI
/// flags are plumbed through as `prefill` so a user who typed values on
/// the command line sees them pre-selected in the browser.
///
/// Headless-env branch routes through [`pairing::run_display_once_pairing`];
/// the outcome dispatch below is identical for both paths because
/// `ApiKeyCreateAckPayload` is shared across transports.
pub async fn run_api_key_create_wizard(
    auth: &crate::cli::AuthArgs,
    prefill: ApiKeyCreatePrefill,
    no_wait: bool,
) -> Result<()> {
    if no_wait {
        let prefill_json = pairing::prefill_api_key_create(&prefill);
        return pairing::run_no_wait_pairing(
            auth,
            pairing::PairingFlow::ApiKeyCreate,
            prefill_json,
        )
        .await;
    }

    let outcome = if is_wizard_eligible() {
        let base_url = auth.resolved_base_url()?;
        let access_token = crate::auth::resolve_access_token(auth)?;
        let base_url_root = base_url.trim_end_matches('/').to_string();
        let proxy = ProxyContext {
            base_url_root,
            access_token,
            profile: auth.profile.clone(),
        };
        server::run_flow(
            server::FlowKind::ApiKeyCreate,
            proxy,
            server::PrefillData::ApiKeyCreate(prefill),
        )
        .await?
    } else {
        let prefill_json = pairing::prefill_api_key_create(&prefill);
        pairing::run_display_once_pairing(auth, pairing::PairingFlow::ApiKeyCreate, prefill_json)
            .await?
    };

    match outcome {
        WizardOutcome::ApiKeyCreateAcknowledged(ack) => {
            attract_terminal("NyxID wizard complete");
            // Field allowlist: only `ack.api_key_id` (validated UUID-ish).
            // The display name is fetched best-effort via the same
            // bearer the wizard already used; missing on failure.
            let display_name =
                try_fetch_display_name(auth, &format!("/api-keys/{}", ack.api_key_id), "name")
                    .await;
            eprintln!("✓ API key created. New value was shown in the browser.");
            if let Some(name) = display_name {
                eprintln!("  Name: {name}");
            }
            eprintln!("  ID: {}", ack.api_key_id);
            eprintln!("  Set as environment variable:");
            eprintln!("    export NYXID_API_KEY=\"<value-from-browser>\"");
            Ok(())
        }
        WizardOutcome::Cancelled => {
            attract_terminal("NyxID wizard cancelled");
            eprintln!("✗ API key wizard cancelled.");
            eprintln!("  If the new key was shown in the browser, it was created on the server.");
            eprintln!("  If you saved it, you're done. If not, run `nyxid api-key create` again.");
            std::process::exit(1);
        }
        WizardOutcome::TimedOut => {
            attract_terminal("NyxID wizard timed out");
            eprintln!("✗ API key wizard timed out.");
            eprintln!("  If the new key was shown in the browser, it was created on the server.");
            eprintln!(
                "  If you didn't save it, run `nyxid api-key create` again to issue a fresh key."
            );
            std::process::exit(1);
        }
        _ => Err(anyhow!(
            "internal: api-key-create wizard returned unexpected outcome"
        )),
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
    no_wait: bool,
) -> Result<()> {
    run_rotation_wizard(
        auth,
        server::FlowKind::ApiKeyRotate,
        prefill,
        no_wait,
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
    no_wait: bool,
) -> Result<()> {
    run_rotation_wizard(
        auth,
        server::FlowKind::NodeRotateToken,
        prefill,
        no_wait,
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
    no_wait: bool,
    success_summary: impl FnOnce(&str, &str),
    resource_label: &'static str,
    rerun_command: &'static str,
) -> Result<()> {
    if no_wait {
        let pairing_flow = rotation_pairing_flow(flow_kind)?;
        let prefill_json = pairing::prefill_rotate(&prefill);
        return pairing::run_no_wait_pairing(auth, pairing_flow, prefill_json).await;
    }

    let display_name_for_summary = prefill.display_name.clone();

    let outcome = if is_wizard_eligible() {
        let base_url = auth.resolved_base_url()?;
        let access_token = crate::auth::resolve_access_token(auth)?;
        let base_url_root = base_url.trim_end_matches('/').to_string();
        let proxy = ProxyContext {
            base_url_root,
            access_token,
            profile: auth.profile.clone(),
        };
        server::run_flow(flow_kind, proxy, server::PrefillData::Rotate(prefill)).await?
    } else {
        // Map the local-server FlowKind to the pairing-transport
        // PairingFlow. All four rotation-shaped flows (api-key-rotate,
        // node-rotate-token, service-account-rotate-secret,
        // developer-app-rotate-secret) share `RotatePrefill` +
        // `RotationAckPayload`; only the kind string differs.
        let pairing_flow = rotation_pairing_flow(flow_kind)?;
        let prefill_json = pairing::prefill_rotate(&prefill);
        pairing::run_display_once_pairing(auth, pairing_flow, prefill_json).await?
    };

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
        WizardOutcome::AiKeyCompleted(_)
        | WizardOutcome::AiKeyPaired(_)
        | WizardOutcome::NodeRegisterAcknowledged(_)
        | WizardOutcome::ApiKeyCreateAcknowledged(_)
        | WizardOutcome::ServiceAccountCreateAcknowledged(_)
        | WizardOutcome::DeveloperAppCreateAcknowledged(_)
        | WizardOutcome::MfaSetupAcknowledged(_) => {
            // Defensive: server dispatch shouldn't produce these for a
            // rotation flow.
            Err(anyhow!(
                "internal: rotation wizard returned a non-rotation outcome (flow dispatch broken)"
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

/// Map a rotation `FlowKind` to its pairing-transport sibling.
/// Centralized so `run_rotation_wizard`'s two branches (no-wait,
/// local-or-pairing fallback) stay in lockstep when a new rotation-
/// shaped flow lands.
fn rotation_pairing_flow(flow_kind: server::FlowKind) -> Result<pairing::PairingFlow> {
    match flow_kind {
        server::FlowKind::ApiKeyRotate => Ok(pairing::PairingFlow::ApiKeyRotate),
        server::FlowKind::NodeRotateToken => Ok(pairing::PairingFlow::NodeRotateToken),
        server::FlowKind::ServiceAccountRotateSecret => {
            Ok(pairing::PairingFlow::ServiceAccountRotateSecret)
        }
        server::FlowKind::DeveloperAppRotateSecret => {
            Ok(pairing::PairingFlow::DeveloperAppRotateSecret)
        }
        other => Err(anyhow!(
            "internal: run_rotation_wizard called with non-rotation FlowKind {other:?}"
        )),
    }
}

/// Shared entry point for `nyxid service-account rotate-secret`.
/// Mirrors `run_api_key_rotate_wizard`. Caller has already resolved
/// the service-account ID and fetched a display name (best-effort).
pub async fn run_service_account_rotate_secret_wizard(
    auth: &crate::cli::AuthArgs,
    prefill: RotatePrefill,
    no_wait: bool,
) -> Result<()> {
    run_rotation_wizard(
        auth,
        server::FlowKind::ServiceAccountRotateSecret,
        prefill,
        no_wait,
        |display, id| {
            eprintln!(
                "✓ Service account '{display}' secret rotated. New value was shown in the browser."
            );
            eprintln!("  ID: {id}");
            eprintln!("  All previously-issued tokens for this service account are now revoked.");
        },
        "client_secret",
        "nyxid service-account rotate-secret <id>",
    )
    .await
}

/// Shared entry point for `nyxid developer-app rotate-secret`.
/// Confidential clients only — the scripted path errors out for
/// public clients before reaching here.
pub async fn run_developer_app_rotate_secret_wizard(
    auth: &crate::cli::AuthArgs,
    prefill: RotatePrefill,
    no_wait: bool,
) -> Result<()> {
    run_rotation_wizard(
        auth,
        server::FlowKind::DeveloperAppRotateSecret,
        prefill,
        no_wait,
        |display, id| {
            eprintln!(
                "✓ Developer app '{display}' client_secret rotated. New value was shown in the browser."
            );
            eprintln!("  ID: {id}");
            eprintln!("  Update any deployments using the previous secret.");
        },
        "client_secret",
        "nyxid developer-app rotate-secret <id>",
    )
    .await
}

/// Shared entry point for the `nyxid service-account create` wizard.
/// All CLI flags are plumbed through `prefill` so values typed on
/// the command line appear pre-populated in the browser.
pub async fn run_service_account_create_wizard(
    auth: &crate::cli::AuthArgs,
    prefill: ServiceAccountCreatePrefill,
    no_wait: bool,
) -> Result<()> {
    if no_wait {
        let prefill_json = pairing::prefill_service_account_create(&prefill);
        return pairing::run_no_wait_pairing(
            auth,
            pairing::PairingFlow::ServiceAccountCreate,
            prefill_json,
        )
        .await;
    }

    let outcome = if is_wizard_eligible() {
        let base_url = auth.resolved_base_url()?;
        let access_token = crate::auth::resolve_access_token(auth)?;
        let base_url_root = base_url.trim_end_matches('/').to_string();
        let proxy = ProxyContext {
            base_url_root,
            access_token,
            profile: auth.profile.clone(),
        };
        server::run_flow(
            server::FlowKind::ServiceAccountCreate,
            proxy,
            server::PrefillData::ServiceAccountCreate(prefill),
        )
        .await?
    } else {
        let prefill_json = pairing::prefill_service_account_create(&prefill);
        pairing::run_display_once_pairing(
            auth,
            pairing::PairingFlow::ServiceAccountCreate,
            prefill_json,
        )
        .await?
    };

    match outcome {
        WizardOutcome::ServiceAccountCreateAcknowledged(ack) => {
            attract_terminal("NyxID wizard complete");
            // Field allowlist: only ack.service_account_id (validated
            // UUID-ish server-side), never the raw client_secret.
            let display_name = try_fetch_display_name(
                auth,
                &format!("/admin/service-accounts/{}", ack.service_account_id),
                "name",
            )
            .await;
            eprintln!("✓ Service account created. The client_secret was shown in the browser.");
            if let Some(name) = display_name {
                eprintln!("  Name: {name}");
            }
            eprintln!("  ID: {}", ack.service_account_id);
            eprintln!("  Save the client_secret from the browser tab — it isn't shown again.");
            Ok(())
        }
        WizardOutcome::Cancelled => {
            attract_terminal("NyxID wizard cancelled");
            eprintln!("✗ Service account wizard cancelled.");
            eprintln!(
                "  If the new client_secret was shown in the browser, the SA was created on the server."
            );
            eprintln!(
                "  If you saved it, you're done. If not, run `nyxid service-account create` again."
            );
            std::process::exit(1);
        }
        WizardOutcome::TimedOut => {
            attract_terminal("NyxID wizard timed out");
            eprintln!("✗ Service account wizard timed out.");
            eprintln!(
                "  If the new client_secret was shown in the browser, the SA was created on the server."
            );
            eprintln!(
                "  If you didn't save it, run `nyxid service-account create` again to issue a fresh secret."
            );
            std::process::exit(1);
        }
        _ => Err(anyhow!(
            "internal: service-account-create wizard returned unexpected outcome"
        )),
    }
}

/// Shared entry point for the `nyxid developer-app create` wizard.
/// Caller MUST gate on `client_type == "confidential"` upstream;
/// public clients have no `client_secret` to display.
pub async fn run_developer_app_create_wizard(
    auth: &crate::cli::AuthArgs,
    prefill: DeveloperAppCreatePrefill,
    no_wait: bool,
) -> Result<()> {
    if no_wait {
        let prefill_json = pairing::prefill_developer_app_create(&prefill);
        return pairing::run_no_wait_pairing(
            auth,
            pairing::PairingFlow::DeveloperAppCreate,
            prefill_json,
        )
        .await;
    }

    let outcome = if is_wizard_eligible() {
        let base_url = auth.resolved_base_url()?;
        let access_token = crate::auth::resolve_access_token(auth)?;
        let base_url_root = base_url.trim_end_matches('/').to_string();
        let proxy = ProxyContext {
            base_url_root,
            access_token,
            profile: auth.profile.clone(),
        };
        server::run_flow(
            server::FlowKind::DeveloperAppCreate,
            proxy,
            server::PrefillData::DeveloperAppCreate(prefill),
        )
        .await?
    } else {
        let prefill_json = pairing::prefill_developer_app_create(&prefill);
        pairing::run_display_once_pairing(
            auth,
            pairing::PairingFlow::DeveloperAppCreate,
            prefill_json,
        )
        .await?
    };

    match outcome {
        WizardOutcome::DeveloperAppCreateAcknowledged(ack) => {
            attract_terminal("NyxID wizard complete");
            // Backend uses `client_name`, not `name`, on this response.
            let display_name = try_fetch_display_name(
                auth,
                &format!("/developer/oauth-clients/{}", ack.developer_app_id),
                "client_name",
            )
            .await;
            eprintln!(
                "✓ Developer app created. The confidential client_secret was shown in the browser."
            );
            if let Some(name) = display_name {
                eprintln!("  Name: {name}");
            }
            eprintln!("  ID: {}", ack.developer_app_id);
            eprintln!("  Save the client_secret from the browser tab — it isn't shown again.");
            Ok(())
        }
        WizardOutcome::Cancelled => {
            attract_terminal("NyxID wizard cancelled");
            eprintln!("✗ Developer app wizard cancelled.");
            eprintln!(
                "  If the new client_secret was shown in the browser, the app was created on the server."
            );
            eprintln!(
                "  If you saved it, you're done. If not, run `nyxid developer-app create` again."
            );
            std::process::exit(1);
        }
        WizardOutcome::TimedOut => {
            attract_terminal("NyxID wizard timed out");
            eprintln!("✗ Developer app wizard timed out.");
            eprintln!(
                "  If the new client_secret was shown in the browser, the app was created on the server."
            );
            eprintln!(
                "  If you didn't save it, run `nyxid developer-app create` again to issue a fresh secret."
            );
            std::process::exit(1);
        }
        _ => Err(anyhow!(
            "internal: developer-app-create wizard returned unexpected outcome"
        )),
    }
}

/// Shared entry point for the `nyxid mfa setup` wizard. The browser
/// runs both halves of TOTP enrollment (`POST /mfa/setup` to mint the
/// secret and render a QR, then `POST /mfa/confirm` to verify the
/// user-entered TOTP and reveal the recovery codes). The CLI never
/// sees the secret, the QR URL, or the recovery codes — only the
/// non-secret `factor_id` echo.
pub async fn run_mfa_setup_wizard(
    auth: &crate::cli::AuthArgs,
    prefill: MfaSetupPrefill,
    no_wait: bool,
) -> Result<()> {
    if no_wait {
        let prefill_json = pairing::prefill_mfa_setup(&prefill);
        return pairing::run_no_wait_pairing(auth, pairing::PairingFlow::MfaSetup, prefill_json)
            .await;
    }

    let outcome = if is_wizard_eligible() {
        let base_url = auth.resolved_base_url()?;
        let access_token = crate::auth::resolve_access_token(auth)?;
        let base_url_root = base_url.trim_end_matches('/').to_string();
        let proxy = ProxyContext {
            base_url_root,
            access_token,
            profile: auth.profile.clone(),
        };
        server::run_flow(
            server::FlowKind::MfaSetup,
            proxy,
            server::PrefillData::MfaSetup(prefill),
        )
        .await?
    } else {
        let prefill_json = pairing::prefill_mfa_setup(&prefill);
        pairing::run_display_once_pairing(auth, pairing::PairingFlow::MfaSetup, prefill_json)
            .await?
    };

    match outcome {
        WizardOutcome::MfaSetupAcknowledged(ack) => {
            attract_terminal("NyxID wizard complete");
            eprintln!("✓ MFA enrollment complete. Recovery codes were shown in the browser.");
            eprintln!("  Factor ID: {}", ack.factor_id);
            eprintln!("  Save the recovery codes from the browser tab — they aren't shown again.");
            Ok(())
        }
        WizardOutcome::Cancelled => {
            attract_terminal("NyxID wizard cancelled");
            eprintln!("✗ MFA wizard cancelled.");
            eprintln!("  If the browser showed recovery codes, MFA was enabled on the server. Run");
            eprintln!("  `nyxid mfa status` to confirm; if you didn't save the codes, disable");
            eprintln!("  and re-run setup to mint fresh ones.");
            std::process::exit(1);
        }
        WizardOutcome::TimedOut => {
            attract_terminal("NyxID wizard timed out");
            eprintln!("✗ MFA wizard timed out.");
            eprintln!("  If the browser showed recovery codes, MFA was enabled on the server. Run");
            eprintln!("  `nyxid mfa status` to confirm.");
            std::process::exit(1);
        }
        _ => Err(anyhow!(
            "internal: mfa-setup wizard returned unexpected outcome"
        )),
    }
}

/// Print a terminal summary for a pairing picked up via
/// `nyxid pairing resume`. The CLI doesn't have the rich CLI-side
/// context the original command had (resolved id-or-name, display
/// labels), so for create-flow outcomes we do a best-effort fetch
/// of the resource's name via `try_fetch_display_name`. Falls
/// through silently on any failure — the ID-only line is always
/// rendered. Field-allowlist discipline preserved: only non-secret
/// identifiers from the ack and the fetched name are printed; never
/// `format!("{ack:?}")`.
pub async fn print_resume_summary(
    auth: &crate::cli::AuthArgs,
    outcome: &WizardOutcome,
    base_url: &str,
) {
    match outcome {
        WizardOutcome::AiKeyPaired(ack) => {
            let pseudo = serde_json::json!({
                "slug": ack.slug,
                "label": ack.label,
            });
            print_wizard_summary(&pseudo, base_url);
        }
        WizardOutcome::ApiKeyCreateAcknowledged(ack) => {
            let display_name =
                try_fetch_display_name(auth, &format!("/api-keys/{}", ack.api_key_id), "name")
                    .await;
            eprintln!("✓ API key created. New value was shown in the browser.");
            if let Some(name) = display_name {
                eprintln!("  Name: {name}");
            }
            eprintln!("  ID: {}", ack.api_key_id);
            eprintln!("  Set as environment variable:");
            eprintln!("    export NYXID_API_KEY=\"<value-from-browser>\"");
        }
        WizardOutcome::ServiceAccountCreateAcknowledged(ack) => {
            let display_name = try_fetch_display_name(
                auth,
                &format!("/admin/service-accounts/{}", ack.service_account_id),
                "name",
            )
            .await;
            eprintln!("✓ Service account created. The client_secret was shown in the browser.");
            if let Some(name) = display_name {
                eprintln!("  Name: {name}");
            }
            eprintln!("  ID: {}", ack.service_account_id);
            eprintln!("  Save the client_secret from the browser tab — it isn't shown again.");
        }
        WizardOutcome::DeveloperAppCreateAcknowledged(ack) => {
            let display_name = try_fetch_display_name(
                auth,
                &format!("/developer/oauth-clients/{}", ack.developer_app_id),
                "client_name",
            )
            .await;
            eprintln!(
                "✓ Developer app created. The confidential client_secret was shown in the browser."
            );
            if let Some(name) = display_name {
                eprintln!("  Name: {name}");
            }
            eprintln!("  ID: {}", ack.developer_app_id);
            eprintln!("  Save the client_secret from the browser tab — it isn't shown again.");
        }
        WizardOutcome::MfaSetupAcknowledged(ack) => {
            eprintln!("✓ MFA enrollment complete. Recovery codes were shown in the browser.");
            eprintln!("  Factor ID: {}", ack.factor_id);
            eprintln!("  Save the recovery codes from the browser tab — they aren't shown again.");
        }
        WizardOutcome::NodeRegisterAcknowledged(ack) => {
            eprintln!("✓ Registration token generated. New value was shown in the browser.");
            eprintln!("  Token ID: {}", ack.token_id);
            eprintln!("  Register a node with:");
            eprintln!(
                "    nyxid node register --token <token-from-browser> --url ws://<server>/api/v1/nodes/ws"
            );
        }
        WizardOutcome::RotationAcknowledged(ack) => {
            eprintln!("✓ Rotation complete. New value was shown in the browser.");
            eprintln!("  Resource ID: {}", ack.resource_id);
            eprintln!("  The previous credential is now revoked.");
        }
        WizardOutcome::AiKeyCompleted(_) => {
            // Cross-transport artifact — local-server wizard only.
            // Not reachable from `pairing resume`, but keep the
            // match exhaustive.
        }
        WizardOutcome::Cancelled => {
            eprintln!("✗ Pairing was cancelled.");
        }
        WizardOutcome::TimedOut => {
            eprintln!("✗ Pairing expired before it was completed. Run the original command again.");
        }
    }
}

/// Returns true when the CLI is running somewhere we can reasonably
/// open a local browser for the wizard. False on SSH / explicit opt-out
/// / Linux without DISPLAY/WAYLAND, in which case the caller falls
/// through to the remote-pairing transport (see
/// [`is_browser_flow_eligible`]) — or ultimately to the scripted
/// stdin path when the user opts out entirely.
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

/// Returns true when the caller should route through the unified
/// wizard-or-pairing path instead of falling through to the scripted
/// stdin prompts. The wizard helper itself picks the local-browser
/// flavor when `is_wizard_eligible` says a browser can launch, and
/// the remote-pairing fallback otherwise (agent bash tool, SSH
/// session, Docker container, etc.).
///
/// The predicate distinguishes three environments:
///
/// 1. **Fully headless** (agent bash tools, subprocess wrappers,
///    SSH without display, CI containers) — stdin is NOT a TTY.
///    The scripted fallback would hang on the first missing-arg
///    prompt, so route through the browser / remote-pairing path
///    unconditionally. This is the main agent-use-case: the agent
///    relays the printed URL + code to the user, the user completes
///    the wizard on a phone or desktop, and the CLI polls for the
///    ack. Users who are scripted but DO have all args can opt out
///    of the pairing detour with `NYXID_NO_WIZARD=1` or `--terminal`.
///
/// 2. **Interactive stdin + piped stdout**
///    (`nyxid api-key create > key.txt`, `| jq ...`) — the user is
///    clearly scripting output but can still answer prompts.
///    Fall through to the stdin-prompt path so redirection keeps
///    working without the user learning any flags.
///
/// 3. **Interactive stdin + interactive stdout** — normal
///    foreground use; route to the wizard.
///
/// `NYXID_NO_WIZARD=1` forces the scripted path regardless of TTY
/// state, and `--no-wait` at the call site always chooses remote
/// pairing for agents that want a resumable handoff.
pub fn is_browser_flow_eligible() -> bool {
    // Explicit opt-out — same env var as the local-wizard predicate.
    if std::env::var_os("NYXID_NO_WIZARD").is_some() {
        return false;
    }
    // Fully headless (no stdin TTY): scripted path can't prompt,
    // so the wizard/remote-pairing path is the only way the
    // command can complete without the caller re-running with
    // every flag supplied manually.
    if !std::io::IsTerminal::is_terminal(&std::io::stdin()) {
        return true;
    }
    // Interactive stdin — scripted path works. Route to wizard
    // only when stdout is ALSO a TTY; a piped/redirected stdout
    // means the user is scripting output and expects the
    // stdin-prompt path (existing `> key.txt` / `| jq` patterns).
    std::io::IsTerminal::is_terminal(&std::io::stdout())
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

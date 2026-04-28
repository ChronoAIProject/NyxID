//! Local axum server for the wizard.
//!
//! Serves an embedded SPA from `127.0.0.1:<ephemeral>`, handles the
//! lifecycle endpoints (heartbeat / cancel / complete / status), and
//! proxies a narrow allowlist of backend requests with the user's bearer
//! token attached server-side.

use std::{
    collections::HashSet,
    net::SocketAddr,
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, anyhow};
use axum::{
    Json, Router,
    body::Body,
    extract::{Request, State},
    http::{HeaderMap, HeaderValue, Method, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{any, get, post},
};
use rand::RngCore;
use reqwest::Client as ReqwestClient;
use rust_embed::RustEmbed;
use serde_json::{Value, json};
use tokio::{
    net::TcpListener,
    sync::{Notify, oneshot},
};

use super::{
    ApiKeyCreateAckPayload, ApiKeyCreatePrefill, NodeRegisterAckPayload, NodeRegisterPrefill,
    ProxyContext, RotatePrefill, RotationAckPayload, WizardOutcome, WizardPrefill,
};

/// Which flow is running. Each flow gets its own allowlist and default
/// page body. v2 shipped only `AiKey`; v3 added the two rotation
/// (DisplayOnce-shaped) flows; v3.1 adds the create-side pair.
#[derive(Debug, Clone, Copy)]
pub enum FlowKind {
    AiKey,
    ApiKeyRotate,
    NodeRotateToken,
    NodeRegisterToken,
    ApiKeyCreate,
}

impl FlowKind {
    /// True for flows whose terminal panel renders a one-time secret.
    /// The heartbeat watchdog uses a longer dead-after window for these
    /// so users have time to alt-tab into a password manager without
    /// the CLI killing itself mid-save. (Previously named
    /// `is_rotation`; v3.1 generalized since `register-token` and
    /// `api-key create` have the same alt-tab risk despite not being
    /// rotations.)
    fn is_display_once(&self) -> bool {
        matches!(
            self,
            FlowKind::ApiKeyRotate
                | FlowKind::NodeRotateToken
                | FlowKind::NodeRegisterToken
                | FlowKind::ApiKeyCreate
        )
    }

    /// String slug embedded in the served HTML's `<meta name="wizard-flow">`
    /// tag. wizard.js dispatches its top-level state machine on this.
    fn slug(&self) -> &'static str {
        match self {
            FlowKind::AiKey => "ai-key",
            FlowKind::ApiKeyRotate => "api-key-rotate",
            FlowKind::NodeRotateToken => "node-rotate-token",
            FlowKind::NodeRegisterToken => "node-register-token",
            FlowKind::ApiKeyCreate => "api-key-create",
        }
    }
}

/// Prefill data routed into the wizard's URL query string. Per-flow
/// shapes — `WizardPrefill` for ai-key, `RotatePrefill` for the two
/// rotation flows, `NodeRegisterPrefill` for `node register-token`,
/// `ApiKeyCreatePrefill` for `api-key create`. Kept as an enum so
/// `server::run_flow`'s signature stays single-typed while each flow's
/// prefill can grow independently.
pub enum PrefillData {
    AiKey(WizardPrefill),
    Rotate(RotatePrefill),
    NodeRegister(NodeRegisterPrefill),
    ApiKeyCreate(ApiKeyCreatePrefill),
}

/// Static assets live under `src/wizard/assets/` and are baked into the binary.
#[derive(RustEmbed)]
#[folder = "src/wizard/assets/"]
struct Assets;

/// Overall ceiling. If a heartbeat is never missed but the user never
/// completes, this kills the session so a walked-away tab eventually frees.
const WIZARD_MAX_DURATION: Duration = Duration::from_secs(1800); // 30 min
/// Wait for the first browser heartbeat before arming the active watchdog.
/// The first successful beat proves the HTML loaded, the JS bundle ran, and
/// the browser can reach this local server. Keep this wide enough for slow
/// browser startup and manual URL copy/paste when auto-open fails.
const FIRST_HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(30);
/// Browser pings `/api/proxy/heartbeat` every 1.2 s, but the CLI should not
/// cancel at the same cadence as the frontend's quick disconnect banner
/// (~3.6 s). A 20 s active window keeps heartbeat as the source of truth
/// while tolerating ordinary browser scheduling delays.
const HEARTBEAT_DEAD_AFTER: Duration = Duration::from_secs(20);
/// DisplayOnce flows render a one-time secret. Keep a separately named window
/// so those flows can tolerate a longer alt-tab into a password manager.
const HEARTBEAT_DEAD_AFTER_ROTATION: Duration = Duration::from_secs(60);
/// How often the CLI checks the last-heartbeat timestamp. 500 ms is
/// tight enough that a watchdog-triggered exit fires promptly after the
/// current timeout window expires.
const HEARTBEAT_CHECK_INTERVAL: Duration = Duration::from_millis(500);

fn heartbeat_watchdog_dead(
    started_at: Instant,
    last_heartbeat: Option<Instant>,
    dead_after: Duration,
    now: Instant,
) -> bool {
    match last_heartbeat {
        Some(t) => now.saturating_duration_since(t) > dead_after,
        None => now.saturating_duration_since(started_at) > FIRST_HEARTBEAT_TIMEOUT,
    }
}

/// A single entry in the proxy allowlist. `path_template` supports literal
/// segments and `:param` placeholders (e.g. `/api/v1/catalog/:slug`). The
/// request path must have the same segment count and every non-placeholder
/// segment must match literally. Query strings are forwarded untouched.
///
/// `body_fields` is the whitelist of permitted top-level JSON keys in the
/// request body. An empty slice means "body must be empty". Any key not
/// in the whitelist causes a 400 — a second layer on top of CSP/CSRF so
/// a compromised wizard page can't smuggle extra fields (e.g. `target_org_id`,
/// `identity_propagation_mode`) through to `POST /keys`.
#[derive(Debug, Clone)]
struct ProxyRoute {
    method: Method,
    path_template: &'static str,
    body_fields: &'static [&'static str],
}

impl ProxyRoute {
    fn matches(&self, method: &Method, path: &str) -> bool {
        if self.method != method {
            return false;
        }
        let want: Vec<&str> = self
            .path_template
            .trim_start_matches('/')
            .split('/')
            .collect();
        let got: Vec<&str> = path.trim_start_matches('/').split('/').collect();
        if want.len() != got.len() {
            return false;
        }
        for (w, g) in want.iter().zip(got.iter()) {
            if w.starts_with(':') {
                if g.is_empty() {
                    return false;
                }
                continue;
            }
            if w != g {
                return false;
            }
        }
        true
    }
}

fn allowlist_for(kind: FlowKind) -> Vec<ProxyRoute> {
    match kind {
        // AI-key flow: catalog read, SimpleKey create, plus OAuth and
        // device-code authorization + poll. Mirrors what the scripted
        // `nyxid service add` uses via cli/src/commands/service.rs.
        FlowKind::AiKey => vec![
            ProxyRoute {
                method: Method::GET,
                path_template: "/api/v1/catalog",
                body_fields: &[],
            },
            ProxyRoute {
                method: Method::GET,
                path_template: "/api/v1/catalog/:slug",
                body_fields: &[],
            },
            // Unified key creation. Fields are the intersection of what
            // the wizard UI actually sends (see `buildCreateBody` in
            // wizard.js) — NOT the full `CreateKeyRequest` surface. Keeps
            // privileged fields like `target_org_id`, `identity_*`,
            // `forward_access_token`, `inject_delegation_token`, and SSH
            // flags out of reach of a compromised wizard page.
            //
            // `node_id` is whitelisted because the shared React confirm
            // panel forwards the CLI's `via_node` prefill to the backend
            // on `nyxid service add --via-node …`. Without it the
            // wizard would create an unbound service, breaking node-only
            // / self-hosted setups.
            ProxyRoute {
                method: Method::POST,
                path_template: "/api/v1/keys",
                body_fields: &[
                    "service_slug",
                    "credential",
                    "label",
                    "endpoint_url",
                    "slug",
                    "auth_method",
                    "auth_key_name",
                    "openapi_spec_url",
                    "node_id",
                ],
            },
            // Needed to poll placeholder key status during OAuth/device-code.
            ProxyRoute {
                method: Method::GET,
                path_template: "/api/v1/keys/:key_id",
                body_fields: &[],
            },
            // NOTE: DELETE /api/v1/keys/:key_id is intentionally NOT in
            // the allowlist. Placeholder cleanup now routes through the
            // wizard-server-local `POST /api/proxy/abandon-placeholder`
            // endpoint, which performs a conditional GET-then-DELETE
            // server-side so a key that just flipped to `active` while
            // the user was abandoning can't be revoked accidentally.
            // OAuth app credentials (client_id, client_secret) stored on the
            // provider entry. Required up-front for providers whose
            // credential_mode is "user" or "both".
            //
            // Both GET and PUT are in the allowlist: the React OAuth sub-
            // flow reads existing credentials first to decide whether to
            // render the client ID/secret form, then writes them if the
            // user provides them. Without the GET entry, `user` and
            // `both` credential_mode providers can't be connected via
            // the Mode A wizard.
            ProxyRoute {
                method: Method::GET,
                path_template: "/api/v1/providers/:provider_id/credentials",
                body_fields: &[],
            },
            ProxyRoute {
                method: Method::PUT,
                path_template: "/api/v1/providers/:provider_id/credentials",
                body_fields: &["client_id", "client_secret", "label"],
            },
            // OAuth: GET returns { authorization_url }.
            ProxyRoute {
                method: Method::GET,
                path_template: "/api/v1/providers/:provider_id/connect/oauth",
                body_fields: &[],
            },
            // Device code: initiate returns { user_code, verification_uri,
            // state, interval }; poll returns status and/or access_token.
            ProxyRoute {
                method: Method::POST,
                path_template: "/api/v1/providers/:provider_id/connect/device-code/initiate",
                body_fields: &[],
            },
            ProxyRoute {
                method: Method::POST,
                path_template: "/api/v1/providers/:provider_id/connect/device-code/poll",
                body_fields: &["state"],
            },
        ],
        // API key rotation. Two routes only:
        //   GET  /api-keys/:id           — sanity-read for the confirm panel's
        //                                  display name + prefix.
        //   POST /api-keys/:id/rotate    — empty body. Backend's `rotate_key`
        //                                  takes no JSON body; the body
        //                                  validator rejects anything beyond
        //                                  `{}`.
        // No DELETE: rotation is server-atomic, there is no placeholder
        // to clean up. The `pending_keys` sniff at line ~656 only fires
        // on `POST /api/v1/keys` so it's inert here.
        FlowKind::ApiKeyRotate => vec![
            ProxyRoute {
                method: Method::GET,
                path_template: "/api/v1/api-keys/:key_id",
                body_fields: &[],
            },
            ProxyRoute {
                method: Method::POST,
                path_template: "/api/v1/api-keys/:key_id/rotate",
                body_fields: &[],
            },
        ],
        // Node token rotation. Same shape as ApiKeyRotate. The backend
        // returns BOTH `auth_token` and `signing_secret` in the rotate
        // response; the wizard.js display-once panel renders both rows
        // and the .txt download bundles both with the `nyxid node rekey
        // ...` template line.
        FlowKind::NodeRotateToken => vec![
            ProxyRoute {
                method: Method::GET,
                path_template: "/api/v1/nodes/:node_id",
                body_fields: &[],
            },
            ProxyRoute {
                method: Method::POST,
                path_template: "/api/v1/nodes/:node_id/rotate-token",
                body_fields: &[],
            },
        ],
        // Node registration-token creation (v3.1). One route: POST with
        // just the node `name`. Body allowlist is tight — no metadata,
        // no target_org_id, no TTL override. The response carries the
        // one-time `token` (nyx_nreg_...) which the wizard renders in
        // the DisplayOnce panel; the browser then posts back only
        // `{ acknowledged, token_id }` to `/api/proxy/complete`.
        FlowKind::NodeRegisterToken => vec![ProxyRoute {
            method: Method::POST,
            path_template: "/api/v1/nodes/register-token",
            body_fields: &["name"],
        }],
        // API key creation (v3.1). Fields mirror what the wizard's
        // scope-picker panel can emit — not the full
        // `CreateApiKeyRequest` surface. Keeps privileged extras off
        // the wire even if the wizard page were compromised. Extra
        // reads: `/orgs` for the owner picker, `/keys` and
        // `/nodes` to populate the scope multi-selects lazily (only
        // fetched when the user picks "Select specific").
        FlowKind::ApiKeyCreate => vec![
            ProxyRoute {
                method: Method::GET,
                path_template: "/api/v1/orgs",
                body_fields: &[],
            },
            ProxyRoute {
                method: Method::GET,
                path_template: "/api/v1/keys",
                body_fields: &[],
            },
            ProxyRoute {
                method: Method::GET,
                path_template: "/api/v1/nodes",
                body_fields: &[],
            },
            ProxyRoute {
                method: Method::POST,
                path_template: "/api/v1/api-keys",
                body_fields: &[
                    "name",
                    "scopes",
                    "expires_at",
                    "allowed_service_ids",
                    "allowed_node_ids",
                    "allow_all_services",
                    "allow_all_nodes",
                    "rate_limit_per_second",
                    "rate_limit_burst",
                    "platform",
                    "callback_url",
                    "target_org_id",
                ],
            },
        ],
    }
}

#[derive(Clone)]
struct ServerState {
    csrf_token: Arc<String>,
    done_tx: Arc<tokio::sync::Mutex<Option<oneshot::Sender<WizardOutcome>>>>,
    shutdown: Arc<Notify>,
    started_at: Instant,
    last_heartbeat: Arc<tokio::sync::Mutex<Option<Instant>>>,
    proxy: Arc<ProxyContext>,
    allowlist: Arc<Vec<ProxyRoute>>,
    upstream: ReqwestClient,
    flow: FlowKind,
    /// Current access token. Starts as `proxy.access_token`, but gets
    /// replaced in-place when the backend returns 401 and we refresh
    /// via the saved refresh_token (mirrors ApiClient::try_refresh_token
    /// in cli/src/api.rs). Held under a Mutex so concurrent 401s from
    /// parallel proxy requests don't race the /auth/refresh call.
    access_token: Arc<tokio::sync::Mutex<String>>,
    /// Count of in-flight mutating proxy requests (POST/PUT/PATCH/DELETE).
    /// Incremented when we enter the proxy handler for a mutator, decremented
    /// when the upstream response resolves. `handle_cancel_unload` refuses
    /// to shut the server down while this is non-zero, closing the
    /// tab-close-mid-POST race Codex flagged.
    in_flight_mutations: Arc<std::sync::atomic::AtomicUsize>,
    /// Ephemeral TCP port bound on 127.0.0.1. Used to verify the
    /// `Origin` and `Host` headers match *this* server exactly, so
    /// another local process on a different port can't bounce through
    /// our proxy even if it passes CSRF.
    bound_port: u16,
    /// IDs of placeholder keys that the proxy has observed transitioning
    /// into `pending_auth` status. Populated by sniffing successful
    /// `POST /api/v1/keys` responses; drained on shutdown so abandoned
    /// OAuth / device-code attempts don't leave stale rows even when
    /// the browser never got a chance to fire a cleanup request
    /// (e.g. tab closed while POST /keys was still in flight).
    pending_keys: Arc<tokio::sync::Mutex<HashSet<String>>>,
    /// JSON-serialized prefill for the flow. Baked into
    /// `window.__WIZARD_BOOTSTRAP__.prefill` so the React bundle can
    /// render the right confirm panel with the right values on mount.
    /// Matches the shape of the per-kind TypeScript `*Prefill`
    /// interfaces in `frontend/src/pages/cli-pair/types.ts`.
    prefill: Arc<serde_json::Value>,
}

/// Mint a 32-byte random CSRF token, hex-encoded.
fn mint_csrf() -> String {
    let mut buf = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut buf);
    hex::encode(buf)
}

/// Constant-time compare of the CSRF header against the minted token.
fn csrf_ok(headers: &HeaderMap, expected: &str) -> bool {
    let provided = match headers.get("x-wizard-csrf") {
        Some(v) => v.as_bytes(),
        None => return false,
    };
    constant_time_eq::constant_time_eq(provided, expected.as_bytes())
}

/// Strict CSP template. The nonce placeholder is substituted per
/// request so the inline `<script>` / `<style>` tags emitted by the
/// Vite single-file bundle (and the bootstrap `<script>` injected at
/// serve time) are allowed, while any script the wizard page might
/// try to inject without a nonce is still blocked.
///
/// `strict-dynamic` means "a nonce-trusted script can load further
/// scripts without them needing their own nonce". Vite's bundle runs
/// a single module script that lazily creates child modules; without
/// strict-dynamic we'd need a nonce on every transitively-loaded
/// chunk, which is impractical for a React app.
const CSP_TEMPLATE: &str = "default-src 'none'; \
     script-src 'self' 'nonce-{NONCE}' 'strict-dynamic'; \
     style-src 'self' 'nonce-{NONCE}' 'unsafe-hashes'; \
     img-src 'self' data:; \
     connect-src 'self'; \
     font-src 'self' data:; \
     form-action 'none'; \
     frame-ancestors 'none'; \
     base-uri 'none'";

fn base_security_headers_with_nonce(nonce: &str) -> HeaderMap {
    let mut h = HeaderMap::new();
    let csp = CSP_TEMPLATE.replace("{NONCE}", nonce);
    h.insert(
        "content-security-policy",
        HeaderValue::from_str(&csp).unwrap_or(HeaderValue::from_static("")),
    );
    h.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    h.insert("x-frame-options", HeaderValue::from_static("DENY"));
    h.insert("referrer-policy", HeaderValue::from_static("no-referrer"));
    h.insert("cache-control", HeaderValue::from_static("no-store"));
    h
}

/// Legacy non-nonce headers for non-index responses (JSON APIs, etc.)
/// where CSP doesn't apply but the other hardening still does.
fn base_security_headers() -> HeaderMap {
    let mut h = HeaderMap::new();
    h.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    h.insert("x-frame-options", HeaderValue::from_static("DENY"));
    h.insert("referrer-policy", HeaderValue::from_static("no-referrer"));
    h.insert("cache-control", HeaderValue::from_static("no-store"));
    h
}

/// Mint a 128-bit random nonce, base64url-encoded. Used once per
/// serve_index response to authorize the bundle's inline script +
/// style + bootstrap script tags under the strict CSP.
fn mint_nonce() -> String {
    use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
    let mut buf = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut buf);
    URL_SAFE_NO_PAD.encode(buf)
}

async fn serve_index(State(state): State<ServerState>) -> Response {
    let raw = match Assets::get("index.html") {
        Some(a) => a,
        None => {
            return (StatusCode::INTERNAL_SERVER_ERROR, "index.html missing").into_response();
        }
    };
    let flow_name = state.flow.slug();
    let html_raw = std::str::from_utf8(raw.data.as_ref()).unwrap_or("");

    // Per-request CSP nonce — authorizes the single Vite-bundled
    // `<script type="module">`, any inline `<style>` tags emitted by
    // the single-file plugin, and the bootstrap script we inject
    // below.
    let nonce = mint_nonce();

    // Annotate every inline `<script>` / `<style>` tag with the
    // nonce so CSP accepts them. Mode A's bundle is built
    // deterministically by `frontend/vite.wizard.config.ts`; observed
    // shapes are `<script type="module" crossorigin>...</script>`
    // and `<style rel="stylesheet" crossorigin>...</style>` (the
    // `rel` + `crossorigin` attributes come from Vite's own
    // single-file inlining). Both with-attributes and without
    // forms need the nonce so the CSP `script-src` /
    // `style-src 'nonce-…'` tokens match.
    let annotated = html_raw
        .replace("<script ", &format!("<script nonce=\"{nonce}\" "))
        .replace("<script>", &format!("<script nonce=\"{nonce}\">"))
        .replace("<style ", &format!("<style nonce=\"{nonce}\" "))
        .replace("<style>", &format!("<style nonce=\"{nonce}\">"));

    // Bootstrap payload — flow name, CSRF token, backend base URL,
    // and per-flow prefill are baked into `window.__WIZARD_BOOTSTRAP__`
    // so the React bundle can render the right panel on mount.
    // base_url_root is the NyxID origin (e.g. https://nyx-api...).
    // It's not secret — the user already knows what backend they
    // logged into — and the browser needs it. We do NOT expose the
    // bearer token here; that stays in CLI process memory and is
    // attached to proxied requests server-side.
    let bootstrap_json = serde_json::json!({
        "flow": flow_name,
        "csrf": state.csrf_token.as_str(),
        "baseUrl": state.proxy.base_url_root,
        "context": "local",
        "prefill": state.prefill.as_ref(),
    });
    // HTML-safe JSON embedding. We splice the JSON directly into an
    // inline `<script>` as an object literal, so any prefill field
    // containing `</script>` or similar HTML-special characters would
    // terminate the tag before JSON.stringify even matters — a stored-
    // XSS primitive against the user's own browser, executable against
    // `/api/proxy/*` with their bearer token.
    //
    // Fix: replace the four HTML-significant characters (`<`, `>`, `&`)
    // plus the JS line-terminator characters (U+2028, U+2029, which
    // break a JS string literal but not a JSON one) with their
    // `\uXXXX` equivalents. These are valid inside a JS string literal
    // *and* inside a JSON string, so the payload round-trips.
    //
    // Ref: https://owasp.org/www-community/attacks/xss/ "Script Tag
    // Break", https://github.com/yahoo/serialize-javascript.
    let bootstrap_safe = bootstrap_json
        .to_string()
        .replace('<', r"\u003c")
        .replace('>', r"\u003e")
        .replace('&', r"\u0026")
        .replace('\u{2028}', r"\u2028")
        .replace('\u{2029}', r"\u2029");
    let bootstrap_script = format!(
        "<script nonce=\"{nonce}\">window.__WIZARD_BOOTSTRAP__ = {bootstrap_safe};</script>",
    );

    // Inject the bootstrap BEFORE the main module script so
    // `window.__WIZARD_BOOTSTRAP__` is defined by the time the
    // React entry reads it.
    let html = if let Some(idx) = annotated.find("<script nonce=") {
        let (before, after) = annotated.split_at(idx);
        format!("{before}{bootstrap_script}{after}")
    } else {
        // No script tag found — the bundle is malformed or the
        // injection pattern drifted. Fall back to appending to head
        // rather than returning an error so the user at least sees
        // something; the NoBootstrapFallback in wizard-entry.tsx
        // surfaces the problem.
        annotated.replace("</head>", &format!("{bootstrap_script}</head>"))
    };

    let mut headers = base_security_headers_with_nonce(&nonce);
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("text/html; charset=utf-8"),
    );
    (StatusCode::OK, headers, html).into_response()
}

async fn serve_asset(axum::extract::Path(name): axum::extract::Path<String>) -> Response {
    // Block path traversal but allow subdirectories (e.g. fonts/x.woff2).
    if name.split('/').any(|seg| seg == ".." || seg.is_empty()) {
        return StatusCode::NOT_FOUND.into_response();
    }
    let asset = match Assets::get(&name) {
        Some(a) => a,
        None => return StatusCode::NOT_FOUND.into_response(),
    };
    let ct = if name.ends_with(".css") {
        "text/css; charset=utf-8"
    } else if name.ends_with(".js") {
        "application/javascript; charset=utf-8"
    } else if name.ends_with(".html") {
        "text/html; charset=utf-8"
    } else if name.ends_with(".svg") {
        "image/svg+xml"
    } else if name.ends_with(".woff2") {
        "font/woff2"
    } else if name.ends_with(".woff") {
        "font/woff"
    } else {
        "application/octet-stream"
    };
    let mut headers = base_security_headers();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_str(ct).unwrap());
    (StatusCode::OK, headers, asset.data.into_owned()).into_response()
}

/// Validate the `Origin` header. When present it must point at *this*
/// server's exact loopback origin — not just any `127.0.0.1:*`. A
/// compromised neighbouring local process on a different port should
/// not pass the check.
///
/// Browsers frequently omit `Origin` on *same-origin GET* requests even when
/// custom headers are present (the main CSRF-defence path is the X-Wizard-CSRF
/// header, which browsers send faithfully). So we accept missing Origin on
/// GETs. On mutating methods we still require Origin + CSRF.
fn origin_matches(headers: &HeaderMap, port: u16) -> Option<bool> {
    headers.get(header::ORIGIN).map(|v| {
        let s = v.to_str().unwrap_or("");
        s == format!("http://127.0.0.1:{port}") || s == format!("http://localhost:{port}")
    })
}

/// Strict origin check for mutators: must be present AND match this port.
fn check_origin_strict(headers: &HeaderMap, port: u16) -> bool {
    origin_matches(headers, port).unwrap_or(false)
}

/// Relaxed origin check for reads: absent → allow, present → must match.
fn check_origin_relaxed(headers: &HeaderMap, port: u16) -> bool {
    origin_matches(headers, port).unwrap_or(true)
}

/// Every HTTP/1.1 browser request carries a `Host` header. Reject if
/// missing or not pointing at our exact bound port. Complements the
/// Origin check as a second layer against DNS-rebind attacks that
/// might pass Origin by forging the referring page.
fn check_host_exact(headers: &HeaderMap, port: u16) -> bool {
    let host = match headers.get(header::HOST).and_then(|v| v.to_str().ok()) {
        Some(h) => h,
        None => return false,
    };
    host == format!("127.0.0.1:{port}") || host == format!("localhost:{port}")
}

/// Combined Origin + Host check for mutating endpoints. Both must
/// match *this* server's exact loopback origin.
fn check_caller_strict(headers: &HeaderMap, port: u16) -> bool {
    check_origin_strict(headers, port) && check_host_exact(headers, port)
}

/// Combined check for read-only endpoints. Host is always required
/// and must match; Origin is optional (browsers omit on same-origin
/// GET) but when present must match.
fn check_caller_relaxed(headers: &HeaderMap, port: u16) -> bool {
    check_origin_relaxed(headers, port) && check_host_exact(headers, port)
}

/// `POST /api/proxy/complete` — browser tells the CLI the user has
/// acknowledged the wizard's terminal step.
///
/// Body parsing dispatches on `state.flow`:
///   - `AiKey`: keep the historical untyped `Value` shape. Only fields
///     in the printer's allowlist (`slug`/`label`/`proxy_url`) are read
///     downstream; nothing here is secret-shaped.
///   - `ApiKeyRotate`/`NodeRotateToken`: parse into the typed
///     `RotationAckPayload` (with `deny_unknown_fields`). A buggy or
///     compromised wizard page that tries to slip `full_key` /
///     `auth_token` / `signing_secret` into the body is rejected with
///     400 BEFORE it reaches CLI process memory at all. The struct's
///     `Debug` impl can also only print fields it holds, so a future
///     `tracing::debug!` of the outcome stays safe.
async fn handle_complete(
    State(state): State<ServerState>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    if !check_caller_strict(&headers, state.bound_port) {
        return (StatusCode::FORBIDDEN, "bad origin/host").into_response();
    }
    if !csrf_ok(&headers, &state.csrf_token) {
        return (StatusCode::FORBIDDEN, "bad csrf").into_response();
    }

    let outcome = match state.flow {
        FlowKind::AiKey => {
            // Empty body is allowed (legacy: callers sometimes posted no
            // body); fall back to Value::Null so `print_wizard_summary`
            // sees no slug and prints the generic "Wizard completed" line.
            let value: Value = if body.is_empty() {
                Value::Null
            } else {
                match serde_json::from_slice(&body) {
                    Ok(v) => v,
                    Err(e) => {
                        return (
                            StatusCode::BAD_REQUEST,
                            format!("complete: invalid JSON body: {e}"),
                        )
                            .into_response();
                    }
                }
            };
            WizardOutcome::AiKeyCompleted(value)
        }
        FlowKind::ApiKeyRotate | FlowKind::NodeRotateToken => {
            let payload: RotationAckPayload = match serde_json::from_slice(&body) {
                Ok(p) => p,
                Err(e) => {
                    // `deny_unknown_fields` surfaces here as a serde
                    // error mentioning the offending field. We
                    // INTENTIONALLY include the serde error message in
                    // the response (it names the unknown field, never
                    // its value) so a developer debugging a wizard.js
                    // bug sees what got rejected. We do NOT echo the
                    // body bytes themselves.
                    return (
                        StatusCode::BAD_REQUEST,
                        format!("complete: invalid rotation ack payload: {e}"),
                    )
                        .into_response();
                }
            };
            // Sanity-pin: the resource_id the browser sent must be a
            // bounded UUID-ish string. Cheap defense against a buggy
            // page sending a giant string we'd then format into the
            // terminal summary.
            if payload.resource_id.is_empty()
                || payload.resource_id.len() > 64
                || !is_uuid_like(&payload.resource_id)
            {
                return (StatusCode::BAD_REQUEST, "complete: bad resource_id").into_response();
            }
            // `acknowledged: true` is required — refusing `false` makes
            // the field load-bearing instead of cosmetic. The browser
            // posts true on ack-button click; a malformed/buggy page
            // posting `false` (or omitting the field, which serde
            // rejects via `deny_unknown_fields` semantics for missing
            // required fields) gets a 400 here.
            if !payload.acknowledged {
                return (
                    StatusCode::BAD_REQUEST,
                    "complete: acknowledged must be true",
                )
                    .into_response();
            }
            WizardOutcome::RotationAcknowledged(payload)
        }
        FlowKind::NodeRegisterToken => {
            let payload: NodeRegisterAckPayload = match serde_json::from_slice(&body) {
                Ok(p) => p,
                Err(e) => {
                    return (
                        StatusCode::BAD_REQUEST,
                        format!("complete: invalid node-register ack payload: {e}"),
                    )
                        .into_response();
                }
            };
            if payload.token_id.is_empty()
                || payload.token_id.len() > 64
                || !is_uuid_like(&payload.token_id)
            {
                return (StatusCode::BAD_REQUEST, "complete: bad token_id").into_response();
            }
            if !payload.acknowledged {
                return (
                    StatusCode::BAD_REQUEST,
                    "complete: acknowledged must be true",
                )
                    .into_response();
            }
            WizardOutcome::NodeRegisterAcknowledged(payload)
        }
        FlowKind::ApiKeyCreate => {
            let payload: ApiKeyCreateAckPayload = match serde_json::from_slice(&body) {
                Ok(p) => p,
                Err(e) => {
                    return (
                        StatusCode::BAD_REQUEST,
                        format!("complete: invalid api-key-create ack payload: {e}"),
                    )
                        .into_response();
                }
            };
            if payload.api_key_id.is_empty()
                || payload.api_key_id.len() > 64
                || !is_uuid_like(&payload.api_key_id)
            {
                return (StatusCode::BAD_REQUEST, "complete: bad api_key_id").into_response();
            }
            if !payload.acknowledged {
                return (
                    StatusCode::BAD_REQUEST,
                    "complete: acknowledged must be true",
                )
                    .into_response();
            }
            WizardOutcome::ApiKeyCreateAcknowledged(payload)
        }
    };
    signal_and_shutdown(state, outcome).await;
    (StatusCode::NO_CONTENT, base_security_headers()).into_response()
}

async fn handle_cancel(State(state): State<ServerState>, headers: HeaderMap) -> Response {
    if !check_caller_strict(&headers, state.bound_port) {
        return (StatusCode::FORBIDDEN, "bad origin/host").into_response();
    }
    if !csrf_ok(&headers, &state.csrf_token) {
        return (StatusCode::FORBIDDEN, "bad csrf").into_response();
    }
    signal_and_shutdown(state, WizardOutcome::Cancelled).await;
    (StatusCode::NO_CONTENT, base_security_headers()).into_response()
}

/// `navigator.sendBeacon` can't set custom headers, so the unload path is
/// treated as a soft cancel guarded only by Origin + short age. This is
/// intentionally weaker than the button-click cancel.
async fn handle_cancel_unload(State(state): State<ServerState>, headers: HeaderMap) -> Response {
    if !check_caller_strict(&headers, state.bound_port) {
        return (StatusCode::FORBIDDEN, "bad origin/host").into_response();
    }
    if state.started_at.elapsed() > WIZARD_MAX_DURATION {
        return (StatusCode::GONE, "too old").into_response();
    }
    // Don't kill the server if a mutating upstream request is currently in
    // flight. sendBeacon fires at tab-unload but an already-dispatched POST
    // to the backend will still complete server-side regardless of what we
    // do here; exiting the CLI with "cancelled" in that window creates an
    // orphan. Swallow the unload and let the in-flight request resolve
    // normally — the heartbeat watchdog will catch a truly dead tab.
    if state
        .in_flight_mutations
        .load(std::sync::atomic::Ordering::Acquire)
        > 0
    {
        return (StatusCode::CONFLICT, "busy").into_response();
    }
    signal_and_shutdown(state, WizardOutcome::Cancelled).await;
    (StatusCode::NO_CONTENT, base_security_headers()).into_response()
}

async fn handle_heartbeat(State(state): State<ServerState>, headers: HeaderMap) -> Response {
    if !check_caller_strict(&headers, state.bound_port) {
        return (StatusCode::FORBIDDEN, "bad origin/host").into_response();
    }
    if !csrf_ok(&headers, &state.csrf_token) {
        return (StatusCode::FORBIDDEN, "bad csrf").into_response();
    }
    *state.last_heartbeat.lock().await = Some(Instant::now());
    (StatusCode::NO_CONTENT, base_security_headers()).into_response()
}

async fn handle_status(State(state): State<ServerState>, headers: HeaderMap) -> Response {
    // GET: Origin may be omitted by the browser on same-origin requests.
    if !check_caller_relaxed(&headers, state.bound_port) {
        return (StatusCode::FORBIDDEN, "bad origin/host").into_response();
    }
    let body = json!({
        "state": "running",
        "uptime_s": state.started_at.elapsed().as_secs(),
    });
    let mut h = base_security_headers();
    h.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/json"),
    );
    (StatusCode::OK, h, body.to_string()).into_response()
}

/// Proxy handler. The browser hits `/api/proxy/api/v1/...`; we strip the
/// `/api/proxy` prefix, check the allowlist, attach the bearer token, and
/// forward to the NyxID backend. The response body + content-type are
/// returned to the browser. Other response headers (set-cookie, auth
/// hints) are deliberately not forwarded.
async fn handle_proxy(State(state): State<ServerState>, req: Request<Body>) -> Response {
    let method = req.method().clone();
    let uri = req.uri().clone();
    let headers = req.headers().clone();

    // Per-method origin enforcement: browsers omit Origin on same-origin GET
    // so we relax for reads. Mutations keep the strict check as a second
    // layer on top of CSRF. Host is always required and must match exactly.
    let caller_ok = if matches!(method, Method::GET | Method::HEAD) {
        check_caller_relaxed(&headers, state.bound_port)
    } else {
        check_caller_strict(&headers, state.bound_port)
    };
    if !caller_ok {
        return (StatusCode::FORBIDDEN, "bad origin/host").into_response();
    }
    if !csrf_ok(&headers, &state.csrf_token) {
        return (StatusCode::FORBIDDEN, "bad csrf").into_response();
    }

    // Strip `/api/proxy` to get the backend-relative path.
    let full_path = uri.path();
    let Some(backend_path) = full_path.strip_prefix("/api/proxy") else {
        return (StatusCode::NOT_FOUND, "not a proxy path").into_response();
    };
    if backend_path.is_empty() {
        return (StatusCode::NOT_FOUND, "empty proxy path").into_response();
    }

    // Allowlist check. Resolve the matching route so we can apply its
    // body-field whitelist below.
    let route = match state
        .allowlist
        .iter()
        .find(|r| r.matches(&method, backend_path))
    {
        Some(r) => r.clone(),
        None => {
            return (
                StatusCode::FORBIDDEN,
                format!("proxy: {} {} not allowed", method, backend_path),
            )
                .into_response();
        }
    };

    // Build the upstream URL. `base_url_root` has no trailing slash.
    let query = uri.query().map(|q| format!("?{q}")).unwrap_or_default();
    let upstream_url = format!("{}{}{}", state.proxy.base_url_root, backend_path, query);

    // Forward the body verbatim (M2 only has GETs so body is usually empty,
    // but the plumbing is generic).
    let body_bytes = match axum::body::to_bytes(req.into_body(), 1024 * 1024).await {
        Ok(b) => b,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                format!("reading request body: {e}"),
            )
                .into_response();
        }
    };

    // Per-route body whitelist. Defense-in-depth: CSP + CSRF already block
    // cross-origin callers, but if the wizard page itself were
    // compromised we don't want it smuggling extra JSON keys into
    // privileged endpoints (e.g. `target_org_id` on `POST /keys`). Body
    // must be empty or a JSON object whose keys are all in `body_fields`.
    if !body_bytes.is_empty() {
        let parsed: Value = match serde_json::from_slice(&body_bytes) {
            Ok(v) => v,
            Err(_) => {
                return (StatusCode::BAD_REQUEST, "body is not valid JSON").into_response();
            }
        };
        match parsed {
            Value::Object(obj) => {
                if route.body_fields.is_empty() && !obj.is_empty() {
                    return (
                        StatusCode::BAD_REQUEST,
                        format!("proxy: {method} {backend_path} does not accept a body"),
                    )
                        .into_response();
                }
                for key in obj.keys() {
                    if !route.body_fields.contains(&key.as_str()) {
                        return (
                            StatusCode::BAD_REQUEST,
                            format!("proxy: unexpected field '{key}' for {method} {backend_path}"),
                        )
                            .into_response();
                    }
                }
            }
            _ => {
                return (StatusCode::BAD_REQUEST, "body must be a JSON object").into_response();
            }
        }
    }

    // Track in-flight mutating requests so handle_cancel_unload can refuse
    // to shut the server down while a POST/PUT/PATCH/DELETE is mid-flight.
    let is_mutator = matches!(
        method,
        Method::POST | Method::PUT | Method::PATCH | Method::DELETE
    );
    struct InFlightGuard(Arc<std::sync::atomic::AtomicUsize>);
    impl Drop for InFlightGuard {
        fn drop(&mut self) {
            self.0.fetch_sub(1, std::sync::atomic::Ordering::Release);
        }
    }
    let _guard = if is_mutator {
        state
            .in_flight_mutations
            .fetch_add(1, std::sync::atomic::Ordering::AcqRel);
        Some(InFlightGuard(state.in_flight_mutations.clone()))
    } else {
        None
    };

    // Small helper so we can rebuild the upstream request with a fresh
    // token on 401 retry. Closure captures the shared pieces (method,
    // URL, CT header, body) and just takes the bearer.
    let ct_hdr = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let body_vec = if body_bytes.is_empty() {
        None
    } else {
        Some(body_bytes.to_vec())
    };
    let build_req = |token: &str| -> reqwest::RequestBuilder {
        let mut r = state
            .upstream
            .request(method.clone(), &upstream_url)
            .bearer_auth(token);
        if let Some(ct) = ct_hdr.as_deref() {
            r = r.header(header::CONTENT_TYPE, ct);
        }
        if let Some(b) = body_vec.as_ref() {
            r = r.body(b.clone());
        }
        r
    };

    let current_token = { state.access_token.lock().await.clone() };
    let first_resp = match build_req(&current_token).send().await {
        Ok(r) => r,
        Err(e) => {
            eprintln!("  proxy error ({method} {upstream_url}): {e}");
            return (
                StatusCode::BAD_GATEWAY,
                json!({ "error": "upstream unreachable", "detail": e.to_string() }).to_string(),
            )
                .into_response();
        }
    };

    // 401 → refresh access token via the saved refresh_token and retry
    // once. Mirrors ApiClient::try_refresh_token + retry pattern in
    // cli/src/api.rs::{get,post,put,patch,delete_empty}. Skipping refresh
    // (or on refresh failure) falls through to the original 401 so the
    // browser gets a real error instead of hanging.
    let upstream_resp = if first_resp.status() == reqwest::StatusCode::UNAUTHORIZED {
        match try_refresh_access_token(&state).await {
            Some(new_token) => match build_req(&new_token).send().await {
                Ok(retry) => retry,
                Err(_) => first_resp,
            },
            None => first_resp,
        }
    } else {
        first_resp
    };

    let status = upstream_resp.status();
    let upstream_ct = upstream_resp
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/octet-stream")
        .to_string();
    let body = match upstream_resp.bytes().await {
        Ok(b) => b,
        Err(e) => {
            return (
                StatusCode::BAD_GATEWAY,
                json!({ "error": "upstream body read failed", "detail": e.to_string() })
                    .to_string(),
            )
                .into_response();
        }
    };

    // Sniff key-lifecycle responses so the server can reliably clean up
    // abandoned `pending_auth` placeholders even when the browser never
    // gets the key_id back (e.g. tab closed between POST and response).
    //   - POST /api/v1/keys 2xx with status=="pending_auth" → track id.
    //   - GET /api/v1/keys/:id  2xx with status!="pending_auth" → untrack
    //     (the key is now active, failed, or revoked — cleanup no longer
    //     applies, and we must not delete an active key later).
    if status.is_success()
        && upstream_ct.starts_with("application/json")
        && let Ok(v) = serde_json::from_slice::<Value>(&body)
    {
        if method == Method::POST && backend_path == "/api/v1/keys" {
            if let (Some(id), Some("pending_auth")) = (
                v.get("id").and_then(|x| x.as_str()),
                v.get("status").and_then(|x| x.as_str()),
            ) {
                state.pending_keys.lock().await.insert(id.to_string());
            }
        } else if method == Method::GET
            && backend_path.starts_with("/api/v1/keys/")
            && !backend_path.contains("/bindings")
            && let (Some(id), Some(s)) = (
                v.get("id").and_then(|x| x.as_str()),
                v.get("status").and_then(|x| x.as_str()),
            )
            && s != "pending_auth"
        {
            state.pending_keys.lock().await.remove(id);
        }
    }

    let mut out_headers = base_security_headers();
    out_headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(&upstream_ct)
            .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream")),
    );
    (
        StatusCode::from_u16(status.as_u16()).unwrap_or(StatusCode::BAD_GATEWAY),
        out_headers,
        body,
    )
        .into_response()
}

/// Conditional abandon for a single placeholder key: `GET /keys/:id`, and
/// only issue `DELETE /keys/:id` if status is still `pending_auth`. Closes
/// the race where the user finished authorizing on the provider moments
/// before cancelling the wizard — an unconditional DELETE would revoke a
/// legitimately-active key. All errors are swallowed: this is best-effort
/// cleanup, and the backend TTL is the backstop.
async fn conditional_abandon_key(state: &ServerState, key_id: &str) {
    let base = state.proxy.base_url_root.trim_end_matches('/');
    let url = format!("{}/api/v1/keys/{}", base, key_id);
    let token = { state.access_token.lock().await.clone() };
    let key = match state.upstream.get(&url).bearer_auth(&token).send().await {
        Ok(r) if r.status().is_success() => r.json::<Value>().await.ok(),
        _ => None,
    };
    let still_pending = key
        .as_ref()
        .and_then(|v| v.get("status"))
        .and_then(|s| s.as_str())
        == Some("pending_auth");
    if still_pending {
        let _ = state.upstream.delete(&url).bearer_auth(&token).send().await;
    }
    state.pending_keys.lock().await.remove(key_id);
}

/// Best-effort access-token refresh via the saved refresh_token for this
/// profile. Mirrors `ApiClient::try_refresh_token` in `cli/src/api.rs`:
/// POST `{base}/api/v1/auth/refresh` → `{access_token, refresh_token}`,
/// persist via `crate::auth::save_tokens_for` (so subsequent CLI
/// commands also benefit), then update this server's mutex. Returns the
/// new access token on success, or `None` on any failure (no saved
/// refresh token, refresh endpoint 4xx/5xx, network error, malformed
/// body). Callers should treat `None` as "keep the original 401".
async fn try_refresh_access_token(state: &ServerState) -> Option<String> {
    let profile = state.proxy.profile.as_deref();
    let refresh_token = crate::auth::read_saved_refresh_token_for(profile)?;
    let url = format!(
        "{}/api/v1/auth/refresh",
        state.proxy.base_url_root.trim_end_matches('/')
    );
    let resp = state
        .upstream
        .post(&url)
        .json(&json!({ "refresh_token": refresh_token }))
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let tokens: Value = resp.json().await.ok()?;
    let new_access = tokens.get("access_token")?.as_str()?.to_string();
    let new_refresh = tokens.get("refresh_token")?.as_str()?.to_string();
    crate::auth::save_tokens_for(profile, &new_access, Some(&new_refresh)).ok()?;
    *state.access_token.lock().await = new_access.clone();
    eprintln!("  [wizard] refreshed expired access token for profile {profile:?}");
    Some(new_access)
}

/// `POST /api/proxy/abandon-placeholder` — client-triggered cleanup of a
/// pending OAuth / device-code placeholder. Replaces the direct
/// `DELETE /api/v1/keys/:id` path so the GET-then-conditional-DELETE
/// sequence runs server-side, eliminating the client-side race window
/// where the key transitions to `active` between the check and the delete.
async fn handle_abandon_placeholder(
    State(state): State<ServerState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    if !check_caller_strict(&headers, state.bound_port) {
        return (StatusCode::FORBIDDEN, "bad origin/host").into_response();
    }
    if !csrf_ok(&headers, &state.csrf_token) {
        return (StatusCode::FORBIDDEN, "bad csrf").into_response();
    }
    let key_id = match body.get("key_id").and_then(|v| v.as_str()) {
        Some(s) if !s.is_empty() && s.len() <= 64 && is_uuid_like(s) => s.to_string(),
        _ => {
            return (StatusCode::BAD_REQUEST, "invalid key_id").into_response();
        }
    };
    conditional_abandon_key(&state, &key_id).await;
    (StatusCode::NO_CONTENT, base_security_headers()).into_response()
}

/// Cheap format gate on the key_id we'll shove into a backend URL. Backend
/// uses UUID v4 strings for key IDs, so restrict to hex + dashes. Not a
/// security boundary on its own — the bearer is attached upstream — but it
/// prevents silly inputs (path traversal, `/..`, `;`, spaces).
fn is_uuid_like(s: &str) -> bool {
    s.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-')
}

/// Drain tracked placeholder keys, best-effort. Closes the
/// tab-close-before-POST-response race: even if the browser never learned
/// the key_id, we observed it in the proxy response and can still clean
/// up the `pending_auth` row server-side. Bounded timeout so a slow
/// backend can't hold the CLI open indefinitely. Called from both the
/// browser-driven shutdown path (`signal_and_shutdown`) and the
/// CLI-side abandonment paths (heartbeat watchdog, overall timeout,
/// Ctrl-C) — see issue #448.
async fn drain_pending_keys(state: &ServerState) {
    let drained: Vec<String> = {
        let mut set = state.pending_keys.lock().await;
        set.drain().collect()
    };
    if drained.is_empty() {
        return;
    }
    let cleanup = {
        let state = state.clone();
        async move {
            for key_id in drained {
                conditional_abandon_key(&state, &key_id).await;
            }
        }
    };
    let _ = tokio::time::timeout(Duration::from_secs(5), cleanup).await;
}

async fn signal_and_shutdown(state: ServerState, outcome: WizardOutcome) {
    drain_pending_keys(&state).await;

    let mut guard = state.done_tx.lock().await;
    if let Some(tx) = guard.take() {
        let _ = tx.send(outcome);
    }
    state.shutdown.notify_waiters();
}

/// Build the query string for the initial browser URL so prefill values
/// are present on page load. Only non-empty fields are emitted. Per-flow
/// shapes — ai-key uses slug/label/via_node/endpoint_url; rotation flows
/// use resource_id + display_name.
fn prefill_query(prefill: &PrefillData) -> String {
    let mut parts = Vec::new();
    let push_opt = |parts: &mut Vec<String>, k: &str, v: &Option<String>| {
        if let Some(val) = v.as_deref()
            && !val.is_empty()
        {
            parts.push(format!("{}={}", k, urlencoding::encode(val)));
        }
    };
    let push = |parts: &mut Vec<String>, k: &str, v: &str| {
        if !v.is_empty() {
            parts.push(format!("{}={}", k, urlencoding::encode(v)));
        }
    };
    match prefill {
        PrefillData::AiKey(p) => {
            push_opt(&mut parts, "slug", &p.slug);
            push_opt(&mut parts, "label", &p.label);
            push_opt(&mut parts, "via_node", &p.via_node);
            push_opt(&mut parts, "endpoint_url", &p.endpoint_url);
            // Issue #414 — custom-mode definitional fields. The SPA
            // primarily reads these out of `__WIZARD_BOOTSTRAP__.prefill`
            // (see `prefill_to_json` below); we also emit them here so
            // the URL is self-describing for any consumer that reads
            // query params (e.g. legacy wizard.js, debug tools).
            if p.custom {
                parts.push("custom=1".to_string());
            }
            push_opt(&mut parts, "custom_slug", &p.custom_slug);
            push_opt(&mut parts, "auth_method", &p.auth_method);
            push_opt(&mut parts, "auth_key_name", &p.auth_key_name);
        }
        PrefillData::Rotate(p) => {
            push(&mut parts, "resource_id", &p.resource_id);
            push(&mut parts, "display_name", &p.display_name);
        }
        PrefillData::NodeRegister(p) => {
            push_opt(&mut parts, "name", &p.name);
        }
        PrefillData::ApiKeyCreate(p) => {
            push_opt(&mut parts, "name", &p.name);
            push_opt(&mut parts, "platform", &p.platform);
            push_opt(&mut parts, "scopes", &p.scopes);
            if let Some(d) = p.expires_in_days {
                parts.push(format!("expires_in_days={d}"));
            }
            if p.allow_all_services {
                parts.push("allow_all_services=1".to_string());
            }
            if p.allow_all_nodes {
                parts.push("allow_all_nodes=1".to_string());
            }
            push_opt(&mut parts, "allowed_services", &p.allowed_services_csv);
            push_opt(&mut parts, "allowed_nodes", &p.allowed_nodes_csv);
            push_opt(&mut parts, "callback_url", &p.callback_url);
            push_opt(&mut parts, "org_id", &p.org_id);
        }
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!("?{}", parts.join("&"))
    }
}

/// Build the JSON payload baked into `window.__WIZARD_BOOTSTRAP__.prefill`.
/// Mirrors the per-kind TypeScript `*Prefill` interfaces declared in
/// `frontend/src/pages/cli-pair/types.ts` so the React bundle's
/// confirm panels can consume it unchanged. Only fields the React
/// panels actually read are emitted — keep the surface narrow.
fn prefill_to_json(prefill: &PrefillData) -> serde_json::Value {
    use serde_json::{Map, Value};
    let mut obj: Map<String, Value> = Map::new();
    let put_opt = |obj: &mut Map<String, Value>, k: &str, v: &Option<String>| {
        if let Some(val) = v.as_deref()
            && !val.is_empty()
        {
            obj.insert(k.to_string(), Value::String(val.to_string()));
        }
    };
    let put_str = |obj: &mut Map<String, Value>, k: &str, v: &str| {
        if !v.is_empty() {
            obj.insert(k.to_string(), Value::String(v.to_string()));
        }
    };
    match prefill {
        PrefillData::AiKey(p) => {
            put_opt(&mut obj, "slug", &p.slug);
            put_opt(&mut obj, "label", &p.label);
            put_opt(&mut obj, "via_node", &p.via_node);
            put_opt(&mut obj, "endpoint_url", &p.endpoint_url);
            // Issue #414 — the SPA's `AiKeyConfirm` reads these to
            // skip the catalog grid (`prefill.custom === true`) and
            // pre-populate the custom-service form. See
            // `frontend/src/components/cli-wizard/ai-key-confirm-panel.tsx`.
            // `custom: false` is omitted to keep the catalog flow's
            // bootstrap byte-identical (matches `prefill_ai_key`'s
            // pairing-transport semantics).
            if p.custom {
                obj.insert("custom".to_string(), Value::Bool(true));
            }
            put_opt(&mut obj, "custom_slug", &p.custom_slug);
            put_opt(&mut obj, "auth_method", &p.auth_method);
            put_opt(&mut obj, "auth_key_name", &p.auth_key_name);
        }
        PrefillData::Rotate(p) => {
            put_str(&mut obj, "resource_id", &p.resource_id);
            put_str(&mut obj, "display_name", &p.display_name);
        }
        PrefillData::NodeRegister(p) => {
            put_opt(&mut obj, "name", &p.name);
        }
        PrefillData::ApiKeyCreate(p) => {
            put_opt(&mut obj, "name", &p.name);
            put_opt(&mut obj, "platform", &p.platform);
            put_opt(&mut obj, "scopes", &p.scopes);
            if let Some(d) = p.expires_in_days {
                obj.insert(
                    "expires_in_days".to_string(),
                    Value::Number(serde_json::Number::from(d)),
                );
            }
            obj.insert(
                "allow_all_services".to_string(),
                Value::Bool(p.allow_all_services),
            );
            obj.insert(
                "allow_all_nodes".to_string(),
                Value::Bool(p.allow_all_nodes),
            );
            put_opt(&mut obj, "allowed_services_csv", &p.allowed_services_csv);
            put_opt(&mut obj, "allowed_nodes_csv", &p.allowed_nodes_csv);
            put_opt(&mut obj, "callback_url", &p.callback_url);
            put_opt(&mut obj, "org_id", &p.org_id);
        }
    }
    Value::Object(obj)
}

/// Flow runner. Binds, serves, opens the browser, waits for exit. The
/// `prefill` enum carries flow-specific URL-query state — see
/// `PrefillData` and `prefill_query`.
pub async fn run_flow(
    kind: FlowKind,
    proxy: ProxyContext,
    prefill: PrefillData,
) -> Result<WizardOutcome> {
    let csrf = mint_csrf();
    let (done_tx, done_rx) = oneshot::channel::<WizardOutcome>();
    let shutdown = Arc::new(Notify::new());

    // connect_timeout caps initial TCP+TLS handshake. timeout caps the full
    // request including response body read, which was a Codex-surfaced bug:
    // without a total timeout, a slow backend strands the browser with
    // disabled buttons and the only escape is tab-close (which then races
    // with the in-flight POST — see handle_cancel_unload + busy_flag below).
    let upstream = reqwest::Client::builder()
        .user_agent(crate::api::CLI_USER_AGENT)
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(60))
        .build()
        .context("building upstream HTTP client for wizard proxy")?;

    // Bind first (port is resolved before we spawn or open the browser) to
    // fix v1 gap #1 (server-spawn race). We also need the bound port inside
    // ServerState so the Origin/Host checks can validate an *exact* match.
    let listener = TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)))
        .await
        .context("binding wizard server to 127.0.0.1:0")?;
    let addr = listener
        .local_addr()
        .context("reading wizard server local addr")?;

    let initial_token = proxy.access_token.clone();
    let prefill_json = prefill_to_json(&prefill);
    let state = ServerState {
        csrf_token: Arc::new(csrf),
        done_tx: Arc::new(tokio::sync::Mutex::new(Some(done_tx))),
        shutdown: shutdown.clone(),
        started_at: Instant::now(),
        last_heartbeat: Arc::new(tokio::sync::Mutex::new(None)),
        proxy: Arc::new(proxy),
        allowlist: Arc::new(allowlist_for(kind)),
        upstream,
        flow: kind,
        access_token: Arc::new(tokio::sync::Mutex::new(initial_token)),
        in_flight_mutations: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
        bound_port: addr.port(),
        pending_keys: Arc::new(tokio::sync::Mutex::new(HashSet::new())),
        prefill: Arc::new(prefill_json),
    };

    let app = Router::new()
        .route("/wizard", get(serve_index))
        .route("/", get(serve_index))
        .route("/assets/{*name}", get(serve_asset))
        .route("/api/proxy/complete", post(handle_complete))
        .route("/api/proxy/cancel", post(handle_cancel))
        .route("/api/proxy/cancel-unload", post(handle_cancel_unload))
        .route("/api/proxy/heartbeat", post(handle_heartbeat))
        .route("/api/proxy/status", get(handle_status))
        .route(
            "/api/proxy/abandon-placeholder",
            post(handle_abandon_placeholder),
        )
        // Catch-all proxy: /api/proxy/<anything>. The path here MUST come
        // after the lifecycle routes so exact matches win.
        .route("/api/proxy/{*rest}", any(handle_proxy))
        .with_state(state.clone());
    let url = format!(
        "http://127.0.0.1:{}/wizard{}",
        addr.port(),
        prefill_query(&prefill),
    );

    let shutdown_rx = shutdown.clone();
    let server_handle = tokio::spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                shutdown_rx.notified().await;
            })
            .await
    });

    // Tell the user what we're doing and open the browser.
    // `NYXID_WIZARD_NO_OPEN=1` skips the browser launch (used by
    // automated validation and CI smoke tests).
    eprintln!("→ Opening {url} … (Ctrl-C to cancel)");
    eprintln!("  Waiting for browser …");
    if std::env::var_os("NYXID_WIZARD_NO_OPEN").is_none() {
        if let Err(e) = open::that(&url) {
            eprintln!("  Couldn't auto-open browser: {e}");
            eprintln!("  Visit the URL above manually.");
        }
    } else {
        eprintln!("  (NYXID_WIZARD_NO_OPEN set — not opening a browser)");
    }

    // Heartbeat watchdog: wait for the browser's first heartbeat before
    // arming active liveness. After that, heartbeat is the source of truth,
    // but the CLI timeout is intentionally more tolerant than the frontend's
    // quick disconnect warning.
    let watchdog_state = state.clone();
    let watchdog_shutdown = shutdown.clone();
    let (watchdog_tx, watchdog_rx) = oneshot::channel::<()>();
    // Per-flow dead-after window. DisplayOnce flows render a one-time
    // secret; keep the branch explicit even though both windows are
    // currently equal.
    let dead_after = if kind.is_display_once() {
        HEARTBEAT_DEAD_AFTER_ROTATION
    } else {
        HEARTBEAT_DEAD_AFTER
    };
    let watchdog_handle = tokio::spawn(async move {
        let tx = watchdog_tx;
        loop {
            tokio::select! {
                _ = watchdog_shutdown.notified() => return,
                _ = tokio::time::sleep(HEARTBEAT_CHECK_INTERVAL) => {}
            }
            let last = *watchdog_state.last_heartbeat.lock().await;
            let dead = heartbeat_watchdog_dead(
                watchdog_state.started_at,
                last,
                dead_after,
                Instant::now(),
            );
            if dead {
                let _ = tx.send(());
                return;
            }
        }
    });

    // Wait for: completion signal, OR overall ceiling, OR watchdog (dead
    // heartbeat), OR Ctrl-C. The non-`done_rx` branches must drain
    // `pending_keys` themselves — the browser never reached `/cancel` or
    // `/cancel-unload` to call `signal_and_shutdown` for us, so without
    // this the placeholder service stays in `pending_auth` forever
    // (issue #448).
    let outcome = tokio::select! {
        v = done_rx => {
            v.map_err(|_| anyhow!("wizard completion channel closed unexpectedly"))?
        }
        _ = watchdog_rx => {
            eprintln!("  Browser stopped responding (tab closed?) — cancelling.");
            drain_pending_keys(&state).await;
            WizardOutcome::Cancelled
        }
        _ = tokio::time::sleep(WIZARD_MAX_DURATION) => {
            drain_pending_keys(&state).await;
            WizardOutcome::TimedOut
        }
        _ = tokio::signal::ctrl_c() => {
            drain_pending_keys(&state).await;
            WizardOutcome::Cancelled
        }
    };
    watchdog_handle.abort();

    // Ensure graceful shutdown fires even if we hit the timeout/ctrl-c paths.
    shutdown.notify_waiters();
    let _ = server_handle.await;

    Ok(outcome)
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        Json as AxumJson, Router,
        extract::{Path, State as AxumState},
        routing::get,
    };
    use std::sync::{
        Mutex as StdMutex,
        atomic::{AtomicUsize, Ordering},
    };

    #[derive(Clone, Default)]
    struct MockBackend {
        deletes: Arc<StdMutex<Vec<String>>>,
        gets: Arc<AtomicUsize>,
    }

    async fn mock_get_key(
        AxumState(mock): AxumState<MockBackend>,
        Path(id): Path<String>,
    ) -> AxumJson<Value> {
        mock.gets.fetch_add(1, Ordering::SeqCst);
        // ID convention: anything starting with "pending-" reports as
        // pending_auth; anything else as active. Lets the test drive
        // both branches of conditional_abandon_key.
        let status = if id.starts_with("pending-") {
            "pending_auth"
        } else {
            "active"
        };
        AxumJson(json!({ "id": id, "status": status }))
    }

    async fn mock_delete_key(
        AxumState(mock): AxumState<MockBackend>,
        Path(id): Path<String>,
    ) -> StatusCode {
        mock.deletes.lock().unwrap().push(id);
        StatusCode::NO_CONTENT
    }

    /// Spin up a tiny axum mock backend. Returns (base_url, mock state).
    async fn spawn_mock() -> (String, MockBackend) {
        let mock = MockBackend::default();
        let app = Router::new()
            .route(
                "/api/v1/keys/{id}",
                get(mock_get_key).delete(mock_delete_key),
            )
            .with_state(mock.clone());
        let listener = TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)))
            .await
            .expect("bind mock");
        let addr = listener.local_addr().expect("local addr");
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        (format!("http://127.0.0.1:{}", addr.port()), mock)
    }

    fn make_state(base_url: String, initial_keys: Vec<&str>) -> ServerState {
        let (done_tx, _done_rx) = oneshot::channel::<WizardOutcome>();
        let mut set = HashSet::new();
        for k in initial_keys {
            set.insert(k.to_string());
        }
        ServerState {
            csrf_token: Arc::new(String::from("test-csrf")),
            done_tx: Arc::new(tokio::sync::Mutex::new(Some(done_tx))),
            shutdown: Arc::new(Notify::new()),
            started_at: Instant::now(),
            last_heartbeat: Arc::new(tokio::sync::Mutex::new(None)),
            proxy: Arc::new(ProxyContext {
                base_url_root: base_url,
                access_token: "test-token".into(),
                profile: None,
            }),
            allowlist: Arc::new(Vec::new()),
            upstream: ReqwestClient::builder()
                .timeout(Duration::from_secs(2))
                .build()
                .unwrap(),
            flow: FlowKind::AiKey,
            access_token: Arc::new(tokio::sync::Mutex::new("test-token".into())),
            in_flight_mutations: Arc::new(AtomicUsize::new(0)),
            bound_port: 0,
            pending_keys: Arc::new(tokio::sync::Mutex::new(set)),
            prefill: Arc::new(Value::Null),
        }
    }

    #[test]
    fn heartbeat_watchdog_waits_for_first_heartbeat_timeout() {
        let started = Instant::now();

        assert!(
            !heartbeat_watchdog_dead(
                started,
                None,
                HEARTBEAT_DEAD_AFTER,
                started + FIRST_HEARTBEAT_TIMEOUT - Duration::from_millis(1),
            ),
            "watchdog must not enforce the active heartbeat window before the first heartbeat"
        );
        assert!(
            heartbeat_watchdog_dead(
                started,
                None,
                HEARTBEAT_DEAD_AFTER,
                started + FIRST_HEARTBEAT_TIMEOUT + Duration::from_millis(1),
            ),
            "watchdog should give up if the browser never sends an initial heartbeat"
        );
    }

    #[test]
    fn heartbeat_watchdog_uses_active_timeout_after_first_heartbeat() {
        let started = Instant::now();
        let first_heartbeat = started + Duration::from_secs(1);

        assert!(
            !heartbeat_watchdog_dead(
                started,
                Some(first_heartbeat),
                HEARTBEAT_DEAD_AFTER,
                first_heartbeat + HEARTBEAT_DEAD_AFTER - Duration::from_millis(1),
            ),
            "active heartbeat timeout should be independent of startup time"
        );
        assert!(
            heartbeat_watchdog_dead(
                started,
                Some(first_heartbeat),
                HEARTBEAT_DEAD_AFTER,
                first_heartbeat + HEARTBEAT_DEAD_AFTER + Duration::from_millis(1),
            ),
            "watchdog should cancel after the active heartbeat window is missed"
        );
    }

    /// Issue #448 fix: drain_pending_keys must empty the HashSet so a
    /// later wizard cancellation path (watchdog / timeout / Ctrl-C)
    /// can't leave behind stale `pending_auth` placeholder services.
    #[tokio::test]
    async fn drain_pending_keys_empties_the_set() {
        let (base_url, _mock) = spawn_mock().await;
        let state = make_state(base_url, vec!["pending-1", "pending-2"]);

        drain_pending_keys(&state).await;

        assert!(
            state.pending_keys.lock().await.is_empty(),
            "drain must leave pending_keys empty"
        );
    }

    /// Safety: drain must NOT delete a key that has flipped to `active`
    /// in the time between the placeholder creation and the wizard
    /// being abandoned (race where the user finished authorizing
    /// moments before closing the tab). Only `pending_auth` keys
    /// should be DELETEd.
    #[tokio::test]
    async fn drain_pending_keys_only_deletes_pending_auth() {
        let (base_url, mock) = spawn_mock().await;
        let state = make_state(
            base_url,
            vec!["pending-keep", "active-keep", "pending-other"],
        );

        drain_pending_keys(&state).await;

        let deleted: Vec<String> = {
            let mut v = mock.deletes.lock().unwrap().clone();
            v.sort();
            v
        };
        assert_eq!(
            deleted,
            vec!["pending-keep".to_string(), "pending-other".to_string()],
            "only pending_auth keys should be DELETEd; got {deleted:?}"
        );
        assert!(state.pending_keys.lock().await.is_empty());
    }

    /// Empty set is a no-op — must not call the backend at all.
    #[tokio::test]
    async fn drain_pending_keys_empty_is_noop() {
        let (base_url, mock) = spawn_mock().await;
        let state = make_state(base_url, vec![]);

        drain_pending_keys(&state).await;

        assert_eq!(mock.gets.load(Ordering::SeqCst), 0);
        assert!(mock.deletes.lock().unwrap().is_empty());
    }
}

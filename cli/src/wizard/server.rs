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

use super::{ProxyContext, RotatePrefill, RotationAckPayload, WizardOutcome, WizardPrefill};

/// Which flow is running. Each flow gets its own allowlist and default
/// page body. v2 shipped only `AiKey`; v3 added the two rotation
/// (DisplayOnce-shaped) flows.
#[derive(Debug, Clone, Copy)]
pub enum FlowKind {
    AiKey,
    ApiKeyRotate,
    NodeRotateToken,
}

impl FlowKind {
    /// True for flows whose Step 3 panel renders a one-time secret. The
    /// heartbeat watchdog uses a longer dead-after window for these so
    /// users have time to alt-tab into a password manager without the
    /// CLI killing itself mid-save.
    fn is_rotation(&self) -> bool {
        matches!(self, FlowKind::ApiKeyRotate | FlowKind::NodeRotateToken)
    }

    /// String slug embedded in the served HTML's `<meta name="wizard-flow">`
    /// tag. wizard.js dispatches its top-level state machine on this.
    fn slug(&self) -> &'static str {
        match self {
            FlowKind::AiKey => "ai-key",
            FlowKind::ApiKeyRotate => "api-key-rotate",
            FlowKind::NodeRotateToken => "node-rotate-token",
        }
    }
}

/// Prefill data routed into the wizard's URL query string. Per-flow
/// shapes — `WizardPrefill` for ai-key (slug/label/via_node/endpoint_url),
/// `RotatePrefill` for the two rotation flows (resource_id/display_name).
/// Kept as an enum so server::run_flow's signature stays single-typed
/// while each flow's prefill can grow independently.
pub enum PrefillData {
    AiKey(WizardPrefill),
    Rotate(RotatePrefill),
}

/// Static assets live under `src/wizard/assets/` and are baked into the binary.
#[derive(RustEmbed)]
#[folder = "src/wizard/assets/"]
struct Assets;

/// Overall ceiling. If a heartbeat is never missed but the user never
/// completes, this kills the session so a walked-away tab eventually frees.
const WIZARD_MAX_DURATION: Duration = Duration::from_secs(1800); // 30 min
/// Browser pings `/api/proxy/heartbeat` every 10 s; miss two in a row
/// and the CLI treats the tab as dead. Grace: 22 s (2 × 10 + jitter).
const HEARTBEAT_DEAD_AFTER: Duration = Duration::from_secs(22);
/// Rotation flows render a one-time secret on Step 3. Users may alt-tab
/// into a password manager / vault / paper to copy it; chrome throttles
/// `setInterval` in hidden tabs and visibility-change pauses our JS
/// heartbeat sender entirely. A wider window means casual alt-tabs
/// don't trip the watchdog and bury the panel under a "disconnected"
/// overlay. Caveat (called out in CLI_WIZARD_V3.md §3): >60 s of
/// silence STILL triggers cancel — this is a heuristic, not a fix.
const HEARTBEAT_DEAD_AFTER_ROTATION: Duration = Duration::from_secs(60);
/// Grace period at startup before we start enforcing the heartbeat dead
/// line. Lets the browser actually load the page.
const HEARTBEAT_STARTUP_GRACE: Duration = Duration::from_secs(8);
/// How often the CLI checks the last-heartbeat timestamp.
const HEARTBEAT_CHECK_INTERVAL: Duration = Duration::from_secs(2);

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

/// Strict CSP: self only, no remote anything, no eval, no framing.
const CSP: &str = "default-src 'none'; script-src 'self'; style-src 'self'; \
                   img-src 'self' data:; connect-src 'self'; font-src 'self'; \
                   form-action 'none'; frame-ancestors 'none'; base-uri 'none'";

fn base_security_headers() -> HeaderMap {
    let mut h = HeaderMap::new();
    h.insert("content-security-policy", HeaderValue::from_static(CSP));
    h.insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    h.insert("x-frame-options", HeaderValue::from_static("DENY"));
    h.insert("referrer-policy", HeaderValue::from_static("no-referrer"));
    h.insert("cache-control", HeaderValue::from_static("no-store"));
    h
}

async fn serve_index(State(state): State<ServerState>) -> Response {
    let raw = match Assets::get("wizard.html") {
        Some(a) => a,
        None => {
            return (StatusCode::INTERNAL_SERVER_ERROR, "wizard.html missing").into_response();
        }
    };
    let flow_name = state.flow.slug();
    // base_url_root is the NyxID origin (e.g. https://nyx-api.chrono-ai.fun).
    // It's not secret — the user already knows what backend they logged into
    // — and the browser needs it to render a real proxy URL on Step 3
    // instead of a placeholder. We do NOT expose the bearer token here;
    // that stays in CLI process memory.
    let html = std::str::from_utf8(raw.data.as_ref())
        .unwrap_or("")
        .replace("{{CSRF_TOKEN}}", &state.csrf_token)
        .replace("{{FLOW}}", flow_name)
        .replace("{{BASE_URL}}", &state.proxy.base_url_root);

    let mut headers = base_security_headers();
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

async fn signal_and_shutdown(state: ServerState, outcome: WizardOutcome) {
    // Drain tracked placeholder keys before the CLI exits. Closes the
    // tab-close-before-POST-response race: even if the browser never
    // learned the key_id, we observed it in the proxy response and can
    // still best-effort clean it up. Bounded timeout so a slow backend
    // can't hold the CLI open indefinitely after the user cancels.
    let drained: Vec<String> = {
        let mut set = state.pending_keys.lock().await;
        set.drain().collect()
    };
    if !drained.is_empty() {
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
        }
        PrefillData::Rotate(p) => {
            push(&mut parts, "resource_id", &p.resource_id);
            push(&mut parts, "display_name", &p.display_name);
        }
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!("?{}", parts.join("&"))
    }
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

    // Heartbeat watchdog: if the browser stops pinging /api/proxy/heartbeat
    // for longer than HEARTBEAT_DEAD_AFTER (after a startup grace window),
    // we treat the tab as closed and cancel.
    let watchdog_state = state.clone();
    let watchdog_shutdown = shutdown.clone();
    let (watchdog_tx, watchdog_rx) = oneshot::channel::<()>();
    // Per-flow dead-after window. Rotation flows render a one-time
    // secret; users may alt-tab into a password manager mid-save and
    // browsers throttle hidden-tab `setInterval` heartbeats. Cap is
    // still bounded (60s) so a truly-dead tab still gets cleaned up.
    let dead_after = if kind.is_rotation() {
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
            if watchdog_state.started_at.elapsed() < HEARTBEAT_STARTUP_GRACE {
                continue;
            }
            let last = *watchdog_state.last_heartbeat.lock().await;
            let dead = match last {
                Some(t) => t.elapsed() > dead_after,
                None => watchdog_state.started_at.elapsed() > HEARTBEAT_STARTUP_GRACE + dead_after,
            };
            if dead {
                let _ = tx.send(());
                return;
            }
        }
    });

    // Wait for: completion signal, OR overall ceiling, OR watchdog (dead
    // heartbeat), OR Ctrl-C.
    let outcome = tokio::select! {
        v = done_rx => {
            v.map_err(|_| anyhow!("wizard completion channel closed unexpectedly"))?
        }
        _ = watchdog_rx => {
            eprintln!("  Browser stopped responding (tab closed?) — cancelling.");
            WizardOutcome::Cancelled
        }
        _ = tokio::time::sleep(WIZARD_MAX_DURATION) => {
            WizardOutcome::TimedOut
        }
        _ = tokio::signal::ctrl_c() => {
            WizardOutcome::Cancelled
        }
    };
    watchdog_handle.abort();

    // Ensure graceful shutdown fires even if we hit the timeout/ctrl-c paths.
    shutdown.notify_waiters();
    let _ = server_handle.await;

    Ok(outcome)
}

//! Local axum server for the wizard.
//!
//! Serves an embedded SPA from `127.0.0.1:<ephemeral>`, handles the
//! lifecycle endpoints (heartbeat / cancel / complete / status), and
//! proxies a narrow allowlist of backend requests with the user's bearer
//! token attached server-side.

use std::{
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

use super::{ProxyContext, WizardOutcome, WizardPrefill};

/// Which flow is running. Each flow gets its own allowlist and default
/// page body. M2 only has `AiKey`.
#[derive(Debug, Clone, Copy)]
pub enum FlowKind {
    AiKey,
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
/// Grace period at startup before we start enforcing the heartbeat dead
/// line. Lets the browser actually load the page.
const HEARTBEAT_STARTUP_GRACE: Duration = Duration::from_secs(8);
/// How often the CLI checks the last-heartbeat timestamp.
const HEARTBEAT_CHECK_INTERVAL: Duration = Duration::from_secs(2);

/// A single entry in the proxy allowlist. `path_template` supports literal
/// segments and `:param` placeholders (e.g. `/api/v1/catalog/:slug`). The
/// request path must have the same segment count and every non-placeholder
/// segment must match literally. Query strings are forwarded untouched.
#[derive(Debug, Clone)]
struct ProxyRoute {
    method: Method,
    path_template: &'static str,
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
            },
            ProxyRoute {
                method: Method::GET,
                path_template: "/api/v1/catalog/:slug",
            },
            ProxyRoute {
                method: Method::POST,
                path_template: "/api/v1/keys",
            },
            // Needed to poll placeholder key status during OAuth/device-code.
            ProxyRoute {
                method: Method::GET,
                path_template: "/api/v1/keys/:key_id",
            },
            // OAuth: GET returns { authorization_url }.
            ProxyRoute {
                method: Method::GET,
                path_template: "/api/v1/providers/:provider_id/connect/oauth",
            },
            // Device code: initiate returns { user_code, verification_uri,
            // state, interval }; poll returns status and/or access_token.
            ProxyRoute {
                method: Method::POST,
                path_template: "/api/v1/providers/:provider_id/connect/device-code/initiate",
            },
            ProxyRoute {
                method: Method::POST,
                path_template: "/api/v1/providers/:provider_id/connect/device-code/poll",
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
    /// Count of in-flight mutating proxy requests (POST/PUT/PATCH/DELETE).
    /// Incremented when we enter the proxy handler for a mutator, decremented
    /// when the upstream response resolves. `handle_cancel_unload` refuses
    /// to shut the server down while this is non-zero, closing the
    /// tab-close-mid-POST race Codex flagged.
    in_flight_mutations: Arc<std::sync::atomic::AtomicUsize>,
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
    let flow_name = match state.flow {
        FlowKind::AiKey => "ai-key",
    };
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

/// Validate the `Origin` header. When present it must point at loopback.
///
/// Browsers frequently omit `Origin` on *same-origin GET* requests even when
/// custom headers are present (the main CSRF-defence path is the X-Wizard-CSRF
/// header, which browsers send faithfully). So we accept missing Origin on
/// GETs. On mutating methods we still require Origin + CSRF.
fn origin_matches(headers: &HeaderMap) -> Option<bool> {
    headers.get(header::ORIGIN).map(|v| {
        let s = v.to_str().unwrap_or("");
        s.starts_with("http://127.0.0.1:") || s.starts_with("http://localhost:")
    })
}

/// Strict origin check for mutators: must be present AND match.
fn check_origin_strict(headers: &HeaderMap) -> bool {
    origin_matches(headers).unwrap_or(false)
}

/// Relaxed origin check for reads: absent → allow, present → must match.
fn check_origin_relaxed(headers: &HeaderMap) -> bool {
    origin_matches(headers).unwrap_or(true)
}

async fn handle_complete(
    State(state): State<ServerState>,
    headers: HeaderMap,
    Json(body): Json<Value>,
) -> Response {
    if !check_origin_strict(&headers) {
        return (StatusCode::FORBIDDEN, "bad origin").into_response();
    }
    if !csrf_ok(&headers, &state.csrf_token) {
        return (StatusCode::FORBIDDEN, "bad csrf").into_response();
    }
    signal_and_shutdown(state, WizardOutcome::Completed(body)).await;
    (StatusCode::NO_CONTENT, base_security_headers()).into_response()
}

async fn handle_cancel(State(state): State<ServerState>, headers: HeaderMap) -> Response {
    if !check_origin_strict(&headers) {
        return (StatusCode::FORBIDDEN, "bad origin").into_response();
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
    if !check_origin_strict(&headers) {
        return (StatusCode::FORBIDDEN, "bad origin").into_response();
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
    if !check_origin_strict(&headers) {
        return (StatusCode::FORBIDDEN, "bad origin").into_response();
    }
    if !csrf_ok(&headers, &state.csrf_token) {
        return (StatusCode::FORBIDDEN, "bad csrf").into_response();
    }
    *state.last_heartbeat.lock().await = Some(Instant::now());
    (StatusCode::NO_CONTENT, base_security_headers()).into_response()
}

async fn handle_status(State(state): State<ServerState>, headers: HeaderMap) -> Response {
    // GET: Origin may be omitted by the browser on same-origin requests.
    if !check_origin_relaxed(&headers) {
        return (StatusCode::FORBIDDEN, "bad origin").into_response();
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
    // layer on top of CSRF.
    let origin_ok = if matches!(method, Method::GET | Method::HEAD) {
        check_origin_relaxed(&headers)
    } else {
        check_origin_strict(&headers)
    };
    if !origin_ok {
        return (StatusCode::FORBIDDEN, "bad origin").into_response();
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

    // Allowlist check.
    let allowed = state
        .allowlist
        .iter()
        .any(|r| r.matches(&method, backend_path));
    if !allowed {
        return (
            StatusCode::FORBIDDEN,
            format!("proxy: {} {} not allowed", method, backend_path),
        )
            .into_response();
    }

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

    let mut upstream_req = state
        .upstream
        .request(method.clone(), &upstream_url)
        .bearer_auth(&state.proxy.access_token);
    if let Some(ct) = headers.get(header::CONTENT_TYPE)
        && let Ok(ct) = ct.to_str()
    {
        upstream_req = upstream_req.header(header::CONTENT_TYPE, ct);
    }
    if !body_bytes.is_empty() {
        upstream_req = upstream_req.body(body_bytes.to_vec());
    }

    let upstream_resp = match upstream_req.send().await {
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

async fn signal_and_shutdown(state: ServerState, outcome: WizardOutcome) {
    let mut guard = state.done_tx.lock().await;
    if let Some(tx) = guard.take() {
        let _ = tx.send(outcome);
    }
    state.shutdown.notify_waiters();
}

/// Build the query string for the initial browser URL so prefill values
/// are present on page load. Only non-empty fields are emitted.
fn prefill_query(prefill: &WizardPrefill) -> String {
    let mut parts = Vec::new();
    let push = |parts: &mut Vec<String>, k: &str, v: &Option<String>| {
        if let Some(val) = v.as_deref()
            && !val.is_empty()
        {
            parts.push(format!("{}={}", k, urlencoding::encode(val)));
        }
    };
    push(&mut parts, "slug", &prefill.slug);
    push(&mut parts, "label", &prefill.label);
    push(&mut parts, "via_node", &prefill.via_node);
    push(&mut parts, "endpoint_url", &prefill.endpoint_url);
    if parts.is_empty() {
        String::new()
    } else {
        format!("?{}", parts.join("&"))
    }
}

/// Flow runner. Binds, serves, opens the browser, waits for exit.
pub async fn run_flow(
    kind: FlowKind,
    proxy: ProxyContext,
    prefill: WizardPrefill,
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
        in_flight_mutations: Arc::new(std::sync::atomic::AtomicUsize::new(0)),
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
        // Catch-all proxy: /api/proxy/<anything>. The path here MUST come
        // after the lifecycle routes so exact matches win.
        .route("/api/proxy/{*rest}", any(handle_proxy))
        .with_state(state.clone());

    // Bind first (port is resolved before we spawn or open the browser) to
    // fix v1 gap #1 (server-spawn race).
    let listener = TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], 0)))
        .await
        .context("binding wizard server to 127.0.0.1:0")?;
    let addr = listener
        .local_addr()
        .context("reading wizard server local addr")?;
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
                Some(t) => t.elapsed() > HEARTBEAT_DEAD_AFTER,
                None => {
                    watchdog_state.started_at.elapsed()
                        > HEARTBEAT_STARTUP_GRACE + HEARTBEAT_DEAD_AFTER
                }
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

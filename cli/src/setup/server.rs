use std::sync::Arc;

use anyhow::{Result, bail};
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{Html, IntoResponse};
use axum::routing::{any, get, post};
use axum::{Json, Router};
use serde::Deserialize;
use tokio::sync::{Mutex, oneshot};

use crate::api::ApiClient;

use super::integrations::IntegrationAdapter;
use super::proxy;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Shared state for the setup wizard's local axum server.
pub struct WizardState {
    /// Authenticated API client — reuses the CLI's saved Bearer token.
    /// Locked because `proxy_request` takes `&mut self` (token refresh).
    pub api_client: Mutex<ApiClient>,

    /// NyxID backend root URL (e.g. `https://auth.nyxid.dev`).
    pub base_url: String,

    /// Integration adapter providing wizard step config.
    pub integration: Box<dyn IntegrationAdapter>,

    /// Random hex CSRF token validated on `POST /api/complete`.
    pub csrf_state: String,

    /// One-shot channel to signal the CLI that setup is done.
    /// Wrapped in `Mutex<Option<…>>` so we can `.take()` it exactly once.
    pub completion_tx: Mutex<Option<oneshot::Sender<CompletionResult>>>,

    /// Port the server is listening on. Used by the proxy's origin check
    /// to reject requests from other localhost ports.
    pub port: u16,
}

/// Data returned from the wizard to the CLI process.
pub struct CompletionResult {
    pub slug: Option<String>,
    pub service_name: Option<String>,
    pub integration_name: String,
}

// ---------------------------------------------------------------------------
// Embedded HTML
// ---------------------------------------------------------------------------

const SETUP_PAGE_HTML: &str = include_str!("pages/setup.html");

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// `GET /` — serve the embedded wizard SPA.
async fn serve_page() -> Html<&'static str> {
    Html(SETUP_PAGE_HTML)
}

/// `GET /api/config` — return integration config + user info for the page.
async fn get_config(State(state): State<Arc<WizardState>>) -> impl IntoResponse {
    // Fetch user info via proxy (best-effort — page still works without it)
    let user_info = {
        let mut api = state.api_client.lock().await;
        api.get_value("/users/me").await.ok()
    };

    let user = user_info.map(|v| {
        serde_json::json!({
            "email": v.get("email").and_then(|e| e.as_str()).unwrap_or(""),
            "display_name": v.get("display_name").and_then(|e| e.as_str()).unwrap_or(""),
        })
    });

    Json(serde_json::json!({
        "integration": state.integration.config(),
        "csrf_state": state.csrf_state,
        "authenticated": true,
        "base_url": state.base_url,
        "user": user,
    }))
}

/// `POST /api/complete` — the page signals that setup is finished.
#[derive(Deserialize)]
struct CompleteRequest {
    csrf_state: String,
    slug: Option<String>,
    service_name: Option<String>,
}

/// Constant-time-ish CSRF check (both are short random hex, no secrets).
fn validate_csrf(provided: &str, expected: &str) -> Result<(), StatusCode> {
    if provided != expected {
        return Err(StatusCode::FORBIDDEN);
    }
    Ok(())
}

async fn handle_complete(
    State(state): State<Arc<WizardState>>,
    Json(body): Json<CompleteRequest>,
) -> impl IntoResponse {
    if let Err(status) = validate_csrf(&body.csrf_state, &state.csrf_state) {
        return (status, "Invalid CSRF state").into_response();
    }

    let tx = state.completion_tx.lock().await.take();
    match tx {
        Some(tx) => {
            let result = CompletionResult {
                slug: body.slug,
                service_name: body.service_name,
                integration_name: state.integration.display_name().to_string(),
            };
            let _ = tx.send(result);
            Json(serde_json::json!({ "ok": true })).into_response()
        }
        None => (StatusCode::CONFLICT, "Already completed").into_response(),
    }
}

// ---------------------------------------------------------------------------
// Router
// ---------------------------------------------------------------------------

fn build_router(state: Arc<WizardState>) -> Router {
    Router::new()
        .route("/", get(serve_page))
        .route("/api/config", get(get_config))
        .route("/api/proxy/{*path}", any(proxy::proxy_api))
        .route("/api/complete", post(handle_complete))
        .with_state(state)
}

// ---------------------------------------------------------------------------
// Server lifecycle
// ---------------------------------------------------------------------------

/// Timeout for the wizard server (5 minutes).
const WIZARD_TIMEOUT_SECS: u64 = 300;

/// Start the local setup wizard server, open the browser, and wait for
/// the user to complete the wizard (or timeout).
///
/// The `listener` is already bound to `127.0.0.1:{port}` — no re-bind needed,
/// which eliminates the TOCTOU race on the port number.
pub async fn run_wizard_server(
    state: Arc<WizardState>,
    listener: tokio::net::TcpListener,
    port: u16,
    completion_rx: oneshot::Receiver<CompletionResult>,
) -> Result<CompletionResult> {
    let router = build_router(Arc::clone(&state));

    let url = format!("http://127.0.0.1:{port}/");
    eprintln!("Opening setup wizard in browser...");
    eprintln!();
    eprintln!("If the browser does not open, visit:");
    eprintln!("  {url}");
    eprintln!();
    let _ = open::that(&url);

    eprintln!("Waiting for setup to complete...");

    // Shutdown signal for the axum server
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();

    // Spawn the server
    let server_handle = tokio::spawn(async move {
        axum::serve(listener, router)
            .with_graceful_shutdown(async {
                let _ = shutdown_rx.await;
            })
            .await
    });

    // Wait for completion or timeout
    let result = tokio::select! {
        res = completion_rx => {
            let _ = shutdown_tx.send(());
            res.map_err(|_| anyhow::anyhow!("Wizard closed unexpectedly"))?
        }
        _ = tokio::time::sleep(std::time::Duration::from_secs(WIZARD_TIMEOUT_SECS)) => {
            let _ = shutdown_tx.send(());
            bail!("Setup timed out after {} seconds", WIZARD_TIMEOUT_SECS);
        }
    };

    // Wait for server to finish cleanly
    let _ = server_handle.await;

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csrf_valid_token_passes() {
        assert!(validate_csrf("abc123", "abc123").is_ok());
    }

    #[test]
    fn csrf_wrong_token_rejected() {
        assert_eq!(
            validate_csrf("wrong", "abc123").unwrap_err(),
            StatusCode::FORBIDDEN
        );
    }

    #[test]
    fn csrf_empty_token_rejected() {
        assert_eq!(
            validate_csrf("", "abc123").unwrap_err(),
            StatusCode::FORBIDDEN
        );
    }
}

//! Per-request middleware that derives `TelemetryContext` from the
//! `X-NyxID-Client` and `X-NyxID-Client-Version` request headers and
//! stashes it in request extensions so handlers can read it at emit
//! time.
//!
//! The middleware runs on every route (authenticated or not) because
//! header-only derivation is safe and side-effect-free. The `surface
//! = "agent"` override for API-key-authenticated requests happens at
//! emit time inside [`crate::telemetry::emit_event`], not here —
//! `AuthUser` is an axum `FromRequestParts` extractor, not a
//! middleware-populated extension, so the decision naturally lives
//! with the handler that holds the `AuthUser`.
//!
//! See `docs/TELEMETRY.md` §5.1 + review-history notes on the
//! middleware/extractor reconciliation.

use axum::{extract::Request, middleware::Next, response::Response};

use crate::telemetry::TelemetryContext;

const CLIENT_HEADER: &str = "x-nyxid-client";
const CLIENT_VERSION_HEADER: &str = "x-nyxid-client-version";

/// Header-only `TelemetryContext` builder. Runs on every request.
pub async fn telemetry_mw(mut req: Request, next: Next) -> Response {
    let client = req
        .headers()
        .get(CLIENT_HEADER)
        .and_then(|v| v.to_str().ok());
    let version = req
        .headers()
        .get(CLIENT_VERSION_HEADER)
        .and_then(|v| v.to_str().ok())
        // Cap at 64 chars: semver strings are short; an absurdly long
        // header is likely malformed or an attacker probing.
        .filter(|v| v.len() <= 64);

    let ctx = TelemetryContext::from_headers(client, version);
    req.extensions_mut().insert(ctx);

    next.run(req).await
}

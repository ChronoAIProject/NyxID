//! Per-request telemetry context populated by `mw/telemetry.rs`.
//!
//! The middleware derives `surface` from the `X-NyxID-Client` header (see
//! docs/TELEMETRY.md §5.0 hot-swap contract). The `agent` override for
//! API-key authenticated requests happens at emit time inside
//! [`emit_event`] — not in the middleware — because `AuthUser` is an axum
//! extractor, not a request-extension, so handlers own the check.

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use std::convert::Infallible;

use super::TelemetryClient;
use super::schema::TelemetryEvent;

/// The subset of the request-scoped context relevant to event emission.
/// Cloned cheaply into each event.
#[derive(Clone, Debug)]
pub struct TelemetryContext {
    /// `ui | cli | mobile | sdk | backend` — server-derived from the
    /// `X-NyxID-Client` header. Handlers override to `"agent"` when the
    /// authenticated user carries an `api_key_id` (see [`emit_event`]).
    pub surface: &'static str,
    /// The client's self-declared semver, from `X-NyxID-Client-Version`.
    /// `None` when the header is absent or malformed.
    pub client_version: Option<String>,
}

impl Default for TelemetryContext {
    fn default() -> Self {
        Self {
            surface: "backend",
            client_version: None,
        }
    }
}

impl TelemetryContext {
    /// Build the context from the two `X-NyxID-Client*` headers. Unknown
    /// header values fall back to `"backend"` (internal / non-NyxID caller).
    pub fn from_headers(client_header: Option<&str>, version_header: Option<&str>) -> Self {
        let surface = match client_header {
            Some("ui") => "ui",
            Some("cli") => "cli",
            Some("mobile") => "mobile",
            Some("sdk") => "sdk",
            _ => "backend",
        };
        Self {
            surface,
            client_version: version_header.map(str::to_owned),
        }
    }
}

/// Fire-and-forget event emission helper. Handles the `agent` surface
/// override for API-key-authenticated requests, pulls `user_id` as the
/// distinct_id, and passes through to the underlying [`TelemetryClient`].
///
/// No-op when `client` is `None`, meaning telemetry is hard-off for this
/// process (no DSN configured). That is the production default in dev
/// and in any deploy that hasn't opted in.
///
/// Callers pass the event's domain type from [`TelemetryEvent`]; the
/// name/properties translation happens inside so emit sites stay terse.
pub fn emit_event(
    client: Option<&TelemetryClient>,
    auth_user_id: &str,
    api_key_id: Option<&str>,
    ctx: &TelemetryContext,
    event: TelemetryEvent,
) {
    let Some(client) = client else {
        return;
    };
    let surface = if api_key_id.is_some() {
        "agent"
    } else {
        ctx.surface
    };
    let ctx = TelemetryContext {
        surface,
        client_version: ctx.client_version.clone(),
    };
    client.track(auth_user_id, event, &ctx, api_key_id);
}

/// Axum extractor: return the request-scoped `TelemetryContext` if the
/// telemetry middleware stashed one, or `TelemetryContext::default()`
/// otherwise. Never errors. Handlers declare it in their signature:
///
/// ```ignore
/// pub async fn create_key(
///     State(state): State<AppState>,
///     auth: AuthUser,
///     tele: TelemetryContext,  // <-- this line
///     Json(body): Json<CreateKeyRequest>,
/// ) -> AppResult<Json<KeyResponse>> { ... }
/// ```
impl<S> FromRequestParts<S> for TelemetryContext
where
    S: Send + Sync,
{
    type Rejection = Infallible;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        Ok(parts
            .extensions
            .get::<TelemetryContext>()
            .cloned()
            .unwrap_or_default())
    }
}

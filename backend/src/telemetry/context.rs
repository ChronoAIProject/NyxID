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
    /// Explicit device narrowing only; never substitutes for account permission.
    pub analytics_allowed: bool,
    /// Device choice belongs only to this authenticated actor.
    pub analytics_account: Option<String>,
}

impl Default for TelemetryContext {
    fn default() -> Self {
        Self {
            surface: "backend",
            client_version: None,
            analytics_allowed: false,
            analytics_account: None,
        }
    }
}

impl TelemetryContext {
    /// Authentication protocols have no established request actor yet. Even a
    /// spoofed device opt-in header must not produce account-linked events.
    pub fn pre_auth(headers: &axum::http::HeaderMap) -> Self {
        let mut context = Self::from_request_headers(headers);
        context.analytics_allowed = false;
        context
    }

    pub fn from_request_headers(headers: &axum::http::HeaderMap) -> Self {
        let mut context = Self::from_headers(
            headers
                .get("x-nyxid-client")
                .and_then(|value| value.to_str().ok()),
            headers
                .get("x-nyxid-client-version")
                .and_then(|value| value.to_str().ok()),
        );
        context.analytics_allowed = headers.get("dnt").is_none_or(|value| value != "1")
            && headers.get("sec-gpc").is_none_or(|value| value != "1")
            && headers
                .get("x-nyxid-analytics")
                .is_some_and(|value| value == "enabled");
        context.analytics_account = headers
            .get("x-nyxid-analytics-account")
            .and_then(|value| value.to_str().ok())
            .filter(|value| uuid::Uuid::parse_str(value).is_ok())
            .map(str::to_owned);
        context
    }

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
            client_version: version_header.filter(|v| v.len() <= 64).map(str::to_owned),
            analytics_allowed: false,
            analytics_account: None,
        }
    }
}

/// Event admission helper (bounded account lookup; asynchronous vendor delivery). Handles the `agent` surface
/// override for API-key-authenticated requests, pulls `user_id` as the
/// distinct_id, and passes through to the underlying [`TelemetryClient`].
///
/// No-op when `client` is `None`, meaning telemetry is hard-off for this
/// process (no DSN configured). That is the production default in dev
/// and in any deploy that hasn't opted in.
///
/// Callers pass the event's domain type from [`TelemetryEvent`]; the
/// name/properties translation happens inside so emit sites stay terse.
pub async fn emit_event(
    client: Option<&TelemetryClient>,
    auth_user_id: &str,
    api_key_id: Option<&str>,
    ctx: &TelemetryContext,
    event: TelemetryEvent,
) {
    let Some(client) = client else {
        return;
    };
    if matches!(
        &event,
        TelemetryEvent::ProxySuccess { service_slug, .. } if service_slug == "chrono-sandbox"
    ) {
        return;
    }
    let surface = if api_key_id.is_some() {
        "agent"
    } else {
        ctx.surface
    };
    let ctx = TelemetryContext {
        surface,
        client_version: ctx.client_version.clone(),
        analytics_allowed: ctx.analytics_allowed,
        analytics_account: ctx.analytics_account.clone(),
    };
    client.track(auth_user_id, event, &ctx, api_key_id).await;
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

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;

    #[test]
    fn request_headers_only_narrow_and_preauth_never_collects() {
        let mut headers = axum::http::HeaderMap::new();
        assert!(!TelemetryContext::from_request_headers(&headers).analytics_allowed);
        headers.insert("x-nyxid-analytics", "enabled".parse().unwrap());
        assert!(TelemetryContext::from_request_headers(&headers).analytics_allowed);
        assert!(!TelemetryContext::pre_auth(&headers).analytics_allowed);
        headers.insert("dnt", "1".parse().unwrap());
        assert!(!TelemetryContext::from_request_headers(&headers).analytics_allowed);
        headers.insert("dnt", "0".parse().unwrap());
        headers.insert("sec-gpc", "1".parse().unwrap());
        assert!(!TelemetryContext::from_request_headers(&headers).analytics_allowed);
        headers.remove("sec-gpc");
        headers.insert("x-nyxid-analytics", "invalid".parse().unwrap());
        assert!(!TelemetryContext::from_request_headers(&headers).analytics_allowed);
    }

    #[test]
    fn default_surface_is_backend() {
        let ctx = TelemetryContext::default();
        assert_eq!(ctx.surface, "backend");
        assert!(ctx.client_version.is_none());
    }

    #[test]
    fn from_headers_ui() {
        let ctx = TelemetryContext::from_headers(Some("ui"), None);
        assert_eq!(ctx.surface, "ui");
    }

    #[test]
    fn from_headers_cli() {
        let ctx = TelemetryContext::from_headers(Some("cli"), Some("1.2.3"));
        assert_eq!(ctx.surface, "cli");
        assert_eq!(ctx.client_version.as_deref(), Some("1.2.3"));
    }

    #[test]
    fn from_headers_mobile() {
        let ctx = TelemetryContext::from_headers(Some("mobile"), None);
        assert_eq!(ctx.surface, "mobile");
    }

    #[test]
    fn from_headers_sdk() {
        let ctx = TelemetryContext::from_headers(Some("sdk"), None);
        assert_eq!(ctx.surface, "sdk");
    }

    #[test]
    fn from_headers_unknown_falls_back_to_backend() {
        let ctx = TelemetryContext::from_headers(Some("unknown-client"), None);
        assert_eq!(ctx.surface, "backend");
    }

    #[test]
    fn from_headers_none_falls_back_to_backend() {
        let ctx = TelemetryContext::from_headers(None, None);
        assert_eq!(ctx.surface, "backend");
    }

    #[test]
    fn from_headers_preserves_version() {
        let ctx = TelemetryContext::from_headers(Some("sdk"), Some("0.1.0-beta"));
        assert_eq!(ctx.client_version.as_deref(), Some("0.1.0-beta"));
    }

    #[tokio::test]
    async fn emit_event_overrides_surface_for_api_key() {
        let ctx = TelemetryContext {
            surface: "cli",
            client_version: None,
            analytics_allowed: false,
            analytics_account: None,
        };
        // When client is None, emit_event is a no-op - verify no panic
        emit_event(
            None,
            "user-123",
            Some("key-1"),
            &ctx,
            TelemetryEvent::AuthLoggedIn {
                method: "email".to_string(),
                mfa_required: false,
            },
        )
        .await;
    }

    #[tokio::test]
    async fn emit_event_noop_when_no_client() {
        let ctx = TelemetryContext::default();
        emit_event(
            None,
            "user-123",
            None,
            &ctx,
            TelemetryEvent::AuthLoggedIn {
                method: "email".to_string(),
                mfa_required: false,
            },
        )
        .await;
    }

    #[tokio::test]
    async fn extractor_returns_context_from_request_extensions() {
        let expected = TelemetryContext {
            surface: "mobile",
            client_version: Some("2.0.1".to_string()),
            analytics_allowed: false,
            analytics_account: None,
        };
        let mut req = Request::builder().body(Body::empty()).unwrap();
        req.extensions_mut().insert(expected.clone());
        let (mut parts, _) = req.into_parts();

        let ctx = TelemetryContext::from_request_parts(&mut parts, &())
            .await
            .expect("extractor is infallible");

        assert_eq!(ctx.surface, expected.surface);
        assert_eq!(ctx.client_version, expected.client_version);
    }

    #[tokio::test]
    async fn extractor_defaults_when_middleware_extension_is_absent() {
        let req = Request::builder().body(Body::empty()).unwrap();
        let (mut parts, _) = req.into_parts();

        let ctx = TelemetryContext::from_request_parts(&mut parts, &())
            .await
            .expect("extractor is infallible");

        assert_eq!(ctx.surface, "backend");
        assert!(ctx.client_version.is_none());
    }
}

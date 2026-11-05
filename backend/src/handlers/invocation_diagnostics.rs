//! Request-local target recording and response decoration. No bearer is stored
//! in session state, task locals, audit rows, metrics, or detached tasks.
use std::sync::{Arc, Mutex};

use axum::{
    body::Body,
    http::Request,
    response::{IntoResponse, Response},
};
use tokio_util::sync::CancellationToken;

use crate::{
    AppState,
    errors::AppResult,
    mw::auth::{AuthMethod, AuthUser},
    services::{
        audit_service::{self, AuditActor},
        invocation_diagnostic_service::{self as diagnostics, Diagnostic, Outcome},
        proxy_service::{self, CallerToken},
    },
    telemetry::{TelemetryContext, TelemetryEvent, emit_event},
};

/// Only populated with exact UserService IDs selected by the execution path.
/// Pool attempts overwrite it, so only the final selected member is diagnosed.
#[derive(Clone, Default)]
pub(crate) struct Target(Arc<Mutex<Option<String>>>);

impl Target {
    pub(crate) fn record(&self, id: Option<&str>) {
        *self.0.lock().unwrap_or_else(|e| e.into_inner()) = id.map(str::to_owned);
    }

    pub(crate) fn record_request(request: &Request<Body>, id: Option<&str>) {
        if let Some(target) = request.extensions().get::<Self>() {
            target.record(id);
        }
    }

    pub(crate) fn clear_extensions(extensions: &axum::http::Extensions) {
        if let Some(target) = extensions.get::<Self>() {
            target.record(None);
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct RequestContext {
    pub cancellation: CancellationToken,
    pub telemetry: TelemetryContext,
}

impl RequestContext {
    pub(crate) fn from_request(request: &Request<Body>) -> Self {
        Self {
            cancellation: crate::downstream_disconnect::request_cancellation(request),
            telemetry: request
                .extensions()
                .get::<TelemetryContext>()
                .cloned()
                .unwrap_or_else(|| TelemetryContext::from_request_headers(request.headers())),
        }
    }
}

pub(crate) struct HttpInvocation {
    target: Target,
    token: Option<CallerToken>,
    context: RequestContext,
}

#[derive(Clone, Copy)]
struct RetainedServiceCapacity;

pub(crate) fn suppress_for_retained_capacity(response: &mut Response) {
    if response.status().is_client_error() || response.status().is_server_error() {
        response.extensions_mut().insert(RetainedServiceCapacity);
    }
}

impl HttpInvocation {
    pub(crate) fn start(state: &AppState, request: &mut Request<Body>) -> Option<Self> {
        // Nested gateway/pool wrappers share the outer invocation's budget.
        if state.config.chrono_probe_url.is_none() || request.extensions().get::<Target>().is_some()
        {
            return None;
        }
        let target = Target::default();
        request.extensions_mut().insert(target.clone());
        Some(Self {
            target,
            token: proxy_service::caller_bearer_token_for_downstream(request.headers(), false),
            context: RequestContext::from_request(request),
        })
    }

    pub(crate) async fn finish(
        invocation: Option<Self>,
        state: &AppState,
        auth: &AuthUser,
        result: AppResult<Response>,
    ) -> AppResult<Response> {
        let Some(invocation) = invocation else {
            return result;
        };
        let eligible = match &result {
            Ok(response) => {
                response.status().is_client_error() || response.status().is_server_error()
            }
            Err(error) => diagnostics::eligible_error(error),
        };
        if !eligible {
            return result;
        }
        if result.as_ref().is_ok_and(|response| {
            response
                .extensions()
                .get::<RetainedServiceCapacity>()
                .is_some()
        }) {
            return result;
        }
        let id = invocation
            .target
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let Some(id) = id else { return result };
        let token = live_bearer(
            &auth.auth_method,
            auth.acting_client_id.as_deref(),
            invocation.token.as_deref(),
        );
        // chrono-probe callbacks must not diagnose their own failed checks.
        if auth.acting_client_id.as_deref() == Some("chrono-probe") {
            return result;
        }
        let actor_id = auth.user_id.to_string();
        let diagnostic = tokio::select! {
            biased;
            () = invocation.context.cancellation.cancelled() => None,
            result = diagnostics::diagnose(&state.db, &state.config, &actor_id, &id, token) => result,
        };
        let Some(diagnostic) = diagnostic else {
            return result;
        };
        observe(
            state,
            &AuditActor::from_auth_user(auth),
            &id,
            &diagnostic,
            &invocation.context.telemetry,
        )
        .await;
        let mut response = match result {
            Ok(response) => response,
            Err(error) => error.into_response(),
        };
        // Only fixed enum values. The downstream status/body/headers are kept.
        // Never import a diagnostic header supplied by the downstream service.
        response
            .headers_mut()
            .insert("x-nyxid-service-diagnostic", header_value(&diagnostic));
        Ok(response)
    }
}

pub(crate) fn live_bearer<'a>(
    method: &AuthMethod,
    acting_client: Option<&str>,
    token: Option<&'a str>,
) -> Option<&'a str> {
    if matches!(method, AuthMethod::AccessToken) && acting_client != Some("chrono-probe") {
        token
    } else {
        None
    }
}

fn header_value(diagnostic: &Diagnostic) -> axum::http::HeaderValue {
    // A fixed three-enum JSON projection, bounded to under 160 bytes.
    axum::http::HeaderValue::from_str(&serde_json::to_string(diagnostic).expect("enum projection"))
        .expect("ASCII enums")
}

pub(crate) async fn observe(
    state: &AppState,
    actor: &AuditActor,
    id: &str,
    diagnostic: &Diagnostic,
    telemetry: &TelemetryContext,
) {
    if diagnostic.attempted
        && matches!(
            diagnostic.outcome,
            Outcome::Completed | Outcome::Timeout | Outcome::Unavailable | Outcome::NoHealthPath
        )
    {
        audit_service::log_async(
            state.db.clone(),
            Some(actor.user_id.clone()),
            "service_invocation_diagnostic".into(),
            Some(serde_json::json!({"user_service_id":id,"diagnostic":diagnostic})),
            None,
            None,
            actor.api_key_id.clone(),
            None,
        );
    }
    // Metrics contain enum labels only: never paths, error text, tokens or IDs.
    tracing::info!(
        target: "nyxid::metrics",
        counter = "service_invocation_diagnostics_total",
        increment = 1u64,
        outcome = ?diagnostic.outcome,
        observation = ?diagnostic.observation,
        assessment = ?diagnostic.assessment,
        "Service invocation diagnostic outcome"
    );
    let _ = tokio::time::timeout(
        std::time::Duration::from_millis(25),
        emit_event(
            state.telemetry.as_deref(),
            &actor.user_id,
            actor.api_key_id.as_deref(),
            telemetry,
            TelemetryEvent::ServiceInvocationDiagnostic {
                diagnostic: diagnostic.clone(),
            },
        ),
    )
    .await;
}

#[cfg(test)]
#[path = "invocation_diagnostics/tests.rs"]
mod integration_tests;

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_original_ordinary_bearers_can_be_used() {
        for method in [
            AuthMethod::ApiKey,
            AuthMethod::Delegated,
            AuthMethod::Relay,
            AuthMethod::Session,
            AuthMethod::ServiceAccount,
        ] {
            assert!(live_bearer(&method, None, Some("secret")).is_none());
        }
        assert_eq!(
            live_bearer(&AuthMethod::AccessToken, None, Some("original")),
            Some("original")
        );
        assert!(
            live_bearer(
                &AuthMethod::AccessToken,
                Some("chrono-probe"),
                Some("secret")
            )
            .is_none()
        );
    }

    #[test]
    fn retained_capacity_suppresses_only_failed_responses() {
        let mut failed = Response::builder().status(503).body(Body::empty()).unwrap();
        suppress_for_retained_capacity(&mut failed);
        assert!(
            failed
                .extensions()
                .get::<RetainedServiceCapacity>()
                .is_some()
        );

        let mut succeeded = Response::new(Body::empty());
        suppress_for_retained_capacity(&mut succeeded);
        assert!(
            succeeded
                .extensions()
                .get::<RetainedServiceCapacity>()
                .is_none()
        );
    }
}

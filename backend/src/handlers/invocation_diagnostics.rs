//! Request-local target recording and response decoration. No bearer is stored
//! in session state, task locals, audit rows, metrics, or detached tasks.
use std::sync::{Arc, Mutex};

use axum::{
    Json,
    body::Body,
    extract::State,
    http::{
        HeaderMap, HeaderValue, Request, StatusCode,
        header::{CACHE_CONTROL, RETRY_AFTER},
    },
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};

use crate::{
    AppState,
    errors::AppResult,
    mw::auth::{AuthMethod, AuthUser},
    services::{
        audit_service::{self, AuditActor},
        invocation_diagnostic_service::{self as diagnostics, Diagnostic, Outcome, RedeemResult},
        proxy_service,
    },
    telemetry::{TelemetryContext, TelemetryEvent, emit_event},
};

pub(crate) const HANDLE_HEADER: &str = "x-nyxid-diagnostic-handle";
pub(crate) const EXPIRES_HEADER: &str = "x-nyxid-diagnostic-expires-in";

/// Internal retained LLM sends must preserve typed failures for their durable
/// delivery coordinator, which classifies uncertain provider outcomes.
#[derive(Clone, Copy)]
pub(crate) struct RetainedLlmExecution;

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

pub(crate) struct HttpInvocation {
    target: Target,
}

impl HttpInvocation {
    pub(crate) fn start(state: &AppState, request: &mut Request<Body>) -> Option<Self> {
        // Nested gateway/pool wrappers share the outer invocation's budget.
        if state.config.chrono_probe_url.is_none()
            || request.extensions().get::<Target>().is_some()
            || request.extensions().get::<RetainedLlmExecution>().is_some()
        {
            return None;
        }
        let target = Target::default();
        request.extensions_mut().insert(target.clone());
        Some(Self { target })
    }

    pub(crate) async fn finish(
        invocation: Option<Self>,
        state: &AppState,
        auth: &AuthUser,
        mut result: AppResult<Response>,
    ) -> AppResult<Response> {
        if let Ok(response) = &mut result {
            // These names are NyxID-owned. A downstream response must never be
            // able to manufacture evidence that a client will redeem as ours.
            response.headers_mut().remove(HANDLE_HEADER);
            response.headers_mut().remove(EXPIRES_HEADER);
        }
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
        let id = invocation
            .target
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let Some(id) = id else { return result };
        // chrono-probe callbacks must not create recursive diagnostics.
        if auth.acting_client_id.as_deref() == Some("chrono-probe") {
            return result;
        }
        let actor_id = auth.user_id.to_string();
        let issued = match diagnostics::issue(
            &state.db,
            &state.config,
            &actor_id,
            &auth.auth_method,
            auth.api_key_id.as_deref(),
            auth.acting_client_id
                .as_deref()
                .or(auth.oauth_client_id.as_deref()),
            &id,
        )
        .await
        {
            Ok(Some(issued)) => issued,
            Ok(None) => return result,
            Err(crate::errors::AppError::MonitoringNotConfigured) => return result,
            Err(error) => {
                tracing::warn!(
                    code = error.error_code(),
                    "Invocation diagnostic handle was not issued"
                );
                return result;
            }
        };
        let mut response = match result {
            Ok(response) => response,
            Err(error) => error.into_response(),
        };
        response.headers_mut().insert(
            HANDLE_HEADER,
            HeaderValue::from_str(&issued.handle).expect("opaque ASCII handle"),
        );
        response.headers_mut().insert(
            EXPIRES_HEADER,
            HeaderValue::from_str(&issued.expires_in.to_string()).expect("integer header"),
        );
        response
            .headers_mut()
            .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
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

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RedeemRequest {
    handle: String,
}

#[derive(Serialize)]
struct RedeemResponse {
    observation: diagnostics::Observation,
    assessment: diagnostics::Assessment,
    outcome: diagnostics::Outcome,
}

impl From<&Diagnostic> for RedeemResponse {
    fn from(diagnostic: &Diagnostic) -> Self {
        Self {
            observation: diagnostic.observation,
            assessment: diagnostic.assessment,
            outcome: diagnostic.outcome,
        }
    }
}

pub async fn redeem(
    State(state): State<AppState>,
    auth: AuthUser,
    telemetry: TelemetryContext,
    headers: HeaderMap,
    Json(request): Json<RedeemRequest>,
) -> AppResult<Response> {
    if auth.ensure_llm_proxy_access().is_err() {
        return Err(crate::errors::AppError::NotFound(
            "Invocation diagnostic not found".into(),
        ));
    }
    let supplied = proxy_service::caller_bearer_token_for_downstream(&headers, false);
    let caller_token = live_bearer(
        &auth.auth_method,
        auth.acting_client_id.as_deref(),
        supplied.as_deref(),
    );
    match diagnostics::redeem(
        &state.db,
        &state.config,
        &auth,
        &request.handle,
        caller_token,
    )
    .await?
    {
        RedeemResult::Completed {
            diagnostic,
            exact_user_service_id,
            fresh,
        } => {
            if fresh {
                observe(
                    &state,
                    &AuditActor::from_auth_user(&auth),
                    &exact_user_service_id,
                    &diagnostic,
                    &telemetry,
                )
                .await;
            }
            let mut response = Json(RedeemResponse::from(&diagnostic)).into_response();
            response
                .headers_mut()
                .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
            Ok(response)
        }
        RedeemResult::NotReady => {
            let mut response = (
                StatusCode::CONFLICT,
                Json(serde_json::json!({
                    "error":"diagnostic_not_ready",
                    "message":"The failed invocation is still releasing capacity; retry this diagnostic once."
                })),
            )
                .into_response();
            response
                .headers_mut()
                .insert(RETRY_AFTER, HeaderValue::from_static("1"));
            response
                .headers_mut()
                .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
            Ok(response)
        }
    }
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
}

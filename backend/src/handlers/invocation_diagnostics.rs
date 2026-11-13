//! Request-local target recording and response decoration. No bearer is stored
//! in session state, task locals, audit rows, metrics, or detached tasks.
use std::sync::{Arc, Mutex};

use axum::{
    Json,
    body::Body,
    extract::State,
    http::{HeaderMap, HeaderValue, Request, header::CACHE_CONTROL},
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};

use crate::{
    AppState,
    errors::{AppError, AppResult},
    mw::auth::{AuthMethod, AuthUser},
    services::{
        audit_service::{self, AuditActor},
        invocation_diagnostic_service::{self as diagnostics, Diagnostic, Outcome, RedeemResult},
        monitoring_service::CallerCredential,
    },
    telemetry::{TelemetryContext, TelemetryEvent, emit_event},
};

pub(crate) use diagnostics::{EXPIRES_HEADER, HANDLE_HEADER};
pub(crate) const PROBE_HEADER: &str = "x-nyxid-invocation-diagnostic-probe";

#[derive(Clone, Copy)]
pub(crate) struct DiagnosticProbe;

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
        // The marker is advisory recursion protection, not authorization. It is
        // always stripped so a reserved NyxID header never reaches downstream.
        let diagnostic_probe = request
            .headers_mut()
            .remove(PROBE_HEADER)
            .and_then(|value| value.to_str().ok().map(str::to_owned))
            .as_deref()
            == Some("1");
        if diagnostic_probe {
            request.extensions_mut().insert(DiagnosticProbe);
        }
        // Nested gateway/pool wrappers share the outer invocation's budget.
        if !state.config.invocation_diagnostics_enabled
            || state.config.chrono_probe_url.is_none()
            || diagnostic_probe
            || request
                .extensions()
                .get::<crate::mw::auth::SenderConstrainedToken>()
                .is_some()
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
            Ok(response) => response.status().is_server_error(),
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
        if !diagnostics::can_probe(
            &auth.auth_method,
            Some(auth.api_key_purpose),
            auth.acting_client_id.as_deref(),
        ) {
            return result;
        }
        let actor_id = auth.user_id.to_string();
        let issued = match diagnostics::issue(
            &state.db,
            &state.config,
            diagnostics::IssueIdentity::new(
                &actor_id,
                &auth.auth_method,
                auth.api_key_id.as_deref(),
                auth.api_key_credential_id.as_deref(),
                auth.acting_client_id
                    .as_deref()
                    .or(auth.oauth_client_id.as_deref()),
            )
            .with_api_key_purpose(auth.api_key_purpose),
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
        match result {
            Ok(mut response) => {
                issued.decorate(&mut response);
                Ok(response)
            }
            // Keep Err through tracing and outer handler layers. Decoration
            // happens only when Axum finally converts the error to a response.
            Err(source) => Err(AppError::InvocationDiagnostic {
                source: Box::new(source),
                handle: issued,
            }),
        }
    }
}

pub(crate) fn live_credential<'a>(
    method: &AuthMethod,
    acting_client: Option<&str>,
    headers: &'a HeaderMap,
) -> Option<CallerCredential<'a>> {
    let bearer = || {
        headers
            .get(axum::http::header::AUTHORIZATION)?
            .to_str()
            .ok()?
            .strip_prefix("Bearer ")
    };
    match method {
        AuthMethod::AccessToken if acting_client != Some("chrono-probe") => {
            bearer().map(CallerCredential::Bearer)
        }
        // AuthUser has already proved this value is an Agent Key. Normalize
        // either accepted inbound form to X-API-Key so chrono-probe does not
        // have to infer whether a Bearer value is a human token or Agent Key.
        AuthMethod::ApiKey => bearer()
            .or_else(|| {
                headers
                    .get("x-api-key")
                    .and_then(|value| value.to_str().ok())
            })
            .map(CallerCredential::ApiKey),
        _ => None,
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
    sender_constraint: Option<axum::Extension<crate::mw::auth::SenderConstrainedToken>>,
    Json(request): Json<RedeemRequest>,
) -> AppResult<Response> {
    let caller_credential = if sender_constraint.is_some() {
        None
    } else {
        live_credential(
            &auth.auth_method,
            auth.acting_client_id.as_deref(),
            &headers,
        )
    };
    match redeem_with_credential(
        &state,
        &auth,
        &request.handle,
        caller_credential,
        &telemetry,
    )
    .await?
    {
        RedeemResult::Completed { diagnostic, .. } => {
            let mut response = Json(RedeemResponse::from(&diagnostic)).into_response();
            response
                .headers_mut()
                .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
            Ok(response)
        }
        RedeemResult::NotReady => Err(AppError::DiagnosticNotReady),
    }
}

pub(crate) async fn redeem_with_credential(
    state: &AppState,
    auth: &AuthUser,
    handle: &str,
    caller_credential: Option<CallerCredential<'_>>,
    telemetry: &TelemetryContext,
) -> AppResult<RedeemResult> {
    if auth.ensure_llm_proxy_access().is_err() {
        return Err(crate::errors::AppError::NotFound(
            "Invocation diagnostic not found".into(),
        ));
    }
    auth.ensure_live_assistant_turn(&state.db, "monitoring.invocation_diagnostic")
        .await
        .map_err(conceal_turn_authority_error)?;
    let result =
        diagnostics::redeem(&state.db, &state.config, auth, handle, caller_credential).await?;
    if let RedeemResult::Completed {
        diagnostic,
        exact_user_service_id,
        fresh: true,
    } = &result
    {
        observe(
            state,
            &AuditActor::from_auth_user(auth),
            exact_user_service_id,
            diagnostic,
            telemetry,
        )
        .await;
    }
    Ok(result)
}

fn conceal_turn_authority_error(error: crate::errors::AppError) -> crate::errors::AppError {
    match error {
        crate::errors::AppError::AssistantTurnRequired
        | crate::errors::AppError::Forbidden(_)
        | crate::errors::AppError::Unauthorized(_)
        | crate::errors::AppError::ApiKeyScopeForbidden(_) => {
            crate::errors::AppError::NotFound("Invocation diagnostic not found".into())
        }
        error => error,
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
    fn only_supported_live_credentials_can_be_used() {
        let bearer_headers = HeaderMap::from_iter([(
            axum::http::header::AUTHORIZATION,
            "Bearer original".parse().unwrap(),
        )]);
        let key_headers =
            HeaderMap::from_iter([("x-api-key".parse().unwrap(), "agent".parse().unwrap())]);
        for method in [
            AuthMethod::Delegated,
            AuthMethod::Relay,
            AuthMethod::Session,
            AuthMethod::ServiceAccount,
        ] {
            assert!(live_credential(&method, None, &bearer_headers).is_none());
        }
        assert!(matches!(
            live_credential(&AuthMethod::AccessToken, None, &bearer_headers),
            Some(CallerCredential::Bearer("original"))
        ));
        assert!(matches!(
            live_credential(&AuthMethod::ApiKey, None, &key_headers),
            Some(CallerCredential::ApiKey("agent"))
        ));
        assert!(matches!(
            live_credential(&AuthMethod::ApiKey, None, &bearer_headers),
            Some(CallerCredential::ApiKey("original"))
        ));
        assert!(
            live_credential(
                &AuthMethod::AccessToken,
                Some("chrono-probe"),
                &bearer_headers,
            )
            .is_none()
        );
    }

    #[test]
    fn only_turn_authority_refusals_are_concealed_as_not_found() {
        for error in [
            crate::errors::AppError::AssistantTurnRequired,
            crate::errors::AppError::Forbidden("stale turn".into()),
            crate::errors::AppError::Unauthorized("stale key".into()),
            crate::errors::AppError::ApiKeyScopeForbidden("scope changed".into()),
        ] {
            assert!(matches!(
                conceal_turn_authority_error(error),
                crate::errors::AppError::NotFound(_)
            ));
        }
        assert!(matches!(
            conceal_turn_authority_error(crate::errors::AppError::Internal(
                "database unavailable".into()
            )),
            crate::errors::AppError::Internal(message) if message == "database unavailable"
        ));
    }
}

//! Installed only on the six authenticated HTTP proxy routes, never internal calls.
use std::future::Future;

use axum::{
    body::Body,
    extract::OriginalUri,
    http::{Method, Request},
    response::Response,
};

use crate::{
    errors::{AppError, AppResult},
    mw::{
        auth::{AuthMethod, AuthUser},
        refusal_throttle::{Admission, ProbeGuard, RefusalThrottle, Transition, request_key},
    },
    services::destination_routing,
};

#[path = "proxy_ingress_body.rs"]
mod body;

#[derive(Clone, Copy)]
pub(crate) struct HttpIngress;

/// Set before routing, authentication or body reads. Only callback routes open
/// the approval scope, including their nested service dispatches.
#[derive(Clone, Copy)]
pub(crate) struct RequestReceivedAt(pub(super) chrono::DateTime<chrono::Utc>);

pub(crate) async fn stamp(mut request: Request<Body>) -> Request<Body> {
    request
        .extensions_mut()
        .insert(RequestReceivedAt(chrono::Utc::now()));
    request
}

pub(crate) async fn callback(request: Request<Body>, next: axum::middleware::Next) -> Response {
    let received = request
        .extensions()
        .get::<RequestReceivedAt>()
        .map_or_else(chrono::Utc::now, |at| at.0);
    crate::services::approval_service::CALLBACK_INGRESS
        .scope(received, next.run(request))
        .await
}

pub(super) async fn execute<F: Future<Output = AppResult<Response>>>(
    limiter: &RefusalThrottle,
    auth: &AuthUser,
    service: &str,
    mut request: Request<Body>,
    forward: impl FnOnce(Request<Body>) -> F,
) -> AppResult<Response> {
    if request.extensions_mut().remove::<HttpIngress>().is_none()
        || request.method() == Method::CONNECT
        || super::is_ws_upgrade_request(&request)
    {
        return forward(request).await;
    }
    let Some(body_digest) = body::fingerprint(&mut request).await else {
        return forward(request).await;
    };
    let mut caller_buffer = uuid::Uuid::encode_buffer();
    let (kind, caller) = caller_identity(
        &auth.auth_method,
        auth.api_key_id.as_deref(),
        auth.user_id,
        &mut caller_buffer,
    );
    let uri = request
        .extensions()
        .get::<OriginalUri>()
        .map_or_else(|| request.uri().clone(), |original| original.0.clone());
    let method = request.method().clone();
    let key = request_key(
        kind,
        caller,
        method.as_str(),
        service,
        uri.path(),
        uri.query(),
        &body_digest,
    );
    let admission = limiter.admit(&key);
    if admission == Admission::Throttled {
        return Ok(AppError::quiet_rate_limit_response(
            crate::mw::refusal_throttle::RETRY_SECONDS,
        ));
    }
    let guard = ProbeGuard::new(limiter, key, admission == Admission::Probe);
    let (result, dispatched) = destination_routing::observe_http_dispatch(forward(request)).await;
    let code = if dispatched {
        None
    } else {
        result
            .as_ref()
            .err()
            .filter(|error| error.counts_for_refusal_throttle())
            .map(AppError::error_code)
    };
    match guard.finish(code) {
        Some(Transition::Armed { code, count }) => tracing::warn!(
            request_digest = %hex::encode(key),
            caller_id = %caller, caller_kind = kind, service, path = uri.path(),
            method = method.as_str(), error_code = code, count,
            "Proxy refusal throttle armed"
        ),
        Some(Transition::Cleared { code, count }) => tracing::warn!(
            request_digest = %hex::encode(key),
            caller_id = %caller, caller_kind = kind, service, path = uri.path(),
            method = method.as_str(), error_code = code, count,
            "Proxy refusal throttle cleared"
        ),
        None => {}
    }
    result
}

fn caller_identity<'a>(
    method: &AuthMethod,
    api_key_id: Option<&'a str>,
    user_id: uuid::Uuid,
    buffer: &'a mut [u8],
) -> (&'static str, &'a str) {
    match (method, api_key_id) {
        (AuthMethod::ApiKey, Some(id)) => ("api_key", id),
        _ => ("user", user_id.hyphenated().encode_lower(buffer)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refusal_throttle_identity_uses_user_for_jwts_even_with_relay_parent_key() {
        let user = uuid::Uuid::nil();
        let mut buffer = uuid::Uuid::encode_buffer();
        for method in [
            AuthMethod::Session,
            AuthMethod::AccessToken,
            AuthMethod::Relay,
            AuthMethod::ServiceAccount,
            AuthMethod::Delegated,
        ] {
            assert_eq!(
                caller_identity(&method, Some("synthetic-parent-key"), user, &mut buffer),
                ("user", "00000000-0000-0000-0000-000000000000")
            );
        }
        assert_eq!(
            caller_identity(
                &AuthMethod::ApiKey,
                Some("synthetic-key"),
                user,
                &mut buffer
            ),
            ("api_key", "synthetic-key")
        );
    }
}

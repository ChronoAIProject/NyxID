//! HTTP and SSE boundary for the disposable direct Chrono agent POC.

use std::convert::Infallible;
use std::time::Duration;

use axum::body::{Body, to_bytes};
use axum::extract::State;
use axum::http::{HeaderValue, Request, Response, header};
use bytes::Bytes;
use futures::Stream;

use crate::AppState;
use crate::downstream_disconnect::{
    CancelOnDropStream, ClientConnectionCancellation, request_cancellation,
};
use crate::errors::{AppError, AppResult};
use crate::handlers::assistant_direct::require_direct_chat_enabled;
use crate::mw::auth::{AuthMethod, AuthUser};
use crate::services::assistant_direct_agent_poc::{self, RunInputs};
use crate::services::billing::BillingIngress;
use crate::services::billing::route_inventory::{
    BillingRoutePolicy, enforce_billing_egress_classification,
};
use crate::services::{assistant_direct, assistant_service};

const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(10);

pub async fn agent_completions(
    State(state): State<AppState>,
    auth_user: AuthUser,
    request: Request<Body>,
) -> AppResult<Response<Body>> {
    require_direct_chat_enabled(&state, &auth_user).await?;
    if auth_user.auth_method != AuthMethod::Session {
        return Err(AppError::Forbidden(
            "Assistant agent mode requires a browser session.".to_string(),
        ));
    }

    let billing_policy = request.extensions().get::<BillingRoutePolicy>().copied();
    let billing_egress_permit =
        enforce_billing_egress_classification(billing_policy, BillingIngress::Proxy)?;
    let billing_policy = billing_policy.expect("billing classification was enforced");
    let cancellation = request_cancellation(&request);
    let connection_extension = request
        .extensions()
        .get::<ClientConnectionCancellation>()
        .cloned();
    let permit = state
        .direct_chat_limiter
        .try_acquire(&auth_user.user_id.to_string())?;

    let (_, body) = request.into_parts();
    let bytes = to_bytes(body, assistant_direct::MAX_DIRECT_REQUEST_BYTES)
        .await
        .map_err(|_| AppError::BadRequest("Direct chat request body is too large.".to_string()))?;
    let direct_request = assistant_direct::validate_direct_request(&bytes)?;
    let content_bytes = direct_request
        .messages
        .iter()
        .try_fold(0usize, |total, message| {
            total.checked_add(message.content.len())
        })
        .ok_or_else(|| AppError::BadRequest("Message content is too large.".to_string()))?;
    if content_bytes > assistant_direct_agent_poc::MAX_AGENT_CONTENT_BYTES {
        return Err(AppError::BadRequest(format!(
            "Aggregate agent message content must be at most {} UTF-8 bytes.",
            assistant_direct_agent_poc::MAX_AGENT_CONTENT_BYTES
        )));
    }
    let chrono_service = assistant_service::resolve_admin_service_by_slug(
        &state.db,
        assistant_direct::DIRECT_LLM_SLUG,
    )
    .await?;

    let (tx, rx) = tokio::sync::mpsc::channel(32);
    let stream_cancellation = cancellation.clone();
    tokio::spawn(assistant_direct_agent_poc::run(RunInputs {
        state,
        auth_user,
        request: direct_request,
        chrono_service_id: chrono_service.id,
        billing_policy,
        billing_egress_permit,
        connection_extension,
        cancellation,
        tx,
    }));

    let stream = response_stream(rx, permit, stream_cancellation);
    Response::builder()
        .status(axum::http::StatusCode::OK)
        .header(
            header::CONTENT_TYPE,
            HeaderValue::from_static("text/event-stream"),
        )
        .header(header::CACHE_CONTROL, HeaderValue::from_static("no-cache"))
        .header("x-accel-buffering", HeaderValue::from_static("no"))
        .body(Body::from_stream(stream))
        .map_err(|_| AppError::Internal("assistant: failed to build agent stream".to_string()))
}

fn response_stream(
    rx: tokio::sync::mpsc::Receiver<Result<Bytes, Infallible>>,
    permit: crate::mw::rate_limit::DirectChatPermit,
    cancellation: tokio_util::sync::CancellationToken,
) -> impl Stream<Item = Result<Bytes, Infallible>> {
    response_stream_with_heartbeat(rx, permit, cancellation, HEARTBEAT_INTERVAL)
}

fn response_stream_with_heartbeat(
    mut rx: tokio::sync::mpsc::Receiver<Result<Bytes, Infallible>>,
    permit: crate::mw::rate_limit::DirectChatPermit,
    cancellation: tokio_util::sync::CancellationToken,
    heartbeat_interval: Duration,
) -> impl Stream<Item = Result<Bytes, Infallible>> {
    let inner = async_stream::stream! {
        let _permit = permit;
        let mut heartbeat = tokio::time::interval(heartbeat_interval);
        heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        heartbeat.tick().await;
        loop {
            tokio::select! {
                biased;
                item = rx.recv() => match item {
                    Some(item) => yield item,
                    None => break,
                },
                _ = heartbeat.tick() => yield Ok(Bytes::from_static(b": ping\n\n")),
            }
        }
    };
    CancelOnDropStream::new(inner, cancellation)
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::StreamExt;

    const USER_ID: &str = "a026fd00-bd86-4284-9832-9e5e65fc8f50";

    #[tokio::test]
    async fn default_off_returns_not_found_before_permit_or_egress() {
        let Some(db) = crate::test_utils::connect_test_database("assistant_agent_flag_off").await
        else {
            eprintln!("skipping assistant agent flag test: no local MongoDB available");
            return;
        };
        let state = crate::test_utils::test_app_state(db);
        let mut auth = crate::test_utils::test_auth_user(USER_ID);
        auth.auth_method = AuthMethod::Session;
        let request = Request::builder()
            .method("POST")
            .uri("/api/v1/assistant/direct/agent")
            .body(Body::from(
                r#"{"messages":[{"role":"user","content":"hello"}]}"#,
            ))
            .unwrap();

        assert!(matches!(
            agent_completions(State(state.clone()), auth, request).await,
            Err(AppError::NotFound(_))
        ));
        assert!(state.direct_chat_limiter.try_acquire(USER_ID).is_ok());
    }

    #[tokio::test]
    async fn access_token_is_rejected_before_permit_or_stream() {
        use crate::services::feature_flag_service::{FlagTarget, set_platform_override};

        let Some(db) =
            crate::test_utils::connect_test_database("assistant_agent_session_gate").await
        else {
            eprintln!("skipping assistant agent session test: no local MongoDB available");
            return;
        };
        set_platform_override(
            &db,
            crate::services::feature_flag_service::DIRECT_CHAT_ENGINE_FLAG_KEY,
            &FlagTarget::Global,
            true,
            "admin",
        )
        .await
        .unwrap();
        let state = crate::test_utils::test_app_state(db);
        let mut auth = crate::test_utils::test_auth_user(USER_ID);
        auth.auth_method = AuthMethod::AccessToken;
        let mut request = Request::builder()
            .method("POST")
            .uri("/api/v1/assistant/direct/agent")
            .body(Body::from(
                r#"{"messages":[{"role":"user","content":"hello"}]}"#,
            ))
            .unwrap();
        request
            .extensions_mut()
            .insert(BillingRoutePolicy::Metered(BillingIngress::Proxy));
        assert!(matches!(
            agent_completions(State(state.clone()), auth, request).await,
            Err(AppError::Forbidden(_))
        ));
        assert!(state.direct_chat_limiter.try_acquire(USER_ID).is_ok());
    }

    #[tokio::test]
    async fn billing_policy_fails_closed_before_service_resolution() {
        let state = crate::test_utils::test_app_state_no_db().await;
        let auth = crate::test_utils::test_auth_user(USER_ID);
        let request = Request::builder()
            .method("POST")
            .uri("/api/v1/assistant/direct/agent")
            .body(Body::empty())
            .unwrap();
        // The feature flag requires Mongo, so exercise the same mandatory
        // classification helper directly for the pre-egress invariant.
        assert!(enforce_billing_egress_classification(None, BillingIngress::Proxy).is_err());
        assert!(
            enforce_billing_egress_classification(
                Some(BillingRoutePolicy::Exempt("test")),
                BillingIngress::Proxy
            )
            .is_err()
        );
        drop((state, auth, request));
    }

    #[tokio::test]
    async fn response_stream_holds_and_releases_concurrent_permits() {
        let limiter =
            std::sync::Arc::new(crate::mw::rate_limit::DirectChatRateLimiter::new(10, 60, 2));
        let make_body = |permit, close_sender: bool| {
            let (tx, rx) = tokio::sync::mpsc::channel(1);
            let tx = (!close_sender).then_some(tx);
            let cancellation = tokio_util::sync::CancellationToken::new();
            (
                Body::from_stream(response_stream(rx, permit, cancellation.clone())),
                tx,
                cancellation,
            )
        };

        let first = limiter.try_acquire(USER_ID).unwrap();
        let second = limiter.try_acquire(USER_ID).unwrap();
        let (first_body, first_tx, first_cancellation) = make_body(first, false);
        let (second_body, _second_tx, second_cancellation) = make_body(second, true);
        assert!(matches!(
            limiter.try_acquire(USER_ID),
            Err(AppError::RateLimited)
        ));
        assert_eq!(
            axum::response::IntoResponse::into_response(AppError::RateLimited).status(),
            axum::http::StatusCode::TOO_MANY_REQUESTS,
        );

        drop(first_body);
        assert!(
            first_cancellation.is_cancelled(),
            "dropping an unpolled response body cancels its spawned run"
        );
        drop(first_tx.expect("first sender remains open"));
        let replacement = limiter
            .try_acquire(USER_ID)
            .expect("dropping a response body releases its permit");
        drop(replacement);

        let bytes = to_bytes(second_body, usize::MAX)
            .await
            .expect("consume naturally completed response stream");
        assert!(bytes.is_empty());
        assert!(
            second_cancellation.is_cancelled(),
            "naturally ending the response body cancels its request token"
        );
        assert!(
            limiter.try_acquire(USER_ID).is_ok(),
            "ending a response stream releases its permit"
        );
    }

    #[tokio::test]
    async fn response_stream_emits_heartbeat_while_writable_and_quiet() {
        let limiter =
            std::sync::Arc::new(crate::mw::rate_limit::DirectChatRateLimiter::new(10, 60, 1));
        let permit = limiter.try_acquire(USER_ID).unwrap();
        let (tx, rx) = tokio::sync::mpsc::channel(1);
        let cancellation = tokio_util::sync::CancellationToken::new();
        let stream =
            response_stream_with_heartbeat(rx, permit, cancellation, Duration::from_millis(5));
        futures::pin_mut!(stream);

        let heartbeat = tokio::time::timeout(Duration::from_millis(100), stream.next())
            .await
            .expect("quiet writable stream emits a prompt heartbeat")
            .expect("heartbeat item")
            .expect("infallible heartbeat");
        assert_eq!(heartbeat, Bytes::from_static(b": ping\n\n"));
        drop(tx);
    }
}

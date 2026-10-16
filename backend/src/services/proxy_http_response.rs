//! Shared direct HTTP response streaming and exact usage settlement.
//! Authentication, target selection and dispatch admission remain with callers.

use crate::downstream_disconnect::{CancelOnDropStream, until_client_disconnect};
use crate::models::service_billing::{BillingMetric, PlatformUsage, ResaleUsage};
use crate::services::billing::MeteredProxyContext;
use crate::services::{llm_usage_service, proxy_service};
use crate::{
    AppState,
    errors::{AppError, AppResult},
};
use axum::{
    body::Body,
    http::{Method, StatusCode},
    response::Response,
};
use futures::StreamExt;
use tokio_stream::wrappers::ReceiverStream;
use tokio_util::sync::CancellationToken;

pub(crate) struct HttpResponseContext {
    pub service_id: String,
    pub path: String,
    pub nyxagent_admission: bool,
    pub model_request: Option<super::assistant_model_observer::ModelRequest>,
    pub response_builder: http::response::Builder,
    pub status: StatusCode,
    pub is_sse: bool,
    pub should_stream: bool,
    pub usage_context: Option<llm_usage_service::UsageAuditContext>,
    pub metered: MeteredProxyContext,
    pub request_body_len: i64,
    pub resale_metric: Option<BillingMetric>,
    pub downstream_cancellation: CancellationToken,
    pub exchange_diagnostics: ProxyExchangeDiagnostics,
}

/// Construct the response relay behind a heap boundary so dispatch callers do
/// not embed its streaming and settlement state in their own poll frames.
pub(crate) fn relay_response<'a>(
    state: &'a AppState,
    context: HttpResponseContext,
    downstream_response: reqwest::Response,
) -> futures::future::BoxFuture<'a, AppResult<Response>> {
    Box::pin(relay_response_inner(state, context, downstream_response))
}

async fn relay_response_inner(
    state: &AppState,
    context: HttpResponseContext,
    downstream_response: reqwest::Response,
) -> AppResult<Response> {
    let HttpResponseContext {
        service_id,
        path,
        nyxagent_admission,
        model_request,
        response_builder,
        status,
        is_sse,
        should_stream,
        usage_context,
        metered,
        request_body_len,
        resale_metric,
        downstream_cancellation,
        exchange_diagnostics,
    } = context;
    let service_id = service_id.as_str();
    let path = path.as_str();
    let bounded_admission_body = nyxagent_admission && matches!(status.as_u16(), 409 | 503);
    if status.is_success()
        && let Some(request) = &model_request
        && request.accepted().await.is_err()
    {
        tracing::warn!("Assistant model success attribution deferred");
    }
    let response = if should_stream {
        // Stream responses without buffering, but use a forwarding task when
        // we need to observe SSE usage so client disconnects are visible.
        if let Some(stream_usage_context) = if is_sse { usage_context.clone() } else { None } {
            let service_id_owned = service_id.to_string();
            let idle_timeout =
                std::time::Duration::from_secs(state.config.proxy_stream_idle_timeout_secs);
            let idle_timeout_secs = state.config.proxy_stream_idle_timeout_secs;
            let mut upstream_stream = downstream_response.bytes_stream();
            let (tx, rx) = tokio::sync::mpsc::channel::<Result<bytes::Bytes, std::io::Error>>(32);
            let stream_billing = state.billing.clone();
            let stream_metered = metered.clone();
            let request_len = request_body_len;
            let task_cancellation = downstream_cancellation.clone();
            let model_protocol = crate::services::assistant_model_observer::protocol(path);
            let model_request = model_request.clone().filter(|_| status.is_success());

            tokio::spawn(async move {
                let mut exchange_diagnostics = ProxyStreamDiagnostics::new(exchange_diagnostics);
                let mut usage_accumulator =
                    llm_usage_service::ReportedLlmUsageAccumulator::default();
                let mut response_len: i64 = 0;
                // Billing keeps the full SSE event; the bounded model observer is diagnostic only.
                let mut sse_buffer = String::new();
                let mut model_observer = model_request.as_ref().map(|_| {
                    crate::services::assistant_model_observer::StreamObserver::new(model_protocol)
                });
                let mut provider_fault_recorded = false;
                let mut framing_fault_recorded = false;

                loop {
                    let next = until_client_disconnect(
                        &task_cancellation,
                        tokio::time::timeout(idle_timeout, upstream_stream.next()),
                    )
                    .await;
                    match next {
                        Err(_) => {
                            if let Some(model_request) = &model_request {
                                let _ = model_request.fault("consumer_cancelled", false).await;
                                let _ = model_request.complete().await;
                            }
                            exchange_diagnostics.finish("client_disconnect");
                            drop(upstream_stream);
                            let usage = usage_accumulator.finalize();
                            if let Some(usage) = usage.clone() {
                                llm_usage_service::log_reported_usage_async(
                                    stream_usage_context.clone(),
                                    usage,
                                );
                            }
                            let resale = resale_metric.and_then(|metric| {
                                resale_usage_from_optional_reported(
                                    metric,
                                    usage.as_ref(),
                                    request_len + response_len,
                                )
                            });
                            settle_meter_async(
                                stream_billing,
                                stream_metered,
                                llm_platform_usage(usage.as_ref(), request_len + response_len),
                                resale,
                                stream_usage_context.model.clone(),
                            )
                            .await;
                            return;
                        }
                        Ok(Ok(Some(Ok(bytes)))) => {
                            response_len += bytes.len() as i64;
                            sse_buffer.push_str(&String::from_utf8_lossy(&bytes));
                            while let Some(event) =
                                super::sse_parser::parse_next_event(&mut sse_buffer)
                            {
                                if let Some((mut usage, mode)) =
                                    llm_usage_service::extract_reported_usage_from_sse_event(
                                        event.event_type.as_deref(),
                                        &event.data,
                                    )
                                {
                                    if !status.is_success() {
                                        usage.images = 0;
                                    }
                                    usage_accumulator.observe(usage, mode);
                                }
                            }

                            if let (Some(observer), Some(request)) =
                                (&mut model_observer, &model_request)
                            {
                                for &byte in &bytes {
                                    let _ = observer.push_byte(byte);
                                }
                                if observer.framing_invalid() && !framing_fault_recorded {
                                    let _ = request.fault("unknown", false).await;
                                    framing_fault_recorded = true;
                                }
                                if observer.provider_failed() && !provider_fault_recorded {
                                    let _ = request.fault("provider_error", true).await;
                                    provider_fault_recorded = true;
                                }
                            }
                            if tx.send(Ok(bytes)).await.is_err() {
                                if let Some(model_request) = &model_request {
                                    let _ = model_request.fault("consumer_cancelled", false).await;
                                    let _ = model_request.complete().await;
                                }
                                exchange_diagnostics.finish("client_disconnect");
                                drop(upstream_stream);
                                let usage = usage_accumulator.finalize();
                                if let Some(usage) = usage.clone() {
                                    llm_usage_service::log_reported_usage_async(
                                        stream_usage_context.clone(),
                                        usage,
                                    );
                                }
                                let resale = resale_metric.and_then(|metric| {
                                    resale_usage_from_optional_reported(
                                        metric,
                                        usage.as_ref(),
                                        request_len + response_len,
                                    )
                                });
                                settle_meter_async(
                                    stream_billing,
                                    stream_metered,
                                    llm_platform_usage(usage.as_ref(), request_len + response_len),
                                    resale,
                                    stream_usage_context.model.clone(),
                                )
                                .await;
                                return;
                            }
                        }
                        Ok(Ok(Some(Err(e)))) => {
                            if let (Some(model_request), Some(observer)) =
                                (&model_request, &model_observer)
                            {
                                let (kind, attributable) = observer.transport_fault();
                                let _ = model_request.fault(kind, attributable).await;
                                let _ = model_request.complete().await;
                            }
                            exchange_diagnostics.finish("body_error");
                            tracing::error!(
                                service_id = %service_id_owned,
                                error = %e,
                                error_debug = ?e,
                                "Proxy stream error from upstream — connection dropped"
                            );
                            let usage = usage_accumulator.finalize();
                            if let Some(usage) = usage.clone() {
                                llm_usage_service::log_reported_usage_async(
                                    stream_usage_context.clone(),
                                    usage,
                                );
                            }
                            let resale = resale_metric.and_then(|metric| {
                                resale_usage_from_optional_reported(
                                    metric,
                                    usage.as_ref(),
                                    request_len + response_len,
                                )
                            });
                            settle_meter_async(
                                stream_billing,
                                stream_metered,
                                llm_platform_usage(usage.as_ref(), request_len + response_len),
                                resale,
                                stream_usage_context.model.clone(),
                            )
                            .await;
                            let _ = tx
                                .send(Err(std::io::Error::other(format!(
                                    "upstream stream error: {e}"
                                ))))
                                .await;
                            return;
                        }
                        Ok(Ok(None)) => {
                            if let Some(kind) =
                                model_observer.as_mut().and_then(|observer| observer.eof())
                                && !(kind == "provider_error" && provider_fault_recorded)
                                && !(kind == "unknown" && framing_fault_recorded)
                                && let Some(model_request) = &model_request
                            {
                                let _ = model_request.fault(kind, kind != "unknown").await;
                            }
                            if let Some(model_request) = &model_request {
                                let _ = model_request.complete().await;
                            }
                            exchange_diagnostics.finish("completed");
                            let usage = usage_accumulator.finalize();
                            if let Some(usage) = usage.clone() {
                                llm_usage_service::log_reported_usage_async(
                                    stream_usage_context.clone(),
                                    usage,
                                );
                            }
                            let resale = resale_metric.and_then(|metric| {
                                resale_usage_from_optional_reported(
                                    metric,
                                    usage.as_ref(),
                                    request_len + response_len,
                                )
                            });
                            settle_meter_async(
                                stream_billing,
                                stream_metered,
                                llm_platform_usage(usage.as_ref(), request_len + response_len),
                                resale,
                                stream_usage_context.model.clone(),
                            )
                            .await;
                            return;
                        }
                        Ok(Err(_)) => {
                            if let (Some(model_request), Some(observer)) =
                                (&model_request, &model_observer)
                            {
                                let (kind, attributable) = observer.transport_fault();
                                let _ = model_request.fault(kind, attributable).await;
                                let _ = model_request.complete().await;
                            }
                            exchange_diagnostics.finish("upstream_error");
                            tracing::warn!(
                                service_id = %service_id_owned,
                                idle_timeout_secs,
                                "Proxy stream idle timeout reached"
                            );
                            let usage = usage_accumulator.finalize();
                            if let Some(usage) = usage.clone() {
                                llm_usage_service::log_reported_usage_async(
                                    stream_usage_context.clone(),
                                    usage,
                                );
                            }
                            let resale = resale_metric.and_then(|metric| {
                                resale_usage_from_optional_reported(
                                    metric,
                                    usage.as_ref(),
                                    request_len + response_len,
                                )
                            });
                            settle_meter_async(
                                stream_billing,
                                stream_metered,
                                llm_platform_usage(usage.as_ref(), request_len + response_len),
                                resale,
                                stream_usage_context.model.clone(),
                            )
                            .await;
                            return;
                        }
                    }
                }
            });

            let body = Body::from_stream(CancelOnDropStream::new(
                ReceiverStream::new(rx),
                downstream_cancellation.clone(),
            ));
            response_builder
                .body(body)
                .map_err(|e| AppError::Internal(format!("Failed to build response: {e}")))?
        } else {
            let service_id_owned = service_id.to_string();
            let idle_timeout =
                std::time::Duration::from_secs(state.config.proxy_stream_idle_timeout_secs);
            let idle_timeout_secs = state.config.proxy_stream_idle_timeout_secs;
            let is_json_body = downstream_response
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .is_some_and(|ct| ct.to_ascii_lowercase().starts_with("application/json"));
            let mut upstream_stream = downstream_response.bytes_stream();
            let stream_billing = state.billing.clone();
            let stream_metered = metered.clone();
            let request_len = request_body_len;
            // Chunked JSON bodies (no Content-Length) stream through this
            // branch; capture a bounded copy so LLM services settle with the
            // provider-reported token count instead of the byte estimate.
            let stream_usage_context = if is_json_body {
                usage_context.clone()
            } else {
                None
            };
            let stream_resale_metric = resale_metric;
            let mut exchange_diagnostics = ProxyStreamDiagnostics::new(exchange_diagnostics);
            let stream = async_stream::stream! {
                let mut response_len: i64 = 0;
                let mut captured: Option<Vec<u8>> =
                    stream_usage_context.as_ref().map(|_| Vec::new());
                loop {
                    match tokio::time::timeout(idle_timeout, upstream_stream.next()).await {
                        Ok(Some(Ok(bytes))) => {
                            response_len += bytes.len() as i64;
                            if let Some(buf) = captured.as_mut() {
                                if buf.len() + bytes.len() <= USAGE_CAPTURE_MAX_BYTES {
                                    buf.extend_from_slice(&bytes);
                                } else {
                                    captured = None;
                                }
                            }
                            yield Ok::<_, std::io::Error>(bytes);
                        }
                        Ok(Some(Err(e))) => {
                            exchange_diagnostics.finish("body_error");
                            tracing::error!(
                                service_id = %service_id_owned,
                                error = %e,
                                error_debug = ?e,
                                "Proxy stream error from upstream — connection dropped"
                            );
                            yield Err(std::io::Error::other(format!(
                                "upstream stream error: {e}"
                            )));
                            break;
                        }
                        Ok(None) => {
                            exchange_diagnostics.finish("completed");
                            break;
                        }
                        Err(_) => {
                            exchange_diagnostics.finish("upstream_error");
                            tracing::warn!(
                                service_id = %service_id_owned,
                                idle_timeout_secs,
                                "Proxy stream idle timeout reached"
                            );
                            break;
                        }
                    }
                }
                let mut reported_usage = None;
                let mut model = None;
                if let Some(ctx) = stream_usage_context
                    && let Some(buf) = captured
                    && let Ok(json) = serde_json::from_slice::<serde_json::Value>(&buf)
                    && let Some(usage) = llm_usage_service::extract_reported_usage_for_path(&json, &ctx.path, status.is_success())
                {
                    model = ctx.model.clone();
                    llm_usage_service::log_reported_usage_async(ctx, usage.clone());
                    reported_usage = Some(usage);
                }
                let resale = stream_resale_metric.and_then(|metric| {
                    resale_usage_from_optional_reported(
                        metric,
                        reported_usage.as_ref(),
                        request_len + response_len,
                    )
                });
                settle_meter_async(
                    stream_billing,
                    stream_metered,
                    llm_platform_usage(reported_usage.as_ref(), request_len + response_len),
                    resale,
                    model,
                )
                .await;
            };
            let body = Body::from_stream(CancelOnDropStream::new(
                stream,
                downstream_cancellation.clone(),
            ));
            response_builder
                .body(body)
                .map_err(|e| AppError::Internal(format!("Failed to build response: {e}")))?
        }
    } else {
        // Buffer small / error responses so we can log diagnostics.
        let upstream_request_id = downstream_response
            .headers()
            .get("x-request-id")
            .and_then(|value| value.to_str().ok())
            .unwrap_or("unknown")
            .to_string();
        let mut admission_response_len = 0_i64;
        let response_body = match until_client_disconnect(&downstream_cancellation, async {
            if bounded_admission_body {
                let mut stream = downstream_response.bytes_stream();
                let mut captured = bytes::BytesMut::new();
                while let Some(chunk) = stream.next().await {
                    let chunk = chunk.map_err(|_| {
                        AppError::Internal("Invalid assistant admission response".into())
                    })?;
                    admission_response_len =
                        admission_response_len.saturating_add(chunk.len() as i64);
                    if captured.len().saturating_add(chunk.len())
                        > crate::services::assistant_engine_policy::ADMISSION_ERROR_BODY_MAX_BYTES
                    {
                        return Err(AppError::Internal(
                            "Assistant admission response too large".into(),
                        ));
                    }
                    captured.extend_from_slice(&chunk);
                }
                Ok(captured.freeze())
            } else {
                downstream_response.bytes().await.map_err(|e| {
                    AppError::Internal(format!("Failed to read downstream response: {e}"))
                })
            }
        })
        .await
        {
            Err(_) => {
                exchange_diagnostics.emit("client_disconnect");
                if bounded_admission_body {
                    settle_meter_async(
                        state.billing.clone(),
                        metered,
                        llm_platform_usage(None, request_body_len + admission_response_len),
                        resale_metric.and_then(|metric| {
                            resale_usage_from_optional_reported(
                                metric,
                                None,
                                request_body_len + admission_response_len,
                            )
                        }),
                        None,
                    )
                    .await;
                }
                return Err(proxy_client_disconnected(service_id));
            }
            Ok(Err(e)) => {
                exchange_diagnostics.emit("body_error");
                if bounded_admission_body {
                    settle_meter_async(
                        state.billing.clone(),
                        metered,
                        llm_platform_usage(None, request_body_len + admission_response_len),
                        resale_metric.and_then(|metric| {
                            resale_usage_from_optional_reported(
                                metric,
                                None,
                                request_body_len + admission_response_len,
                            )
                        }),
                        None,
                    )
                    .await;
                }
                return Err(AppError::Internal(format!(
                    "Failed to read downstream response: {e}"
                )));
            }
            Ok(Ok(body)) => body,
        };
        exchange_diagnostics.emit("completed");

        if !status.is_success() {
            log_upstream_error(
                service_id,
                status,
                response_body.len(),
                &upstream_request_id,
            );
        }

        let mut reported_usage = None;
        let mut model = None;
        if let Some(nonstream_usage_context) = usage_context
            && let Ok(json) = serde_json::from_slice::<serde_json::Value>(&response_body)
            && let Some(usage) = llm_usage_service::extract_reported_usage_for_path(
                &json,
                &nonstream_usage_context.path,
                status.is_success(),
            )
        {
            model = nonstream_usage_context.model.clone();
            llm_usage_service::log_reported_usage_async(nonstream_usage_context, usage.clone());
            reported_usage = Some(usage);
        }

        let request_len = request_body_len;
        let response_len = response_body.len() as i64;
        let resale = resale_metric.and_then(|metric| {
            resale_usage_from_optional_reported(
                metric,
                reported_usage.as_ref(),
                request_len + response_len,
            )
        });
        let rejected_admission =
            proven_nyxagent_refusal(nyxagent_admission, status.as_u16(), &response_body);
        if rejected_admission {
            // Returning the refusal admits a retry. Its funding must be free first.
            if let Err(error) = state
                .billing
                .settle(
                    &metered,
                    PlatformUsage::default(),
                    resale.map(|mut usage| {
                        usage.quantity = 0;
                        usage
                    }),
                    model,
                )
                .await
            {
                tracing::error!(
                    code = error.error_code(),
                    "Assistant admission funding release failed"
                );
                return Err(AppError::Internal(
                    "Assistant admission funding release failed".into(),
                ));
            }
        } else {
            settle_meter_async(
                state.billing.clone(),
                metered,
                llm_platform_usage(reported_usage.as_ref(), request_len + response_len),
                resale,
                model,
            )
            .await;
        }

        response_builder
            .body(Body::from(response_body))
            .map_err(|e| AppError::Internal(format!("Failed to build response: {e}")))?
    };

    if !(should_stream && is_sse)
        && let Some(request) = &model_request
    {
        let _ = request.complete().await;
    }
    Ok(response)
}

pub(crate) fn proven_nyxagent_refusal(engine: bool, status: u16, body: &[u8]) -> bool {
    engine
        && serde_json::from_slice::<serde_json::Value>(body)
            .ok()
            .and_then(|value| value["error"]["code"].as_str().map(str::to_owned))
            .is_some_and(|code| {
                crate::services::assistant_nyxagent::admission_refusal(status, &code)
            })
}

pub(crate) async fn observe_model_response(
    request: Option<crate::services::assistant_model_observer::ModelRequest>,
    path: &str,
    response: Response,
) -> AppResult<Response> {
    let Some(request) = request else {
        return Ok(response);
    };
    if !response.status().is_success() {
        let _ = request.complete().await;
        return Ok(response);
    }
    if request.accepted().await.is_err() {
        tracing::warn!("Assistant model success attribution deferred");
    }
    if !response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .is_some_and(crate::mw::security_headers::is_sse_media_type)
    {
        let _ = request.complete().await;
        return Ok(response);
    }
    let protocol = crate::services::assistant_model_observer::protocol(path);
    let (parts, body) = response.into_parts();
    let (tx, rx) = tokio::sync::mpsc::channel(1);
    tokio::spawn(async move {
        let mut body = body.into_data_stream();
        let mut observer = crate::services::assistant_model_observer::StreamObserver::new(protocol);
        let mut provider_fault_recorded = false;
        let mut framing_fault_recorded = false;
        loop {
            let chunk = tokio::select! {
                biased;
                _ = tx.closed() => { let _ = request.fault("consumer_cancelled", false).await; let _ = request.complete().await; return; }
                chunk = body.next() => chunk,
            };
            match chunk {
                Some(Ok(bytes)) => {
                    for &byte in &bytes {
                        let _ = observer.push_byte(byte);
                    }
                    if observer.framing_invalid() && !framing_fault_recorded {
                        let _ = request.fault("unknown", false).await;
                        framing_fault_recorded = true;
                    }
                    if observer.provider_failed() && !provider_fault_recorded {
                        let _ = request.fault("provider_error", true).await;
                        provider_fault_recorded = true;
                    }
                    if tx.send(Ok::<_, std::io::Error>(bytes)).await.is_err() {
                        let _ = request.fault("consumer_cancelled", false).await;
                        let _ = request.complete().await;
                        return;
                    }
                }
                Some(Err(_)) => {
                    let (kind, attributable) = observer.transport_fault();
                    let _ = request.fault(kind, attributable).await;
                    let _ = request.complete().await;
                    let _ = tx
                        .send(Err(std::io::Error::other("Model stream interrupted")))
                        .await;
                    return;
                }
                None => {
                    if let Some(kind) = observer.eof()
                        && !(kind == "provider_error" && provider_fault_recorded)
                        && !(kind == "unknown" && framing_fault_recorded)
                    {
                        let _ = request.fault(kind, kind != "unknown").await;
                    }
                    let _ = request.complete().await;
                    return;
                }
            }
        }
    });
    Ok(Response::from_parts(
        parts,
        Body::from_stream(ReceiverStream::new(rx)),
    ))
}

pub(crate) fn proxy_client_disconnected(service_id: &str) -> AppError {
    tracing::debug!(
        service_id,
        "Cancelled upstream proxy work after downstream client disconnected"
    );
    // Deliberately not `Internal`: the client hanging up is not a server fault,
    // and this response is never written anywhere. Reporting it as a 500 turns
    // ordinary client cancellation into false error-rate signal.
    AppError::ClientDisconnected
}

pub(crate) fn log_upstream_error(
    service_id: &str,
    status: StatusCode,
    response_size: usize,
    upstream_request_id: &str,
) {
    tracing::error!(
        service_id = %service_id,
        status = %status,
        response_size,
        upstream_request_id,
        "Upstream returned error response"
    );
}

pub(crate) const ALLOWED_RESPONSE_HEADERS: &[&str] = &[
    "content-type",
    "content-length",
    "content-encoding",
    "content-language",
    "content-disposition",
    "cache-control",
    "etag",
    "last-modified",
    "x-request-id",
    "x-correlation-id",
    "accept-ranges",
    "content-range",
    "content-profile",
    "range-unit",
    "retry-after",
    "preference-applied",
    "location",
    "operation-location",
    "content-location",
];

#[derive(Clone)]
pub(crate) struct ProxyExchangeDiagnostics {
    started_at: std::time::Instant,
    headers_at: std::time::Instant,
    target_resolution_admission_ms: u64,
    downstream_headers_ms: Option<u64>,
    method: String,
    status: u16,
    response_mode: &'static str,
    trace_id: String,
    request_id: String,
}

impl ProxyExchangeDiagnostics {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        started_at: std::time::Instant,
        target_resolution_admission_ms: u64,
        downstream_headers_ms: Option<u64>,
        method: &Method,
        status: StatusCode,
        response_mode: &'static str,
        headers: &axum::http::HeaderMap,
    ) -> Self {
        Self {
            started_at,
            headers_at: std::time::Instant::now(),
            target_resolution_admission_ms,
            downstream_headers_ms,
            method: method.as_str().to_string(),
            status: status.as_u16(),
            response_mode,
            trace_id: sanitized_trace_id(headers).unwrap_or_default(),
            request_id: sanitized_request_id(headers).unwrap_or_default(),
        }
    }

    pub(crate) fn emit(&self, termination: &'static str) {
        let body_ms = elapsed_ms(self.headers_at);
        let downstream_headers_available = self.downstream_headers_ms.is_some();
        let downstream_headers_ms = self.downstream_headers_ms.unwrap_or(0);
        tracing::info!(
            target_resolution_admission_ms = self.target_resolution_admission_ms,
            connect_send_ms = tracing::field::Empty,
            downstream_headers_ms,
            downstream_headers_available,
            body_ms,
            total_ms = elapsed_ms(self.started_at),
            method = %self.method,
            upstream_status = self.status,
            response_mode = self.response_mode,
            termination,
            trace_id = %self.trace_id,
            request_id = %self.request_id,
            "Proxy exchange phase diagnostics"
        );
    }
}

pub(crate) struct ProxyStreamDiagnostics {
    diagnostics: ProxyExchangeDiagnostics,
    finished: bool,
}

impl ProxyStreamDiagnostics {
    pub(crate) fn new(diagnostics: ProxyExchangeDiagnostics) -> Self {
        Self {
            diagnostics,
            finished: false,
        }
    }

    pub(crate) fn finish(&mut self, termination: &'static str) {
        if !self.finished {
            self.finished = true;
            self.diagnostics.emit(termination);
        }
    }
}

impl Drop for ProxyStreamDiagnostics {
    fn drop(&mut self) {
        if !self.finished {
            self.finished = true;
            self.diagnostics.emit("client_disconnect");
        }
    }
}

pub(crate) fn elapsed_ms(started_at: std::time::Instant) -> u64 {
    started_at.elapsed().as_millis().min(u64::MAX as u128) as u64
}

pub(crate) fn emit_preheader_diagnostics(
    started_at: std::time::Instant,
    target_resolution_admission_ms: u64,
    method: &Method,
    headers: &axum::http::HeaderMap,
    termination: &'static str,
) {
    let trace_id = sanitized_trace_id(headers).unwrap_or_default();
    let request_id = sanitized_request_id(headers).unwrap_or_default();
    tracing::info!(
        target_resolution_admission_ms,
        connect_send_ms = tracing::field::Empty,
        downstream_headers_ms = 0u64,
        downstream_headers_available = false,
        body_ms = 0u64,
        total_ms = elapsed_ms(started_at),
        method = %method,
        upstream_status = 0u16,
        response_mode = "buffered",
        termination,
        trace_id = %trace_id,
        request_id = %request_id,
        "Proxy exchange phase diagnostics"
    );
}

pub(crate) fn sanitized_trace_id(headers: &axum::http::HeaderMap) -> Option<String> {
    if !proxy_service::valid_traceparent_header(headers) {
        return None;
    }
    let traceparent = headers.get("traceparent")?.to_str().ok()?;
    Some(traceparent[3..35].to_string())
}

pub(crate) fn sanitized_request_id(headers: &axum::http::HeaderMap) -> Option<String> {
    let value = headers.get("x-request-id")?.to_str().ok()?;
    (value.len() <= 128
        && !value.is_empty()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b':' | b'-')))
    .then(|| value.to_string())
}

pub(crate) fn should_capture_llm_usage(
    service: &crate::models::downstream_service::DownstreamService,
    platform_metric: BillingMetric,
) -> bool {
    platform_metric.is_token_family()
        || crate::services::billing::metric_resolution::captures_usage(service)
}

pub(crate) fn resale_usage_from_optional_reported(
    metric: BillingMetric,
    usage: Option<&llm_usage_service::ReportedLlmUsage>,
    fallback_bytes: i64,
) -> Option<ResaleUsage> {
    match metric {
        BillingMetric::Tokens => Some(ResaleUsage {
            metric,
            quantity: llm_usage_service::token_quantity_or_estimate(usage, fallback_bytes),
        }),
        BillingMetric::Requests => Some(ResaleUsage {
            metric,
            quantity: 1,
        }),
        BillingMetric::Bytes => Some(ResaleUsage {
            metric,
            quantity: fallback_bytes.max(0),
        }),
        _ => None,
    }
}

pub(crate) fn llm_platform_usage(
    usage: Option<&llm_usage_service::ReportedLlmUsage>,
    fallback_bytes: i64,
) -> PlatformUsage {
    llm_usage_service::platform_usage(usage, fallback_bytes, true)
}

pub(crate) async fn settle_meter_async(
    billing: std::sync::Arc<crate::services::billing::BillingService>,
    metered: crate::services::billing::MeteredProxyContext,
    platform: PlatformUsage,
    resale: Option<ResaleUsage>,
    model: Option<String>,
) {
    if !metered.is_enabled() {
        return;
    }

    if billing
        .settle_deferred(&metered, platform, resale, model)
        .await
        .is_err()
    {
        let billing_request_id = metered
            .route
            .as_ref()
            .map(|route| route.billing_request_id.as_str())
            .unwrap_or("unknown");
        tracing::warn!(
            billing_request_id,
            "Failed to persist usage settlement intent"
        );
    }
}

/// Threshold below which non-error responses are buffered (so small API
/// responses keep the existing diagnostic-logging path).
const STREAM_SIZE_THRESHOLD: u64 = 256 * 1024;

/// Cap on the bounded response copy kept for LLM usage extraction when a
/// chunked JSON body streams through the passthrough branch. Bodies larger
/// than this fall back to the byte-based token estimate.
///
/// This buffer is a second copy of the body, retained for the lifetime of the
/// response and multiplied by in-flight LLM request concurrency, so the cap is
/// what bounds the handler's worst-case footprint. 512 KiB of JSON is on the
/// order of 100k output tokens in a single non-streaming completion — past what
/// providers will return in one response — so real bodies stay under it. Going
/// over is not a billing regression: the fallback byte estimate is what every
/// one of these responses used before usage capture existed.
pub(crate) const USAGE_CAPTURE_MAX_BYTES: usize = 512 * 1024;

/// Content types that should always be streamed regardless of size.
pub(crate) const STREAMING_CONTENT_TYPES: &[&str] = &[
    "text/event-stream",
    "video/",
    "audio/",
    "application/octet-stream",
    "image/",
    "application/pdf",
];

pub(crate) fn forwardable_response_header(name_lower: &str, is_sse: bool) -> bool {
    if is_sse && name_lower == "content-length" {
        return false;
    }
    ALLOWED_RESPONSE_HEADERS.contains(&name_lower)
}

pub(crate) fn should_stream_response(
    response: &reqwest::Response,
    status: StatusCode,
    is_sse: bool,
) -> bool {
    // SSE always streams (existing behaviour)
    if is_sse {
        return true;
    }

    // 206 Partial Content always streams (range responses)
    if status == StatusCode::PARTIAL_CONTENT {
        return true;
    }

    // Check content type for media / binary types
    if let Some(ct) = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
    {
        let ct_lower = ct.to_lowercase();
        if STREAMING_CONTENT_TYPES
            .iter()
            .any(|prefix| ct_lower.starts_with(prefix))
        {
            return true;
        }
    }

    // Stream when content-length is absent (unknown size) or large
    match response.content_length() {
        None => true,
        Some(len) => len > STREAM_SIZE_THRESHOLD,
    }
}

#[cfg(test)]
#[path = "proxy_http_response_tests.rs"]
mod tests;

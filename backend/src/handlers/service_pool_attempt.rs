use super::*;
use crate::models::usage_meter::{PoolAttemptAccounting, PoolAttemptOutcome, PoolCompletionCause};

#[derive(Clone)]
pub(super) struct AttemptContext {
    pub prepared_chat: Option<crate::services::pool_ai_service::PreparedChat>,
    pub audit: std::sync::Arc<AttemptAudit>,
    pub request_id: String,
    pub finalization: std::sync::Arc<Finalization>,
    pub metadata: PoolAttemptAccounting,
    pub ticket: service_pool_health_service::ObservationTicket,
    pub policy: crate::models::service_pool::FailoverPolicy,
    pub timed_out: std::sync::Arc<std::sync::atomic::AtomicBool>,
    pub lease_lost: std::sync::Arc<std::sync::atomic::AtomicBool>,
    /// Set before completion settles; a renew failing after this is expected.
    pub settling: std::sync::Arc<std::sync::atomic::AtomicBool>,
    pub status: std::sync::Arc<std::sync::atomic::AtomicU16>,
    pub node_dispatched: std::sync::Arc<std::sync::atomic::AtomicBool>,
    pub retry_after: std::sync::Arc<std::sync::Mutex<Option<std::time::Duration>>>,
}

impl AttemptContext {
    fn cancellation_cause(&self) -> PoolCompletionCause {
        if self.lease_lost.load(std::sync::atomic::Ordering::Acquire) {
            PoolCompletionCause::LeaseLost
        } else if self.timed_out.load(std::sync::atomic::Ordering::Acquire) {
            PoolCompletionCause::UpstreamTimeout
        } else {
            PoolCompletionCause::CallerCancelled
        }
    }

    pub(super) fn infrastructure_failed(&self) -> bool {
        self.lease_lost.load(std::sync::atomic::Ordering::Acquire)
            || self
                .finalization
                .state
                .load(std::sync::atomic::Ordering::Acquire)
                == 3
    }

    pub(super) async fn observe_failure(
        &self,
        db: &mongodb::Database,
        evidence: crate::services::pool_failover::AttemptEvidence,
        status: Option<StatusCode>,
        retry_after: Option<std::time::Duration>,
    ) -> AppResult<()> {
        if let Some(trigger) = crate::services::pool_failover::trigger_for(evidence)
            && self.policy.retry_on.contains(&trigger)
        {
            service_pool_health_service::record_failure(
                db,
                &self.ticket,
                &self.policy.cooldown,
                trigger.as_str(),
                status.map(|s| i32::from(s.as_u16())),
                retry_after,
            )
            .await?;
        }
        Ok(())
    }

    fn observe_headers(&self, status: StatusCode, headers: &HeaderMap) {
        *self.retry_after.lock().unwrap_or_else(|e| e.into_inner()) =
            crate::services::pool_failover::parse_retry_after(headers, chrono::Utc::now());
        self.status
            .store(status.as_u16(), std::sync::atomic::Ordering::Release);
    }

    pub(super) fn retry_after(&self) -> Option<std::time::Duration> {
        *self.retry_after.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// One metadata owner spans preparation, transport, body completion and Drop.
/// Body completion can precede the first-frame gate; defer its event until the
/// loop commits the response or supplies the final retry/transport decision.
pub(super) struct AttemptAudit {
    db: mongodb::Database,
    auth: AuthUser,
    pool_id: String,
    member_id: String,
    attempt: u32,
    tier: u32,
    state: std::sync::Mutex<AttemptAuditState>,
}

#[derive(Default)]
struct AttemptAuditState {
    emitted: bool,
    committed: bool,
    body: Option<(StatusCode, PoolCompletionCause)>,
}

impl AttemptAudit {
    pub(super) fn new(
        db: &mongodb::Database,
        auth: &AuthUser,
        pool_id: &str,
        member_id: &str,
        attempt: u32,
        tier: u32,
    ) -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self {
            db: db.clone(),
            auth: auth.clone(),
            pool_id: pool_id.into(),
            member_id: member_id.into(),
            attempt,
            tier,
            state: Default::default(),
        })
    }

    fn emit(
        &self,
        state: &mut AttemptAuditState,
        reason: &str,
        status: Option<StatusCode>,
        outcome: &str,
    ) {
        if state.emitted {
            return;
        }
        state.emitted = true;
        audit_service::log_for_user(
            self.db.clone(),
            &self.auth,
            "service_pool_attempt",
            Some(serde_json::json!({
                "pool_id": self.pool_id, "user_service_id": self.member_id,
                "priority": self.tier, "attempt": self.attempt,
                "upstream_status": status.map(|s| s.as_u16()), "reason": reason, "outcome": outcome,
            })),
        );
    }

    pub(super) fn record(&self, reason: &str, status: Option<StatusCode>, outcome: &str) {
        self.emit(
            &mut self.state.lock().unwrap_or_else(|e| e.into_inner()),
            reason,
            status,
            outcome,
        );
    }

    fn emit_body(&self, state: &mut AttemptAuditState) {
        if let Some((status, cause)) = state.body {
            let reason = match cause {
                PoolCompletionCause::Complete if !status.is_success() => {
                    format!("http_{}", status.as_u16())
                }
                PoolCompletionCause::Complete => "success".into(),
                _ => cause.as_str().into(),
            };
            let outcome = match cause {
                PoolCompletionCause::Complete => "response_complete",
                PoolCompletionCause::UpstreamBodyFailure | PoolCompletionCause::UpstreamTimeout => {
                    "body_interrupted"
                }
                _ => cause.as_str(),
            };
            self.emit(state, &reason, Some(status), outcome);
        }
    }

    fn body_finished(&self, status: StatusCode, cause: PoolCompletionCause) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.body = Some((status, cause));
        if state.committed {
            self.emit_body(&mut state);
        }
    }

    pub(super) fn commit(&self) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        state.committed = true;
        self.emit_body(&mut state);
    }
}

impl Drop for AttemptAudit {
    fn drop(&mut self) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        // Covers cancellation during preparation or failed finalization, where
        // neither the loop nor the response owner reaches its explicit outcome.
        self.emit(&mut state, "cancelled", None, "cancelled");
    }
}

#[derive(Default)]
pub(super) struct Finalization {
    state: std::sync::atomic::AtomicU8,
    changed: tokio::sync::Notify,
}

impl Finalization {
    fn finish(&self, success: bool) {
        self.state.store(
            if success { 2 } else { 3 },
            std::sync::atomic::Ordering::Release,
        );
        self.changed.notify_waiters();
    }
}

pub(super) async fn attempt_timeout<T>(
    context: &AttemptContext,
    deadline: std::time::Instant,
    work: impl std::future::Future<Output = T>,
) -> Result<T, ()> {
    tokio::pin!(work);
    tokio::select! {
        result = &mut work => Ok(result),
        _ = tokio::time::sleep_until(tokio::time::Instant::from_std(deadline)) => {
            context.timed_out.store(true, std::sync::atomic::Ordering::Release);
            Err(())
        }
    }
}

pub(super) async fn within_deadline<T>(
    deadline: std::time::Instant,
    summaries: &[crate::errors::PoolAttemptSummary],
    work: impl std::future::Future<Output = AppResult<T>>,
) -> AppResult<T> {
    if std::time::Instant::now() >= deadline {
        return Err(AppError::ServicePoolDeadlineExceeded {
            attempts: summaries.to_vec(),
        });
    }
    tokio::time::timeout_at(tokio::time::Instant::from_std(deadline), work)
        .await
        .map_err(|_| AppError::ServicePoolDeadlineExceeded {
            attempts: summaries.to_vec(),
        })?
}

async fn await_finalization(finalization: &Finalization) -> AppResult<()> {
    loop {
        let changed = finalization.changed.notified();
        tokio::pin!(changed);
        changed.as_mut().enable();
        match finalization
            .state
            .load(std::sync::atomic::Ordering::Acquire)
        {
            1 => changed.await,
            3 => {
                return Err(AppError::ServicePoolInfrastructureUnavailable);
            }
            _ => return Ok(()),
        }
    }
}

pub(super) async fn finish_attempt(
    db: &mongodb::Database,
    context: &AttemptContext,
    outcome: PoolAttemptOutcome,
    deadline: std::time::Instant,
    summaries: &[crate::errors::PoolAttemptSummary],
) -> AppResult<()> {
    within_deadline(deadline, summaries, async {
        await_finalization(&context.finalization).await?;
        Box::pin(crate::services::billing::pool_attempt::finish(
            db,
            &context.request_id,
            outcome,
        ))
        .await
        .map(|_| ())
        .map_err(|error| {
            tracing::warn!(error=%error,"Pool attempt cleanup requires recovery");
            AppError::ServicePoolInfrastructureUnavailable
        })
    })
    .await
}

#[derive(Clone)]
pub(super) struct Observability {
    pub usage: Option<llm_usage_service::UsageAuditContext>,
    pub diagnostics: Option<ProxyExchangeDiagnostics>,
}

/// Owns accounting even before the lazy body is polled. Dropping a response
/// cannot discard its reported usage or its durable cleanup obligation.
struct Completion {
    db: mongodb::Database,
    billing: std::sync::Arc<crate::services::billing::BillingService>,
    auth: AuthUser,
    context: AttemptContext,
    metered: crate::services::billing::MeteredProxyContext,
    path: String,
    status: StatusCode,
    request_len: i64,
    response_len: i64,
    captured: Option<Vec<u8>>,
    events: llm_usage_service::BoundedUsageEvents,
    extracted_usage: Option<llm_usage_service::ReportedLlmUsage>,
    usage_extracted: bool,
    heartbeat: Option<tokio::task::JoinHandle<()>>,
    sse: bool,
    completed: bool,
    cause: Option<PoolCompletionCause>,
    usage_logged: bool,
    spawn_cleanup_on_drop: bool,
    observability: Observability,
}

impl Completion {
    fn observe(&mut self, bytes: &[u8]) {
        self.response_len = self.response_len.saturating_add(bytes.len() as i64);
        if self.sse {
            self.events.push(bytes);
        } else if let Some(captured) = self.captured.as_mut() {
            if captured.len() + bytes.len() <= USAGE_CAPTURE_MAX_BYTES {
                captured.extend_from_slice(bytes);
            } else {
                self.captured = None;
            }
        }
    }

    async fn finish(&mut self, cause: PoolCompletionCause) -> AppResult<()> {
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(30),
            Box::pin(self.finish_inner(cause)),
        )
        .await
        .unwrap_or(Err(AppError::ServicePoolInfrastructureUnavailable));
        if self.completed {
            return Ok(());
        }
        if result.is_err() {
            self.context.finalization.finish(false);
        }
        result.map_err(|error| {
            tracing::warn!(error = %error, "Pool completion bookkeeping awaits recovery");
            AppError::ServicePoolInfrastructureUnavailable
        })
    }

    async fn finish_inner(&mut self, cause: PoolCompletionCause) -> AppResult<()> {
        // Settlement releases the meter rows, after which a renew still in
        // flight matches nothing. Mark settlement first so the heartbeat keeps
        // the lease alive until then but never reports a settled attempt as
        // lease-lost (an infrastructure failure).
        self.context
            .settling
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let cause = *self.cause.get_or_insert(cause);
        let complete_body = cause == PoolCompletionCause::Complete;
        self.context.audit.body_finished(self.status, cause);
        if !self.usage_extracted {
            self.extracted_usage = if self.sse {
                std::mem::take(&mut self.events).finalize_success(self.status.is_success())
            } else {
                self.captured.as_deref().and_then(|body| {
                    llm_usage_service::usage_from_body(body, &self.path, self.status.is_success())
                })
            };
            self.usage_extracted = true;
        }
        let usage = self.extracted_usage.clone();
        crate::services::billing::pool_attempt::record_completion(
            &self.db,
            &self.context.request_id,
            cause,
        )
        .await?;
        let quantity_is_known = usage.is_some()
            || (complete_body
                && self.status.is_success()
                && self.metered.route.as_ref().is_some_and(|route| {
                    !route.capture_tokens
                        && route.resale.as_ref().is_none_or(|spec| {
                            matches!(spec.metric, BillingMetric::Requests | BillingMetric::Bytes)
                        })
                }));
        if quantity_is_known {
            let bytes = self.request_len.saturating_add(self.response_len);
            let platform = llm_usage_service::platform_usage(usage.as_ref(), bytes, false);
            let resale = self
                .metered
                .route
                .as_ref()
                .and_then(|route| route.resale.as_ref())
                .and_then(|spec| {
                    resale_usage_from_optional_reported(spec.metric, usage.as_ref(), bytes)
                });
            // Await real settlement, not just the deferred-intent spawn: the
            // backup may need these exact same wallet/grant/allowance funds.
            Box::pin(self.billing.settle(&self.metered, platform, resale, None)).await?;
        } else {
            Box::pin(crate::services::billing::pool_attempt::finish(
                &self.db,
                &self.context.request_id,
                if self.status == StatusCode::TOO_MANY_REQUESTS {
                    PoolAttemptOutcome::Rejected
                } else {
                    PoolAttemptOutcome::Unknown
                },
            ))
            .await?;
        }
        if !self.usage_logged {
            if let Some(context) = &self.observability.usage
                && let Some(usage) = usage
            {
                llm_usage_service::log_reported_usage_async(context.clone(), usage);
            }
            self.usage_logged = true;
        }
        if let Some(diagnostics) = &self.observability.diagnostics {
            diagnostics.emit(cause.as_str());
        }
        if let Some(heartbeat) = self.heartbeat.take() {
            heartbeat.abort();
        }
        self.completed = true;
        self.context.finalization.finish(true);
        if complete_body && self.status.is_success() {
            // Response EOF follows acknowledged settlement. Advisory health must
            // not keep an empty success behind the first-frame deadline. The
            // ticket's durable sequence/revision fences still order this write.
            let db = self.db.clone();
            let ticket = self.context.ticket.clone();
            tokio::spawn(async move {
                match tokio::time::timeout(
                    std::time::Duration::from_secs(30),
                    service_pool_health_service::record_success(&db, &ticket),
                )
                .await
                {
                    Ok(Ok(_)) => {}
                    Ok(Err(error)) => {
                        tracing::warn!(error = %error, "Pool success health observation could not be recorded")
                    }
                    Err(_) => tracing::warn!("Pool success health observation timed out"),
                }
            });
        } else if matches!(
            cause,
            PoolCompletionCause::UpstreamBodyFailure | PoolCompletionCause::UpstreamTimeout
        ) {
            let evidence = if !self.status.is_success() {
                crate::services::pool_failover::AttemptEvidence::Upstream(self.status)
            } else if cause == PoolCompletionCause::UpstreamTimeout {
                crate::services::pool_failover::AttemptEvidence::Timeout
            } else {
                crate::services::pool_failover::AttemptEvidence::TransportAfterDispatch
            };
            if let Err(error) = self
                .context
                .observe_failure(
                    &self.db,
                    evidence,
                    Some(self.status),
                    self.context.retry_after(),
                )
                .await
            {
                tracing::warn!(error = %error, "Pool body failure health observation could not be recorded");
            }
        }
        Ok(())
    }
}

impl Drop for Completion {
    fn drop(&mut self) {
        if let Some(task) = self.heartbeat.take() {
            task.abort();
        }
        if self.completed {
            return;
        }
        if !self.spawn_cleanup_on_drop {
            self.context.finalization.finish(false);
            return;
        }
        // Move observations into a guard-owned finalizer. The lease on every
        // meter row is the process-crash backstop for this best-effort task.
        let mut completion = Self {
            db: self.db.clone(),
            billing: self.billing.clone(),
            auth: self.auth.clone(),
            context: self.context.clone(),
            metered: self.metered.clone(),
            path: self.path.clone(),
            status: self.status,
            request_len: self.request_len,
            response_len: self.response_len,
            captured: self.captured.take(),
            events: std::mem::take(&mut self.events),
            extracted_usage: self.extracted_usage.clone(),
            usage_extracted: self.usage_extracted,
            heartbeat: None,
            sse: self.sse,
            completed: false,
            cause: self.cause,
            usage_logged: self.usage_logged,
            spawn_cleanup_on_drop: false,
            observability: self.observability.clone(),
        };
        self.completed = true;
        tokio::spawn(async move {
            if let Err(error) =
                Box::pin(completion.finish(completion.context.cancellation_cause())).await
            {
                completion.context.finalization.finish(false);
                tracing::warn!(error = %error, "Pool attempt finalization deferred to durable recovery");
            }
            // Never recursively spawn another Drop finalizer on DB failure.
            completion.completed = true;
            drop(completion);
        });
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn direct_response(
    state: &AppState,
    auth: &AuthUser,
    context: AttemptContext,
    response: reqwest::Response,
    metered: crate::services::billing::MeteredProxyContext,
    request_len: i64,
    path: &str,
    cancellation: tokio_util::sync::CancellationToken,
    location: Option<&AsyncLocationContext>,
    observability: Observability,
) -> AppResult<Response> {
    let status = response.status();
    context.observe_headers(status, response.headers());
    let sse = response
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .is_some_and(crate::mw::security_headers::is_sse_media_type);
    let mut builder = Response::builder().status(status);
    for (name, value) in response.headers() {
        if let Some(value) =
            forwarded_response_header_value(name.as_str(), value.as_bytes(), sse, location)
        {
            builder = builder.header(name, value);
        }
    }
    let response = builder
        .body(Body::from_stream(response.bytes_stream()))
        .map_err(|error| AppError::Internal(error.to_string()))?;
    response_body(
        state,
        auth,
        context,
        response,
        metered,
        request_len,
        path,
        cancellation,
        observability,
    )
    .await
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn response_body(
    state: &AppState,
    auth: &AuthUser,
    context: AttemptContext,
    response: Response,
    metered: crate::services::billing::MeteredProxyContext,
    request_len: i64,
    path: &str,
    cancellation: tokio_util::sync::CancellationToken,
    observability: Observability,
) -> AppResult<Response> {
    let (mut parts, body) = response.into_parts();
    let status = parts.status;
    context.observe_headers(status, &parts.headers);
    let encoded = parts
        .headers
        .get_all(axum::http::header::CONTENT_ENCODING)
        .iter()
        .any(|value| {
            value.to_str().map_or(true, |value| {
                value
                    .split(',')
                    .any(|encoding| !encoding.trim().eq_ignore_ascii_case("identity"))
            })
        });
    if encoded && status.is_success() {
        return Err(AppError::ServicePoolUpstreamEncodingUnsupported);
    }
    let sse = !encoded
        && parts
            .headers
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .is_some_and(crate::mw::security_headers::is_sse_media_type);
    context
        .finalization
        .state
        .store(1, std::sync::atomic::Ordering::Release);
    let mut adapter = if status.is_success() {
        context.prepared_chat.as_ref().map(|prepared| {
            crate::services::pool_ai_service::ResponseAdapter::new(prepared)
                .with_native_streaming(sse)
        })
    } else {
        None
    };
    if adapter.is_some() {
        parts.headers.remove(axum::http::header::CONTENT_LENGTH);
        parts.headers.remove(axum::http::header::CONTENT_ENCODING);
        parts.headers.insert(
            axum::http::header::CONTENT_TYPE,
            axum::http::HeaderValue::from_static(
                if context.prepared_chat.as_ref().is_some_and(|p| p.streaming) {
                    "text/event-stream"
                } else {
                    "application/json"
                },
            ),
        );
    }
    let mut completion = Completion {
        db: state.db.clone(),
        billing: state.billing.clone(),
        auth: auth.clone(),
        context,
        metered,
        path: path.into(),
        status,
        request_len,
        response_len: 0,
        captured: (!encoded).then(Vec::new),
        events: Default::default(),
        extracted_usage: None,
        usage_extracted: false,
        heartbeat: None,
        sse,
        completed: false,
        cause: None,
        usage_logged: false,
        spawn_cleanup_on_drop: true,
        observability,
    };
    let idle = std::time::Duration::from_secs(state.config.proxy_stream_idle_timeout_secs);
    let lease = chrono::Duration::seconds(state.config.proxy_stream_idle_timeout_secs as i64 + 30);
    let expected_rows = completion.metered.route.as_ref().map_or(0, |route| {
        (if route.platform_metered() {
            route.platform_specs().count()
        } else {
            0
        }) as u64
            + u64::from(route.resale.is_some())
    });
    let mut lease_until = completion.context.metadata.lease_until;
    let heartbeat_db = completion.db.clone();
    let heartbeat_request = completion.context.request_id.clone();
    let heartbeat_cancel = cancellation.clone();
    let lease_lost = completion.context.lease_lost.clone();
    let settling = completion.context.settling.clone();
    completion.heartbeat = Some(tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(10));
        loop {
            tokio::select! {
                () = heartbeat_cancel.cancelled() => return,
                _ = interval.tick() => {
                    let remaining = (lease_until - chrono::Utc::now()).to_std().unwrap_or_default();
                    let next_lease = chrono::Utc::now() + lease;
                    let result = tokio::time::timeout(remaining, crate::services::billing::pool_attempt::renew(&heartbeat_db, &heartbeat_request, next_lease, expected_rows)).await;
                    if !matches!(result, Ok(Ok(true))) {
                        if settling.load(std::sync::atomic::Ordering::SeqCst) {
                            // Settlement released the rows; the lease is no longer needed.
                            return;
                        }
                        lease_lost.store(true, std::sync::atomic::Ordering::Release);
                        heartbeat_cancel.cancel();
                        return;
                    }
                    lease_until = next_lease;
                }
            }
        }
    }));
    let mut upstream = body.into_data_stream();
    let token = cancellation.clone();
    let (tx, rx) = tokio::sync::mpsc::channel::<Result<bytes::Bytes, std::io::Error>>(8);
    // Provider ownership stays in this task. Cancellation wins even when the
    // client stops polling and the bounded response queue is full.
    tokio::spawn(async move {
        loop {
            let next =
                until_client_disconnect(&token, tokio::time::timeout(idle, upstream.next())).await;
            match next {
                Ok(Ok(Some(Ok(bytes)))) => {
                    completion.observe(&bytes);
                    let chunks = match adapter
                        .as_mut()
                        .map_or_else(|| Ok(vec![bytes.clone()]), |adapter| adapter.push(&bytes))
                    {
                        Ok(chunks) => chunks,
                        Err(_) => {
                            drop(upstream);
                            let _ = Box::pin(
                                completion.finish(PoolCompletionCause::UpstreamBodyFailure),
                            )
                            .await;
                            let _ = tx
                                .send(Err(std::io::Error::other("Native AI response failed")))
                                .await;
                            return;
                        }
                    };
                    for chunk in chunks {
                        if !matches!(
                            until_client_disconnect(&token, tx.send(Ok(chunk))).await,
                            Ok(Ok(()))
                        ) {
                            drop(upstream);
                            let _ = Box::pin(
                                completion.finish(completion.context.cancellation_cause()),
                            )
                            .await;
                            return;
                        }
                    }
                }
                Ok(Ok(None)) => {
                    let chunks = match adapter
                        .as_mut()
                        .map_or_else(|| Ok(Vec::new()), |adapter| adapter.finish())
                    {
                        Ok(chunks) => chunks,
                        Err(_) => {
                            let _ = Box::pin(
                                completion.finish(PoolCompletionCause::UpstreamBodyFailure),
                            )
                            .await;
                            let _ = tx
                                .send(Err(std::io::Error::other(
                                    "Native AI response ended without completion",
                                )))
                                .await;
                            return;
                        }
                    };
                    // Native EOF/adapter completion is authoritative even if downstream
                    // disconnects or bookkeeping fails while sending final output.
                    completion.cause = Some(PoolCompletionCause::Complete);
                    for chunk in chunks {
                        if !matches!(
                            until_client_disconnect(&token, tx.send(Ok(chunk))).await,
                            Ok(Ok(()))
                        ) {
                            let _ =
                                Box::pin(completion.finish(PoolCompletionCause::Complete)).await;
                            return;
                        }
                    }
                    // Recovery owns bookkeeping errors; never append an upstream error
                    // after a successfully completed body or [DONE].
                    let _ = Box::pin(completion.finish(PoolCompletionCause::Complete)).await;
                    return;
                }
                failed => {
                    drop(upstream);
                    let cause = match failed {
                        Err(_) => completion.context.cancellation_cause(),
                        Ok(Err(_)) => PoolCompletionCause::UpstreamTimeout,
                        _ => PoolCompletionCause::UpstreamBodyFailure,
                    };
                    let _ = Box::pin(completion.finish(cause)).await;
                    let _ = tx.send(Err(std::io::Error::other(cause.as_str()))).await;
                    return;
                }
            }
        }
    });
    let mut response = Response::from_parts(
        parts,
        Body::from_stream(CancelOnDropStream::new(
            ReceiverStream::new(rx),
            cancellation,
        )),
    );
    apply_agent_attribution_headers(&mut response, auth.api_key_id.as_deref(), None);
    Ok(response)
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn node_response(
    state: &AppState,
    auth: &AuthUser,
    context: AttemptContext,
    response: ProxyResponseType,
    metered: crate::services::billing::MeteredProxyContext,
    request_len: i64,
    path: &str,
    cancellation: tokio_util::sync::CancellationToken,
    location: Option<&AsyncLocationContext>,
    observability: Observability,
) -> AppResult<Response> {
    let (status, headers, body) = match response {
        ProxyResponseType::Complete(response) => {
            (response.status, response.headers, Body::from(response.body))
        }
        ProxyResponseType::Streaming(mut rx) => {
            let Some(StreamChunk::Start { status, headers }) = rx.recv().await else {
                return Err(AppError::PoolAttemptTransport(
                    crate::errors::PoolAttemptTransportKind::NodeAfterDispatch,
                ));
            };
            let stream = async_stream::stream! {
                loop {
                    match rx.recv().await {
                        Some(StreamChunk::Data(bytes)) => yield Ok::<_, std::io::Error>(bytes::Bytes::from(bytes)),
                        Some(StreamChunk::Start { .. }) => {},
                        Some(StreamChunk::End) => return,
                        _ => { yield Err(std::io::Error::other("Node response interrupted")); return; }
                    }
                }
            };
            (status, headers, Body::from_stream(stream))
        }
    };
    let is_sse = node_is_sse_headers(&headers);
    let mut builder = Response::builder().status(status);
    for (name, value) in headers {
        if let Some(value) = forwarded_response_header_value(
            &name.to_ascii_lowercase(),
            value.as_bytes(),
            is_sse,
            location,
        ) {
            builder = builder.header(name, value);
        }
    }
    let response = builder
        .body(body)
        .map_err(|e| AppError::Internal(e.to_string()))?;
    response_body(
        state,
        auth,
        context,
        response,
        metered,
        request_len,
        path,
        cancellation,
        observability,
    )
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::usage_meter::{UsageMeterRow, UsageStatus};
    use crate::services::billing::{BillingIngress, BillingRouteContext, NodeIntent, meter};
    use mongodb::bson::doc;

    async fn completion_fixture(
        label: &str,
    ) -> (mongodb::Database, Completion, std::sync::Arc<Finalization>) {
        let db = crate::test_utils::connect_transaction_test_database(label).await;
        let state = crate::test_utils::test_app_state(db.clone());
        let auth = crate::test_utils::test_auth_user(&uuid::Uuid::new_v4().to_string());
        let request_id = uuid::Uuid::new_v4().to_string();
        let metadata = PoolAttemptAccounting {
            pool_id: "pool".into(),
            member_id: "member".into(),
            attempt: 1,
            lease_until: chrono::Utc::now() + chrono::Duration::minutes(1),
            outcome: None,
            completion_cause: None,
        };
        let mut route = BillingRouteContext::new(
            BillingIngress::Proxy,
            request_id.clone(),
            auth.user_id.to_string(),
            auth.user_id.to_string(),
            None,
            Some("member".into()),
            Some("catalog".into()),
            Some("chat".into()),
            NodeIntent::Direct,
            "bearer".into(),
            CredentialClass::UserOwned,
            BillingMetric::Tokens,
            None,
            false,
        )
        .with_platform_metering(true);
        route.pool_attempt = Some(metadata.clone());
        let metered = meter::open(&db, &route, None).await.unwrap();
        meter::mark_forwarded(&db, &metered).await.unwrap();
        let finalization = std::sync::Arc::new(Finalization::default());
        finalization
            .state
            .store(1, std::sync::atomic::Ordering::Release);
        let ticket = service_pool_health_service::ObservationTicket {
            observation_id: uuid::Uuid::new_v4().to_string(),
            observation_sequence: 1,
            pool_reset_generation: 0,
            member_reset_generation: 0,
            health_instance_id: "expired".into(),
            scope: service_pool_health_service::HealthScope {
                pool_id: "pool".into(),
                user_service_id: "member".into(),
                owner_id: auth.user_id.to_string(),
                pool_config_revision: 0,
                credential_identity: "test-only".into(),
                credential_epoch: 0,
                destination_fingerprint: "test-only".into(),
                config_fingerprint: "test-only".into(),
                model: None,
            },
        };
        let context = AttemptContext {
            prepared_chat: None,
            audit: AttemptAudit::new(&db, &auth, "pool", "member", 1, 0),
            request_id: request_id.clone(),
            finalization: finalization.clone(),
            metadata,
            ticket,
            policy: Default::default(),
            lease_lost: Default::default(),
            settling: Default::default(),
            timed_out: Default::default(),
            status: Default::default(),
            node_dispatched: Default::default(),
            retry_after: Default::default(),
        };
        let completion = Completion {
            db: db.clone(),
            billing: state.billing.clone(),
            auth,
            context,
            metered,
            path: "chat/completions".into(),
            status: StatusCode::TOO_MANY_REQUESTS,
            request_len: 100,
            response_len: 0,
            captured: None,
            events: Default::default(),
            extracted_usage: None,
            usage_extracted: false,
            heartbeat: None,
            sse: true,
            completed: false,
            cause: None,
            usage_logged: false,
            spawn_cleanup_on_drop: true,
            observability: Observability {
                usage: None,
                diagnostics: None,
            },
        };
        (db, completion, finalization)
    }

    #[tokio::test]
    async fn pool_reported_sse_usage_survives_cancellation_before_settlement_acknowledgment() {
        let (db, mut completion, finalization) = completion_fixture("pool_cancel_settlement").await;
        let request_id = completion.context.request_id.clone();
        completion.observe(b"data: {\"usage\":{\"prompt_tokens\":2,\"completion_tokens\":1,\"total_tokens\":3}}\n\n");
        {
            let finish = completion.finish_inner(PoolCompletionCause::Complete);
            tokio::pin!(finish);
            // Poll into actual Mongo settlement, then cancel the owning future.
            // Extraction has consumed the SSE accumulator before this await.
            assert!(futures::poll!(&mut finish).is_pending());
        }
        assert!(completion.usage_extracted && completion.extracted_usage.is_some());
        drop(completion);
        tokio::time::timeout(
            std::time::Duration::from_secs(10),
            await_finalization(&finalization),
        )
        .await
        .unwrap()
        .unwrap();
        let row = db
            .collection::<UsageMeterRow>("usage_meter")
            .find_one(doc! {"billing_request_id":request_id})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(row.quantity, Some(3));
        assert_eq!(row.status, UsageStatus::Finalized);
        assert_eq!(
            row.pool_attempt.unwrap().outcome,
            Some(PoolAttemptOutcome::Reported)
        );
        assert!(row.released);
        db.drop().await.unwrap();
    }
    #[tokio::test]
    async fn pool_completed_response_survives_failed_and_delayed_health_writes() {
        for delayed in [false, true] {
            let (db, completion, _) = completion_fixture("pool_health_bookkeeping").await;
            let state = crate::test_utils::test_app_state(db.clone());
            let mut context = completion.context.clone();
            let auth = completion.auth.clone();
            let metered = completion.metered.clone();
            // This harness transfers ownership into the real response owner.
            let mut completion = completion;
            completion.completed = true;
            db.collection::<mongodb::bson::Document>("service_pools").insert_one(doc! {
                "_id":"pool","user_id":auth.user_id.to_string(),"slug":"pool","name":"Pool","strategy":"priority","config_revision":0_i64,
                "is_active":true,"members":[{"user_service_id":"member","enabled":true}],"created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now(),
            }).await.unwrap();
            context.ticket = service_pool_health_service::issue_observation_ticket(
                &db,
                context.ticket.scope.clone(),
            )
            .await
            .unwrap();
            let mut session = db.client().start_session().await.unwrap();
            if delayed {
                session.start_transaction().await.unwrap();
                db.collection::<mongodb::bson::Document>("service_pool_member_health")
                    .update_one(
                        doc! {"_id":&context.ticket.health_instance_id},
                        doc! {"$set":{"test_lock":true}},
                    )
                    .session(&mut session)
                    .await
                    .unwrap();
            } else {
                db.run_command(doc! {"collMod":"service_pool_member_health","validator":{"observation_outcome":{"$ne":"success"}},"validationLevel":"strict"}).await.unwrap();
            }
            let cancellation = tokio_util::sync::CancellationToken::new();
            let upstream = Response::builder().status(200).header("content-type","text/event-stream")
                .body(Body::from("data: {\"usage\":{\"prompt_tokens\":2,\"completion_tokens\":1,\"total_tokens\":3}}\n\ndata: [DONE]\n\n")).unwrap();
            let response = response_body(
                &state,
                &auth,
                context.clone(),
                upstream,
                metered,
                0,
                "chat/completions",
                cancellation.clone(),
                Observability {
                    usage: None,
                    diagnostics: None,
                },
            )
            .await
            .unwrap();
            // Let provider EOF and settlement precede the first-frame gate in the
            // failed-write case, exercising short buffered-response races too.
            tokio::time::timeout(
                std::time::Duration::from_secs(5),
                await_finalization(&context.finalization),
            )
            .await
            .unwrap()
            .unwrap();
            if delayed {
                tokio::time::sleep(std::time::Duration::from_secs(11)).await;
                assert!(
                    !context
                        .lease_lost
                        .load(std::sync::atomic::Ordering::Acquire)
                );
                assert!(
                    !cancellation.is_cancelled(),
                    "settled rows cannot trigger lease loss while health is blocked"
                );
                session.abort_transaction().await.unwrap();
            }
            assert!(!context.infrastructure_failed());
            let response = match gate_pool_response(response).await {
                PoolResponseGate::Response(response) => response,
                _ => panic!("successful provider body must survive health failure"),
            };
            assert_eq!(response.status(), StatusCode::OK);
            let bytes = axum::body::to_bytes(response.into_body(), 4096)
                .await
                .unwrap();
            assert!(
                String::from_utf8(bytes.to_vec())
                    .unwrap()
                    .contains("[DONE]")
            );
            let row = db
                .collection::<UsageMeterRow>("usage_meter")
                .find_one(doc! {})
                .await
                .unwrap()
                .unwrap();
            assert_eq!(row.quantity, Some(3));
            assert_eq!(
                row.pool_attempt.unwrap().completion_cause,
                Some(PoolCompletionCause::Complete)
            );
            assert_eq!(
                db.collection::<mongodb::bson::Document>("service_pool_member_health")
                    .count_documents(doc! {"cooldown_until":{"$gt":bson::DateTime::now()}})
                    .await
                    .unwrap(),
                0
            );
            db.drop().await.unwrap();
        }
    }
    #[tokio::test]
    async fn pool_success_eof_does_not_wait_for_locked_health() {
        for (status, bytes) in [(204, ""), (200, ""), (200, "complete")] {
            let (db, mut prior, _) = completion_fixture("pool_health_eof").await;
            prior.completed = true;
            let state = crate::test_utils::test_app_state(db.clone());
            let mut context = prior.context.clone();
            db.collection::<mongodb::bson::Document>("service_pools").insert_one(doc! {
                "_id":"pool","user_id":prior.auth.user_id.to_string(),"slug":"pool","name":"Pool","strategy":"priority","config_revision":0_i64,
                "is_active":true,"members":[{"user_service_id":"member","enabled":true}],"created_at":bson::DateTime::now(),"updated_at":bson::DateTime::now(),
            }).await.unwrap();
            context.ticket = service_pool_health_service::issue_observation_ticket(
                &db,
                context.ticket.scope.clone(),
            )
            .await
            .unwrap();
            let mut session = db.client().start_session().await.unwrap();
            session.start_transaction().await.unwrap();
            db.collection::<mongodb::bson::Document>("service_pool_member_health")
                .update_one(
                    doc! {"_id":&context.ticket.health_instance_id},
                    doc! {"$set":{"test_lock":true}},
                )
                .session(&mut session)
                .await
                .unwrap();
            let response = response_body(
                &state,
                &prior.auth,
                context.clone(),
                Response::builder()
                    .status(status)
                    .body(Body::from(bytes))
                    .unwrap(),
                prior.metered.clone(),
                0,
                "perform",
                tokio_util::sync::CancellationToken::new(),
                Observability {
                    usage: None,
                    diagnostics: None,
                },
            )
            .await
            .unwrap();
            let response = attempt_timeout(
                &context,
                std::time::Instant::now() + std::time::Duration::from_secs(2),
                gate_pool_response(response),
            )
            .await
            .expect("settled success must arrive while health remains locked");
            let PoolResponseGate::Response(response) = response else {
                panic!("successful response")
            };
            assert_eq!(response.status().as_u16(), status);
            let body = tokio::time::timeout(
                std::time::Duration::from_secs(2),
                axum::body::to_bytes(response.into_body(), 4096),
            )
            .await
            .expect("nonempty body EOF must also be independent of health")
            .unwrap();
            assert_eq!(body.as_ref(), bytes.as_bytes());
            assert!(!context.timed_out.load(std::sync::atomic::Ordering::Acquire));
            assert!(!context.infrastructure_failed());
            assert_eq!(
                db.collection::<mongodb::bson::Document>("service_pool_member_health")
                    .count_documents(doc! {"cooldown_until":{"$gt":bson::DateTime::now()}})
                    .await
                    .unwrap(),
                0
            );
            session.abort_transaction().await.unwrap();
            // The original ticket still records success after the lock releases.
            tokio::time::timeout(std::time::Duration::from_secs(5), async {
                loop {
                    if db
                        .collection::<mongodb::bson::Document>("service_pool_member_health")
                        .count_documents(doc! {"observation_outcome":"success"})
                        .await
                        .unwrap()
                        == 1
                    {
                        break;
                    }
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
            })
            .await
            .unwrap();
            let row = db
                .collection::<UsageMeterRow>("usage_meter")
                .find_one(doc! {})
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                row.pool_attempt.unwrap().completion_cause,
                Some(PoolCompletionCause::Complete)
            );
            db.drop().await.unwrap();
        }
    }

    #[tokio::test]
    async fn pool_node_missing_start_preserves_strict_unsafe_method_safety() {
        let (db, mut completion, _) = completion_fixture("pool_node_missing_start").await;
        completion.completed = true;
        let state = crate::test_utils::test_app_state(db.clone());
        let (tx, rx) = tokio::sync::mpsc::channel(1);
        drop(tx);
        let error = node_response(
            &state,
            &completion.auth,
            completion.context.clone(),
            ProxyResponseType::Streaming(rx.into()),
            completion.metered.clone(),
            0,
            "perform",
            tokio_util::sync::CancellationToken::new(),
            None,
            Observability {
                usage: None,
                diagnostics: None,
            },
        )
        .await
        .expect_err("missing Start must fail");
        let evidence = pool_attempt_evidence(&error).unwrap();
        assert_eq!(
            evidence,
            crate::services::pool_failover::AttemptEvidence::NodeTransportAfterDispatch
        );
        let policy = crate::models::service_pool::FailoverPolicy {
            retry_ambiguous_dispatch: true,
            retry_on: vec![crate::models::service_pool::RetryTrigger::TransportError],
            ..Default::default()
        };
        assert!(!pool_should_retry(evidence, &Method::POST, &policy));
        assert!(pool_should_retry(evidence, &Method::GET, &policy));
        db.drop().await.unwrap();
    }
    #[tokio::test]
    async fn pool_first_body_deadline_persists_timeout_not_caller_cancel() {
        let (db, mut prior, _) = completion_fixture("pool_timeout_cause").await;
        prior.completed = true;
        let state = crate::test_utils::test_app_state(db.clone());
        let context = prior.context.clone();
        let body = Body::from_stream(futures::stream::pending::<
            Result<bytes::Bytes, std::io::Error>,
        >());
        let response = response_body(
            &state,
            &prior.auth,
            context.clone(),
            Response::new(body),
            prior.metered.clone(),
            0,
            "perform",
            tokio_util::sync::CancellationToken::new(),
            Observability {
                usage: None,
                diagnostics: None,
            },
        )
        .await
        .unwrap();
        assert!(
            attempt_timeout(
                &context,
                std::time::Instant::now() + std::time::Duration::from_millis(50),
                gate_pool_response(response)
            )
            .await
            .is_err()
        );
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            await_finalization(&context.finalization),
        )
        .await
        .unwrap()
        .unwrap();
        crate::services::billing::pool_attempt::record_completion(
            &db,
            &context.request_id,
            PoolCompletionCause::CallerCancelled,
        )
        .await
        .unwrap();
        let row = db
            .collection::<UsageMeterRow>("usage_meter")
            .find_one(doc! {})
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            row.pool_attempt.unwrap().completion_cause,
            Some(PoolCompletionCause::UpstreamTimeout)
        );
        assert!(row.released && row.quantity.is_none());
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn pool_completed_stream_keeps_success_when_settlement_needs_durable_recovery() {
        let (db, mut prior, _) = completion_fixture("pool_settlement_recovery").await;
        prior.completed = true;
        let state = crate::test_utils::test_app_state(db.clone());
        let context = prior.context.clone();
        // Keep the known finalized quantity durable, then fail reservation
        // release. Single-meter routes store intent in quantity/status rather
        // than the multi-component pending_platform_usage coordinator field.
        db.run_command(doc! {"collMod":"usage_meter","validator":{"released":{"$ne":true}},"validationLevel":"strict"}).await.unwrap();
        let (release, pending) = tokio::sync::oneshot::channel::<()>();
        let native = async_stream::stream! {
            yield Ok::<_,std::io::Error>(bytes::Bytes::from_static(b"data: {\"choices\":[{\"delta\":{\"content\":\"hello\"}}]}\n\n"));
            pending.await.unwrap();
            yield Ok(bytes::Bytes::from_static(b"data: {\"usage\":{\"prompt_tokens\":2,\"completion_tokens\":1,\"total_tokens\":3}}\n\ndata: [DONE]\n\n"));
        };
        let upstream = Response::builder()
            .header("content-type", "text/event-stream")
            .body(Body::from_stream(native))
            .unwrap();
        let response = response_body(
            &state,
            &prior.auth,
            context.clone(),
            upstream,
            prior.metered.clone(),
            0,
            "chat/completions",
            tokio_util::sync::CancellationToken::new(),
            Observability {
                usage: None,
                diagnostics: None,
            },
        )
        .await
        .unwrap();
        let response = match gate_pool_response(response).await {
            PoolResponseGate::Response(response) => response,
            _ => panic!("first body"),
        };
        context.audit.commit();
        release.send(()).unwrap();
        let bytes = axum::body::to_bytes(response.into_body(), 4096)
            .await
            .expect("bookkeeping cannot append body error after success");
        assert!(
            String::from_utf8(bytes.to_vec())
                .unwrap()
                .contains("[DONE]")
        );
        assert!(matches!(
            await_finalization(&context.finalization).await,
            Err(AppError::ServicePoolInfrastructureUnavailable)
        ));
        let rows = db.collection::<UsageMeterRow>("usage_meter");
        let pending = rows.find_one(doc! {}).await.unwrap().unwrap();
        assert_eq!(pending.quantity, Some(3));
        // Known quantity remains finalized; failed money release uses the
        // existing retry status, not a new pool-specific settlement path.
        assert!(pending.finalized_at.is_some());
        assert_eq!(pending.status, UsageStatus::Failed);
        assert!(pending.settlement_attempts >= 1);
        assert!(pending.settlement_next_retry_at.is_some());
        assert_eq!(
            pending.pool_attempt.as_ref().unwrap().completion_cause,
            Some(PoolCompletionCause::Complete)
        );
        assert!(!pending.released);
        db.run_command(doc! {"collMod":"usage_meter","validator":{}})
            .await
            .unwrap();
        crate::services::billing::pool_attempt::finish(
            &db,
            &context.request_id,
            PoolAttemptOutcome::Unknown,
        )
        .await
        .unwrap();
        let recovered = rows.find_one(doc! {}).await.unwrap().unwrap();
        assert!(recovered.released);
        assert_eq!(recovered.quantity, Some(3));
        let metadata = recovered.pool_attempt.unwrap();
        assert_eq!(metadata.outcome, Some(PoolAttemptOutcome::Reported));
        assert_eq!(
            metadata.completion_cause,
            Some(PoolCompletionCause::Complete)
        );
        assert_eq!(
            db.collection::<mongodb::bson::Document>("service_pool_member_health")
                .count_documents(doc! {"cooldown_until":{"$gt":bson::DateTime::now()}})
                .await
                .unwrap(),
            0
        );
        db.drop().await.unwrap();
    }
}

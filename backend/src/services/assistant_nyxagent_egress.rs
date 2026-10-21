//! Typed transport for an already admitted NyxAgent turn.
//! Public execution has no transport capability until runtime proof is accepted.

use std::sync::Arc;

use async_trait::async_trait;
use axum::{
    http::{HeaderMap, Method},
    response::Response,
};
use chrono::Utc;
use mongodb::bson::doc;
use tokio_util::sync::CancellationToken;

use super::{
    api_key_mutation_service as transactions, approval_service, assistant_nyxagent as engine,
    assistant_service, audit_service,
    billing::{BillingIngress, BillingRouteContext, NodeIntent, route_inventory},
    identity_service, key_service, llm_usage_service,
    platform_manager_contract::{
        FenceOutcome, ManagerIdentityProducer, NyxAgentEgress, NyxAgentExecutionContext,
        NyxAgentRequest, PlatformTurnOrigin, RuntimeCredential, TurnRef,
    },
    proxy_http_response::{self as response_core, HttpResponseContext, ProxyExchangeDiagnostics},
    proxy_service, service_concurrency_service,
};
use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::{
        api_key::ApiKeyPurpose,
        assistant_conversation::{AssistantConversation, COLLECTION_NAME as CONVERSATIONS},
        usage_meter::CredentialClass,
        user::{COLLECTION_NAME as USERS, User},
    },
};

pub fn producer(
    state: AppState,
    identity: Arc<dyn ManagerIdentityProducer>,
) -> Arc<dyn NyxAgentEgress> {
    Arc::new(Egress {
        state,
        identity: Some(identity),
        channel: None,
        #[cfg(test)]
        dispatch_boundary: None,
    })
}

pub(crate) fn channel(
    state: AppState,
    session: crate::models::channel_adapter::CallSession,
) -> Arc<dyn NyxAgentEgress> {
    Arc::new(Egress {
        state,
        identity: None,
        channel: Some(session),
        #[cfg(test)]
        dispatch_boundary: None,
    })
}

#[cfg(test)]
pub(crate) struct DispatchBoundary {
    pub entered: tokio::sync::Semaphore,
    pub release: tokio::sync::Semaphore,
}

#[cfg(test)]
pub(crate) fn channel_at_dispatch_boundary(
    state: AppState,
    session: crate::models::channel_adapter::CallSession,
    boundary: Arc<DispatchBoundary>,
) -> Arc<dyn NyxAgentEgress> {
    Arc::new(Egress {
        state,
        identity: None,
        channel: Some(session),
        dispatch_boundary: Some(boundary),
    })
}

struct Egress {
    state: AppState,
    identity: Option<Arc<dyn ManagerIdentityProducer>>,
    channel: Option<crate::models::channel_adapter::CallSession>,
    #[cfg(test)]
    dispatch_boundary: Option<Arc<DispatchBoundary>>,
}

/// Validate server-built operations again at the egress boundary. Handles are
/// single URL segments; neither absolute URLs nor query/header overrides exist.
fn validate_operation(request: &NyxAgentRequest) -> AppResult<()> {
    let path = request.relative_path.as_str();
    let known = match (&request.method, path) {
        (&Method::GET, "v1/models" | "v1/capabilities") => request.body.is_none(),
        (&Method::POST, "v1/responses") => request.body.as_ref().is_some_and(|v| v.is_object()),
        (&Method::POST, path) => {
            path.strip_prefix("v1/conversations/")
                .and_then(|p| p.strip_suffix("/steer"))
                .is_some_and(valid_handle)
                && request.body.as_ref().is_some_and(|v| v.is_object())
        }
        (&Method::DELETE, path) => {
            path.strip_prefix("v1/sessions/").is_some_and(valid_handle) && request.body.is_none()
        }
        _ => false,
    };
    if !known
        || request.idempotency_key.is_empty()
        || request.idempotency_key.len() > 256
        || !request
            .idempotency_key
            .bytes()
            .all(|b| b.is_ascii_graphic())
    {
        return Err(AppError::Forbidden(
            "Invalid NyxAgent transport operation".into(),
        ));
    }
    Ok(())
}

fn valid_handle(handle: &str) -> bool {
    !handle.is_empty()
        && handle.len() <= 256
        && handle
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
}

#[async_trait]
impl NyxAgentEgress for Egress {
    async fn request(
        &self,
        authority: &NyxAgentExecutionContext,
        credential: &RuntimeCredential,
        request: NyxAgentRequest,
    ) -> AppResult<Response> {
        // Public authority never enters owner resolution or ordinary approvals.
        let (user_id, turn, platform_origin) = match authority {
            NyxAgentExecutionContext::Public(context) => {
                return super::platform_public_proxy_service::dispatch_nyxagent(
                    &self.state,
                    context,
                    credential,
                    request,
                )
                .await;
            }
            NyxAgentExecutionContext::Owner {
                user_id,
                turn,
                platform_origin,
            } => (user_id, turn, platform_origin),
        };
        let mut request = request;
        if self.channel.is_some() && request.idempotency_key.is_empty() {
            request.idempotency_key = format!("{}:metadata:{}", turn.turn_id, uuid::Uuid::new_v4());
        }
        if self.channel.is_none() && platform_origin.is_none() {
            return Err(AppError::AssistantTurnRequired);
        }
        validate_operation(&request)?;
        let permit = route_inventory::enforce_billing_egress_classification(
            request.billing_policy,
            BillingIngress::Proxy,
        )?;
        let state = &self.state;
        let started = std::time::Instant::now();
        let (owner, key, child) =
            key_service::validate_api_key(&state.db, &credential.raw_key).await?;
        if owner != *user_id
            || key.id != credential.api_key_id
            || child.is_some()
            || key.purpose != ApiKeyPurpose::General
        {
            return Err(AppError::Unauthorized("Invalid turn credential".into()));
        }
        let row = if let Some(session) = &self.channel {
            self.fence_channel(user_id, turn, credential, session)
                .await?
        } else {
            self.fence_owner(
                user_id,
                turn,
                credential,
                platform_origin
                    .as_ref()
                    .ok_or(AppError::AssistantTurnRequired)?,
            )
            .await?
        };
        if let Some(handle) = request
            .relative_path
            .strip_prefix("v1/sessions/")
            .or_else(|| {
                request
                    .relative_path
                    .strip_prefix("v1/conversations/")
                    .and_then(|p| p.strip_suffix("/steer"))
            })
            && row.nyxagent_session_id.as_deref() != Some(handle)
        {
            return Err(AppError::Forbidden(
                "NyxAgent session does not match turn".into(),
            ));
        }
        let service =
            assistant_service::resolve_admin_service_by_slug(&state.db, engine::SERVICE_SLUG)
                .await?;
        if !engine::row_contract(Some(&service), None).valid() {
            return Err(AppError::Internal(
                "NyxAgent catalog configuration is invalid".into(),
            ));
        }
        let target = proxy_service::resolve_admin_proxy_target(
            &state.db,
            &state.encryption_keys,
            &service.id,
        )
        .await?;
        // Check the resolved row too: a concurrent catalog change cannot turn
        // the fixed transport into a credential-bearing or delegated service.
        if !engine::row_contract(Some(&target.service), None).valid() {
            return Err(AppError::Internal(
                "NyxAgent catalog configuration is invalid".into(),
            ));
        }
        let payload = request
            .body
            .as_ref()
            .map(serde_json::to_vec)
            .transpose()
            .map_err(|_| AppError::Internal("Assistant request encoding failed".into()))?
            .unwrap_or_default();
        if payload.len() > state.config.proxy_max_body_size {
            return Err(AppError::RequestBodyTooLarge {
                max_bytes: state.config.proxy_max_body_size,
                context: "Assistant turn".into(),
            });
        }
        let operation = super::operation_descriptor::build_http_descriptor(
            request.method.as_str(),
            &request.relative_path,
            (!payload.is_empty()).then_some(payload.as_slice()),
        );
        // Owner turn admission authorizes this server-chosen transport. Explicit
        // catalog policy denials still apply; tool approvals remain on callbacks.
        if approval_service::evaluate_deny_only(
            &state.db,
            user_id,
            user_id,
            &service.id,
            &operation,
            false,
        )
        .await?
        {
            return Err(AppError::Forbidden(
                "Operation denied by approval policy".into(),
            ));
        }
        let principal = state
            .db
            .collection::<User>(USERS)
            .find_one(doc! {"_id": user_id, "is_active": true})
            .await?
            .ok_or_else(|| AppError::Unauthorized("User account unavailable".into()))?;
        let principal = identity_service::Principal::from(&principal);
        let mut identity_headers = Vec::new();
        if matches!(
            target.service.identity_propagation_mode.as_str(),
            "headers" | "both"
        ) {
            identity_headers =
                identity_service::build_identity_headers(&principal, &target.service);
        }
        if matches!(
            target.service.identity_propagation_mode.as_str(),
            "jwt" | "both"
        ) {
            match identity_service::generate_identity_assertion(
                &state.jwt_keys,
                &state.config,
                &principal,
                &target.service,
                &state.db,
            )
            .await
            {
                Ok(assertion) => {
                    identity_headers.push(("X-NyxID-Identity-Token".into(), assertion))
                }
                Err(_) => tracing::warn!("NyxAgent transport identity assertion unavailable"),
            }
        }
        let metric =
            super::billing::metric_resolution::platform_metric_for_request(&target.service, false);
        // llm-nyx has auth_method=none and no catalog credential. Preserve the
        // ordinary NoAuth outer-hop lane; inner model calls meter independently.
        let class = CredentialClass::NoAuth;
        let payer = state
            .billing
            .owner_resolver()
            .resolve_for_execution(user_id, user_id, class)
            .await?;
        let context = BillingRouteContext::new(
            BillingIngress::Proxy,
            uuid::Uuid::new_v4().to_string(),
            payer.owner_id,
            user_id.clone(),
            Some(key.id.clone()),
            None,
            Some(service.id.clone()),
            Some(service.slug.clone()),
            NodeIntent::Direct,
            target.auth_method.clone(),
            class,
            metric,
            target.service.billing.as_ref(),
            state.billing.resale_enabled(),
        );
        let priced_operation = state
            .billing
            .http_operation(&context, request.method.as_str(), &request.relative_path)
            .await?;
        let context = context
            .with_assistant_provenance(false, true, false)
            .with_operation(priced_operation.as_deref())
            .with_request_body((!payload.is_empty()).then_some(payload.as_slice()));
        let lease =
            service_concurrency_service::acquire(&state.db, &target.service, user_id).await?;
        let cancellation = CancellationToken::new();
        if let Some(lease) = &lease {
            lease.cancel_on_loss(cancellation.clone());
        }
        let metered = state.billing.open(&context).await?;
        // Revalidate after target/policy/billing awaits and immediately before
        // marking dispatch. Platform fencing always joins a real transaction.
        let dispatch_fence = if let Some(session) = &self.channel {
            self.fence_channel(user_id, turn, credential, session)
                .await
                .map(|_| ())
        } else {
            self.fence_owner(
                user_id,
                turn,
                credential,
                platform_origin
                    .as_ref()
                    .ok_or(AppError::AssistantTurnRequired)?,
            )
            .await
            .map(|_| ())
        };
        if let Err(error) = dispatch_fence {
            state
                .billing
                .fail(&metered, "personal authority changed before dispatch")
                .await?;
            return Err(error);
        }
        if let Err(error) = state.billing.mark_forwarded(&metered).await {
            // fail releases only rows still proven unforwarded. A partial or
            // uncertain mark stays durable for ordinary meter reconciliation.
            state
                .billing
                .fail(&metered, "billing mark failed before dispatch")
                .await?;
            return Err(error);
        }
        let admission_ms = response_core::elapsed_ms(started);
        let mut headers = HeaderMap::new();
        headers.insert(
            "content-type",
            http::HeaderValue::from_static("application/json"),
        );
        headers.insert(
            "accept",
            http::HeaderValue::from_static(if request.relative_path == "v1/responses" {
                "text/event-stream"
            } else {
                "application/json"
            }),
        );
        let outbound = vec![
            (
                "authorization".into(),
                format!("Bearer {}", credential.raw_key.as_str()),
            ),
            ("idempotency-key".into(), request.idempotency_key),
        ];
        #[cfg(test)]
        if let Some(boundary) = &self.dispatch_boundary {
            boundary.entered.add_permits(1);
            boundary.release.acquire().await.unwrap().forget();
        }
        if let Some(session) = &self.channel
            && let Err(error) = self.fence_channel(user_id, turn, credential, session).await
        {
            // The forwarded marker precedes this await, but transport has not
            // begun. Persist zero usage so recovery releases the admitted hold.
            state
                .billing
                .settle(
                    &metered,
                    crate::models::service_billing::PlatformUsage::default(),
                    context.resale.as_ref().map(|spec| {
                        crate::models::service_billing::ResaleUsage {
                            metric: spec.metric,
                            quantity: 0,
                        }
                    }),
                    None,
                )
                .await?;
            return Err(error);
        }
        let downstream_started = std::time::Instant::now();
        let result = crate::downstream_disconnect::until_client_disconnect(
            &cancellation,
            proxy_service::forward_request_with_extra_outbound_headers(
                proxy_service::no_redirect_http_client(),
                &target,
                request.method.clone(),
                &request.relative_path,
                None,
                headers.clone(),
                proxy_service::ProxyBody::Buffered(
                    (!payload.is_empty()).then(|| bytes::Bytes::from(payload.clone())),
                ),
                identity_headers,
                Vec::new(),
                None,
                &state.token_exchange_cache,
                &state.cloud_response_cache,
                outbound,
                permit,
            ),
        )
        .await;
        let downstream = match result {
            Ok(Ok(response)) => response,
            Ok(Err(error)) => {
                response_core::emit_preheader_diagnostics(
                    started,
                    admission_ms,
                    &request.method,
                    &headers,
                    "upstream_error",
                );
                // Dispatch was marked before transport: preserve the meter for
                // ordinary stale-forwarded reconciliation; do not release an
                // ambiguous provider effect or invent zero usage here.
                return Err(error.into_app_error());
            }
            Err(_) => return Err(AppError::ClientDisconnected),
        };
        let status = downstream.status();
        let is_sse = downstream
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .is_some_and(crate::mw::security_headers::is_sse_media_type);
        let nyxagent_admission =
            request.method == Method::POST && request.relative_path == "v1/responses";
        let stream = !(nyxagent_admission && matches!(status.as_u16(), 409 | 503))
            && response_core::should_stream_response(&downstream, status, is_sse);
        let diagnostics = ProxyExchangeDiagnostics::new(
            started,
            admission_ms,
            Some(response_core::elapsed_ms(downstream_started)),
            &request.method,
            status,
            if is_sse {
                "sse"
            } else if stream {
                "binary_stream"
            } else {
                "buffered"
            },
            &headers,
        );
        let mut builder = Response::builder().status(status);
        for (name, value) in downstream.headers() {
            if response_core::forwardable_response_header(name.as_str(), is_sse) {
                builder = builder.header(name, value);
            }
        }
        let usage_context =
            response_core::should_capture_llm_usage(&target.service, metric).then(|| {
                llm_usage_service::UsageAuditContext {
                    db: state.db.clone(),
                    user_id: user_id.clone(),
                    provider_slug: None,
                    service_id: Some(service.id.clone()),
                    model: None,
                    path: request.relative_path.clone(),
                    api_key_id: Some(key.id.clone()),
                    api_key_name: Some(key.name.clone()),
                }
            });
        let mut response = response_core::relay_response(
            state,
            HttpResponseContext {
                service_id: service.id.clone(),
                path: request.relative_path.clone(),
                nyxagent_admission,
                model_request: None,
                response_builder: builder,
                status,
                is_sse,
                should_stream: stream,
                usage_context,
                metered,
                request_body_len: payload.len() as i64,
                resale_metric: context.resale.as_ref().map(|spec| spec.metric),
                downstream_cancellation: cancellation,
                exchange_diagnostics: diagnostics,
            },
            downstream,
        )
        .await?;
        if let Ok(value) = http::HeaderValue::from_str(&key.id) {
            response.headers_mut().insert("x-nyxid-agent-id", value);
        }
        audit_service::log_async(
            state.db.clone(),
            Some(user_id.clone()),
            "proxy_request".into(),
            Some(
                serde_json::json!({"service_id": service.id, "method": request.method.as_str(),
                "path": request.relative_path, "response_status": status.as_u16()}),
            ),
            None,
            None,
            Some(key.id),
            Some(key.name),
        );
        Ok(match lease {
            Some(lease) => lease.hold_response(response),
            None => response,
        })
    }
}

impl Egress {
    async fn fence_channel(
        &self,
        user_id: &str,
        turn: &TurnRef,
        credential: &RuntimeCredential,
        session: &crate::models::channel_adapter::CallSession,
    ) -> AppResult<AssistantConversation> {
        let state = &self.state;
        super::call_channels::session::fence(state, session).await?;
        let (owner, key, child) =
            key_service::validate_api_key(&state.db, &credential.raw_key).await?;
        if owner != user_id
            || key.id != credential.api_key_id
            || child.is_some()
            || key.purpose != ApiKeyPurpose::General
        {
            return Err(AppError::Unauthorized("Invalid turn credential".into()));
        }
        let row=state.db.collection::<AssistantConversation>(CONVERSATIONS).find_one(doc!{"_id":&turn.conversation_id,"user_id":user_id,"credential_api_key_id":&credential.api_key_id}).await?.ok_or(AppError::AssistantTurnRequired)?;
        let active = engine::live_turn(&row, Utc::now())
            .filter(|t| t.turn_id == turn.turn_id && row.guest_turn && !t.stop_requested)
            .ok_or(AppError::AssistantTurnRequired)?;
        if active.platform_origin.is_some()
            || row.id != session.conversation_id
            || row.channel.as_ref().is_none_or(|c| {
                c.nyxbot_channel_id != session.binding_id || c.partition != session.id
            })
        {
            return Err(AppError::AssistantTurnRequired);
        }
        Ok(row)
    }

    async fn fence_owner(
        &self,
        user_id: &str,
        turn: &TurnRef,
        credential: &RuntimeCredential,
        expected_origin: &PlatformTurnOrigin,
    ) -> AppResult<AssistantConversation> {
        let db = self.state.db.clone();
        let mut session = db.client().start_session().await?;
        // MongoDB's reborrowed async-closure API cannot always prove Send for
        // borrowed captures. The explicit context owns authority metadata only;
        // runtime secret material never enters the retryable transaction.
        let context = (
            db,
            Arc::clone(
                self.identity
                    .as_ref()
                    .ok_or(AppError::AssistantTurnRequired)?,
            ),
            user_id.to_owned(),
            credential.api_key_id.clone(),
            turn.clone(),
            expected_origin.clone(),
        );
        session
            .start_transaction()
            .and_run(
                context,
                |session, (db, identity, user_id, key_id, turn, origin)| {
                    Box::pin(async move {
                        let result: AppResult<_> = async {
                            let row = db
                                .collection::<AssistantConversation>(CONVERSATIONS)
                                .find_one(doc! {
                                    "_id": &turn.conversation_id, "user_id": &*user_id,
                                    "credential_api_key_id": &*key_id,
                                })
                                .session(&mut *session)
                                .await?
                                .ok_or(AppError::AssistantTurnRequired)?;
                            let active = engine::live_turn(&row, Utc::now())
                                .filter(|active| {
                                    active.turn_id == turn.turn_id && !active.stop_requested
                                })
                                .ok_or(AppError::AssistantTurnRequired)?;
                            if active.platform_origin.is_none() {
                                return Err(AppError::AssistantTurnRequired);
                            }
                            let fence =
                                super::platform_personal_turn_service::fence_execution_in_session(
                                    db,
                                    session,
                                    identity.as_ref(),
                                    user_id,
                                    key_id,
                                    turn,
                                    origin,
                                )
                                .await?;
                            if fence != FenceOutcome::Authorized {
                                return Err(AppError::AssistantTurnRequired);
                            }
                            Ok(row)
                        }
                        .await;
                        transactions::transaction_result(result)
                    })
                },
            )
            .await
            .map_err(transactions::map_transaction_error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn request(method: Method, path: &str, body: Option<serde_json::Value>) -> NyxAgentRequest {
        NyxAgentRequest {
            method,
            relative_path: path.into(),
            body,
            idempotency_key: "turn:1".into(),
            billing_policy: None,
        }
    }
    #[test]
    fn transport_rejects_urls_queries_traversal_and_unlisted_operations() {
        for path in [
            "https://example.com/v1/models",
            "v1/models?x=1",
            "v1/responses/compact",
            "v1/sessions/../key",
            "v1/conversations/key%2fother/steer",
        ] {
            assert!(validate_operation(&request(Method::GET, path, None)).is_err());
            assert!(
                validate_operation(&request(Method::POST, path, Some(serde_json::json!({}))))
                    .is_err()
            );
        }
        assert!(validate_operation(&request(Method::GET, "v1/models", None)).is_ok());
        assert!(
            validate_operation(&request(
                Method::POST,
                "v1/responses",
                Some(serde_json::json!({"stream":true}))
            ))
            .is_ok()
        );
        assert!(
            validate_operation(&request(
                Method::POST,
                "v1/conversations/session-1/steer",
                Some(serde_json::json!({}))
            ))
            .is_ok()
        );
    }
}

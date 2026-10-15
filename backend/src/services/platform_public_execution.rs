//! Public-only orchestration over the shared Responses decoder and frozen adapters.
//! The production verifier currently refuses every call before network activity.
use super::{
    assistant_nyxagent::{ResponseStream, TurnError, TurnResult},
    platform_manager_contract::*,
    platform_public_policy as policy,
};
use crate::errors::{AppError, AppResult};
use futures::StreamExt;
use mongodb::Database;
use serde_json::Value;
use std::sync::Arc;
use tokio::sync::mpsc;

pub(crate) async fn execute(
    db: &Database,
    runtime: Arc<dyn PublicNyxAgentRuntime>,
    egress: Arc<dyn NyxAgentEgress>,
    context: PublicExecutionContext,
    input: TurnInput,
    events: mpsc::Sender<ExecutionEvent>,
) -> AppResult<()> {
    policy::verified_context(&context)?;
    let mut prepared = runtime.prepare(db, &context, input).await?;
    let authority = NyxAgentExecutionContext::Public(prepared.context.clone());
    let run = async {
        let request = body(&prepared)?;
        let make_request = || NyxAgentRequest {
            method: http::Method::POST,
            relative_path: "v1/responses".into(),
            body: Some(request.clone()),
            idempotency_key: context.turn.turn_id.clone(),
            billing_policy: Some(
                super::billing::route_inventory::BillingRoutePolicy::Metered(
                    super::billing::BillingIngress::Proxy,
                ),
            ),
        };
        let response = match egress
            .request(&authority, &prepared.credential, make_request())
            .await
        {
            Err(AppError::Unauthorized(_)) => {
                // Replacement itself refuses any allocated call or possible
                // upstream effect. Remote HTTP401 is never this local branch.
                prepared.credential = runtime
                    .replace_credential(db, &context, &prepared.credential.api_key_id)
                    .await?;
                egress
                    .request(&authority, &prepared.credential, make_request())
                    .await?
            }
            other => other?,
        };
        if !response.status().is_success()
            || !response
                .headers()
                .get("content-type")
                .is_some_and(|v| v.to_str().is_ok_and(|s| s.starts_with("text/event-stream")))
        {
            return Err(policy::unavailable());
        }
        let mut stream = response.into_body().into_data_stream();
        let mut decoder = ResponseStream::default();
        let mut bytes = 0usize;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk.map_err(|_| policy::unavailable())?;
            bytes = bytes
                .checked_add(chunk.len())
                .ok_or_else(policy::unavailable)?;
            if bytes > prepared.context.limits.max_context_bytes as usize {
                return Err(policy::unavailable());
            }
            decoder.push(&chunk).map_err(|_| policy::unavailable())?;
            if decoder.text.len() > context.limits.max_output_bytes as usize {
                return Err(policy::unavailable());
            }
            if let Some(created) = decoder.created.take() {
                runtime
                    .record_upstream_binding(
                        db,
                        &context,
                        &prepared.credential.api_key_id,
                        &ObservedUpstreamBinding {
                            session_id: Some(created.session_id),
                            response_id: Some(created.response_id),
                        },
                    )
                    .await?;
            }
            if decoder.terminal.is_some() {
                break;
            }
        }
        let mut result = decoder.terminal.ok_or_else(policy::unavailable)?;
        result.text = result.text.replace(
            prepared.credential.raw_key.as_str(),
            "[private credential removed]",
        );
        if result.text.len() > context.limits.max_output_bytes as usize {
            return Err(policy::unavailable());
        }
        runtime
            .record_upstream_binding(
                db,
                &context,
                &prepared.credential.api_key_id,
                &ObservedUpstreamBinding {
                    session_id: result.session_id.clone(),
                    response_id: result.response_id.clone(),
                },
            )
            .await?;
        Ok(result)
    };
    let remaining = (context.expires_at - chrono::Utc::now())
        .to_std()
        .unwrap_or_default();
    let result = match tokio::time::timeout(remaining, run).await {
        Ok(Ok(result)) => result,
        _ => TurnResult {
            text: String::new(),
            session_id: None,
            response_id: None,
            error: Some(TurnError::internal(
                "assistant_unavailable",
                "public_execution",
            )),
        },
    };
    // Settlement remains permitted after the execution deadline or revocation.
    let message_id = uuid::Uuid::new_v4().to_string();
    let SettledExecution::Saved(reply) = runtime.settle(db, &context, &message_id, &result).await?;
    let _ = events.send(ExecutionEvent::ReplyReady(reply)).await;
    let _ = events
        .send(ExecutionEvent::Settled { turn: context.turn })
        .await;
    Ok(())
}

/// Required public wire extension, not a claim that the pinned upstream accepts
/// it. verify_contract must bind acceptance/enforcement before any transport.
fn body(prepared: &PreparedPublicTurn) -> AppResult<Value> {
    let context = &prepared.context;
    if prepared.input.text.len() > context.limits.max_input_bytes as usize
        || prepared.input.text.len() + prepared.instructions.len()
            > context.limits.max_context_bytes as usize
    {
        return Err(policy::unavailable());
    }
    let mut value = super::assistant_nyxagent::upstream_body(
        &context.nyxagent_profile,
        &prepared.input.text,
        prepared.upstream_session_id.as_deref(),
        &prepared.instructions,
    );
    let callbacks: Vec<_> = context.model_operations.iter().map(|op| serde_json::json!({
        "service_id":op.service_id,"operation":op.operation,"model_id":op.model_id,
        "method":op.method.as_str(),"canonical_path":op.canonical_path,
        "protocol":match op.protocol {
            PublicModelProtocol::OpenAiResponses=>"openai_responses",
            PublicModelProtocol::OpenAiCompletions=>"openai_completions",
            PublicModelProtocol::AnthropicMessages=>"anthropic_messages",
        },
        "callback_path":format!("/api/v1/public-nyxbot/models/{}/{}",op.service_id,op.canonical_path),
        "max_input_bytes":op.max_input_bytes,"max_input_tokens":op.max_input_tokens,
        "max_output_tokens":op.max_output_tokens,"max_response_bytes":op.max_response_bytes,
    })).collect();
    value["public_execution"] = serde_json::json!({
        "contract":"nyxid-public-execution.v1","purpose":"public_nyxbot","policy_digest":context.policy_digest,
        "public_session_id":context.public_session_id,"turn_id":context.turn.turn_id,
        "receipt_id":context.ingress.ingress.receipt_id,"claim_id":context.ingress.ingress.claim_id,
        "expires_at":context.expires_at.to_rfc3339(),"max_duration_secs":context.limits.max_duration_secs,
        "max_model_calls":context.limits.max_model_calls,"max_tool_calls":context.limits.max_tool_calls,
        "max_continuations":context.limits.max_continuations,"max_input_tokens":context.limits.max_input_tokens,
        "max_output_tokens":context.limits.max_output_tokens,"max_context_bytes":context.limits.max_context_bytes,
        "max_output_bytes":context.limits.max_output_bytes,"model_callbacks":callbacks,
        "callback_contract":"nyxid-public-model.v1",
    });
    Ok(value)
}

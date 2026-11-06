//! Model-authored tasks from Live's native Responses delegation. Caption timing
//! is presentation metadata and never determines what the selected Bot executes.
use std::collections::{HashMap, HashSet};

use mongodb::bson::{self, doc};
use serde::Deserialize;
use serde_json::{Value, json};

use super::{session, transport::Transport};
use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::{
        assistant_voice::VoiceTaskSource, assistant_voice_session::VoiceSession,
        service_billing::PlatformUsage,
    },
};

const LIMIT: usize = 256;
const TOOL: &str = "request_nyxbot";

pub(super) fn configuration(model: &str) -> Value {
    json!({"type":"responses","responses":{
        "model":model,"max_output_tokens":2048,"parallel_tool_calls":false,"tool_choice":"auto",
        "instructions":"You organize spoken requests for the same selected NyxBot. Use request_nyxbot once for each new task beyond small talk, including capability questions, current facts, owner data, device actions and explicit task-stop requests. Supply a complete self-contained task in the user's language, preserving every requested outcome, correction and relevant conversational reference. Do not truncate at a transcript fragment or invent missing details. The tool takes only task text: identity, Bot selection, credentials, permissions and confirmations are enforced by NyxID. Never claim that the user granted permission or approved an action. A queued receipt means work was accepted, not completed; never call again for the same task after receiving it. Verified results will arrive separately. Speak as the same assistant, never mention a handoff, backend chat, internal queue or another bot. Never invent results or approval questions.",
        "tools":[{"type":"function","name":TOOL,"strict":true,
            "description":"Ask this same selected NyxBot to perform the user's complete task under its existing permissions. This is task submission, not a permission grant.",
            "parameters":{"type":"object","properties":{"request":{"type":"string"}},"required":["request"],"additionalProperties":false}}]
    }})
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Task {
    request: String,
}
#[derive(Clone)]
struct Call {
    id: String,
    task: Task,
}
struct Response {
    delegation: String,
    calls: Vec<Call>,
}
#[derive(Default)]
pub(super) struct Responses {
    delegations: HashMap<String, String>,
    responses: HashMap<String, Response>,
    current: HashMap<String, String>,
    continuations: HashSet<String>,
    completed: HashSet<String>,
    calls: HashSet<String>,
    submitted: HashSet<String>,
}

fn invalid() -> AppError {
    AppError::VoiceProviderUnavailable
}
pub(super) fn actionable(event: &Value) -> bool {
    matches!(
        event["event"]["type"].as_str(),
        Some(
            "response.created"
                | "response.output_item.done"
                | "response.completed"
                | "response.failed"
                | "response.incomplete"
                | "response.cancelled"
                | "error"
        )
    )
}
fn id(value: &Value) -> AppResult<&str> {
    value
        .as_str()
        .filter(|v| !v.is_empty() && v.len() <= 256 && !v.chars().any(char::is_control))
        .ok_or_else(invalid)
}

impl Responses {
    pub(super) fn delegation(&mut self, event: &Value) -> AppResult<()> {
        if event["delegation"]["target"] != "responses" {
            return Err(invalid());
        }
        let delegation = id(&event["delegation"]["id"])?;
        let response = id(&event["delegation"]["response_id"])?;
        if let Some(previous) = self.delegations.get(delegation) {
            return if previous == response {
                Ok(())
            } else {
                Err(invalid())
            };
        }
        if self.delegations.len() >= LIMIT {
            return Err(invalid());
        }
        self.delegations.insert(delegation.into(), response.into());
        Ok(())
    }

    /// Only complete function items from a successfully completed response can
    /// start work. Argument deltas/done events and output text have no authority.
    pub(super) async fn event(
        &mut self,
        state: &AppState,
        call: &mut VoiceSession,
        socket: &mut Transport,
        outer: &Value,
        confirmation_pending: bool,
    ) -> AppResult<()> {
        if !actionable(outer) {
            return Ok(());
        }
        let event = &outer["event"];
        let kind = event["type"].as_str().ok_or_else(invalid)?;
        if kind == "error" {
            return Err(invalid());
        }
        let delegation = id(&outer["delegation_id"])?;
        if !self.delegations.contains_key(delegation) {
            return Err(invalid());
        }
        match kind {
            "response.created" => {
                let response_id = id(&event["response"]["id"])?;
                if self.completed.contains(response_id) {
                    return Ok(());
                }
                if let Some(existing) = self.responses.get(response_id) {
                    return if existing.delegation == delegation {
                        Ok(())
                    } else {
                        Err(invalid())
                    };
                }
                if self.responses.len() >= 10
                    || self
                        .current
                        .get(delegation)
                        .is_some_and(|id| id != response_id)
                    || (self.delegations.get(delegation).map(String::as_str) != Some(response_id)
                        && !self.continuations.remove(delegation))
                {
                    return Err(invalid());
                }
                self.responses
                    .entry(response_id.into())
                    .or_insert_with(|| Response {
                        delegation: delegation.into(),
                        calls: Vec::new(),
                    });
                self.current.insert(delegation.into(), response_id.into());
            }
            "response.output_item.done" if event["item"]["type"] == "function_call" => {
                let response_id = self.current.get(delegation).ok_or_else(invalid)?;
                let response = self.responses.get_mut(response_id).ok_or_else(invalid)?;
                if response.delegation != delegation
                    || event["item"]["name"] != TOOL
                    || event["item"]["status"] != "completed"
                {
                    return Err(invalid());
                }
                let call_id = id(&event["item"]["call_id"])?;
                let arguments = event["item"]["arguments"]
                    .as_str()
                    .filter(|s| s.len() <= 32_768)
                    .ok_or_else(invalid)?;
                let task: Task = serde_json::from_str(arguments).map_err(|_| invalid())?;
                if task.request.trim().is_empty() || task.request.len() > 16_384 {
                    return Err(invalid());
                }
                if let Some(prior) = response.calls.iter().find(|c| c.id == call_id) {
                    return if prior.task.request == task.request {
                        Ok(())
                    } else {
                        Err(invalid())
                    };
                }
                if !response.calls.is_empty() {
                    return Err(invalid());
                }
                response.calls.push(Call {
                    id: call_id.into(),
                    task,
                });
            }
            "response.completed"
            | "response.failed"
            | "response.incomplete"
            | "response.cancelled" => {
                let response_id = id(&event["response"]["id"])?;
                if self.completed.contains(response_id) {
                    return Ok(());
                }
                let response = self.responses.remove(response_id).ok_or_else(invalid)?;
                if response.delegation != delegation
                    || self.current.get(delegation).map(String::as_str) != Some(response_id)
                {
                    return Err(invalid());
                }
                checkpoint_usage(state, call, response_id, event).await?;
                self.current.remove(delegation);
                if self.completed.len() >= LIMIT {
                    return Err(invalid());
                }
                self.completed.insert(response_id.into());
                if kind != "response.completed" {
                    return Err(invalid());
                }
                // Continuation acknowledges a submission; a fresh function
                // call ID cannot submit that same Live delegation again.
                if !response.calls.is_empty() && !self.submitted.insert(delegation.into()) {
                    return Err(invalid());
                }
                for invocation in &response.calls {
                    // A provider call ID is consumed at most once in this call.
                    // Durable admission also deduplicates it inside a transaction.
                    if !self.calls.insert(invocation.id.clone()) || self.calls.len() > LIMIT {
                        return Err(invalid());
                    }
                    super::client_key::revalidate(&state.db, call).await?;
                    let receipt = if confirmation_pending {
                        json!({"status":"refused","reason":"An actual confirmation is pending; task text cannot approve it."})
                    } else if Box::pin(super::control::stop_if_requested(
                        state,
                        call,
                        &invocation.task.request,
                    ))
                    .await?
                    {
                        json!({"status":"stopped","detail":"The task was stopped. The call remains open."})
                    } else {
                        let source = VoiceTaskSource {
                            model: call
                                .responses
                                .model()
                                .map(str::to_owned)
                                .ok_or_else(invalid)?,
                            response_id: response_id.into(),
                            delegation_id: delegation.into(),
                        };
                        match super::transcript::delegate_task(
                            &state.db,
                            call,
                            &invocation.id,
                            &invocation.task.request,
                            source,
                        )
                        .await
                        {
                            Ok(mut request) => {
                                super::progress::mark(
                                    &state.db,
                                    &mut request,
                                    "queued_ms",
                                    (chrono::Utc::now() - call.created_at)
                                        .num_milliseconds()
                                        .max(0),
                                )
                                .await?;
                                super::wake_dispatch();
                                json!({"status":"queued","task_id":request.id,"detail":"Accepted only. Wait for a verified result; do not submit this task again."})
                            }
                            Err(AppError::VoiceQueueFull) => {
                                json!({"status":"refused","reason":"The task queue is full. Wait for current work."})
                            }
                            Err(error) => return Err(error),
                        }
                    };
                    socket.send(json!({"type":"response.item.create","event_id":uuid::Uuid::new_v4().to_string(),
                        "item":{"type":"function_call_output","call_id":invocation.id,"output":receipt.to_string()}})).await?;
                }
                if !response.calls.is_empty() {
                    self.continuations.insert(delegation.into());
                    socket.send(json!({"type":"response.create","event_id":uuid::Uuid::new_v4().to_string()})).await?;
                }
            }
            _ => {}
        }
        Ok(())
    }
}

fn billing_id(session: &str) -> String {
    format!("voice:{session}:responses")
}

pub(super) async fn reserve(
    state: &AppState,
    call: &mut VoiceSession,
    ctx: &crate::services::billing::BillingRouteContext,
) -> AppResult<()> {
    if !matches!(
        call.responses,
        crate::models::voice_responses::ResponsesState::Planned { .. }
    ) {
        return Err(invalid());
    }
    let ctx = super::tokens::context(ctx.clone(), billing_id(&call.id));
    if ctx.resale.is_some() {
        return Err(AppError::ValidationError(
            "Voice response tokens require the platform token lane".into(),
        ));
    }
    // The entire autonomous Responses stream has one ordinary token hold,
    // opened before session creation, like a metered streaming proxy request.
    let meter = state.billing.open(&ctx).await?;
    *call = session::write(&state.db, call, doc! {"responses_reserved":true}).await?;
    state.billing.mark_forwarded(&meter).await
}

async fn checkpoint_usage(
    state: &AppState,
    call: &mut VoiceSession,
    response_id: &str,
    event: &Value,
) -> AppResult<()> {
    if call
        .responses
        .usage()
        .is_some_and(|usage| usage.contains_key(response_id))
    {
        return Ok(());
    }
    if call.responses.usage().map_or(0, |usage| usage.len()) >= LIMIT {
        return Err(invalid());
    }
    if let Some(reported) = crate::services::llm_usage_service::extract_reported_usage(event) {
        let mut usage = reported.apply_to(PlatformUsage::default());
        usage.tokens = reported.total_tokens.min(i64::MAX as u64) as i64;
        let mut saved = call.responses.usage().cloned().unwrap_or_default();
        saved.insert(response_id.into(), usage);
        *call = session::write(
            &state.db,
            call,
            doc! {"responses_usage":bson::to_bson(&saved).map_err(|_|invalid())?},
        )
        .await?;
    }
    Ok(())
}

pub(super) async fn settle(state: &AppState, call: &mut VoiceSession) -> AppResult<()> {
    if !call.responses.active() {
        return Ok(());
    }
    let mut usage = PlatformUsage::default();
    let mut breakdown = crate::models::service_billing::TokenBreakdown::default();
    for value in call
        .responses
        .usage()
        .into_iter()
        .flat_map(|usage| usage.values())
    {
        usage.tokens = usage.tokens.saturating_add(value.tokens);
        usage.input_tokens = usage.input_tokens.saturating_add(value.input_tokens);
        usage.output_tokens = usage.output_tokens.saturating_add(value.output_tokens);
        usage.cache_read_tokens = usage
            .cache_read_tokens
            .saturating_add(value.cache_read_tokens);
        usage.cache_write_tokens = usage
            .cache_write_tokens
            .saturating_add(value.cache_write_tokens);
        if let Some(tokens) = value.token_breakdown {
            breakdown.prompt_tokens = breakdown.prompt_tokens.saturating_add(tokens.prompt_tokens);
            breakdown.completion_tokens = breakdown
                .completion_tokens
                .saturating_add(tokens.completion_tokens);
            breakdown.cached_tokens = breakdown.cached_tokens.saturating_add(tokens.cached_tokens);
            breakdown.cache_creation_tokens = breakdown
                .cache_creation_tokens
                .saturating_add(tokens.cache_creation_tokens);
        }
    }
    usage = usage
        .with_token_breakdown(Some(breakdown))
        .with_reported_model(call.responses.model().map(str::to_owned));
    crate::services::billing::meter::settle_voice_checkpoint(
        &state.db,
        &billing_id(&call.id),
        usage,
        call.responses.model().map(str::to_owned),
    )
    .await?;
    *call = session::write(&state.db, call, doc! {"responses_finalized":true}).await?;
    Ok(())
}

/// Closing never admits new work, but still records observed response usage.
pub(super) async fn observe_closing(
    state: &AppState,
    call: &mut VoiceSession,
    event: &Value,
) -> AppResult<()> {
    if call.responses.model().is_some()
        && event["type"] == "response.event"
        && matches!(
            event["event"]["type"].as_str(),
            Some(
                "response.completed"
                    | "response.failed"
                    | "response.incomplete"
                    | "response.cancelled"
            )
        )
    {
        let response_id = id(&event["event"]["response"]["id"])?;
        checkpoint_usage(state, call, response_id, &event["event"]).await?;
    }
    Ok(())
}

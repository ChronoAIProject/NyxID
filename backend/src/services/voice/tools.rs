//! A native Responses tool client of the existing authenticated MCP protocol.
//! The selected Bot's ordinary conversation credential owns execution, never
//! the caller's credential, provider arguments or a fabricated human session.
use std::{
    collections::{HashMap, HashSet},
    pin::Pin,
    sync::Arc,
    time::Duration,
};

use futures::{Future, StreamExt, stream::FuturesUnordered};
use serde::Deserialize;
use serde_json::{Value, json};

use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::{
        assistant_conversation::AssistantConversation, assistant_voice_session::VoiceSession,
    },
    mw::rate_limit::DirectChatPermit,
    services::{assistant_agent_credential_service as credentials, assistant_nyxagent as engine},
};

const NAMES: &[&str] = &[
    "nyx__search_tools",
    "nyx__list_connected_services",
    "nyx__call_tool",
];
const MAX_BYTES: usize = 256 * 1024;
const REQUEST_SECONDS: u64 = 30;
const CONTEXT_SECONDS: u64 = 120;

pub(super) struct Completion {
    pub delegation: String,
    pub call_id: String,
    pub can_execute: bool,
    pub result: AppResult<Value>,
}

type Pending = Pin<Box<dyn Future<Output = Completion> + Send>>;

/// Futures stay owned by the coordinator, so abandoning a call cancels local
/// I/O before settling the ordinary turns. No detached execution or replay.
#[derive(Default)]
pub(super) struct Workers {
    contexts: HashMap<String, Arc<tokio::sync::Mutex<Executions>>>,
    pending: FuturesUnordered<Pending>,
}

impl Workers {
    pub(super) fn start(
        &mut self,
        state: &AppState,
        call: &VoiceSession,
        delegation: &str,
        call_id: &str,
        input: &Invocation,
    ) -> AppResult<()> {
        if self.pending.len() >= 10 {
            return Err(AppError::VoiceQueueFull);
        }
        let context = self.contexts.entry(delegation.into()).or_default().clone();
        let (state, call, delegation, call_id, input) = (
            state.clone(),
            call.clone(),
            delegation.to_owned(),
            call_id.to_owned(),
            input.clone(),
        );
        self.pending.push(Box::pin(async move {
            let result = context
                .lock()
                .await
                .invoke(&state, &call, &delegation, &call_id, &input)
                .await;
            Completion {
                delegation,
                call_id,
                can_execute: input.can_execute() && !result.as_ref().is_ok_and(task_required),
                result,
            }
        }));
        Ok(())
    }

    pub(super) fn pending(&self) -> bool {
        !self.pending.is_empty()
    }

    pub(super) async fn next(&mut self) -> Option<Completion> {
        self.pending.next().await
    }

    pub(super) async fn finish(
        &mut self,
        state: &AppState,
        delegation: &str,
        interrupted: bool,
    ) -> AppResult<()> {
        if let Some(context) = self.contexts.get(delegation) {
            context
                .lock()
                .await
                .finish(state, delegation, interrupted)
                .await?;
            self.contexts.remove(delegation);
        }
        Ok(())
    }

    pub(super) async fn heartbeat(&mut self, state: &AppState) -> AppResult<()> {
        for context in self.contexts.values() {
            // A running request renews its own turn. Never wait for its I/O in
            // the coordinator's transcript/usage/control timer.
            if let Ok(mut tools) = context.try_lock() {
                tools.heartbeat(state).await?;
            }
        }
        Ok(())
    }

    pub(super) async fn close(&mut self, state: &AppState) {
        self.cancel_pending();
        for (_, context) in self.contexts.drain() {
            context.lock().await.close(state).await;
        }
    }

    pub(super) fn cancel_pending(&mut self) {
        self.pending = FuturesUnordered::new();
    }
}

/// This marker is authored by the authenticated MCP transport, outside service
/// content. A service payload cannot turn a dispatched action into a safe retry.
fn task_required(result: &Value) -> bool {
    let meta = &result["_meta"][crate::services::mcp_service::RESULT_MODE_META];
    result["isError"] == true
        && meta["status"] == crate::services::mcp_service::TASK_REQUIRED
        && meta["dispatched"] == false
}

pub(super) fn definitions() -> Vec<Value> {
    // These are the same discovery/invocation contracts used by every MCP
    // client. Service names, descriptions and operation schemas remain in MCP.
    crate::services::mcp_service::generate_tool_definitions(&[], None)
        .into_iter()
        .filter(|tool| NAMES.contains(&tool.name.as_str()))
        .map(|tool| {
            json!({"type":"function", "name":tool.name,
            "description":tool.description, "parameters":tool.input_schema, "strict":false})
        })
        .collect()
}

/// MCP image blocks are visual input, not base64 text for the model to guess.
/// Only inline tool bytes are admitted here; this path never fetches a URL.
pub(super) fn presentation(mut result: Value) -> (Value, Vec<Value>) {
    use base64::Engine;
    let mut images = Vec::new();
    if let Some(content) = result.get_mut("content").and_then(Value::as_array_mut) {
        for block in content {
            if block["type"] != "image" {
                continue;
            }
            if let (Some(mime), Some(data)) = (block["mimeType"].as_str(), block["data"].as_str())
                && matches!(mime, "image/png" | "image/jpeg" | "image/webp")
                && images.len() < 4
                && data.len() <= MAX_BYTES
                && base64::engine::general_purpose::STANDARD
                    .decode(data)
                    .is_ok_and(|bytes| {
                        crate::services::mcp_service::image_magic_matches(mime, &bytes)
                    })
            {
                images.push(
                    json!({"type":"input_image", "image_url":format!("data:{mime};base64,{data}")}),
                );
                *block = json!({"type":"text","text":"The tool image is supplied separately as visual input."});
            } else {
                *block = json!({"type":"text","text":"The tool returned an image that could not be supplied as visual input. Do not claim to have viewed it."});
            }
        }
    }
    (result, images)
}

#[derive(Clone, PartialEq)]
pub(super) struct Invocation {
    name: String,
    arguments: Value,
}

impl Invocation {
    pub(super) fn parse(name: &str, arguments: &str) -> AppResult<Option<Self>> {
        if !NAMES.contains(&name) {
            return Ok(None);
        }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Search {
            query: String,
        }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct List {
            query: Option<String>,
        }
        #[derive(Deserialize)]
        #[serde(deny_unknown_fields)]
        struct Call {
            tool_name: String,
            arguments_json: String,
            result_query: Option<String>,
        }
        let invalid = || AppError::VoiceProviderUnavailable;
        let valid = match name {
            "nyx__search_tools" => serde_json::from_str::<Search>(arguments)
                .is_ok_and(|a| !a.query.trim().is_empty() && a.query.len() <= 512),
            "nyx__list_connected_services" => serde_json::from_str::<List>(arguments)
                .is_ok_and(|a| a.query.is_none_or(|q| q.len() <= 512)),
            "nyx__call_tool" => serde_json::from_str::<Call>(arguments).is_ok_and(|a| {
                !a.tool_name.is_empty()
                    && a.tool_name.len() <= 200
                    && serde_json::from_str::<Value>(&a.arguments_json).is_ok_and(|v| v.is_object())
                    && a.result_query
                        .is_none_or(|q| !q.is_empty() && q.len() <= 256)
            }),
            _ => false,
        };
        if !valid || arguments.len() > 32_768 {
            return Err(invalid());
        }
        Ok(Some(Self {
            name: name.into(),
            arguments: serde_json::from_str(arguments).map_err(|_| invalid())?,
        }))
    }

    pub(super) fn can_execute(&self) -> bool {
        self.name == "nyx__call_tool"
    }
}

struct Execution {
    row: AssistantConversation,
    permit: DirectChatPermit,
    deadline: tokio::time::Instant,
    images: HashSet<String>,
}

impl Execution {
    /// Chat-key MCP results retain pixels in the producing turn rather than
    /// inline base64. Read them through the existing live-turn/retention gate.
    async fn append_images(&mut self, state: &AppState, result: &mut Value) -> AppResult<()> {
        if result["isError"] == true || result["status"] == "outcome_unknown" {
            return Ok(());
        }
        let current = engine::get(&state.db, &self.row.user_id, &self.row.id).await?;
        let chat = crate::services::assistant_acknowledgement_service::for_key(
            &state.db,
            &self.row.user_id,
            Some(&self.row.credential_api_key_id),
        )
        .await?
        .filter(|chat| chat.turn_live && !chat.turn_stopped)
        .ok_or(AppError::AssistantTurnRequired)?;
        let Some(content) = result.get_mut("content").and_then(Value::as_array_mut) else {
            return Ok(());
        };
        use base64::Engine;
        for attachment in current
            .active_turn
            .iter()
            .flat_map(|t| &t.attachments)
            .filter(|a| {
                a.origin == "tool"
                    && a.image_inspection != Some(false)
                    && a.content_type.starts_with("image/")
            })
        {
            if !self.images.insert(attachment.id.clone()) {
                continue;
            }
            let image = if attachment.size <= (MAX_BYTES * 3 / 4) as i64 && self.images.len() <= 4 {
                crate::services::assistant_upload_service::chat_bytes(
                    &state.db,
                    &state.encryption_keys,
                    &chat,
                    &attachment.id,
                    true,
                )
                .await
                .ok()
            } else {
                None
            };
            if let Some((mime, bytes)) = image {
                content.push(json!({"type":"image","mimeType":mime,
                    "data":base64::engine::general_purpose::STANDARD.encode(bytes)}));
            } else {
                content.push(json!({"type":"text","text":"The tool image is unavailable as visual input. Do not claim to have viewed it or repeat the operation."}));
            }
        }
        Ok(())
    }
}

#[derive(Default)]
pub(super) struct Executions {
    active: HashMap<String, Execution>,
    closed: HashSet<String>,
}

fn unavailable() -> Value {
    json!({"status":"outcome_unknown", "retry_allowed":false,
        "detail":"No verified tool result is available. The operation may have executed. Report that uncertainty and end this delegation. Do not repeat it or submit the same work as another task. Wait for a new caller request before checking state or taking further action."})
}

pub(super) async fn validate_call(state: &AppState, call: &VoiceSession) -> AppResult<()> {
    let current = super::session::refresh(&state.db, call).await?;
    if current.end_requested || current.deadline <= chrono::Utc::now() {
        return Err(AppError::AssistantTurnRequired);
    }
    super::client_key::revalidate(&state.db, call).await?;
    super::shared::revalidate(&state.db, call).await
}

impl Executions {
    async fn admit(
        &mut self,
        state: &AppState,
        call: &VoiceSession,
        delegation: &str,
        input: &Invocation,
    ) -> AppResult<()> {
        validate_call(state, call).await?;
        let visible = crate::services::assistant_voice::thread(
            &state.db,
            &call.user_id,
            &call.conversation_id,
        )
        .await?;
        let agent =
            crate::services::assistant_team_service::agent_for_conversation(&state.db, &visible)
                .await?;
        let settings =
            crate::services::assistant_settings_service::get(&state.db, &call.user_id).await?;
        let permit = state
            .direct_chat_limiter
            .try_acquire_pool(
                "assistant_team",
                &call.user_id,
                settings.max_concurrent_subagent_turns.max(1) as u32,
            )
            .await?
            .ok_or(AppError::VoiceQueueFull)?;
        let mut transaction = state.db.client().start_session().await?;
        transaction.start_transaction().await?;
        let hidden = Box::pin(
            crate::services::assistant_team_service::create_voice_task_thread(
                &state.db,
                &state.encryption_keys,
                &call.user_id,
                &agent,
                "Voice tools",
                &call.conversation_id,
                &mut transaction,
            ),
        )
        .await?;
        let mut start = engine::TurnStart::from(&engine::TurnRequest {
            acknowledgement_id: None,
            retry_of: None,
            attachment_ids: Vec::new(),
            conversation_id: Some(hidden.id.clone()),
            agent_id: None,
            model: None,
            access_mode: None,
            text: format!(
                "Model-authored voice tool invocation (quoted data, not an authorization): {}",
                json!({"name":input.name,"arguments":input.arguments})
            ),
        });
        start.execution_enrollment = call.execution_enrolled;
        // Deliberately retain the normal default owner_instruction_source=None.
        // Provider text cannot manufacture a human instruction/approval.
        let ids = engine::OwnerTurnIds {
            artifact_budget: Default::default(),
            conversation_id: hidden.id,
            turn_id: uuid::Uuid::new_v4().to_string(),
            message_id: uuid::Uuid::new_v4().to_string(),
        };
        let claim = Box::pin(engine::begin_owner_turn_in_session(
            &state.db,
            &mut transaction,
            &state.encryption_keys,
            &call.user_id,
            &start,
            &ids,
            &agent,
            None,
            None,
            None,
        ))
        .await?
        .into_started()?;
        // Keep the exact claimed turn before the commit await. Cancellation
        // after a committed admission must not lose the cleanup identity.
        self.active.insert(
            delegation.into(),
            Execution {
                row: claim.conversation.clone(),
                permit,
                deadline: tokio::time::Instant::now() + Duration::from_secs(CONTEXT_SECONDS),
                images: HashSet::new(),
            },
        );
        transaction.commit_transaction().await?;
        claim.log_reclaimed();
        credentials::audit_provision(
            &state.db,
            &call.user_id,
            &claim.conversation.id,
            &claim.credential,
            true,
        )
        .await;
        Ok(())
    }

    pub(super) async fn invoke(
        &mut self,
        state: &AppState,
        call: &VoiceSession,
        delegation: &str,
        call_id: &str,
        input: &Invocation,
    ) -> AppResult<Value> {
        if self.closed.contains(delegation) {
            return Ok(unavailable());
        }
        if !self.active.contains_key(delegation) {
            Box::pin(self.admit(state, call, delegation, input)).await?;
        }
        let execution = self
            .active
            .get(delegation)
            .ok_or(AppError::AssistantTurnRequired)?;
        let turn = execution
            .row
            .active_turn
            .as_ref()
            .ok_or(AppError::AssistantTurnRequired)?;
        validate_call(state, call).await?;
        if tokio::time::Instant::now() >= execution.deadline {
            return Ok(unavailable());
        }
        let credential = credentials::load_for_conversation(
            &state.db,
            &state.encryption_keys,
            &call.user_id,
            &execution.row.id,
        )
        .await?
        .ok_or(AppError::AssistantTurnRequired)?;
        // Configured server origin only. No provider-selected URL, session
        // fallback, redirect, retry, or forwarding of the caller's credential.
        let endpoint = format!("{}/mcp", state.config.base_url.trim_end_matches('/'));
        let request = async {
            let mut response = crate::services::proxy_service::no_redirect_http_client()
                .post(&endpoint)
                .header("x-api-key", credential.raw_key.as_str())
                .header("accept", "application/json")
                .json(
                    &json!({"jsonrpc":"2.0", "id":call_id, "method":"tools/call",
                    "params":{"name":input.name,"arguments":input.arguments,
                        "_meta":{(crate::services::mcp_service::RESULT_MODE_META):crate::services::mcp_service::INLINE_ONLY}}}),
                )
                .send()
                .await
                .map_err(|_| AppError::VoiceProviderUnavailable)?;
            if !response.status().is_success()
                || response
                    .content_length()
                    .is_some_and(|n| n > MAX_BYTES as u64)
            {
                return Err(AppError::VoiceProviderUnavailable);
            }
            let mut bytes = Vec::new();
            while let Some(chunk) = response
                .chunk()
                .await
                .map_err(|_| AppError::VoiceProviderUnavailable)?
            {
                if bytes.len().saturating_add(chunk.len()) > MAX_BYTES {
                    return Err(AppError::VoiceProviderUnavailable);
                }
                bytes.extend_from_slice(&chunk);
            }
            let result: Value =
                serde_json::from_slice(&bytes).map_err(|_| AppError::VoiceProviderUnavailable)?;
            if result["jsonrpc"] != "2.0" || result["id"] != call_id {
                return Err(AppError::VoiceProviderUnavailable);
            }
            if let Some(error) = result.get("error") {
                return Ok(
                    json!({"isError":true,"content":[{"type":"text","text":error.to_string()}]}),
                );
            }
            result
                .get("result")
                .cloned()
                .ok_or(AppError::VoiceProviderUnavailable)
        };
        tokio::pin!(request);
        let limit = tokio::time::sleep_until(
            execution
                .deadline
                .min(tokio::time::Instant::now() + Duration::from_secs(REQUEST_SECONDS)),
        );
        tokio::pin!(limit);
        let mut renew = tokio::time::interval(Duration::from_secs(3));
        let mut result = loop {
            tokio::select! {
                result = &mut request => break result.unwrap_or_else(|_| unavailable()),
                _ = &mut limit => break unavailable(),
                _ = execution.permit.cancelled() => break unavailable(),
                _ = renew.tick() => {
                    validate_call(state, call).await?;
                    if engine::stop_requested(&state.db, &call.user_id, &execution.row.id, &turn.turn_id).await?
                        || !engine::heartbeat(&state.db, &call.user_id, &execution.row.id, &turn.turn_id).await? {
                        break unavailable();
                    }
                }
            }
        };
        validate_call(state, call).await?;
        let execution = self
            .active
            .get_mut(delegation)
            .ok_or(AppError::AssistantTurnRequired)?;
        // Optional image delivery cannot turn an already verified action into
        // a failed/unknown action or trigger replay.
        if let Err(error) = execution.append_images(state, &mut result).await {
            tracing::warn!(
                code = error.error_code(),
                "Voice tool image input unavailable"
            );
            if let Some(content) = result.get_mut("content").and_then(Value::as_array_mut) {
                content.push(json!({"type":"text","text":"Optional image input is unavailable. Keep the verified tool result; do not repeat the operation."}));
            }
        }
        Ok(result)
    }

    pub(super) async fn finish(
        &mut self,
        state: &AppState,
        delegation: &str,
        interrupted: bool,
    ) -> AppResult<()> {
        if let Some(execution) = self.active.get(delegation) {
            let result = engine::TurnResult {
                text: String::new(),
                session_id: None,
                response_id: None,
                error: interrupted.then(|| engine::TurnError::internal("cancelled", "voice_tools")),
            };
            Box::pin(engine::finish_turn_with_instructions(
                &state.db,
                &execution.row,
                &execution.row.credential_api_key_id,
                &uuid::Uuid::new_v4().to_string(),
                &result,
                None,
            ))
            .await?;
            self.active.remove(delegation);
            self.closed.insert(delegation.into());
        }
        Ok(())
    }

    pub(super) async fn heartbeat(&mut self, state: &AppState) -> AppResult<()> {
        let mut expired = Vec::new();
        for (delegation, execution) in &self.active {
            let turn = execution
                .row
                .active_turn
                .as_ref()
                .ok_or(AppError::AssistantTurnRequired)?;
            if tokio::time::Instant::now() >= execution.deadline
                || engine::stop_requested(
                    &state.db,
                    &execution.row.user_id,
                    &execution.row.id,
                    &turn.turn_id,
                )
                .await?
                || !engine::heartbeat(
                    &state.db,
                    &execution.row.user_id,
                    &execution.row.id,
                    &turn.turn_id,
                )
                .await?
            {
                expired.push(delegation.clone());
            }
        }
        for delegation in expired {
            self.finish(state, &delegation, true).await?;
        }
        Ok(())
    }

    pub(super) async fn close(&mut self, state: &AppState) {
        for delegation in self.active.keys().cloned().collect::<Vec<_>>() {
            if let Err(error) = self.finish(state, &delegation, true).await {
                tracing::warn!(
                    code = error.error_code(),
                    "Voice tool turn cleanup deferred to ordinary orphan recovery"
                );
            }
        }
    }
}

#[cfg(test)]
#[path = "tool_tests.rs"]
mod tests;

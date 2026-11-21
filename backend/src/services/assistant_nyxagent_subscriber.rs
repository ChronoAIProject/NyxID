use super::*;
use crate::models::assistant_conversation::{
    InstructionBinding, ReattachWindow, RestartAdmission, RunningResponse, SubscriberCheckpoint,
    WindowRefusal,
};
use mongodb::bson::{Binary, spec::BinarySubtype};

#[derive(serde::Serialize, serde::Deserialize)]
struct Input {
    body: Value,
    prefix: String,
    binding: InstructionBinding,
    #[serde(default)]
    follow_up: Option<FollowUp>,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct FollowUp {
    continuations: super::super::assistant_continuation::Continuations,
    images: images::ToolImages,
    base_text: Option<String>,
    notes: String,
    notes_with_memory: String,
    #[serde(default)]
    user_image_ids: Vec<String>,
}

impl FollowUp {
    pub(super) fn capture(
        continuations: &super::super::assistant_continuation::Continuations,
        images: &images::ToolImages,
        context: &super::super::assistant_instruction_context::Prepared,
        listing: &str,
        user_image_ids: Vec<String>,
    ) -> Self {
        Self {
            continuations: continuations.clone(),
            images: images.clone(),
            base_text: images.fallback.as_ref().map(|result| result.text.clone()),
            notes: context.input_with_memory(listing, "", false),
            notes_with_memory: context.input_with_memory(listing, "", true),
            user_image_ids,
        }
    }
}

impl Input {
    async fn open(
        state: &AppState,
        row: &AssistantConversation,
        window: &ReattachWindow,
    ) -> Result<Self, TurnError> {
        let encrypted = window.input.as_ref().ok_or_else(unknown)?;
        let bytes = zeroize::Zeroizing::new(
            state
                .encryption_keys
                .decrypt_with_aad(&encrypted.bytes, &aad(row, &window.key)?)
                .await
                .map_err(|_| unknown())?,
        );
        serde_json::from_slice(&bytes).map_err(|_| unknown())
    }

    fn redact(&self, text: &str, credential: &AssistantCredential) -> String {
        let text = text.replace(credential.raw_key.as_str(), "[redacted]");
        match self.binding.marker.as_deref() {
            Some(marker) => text.replace(marker, "[redacted context marker]"),
            None => text,
        }
    }

    fn full_text(&self, safe: &str) -> Result<String, TurnError> {
        if self.prefix.len().saturating_add(safe.len()) > engine::MAX_OUTPUT_BYTES {
            return Err(TurnError::internal("output_too_large", "stream"));
        }
        Ok(format!("{}{safe}", self.prefix))
    }
}

fn unknown() -> TurnError {
    TurnError::internal("outcome_unknown", "reattach")
}

fn before_admission(
    value: &Value,
    window: &ReattachWindow,
    active: &crate::models::assistant_conversation::ActiveTurn,
    version: i32,
) -> bool {
    let now = Utc::now();
    !window.accepted
        && !window.terminal
        && active.running_response.is_none()
        && !active
            .reply
            .as_ref()
            .and_then(|reply| reply.upstream.as_ref())
            .is_some_and(|cursor| cursor.window_key == window.key)
        && !(version == 1
            && (active.capacity_accepted_at.is_some()
                || active.activity_total > 0
                || active
                    .reply
                    .as_ref()
                    .is_some_and(|reply| !reply.text.is_empty())))
        && now >= active.started_at
        && window
            .absence_retry_before
            .is_some_and(|cutoff| now < cutoff)
        && [
            "response",
            "response_id",
            "conversation_id",
            "last_sequence_number",
        ]
        .iter()
        .all(|field| value.get(*field).is_some_and(Value::is_null))
}

async fn confirm(
    state: &AppState,
    row: &AssistantConversation,
    key: &str,
) -> Result<(), TurnError> {
    if super::super::assistant_turn_restart::confirm_window(&state.db, row, key)
        .await
        .map_err(|_| unknown())?
    {
        Ok(())
    } else {
        Err(TurnError::new("cancelled"))
    }
}

pub(super) async fn resolve_budget_window(
    state: &AppState,
    egress: &dyn contract::NyxAgentEgress,
    authority: &contract::NyxAgentExecutionContext,
    row: &AssistantConversation,
    credential: &AssistantCredential,
    policy: Option<BillingRoutePolicy>,
    key: &str,
) -> Result<(), TurnError> {
    tokio::time::timeout(Duration::from_secs(3), async {
        let filter = engine::WorkerFence::matching(
            row,
            doc! {
                "active_turn.stop_requested":false, "active_turn.restart_admission.window.key":key,
                "active_turn.restart_admission.credential_ended":{"$ne":true},
            },
        )
        .map_err(|_| unknown())?;
        let current = state
            .db
            .collection::<AssistantConversation>(
                crate::models::assistant_conversation::COLLECTION_NAME,
            )
            .find_one(filter)
            .await
            .map_err(|_| unknown())?
            .ok_or_else(unknown)?;
        let active = current.active_turn.as_ref().ok_or_else(unknown)?;
        let admission = active.restart_admission.as_ref().ok_or_else(unknown)?;
        let window = admission.window.as_ref().ok_or_else(unknown)?;
        let deployment = admission.reattach.as_ref().ok_or_else(unknown)?;
        if window.terminal {
            return Ok(());
        }
        let response = request(
            state,
            egress,
            authority,
            credential,
            "GET",
            "v1/responses/lookup",
            None,
            Some(key),
            Some(contract::NyxAgentDestination(
                deployment.destination_hash.clone(),
            )),
            policy,
        )
        .await
        .map_err(|_| unknown())?;
        if !response.status().is_success() {
            return Err(unknown());
        }
        let value = envelope(response).await?;
        let absent = value["status"] == "not_found"
            && before_admission(&value, window, active, admission.version);
        let failed_budget = value["status"] == "failed"
            && identity(&value, credential)
                .and_then(|binding| terminal_snapshot(&value, &binding).ok())
                .and_then(|result| result.error)
                .is_some_and(|error| {
                    matches!(error.code, "tool_budget_exhausted" | "turn_timeout")
                });
        if !absent && !failed_budget {
            return Err(unknown());
        }
        confirm(state, &current, key).await
    })
    .await
    .map_err(|_| unknown())?
}

pub(super) async fn save_refusal(
    state: &AppState,
    row: &AssistantConversation,
    key: &str,
    error: &TurnError,
    delay: Option<Duration>,
    capacity_deadline: DateTime<Utc>,
    channel_capacity_retry: bool,
) -> Result<WindowRefusal, TurnError> {
    let refusal = WindowRefusal {
        status: error.http_status.ok_or_else(unknown)?,
        code: error
            .upstream_code
            .as_deref()
            .unwrap_or(error.code)
            .to_owned(),
        retry_at: delay
            .map(|delay| chrono::Duration::from_std(delay).map(|delay| Utc::now() + delay))
            .transpose()
            .map_err(|_| unknown())?,
    };
    let filter = engine::WorkerFence::matching(row, doc! {
        "active_turn.stop_requested":false,"active_turn.restart_admission.credential_ended":{"$ne":true},
        "active_turn.restart_admission.window.key":key,"active_turn.restart_admission.window.accepted":{"$ne":true},
    }).map_err(|_| unknown())?;
    let encoded = mongodb::bson::to_bson(&refusal).map_err(|_| unknown())?;
    let mut fields = doc! {"active_turn.restart_admission.window.refusal":encoded,"active_turn.restart_admission.window.capacity_deadline":mongodb::bson::DateTime::from_chrono(capacity_deadline)};
    if channel_capacity_retry {
        fields.insert(
            "active_turn.restart_admission.window.channel_capacity_retry",
            true,
        );
    }
    let changed = state
        .db
        .collection::<mongodb::bson::Document>(
            crate::models::assistant_conversation::COLLECTION_NAME,
        )
        .update_one(filter, doc! {"$set":fields})
        .write_concern(mongodb::options::WriteConcern::majority())
        .await
        .map_err(|_| unknown())?;
    if changed.matched_count != 1 {
        return Err(unknown());
    }
    Ok(refusal)
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn reconnect(
    state: &AppState,
    egress: &dyn contract::NyxAgentEgress,
    authority: &contract::NyxAgentExecutionContext,
    row: &AssistantConversation,
    credential: &AssistantCredential,
    policy: Option<BillingRoutePolicy>,
    events: &mut Events,
    block: &str,
    partial: &mut String,
) -> Result<TurnResult, TurnError> {
    let saved = state
        .db
        .collection::<AssistantConversation>(crate::models::assistant_conversation::COLLECTION_NAME)
        .find_one(
            engine::WorkerFence::matching(row, doc! {"active_turn.stop_requested":false})
                .map_err(|_| unknown())?,
        )
        .await
        .map_err(|_| unknown())?
        .ok_or_else(unknown)?;
    let admission = saved
        .active_turn
        .as_ref()
        .and_then(|turn| turn.restart_admission.as_deref())
        .ok_or_else(unknown)?;
    let window = admission.window.as_ref().ok_or_else(unknown)?;
    recover(
        state, egress, authority, &saved, credential, policy, admission, window, events, block,
        partial, &mut None,
    )
    .await
}

fn aad(row: &AssistantConversation, key: &str) -> Result<Vec<u8>, TurnError> {
    serde_json::to_vec(&(
        "nyxid/reattach-window/1",
        &row.user_id,
        &row.id,
        &row.credential_api_key_id,
        key,
    ))
    .map_err(|_| unknown())
}

pub(super) async fn seal(
    state: &AppState,
    row: &AssistantConversation,
    key: &str,
    body: &Value,
    prefix: &str,
    binding: &InstructionBinding,
    follow_up: Option<FollowUp>,
) -> Result<Binary, TurnError> {
    let bytes = zeroize::Zeroizing::new(
        serde_json::to_vec(&Input {
            body: body.clone(),
            prefix: prefix.into(),
            binding: binding.clone(),
            follow_up,
        })
        .map_err(|_| unknown())?,
    );
    let encrypted = state
        .encryption_keys
        .encrypt_with_aad(&bytes, &aad(row, key)?)
        .await
        .map_err(|_| unknown())?;
    Ok(Binary {
        subtype: BinarySubtype::Generic,
        bytes: encrypted,
    })
}

pub(super) fn checkpoint(
    key: &str,
    id: &str,
    sequence: u64,
    window_text: &str,
) -> SubscriberCheckpoint {
    SubscriberCheckpoint {
        window_key: key.into(),
        response_id: id.into(),
        sequence,
        window_text: window_text.into(),
    }
}

fn identity(value: &Value, credential: &AssistantCredential) -> Option<RunningResponse> {
    let session = value["conversation_id"]
        .as_str()
        .filter(|id| engine::prefixed_hex(id, "conv_"))?;
    let response = value["response_id"]
        .as_str()
        .filter(|id| engine::prefixed_hex(id, &format!("resp_{}_", &session[5..])))?;
    Some(RunningResponse {
        credential_api_key_id: credential.api_key_id.clone(),
        response_id: response.into(),
        session_id: session.into(),
    })
}

fn snapshot(response: &Value, id: &str) -> Result<TurnResult, TurnError> {
    if response["id"] != id {
        return Err(unknown());
    }
    let status = response["status"].as_str().ok_or_else(unknown)?;
    if !matches!(status, "completed" | "failed" | "cancelled" | "in_progress") {
        return Err(unknown());
    }
    engine::parse_response(response, status)
}

fn terminal_snapshot(value: &Value, binding: &RunningResponse) -> Result<TurnResult, TurnError> {
    let response = &value["response"];
    let status = match value["status"].as_str() {
        Some("lost") if response["error"]["code"] == "executor_lost" => "failed",
        Some(status @ ("completed" | "failed" | "cancelled")) => status,
        _ => return Err(unknown()),
    };
    if response["status"] != status || response["conversation"]["id"] != binding.session_id {
        return Err(unknown());
    }
    snapshot(response, &binding.response_id)
}

async fn envelope(response: axum::response::Response) -> Result<Value, TurnError> {
    let bytes = axum::body::to_bytes(response.into_body(), engine::MAX_STREAM_BYTES)
        .await
        .map_err(|_| unknown())?;
    serde_json::from_slice(&bytes).map_err(|_| unknown())
}

pub(crate) struct ConfirmedCancellation {
    pub terminal: Option<TurnResult>,
}

pub(crate) async fn cancel_background(
    state: &AppState,
    egress: &dyn contract::NyxAgentEgress,
    row: &AssistantConversation,
    credential: &AssistantCredential,
    policy: Option<BillingRoutePolicy>,
) -> Option<ConfirmedCancellation> {
    let turn = row.active_turn.as_ref()?;
    let admission = turn.restart_admission.as_deref()?;
    let window = admission.window.as_ref()?;
    if window.terminal {
        return Some(ConfirmedCancellation { terminal: None });
    }
    let deployment = admission.reattach.as_ref()?;
    if admission.credential_id != credential.api_key_id {
        return None;
    }
    let authority = owner_context(row).ok()?;
    let id = turn
        .running_response
        .as_ref()
        .filter(|binding| {
            binding.credential_api_key_id == credential.api_key_id
                && engine::prefixed_hex(&binding.session_id, "conv_")
                && engine::prefixed_hex(
                    &binding.response_id,
                    &format!("resp_{}_", &binding.session_id[5..]),
                )
        })
        .map(|binding| binding.response_id.as_str());
    let path = id.filter(|_| turn.platform_origin.is_none()).map_or_else(
        || "v1/responses/cancel".into(),
        |id| format!("v1/responses/{id}/cancel"),
    );
    let mut outcome = "cancel_unconfirmed";
    let mut ordered = false;
    let mut stopped_binding = None;
    let result = tokio::time::timeout(Duration::from_secs(3), async {
        let mut polling = false;
        loop {
            let operation = if polling {
                ("GET", "v1/responses/lookup")
            } else {
                ("POST", path.as_str())
            };
            if let Ok(response) = request(
                state,
                egress,
                &authority,
                credential,
                operation.0,
                operation.1,
                None,
                Some(&window.key),
                Some(contract::NyxAgentDestination(
                    deployment.destination_hash.clone(),
                )),
                policy,
            )
            .await
                && response.status().is_success()
                && let Ok(value) = envelope(response).await
            {
                match value["status"].as_str() {
                    Some("cancelling") => {
                        polling = true;
                        ordered = true;
                    }
                    Some("cancelled")
                        if id.is_none()
                            && before_admission(&value, window, turn, admission.version) =>
                    {
                        outcome = "cancelled";
                        stopped_binding = Some((
                            row.nyxagent_session_id.clone(),
                            row.nyxagent_last_response_id.clone(),
                            row.nyxagent_instruction_binding.clone(),
                        ));
                        return None;
                    }
                    Some("cancelled" | "completed" | "failed" | "lost") => {
                        let binding = identity(&value, credential)?;
                        if id.is_some_and(|id| id != binding.response_id) {
                            return None;
                        }
                        let mut result = terminal_snapshot(&value, &binding).ok()?;
                        if executor_lost(&result) {
                            return None;
                        }
                        outcome = if value["status"] == "cancelled" {
                            "cancelled"
                        } else {
                            "terminal"
                        };
                        let input = Input::open(state, row, window).await.ok()?;
                        result.text = input
                            .full_text(&input.redact(&result.text, credential))
                            .ok()?;
                        if value["status"] == "completed" && result.error.is_none() {
                            let mut prepared = input.binding;
                            prepared.session_id.clone_from(&binding.session_id);
                            stopped_binding = Some((
                                Some(binding.session_id),
                                Some(binding.response_id),
                                Some(prepared),
                            ));
                        }
                        return Some(result);
                    }
                    _ => polling = false,
                }
            }
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
    })
    .await
    .ok()
    .flatten();
    if !turn.stop_requested && outcome != "cancel_unconfirmed" {
        let _ = confirm(state, row, &window.key).await;
    }
    if turn.stop_requested && outcome != "cancel_unconfirmed" {
        let mut filter = doc! {
            "_id": &row.id, "user_id": &row.user_id, "active_turn": mongodb::bson::Bson::Null,
            "credential_api_key_id": &admission.credential_id,
            "stopped_window.turn_id": &turn.turn_id,
            "stopped_window.window_key": &window.key,
            "stopped_window.credential_id": &admission.credential_id,
        };
        let mut update = doc! {"$unset": {"stopped_window": ""}};
        if let Some((Some(session), response, prepared)) = stopped_binding {
            filter.insert("context_reset_reason", "stopped");
            filter.insert("nyxagent_session_id", mongodb::bson::Bson::Null);
            filter.insert("model", &row.model);
            let prepared = mongodb::bson::to_bson(&prepared).ok()?;
            update.insert("$set", doc! {
                "nyxagent_session_id":session, "nyxagent_last_response_id": response,
                "nyxagent_instruction_binding":prepared, "context_reset_at":row.context_reset_at.map(mongodb::bson::DateTime::from_chrono),
                "context_reset_reason": &row.context_reset_reason,
            });
        }
        if state
            .db
            .collection::<mongodb::bson::Document>(
                crate::models::assistant_conversation::COLLECTION_NAME,
            )
            .update_one(filter, update)
            .write_concern(mongodb::options::WriteConcern::majority())
            .await
            .is_err()
        {
            tracing::warn!(conversation_id = %row.id, turn_id = %turn.turn_id, "Stopped window confirmation could not be saved");
        }
    }
    crate::services::audit_service::log_async(
        state.db.clone(),
        Some(row.user_id.clone()),
        "assistant_background_cancel".into(),
        Some(
            json!({"conversation_id":row.id,"turn_id":turn.turn_id,"response_id":id,"outcome":outcome,"cancel_ordered":ordered}),
        ),
        None,
        None,
        None,
        None,
    );
    (outcome != "cancel_unconfirmed").then_some(ConfirmedCancellation { terminal: result })
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn recover(
    state: &AppState,
    egress: &dyn contract::NyxAgentEgress,
    authority: &contract::NyxAgentExecutionContext,
    row: &AssistantConversation,
    credential: &AssistantCredential,
    policy: Option<BillingRoutePolicy>,
    admission: &RestartAdmission,
    window: &ReattachWindow,
    events: &mut Events,
    block: &str,
    partial: &mut String,
    recovered_binding: &mut Option<InstructionBinding>,
) -> Result<TurnResult, TurnError> {
    let mut saved = row.clone();
    let mut window = window.clone();
    let mut post_first = false;
    loop {
        let input = Input::open(state, &saved, &window).await?;
        *recovered_binding = Some(input.binding.clone());
        let mut image_unsupported = false;
        let result = recover_window(
            state,
            egress,
            authority,
            &saved,
            credential,
            policy,
            admission,
            &window,
            events,
            block,
            partial,
            &input,
            post_first,
            &mut image_unsupported,
        )
        .await;
        let Some(mut plan) = input.follow_up else {
            return result;
        };
        if image_unsupported {
            for id in &plan.user_image_ids {
                let _ = images::record_upload_outcome(
                    state,
                    &saved,
                    id,
                    "unavailable",
                    Some("image_input_unsupported"),
                )
                .await;
            }
            plan.images.unsupported(state, &saved).await;
            events.emit("turn.notice",json!({"code":"image_input_unavailable","message":crate::services::assistant_upload_service::MODEL_IMAGE_FALLBACK}));
        }
        let failed = match &result {
            Err(error) => Some(error),
            Ok(result) => result.error.as_ref(),
        };
        if let Some(error) = failed
            && let Some(base) = &plan.base_text
            && allows_fallback(error)
        {
            plan.images.failed(state, &saved).await;
            clear_session(state, &saved, "image_input_unavailable")
                .await
                .map_err(|_| unknown())?;
            *recovered_binding = None;
            *partial = base.clone();
            return Ok(TurnResult {
                text: base.clone(),
                session_id: None,
                response_id: None,
                error: None,
            });
        }
        let mut result = result?;
        let Some(session_id) = result.session_id.as_deref() else {
            return Ok(result);
        };
        let turn = saved.active_turn.as_ref().ok_or_else(unknown)?;
        let (key, text) = if let Some(error) = &result.error {
            if !matches!(error.code, "tool_budget_exhausted" | "turn_timeout")
                || plan.base_text.is_some()
            {
                return Ok(result);
            }
            if let Some(next) = &window.next_continuation {
                plan.continuations = next.clone();
            } else {
                let suffix = result
                    .text
                    .strip_prefix(input.prefix.as_str())
                    .unwrap_or(&result.text);
                match continue_turn(
                    state,
                    &saved,
                    events,
                    &mut plan.continuations,
                    Some(session_id),
                    error,
                    suffix,
                )
                .await
                {
                    Ok(true) => {}
                    Ok(false) => return Ok(result),
                    Err(error) => {
                        result.error = Some(error);
                        return Ok(result);
                    }
                }
            }
            (
                format!("{}:continuation:{}", turn.turn_id, plan.continuations.count),
                super::super::assistant_continuation::INSTRUCTION.to_owned(),
            )
        } else {
            plan.images.sent(state, &saved).await;
            if !plan
                .images
                .prepare(state, egress, authority, &saved, credential, policy)
                .await
                .unwrap_or(false)
            {
                return Ok(result);
            }
            images::renew(state, &saved, session_id, events, plan.images.rounds).await?;
            plan.images.attempted(state, &saved).await;
            plan.base_text = Some(result.text.clone());
            (
                format!("{}:images:{}", turn.turn_id, plan.images.rounds),
                format!(
                    "Continue after viewing the tool images.\n\n{}",
                    plan.images.note
                ),
            )
        };
        let notes = if plan.continuations.count == 0 {
            &plan.notes_with_memory
        } else {
            &plan.notes
        };
        let text = text.replace(credential.raw_key.as_str(), "[redacted]");
        let text = input.binding.marker.as_ref().map_or_else(
            || text.clone(),
            |marker| text.replace(marker, "[redacted context marker]"),
        );
        let text = format!("{notes}{text}");
        let mut body = input.body;
        body["conversation"] = json!(session_id);
        body["input"] = if plan.images.parts.is_empty() {
            json!(text)
        } else {
            let mut parts = vec![json!({"type":"input_text","text":text})];
            parts.extend(plan.images.parts.iter().cloned());
            json!([{"role":"user","content":parts}])
        };
        let prefix = if result.text.is_empty() {
            String::new()
        } else {
            format!("{}\n\n", result.text)
        };
        let now = Utc::now();
        let run_cap = (turn.trigger_run_id.is_some() && !turn.trigger_continuation).then(|| {
            now + chrono::Duration::from_std(
                super::super::assistant_engine_policy::AUTOMATION_CAPACITY_CAP,
            )
            .expect("bounded capacity cap")
        });
        let deadline = now
            + chrono::Duration::from_std(engine::capacity_budget(now, run_cap, now))
                .map_err(|_| unknown())?;
        let sealed = seal(
            state,
            &saved,
            &key,
            &body,
            &prefix,
            &input.binding,
            Some(plan),
        )
        .await?;
        super::super::assistant_turn_observability::attempt(
            &state.db,
            &saved,
            &key,
            deadline,
            Some(sealed),
            None,
        )
        .await
        .map_err(|_| unknown())?;
        let filter = engine::WorkerFence::matching(
            &saved,
            doc! {
                "active_turn.stop_requested": false,
                "active_turn.restart_admission.window.key": &key,
            },
        )
        .map_err(|_| unknown())?;
        saved = state
            .db
            .collection::<AssistantConversation>(
                crate::models::assistant_conversation::COLLECTION_NAME,
            )
            .find_one(filter)
            .await
            .map_err(|_| unknown())?
            .ok_or_else(unknown)?;
        window = saved
            .active_turn
            .as_ref()
            .and_then(|turn| turn.restart_admission.as_ref()?.window.as_ref())
            .cloned()
            .ok_or_else(unknown)?;
        post_first = true;
    }
}

#[allow(clippy::too_many_arguments)]
async fn recover_window(
    state: &AppState,
    egress: &dyn contract::NyxAgentEgress,
    authority: &contract::NyxAgentExecutionContext,
    row: &AssistantConversation,
    credential: &AssistantCredential,
    policy: Option<BillingRoutePolicy>,
    admission: &RestartAdmission,
    window: &ReattachWindow,
    events: &mut Events,
    block: &str,
    partial: &mut String,
    input: &Input,
    post_first: bool,
    image_unsupported: &mut bool,
) -> Result<TurnResult, TurnError> {
    let active = row.active_turn.as_ref().ok_or_else(unknown)?;
    let deployment = admission.reattach.as_ref().ok_or_else(unknown)?;
    if row.credential_api_key_id != admission.credential_id
        || credential.api_key_id != admission.credential_id
    {
        return Err(unknown());
    }
    let mut key = window.key.clone();
    let mut accepted = window.accepted;
    let redact = |text: &str| input.redact(text, credential);
    let prior = active.reply.as_ref();
    *partial = prior
        .filter(|reply| reply.text.starts_with(&input.prefix))
        .map_or_else(|| input.prefix.clone(), |reply| reply.text.clone());
    let cursor = prior
        .and_then(|reply| reply.upstream.as_ref())
        .filter(|cursor| cursor.window_key == key);
    let mut sequence = cursor.map(|cursor| cursor.sequence);
    let mut text = cursor
        .map(|cursor| cursor.window_text.clone())
        .unwrap_or_default();
    let mut expected = cursor.map(|cursor| cursor.response_id.clone()).or_else(|| {
        active
            .running_response
            .as_ref()
            .map(|running| running.response_id.clone())
    });
    let deadline = tokio::time::Instant::now() + turn_remaining(row);
    let destination = || {
        Some(contract::NyxAgentDestination(
            deployment.destination_hash.clone(),
        ))
    };
    let mut retry = false;
    let mut fresh_post = post_first;
    let mut recovery = engine::Recovery::default();
    let mut refusal = window.refusal.clone();
    let mut capacity_deadline = window.capacity_deadline;
    let mut channel_capacity_retry = window.channel_capacity_retry;
    let mut first_channel_refusal =
        active.engine_attempts == 1 && !accepted && refusal.is_none() && !channel_capacity_retry;
    let channel_guard = || {
        super::super::assistant_turn_observability::channel_retry_guard(
            row,
            input.body["conversation"].as_str(),
        )
    };
    let mut bound = None;
    loop {
        if retry {
            tokio::time::sleep(Duration::from_millis(350)).await;
        }
        retry = true;
        if tokio::time::Instant::now() >= deadline {
            return Err(unknown());
        }
        if let Some(saved) = refusal.take() {
            let error = post_refusal(saved.status, &saved.code, None);
            if let Some(at) = saved.retry_at {
                tokio::time::sleep(
                    (at.min(capacity_deadline) - Utc::now())
                        .to_std()
                        .unwrap_or_default(),
                )
                .await;
                if Utc::now() >= capacity_deadline {
                    return Err(error);
                }
                let mut filter = engine::WorkerFence::matching(row, doc! {"active_turn.stop_requested":false,
                    "active_turn.restart_admission.credential_ended":{"$ne":true},"active_turn.restart_admission.window.key":&key,
                    "active_turn.restart_admission.window.refusal.retry_at":mongodb::bson::DateTime::from_chrono(at)})
                    .map_err(|_| unknown())?;
                if channel_capacity_retry {
                    filter.extend(channel_guard());
                }
                let changed = state.db.collection::<mongodb::bson::Document>(crate::models::assistant_conversation::COLLECTION_NAME)
                    .update_one(filter, doc! {"$unset":{"active_turn.restart_admission.window.refusal":""},"$inc":{"active_turn.engine_attempts":1}})
                    .write_concern(mongodb::options::WriteConcern::majority()).await.map_err(|_| unknown())?;
                if changed.matched_count != 1 {
                    return Err(unknown());
                }
                fresh_post = true;
            } else if matches!(error.code, "turn_timeout" | "tool_budget_exhausted") {
                let session_id = input.body["conversation"].as_str().map(str::to_owned);
                if session_id.is_some()
                    && input.follow_up.as_ref().is_some_and(|plan| {
                        plan.base_text.is_none()
                            && plan.continuations.count < plan.continuations.maximum
                    })
                {
                    resolve_budget_window(state, egress, authority, row, credential, policy, &key)
                        .await?;
                }
                return Ok(TurnResult {
                    text: partial.clone(),
                    session_id,
                    response_id: None,
                    error: Some(error),
                });
            } else {
                return Err(error);
            }
        }
        let mut reader = if fresh_post {
            fresh_post = false;
            let source_expiry = if let Some(
                crate::models::assistant_conversation::ChatRestartSource::PrivateChannel {
                    event_id,
                    ..
                },
            ) = admission.chat_source.as_deref()
            {
                Some(
                    egress
                        .channel_retry_expiry(row, event_id)
                        .await
                        .map_err(|_| TurnError::internal("assistant_unavailable", "prepare"))?,
                )
            } else {
                None
            };
            if Utc::now()
                >= source_expiry.map_or(capacity_deadline, |expiry| capacity_deadline.min(expiry))
            {
                return Err(TurnError::internal("capacity_exceeded", "capacity"));
            }
            let bounds = stream_bounds();
            let mut first_byte = super::super::assistant_start_retry::FirstByteDeadline::new(
                bounds.first_byte,
                bounds.no_progress,
            );
            let response = first_byte
                .wait(
                    &state.db,
                    row,
                    request(
                        state,
                        egress,
                        authority,
                        credential,
                        "POST",
                        "v1/responses",
                        Some(input.body.clone()),
                        Some(&key),
                        destination(),
                        policy,
                    ),
                )
                .await?
                .ok();
            if let Some(rejected) = response.as_ref().filter(|response| {
                !response.status().is_success() && response.status().as_u16() != 410
            }) {
                let status = rejected.status().as_u16();
                let headers = rejected.headers();
                let retry_after = headers
                    .get("retry-after")
                    .and_then(|value| value.to_str().ok())
                    .and_then(engine::retry_after);
                let request_id = headers
                    .get("x-request-id")
                    .and_then(|value| value.to_str().ok())
                    .and_then(sanitize_upstream_request_id);
                let bytes = tokio::time::timeout(
                    crate::services::assistant_engine_policy::ADMISSION_ERROR_BODY_TIMEOUT,
                    axum::body::to_bytes(
                        response.expect("refusal").into_body(),
                        crate::services::assistant_engine_policy::ADMISSION_ERROR_BODY_MAX_BYTES,
                    ),
                )
                .await
                .map_err(|_| TurnError::internal("first_byte_timeout", "first_byte"))?
                .map_err(|_| TurnError::internal("invalid_stream", "upstream_response"))?;
                let value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
                let code = upstream_error_code(status, &value);
                if let Some(expiry) = source_expiry {
                    if first_channel_refusal && status == 503 && code == "capacity_exceeded" {
                        let started = active
                            .engine_request_started_at
                            .unwrap_or(active.started_at);
                        capacity_deadline = started
                            + chrono::Duration::from_std(
                                crate::services::assistant_engine_policy::CHANNEL_CAPACITY_BUDGET,
                            )
                            .expect("bounded channel budget");
                        channel_capacity_retry = true;
                    }
                    capacity_deadline = capacity_deadline.min(expiry);
                    first_channel_refusal = false;
                }
                let error = post_refusal(status, code, request_id);
                let delay = engine::admission_refusal(status, code).then(|| {
                    recovery.retries = recovery.retries.saturating_add(1);
                    engine::capacity_delay(recovery.retries, retry_after, rand::random())
                });
                refusal = Some(
                    save_refusal(
                        state,
                        row,
                        &key,
                        &error,
                        delay,
                        capacity_deadline,
                        channel_capacity_retry,
                    )
                    .await?,
                );
                retry = false;
                continue;
            }
            if response
                .as_ref()
                .is_some_and(|response| response.status().is_success())
            {
                super::super::assistant_turn_observability::accepted(&state.db, row)
                    .await
                    .map_err(|_| unknown())?;
                accepted = true;
            }
            response
        } else {
            None
        };
        if reader.is_none() {
            let Ok(reply) = request(
                state,
                egress,
                authority,
                credential,
                "GET",
                "v1/responses/lookup",
                None,
                Some(&key),
                destination(),
                policy,
            )
            .await
            else {
                continue;
            };
            if !reply.status().is_success() {
                continue;
            }
            let Ok(value) = envelope(reply).await else {
                continue;
            };
            let status = value["status"].as_str().unwrap_or("");
            let binding = identity(&value, credential);
            if expected.as_ref().is_some_and(|expected| {
                binding
                    .as_ref()
                    .is_none_or(|binding| &binding.response_id != expected)
            }) {
                return Err(unknown());
            }
            let absent = !accepted
                && status == "not_found"
                && before_admission(&value, window, active, admission.version);
            let not_started = status == "failed"
                && sequence.is_none()
                && binding.as_ref().is_some_and(|binding| {
                    value["response"]["id"] == binding.response_id
                        && value["response"]["status"] == "failed"
                        && value["response"]["error"]["code"] == "not_started"
                        && value["response"]["error"]["tool_dispatch"] == "none"
                });
            if absent || not_started {
                let now = Utc::now();
                if window.terminal
                    || (absent && (cursor.is_some() || expected.is_some()))
                    || now >= capacity_deadline
                    || (absent
                        && window
                            .absence_retry_before
                            .is_none_or(|cutoff| now >= cutoff || now < active.started_at))
                {
                    return Err(unknown());
                }
                let next = Uuid::new_v4().to_string();
                let saved = seal(
                    state,
                    row,
                    &next,
                    &input.body,
                    &input.prefix,
                    &input.binding,
                    input.follow_up.clone(),
                )
                .await?;
                let mut filter = engine::WorkerFence::matching(row, doc! {"active_turn.stop_requested":false,"credential_api_key_id":&admission.credential_id,"active_turn.restart_admission.credential_ended":{"$ne":true},"active_turn.restart_admission.window.key":&key}).map_err(|_| unknown())?;
                if channel_capacity_retry {
                    filter.extend(channel_guard());
                }
                if absent {
                    filter.insert(
                        "active_turn.restart_admission.window.absence_retry_before",
                        doc! {"$gt":mongodb::bson::DateTime::now()},
                    );
                }
                let changed = state.db.collection::<mongodb::bson::Document>(crate::models::assistant_conversation::COLLECTION_NAME).update_one(filter,doc!{"$set":{"active_turn.restart_admission.window.key":&next,"active_turn.restart_admission.window.terminal":false,"active_turn.restart_admission.window.accepted":false,"active_turn.restart_admission.window.input":saved,"active_turn.running_response":null},"$unset":{"active_turn.reply.upstream":"","active_turn.restart_admission.window.next_continuation":""}}).write_concern(mongodb::options::WriteConcern::majority()).await.map_err(|_|unknown())?;
                if changed.matched_count != 1 {
                    return Err(unknown());
                }
                key = next;
                accepted = false;
                expected = None;
                sequence = None;
                text.clear();
                bound = None;
                fresh_post = true;
                continue;
            }
            if status == "cancelled"
                && binding.is_none()
                && !accepted
                && before_admission(&value, window, active, admission.version)
            {
                confirm(state, row, &key).await?;
                return Err(TurnError::new("cancelled"));
            }
            let Some(binding) = binding else {
                return Err(unknown());
            };
            expected = Some(binding.response_id.clone());
            match status {
                "running" => {}
                "completed" | "failed" | "cancelled" | "lost" => {
                    let mut result = terminal_snapshot(&value, &binding)?;
                    *image_unsupported = engine::unsupported_images(&value["response"]);
                    result.text = input.full_text(&redact(&result.text))?;
                    if status == "lost" {
                        result.error = Some(TurnError::new("executor_lost"));
                        if partial.starts_with(&result.text) {
                            result.text = partial.clone();
                        }
                    }
                    if !executor_lost(&result) {
                        confirm(state, row, &key).await?;
                    }
                    return Ok(result);
                }
                _ => return Err(unknown()),
            }
            if bound.as_ref() != Some(&binding.response_id) {
                if !crate::services::assistant_steering::set_running_response(
                    &state.db,
                    row,
                    Some(&binding),
                )
                .await
                .map_err(|_| unknown())?
                {
                    return Err(TurnError::new("cancelled"));
                }
                bound = Some(binding.response_id.clone());
            }
            let path = format!(
                "v1/responses/{}/events{}",
                binding.response_id,
                sequence
                    .map(|seq| format!("?after={seq}"))
                    .unwrap_or_default()
            );
            reader = request(
                state,
                egress,
                authority,
                credential,
                "GET",
                &path,
                None,
                Some(&key),
                destination(),
                policy,
            )
            .await
            .ok();
        }
        let Some(response) = reader else { continue };
        let mut gap = None;
        if response.status().as_u16() == 410 {
            gap = envelope(response).await.ok();
        } else if response.status().as_u16() == 200 {
            let mut body = response.into_body().into_data_stream();
            let mut decoder = ResponseStream::default();
            decoder.replay = true;
            decoder.sequence = sequence;
            decoder.text.clone_from(&text);
            let bounds = stream_bounds();
            let mut last_progress = tokio::time::Instant::now();
            let mut received = false;
            loop {
                let timeout = if received {
                    stream_timeout(bounds, Duration::ZERO, last_progress.elapsed())
                } else {
                    bounds.first_byte
                };
                let Ok(Some(Ok(bytes))) = tokio::time::timeout(timeout, body.next()).await else {
                    break;
                };
                received = true;
                let before = decoder.data_events;
                if decoder.push(&bytes).is_err() {
                    break;
                }
                if decoder.data_events != before {
                    last_progress = tokio::time::Instant::now();
                }
                if let Some(created) = decoder.created.take() {
                    if expected
                        .as_ref()
                        .is_some_and(|id| id != &created.response_id)
                    {
                        return Err(unknown());
                    }
                    expected = Some(created.response_id);
                }
                if let Some(mut result) = decoder.terminal.take() {
                    *image_unsupported = decoder.image_input_unsupported;
                    if expected.as_deref() != result.response_id.as_deref() {
                        return Err(unknown());
                    }
                    result.text = input.full_text(&redact(&result.text))?;
                    if !executor_lost(&result) {
                        confirm(state, row, &key).await?;
                    }
                    return Ok(result);
                }
                let safe = redact(&decoder.text);
                let held = engine::secret_prefix_suffix(
                    &safe,
                    &[
                        &credential.raw_key,
                        input.binding.marker.as_deref().unwrap_or(""),
                    ],
                );
                if held.is_empty()
                    && let (Some(id), Some(seq)) = (&expected, decoder.sequence)
                {
                    let full = input.full_text(&safe)?;
                    if full != *partial {
                        events.emit_with_checkpoint("block.completed",json!({"block_id":block,"block":{"type":"text","block_id":block,"text":full}}), Some(checkpoint(&key, id, seq, &safe)));
                    }
                    *partial = full;
                    sequence = Some(seq);
                    text = safe;
                }
                if let Some(error) = decoder.reader_error.take() {
                    gap = Some(error);
                    break;
                }
            }
        }
        if let Some(gap) = gap.filter(|value| value["error"]["code"] == "events_not_retained")
            && let Some(id) = &expected
        {
            let response = &gap["response"];
            let mut result = snapshot(response, id)?;
            let safe = redact(&result.text);
            if response["status"] != "in_progress" {
                *image_unsupported = engine::unsupported_images(response);
                result.text = input.full_text(&safe)?;
                if !executor_lost(&result) {
                    confirm(state, row, &key).await?;
                }
                return Ok(result);
            }
            let next = gap["snapshot_sequence_number"]
                .as_u64()
                .ok_or_else(unknown)?;
            if sequence.is_some_and(|before| before > next) {
                return Err(unknown());
            }
            text = result.text;
            sequence = Some(next);
            let held = engine::secret_prefix_suffix(
                &safe,
                &[
                    &credential.raw_key,
                    input.binding.marker.as_deref().unwrap_or(""),
                ],
            );
            if held.is_empty() {
                *partial = input.full_text(&safe)?;
                events.emit_with_checkpoint(
                    "block.completed",
                    json!({"block_id":block,"block":{"type":"text","block_id":block,"text":partial}}),
                    Some(checkpoint(&key, id, next, &safe)),
                );
            }
        }
    }
}

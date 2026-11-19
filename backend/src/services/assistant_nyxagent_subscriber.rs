use super::*;
use crate::models::assistant_conversation::{
    InstructionBinding, ReattachWindow, RestartAdmission, RunningResponse, SubscriberCheckpoint,
};
use mongodb::bson::{Binary, spec::BinarySubtype};

#[derive(serde::Serialize, serde::Deserialize)]
struct Input {
    body: Value,
    prefix: String,
    binding: InstructionBinding,
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
}

fn unknown() -> TurnError {
    TurnError::internal("outcome_unknown", "reattach")
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
) -> Result<Binary, TurnError> {
    let bytes = zeroize::Zeroizing::new(
        serde_json::to_vec(&Input {
            body: body.clone(),
            prefix: prefix.into(),
            binding: binding.clone(),
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
    let mut saved = response.clone();
    let kind = if status == "in_progress" {
        "completed"
    } else {
        status
    };
    saved["status"] = json!(kind);
    let mut decoder = ResponseStream::default();
    decoder
        .event(json!({"type":format!("response.{kind}"),"sequence_number":0,"response":saved}))?;
    decoder.terminal.ok_or_else(unknown)
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
    let path = id.map_or_else(
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
                            && [
                                "response",
                                "response_id",
                                "conversation_id",
                                "last_sequence_number",
                            ]
                            .iter()
                            .all(|field| value.get(*field).is_some_and(Value::is_null)) =>
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
                        outcome = if value["status"] == "cancelled" {
                            "cancelled"
                        } else {
                            "terminal"
                        };
                        let input = Input::open(state, row, window).await.ok()?;
                        result.text =
                            format!("{}{}", input.prefix, input.redact(&result.text, credential));
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
    let active = row.active_turn.as_ref().ok_or_else(unknown)?;
    let deployment = admission.reattach.as_ref().ok_or_else(unknown)?;
    if row.credential_api_key_id != admission.credential_id
        || credential.api_key_id != admission.credential_id
    {
        return Err(unknown());
    }
    let mut key = window.key.clone();
    let input = Input::open(state, row, window).await?;
    *recovered_binding = Some(input.binding.clone());
    let redact = |text: &str| input.redact(text, credential);
    let prior = active.reply.as_ref();
    *partial = prior.map_or_else(|| input.prefix.clone(), |reply| reply.text.clone());
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
    let deadline = active.started_at
        + chrono::Duration::from_std(stream_bounds().turn_cap).map_err(|_| unknown())?;
    let destination = || {
        Some(contract::NyxAgentDestination(
            deployment.destination_hash.clone(),
        ))
    };
    let mut retry = false;
    let mut fresh_post = false;
    let mut bound = None;
    loop {
        if retry {
            tokio::time::sleep(Duration::from_millis(350)).await;
        }
        retry = true;
        if Utc::now() >= deadline {
            return Err(unknown());
        }
        let mut reader = if fresh_post {
            fresh_post = false;
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
            )
            .await
            .ok()
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
            let absent = status == "not_found"
                && value["response"].is_null()
                && value["response_id"].is_null()
                && value["conversation_id"].is_null()
                && value["last_sequence_number"].is_null();
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
                if (absent
                    && (active.capacity_accepted_at.is_some()
                        || !partial.is_empty()
                        || active.activity_total > 0))
                    || now >= window.capacity_deadline
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
                )
                .await?;
                let mut filter = engine::WorkerFence::matching(row, doc! {"active_turn.stop_requested":false,"credential_api_key_id":&admission.credential_id,"active_turn.restart_admission.credential_ended":{"$ne":true},"active_turn.restart_admission.window.key":&key}).map_err(|_| unknown())?;
                if absent {
                    filter.insert(
                        "active_turn.restart_admission.window.absence_retry_before",
                        doc! {"$gt":mongodb::bson::DateTime::now()},
                    );
                }
                let changed = state.db.collection::<mongodb::bson::Document>(crate::models::assistant_conversation::COLLECTION_NAME).update_one(filter,doc!{"$set":{"active_turn.restart_admission.window.key":&next,"active_turn.restart_admission.window.input":saved,"active_turn.running_response":null},"$unset":{"active_turn.reply.upstream":""}}).write_concern(mongodb::options::WriteConcern::majority()).await.map_err(|_|unknown())?;
                if changed.matched_count != 1 {
                    return Err(unknown());
                }
                key = next;
                expected = None;
                sequence = None;
                text.clear();
                bound = None;
                fresh_post = true;
                continue;
            }
            if status == "cancelled" && binding.is_none() && value["response"].is_null() {
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
                    result.text = format!("{}{}", input.prefix, redact(&result.text));
                    if status == "lost" {
                        result.error = Some(TurnError::new("executor_lost"));
                        if partial.starts_with(&result.text) {
                            result.text = partial.clone();
                        }
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
            while let Some(Ok(bytes)) = body.next().await {
                if decoder.push(&bytes).is_err() {
                    break;
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
                    if expected.as_deref() != result.response_id.as_deref() {
                        return Err(unknown());
                    }
                    result.text = format!("{}{}", input.prefix, redact(&result.text));
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
                    let full = format!("{}{safe}", input.prefix);
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
                result.text = format!("{}{safe}", input.prefix);
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
                *partial = format!("{}{safe}", input.prefix);
                events.emit_with_checkpoint(
                    "block.completed",
                    json!({"block_id":block,"block":{"type":"text","block_id":block,"text":partial}}),
                    Some(checkpoint(&key, id, next, &safe)),
                );
            }
        }
    }
}

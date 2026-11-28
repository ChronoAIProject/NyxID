use super::*;

const PROVIDER_PENDING_NOTICE: &str = "The provider accepted the operation (HTTP 202); completion is pending and unverified. It will not be sent again.";

/// In-process capability created only after the stored source and live turn match.
/// No public request deserializes this object.
#[derive(Clone)]
pub struct Resume {
    pub(crate) supervisor_assignment: bool,
    pub(crate) connected_wait: Option<Arc<super::v2::blocking::WaitGuard>>,
    pub version: u32,
    pub request_id: String,
    pub claim_id: String,
    pub effect_id: String,
    pub grant_id: String,
    pub source_credential_id: Option<String>,
    pub prepared_digest: String,
    pub authority_digest: String,
    pub effect: PreparedEffect,
    pub independent_card_id: Option<String>,
}
impl std::fmt::Debug for Resume {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Resume { [REDACTED] }")
    }
}
impl Resume {
    pub(in crate::services::execution_permission_dispatch) async fn node_not_sent(
        &self,
        db: &Database,
    ) -> AppResult<()> {
        let mut session = db.client().start_session().await?;
        session.start_transaction().await?;
        let filter = v3::rebase(
            doc! {
                "_id": &self.request_id,
                "execution_proposal.version": self.version as i32,
                "execution_proposal.v1.claim_id": &self.claim_id,
                "execution_proposal.v1.grant_id": &self.grant_id,
                "execution_proposal.v1.delivery_abandoned": {"$ne": true},
                "execution_proposal.v1.state": "dispatched",
            },
            self.version,
        );
        let update = v3::rebase(
            doc! {
                "$set": {
                    "execution_proposal.v1.state": "not_sent",
                    "execution_proposal.v1.failure_code": "operation_not_sent",
                },
                "$unset": Slot::Assistant(AssistantVersion::V1).unset(&[Field::Payload])
            },
            self.version,
        );
        if db
            .collection::<Document>(REQUESTS)
            .update_one(filter, update)
            .session(&mut session)
            .await?
            .matched_count
            == 1
        {
            lifecycle::retire_operations(db, &mut session, std::slice::from_ref(&self.request_id))
                .await?;
        }
        session.commit_transaction().await?;
        Ok(())
    }

    pub(in crate::services::execution_permission_dispatch) async fn fence(
        &self,
        db: &Database,
        session: &mut ClientSession,
        dispatch: bool,
    ) -> AppResult<()> {
        if self.version == 2 {
            return super::v2::fence_resume(self, db, session, dispatch).await;
        }
        let row = db
            .collection::<ApprovalRequest>(REQUESTS)
            .find_one(doc! {"_id":&self.request_id})
            .session(&mut *session)
            .await?
            .ok_or_else(refused)?;
        let version = AssistantVersion::of(&row)?;
        if version.number() != self.version {
            return Err(conflict());
        }
        version.require_serving(db).await?;
        fence_endpoint(db, session, proposal(&row)?).await?;

        let update = if dispatch {
            doc! {"$set":{"execution_proposal.v1.state":"dispatched"}}
        } else {
            doc! {"$inc":{"execution_dispatch_fence":1_i64}}
        };
        if db.collection::<Document>(REQUESTS).update_one(version.selector(doc!{"_id":&self.request_id,"status":"approved","execution_proposal.v1.state":"preparing","execution_proposal.v1.claim_id":&self.claim_id,"execution_proposal.v1.grant_id":&self.grant_id}),version.update(update)).session(session).await?.matched_count!=1{return Err(uncertain());}
        Ok(())
    }
}

// A task root and a later assigning Event may authenticate distinct original
// messages in the same owner chat. Each proof is validated independently; only
// their shared authorization identity is comparable across those messages.
fn same_owner_chat(
    left: &crate::models::execution_permission_channel::ChannelSource,
    right: &crate::models::execution_permission_channel::ChannelSource,
) -> bool {
    left.version == right.version
        && left.actor_user_id == right.actor_user_id
        && left.agent_id == right.agent_id
        && left.conversation_id == right.conversation_id
        && left.key_id == right.key_id
        && left.channel_id == right.channel_id
        && left.thread_id == right.thread_id
        && left.settings_thread_id == right.settings_thread_id
        && left.channel_revision == right.channel_revision
        && left.thread_revision == right.thread_revision
        && left.audience_id == right.audience_id
}

const PROVIDER_REJECTION: &str = "The provider rejected this write; this attempt is not confirmed as successful. An independent read reports observed state only, even if it already matches the requested value.";

fn rejected(request: &ApprovalRequest, p: &Proposal) -> bool {
    matches!(
        p.failure_code.as_deref(),
        Some("provider_http_rejected" | "provider_application_rejected")
    ) || (p.state == "completed" && request.activity_outcome.as_deref() == Some("failed"))
}

fn event_source_matches(
    event: &crate::models::assistant_conversation::AgentEvent,
    p: &Proposal,
) -> bool {
    event.kind == "permission_decided"
        && event.bound
        && event.report_to == p.report_to
        && event.invocation_id == p.invocation_id
        && event
            .execution_task
            .as_ref()
            .is_some_and(|task| super::super::super::supervisor_write::same_task(task, &p.task))
        && event.channel_contexts.is_empty()
}

async fn require_durable_event(
    db: &Database,
    row: &AssistantConversation,
    turn: &crate::models::assistant_conversation::ActiveTurn,
    event: &crate::models::assistant_conversation::AgentEvent,
    p: &Proposal,
) -> AppResult<()> {
    if turn.origin != crate::models::assistant_conversation::TurnOrigin::Event
        || !event_source_matches(event, p)
        || turn.report_to != p.report_to
        || turn.invocation_id != p.invocation_id
    {
        return Err(refused());
    }
    let stored = db
        .collection::<Document>("assistant_conversations")
        .find_one(doc! {
            "_id":&row.id,"user_id":&row.user_id,"credential_api_key_id":&p.key_id,
            "active_turn.turn_id":&turn.turn_id,"active_turn.stop_requested":false,
            "active_turn.report_to":&p.report_to,"active_turn.invocation_id":&p.invocation_id
        })
        .projection(doc! {"active_turn.events":1})
        .await?
        .ok_or_else(refused)?;
    let events: Vec<crate::models::assistant_conversation::AgentEvent> = bson::from_bson(
        stored
            .get_document("active_turn")
            .map_err(encoding)?
            .get("events")
            .cloned()
            .ok_or_else(refused)?,
    )
    .map_err(encoding)?;
    if !events
        .iter()
        .any(|stored| stored.id == event.id && event_source_matches(stored, p))
    {
        return Err(refused());
    }
    Ok(())
}

fn same_supervisor_source(
    left: &crate::models::approval_request::SupervisorSource,
    right: &crate::models::approval_request::SupervisorSource,
) -> bool {
    // Within one stored assignment, preserve every authorization field. The
    // audience label is explicitly display-only, never source authority.
    same_owner_chat(&left.channel, &right.channel)
        && left.channel.event_id == right.channel.event_id
        && left.channel.original_turn_id == right.channel.original_turn_id
        && left.channel.policy_digest == right.channel.policy_digest
        && left.channel.expires_at == right.channel.expires_at
        && left.assigning_turn == right.assigning_turn
        && left.reset_at == right.reset_at
}

fn specialist_readback(
    row: &AssistantConversation,
    proposal: &Proposal,
    resume: &Resume,
) -> Option<(String, crate::models::approval_request::SupervisorSource)> {
    let turn = row.active_turn.as_ref()?;
    let origin = resume.effect.supervisor.as_deref()?;
    let task_source = proposal.task.channel_source.as_deref()?;
    let target = origin.source.channel.conversation_id.as_str();
    if !crate::services::supervisor_write::personal(row)
        || !row.is_subagent()
        || turn.origin != crate::models::assistant_conversation::TurnOrigin::Event
        || turn.voice_request_id.is_some()
        || turn.trigger_run_id.is_some()
        || turn.automation_work.is_some()
        || turn.invocation_id.is_some()
        || turn.platform_origin.is_some()
        || turn.channel_adapter_origin.is_some()
        || turn.report_to.as_deref() != Some(target)
        || proposal.report_to.as_deref() != Some(target)
        || turn
            .events
            .first()
            .is_none_or(|event| !event.bound || event.report_to.as_deref() != Some(target))
        || row.agent_id.as_deref() != Some(origin.agent.as_str())
        || origin.conversation != row.id
        || origin.key != row.credential_api_key_id
        || origin.connection != resume.effect.connection
        || origin.catalog != resume.effect.catalog
        || origin.marked.operation_id != proposal.effect_id
        || origin.marked.origin_turn != proposal.source_turn_id
        || !crate::services::supervisor_write::same_task(&origin.marked.task, &proposal.task)
        || !same_owner_chat(task_source, &origin.source.channel)
        || origin.source.channel.actor_user_id != row.user_id
        || turn
            .supervisor_source
            .as_deref()
            .is_some_and(|source| !same_supervisor_source(source, &origin.source))
    {
        return None;
    }
    crate::services::supervisor_write::readback_instruction(&resume.effect)
        .map(|instruction| (instruction, origin.source.clone()))
}

/// Recheck restored assignment after the receipt is durable, before choosing a
/// model input. Failed optional reads never turn a known effect into uncertainty.
async fn require_specialist_source(
    db: &Database,
    row: &AssistantConversation,
    expected: &crate::models::approval_request::SupervisorSource,
) -> AppResult<()> {
    let task = row
        .active_turn
        .as_ref()
        .and_then(|turn| turn.execution_task.as_ref())
        .ok_or_else(refused)?;
    tasks::check(db, task).await?;
    let mut session = db.client().start_session().await?;
    crate::services::execution_permission_channel::validate_supervisor(
        db,
        &mut session,
        &expected.channel,
        false,
    )
    .await?;
    let parent = db
        .collection::<AssistantConversation>("assistant_conversations")
        .find_one(doc! {
            "_id": &expected.channel.conversation_id,
            "user_id": &expected.channel.actor_user_id,
            "agent_id": &expected.channel.agent_id,
            "credential_api_key_id": &expected.channel.key_id,
            "context_reset_at": expected.reset_at.map(bson::DateTime::from_chrono),
        })
        .await?
        .ok_or_else(refused)?;
    if !crate::services::supervisor_write::personal(&parent) || parent.is_subagent() {
        return Err(refused());
    }
    let mut filter = crate::services::assistant_nyxagent::WorkerFence::capture(row)?.filter();
    filter.extend(doc! {
        "credential_api_key_id": &row.credential_api_key_id,
        "agent_id": &row.agent_id, "model": &row.model,
        "active_turn.stop_requested": false,
        "active_turn.report_to": &expected.channel.conversation_id,
    });
    let current = db
        .collection::<AssistantConversation>("assistant_conversations")
        .find_one(filter)
        .await?
        .ok_or_else(refused)?;
    let live =
        crate::services::assistant_nyxagent::live_turn(&current, Utc::now()).ok_or_else(refused)?;
    if !crate::services::supervisor_write::personal(&current)
        || !current.is_subagent()
        || live.execution_task.as_ref() != Some(task)
        || live
            .supervisor_source
            .as_deref()
            .is_none_or(|source| !same_supervisor_source(source, expected))
    {
        return Err(refused());
    }
    Ok(())
}

/// Runs only typed, server-authored decision events, never ordinary turns. The
/// normal turn lease and admission remain responsible for requester isolation.
pub(crate) async fn resume_turn(
    state: &crate::AppState,
    egress: &dyn super::super::super::platform_manager_contract::NyxAgentEgress,
    row: &AssistantConversation,
    credential: &crate::services::assistant_agent_credential_service::AssistantCredential,
) -> AppResult<Option<ResumeOutcome>> {
    if let Some(note) = Box::pin(super::v2::resume_voice(state, row)).await? {
        return Ok(Some(ResumeOutcome::Receipt(note)));
    }
    let Some(turn) = row.active_turn.as_ref() else {
        return Ok(None);
    };
    let decisions: Vec<_> = turn
        .events
        .iter()
        .filter_map(|event| {
            event
                .id
                .strip_prefix("execution-decision:")
                .map(|id| (event, id))
        })
        .collect();
    if decisions.is_empty() {
        return Ok(None);
    }
    if decisions
        .iter()
        .any(|(event, _)| event.kind != "permission_decided")
    {
        return Err(refused());
    }
    // A readback-only model input cannot replace notices for other coalesced
    // decisions. Multiple approved writes keep explicit per-occurrence status
    // and unverified readback; none silently chooses another write's target.
    let single_result = decisions.len() == 1 && turn.events.len() == 1;
    let ids: Vec<String> = decisions.iter().map(|(_, id)| (*id).to_owned()).collect();
    // Stored batch IDs are evidence only. They never add executable decisions
    // to this admitted wake; an incomplete recovery remains receipt-only.
    let mut batch = ids.clone();
    batch.sort();
    batch.dedup();
    if batch.len() > 64 {
        return Err(refused());
    }
    let typed_wake = turn.events.iter().all(|event| {
        event.kind == "permission_decided" && event.id.starts_with("execution-decision:")
    });
    let filter = assistant_selector(doc! {"_id":{"$in":&batch}, "user_id":&row.user_id,
    "execution_proposal.v1.conversation_id":&row.id,
    "execution_proposal.v1.key_id":&row.credential_api_key_id});
    let mut requests: Vec<ApprovalRequest> = state
        .db
        .collection::<ApprovalRequest>(REQUESTS)
        .find(filter.clone())
        .limit(64)
        .await?
        .try_collect()
        .await?;
    if requests.len() != batch.len() {
        return Err(missing());
    }
    let batch_bound = continuation::bind_batch(state, row, &requests, &batch, typed_wake).await?;
    if batch_bound {
        requests = state
            .db
            .collection::<ApprovalRequest>(REQUESTS)
            .find(filter)
            .limit(64)
            .await?
            .try_collect()
            .await?;
    }
    let mut notes = Vec::new();
    let mut specialist = None;
    let mut receipts = Vec::new();
    let mut withheld = Vec::new();
    let mut continuation_ready = batch_bound.then_some(true);
    let mut owner_source = None;
    let mut provider_notice = None;
    let mut reviews = presentation::ReviewNotes::default();
    let mut seen = std::collections::HashSet::new();
    for (event, id) in decisions {
        // Replayed wake IDs are the same receipt, never another operation.
        if !seen.insert(id) {
            continue;
        }
        let associated = |text: String| {
            if single_result {
                text
            } else {
                format!("Request {id}: {text}")
            }
        };
        let request = requests
            .iter()
            .find(|request| request.id == id)
            .ok_or_else(missing)?;
        let p = proposal(request)?;
        let version = AssistantVersion::of(request)?;
        let mut eligible = continuation::eligible(row, p, version);
        if version == AssistantVersion::V5 {
            require_durable_event(&state.db, row, turn, event, p).await?;
        }
        if turn.execution_task.as_ref().is_none_or(|t| {
            if version == AssistantVersion::V5 {
                return !super::super::super::supervisor_write::same_task(t, &p.task);
            }
            t.task != p.task.task
                || t.epoch != p.task.epoch
                || t.actor_user_id != p.task.actor_user_id
                || t.audience != p.task.audience
                || t.channel_source != p.task.channel_source
        }) {
            return Err(refused());
        }
        if p.task.channel_source.is_some() {
            super::super::super::execution_permission_channel::check(&state.db, &p.task).await?;
        }
        if request.status != "approved" {
            reviews.push(request, p);
            continue;
        }
        if p.state == "completed" {
            if version == AssistantVersion::V5 {
                if rejected(request, p) {
                    notes.push(associated(format!("{PROVIDER_REJECTION} No independent readback is verified in this recovery; the final row state is unconfirmed. Do not repeat the write.")));
                    continue;
                }
                notes.push(associated("The retained write has a provider receipt, but no independent readback is verified in this recovery. Do not repeat it; the final row state is unconfirmed.".into()));
                continue;
            }
            notes.push(associated(if matches!(p.contract.context.audience,Audience::Group{..}) {"The operation completed. Its private result is available on your permission card.".into()}else{result_for_human(state,request).await?.unwrap_or_else(||"The operation completed; its retained result is no longer available.".into())}));
            if p.owner_continuation.as_ref().is_some_and(|release| {
                matches!(release.version, 1 | 2)
                    && !release.blocked
                    && release.released_turn_id.is_none()
            }) {
                let receipt = continuation::Receipt {
                    id: id.into(),
                    version,
                    claim_id: p.claim_id.clone().ok_or_else(conflict)?,
                };
                if eligible
                    && p.owner_continuation
                        .as_ref()
                        .is_some_and(|release| release.version == 2)
                    && p.owner_continuation
                        .as_ref()
                        .and_then(|release| release.batch.as_ref())
                        == Some(&batch)
                    && request.activity_outcome.as_deref() == Some("completed")
                    && p.delivered_turn_id.is_none()
                    && p.result.is_some()
                {
                    let source = match owner_source {
                        Some(source) => source,
                        None => {
                            let source =
                                continuation::original_owner_source(&state.db, row).await?;
                            owner_source = Some(source);
                            source
                        }
                    };
                    if source {
                        receipts.push(receipt);
                    } else {
                        withheld.push(receipt);
                    }
                } else {
                    withheld.push(receipt);
                }
            }
            continue;
        }
        if p.state == "dispatched"
            && p.resume_turn_id.as_deref() == Some(turn.turn_id.as_str())
            && p.claimed_at
                .is_some_and(|at| at > Utc::now() - Duration::minutes(2))
        {
            notes.push(associated(
                "The approved operation is already being processed.".into(),
            ));
            continue;
        }
        if matches!(p.state.as_str(), "dispatched" | "unknown") {
            if p.failure_code.as_deref() == Some("provider_completion_pending") {
                notes.push(associated(if version == AssistantVersion::V5 {
                    format!("{PROVIDER_PENDING_NOTICE} The final row state is unconfirmed; a later observation cannot prove this attempt caused it.")
                } else { PROVIDER_PENDING_NOTICE.into() }));
                continue;
            }
            let known_rejection = version == AssistantVersion::V5
                && (rejected(request, p) || request.activity_outcome.as_deref() == Some("failed"));
            let failure = if known_rejection {
                if p.failure_code.as_deref() == Some("provider_application_rejected") {
                    "provider_application_rejected"
                } else {
                    "provider_http_rejected"
                }
            } else {
                "provider_outcome_unknown"
            };
            let mut session = state.db.client().start_session().await?;
            session.start_transaction().and_run((&state.db,id),move |session,(db,id)|Box::pin(async move {
                let result:AppResult<()>=async {
                    if db.collection::<Document>(REQUESTS).update_one(version.selector(doc!{"_id":*id,"execution_proposal.v1.state":{"$in":["dispatched","unknown"]}}),version.update(doc!{"$set":{"execution_proposal.v1.state":"unknown","execution_proposal.v1.failure_code":failure},"$unset":Slot::Assistant(version).unset(&[Field::Payload])})).session(&mut *session).await?.matched_count==1 {
                        lifecycle::retire_operations(db,session,&[(*id).to_owned()]).await?;
                    }
                    Ok(())
                }.await;
                super::super::super::api_key_mutation_service::transaction_result(result)
            })).await.map_err(super::super::super::api_key_mutation_service::map_transaction_error)?;
            notes.push(associated(if known_rejection {
                format!("{PROVIDER_REJECTION} The final row state is unknown and unconfirmed. A later observation cannot prove this attempt caused it. It will not be sent again.")
            } else if version == AssistantVersion::V5 {
                "The write outcome is unknown and its final row state is unconfirmed. A later observation cannot prove this attempt caused it. It will not be sent again.".into()
            } else { "The operation's outcome is unknown. It will not be sent again.".into() }));
            continue;
        }
        if !matches!(p.state.as_str(), "queued" | "preparing") {
            notes.push(associated(format!(
                "The operation was not sent ({}).",
                p.failure_code.as_deref().unwrap_or("unavailable")
            )));
            continue;
        }
        super::super::super::security_readiness_service::require_serving(&state.db, READINESS)
            .await?;
        version.require_serving(&state.db).await?;
        let live = async {
            tasks::check(&state.db, &p.task).await?;
            require_enrolled(&state.db, &row.user_id, None).await
        }
        .await;
        if let Err(error) = live {
            if !error.is_forbidden()
                && !matches!(
                    error,
                    AppError::Conflict(_) | AppError::NotFound(_) | AppError::AssistantTurnRequired
                )
            {
                return Err(error);
            }
            let mut session = state.db.client().start_session().await?;
            session.start_transaction().await?;
            lifecycle::cancel_in_session(
                &state.db,
                &mut session,
                doc! {"_id":id},
                "authority_or_task_unavailable",
            )
            .await?;
            session.commit_transaction().await?;
            notes.push(associated(
                "The operation was not sent because its authority or task is no longer available."
                    .into(),
            ));
            continue;
        }
        // This optional readiness read precedes the provider attempt. A failure
        // cannot discard a known provider success or strand its durable receipt.
        if eligible && continuation_ready.is_none() {
            continuation_ready = Some(
                crate::services::security_readiness_service::serving_report(
                    &state.db,
                    continuation::READINESS,
                )
                .await?
                .serving_ready,
            );
        }
        if eligible && continuation_ready == Some(true) {
            let source = match owner_source {
                Some(source) => source,
                None => {
                    let source = continuation::original_owner_source(&state.db, row).await?;
                    owner_source = Some(source);
                    source
                }
            };
            eligible = source;
        }
        if credential.api_key_id != row.credential_api_key_id || credential.raw_key.is_empty() {
            return Err(refused());
        }
        let raw = Field::Payload
            .open(&state.encryption_keys, request)
            .await?
            .ok_or_else(conflict)?;
        if raw.len() > MAX_PAYLOAD {
            return Err(conflict());
        }
        let effect: PreparedEffect = serde_json::from_slice(&raw).map_err(encoding)?;
        let claim_id = Uuid::new_v4().to_string();
        if !claim_preparation(&state.db, id, &claim_id, &turn.turn_id).await? {
            notes.push(associated(
                "The approved operation is already being processed.".into(),
            ));
            continue;
        }
        let resume = Resume {
            supervisor_assignment: p.report_to.is_some(),
            connected_wait: None,
            version: version.number(),
            source_credential_id: p.source_credential_id.clone(),
            request_id: id.into(),
            claim_id: claim_id.clone(),
            effect_id: p.effect_id.clone(),
            grant_id: p.grant_id.clone().ok_or_else(conflict)?,
            prepared_digest: p.prepared_digest.clone(),
            authority_digest: p.authority_digest.clone(),
            effect,
            independent_card_id: p.independent_card_id.clone(),
        };
        let readback = if typed_wake && single_result && version == AssistantVersion::V5 {
            specialist_readback(row, p, &resume)
        } else {
            None
        };
        let provider_path = (version == AssistantVersion::V5).then(|| resume.effect.path.clone());
        let outcome = egress.resume_execution(row, resume).await;
        let current = state
            .db
            .collection::<ApprovalRequest>(REQUESTS)
            .find_one(version.selector(doc! {"_id":id,"execution_proposal.v1.claim_id":&claim_id}))
            .await?
            .ok_or_else(conflict)?;
        if proposal(&current)?.state == "not_sent"
            && proposal(&current)?.failure_code.as_deref() == Some("operation_not_sent")
        {
            notes.push(associated(
                "The node was unavailable. Nothing was sent.".into(),
            ));
            continue;
        }
        let dispatched = proposal(&current)?;
        let sent = dispatched.state == "dispatched";
        let fresh_readback = sent
            && dispatched.resume_turn_id.as_deref() == Some(turn.turn_id.as_str())
            && dispatched.effect_id == p.effect_id
            && dispatched.prepared_digest == p.prepared_digest
            && dispatched.authority_digest == p.authority_digest;
        let mut independent_request = None;
        let mut independent_card = None;
        let mut private_result = None;
        let mut activity_outcome = None;
        let (state_name, text, code) = match outcome {
            Ok(response) if sent => {
                let status = response.status();
                if status == axum::http::StatusCode::ACCEPTED {
                    activity_outcome = Some("pending");
                    (
                        "unknown",
                        PROVIDER_PENDING_NOTICE.into(),
                        Some("provider_completion_pending"),
                    )
                } else {
                    match tokio::time::timeout(std::time::Duration::from_secs(120),axum::body::to_bytes(response.into_body(),MAX_PAYLOAD)).await {
                    Ok(Ok(bytes))=>{
                        if let Some(path) = &provider_path {
                            use crate::services::supervisor_write::{ProviderResult, provider_result};
                            match provider_result(path, status.as_u16(), &bytes) {
                                ProviderResult::Rejected => {
                                    activity_outcome = Some("failed");
                                    let code = if status.is_success() {"provider_application_rejected"} else {"provider_http_rejected"};
                                    let text = format!("{PROVIDER_REJECTION} HTTP {}. The final row state is unconfirmed; do not repeat the write.", status.as_u16());
                                    private_result = Some(text.clone());
                                    ("completed", text, Some(code))
                                }
                                ProviderResult::Unconfirmed => {
                                    let text = "The provider response did not confirm this exact record update. The write outcome is unknown and its final row state is unconfirmed. Do not repeat the write.".to_owned();
                                    private_result = Some(text.clone());
                                    ("unknown", text, Some("provider_application_unconfirmed"))
                                }
                                ProviderResult::Succeeded => {
                                    activity_outcome = Some("completed");
                                    let text = format!("Operation finished (HTTP {}). A provider receipt was received; an independent read is required before reporting the final row state.", status.as_u16());
                                    private_result = Some(text.clone());
                                    ("completed", text, None)
                                }
                            }
                        } else {
                        let text=String::from_utf8_lossy(&bytes);
                        let text=match serde_json::from_slice::<serde_json::Value>(&bytes) {
                            Ok(mut value)=>{redact(&mut value);value.to_string()},
                            Err(_)=>text.into_owned(),
                        };
                        let text = crate::services::assistant_agent_credential_service::redact_key(
                            &text, credential.raw_key.as_str(),
                        );
                        let result=format!("Operation finished (HTTP {}).\n{}",status.as_u16(),text.chars().take(16_384).collect::<String>());
                        private_result=Some(result.clone());
                        activity_outcome = Some(if status.is_success() {"completed"}else{"failed"});
                        ("completed",if row.group_id.is_some() || row.collaboration.is_some(){format!("Operation finished (HTTP {}). The private result is available on the requester's permission card.",status.as_u16())}else{result},None)
                        }
                    },
                    _=>("unknown","The response was incomplete or unavailable. The operation will not be sent again.".into(),Some("response_unavailable")),
                }
                }
            }
            Ok(response) => {
                let bytes = axum::body::to_bytes(response.into_body(), MAX_PAYLOAD)
                    .await
                    .unwrap_or_default();
                let value = serde_json::from_slice::<serde_json::Value>(&bytes).unwrap_or_default();
                independent_card = value
                    .get("acknowledgement_id")
                    .or_else(|| value.pointer("acknowledgement/id"))
                    .or_else(|| value.get("card_id"))
                    .and_then(|v| v.as_str())
                    .map(str::to_owned);
                if independent_card.is_some() {
                    ("waiting","The permission is saved. The operation is waiting for a separate required review; nothing was sent.".into(),Some("independent_approval_required"))
                } else {
                    (
                        "not_sent",
                        "The operation was not sent: execution admission was unavailable.".into(),
                        Some("admission_unavailable"),
                    )
                }
            }
            Err(_) if sent => (
                "unknown",
                "The operation's outcome is unknown. It will not be sent again.".into(),
                Some("provider_outcome_unknown"),
            ),
            Err(error) => {
                let request_id = match error {
                    AppError::ApprovalRequired { request_id }
                    | AppError::ApprovalFailed { request_id, .. } => Some(request_id),
                    _ => None,
                };
                // A timed-out legacy waiter is not evidence of a pending card.
                // Preserve the effect only while that exact independent review
                // still exists and is live; expired/denied reviews are terminal.
                if let Some(request_id) = request_id
                    && state
                        .db
                        .collection::<ApprovalRequest>(REQUESTS)
                        .find_one(doc! {
                            "_id":&request_id,"execution_proposal":null,"status":"pending",
                            "expires_at":{"$gt":bson::DateTime::now()}
                        })
                        .await?
                        .is_some()
                {
                    independent_request = Some(request_id);
                    ("waiting","The permission is saved. The operation is waiting for a separate required review; nothing was sent.".into(),Some("independent_approval_required"))
                } else {
                    ("not_sent","The operation was not sent because its authority or source is no longer available.".into(),Some("authority_or_operation_unavailable"))
                }
            }
        };
        let (result, marker) = Slot::Assistant(version)
            .seal(
                &state.encryption_keys,
                id,
                &request.user_id,
                Field::Result,
                private_result.as_deref().unwrap_or(&text).as_bytes(),
            )
            .await?;
        let mut unset = if state_name == "waiting" {
            Document::new()
        } else {
            Slot::Assistant(version).unset(&[Field::Payload])
        };
        let mut set = doc! {
            "activity_outcome": activity_outcome,
            "execution_proposal.v1.state": state_name,
            "execution_proposal.v1.failure_code": code,
            "execution_proposal.v1.result": binary(result),
            "execution_proposal.v1.independent_request_id": independent_request,
            "execution_proposal.v1.independent_card_id": independent_card,
        };
        let successful = state_name == "completed" && activity_outcome == Some("completed");
        let release_pending = typed_wake
            && eligible
            && successful
            && continuation_ready == Some(true)
            && p.owner_continuation.as_ref().is_some_and(|marker| {
                marker.version == 2
                    && !marker.blocked
                    && marker.released_turn_id.is_none()
                    && marker.batch.as_ref() == Some(&batch)
            });
        if p.owner_continuation.is_some() && !release_pending && state_name != "waiting" {
            set.insert("execution_proposal.v1.owner_continuation.blocked", true);
        }
        if let Some(version) = marker {
            set.insert(
                "execution_proposal.v1.result_context_version",
                i32::from(version),
            );
        } else {
            unset.insert("execution_proposal.v1.result_context_version", "");
        }
        let update = doc! {"$set": set, "$unset": unset};
        let mut session = state.db.client().start_session().await?;
        session.start_transaction().and_run((&state.db,id,&claim_id,&update,state_name),move |session,(db,id,claim,update,state_name)|Box::pin(async move {
            let result:AppResult<()>=async {
                if db.collection::<Document>(REQUESTS).update_one(version.selector(doc!{"_id":*id,"execution_proposal.v1.claim_id":*claim,"execution_proposal.v1.delivery_abandoned":{"$ne":true},"execution_proposal.v1.state":{"$in":["preparing","dispatched"]}}),version.update((*update).clone())).session(&mut *session).await?.matched_count!=1{return Err(uncertain());}
                if *state_name!="waiting" {lifecycle::retire_operations(db,session,&[(*id).to_owned()]).await?;}
                Ok(())
            }.await;
            super::super::super::api_key_mutation_service::transaction_result(result)
        })).await.map_err(super::super::super::api_key_mutation_service::map_transaction_error)?;
        #[cfg(test)]
        if release_pending
            && continuation::FAIL_AFTER_RECEIPT
                .try_with(|fail| *fail)
                .unwrap_or(false)
        {
            let _ = continuation::RECEIPT_INTERRUPTION.try_with(|observed| {
                observed
                    .lock()
                    .unwrap()
                    .push(continuation::ReceiptInterruption {
                        request_id: id.into(),
                        turn_id: turn.turn_id.clone(),
                        claim_id: claim_id.clone(),
                    });
            });
            return Err(AppError::Internal("Injected receipt interruption".into()));
        }
        if release_pending {
            receipts.push(continuation::Receipt {
                id: id.into(),
                version,
                claim_id,
            });
        }
        if matches!(state_name, "completed" | "unknown")
            && fresh_readback
            && let Some((mut instruction, source)) = readback
        {
            if state_name == "unknown" {
                instruction.push_str(" The write outcome is unknown. Report any current state as an observation only; it does not prove this write attempt caused it.");
            }
            if activity_outcome == Some("failed") {
                instruction.push_str(&format!(" {PROVIDER_REJECTION} Preserve the rejection in your report to NyxBot; do not infer a successful write from the read."));
                provider_notice = Some(PROVIDER_REJECTION);
            }
            // This check follows the durable receipt commit. A readiness or
            // authority read failure cannot discard the already known effect.
            require_specialist_source(&state.db, row, &source).await?;
            specialist = Some(instruction);
        }
        if version == AssistantVersion::V5 && sent && activity_outcome == Some("failed") {
            notes.push(associated(format!("{PROVIDER_REJECTION} The independent readback is unverified in this receipt and the final row state is unconfirmed; do not repeat the write.")));
        } else if !single_result && version == AssistantVersion::V5 && sent {
            let outcome = activity_outcome.unwrap_or(state_name);
            let qualification = if state_name == "unknown" {
                " An observation cannot prove this attempt caused the state."
            } else {
                ""
            };
            notes.push(format!("Request {id}: the write attempt is {outcome}; independent readback is unverified and the final row state is unconfirmed. Do not repeat the write.{qualification}"));
        } else {
            notes.push(associated(text));
        }
    }
    notes.extend(reviews.render());
    let text = notes.join("\n\n");
    if typed_wake && withheld.is_empty() && receipts.len() == seen.len() && !receipts.is_empty() {
        return Ok(Some(ResumeOutcome::Continue(
            continuation::prepare(state, row, receipts, text).await?,
        )));
    }
    receipts.extend(withheld);
    if !receipts.is_empty() {
        continuation::withhold(&state.db, &receipts).await?;
    }
    if let Some(instruction) = specialist {
        return Ok(Some(ResumeOutcome::SpecialistReadback {
            instruction,
            provider_notice,
        }));
    }
    Ok(Some(ResumeOutcome::Receipt(text)))
}
/// Called only after first-party-human ownership checks. Public projections
/// never decrypt the provider receipt.
pub(crate) async fn result_for_human(
    state: &crate::AppState,
    row: &ApprovalRequest,
) -> AppResult<Option<String>> {
    if row
        .execution_proposal
        .as_ref()
        .is_some_and(|p| p.version == 2)
    {
        return Ok(super::v2::decrypt_result(state, row).await?.map(|r| {
            format!(
                "HTTP {}{}: {}",
                r.status,
                if r.partial { " (partial response)" } else { "" },
                r.body
            )
        }));
    }
    if proposal(row).is_err() {
        return Ok(None);
    }
    let Some(raw) = Field::Result.open(&state.encryption_keys, row).await? else {
        return Ok(None);
    };
    if raw.len() > MAX_PAYLOAD {
        return Err(conflict());
    }
    Ok(Some(String::from_utf8_lossy(&raw).into_owned()))
}

/// Reclaiming a stale preparer also releases its matching one-operation
/// reservation. Both updates serialize against the old preparer's send CAS:
/// once dispatched, neither the proposal nor the grant can be reset here.
async fn claim_preparation(db: &Database, id: &str, claim: &str, turn: &str) -> AppResult<bool> {
    let mut session = db.client().start_session().await?;
    session.start_transaction().and_run((db,id,claim,turn),|session,(db,id,claim,turn)|Box::pin(async move {
        let result:AppResult<bool>=async {
            let filter=doc!{"_id":*id,"status":"approved","$or":[
                {"execution_proposal.v1.state":"queued"},
                {"execution_proposal.v1.state":"preparing","execution_proposal.v1.claimed_at":{"$lt":bson::DateTime::from_chrono(Utc::now()-Duration::minutes(2))}}
            ]};
            let Some(row)=db.collection::<ApprovalRequest>(REQUESTS).find_one(assistant_selector(filter.clone())).session(&mut *session).await? else {return Ok(false)};
            let p=proposal(&row)?;
            let version=AssistantVersion::of(&row)?;
            version.require_serving(db).await?;
            if db.collection::<Document>(REQUESTS).update_one(version.selector(filter),version.update(doc!{"$set":{
                "execution_proposal.v1.state":"preparing","execution_proposal.v1.claim_id":*claim,
                "execution_proposal.v1.resume_turn_id":*turn,"execution_proposal.v1.claimed_at":bson::DateTime::now()
            }})).session(&mut *session).await?.matched_count!=1{return Ok(false);}
            if p.state=="preparing" && let Some(grant)=&p.grant_id {
                db.collection::<Document>("approval_grants").update_one(doc!{
                    "_id":grant,"revoked":false,"approval_request_id":*id,
                    "execution_use.version":VERSION as i32,"execution_use.state":"claimed",
                    "execution_use.operation_id":&p.effect_id,"execution_use.prepared_digest":&p.prepared_digest,
                    "execution_use.dispatched_at":null,
                },doc!{"$unset":{"execution_use":""}}).session(&mut *session).await?;
            }
            Ok(true)
        }.await;
        super::super::super::api_key_mutation_service::transaction_result(result)
    })).await.map_err(super::super::super::api_key_mutation_service::map_transaction_error)
}

#[cfg(test)]
mod specialist_source_tests {
    use super::*;
    use crate::models::{
        approval_request::SupervisorSource, execution_permission_channel::ChannelSource,
    };

    fn source() -> SupervisorSource {
        SupervisorSource {
            channel: ChannelSource {
                version: 4,
                event_id: "root-event".into(),
                actor_user_id: "owner".into(),
                agent_id: "nyxbot".into(),
                conversation_id: "owner-home".into(),
                key_id: "owner-key".into(),
                original_turn_id: "root-turn".into(),
                channel_id: "owner-channel".into(),
                thread_id: "private-chat".into(),
                settings_thread_id: "private-chat".into(),
                channel_revision: 0,
                thread_revision: 0,
                audience_id: "owner-audience".into(),
                audience_label: "Telegram · Owner bot · original chat/thread".into(),
                policy_digest: "root-event-policy".into(),
                expires_at: Utc::now() + Duration::minutes(20),
            },
            assigning_turn: "assigning-event-turn".into(),
            reset_at: None,
        }
    }

    #[test]
    fn supervisor_readback_source_proofs_allow_labels_and_distinct_event_identity() {
        let root = source();
        let mut assignment = root.clone();
        assignment.channel.audience_label.clear();
        assert!(same_owner_chat(&root.channel, &assignment.channel));
        assert!(same_supervisor_source(&root, &assignment));
        assignment.channel.event_id = "later-owner-event".into();
        assignment.channel.original_turn_id = "later-owner-turn".into();
        assignment.channel.policy_digest = "later-event-policy".into();
        assignment.channel.expires_at += Duration::minutes(1);
        assert!(same_owner_chat(&root.channel, &assignment.channel));
        assert!(!same_supervisor_source(&root, &assignment));
        // A restored assignment still binds its own complete event proof.
        let expected = assignment.clone();
        assignment.channel.audience_label = "another display label".into();
        assert!(same_supervisor_source(&expected, &assignment));
        assignment.assigning_turn = "different-assigning-turn".into();
        assert!(!same_supervisor_source(&expected, &assignment));
        assignment = expected.clone();
        assignment.reset_at = Some(Utc::now());
        assert!(!same_supervisor_source(&expected, &assignment));
    }

    #[test]
    fn supervisor_readback_source_proofs_refuse_authorization_identity_drift() {
        let source = source();
        let controls: &[fn(&mut ChannelSource)] = &[
            |s| s.version += 1,
            |s| s.actor_user_id.push('x'),
            |s| s.agent_id.push('x'),
            |s| s.conversation_id.push('x'),
            |s| s.key_id.push('x'),
            |s| s.channel_id.push('x'),
            |s| s.thread_id.push('x'),
            |s| s.settings_thread_id.push('x'),
            |s| s.channel_revision += 1,
            |s| s.thread_revision += 1,
            |s| s.audience_id.push('x'),
        ];
        for change in controls {
            let mut changed = source.clone();
            change(&mut changed.channel);
            assert!(!same_owner_chat(&source.channel, &changed.channel));
            assert!(!same_supervisor_source(&source, &changed));
        }
        for change in [
            (|s: &mut ChannelSource| s.event_id.push('x')) as fn(&mut ChannelSource),
            |s| s.original_turn_id.push('x'),
            |s| s.policy_digest.push('x'),
            |s| s.expires_at += Duration::seconds(1),
        ] {
            let mut changed = source.clone();
            change(&mut changed.channel);
            assert!(same_owner_chat(&source.channel, &changed.channel));
            assert!(!same_supervisor_source(&source, &changed));
        }
    }
}

use super::*;

const PROVIDER_PENDING_NOTICE: &str = "The provider accepted the operation (HTTP 202); completion is pending and unverified. It will not be sent again.";

/// In-process capability created only after the stored source and live turn match.
/// No public request deserializes this object.
#[derive(Clone)]
pub struct Resume {
    pub(super) turn_authority: Option<super::batch::TurnAuthority>,
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
        if let Some(authority) = &self.turn_authority {
            authority.fence(db, session).await?;
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
    let ids: Vec<String> = turn
        .events
        .iter()
        .filter_map(|e| e.id.strip_prefix("execution-decision:").map(str::to_owned))
        .collect();
    if ids.is_empty() {
        return Ok(None);
    }
    // Stored batch IDs are evidence only. They never add executable decisions
    // to this admitted wake; an incomplete recovery remains receipt-only.
    let mut batch = ids.clone();
    batch.sort();
    batch.dedup();
    if batch.len() > 64 {
        return Err(refused());
    }
    let typed_wake = super::batch::typed_wake(&turn.events);
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
    if let Some(text) = super::local::expired_channel(state, row, credential, &batch).await? {
        return Ok(Some(ResumeOutcome::Receipt(text)));
    }
    let continuation_ready =
        continuation::bind_batch(state, row, &requests, &batch, typed_wake).await?;
    if continuation_ready == Some(true) {
        requests = state
            .db
            .collection::<ApprovalRequest>(REQUESTS)
            .find(filter)
            .limit(64)
            .await?
            .try_collect()
            .await?;
    }
    #[cfg(test)]
    if continuation::FAIL_BEFORE_SEND
        .try_with(|fail| *fail)
        .unwrap_or(false)
    {
        return Err(AppError::Internal("Injected pre-send interruption".into()));
    }
    let mut notes = Vec::new();
    let mut owner_source = None;
    let mut reviews = presentation::ReviewNotes::default();
    let mut seen = std::collections::HashSet::new();
    for id in ids.iter().map(String::as_str) {
        // Replayed wake IDs are the same receipt, never another operation.
        if !seen.insert(id) {
            continue;
        }
        let request = requests
            .iter()
            .find(|request| request.id == id)
            .ok_or_else(missing)?;
        let p = proposal(request)?;
        let version = AssistantVersion::of(request)?;
        let mut eligible = continuation::eligible(row, p, version);
        if turn.execution_task.as_ref().is_none_or(|t| {
            t.task != p.task.task
                || t.epoch != p.task.epoch
                || t.actor_user_id != p.task.actor_user_id
                || t.audience != p.task.audience
                || t.channel_source != p.task.channel_source
        }) {
            return Err(refused());
        }
        // A still-owned admitted turn may settle this exact expired operation
        // locally. Its expired channel remains unusable for execution/delivery.
        let local_expired = eligible
            && request.status == "approved"
            && matches!(p.state.as_str(), "queued" | "preparing")
            && turn.execution_task.as_ref() == Some(&p.task)
            && p.task.hard_deadline <= Utc::now();
        if p.task.channel_source.is_some() && !local_expired {
            super::super::super::execution_permission_channel::check(&state.db, &p.task).await?;
        }
        if request.status != "approved" {
            reviews.push(request, p);
            continue;
        }
        if p.state == "completed" {
            notes.push(if matches!(p.contract.context.audience,Audience::Group{..}) {"The operation completed. Its private result is available on your permission card.".into()}else{result_for_human(state,request).await?.unwrap_or_else(||"The operation completed; its retained result is no longer available.".into())});
            continue;
        }
        if p.state == "dispatched"
            && p.resume_turn_id.as_deref() == Some(turn.turn_id.as_str())
            && p.claimed_at
                .is_some_and(|at| at > Utc::now() - Duration::minutes(2))
        {
            notes.push("The approved operation is already being processed.".into());
            continue;
        }
        if matches!(p.state.as_str(), "dispatched" | "unknown") {
            if p.failure_code.as_deref() == Some("provider_completion_pending") {
                notes.push(PROVIDER_PENDING_NOTICE.into());
                continue;
            }
            if p.state == "unknown" {
                notes.push("The operation's outcome is unknown. It will not be sent again.".into());
                continue;
            }
            let mut session = state.db.client().start_session().await?;
            session.start_transaction().and_run((&state.db,id,row),move |session,(db,id,row)|Box::pin(async move {
                let result:AppResult<()>=async {
                    super::batch::fence_turn(db, session, row).await?;
                    if db.collection::<Document>(REQUESTS).update_one(version.selector(doc!{"_id":*id,"execution_proposal.v1.state":{"$in":["dispatched","unknown"]}}),version.update(doc!{"$set":{"execution_proposal.v1.state":"unknown","execution_proposal.v1.failure_code":"provider_outcome_unknown"},"$unset":Slot::Assistant(version).unset(&[Field::Payload])})).session(&mut *session).await?.matched_count==1 {
                        lifecycle::retire_operations(db,session,&[(*id).to_owned()]).await?;
                    }
                    Ok(())
                }.await;
                super::super::super::api_key_mutation_service::transaction_result(result)
            })).await.map_err(super::super::super::api_key_mutation_service::map_transaction_error)?;
            notes.push("The operation's outcome is unknown. It will not be sent again.".into());
            continue;
        }
        if !matches!(p.state.as_str(), "queued" | "preparing") {
            notes.push(format!(
                "The operation was not sent ({}).",
                p.failure_code.as_deref().unwrap_or("unavailable")
            ));
            continue;
        }
        if continuation::owner_task(row).is_some()
            && turn
                .execution_task
                .as_ref()
                .is_none_or(|task| *task != p.task)
        {
            return Err(refused());
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
            let cancel_filter = doc! {
                "_id":id,"user_id":&row.user_id,"status":"approved",
                "execution_proposal.v1.conversation_id":&row.id,
                "execution_proposal.v1.key_id":&row.credential_api_key_id,
                "execution_proposal.v1.state":&p.state,
                "execution_proposal.v1.claim_id":&p.claim_id,
                "execution_proposal.v1.effect_id":&p.effect_id,
                "execution_proposal.v1.grant_id":&p.grant_id,
                "execution_proposal.v1.prepared_digest":&p.prepared_digest,
            };
            let mut session = state.db.client().start_session().await?;
            let cancelled = session
                .start_transaction()
                .and_run(
                    (&state.db, row, &p.task, &cancel_filter),
                    |session, (db, row, task, filter)| {
                        Box::pin(async move {
                            let result: AppResult<bool> = async {
                                super::batch::fence_turn(db, session, row).await?;
                                match tasks::fence(db, session, task).await {
                                    Ok(()) => return Ok(false),
                                    Err(error)
                                        if error.is_forbidden()
                                            || matches!(
                                                error,
                                                AppError::Conflict(_)
                                                    | AppError::NotFound(_)
                                                    | AppError::AssistantTurnRequired
                                            ) => {}
                                    Err(error) => return Err(error),
                                }
                                if continuation::owner_task(row).is_some() {
                                    super::batch::fence_original_task(db, session, row).await?;
                                }
                                if db
                                    .collection::<Document>(REQUESTS)
                                    .find_one(assistant_selector((*filter).clone()))
                                    .projection(doc! {"_id":1})
                                    .session(&mut *session)
                                    .await?
                                    .is_none()
                                {
                                    return Ok(false);
                                }
                                lifecycle::cancel_in_session(
                                    db,
                                    session,
                                    (*filter).clone(),
                                    "authority_or_task_unavailable",
                                )
                                .await?;
                                Ok(true)
                            }
                            .await;
                            super::super::super::api_key_mutation_service::transaction_result(
                                result,
                            )
                        })
                    },
                )
                .await
                .map_err(super::super::super::api_key_mutation_service::map_transaction_error)?;
            notes.push(if cancelled {
                "The operation was not sent because its authority or task is no longer available."
            } else {
                "The operation requires a fresh status check; no new operation was sent."
            }.into());
            continue;
        }
        // Optional continuation readiness is resolved only for a marked root
        // by bind_batch. An authentic legacy retained effect keeps its send and
        // receipt when that optional dependency is unavailable.
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
        #[cfg(test)]
        if let Ok(hook) = continuation::BEFORE_EFFECT_CLAIM.try_with(Clone::clone) {
            hook(state.db.clone(), request.clone()).await;
        }
        if !claim_preparation(&state.db, id, &claim_id, row).await? {
            notes.push("The approved operation is already being processed.".into());
            continue;
        }
        let resume = Resume {
            turn_authority: Some(super::batch::TurnAuthority::capture(row)?),
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
            notes.push("The node was unavailable. Nothing was sent.".into());
            continue;
        }
        let sent = proposal(&current)?.state == "dispatched";
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
                        activity_outcome = Some(if status.is_success() { "completed" } else { "failed" });
                        ("completed",if row.group_id.is_some() || row.collaboration.is_some(){format!("Operation finished (HTTP {}). The private result is available on the requester's permission card.",status.as_u16())}else{result},None)
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
        if p.owner_continuation.as_ref().is_some_and(|proof| {
            !typed_wake
                || !eligible
                || !successful
                || (proof.version == 0 && continuation_ready != Some(true))
        }) {
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
        session.start_transaction().and_run((&state.db,id,&claim_id,&update,state_name,sent,row),move |session,(db,id,claim,update,state_name,sent,row)|Box::pin(async move {
            let result:AppResult<()>=async {
                if !*sent { super::batch::fence_turn(db, session, row).await?; }
                if db.collection::<Document>(REQUESTS).update_one(version.selector(doc!{"_id":*id,"execution_proposal.v1.claim_id":*claim,"execution_proposal.v1.delivery_abandoned":{"$ne":true},"execution_proposal.v1.state":if *sent {"dispatched"} else {"preparing"}}),version.update((*update).clone())).session(&mut *session).await?.matched_count!=1{return Err(uncertain());}
                if *state_name!="waiting" {lifecycle::retire_operations(db,session,&[(*id).to_owned()]).await?;}
                Ok(())
            }.await;
            super::super::super::api_key_mutation_service::transaction_result(result)
        })).await.map_err(super::super::super::api_key_mutation_service::map_transaction_error)?;
        #[cfg(test)]
        if successful
            && continuation::FAIL_AFTER_RECEIPT
                .try_with(|fail| *fail)
                .unwrap_or(false)
        {
            return Err(AppError::Internal("Injected receipt interruption".into()));
        }
        notes.push(text);
    }
    notes.extend(reviews.render());
    let text = notes.join("\n\n");
    if let Some(continuation) = super::batch::reconcile(state, row, &batch, typed_wake).await? {
        return Ok(Some(ResumeOutcome::Continue(continuation)));
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
async fn claim_preparation(
    db: &Database,
    id: &str,
    claim: &str,
    row: &AssistantConversation,
) -> AppResult<bool> {
    let turn = &row.active_turn.as_ref().ok_or_else(refused)?.turn_id;
    let mut session = db.client().start_session().await?;
    session.start_transaction().and_run((db,id,claim,turn,row),|session,(db,id,claim,turn,row)|Box::pin(async move {
        let result:AppResult<bool>=async {
            super::batch::fence_turn(db, session, row).await?;
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

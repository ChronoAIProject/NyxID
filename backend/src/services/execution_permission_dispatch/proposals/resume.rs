use super::*;

/// In-process capability created only after the stored source and live turn match.
/// No public request deserializes this object.
#[derive(Clone)]
pub struct Resume {
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
        version.require(db).await?;
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
) -> AppResult<Option<String>> {
    if let Some(note) = Box::pin(super::v2::resume_voice(state, row)).await? {
        return Ok(Some(note));
    }
    let Some(turn) = row.active_turn.as_ref() else {
        return Ok(None);
    };
    let ids: Vec<_> = turn
        .events
        .iter()
        .filter_map(|e| e.id.strip_prefix("execution-decision:"))
        .collect();
    if ids.is_empty() {
        return Ok(None);
    }
    let mut notes = Vec::new();
    for id in ids {
        let request=state.db.collection::<ApprovalRequest>(REQUESTS).find_one(assistant_selector(doc!{"_id":id,"user_id":&row.user_id,"execution_proposal.v1.conversation_id":&row.id,"execution_proposal.v1.key_id":&row.credential_api_key_id})).await?.ok_or_else(missing)?;
        let p = proposal(&request)?;
        let version = AssistantVersion::of(&request)?;
        if turn.execution_task.as_ref().is_none_or(|t| {
            t.task != p.task.task
                || t.epoch != p.task.epoch
                || t.actor_user_id != p.task.actor_user_id
                || t.audience != p.task.audience
        }) {
            return Err(refused());
        }
        if request.status != "approved" {
            notes.push("The execution permission was not approved. Nothing was sent.".into());
            continue;
        }
        if p.state == "completed" {
            notes.push(if matches!(p.contract.context.audience,Audience::Group{..}) {"The operation completed. Its private result is available on your permission card.".into()}else{result_for_human(state,&request).await?.unwrap_or_else(||"The operation completed; its retained result is no longer available.".into())});
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
            let mut session = state.db.client().start_session().await?;
            session.start_transaction().and_run((&state.db,id),move |session,(db,id)|Box::pin(async move {
                let result:AppResult<()>=async {
                    if db.collection::<Document>(REQUESTS).update_one(version.selector(doc!{"_id":*id,"execution_proposal.v1.state":{"$in":["dispatched","unknown"]}}),version.update(doc!{"$set":{"execution_proposal.v1.state":"unknown","execution_proposal.v1.failure_code":"provider_outcome_unknown"},"$unset":{"execution_proposal.v1.payload":""}})).session(&mut *session).await?.matched_count==1 {
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
        super::super::super::security_readiness_service::require(&state.db, READINESS).await?;
        version.require(&state.db).await?;
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
            notes.push(
                "The operation was not sent because its authority or task is no longer available."
                    .into(),
            );
            continue;
        }
        let payload = p.payload.as_ref().ok_or_else(conflict)?;
        let raw = Zeroizing::new(
            state
                .encryption_keys
                .decrypt_with_aad(&payload.bytes, aad(id).as_bytes())
                .await?,
        );
        if raw.len() > MAX_PAYLOAD {
            return Err(conflict());
        }
        let effect: PreparedEffect = serde_json::from_slice(&raw).map_err(encoding)?;
        let claim_id = Uuid::new_v4().to_string();
        if !claim_preparation(&state.db, id, &claim_id, &turn.turn_id).await? {
            notes.push("The approved operation is already being processed.".into());
            continue;
        }
        let resume = Resume {
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
        let sent = proposal(&current)?.state == "dispatched";
        let mut independent_request = None;
        let mut independent_card = None;
        let mut private_result = None;
        let (state_name, text, code) = match outcome {
            Ok(response) if sent => {
                let status = response.status();
                match tokio::time::timeout(std::time::Duration::from_secs(120),axum::body::to_bytes(response.into_body(),MAX_PAYLOAD)).await {
                    Ok(Ok(bytes))=>{
                        let text=String::from_utf8_lossy(&bytes);
                        let text=match serde_json::from_slice::<serde_json::Value>(&bytes) {
                            Ok(mut value)=>{redact(&mut value);value.to_string()},
                            Err(_)=>text.into_owned(),
                        };
                        let result=format!("Operation finished (HTTP {}).\n{}",status.as_u16(),text.chars().take(16_384).collect::<String>());
                        private_result=Some(result.clone());
                        ("completed",if row.group_id.is_some() || row.collaboration.is_some(){format!("Operation finished (HTTP {}). The private result is available on the requester's permission card.",status.as_u16())}else{result},None)
                    },
                    _=>("unknown","The response was incomplete or unavailable. The operation will not be sent again.".into(),Some("response_unavailable")),
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
        let result = state
            .encryption_keys
            .encrypt_with_aad(
                private_result.as_deref().unwrap_or(&text).as_bytes(),
                format!("{}:result", aad(id)).as_bytes(),
            )
            .await?;
        let mut update = doc! {"$set":{"execution_proposal.v1.state":state_name,"execution_proposal.v1.failure_code":code,"execution_proposal.v1.result":binary(result),"execution_proposal.v1.independent_request_id":independent_request,"execution_proposal.v1.independent_card_id":independent_card}};
        if state_name != "waiting" {
            update.insert("$unset", doc! {"execution_proposal.v1.payload":""});
        }
        let mut session = state.db.client().start_session().await?;
        session.start_transaction().and_run((&state.db,id,&claim_id,&update,state_name),move |session,(db,id,claim,update,state_name)|Box::pin(async move {
            let result:AppResult<()>=async {
                if db.collection::<Document>(REQUESTS).update_one(version.selector(doc!{"_id":*id,"execution_proposal.v1.claim_id":*claim,"execution_proposal.v1.delivery_abandoned":{"$ne":true},"execution_proposal.v1.state":{"$in":["preparing","dispatched"]}}),version.update((*update).clone())).session(&mut *session).await?.matched_count!=1{return Err(uncertain());}
                if *state_name!="waiting" {lifecycle::retire_operations(db,session,&[(*id).to_owned()]).await?;}
                Ok(())
            }.await;
            super::super::super::api_key_mutation_service::transaction_result(result)
        })).await.map_err(super::super::super::api_key_mutation_service::map_transaction_error)?;
        notes.push(text);
    }
    Ok(Some(notes.join("\n\n")))
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
    let Ok(p) = proposal(row) else {
        return Ok(None);
    };
    let Some(result) = p.result.as_ref() else {
        return Ok(None);
    };
    let raw = Zeroizing::new(
        state
            .encryption_keys
            .decrypt_with_aad(&result.bytes, format!("{}:result", aad(&row.id)).as_bytes())
            .await?,
    );
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
            version.require(db).await?;
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

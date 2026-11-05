use super::*;
use axum::response::Response;

pub(crate) fn attach_resume(auth: &mut crate::mw::auth::AuthUser, resume: Resume) -> AppResult<()> {
    let old = auth.execution_permission.as_ref().ok_or_else(refused)?;
    let key = auth.api_key_id.as_deref().ok_or_else(refused)?;
    if auth.api_key_credential_id != resume.source_credential_id {
        return Err(refused());
    }
    let mut context = Arc::try_unwrap(
        RequestContext::enrolled(
            Some(VERSION),
            &old.actor,
            key,
            auth.assistant_chat.as_deref(),
        )
        .ok_or_else(refused)?,
    )
    .map_err(|_| refused())?;
    context.operation_id = resume.effect_id.clone();
    if let Some(k) = old.authenticated_key.get() {
        let _ = context.authenticated_key.set(k.clone());
    }
    if let Some(k) = old.source_credential.get() {
        let _ = context.source_credential.set(k.clone());
    }
    if let Some(v) = resume.effect.generated_correlation.as_ref() {
        context.bind_generated_correlation(v);
    }
    context.descendant = old.descendant.clone();
    context.resume = Some(Arc::new(resume));
    context.enable_authoring();
    let context = Arc::new(context);
    if let Some(chat) = &auth.assistant_chat {
        let mut chat = (**chat).clone();
        chat.execution_permission = Some(context.clone());
        auth.assistant_chat = Some(Arc::new(chat));
    }
    auth.execution_permission = Some(context);
    Ok(())
}

pub(crate) async fn fence_resume(
    resume: &Resume,
    db: &Database,
    session: &mut ClientSession,
    dispatch: bool,
) -> AppResult<()> {
    if resume
        .connected_wait
        .as_ref()
        .is_some_and(|guard| guard.cancellation.is_cancelled())
    {
        return Err(AppError::ClientDisconnected);
    }
    crate::services::security_readiness_service::require(db, READINESS).await?;
    let row = db
        .collection::<ApprovalRequest>(REQUESTS)
        .find_one(doc! {"_id":&resume.request_id})
        .session(&mut *session)
        .await?
        .ok_or_else(refused)?;
    fence_source(db, session, &row).await?;
    let update = if dispatch {
        doc! {"$set":{"execution_proposal.v2.state":"dispatched","execution_proposal.v2.claimed_at":bson::DateTime::now()}}
    } else {
        doc! {"$inc":{"execution_dispatch_fence":1_i64}}
    };
    if db.collection::<Document>(REQUESTS).update_one(doc!{"_id":&resume.request_id,"status":"approved","execution_proposal.version":2,"execution_proposal.v2.state":"preparing","execution_proposal.v2.claim_id":&resume.claim_id,"execution_proposal.v2.grant_id":&resume.grant_id},update).session(session).await?.matched_count!=1{return Err(uncertain());}
    Ok(())
}

pub(super) async fn claim(
    state: &crate::AppState,
    row: &ApprovalRequest,
) -> AppResult<Option<(Resume, PreparedAdapter)>> {
    claim_inner(&state.db, &state.encryption_keys, row, None).await
}

pub(super) async fn claim_inner(
    db: &Database,
    keys: &Arc<EncryptionKeys>,
    row: &ApprovalRequest,
    connected_wait: Option<Arc<blocking::WaitGuard>>,
) -> AppResult<Option<(Resume, PreparedAdapter)>> {
    let p = proposal(row)?;
    if (!p.background_allowed && connected_wait.is_none())
        || !matches!(p.state.as_str(), "queued" | "preparing")
    {
        return Ok(None);
    }
    crate::services::security_readiness_service::require(db, READINESS).await?;
    let raw = Zeroizing::new(
        keys.decrypt_with_aad(
            &p.payload.as_ref().ok_or_else(conflict)?.bytes,
            aad(&row.id).as_bytes(),
        )
        .await?,
    );
    if raw.len() > MAX_PAYLOAD {
        return Err(conflict());
    }
    let retained: RetainedEffect = serde_json::from_slice(&raw).map_err(encoding)?;
    let claim_id = Uuid::new_v4().to_string();
    let mut session = db.client().start_session().await?;
    let won=session.start_transaction().and_run((db,&row.id,&claim_id),|session,(db,id,claim)|Box::pin(async move{
        let result:AppResult<bool>=async{
            let filter=doc!{"_id":*id,"status":"approved","execution_proposal.version":2,"$or":[{"execution_proposal.v2.state":"queued"},{"execution_proposal.v2.state":"preparing","execution_proposal.v2.claimed_at":{"$lt":bson::DateTime::from_chrono(Utc::now()-Duration::minutes(2))}}]};
            let Some(row)=db.collection::<ApprovalRequest>(REQUESTS).find_one(filter.clone()).session(&mut *session).await? else{return Ok(false)};
            let p=proposal(&row)?;
            require_enrolled(db,&row.user_id,Some(session)).await?;
            fence_source(db,session,&row).await?;
            if db.collection::<Document>(REQUESTS).update_one(filter,doc!{"$set":{"execution_proposal.v2.state":"preparing","execution_proposal.v2.claim_id":*claim,"execution_proposal.v2.claimed_at":bson::DateTime::now()}}).session(&mut *session).await?.matched_count!=1{return Ok(false);}
            if p.state=="preparing" && let Some(grant)=&p.grant_id {
                db.collection::<Document>("approval_grants").update_one(doc!{"_id":grant,"revoked":false,"approval_request_id":*id,"execution_use.version":VERSION as i32,"execution_use.state":"claimed","execution_use.operation_id":&p.effect_id,"execution_use.prepared_digest":&p.prepared_digest,"execution_use.dispatched_at":null},doc!{"$unset":{"execution_use":""}}).session(&mut *session).await?;
            }
            Ok(true)
        }.await;
        crate::services::api_key_mutation_service::transaction_result(result)
    })).await.map_err(crate::services::api_key_mutation_service::map_transaction_error)?;
    if !won {
        return Ok(None);
    }
    Ok(Some((
        Resume {
            connected_wait,
            version: 2,
            request_id: row.id.clone(),
            claim_id,
            effect_id: p.effect_id.clone(),
            grant_id: p.grant_id.clone().ok_or_else(conflict)?,
            source_credential_id: p.source_credential_id.clone(),
            prepared_digest: p.prepared_digest.clone(),
            authority_digest: p.authority_digest.clone(),
            effect: retained.effect,
            independent_card_id: p.independent_card_id.clone(),
        },
        retained.adapter,
    )))
}

/// A durable claim wins independently of grant lifetime. Polling never calls this.
pub(crate) async fn execute(
    state: &crate::AppState,
    row: &ApprovalRequest,
) -> AppResult<Option<Response>> {
    let p = proposal(row)?;
    if matches!(p.source, ProposalSource::Watch { .. }) {
        return Ok(None);
    }
    let Some((resume, adapter)) = claim(state, row).await? else {
        return Ok(None);
    };
    let outcome = async {
        let key=state.db.collection::<crate::models::api_key::ApiKey>("api_keys").find_one(doc!{"_id":&p.key_id,"user_id":&row.user_id,"is_active":true,"$or":[{"expires_at":null},{"expires_at":{"$gt":bson::DateTime::now()}}]}).await?.ok_or_else(refused)?;
        let mut auth = crate::mw::auth::api_key_auth_user(
            &state.db,
            &key,
            p.source_credential_id.clone(),
            None,
            None,
        )
        .await?;
        attach_resume(&mut auth, resume.clone())?;
        if let Some(context) = &auth.execution_permission {
            let _ = context.ingress.set(adapter.clone());
        }
        Box::pin(crate::handlers::execution_permission_resume::send(
            state,
            &auth,
            resume.effect.clone(),
            adapter,
            resume.independent_card_id.clone(),
        ))
        .await
    }
    .await;
    complete_response(state, row, &resume, outcome).await
}

pub(super) async fn complete_response(
    state: &crate::AppState,
    row: &ApprovalRequest,
    resume: &Resume,
    outcome: AppResult<Response>,
) -> AppResult<Option<Response>> {
    let current = state
        .db
        .collection::<ApprovalRequest>(REQUESTS)
        .find_one(doc! {"_id":&row.id,"execution_proposal.v2.claim_id":&resume.claim_id})
        .await?
        .ok_or_else(conflict)?;
    let dispatched = proposal(&current)?.state == "dispatched";
    match outcome {
        Ok(response) if dispatched => {
            let (parts, body) = response.into_parts();
            let state = state.clone();
            let row = row.clone();
            let resume = resume.clone();
            let status = parts.status.as_u16();
            let headers = parts
                .headers
                .iter()
                .filter(|(k, _)| matches!(k.as_str(), "content-type" | "retry-after"))
                .filter_map(|(k, v)| v.to_str().ok().map(|v| (k.to_string(), v.to_owned())))
                .collect::<Vec<_>>();
            // Tee the actual stream; never call a provider to reconstruct output.
            let stream = async_stream::stream! {
                use futures::StreamExt;
                let mut stream=body.into_data_stream();let mut bytes=Vec::new();let mut partial=false;
                while let Some(next)=stream.next().await {
                    match next {
                        Ok(chunk)=>{if !partial && bytes.len()+chunk.len()+4096<=MAX_PAYLOAD{bytes.extend_from_slice(&chunk);}else{partial=true;}yield Ok::<_,axum::Error>(chunk);}
                        Err(error)=>{let _=finish(&state,&row,&resume,"unknown",Some("response_unavailable"),None).await;yield Err(error);return;}
                    }
                }
                let body = match String::from_utf8(bytes) {
                    Ok(body) => body,
                    Err(_) => { partial = true; String::new() }
                };
                let result=ExecutionResult{status,headers,body,partial};
                let _=finish(&state,&row,&resume,"completed",partial.then_some("response_partial"),Some(result)).await;
            };
            Ok(Some(Response::from_parts(
                parts,
                axum::body::Body::from_stream(stream),
            )))
        }
        Err(AppError::ApprovalRequired { request_id }) if !dispatched => {
            state.db.collection::<Document>(REQUESTS).update_one(doc!{"_id":&row.id,"execution_proposal.v2.claim_id":&resume.claim_id,"execution_proposal.v2.state":"preparing"},doc!{"$set":{"execution_proposal.v2.state":"waiting","execution_proposal.v2.independent_request_id":request_id}}).await?;
            Ok(None)
        }
        Ok(response) => {
            let bytes = axum::body::to_bytes(response.into_body(), MAX_PAYLOAD)
                .await
                .unwrap_or_default();
            let value = serde_json::from_slice::<serde_json::Value>(&bytes).unwrap_or_default();
            let pending = value
                .get("request_id")
                .filter(|_| {
                    value.get("error").and_then(|v| v.as_str()) == Some("approval_required")
                })
                .or_else(|| value.pointer("/result/execution/request_id"))
                .and_then(|v| v.as_str());
            if let Some(id) = pending {
                if state.db.collection::<Document>(REQUESTS).find_one(doc! {"_id":id,"execution_proposal":null,"$or":[{"user_id":&row.user_id},{"notify_user_ids":&row.user_id}]}).await?.is_none() {return Err(refused());}
                state.db.collection::<Document>(REQUESTS).update_one(doc! {"_id":&row.id,"execution_proposal.v2.claim_id":&resume.claim_id,"execution_proposal.v2.state":"preparing"},doc! {"$set":{"execution_proposal.v2.state":"waiting","execution_proposal.v2.independent_request_id":id}}).await?;
                return Ok(None);
            }
            let card = value
                .get("acknowledgement_id")
                .or_else(|| value.pointer("/acknowledgement/id"))
                .and_then(|v| v.as_str());
            if let Some(card) = card {
                let p = proposal(row)?;
                if state.db.collection::<Document>("assistant_acknowledgements").find_one(doc!{"_id":card,"user_id":&row.user_id,"conversation_id":&p.conversation_id,"api_key_id":&p.key_id,"status":"pending","execution_task.task":bson::to_bson(&p.task.as_ref().map(|t|&t.task)).map_err(encoding)?}).await?.is_none(){return Err(refused());}
                state.db.collection::<Document>(REQUESTS).update_one(doc!{"_id":&row.id,"execution_proposal.v2.claim_id":&resume.claim_id,"execution_proposal.v2.state":"preparing"},doc!{"$set":{"execution_proposal.v2.state":"waiting","execution_proposal.v2.independent_card_id":card}}).await?;
                return Ok(None);
            }
            finish(
                state,
                row,
                resume,
                "not_sent",
                Some("operation_not_sent"),
                None,
            )
            .await?;
            Ok(None)
        }
        other => {
            let code = if dispatched {
                "provider_outcome_unknown"
            } else {
                "operation_not_sent"
            };
            finish(
                state,
                row,
                resume,
                if dispatched { "unknown" } else { "not_sent" },
                Some(code),
                None,
            )
            .await?;
            match other {
                Err(e) => Err(e),
                Ok(_) => Ok(None),
            }
        }
    }
}

pub(super) async fn finish(
    state: &crate::AppState,
    row: &ApprovalRequest,
    resume: &Resume,
    state_name: &str,
    code: Option<&str>,
    result: Option<ExecutionResult>,
) -> AppResult<()> {
    let mut code = code;
    let encrypted = if let Some(result) = result {
        let raw = Zeroizing::new(serde_json::to_vec(&result).map_err(encoding)?);
        if raw.len() + 2048 > MAX_PAYLOAD {
            code = Some("response_unavailable");
            None
        } else {
            Some(binary(
                state
                    .encryption_keys
                    .encrypt_with_aad(&raw, format!("{}:result", aad(&row.id)).as_bytes())
                    .await?,
            ))
        }
    } else {
        None
    };
    let mut session = state.db.client().start_session().await?;
    session.start_transaction().await?;
    if state.db.collection::<Document>(REQUESTS).update_one(doc!{"_id":&row.id,"execution_proposal.v2.claim_id":&resume.claim_id,"execution_proposal.v2.delivery_abandoned":{"$ne":true},"execution_proposal.v2.state":{"$in":["preparing","dispatched"]}},doc!{"$set":{"execution_proposal.v2.state":state_name,"execution_proposal.v2.failure_code":code,"execution_proposal.v2.result":bson::to_bson(&encrypted).map_err(encoding)?},"$unset":{"execution_proposal.v2.payload":""}}).session(&mut session).await?.matched_count==1{super::super::lifecycle::retire_operations(&state.db,&mut session,std::slice::from_ref(&row.id)).await?;}
    session.commit_transaction().await?;
    Ok(())
}

pub(crate) async fn sweep(state: &crate::AppState) -> AppResult<()> {
    let rows:Vec<ApprovalRequest>=state.db.collection::<ApprovalRequest>(REQUESTS).find(doc!{"execution_proposal.version":2,"execution_proposal.v2.state":{"$in":["pending","queued","preparing","dispatched","waiting"]},"$or":[{"execution_proposal.v2.maintenance_at":null},{"execution_proposal.v2.maintenance_at":{"$lt":bson::DateTime::from_chrono(Utc::now()-Duration::seconds(15))}}]}).sort(doc!{"execution_proposal.v2.maintenance_at":1,"created_at":1}).limit(32).await?.try_collect().await?;
    for row in rows {
        if state.db.collection::<Document>(REQUESTS).update_one(doc!{"_id":&row.id,"execution_proposal.v2.maintenance_at":proposal(&row)?.maintenance_at.map(bson::DateTime::from_chrono)},doc!{"$set":{"execution_proposal.v2.maintenance_at":bson::DateTime::now()}}).await?.matched_count!=1{continue;}
        if let Err(e) = maintain(state, &row).await {
            tracing::debug!(
                code = e.error_code(),
                "Execution proposal maintenance deferred"
            );
        }
    }
    state.db.collection::<Document>(REQUESTS).update_many(doc!{"execution_proposal.version":2,"created_at":{"$lt":bson::DateTime::from_chrono(Utc::now()-Duration::days(1))}},doc!{"$unset":{"execution_proposal.v2.payload":"","execution_proposal.v2.result":""}}).await?;
    Ok(())
}
async fn maintain(state: &crate::AppState, row: &ApprovalRequest) -> AppResult<()> {
    let p = proposal(row)?;
    if !p.background_allowed
        && p.state != "dispatched"
        && p.waiter_until.is_none_or(|deadline| deadline <= Utc::now())
    {
        blocking::abandon(&state.db, &row.id, "approval_wait_timed_out").await?;
        return Ok(());
    }
    if row.expires_at <= Utc::now() {
        return cancel(&state.db, &row.id, "source_expired").await;
    }
    if let Err(error) = require_enrolled(&state.db, &row.user_id, None).await {
        if error.is_forbidden() {
            return cancel(&state.db, &row.id, "enrollment_disabled").await;
        }
        return Err(error);
    }
    if p.state != "dispatched" {
        match authority::Snapshot::load(
            &state.db,
            &p.contract.binding.connection_id,
            &p.key_id,
            p.source_credential_id.as_deref(),
        )
        .await
        {
            Ok(live) if live.digest() == p.authority_digest => {}
            Ok(_) => return cancel(&state.db, &row.id, "authority_lost").await,
            Err(error) if error.is_forbidden() || matches!(error, AppError::NotFound(_)) => {
                return cancel(&state.db, &row.id, "authority_lost").await;
            }
            Err(error) => return Err(error),
        }
        if let Some(task) = &p.task
            && let Err(error) = tasks::check(&state.db, task).await
        {
            if error.is_forbidden()
                || matches!(error, AppError::NotFound(_) | AppError::Conflict(_))
            {
                return cancel(&state.db, &row.id, "task_unavailable").await;
            }
            return Err(error);
        }
    }
    if p.state == "pending" {
        notify_once(state, row).await?;
        return Ok(());
    }
    if p.state == "dispatched" {
        if p.claimed_at
            .is_some_and(|at| at < Utc::now() - Duration::minutes(5))
        {
            cancel(&state.db, &row.id, "provider_outcome_unknown").await?;
        }
        return Ok(());
    }
    if p.state == "waiting" {
        let review = if let Some(id) = &p.independent_request_id {
            state
                .db
                .collection::<Document>(REQUESTS)
                .find_one(doc! {"_id":id,"execution_proposal":null,"$or":[{"user_id":&row.user_id},{"notify_user_ids":&row.user_id}]})
                .await?
        } else if let Some(id) = &p.independent_card_id {
            state
                .db
                .collection::<Document>("assistant_acknowledgements")
                .find_one(doc! {"_id":id,"user_id":&row.user_id})
                .await?
        } else {
            None
        };
        let live = review.as_ref().is_some_and(|r| {
            r.get_datetime("expires_at")
                .is_ok_and(|t| t.to_chrono() > Utc::now())
        });
        let status = review.as_ref().and_then(|r| r.get_str("status").ok());
        if live && matches!(status, Some("approved" | "allowed")) {
            state
                .db
                .collection::<Document>(REQUESTS)
                .update_one(
                    doc! {"_id":&row.id,"execution_proposal.v2.state":"waiting"},
                    doc! {"$set":{"execution_proposal.v2.state":"queued"}},
                )
                .await?;
        } else if !live || status != Some("pending") {
            cancel(&state.db, &row.id, "independent_review_unavailable").await?;
        }
        return Ok(());
    }
    // A connected occurrence can only be claimed by its original in-process
    // waiter. Maintenance may notify/expire it but can never dispatch it.
    if !p.background_allowed {
        return Ok(());
    }
    if matches!(p.source, ProposalSource::Voice { .. }) {
        let mut session = state.db.client().start_session().await?;
        session.start_transaction().await?;
        fence_source(&state.db, &mut session, row).await?;
        decision::queue_voice(&state.db, &mut session, row).await?;
        session.commit_transaction().await?;
        return Ok(());
    }
    if matches!(p.source, ProposalSource::Watch { .. }) {
        return Ok(());
    }
    if p.waiter_until.is_some_and(|until| until > Utc::now()) {
        return Ok(());
    }
    if let Some(response) = Box::pin(execute(state, row)).await? {
        drain(response).await;
    }
    Ok(())
}

pub(super) async fn cancel(db: &Database, id: &str, reason: &str) -> AppResult<()> {
    let mut session = db.client().start_session().await?;
    session.start_transaction().await?;
    cancel_in_session(db, &mut session, doc! {"_id":&id,
        "execution_proposal.v2.state":{"$in":["pending","queued","preparing","waiting","dispatched","unknown"]}}, reason).await?;
    session.commit_transaction().await?;
    Ok(())
}
pub(crate) async fn cancel_in_session(
    db: &Database,
    session: &mut ClientSession,
    mut filter: Document,
    reason: &str,
) -> AppResult<()> {
    filter.insert("execution_proposal.version", 2);
    let mut cursor = db
        .collection::<ApprovalRequest>(REQUESTS)
        .find(filter.clone())
        .session(&mut *session)
        .await?;
    let rows = cursor.stream(&mut *session).try_collect::<Vec<_>>().await?;
    for row in rows {
        let p = proposal(&row)?;
        if matches!(
            p.state.as_str(),
            "completed" | "not_sent" | "denied" | "expired" | "cancelled"
        ) {
            // Erasure or departure can withdraw delivery, but cannot rewrite
            // the recorded outcome of an effect that has already settled.
            db.collection::<Document>(REQUESTS).update_one(doc! {"_id":&row.id}, doc! {
                "$set":{"execution_proposal.v2.delivery_abandoned":true},
                "$unset":{"execution_proposal.v2.payload":"","execution_proposal.v2.result":""}
            }).session(&mut *session).await?;
            super::super::lifecycle::retire_operations(db, session, std::slice::from_ref(&row.id))
                .await?;
            continue;
        }
        let state = if matches!(p.state.as_str(), "dispatched" | "unknown") {
            "unknown"
        } else {
            "cancelled"
        };
        db.collection::<Document>(REQUESTS).update_one(doc!{"_id":&row.id},doc!{"$set":{"status":"expired","execution_proposal.v2.state":state,"execution_proposal.v2.failure_code":reason,"execution_proposal.v2.delivery_abandoned":true},"$unset":{"execution_proposal.v2.payload":"","execution_proposal.v2.result":""}}).session(&mut *session).await?;
        super::super::lifecycle::retire_operations(db, session, std::slice::from_ref(&row.id))
            .await?;
    }
    Ok(())
}

pub(crate) async fn resume_voice(
    state: &crate::AppState,
    row: &AssistantConversation,
) -> AppResult<Option<String>> {
    let Some(turn) = &row.active_turn else {
        return Ok(None);
    };
    if turn.execution_task.is_none() {
        return Ok(None);
    }
    let mut notes = Vec::new();
    let mut ids = turn
        .events
        .iter()
        .filter_map(|e| {
            e.id.strip_prefix("execution-decision-v2:")
                .map(str::to_owned)
        })
        .collect::<Vec<_>>();
    if let Some(voice_id) = &turn.voice_request_id
        && let Some(request) = state.db.collection::<crate::models::assistant_voice::VoiceRequest>(crate::models::assistant_voice::REQUESTS).find_one(doc!{"_id":voice_id,"user_id":&row.user_id,"task_conversation_id":&row.id,"execution_enrolled":true}).await?
        && let Some(id) = request.source_id.strip_prefix("execution-decision-v2:") {
        ids.push(id.split(':').next().ok_or_else(refused)?.to_owned());
    }
    for id in ids {
        let request=state.db.collection::<ApprovalRequest>(REQUESTS).find_one(doc!{"_id":&id,"user_id":&row.user_id,"execution_proposal.version":2,"execution_proposal.v2.conversation_id":&row.id,"execution_proposal.v2.key_id":&row.credential_api_key_id}).await?.ok_or_else(refused)?;
        let p = proposal(&request)?;
        if !matches!(p.source, ProposalSource::Voice { .. })
            || turn
                .execution_task
                .as_ref()
                .zip(p.task.as_ref())
                .is_none_or(|(a, b)| {
                    a.task != b.task
                        || a.epoch != b.epoch
                        || a.actor_user_id != b.actor_user_id
                        || a.audience != b.audience
                })
        {
            return Err(refused());
        }
        if matches!(p.state.as_str(), "queued" | "preparing")
            && let Some(response) = Box::pin(execute(state, &request)).await?
        {
            drain(response).await;
        }
        let current = state
            .db
            .collection::<ApprovalRequest>(REQUESTS)
            .find_one(doc! {"_id":&id})
            .await?
            .ok_or_else(refused)?;
        let p = proposal(&current)?;
        notes.push(format!(
            "The retained operation is {}. Do not repeat it.",
            p.state
        ));
        if let Some(result) = super::readback::decrypt_result(state, &current).await? {
            notes.push(format!(
                "HTTP {}{}: {}",
                result.status,
                if result.partial {
                    " (partial response)"
                } else {
                    ""
                },
                result.body.chars().take(16384).collect::<String>()
            ));
        }
    }
    Ok((!notes.is_empty()).then(|| notes.join("\n")))
}

async fn drain(response: Response) {
    use futures::StreamExt;
    let _ = tokio::time::timeout(std::time::Duration::from_secs(120), async {
        let mut chunks = response.into_body().into_data_stream();
        while let Some(chunk) = chunks.next().await {
            if chunk.is_err() {
                break;
            }
        }
    })
    .await;
}

/// Shared by the post-commit sender and the recovery sweep. Only the CAS winner
/// notifies; no message or credential data enters diagnostics.
pub(super) async fn notify_once(state: &crate::AppState, row: &ApprovalRequest) -> AppResult<()> {
    if state
        .db
        .collection::<Document>(REQUESTS)
        .update_one(
            doc! {"_id": &row.id, "status": "pending", "execution_proposal.version": 2,
            "execution_proposal.v2.notification_claimed_at": null},
            doc! {"$set": {"execution_proposal.v2.notification_claimed_at": bson::DateTime::now()}},
        )
        .await?
        .matched_count
        == 1
    {
        crate::services::notification_service::send_approval_notification(
            &state.db,
            &state.config,
            &state.http_client,
            state.fcm_auth.as_deref(),
            state.apns_auth.as_deref(),
            &row.user_id,
            row,
            None,
        )
        .await?;
    }
    Ok(())
}

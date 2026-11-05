use super::*;

/// Called inside the existing source/key cancellation transaction. Dispatched
/// effects retain uncertainty; cancellation never claims that a send was undone.
pub(crate) async fn cancel_in_session(
    db: &Database,
    session: &mut ClientSession,
    mut filter: Document,
    reason: &str,
) -> AppResult<()> {
    filter.insert("execution_proposal.version", VERSION as i32);
    let mut cursor = db
        .collection::<Document>(REQUESTS)
        .find(filter.clone())
        .projection(doc! {"_id":1})
        .session(&mut *session)
        .await?;
    let ids = cursor
        .stream(&mut *session)
        .try_collect::<Vec<_>>()
        .await?
        .into_iter()
        .filter_map(|row| row.get_str("_id").ok().map(str::to_owned))
        .collect::<Vec<_>>();
    retire_operations(db, session, &ids).await?;
    filter.insert(
        "execution_proposal.v1.state",
        doc! {"$in":["pending","queued","preparing","waiting"]},
    );
    db.collection::<Document>(REQUESTS).update_many(filter.clone(),doc!{
        "$set":{"status":"expired","execution_proposal.v1.state":"cancelled","execution_proposal.v1.failure_code":reason},
        "$unset":{"execution_proposal.v1.payload":"","execution_proposal.v1.result":""}
    }).session(&mut *session).await?;
    filter.insert(
        "execution_proposal.v1.state",
        doc! {"$in":["dispatched","unknown","completed","not_sent","denied","expired","cancelled"]},
    );
    db.collection::<Document>(REQUESTS).update_many(filter,doc!{"$set":{"execution_proposal.v1.delivery_abandoned":true},"$unset":{"execution_proposal.v1.payload":"","execution_proposal.v1.result":""}}).session(session).await?;
    Ok(())
}

fn event(row: &ApprovalRequest, p: &Proposal) -> crate::models::assistant_conversation::AgentEvent {
    let mut event = super::super::super::assistant_team_service::event(
        "permission_decided",
        "NyxID is delivering the retained operation's status. Do not reconstruct or repeat it."
            .into(),
        None,
    )
    .bound_to(p.report_to.as_deref())
    .for_invocation(p.invocation_id.as_deref());
    event.id = format!("execution-decision:{}", row.id);
    event.execution_task = Some(p.task.clone());
    event
}

/// Indexed, bounded maintenance on new-format rows only. Existing turn/request
/// paths never load proposal state unless their enrollment/source hint is present.
pub(crate) async fn sweep(state: &crate::AppState) -> AppResult<()> {
    let rows:Vec<ApprovalRequest>=state.db.collection::<ApprovalRequest>(REQUESTS).find(doc!{
        "execution_proposal.version":VERSION as i32,"execution_proposal.v1.state":{"$in":["pending","queued","preparing","dispatched","waiting","denied","expired","completed","not_sent","unknown"]},
        "execution_proposal.v1.delivered_turn_id":null,"execution_proposal.v1.delivery_abandoned":{"$ne":true},
        "$or":[{"execution_proposal.v1.maintenance_at":null},{"execution_proposal.v1.maintenance_at":{"$lt":bson::DateTime::from_chrono(Utc::now()-Duration::seconds(15))}}]
    }).sort(doc!{"execution_proposal.v1.maintenance_at":1,"created_at":1}).limit(64).await?.try_collect().await?;
    for row in rows {
        // A bounded lease schedules every row fairly even when many old reviews
        // remain pending. Concurrent sweepers cannot repeatedly notify or wake it.
        if state.db.collection::<Document>(REQUESTS).update_one(doc!{"_id":&row.id,"$or":[{"execution_proposal.v1.maintenance_at":null},{"execution_proposal.v1.maintenance_at":{"$lt":bson::DateTime::from_chrono(Utc::now()-Duration::seconds(15))}}]},doc!{"$set":{"execution_proposal.v1.maintenance_at":bson::DateTime::now()}}).await?.matched_count!=1 {continue;}
        if let Err(error) = maintain(state, &row).await {
            tracing::debug!(
                code = error.error_code(),
                "Execution proposal maintenance deferred"
            );
        }
    }
    // Receipt replay is bounded; metadata remains under the existing 90-day TTL.
    state.db.collection::<Document>(REQUESTS).update_many(doc!{
        "execution_proposal.version":VERSION as i32,"created_at":{"$lt":bson::DateTime::from_chrono(Utc::now()-Duration::days(1))},
        "$or":[{"execution_proposal.v1.payload":{"$ne":null}},{"execution_proposal.v1.result":{"$ne":null}}]
    },doc!{"$unset":{"execution_proposal.v1.payload":"","execution_proposal.v1.result":""}}).await?;
    Ok(())
}
async fn maintain(state: &crate::AppState, row: &ApprovalRequest) -> AppResult<()> {
    let p = proposal(row)?;
    if let Err(error) = require_enrolled(&state.db, &row.user_id, None).await {
        if !error.is_forbidden() {
            return Err(error);
        }
        let mut session = state.db.client().start_session().await?;
        session.start_transaction().await?;
        cancel_in_session(
            &state.db,
            &mut session,
            doc! {"_id":&row.id},
            "enrollment_disabled",
        )
        .await?;
        session.commit_transaction().await?;
        return Ok(());
    }
    if row.status == "pending" && row.expires_at > Utc::now() && p.state == "pending" {
        // Claim before sending; a failed notification cannot duplicate a card or
        // lose its web inbox entry. Existing channels remain link-only.
        if state.db.collection::<Document>(REQUESTS).update_one(doc!{"_id":&row.id,"status":"pending","execution_proposal.v1.notification_claimed_at":null},doc!{"$set":{"execution_proposal.v1.notification_claimed_at":bson::DateTime::now()}}).await?.matched_count==1 {
            super::super::super::notification_service::send_approval_notification(&state.db,&state.config,&state.http_client,state.fcm_auth.as_deref(),state.apns_auth.as_deref(),&row.user_id,row,None).await?;
        }
        return Ok(());
    }
    let mut session = state.db.client().start_session().await?;
    session.start_transaction().and_run((&state.db,&row.id),|session,(db,id)|Box::pin(async move {
        let result:AppResult<()>=async {
            let requests=db.collection::<ApprovalRequest>(REQUESTS);
            let row=requests.find_one(doc!{"_id":*id}).session(&mut *session).await?.ok_or_else(missing)?;
            let p=proposal(&row)?;
            if p.delivered_turn_id.is_some(){return Ok(());}
            let chats=db.collection::<AssistantConversation>("assistant_conversations");
            let source=chats.find_one(doc!{"_id":&p.conversation_id,"user_id":&row.user_id,"credential_api_key_id":&p.key_id}).session(&mut *session).await?;
            let Some(source)=source else {
                cancel_in_session(db,session,doc!{"_id":&row.id},"source_unavailable").await?;
                return Ok(());
            };
            let event_id=format!("execution-decision:{}",row.id);
            if source.active_turn.is_some() || source.pending_events.iter().any(|e|e.id==event_id) {return Ok(());}
            // Never steal a live preparer. A stale lease has not crossed the CAS
            // send barrier and is safe for a new fenced preparer.
            if p.state=="preparing" && p.claimed_at.is_some_and(|at|at>Utc::now()-Duration::minutes(2)){return Ok(());}
            if p.wake_at.is_some_and(|at|at>Utc::now()-Duration::seconds(15)){return Ok(());}
            if let Err(error)=tasks::fence(db,session,&p.task).await {
                if error.is_forbidden() || matches!(error,AppError::Conflict(_)|AppError::NotFound(_)|AppError::AssistantTurnRequired) {
                    cancel_in_session(db,session,doc!{"_id":&row.id},"task_unavailable").await?;
                    return Ok(());
                }
                return Err(error);
            }
            let mut next_state=p.state.as_str();
            if p.state=="waiting" {
                let review=if let Some(id)=&p.independent_request_id {
                    db.collection::<Document>(REQUESTS).find_one(doc!{"_id":id,"execution_proposal":null}).session(&mut *session).await?
                }else if let Some(id)=&p.independent_card_id {
                    db.collection::<Document>("assistant_acknowledgements").find_one(doc!{"_id":id}).session(&mut *session).await?
                }else{None};
                let live=review.as_ref().is_some_and(|review|review.get_datetime("expires_at").is_ok_and(|at|at.to_chrono()>Utc::now()));
                let status=review.as_ref().and_then(|review|review.get_str("status").ok());
                if live && status==Some("pending") {return Ok(());}
                let allowed=live && matches!(status,Some("approved"|"allowed"));
                next_state=if allowed{"queued"}else{"not_sent"};
                let mut update=doc!{"$set":{"execution_proposal.v1.state":next_state}};
                if !allowed {
                    update.get_document_mut("$set").expect("set").insert("execution_proposal.v1.failure_code","independent_review_unavailable");
                    update.insert("$unset",doc!{"execution_proposal.v1.payload":""});
                    retire_operations(db,session,std::slice::from_ref(&row.id)).await?;
                }
                if db.collection::<Document>(REQUESTS).update_one(doc!{"_id":&row.id,"execution_proposal.v1.state":"waiting"},update).session(&mut *session).await?.matched_count!=1 {return Err(conflict());}
            }
            if p.state=="pending"&&(row.status!="pending"||row.expires_at<=Utc::now()) {
                if db.collection::<Document>(REQUESTS).update_one(doc!{"_id":&row.id,"execution_proposal.v1.state":"pending"},doc!{"$set":{"status":"expired","execution_proposal.v1.state":"expired"},"$unset":{"execution_proposal.v1.payload":""}}).session(&mut *session).await?.matched_count!=1 {return Err(conflict());}
                next_state="expired";
            }
            if source.collaboration.is_some(){super::super::super::collaboration_group_service::runtime::queue_decision(db,&source,session).await?;}
            let entry=bson::to_bson(&event(&row,p)).map_err(encoding)?;
            if chats.update_one(doc!{"_id":&source.id,"active_turn":null,"pending_events.63":{"$exists":false},"pending_events.id":{"$ne":&event_id}},doc!{"$push":{"pending_events":entry},"$inc":{"execution_dispatch_fence":1_i64}}).session(&mut *session).await?.matched_count!=1{return Err(conflict());}
            if db.collection::<Document>(REQUESTS).update_one(doc!{"_id":&row.id,"execution_proposal.v1.state":next_state},doc!{"$set":{"execution_proposal.v1.wake_at":bson::DateTime::now()}}).session(&mut *session).await?.matched_count!=1 {return Err(conflict());}
            Ok(())
        }.await;
        super::super::super::api_key_mutation_service::transaction_result(result)
    })).await.map_err(super::super::super::api_key_mutation_service::map_transaction_error)
}

/// Reusable authority is never consumed by a receipt. Operation permissions are
/// retired together with a terminal effect/source, including pre-send failures.
pub(super) async fn retire_operations(
    db: &Database,
    session: &mut ClientSession,
    ids: &[String],
) -> AppResult<()> {
    if !ids.is_empty() {
        db.collection::<Document>("approval_grants").update_many(doc!{"approval_request_id":{"$in":ids},"execution.version":VERSION as i32,"execution.v1.validity.kind":"operation","revoked":false},doc!{"$set":{"revoked":true,"expires_at":bson::DateTime::from_chrono(contract::terminal_expiry(Utc::now()))}}).session(session).await?;
    }
    Ok(())
}

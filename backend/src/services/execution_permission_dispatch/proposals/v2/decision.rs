use super::*;

fn replay(row: &ApprovalRequest, input: &Decision) -> AppResult<ApprovalRequest> {
    let p = proposal(row)?;
    if row.decision_idempotency_key.as_deref() == Some(input.idempotency_key.as_str())
        && p.revision == input.expected_revision
        && p.selected_choice == input.choice_id
        && p.selected_choice_digest == input.choice_digest
        && (row.status == "approved") == input.approved
    {
        Ok(row.clone())
    } else {
        Err(conflict())
    }
}

pub(crate) async fn decide(
    state: &crate::AppState,
    actor: &str,
    id: &str,
    input: &Decision,
) -> AppResult<ApprovalRequest> {
    crate::services::security_readiness_service::require(&state.db, READINESS).await?;
    let row = state
        .db
        .collection::<ApprovalRequest>(REQUESTS)
        .find_one(doc! {"_id":id,"user_id":actor,"execution_proposal.version":2})
        .await?
        .ok_or_else(missing)?;
    if row.status != "pending" {
        return replay(&row, input);
    }
    let p = proposal(&row)?;
    let snapshot = authority::Snapshot::load(
        &state.db,
        &p.contract.binding.connection_id,
        &p.key_id,
        p.source_credential_id.as_deref(),
    )
    .await?;
    if snapshot.digest() != p.authority_digest {
        return Err(conflict());
    }
    let grant_id = Uuid::new_v4().to_string();
    let mut session = state.db.client().start_session().await?;
    session.start_transaction().and_run((&state.db,actor,id,input,&snapshot,&grant_id),|session,(db,actor,id,input,snapshot,grant_id)|Box::pin(async move{
        let result:AppResult<ApprovalRequest>=async{
            require_enrolled(db,actor,Some(session)).await?;
            let collection=db.collection::<ApprovalRequest>(REQUESTS);
            let mut row=collection.find_one(doc!{"_id":*id,"user_id":*actor}).session(&mut *session).await?.ok_or_else(missing)?;
            if row.status!="pending"{return replay(&row,input);}
            let p=proposal(&row)?.clone();
            if p.revision!=input.expected_revision{return Err(conflict());}
            fence_source(db,session,&row).await?;
            snapshot.fence(db,session).await?;
            let now=Utc::now();
            let mut updated=p.clone();
            if input.approved{
                let choice=p.choices.iter().find(|c|Some(&c.id)==input.choice_id.as_ref()&&Some(&c.digest)==input.choice_digest.as_ref()).ok_or_else(conflict)?;
                if p.source_credential_id.is_some() && !matches!(choice.validity,Validity::Operation{..}){return Err(refused());}
                if p.task.is_none() && matches!(choice.validity,Validity::Task{..}){return Err(refused());}
                let mut contract=p.contract.clone();contract.validity=choice.validity.clone();
                db.collection::<ApprovalGrant>("approval_grants").insert_one(ApprovalGrant{execution_use:None,execution_review:Some(p.review.clone()),execution_last_used_at:None,execution:Some(ExecutionPermission{version:VERSION,v1:Some(contract.clone())}),id:(*grant_id).clone(),user_id:actor.to_string(),service_id:row.service_id.clone(),service_name:row.service_name.clone(),requester_type:row.requester_type.clone(),requester_id:row.requester_id.clone(),requester_label:row.requester_label.clone(),approval_request_id:row.id.clone(),scope:Some(LEGACY_SCOPE.into()),granted_at:now,expires_at:contract::legacy_expiry(&contract.validity),revoked:false,org_scoped:false}).session(&mut *session).await?;
                updated.grant_id=Some((*grant_id).clone());updated.selected_choice=input.choice_id.clone();updated.selected_choice_digest=input.choice_digest.clone();updated.state="queued".into();updated.wake_at=Some(now);
            }else{updated.state="denied".into();updated.payload=None;}
            row.status=if input.approved{"approved"}else{"rejected"}.into();row.decided_at=Some(now);row.decision_channel=Some("web".into());row.decision_idempotency_key=Some(input.idempotency_key.clone());row.execution_proposal=Some(ExecutionProposal{version:2,v1:None,v2:Some(updated),v3:None});
            if collection.replace_one(doc!{"_id":&row.id,"status":"pending","execution_proposal.v2.revision":p.revision},&row).session(&mut *session).await?.matched_count!=1{return Err(conflict());}
            // Wakeups are durable row state, never a process-local send. Voice uses
            // its original isolated task's event admission; watches own their lease.
            if matches!(p.source,ProposalSource::Voice{..}){queue_voice(db,session,&row).await?;}
            audit(db,session,actor,if input.approved{"execution_permission_saved"}else{"execution_permission_denied"},id).await?;
            Ok(row)
        }.await;
        crate::services::api_key_mutation_service::transaction_result(result)
    })).await.map_err(crate::services::api_key_mutation_service::map_transaction_error)
}

pub(super) async fn queue_voice(
    db: &Database,
    session: &mut ClientSession,
    row: &ApprovalRequest,
) -> AppResult<()> {
    use crate::models::assistant_voice::{RequestState, VoiceRequest};
    let p = proposal(row)?;
    let ProposalSource::Voice { request_id } = &p.source else {
        return Err(refused());
    };
    let parent=db.collection::<VoiceRequest>(crate::models::assistant_voice::REQUESTS).find_one(doc!{"_id":request_id,"user_id":&row.user_id,"execution_enrolled":true,"state":{"$ne":"cancelled"}}).session(&mut *session).await?.ok_or_else(refused)?;
    let now = Utc::now();
    let continuation = VoiceRequest {
        execution_enrolled: true,
        model_task: None,
        progress: None,
        timing: Default::default(),
        started_at: None,
        terminal_error_code: None,
        async_operation_pending: false,
        id: Uuid::new_v4().to_string(),
        user_id: row.user_id.clone(),
        conversation_id: parent.conversation_id.clone(),
        session_id: parent.session_id.clone(),
        source_id: match p
            .independent_card_id
            .as_ref()
            .or(p.independent_request_id.as_ref())
        {
            Some(id) => format!("execution-decision-v2:{}:{id}", row.id),
            None => format!("execution-decision-v2:{}", row.id),
        },
        message_id: Uuid::new_v4().to_string(),
        message_seq: 0,
        turn_id: Uuid::new_v4().to_string(),
        state: RequestState::Queued,
        pending_acknowledgement_ids: Vec::new(),
        acknowledgement_id: None,
        credential_api_key_id: Some(p.key_id.clone()),
        parent_turn_id: Some(parent.turn_id),
        root_request_id: Some(parent.root_request_id.unwrap_or(parent.id)),
        task_conversation_id: parent.task_conversation_id,
        result_message_id: None,
        recovery_replays: 0,
        created_at: now,
        expires_at: row.expires_at,
    };
    crate::services::assistant_voice::enqueue_in_session(
        db,
        continuation,
        "NyxID is delivering the retained operation's status. Do not reconstruct or repeat it.",
        session,
    )
    .await?;
    Ok(())
}

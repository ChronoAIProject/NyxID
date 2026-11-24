//! Publication borrows exact original sources, never the home's newest turn.
use super::*;

pub(super) async fn insert_receipt(
    db: &Database,
    chat: &ChatAuthority,
    row: &AssistantAcknowledgement,
) -> AppResult<()> {
    super::super::transaction_service::run_db_only(db,&(db,chat,row),|tx,&(db,chat,row)|Box::pin(async move {
        super::super::erasure_admission::fence_in_session(db,&chat.user_id,tx).await?;
        let changed=db.collection::<bson::Document>(CONVERSATIONS).update_one(
            doc! {"_id":&chat.conversation_id,"user_id":&chat.user_id},doc! {"$inc":{"receipt_fence":1_i64}})
            .session(&mut *tx).await?;
        if changed.matched_count!=1 {return Err(AppError::AssistantTurnRequired)}
        if let Some(turn)=chat.turn_id.as_deref() {
            if db.collection::<bson::Document>(MESSAGES).find_one(doc! {"user_id":&chat.user_id,"conversation_id":&chat.conversation_id,"turn_id":turn})
                .projection(doc! {"_id":1}).session(&mut *tx).await?.is_none() {return Err(AppError::AssistantTurnRequired)}
            for proof in super::super::channel_meta_service::scopes::assistant_turn_references(db,tx,&chat.user_id,&chat.conversation_id,turn).await? {
                super::super::channel_meta_service::scopes::fence_reference(db,tx,&proof,false).await?;
            }
        }
        db.collection::<AssistantAcknowledgement>(ACKS).insert_one(row).session(tx).await?;
        Ok(())
    })).await
}

/// A decision callback may resume after deletion. Reread and fence the exact
/// stored card before copying its summary into a new event.
pub(crate) async fn publish_owner_denial(
    db: &Database,
    owner: &str,
    id: &str,
    conversation: &str,
) -> AppResult<bool> {
    super::super::transaction_service::run_db_only(db,&(db,owner,id,conversation),|tx,&(db,owner,id,conversation)|Box::pin(async move {
        let Some(row)=db.collection::<AssistantAcknowledgement>(ACKS).find_one_and_update(
            doc! {"_id":id,"user_id":owner,"conversation_id":conversation,"status":"denied"},doc! {"$inc":{"continuation_fence":1_i64}})
            .session(&mut *tx).await? else {return Ok(false)};
        if row.automation_work.is_some() || row.decider!="user" || row.trigger_run_id.is_some()
            || row.voice_request_id.is_some() || row.invocation_id.is_some() {return Ok(false)}
        super::super::erasure_admission::fence_in_session(db,owner,tx).await?;
        for turn in [row.creation_turn_id.as_deref(),row.requested_turn_id.as_deref()].into_iter().flatten() {
            for proof in super::super::channel_meta_service::scopes::assistant_turn_references(db,tx,owner,conversation,turn).await? {
                if !row.meta_contexts.iter().any(|context|context.meta_grant.as_ref()==Some(&proof)) {
                    return Err(super::super::channel_meta_service::grants::incomplete());
                }
            }
        }
        super::super::async_service_operation::meta_lineage::fence(db,tx,&row.meta_contexts,true).await?;
        let mut event=crate::models::assistant_acknowledgement::bind_continuation(row.routing.as_ref(),
            super::super::assistant_team_service::event("action_denied",format!(
                "The owner denied: {}. Do not execute or retry this action. Explain plainly what you skipped. {}",
                super::super::assistant_nyxagent::excerpt(&row.summary,2000),decision_narration(false)),None));
        event.id=Uuid::new_v5(&Uuid::NAMESPACE_OID,format!("nyxid:card-denial:{}",row.id).as_bytes()).to_string();
        event.execution_task=row.execution_task.clone();
        event.channel_contexts=row.meta_contexts.clone();
        super::super::assistant_nyxagent::append_events_in_session(db,owner,conversation,&[event],tx).await
    })).await
}

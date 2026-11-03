//! Durable, metadata-only setup notices. Content is rendered from closed codes.
#[cfg(test)]
use super::model_diagnostic::Reason;
use super::*;
pub const KIND: &str = "guest_model_setup_notice";

#[cfg(test)]
pub(crate) async fn enqueue(
    db: &Database,
    chat: &acks::ChatAuthority,
    reason: Reason,
) -> AppResult<()> {
    enqueue_setup_failure(db, chat, &reason.code(), reason.fix()).await
}

#[cfg(test)]
pub(crate) async fn enqueue_setup_failure(
    db: &Database,
    chat: &acks::ChatAuthority,
    code: &str,
    fix: &str,
) -> AppResult<()> {
    enqueue_failure(db, &chat.user_id, &chat.agent_id, code, fix).await
}

pub(crate) async fn enqueue_failure(
    db: &Database,
    actor: &str,
    agent_id: &str,
    code: &str,
    fix: &str,
) -> AppResult<()> {
    // Use the agent row as the cross-replica serialization fence. Claim and
    // notice insertion are atomic; a crash cannot consume a notification.
    let agent = crate::services::assistant_team_service::agent(db, actor, agent_id).await?;
    if !crate::services::assistant_guest_model_setup::eligible(db, actor, &agent).await? {
        return Ok(());
    }
    let context = (
        db.clone(),
        actor.to_owned(),
        agent_id.to_owned(),
        code.to_owned(),
        fix.to_owned(),
        agent.user_id,
    );
    crate::services::transaction_service::run_db_only(db,&context,|tx,(db,actor,agent_id,code,fix,agent_owner)| Box::pin(async move {
        crate::services::erasure_admission::fence_in_session(db,actor,tx).await?;
        // Keep the existing flat map of timestamps readable by older replicas.
        let field = format!("guest_model_notices.{}_{code}",crate::crypto::token::hash_token(actor));
        let mut filter = doc! {"_id":agent_id,"user_id":agent_owner,"destroyed_at":null};
        filter.insert("$or",vec![doc! {&field:null}, doc! {&field:{"$lte":bson::DateTime::from_chrono(Utc::now()-Duration::hours(24))}}]);
        let claim=db.collection::<Document>("assistant_agents").update_one(filter,doc! {"$set":{&field:bson::DateTime::now()}}).session(&mut *tx).await?;
        if claim.modified_count == 0 {return Ok(());}
        db.collection::<Document>(COLLECTION).insert_one(doc! {
            "_id":uuid::Uuid::new_v4().to_string(),"kind":KIND,"user_id":actor,
            "agent_id":agent_id,"reason":code,"fix":fix,"pending":true,
            "created_at":bson::DateTime::now(),"expires_at":bson::DateTime::from_chrono(Utc::now()+Duration::hours(48)),
        }).session(tx).await?;
        Ok(())
    })).await
}

/// Append a fixed notice without asking the unavailable model to write it.
pub(crate) async fn in_app(
    db: &Database,
    notice: &Document,
    conversation: &str,
    text: &str,
) -> AppResult<bool> {
    let context = (
        db.clone(),
        notice.clone(),
        conversation.to_owned(),
        text.to_owned(),
    );
    crate::services::transaction_service::run_db_only(db,&context,|tx,(db,notice,conversation,text)| Box::pin(async move {
        let id=notice.get_str("_id").map_err(|_|AppError::Internal("Invalid setup notice".into()))?;
        let owner=notice.get_str("user_id").map_err(|_|AppError::Internal("Invalid setup notice".into()))?;
        crate::services::erasure_admission::fence_in_session(db,owner,tx).await?;
        let claimed=db.collection::<Document>(COLLECTION).update_one(doc! {"_id":id,"kind":KIND,"pending":true,"app_done":{"$ne":true}},doc! {"$set":{"app_done":true}}).session(&mut *tx).await?;
        if claimed.modified_count==0 {return Ok(false);}
        let parent=db.collection::<Document>("assistant_conversations").find_one_and_update(doc! {"_id":conversation,"user_id":owner},doc! {"$inc":{"message_count":1_i64},"$set":{"updated_at":bson::DateTime::now()}}).return_document(mongodb::options::ReturnDocument::After).session(&mut *tx).await?.ok_or(AppError::AssistantTurnRequired)?;
        let seq=parent.get_i64("message_count").or_else(|_|parent.get_i32("message_count").map(i64::from)).unwrap_or(1);
        db.collection::<Document>("assistant_messages").insert_one(doc! {"_id":uuid::Uuid::new_v4().to_string(),"user_id":owner,"conversation_id":conversation,"turn_id":id,"seq":seq,"role":"assistant","status":"completed","text":text,"created_at":bson::DateTime::now()}).session(tx).await?;
        Ok(true)
    })).await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn readiness_2092_notice_is_durable_deduplicated_and_fixed_text() {
        let f = Box::pin(crate::services::assistant_authority_tests::direct_fixture(
            "readiness_notice",
        ))
        .await;
        let (a, b) = tokio::join!(
            enqueue(&f.state.db, &f.chat, Reason::RouteMismatch),
            enqueue(&f.state.db, &f.chat, Reason::RouteMismatch)
        );
        a.unwrap();
        b.unwrap();
        let filter = doc! {"kind":KIND};
        assert_eq!(
            f.state
                .db
                .collection::<Document>(COLLECTION)
                .count_documents(filter.clone())
                .await
                .unwrap(),
            1
        );
        let notice = f
            .state
            .db
            .collection::<Document>(COLLECTION)
            .find_one(filter.clone())
            .await
            .unwrap()
            .unwrap();
        assert!(notice.get_datetime("expires_at").is_ok());
        assert!(notice.get("text").is_none());
        assert!(
            in_app(&f.state.db, &notice, &f.row.id, "Fixed setup notice.")
                .await
                .unwrap()
        );
        assert!(
            !in_app(
                &f.state.db,
                &notice,
                &f.row.id,
                "Duplicate must not appear."
            )
            .await
            .unwrap()
        );
        assert_eq!(
            f.state
                .db
                .collection::<Document>("assistant_messages")
                .count_documents(doc! {"text":"Fixed setup notice."})
                .await
                .unwrap(),
            1
        );
        enqueue(&f.state.db, &f.chat, Reason::HeaderMismatch)
            .await
            .unwrap();
        assert_eq!(
            f.state
                .db
                .collection::<Document>(COLLECTION)
                .count_documents(filter.clone())
                .await
                .unwrap(),
            2
        );
        f.state.db.collection::<Document>("assistant_agents").update_one(doc! {"_id":&f.chat.agent_id},doc! {"$set":{format!("guest_model_notices.{}_route_mismatch",crate::crypto::token::hash_token(&f.owner)):bson::DateTime::from_chrono(Utc::now()-Duration::hours(25))}}).await.unwrap();
        enqueue(&f.state.db, &f.chat, Reason::RouteMismatch)
            .await
            .unwrap();
        assert_eq!(
            f.state
                .db
                .collection::<Document>(COLLECTION)
                .count_documents(filter)
                .await
                .unwrap(),
            3
        );
        f.state.db.drop().await.unwrap();
    }
}

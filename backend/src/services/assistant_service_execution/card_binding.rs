//! A delayed binding cannot recreate a card descendant after source erasure.
use super::*;

pub(crate) async fn publish(
    db: &Database,
    chat: &acks::ChatAuthority,
    id: &str,
    fields: Document,
) -> AppResult<()> {
    super::super::transaction_service::run_db_only(db,&(db,chat,id,&fields),|tx,&(db,chat,id,fields)| {
        Box::pin(async move {
            let card=db.collection::<Document>(crate::models::assistant_acknowledgement::COLLECTION_NAME)
                .find_one_and_update(doc! {"_id":id,"user_id":&chat.user_id,"conversation_id":&chat.conversation_id,
                    "api_key_id":&chat.api_key_id},doc! {"$inc":{"binding_fence":1_i64}})
                .session(&mut *tx).await?.ok_or(AppError::AssistantTurnRequired)?;
            super::super::erasure_admission::fence_in_session(db,&chat.user_id,tx).await?;
            for turn in [card.get_str("creation_turn_id").ok(),card.get_str("requested_turn_id").ok(),chat.turn_id.as_deref()]
                .into_iter().flatten() {
                for proof in super::super::channel_meta_service::scopes::assistant_turn_references(db,tx,&chat.user_id,&chat.conversation_id,turn).await? {
                    super::super::channel_meta_service::scopes::fence_reference(db,tx,&proof,true).await?;
                }
            }
            db.collection::<Document>(COLLECTION).update_one(doc! {"_id":id},doc! {"$setOnInsert":fields.clone()})
                .upsert(true).session(tx).await?;
            Ok(())
        })
    }).await
}

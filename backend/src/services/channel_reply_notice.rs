//! A transport failure is distinct from successful assistant execution.
//! Only a server-bound NyxBot event can attach a notice to its saved thread.
use super::api_key_mutation_service as tx;
use crate::{
    errors::AppResult,
    models::{
        assistant_conversation::{AssistantConversation, COLLECTION_NAME as CONVERSATIONS},
        assistant_message::{AssistantMessage, COLLECTION_NAME as MESSAGES},
        nyxbot_channel::{
            COLLECTION_NAME as CHANNELS, EVENTS_COLLECTION_NAME as EVENTS, NyxbotEvent,
        },
    },
};
use bson::doc;

pub async fn failed(
    db: &mongodb::Database,
    receipt: &str,
    bot: &str,
    inbound: &str,
) -> AppResult<()> {
    let Some(event) = db.collection::<NyxbotEvent>(EVENTS).find_one(doc! {
        "$or":[{"relay_message_id": inbound},{"event_id":inbound}],
        "delivery.guest":false, "conversation_id":{"$type":"string"}, "turn_id":{"$type":"string"},
    }).await? else { return Ok(()); };
    // A caller-controlled inbound ID alone never selects an assistant thread.
    if db
        .collection::<bson::Document>(CHANNELS)
        .find_one(doc! {
            "_id": &event.channel_id, "channel_bot_id":bot,
        })
        .await?
        .is_none()
    {
        return Ok(());
    }
    let mut session = db.client().start_session().await?;
    session.start_transaction().and_run((db, receipt, &event), |session, (db, receipt, event)| Box::pin(async move {
        let operation: AppResult<()> = async {
            let Some(record) = db.collection::<bson::Document>(crate::models::channel_reply_delivery::COLLECTION_NAME)
                .find_one_and_update(doc! {"_id":*receipt,"complete":false,"owner_notice_id":bson::Bson::Null},
                    doc! {"$set":{"owner_notice_id":uuid::Uuid::new_v4().to_string()}})
                .return_document(mongodb::options::ReturnDocument::After).session(&mut *session).await?
            else { return Ok(()); };
            let parts = record.get_array("parts").ok();
            let accepted = parts.into_iter().flatten().filter(|p| p.as_document().is_some_and(|p| p.get_str("status")==Ok("accepted"))).count();
            let total = parts.map_or(0, Vec::len);
            let unknown = parts.into_iter().flatten().any(|p| p.as_document().is_some_and(|p| p.get_str("status")==Ok("uncertain")));
            let thread = event.conversation_id.as_deref().unwrap();
            let now = chrono::Utc::now();
            let Some(conversation) = db.collection::<AssistantConversation>(CONVERSATIONS)
                .find_one_and_update(doc! {"_id":thread,"user_id":&event.user_id},
                    doc! {"$inc":{"message_count":1},"$set":{"updated_at":bson::DateTime::from_chrono(now)}})
                .return_document(mongodb::options::ReturnDocument::After).session(&mut *session).await?
            else { return Ok(()); };
            let text = format!("Your answer is saved in this thread, but Telegram delivery is incomplete. {accepted} of {total} parts were confirmed. {} Later parts were not sent. Read the full answer here and check the bot's channel health before requesting another send; confirmed or uncertain parts are not automatically resent.",
                if unknown { "The last send has an unknown outcome." } else { "A part was not delivered." });
            let notice_id = record.get_str("owner_notice_id").unwrap().to_owned();
        let message = AssistantMessage {
            automation_work: None,
            execution_task: None,
            delegated_images: Default::default(),
            media_event_ids: Vec::new(),
            media_queue_notice_id: None,
            media_pending: None,
            media_bound: None,
            platform_delivery_recovery_queued: false,
            platform_reply: None,
            platform_origin: None,
            author: None,
            retry_claimed_by: None,
            activity_count: None,
            activity_total: 0,
            report_to: None,
            id: notice_id.clone(),
            conversation_id: thread.to_owned(),
            user_id: event.user_id.clone(),
            seq: conversation.message_count,
            turn_id: notice_id.clone(),
            role: "event".into(),
            text,
            status: "completed".into(),
            error_code: None,
            upstream_error_code: None,
            error_detail: None,
            turn_metadata: None,
            created_at: now,
            activities: vec![],
            attachments: vec![],
            origin: None,
            via: None,
            continuation_of: None,
            retry_of: None,
            specialist_id: None,
            steering: None,
            voice: None,
            execution_pending: false,
        };
            db.collection::<AssistantMessage>(MESSAGES).insert_one(message).session(&mut *session).await?;
            Ok(())
        }.await;
        tx::transaction_result(operation)
    })).await.map_err(tx::map_transaction_error)
}

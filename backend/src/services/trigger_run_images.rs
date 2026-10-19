use bson::{Document, doc};
use mongodb::{ClientSession, Database};

use crate::{
    errors::AppResult,
    models::{
        assistant_conversation::AssistantConversation,
        channel_bot::COLLECTION_NAME as BOTS,
        nyxbot_channel::{COLLECTION_NAME as CHANNELS, THREADS_COLLECTION_NAME as THREADS},
        nyxbot_image_delivery::COLLECTION_NAME as CLAIMS,
        trigger_run::{COLLECTION_NAME as RUNS, RunImageRef},
        trigger_schedule::DeliverTo,
    },
    services::{assistant_nyxagent as engine, channel_image_audience, channel_image_delivery},
};

pub const MAX_RUN_IMAGES: usize = 16;

#[derive(serde::Deserialize)]
struct RunImages {
    user_id: String,
    #[serde(default)]
    deliver_to: DeliverTo,
    #[serde(default)]
    image_refs: Vec<RunImageRef>,
    #[serde(default)]
    image_refs_truncated: u32,
}

async fn audience_in_session(
    db: &Database,
    session: &mut ClientSession,
    run: &RunImages,
) -> AppResult<Option<String>> {
    let DeliverTo::Chat { chat_id } = &run.deliver_to else {
        return Ok(None);
    };
    let Some(thread) = db
        .collection::<Document>(THREADS)
        .find_one(doc! {"_id":chat_id,"user_id":&run.user_id})
        .projection(doc! {"channel_id":1,"platform_chat_id":1,"platform_thread_id":1})
        .session(&mut *session)
        .await?
    else {
        return Ok(None);
    };
    let Ok(channel_id) = thread.get_str("channel_id") else {
        return Ok(None);
    };
    let Some(channel) = db
        .collection::<Document>(CHANNELS)
        .find_one(doc! {"_id":channel_id,"user_id":&run.user_id})
        .projection(doc! {"channel_bot_id":1})
        .session(&mut *session)
        .await?
    else {
        return Ok(None);
    };
    let Ok(bot_id) = channel.get_str("channel_bot_id") else {
        return Ok(None);
    };
    let Some(bot) = db
        .collection::<Document>(BOTS)
        .find_one(doc! {"_id":bot_id})
        .projection(doc! {"platform":1})
        .session(&mut *session)
        .await?
    else {
        return Ok(None);
    };
    let (Ok(chat), Ok(platform)) = (thread.get_str("platform_chat_id"), bot.get_str("platform"))
    else {
        return Ok(None);
    };
    let mut reply = super::channel_platform::OutboundReply {
        text: None,
        metadata: None,
        attachments: Vec::new(),
        reply_to_platform_message_id: None,
    };
    if let Ok(thread) = thread.get_str("platform_thread_id") {
        reply.metadata = Some(match platform {
            "telegram" | "telegram-new" => serde_json::json!({"message_thread_id":thread}),
            "slack" => serde_json::json!({"thread_ts":thread}),
            _ => serde_json::json!({}),
        });
    }
    Ok(Some(channel_image_audience::resolve(
        bot_id, platform, chat, &reply,
    )))
}

/// The caller holds the persisted turn's settlement fence in this transaction.
pub async fn accumulate_in_session(
    db: &Database,
    session: &mut ClientSession,
    conversation: &AssistantConversation,
) -> AppResult<()> {
    let Some(turn) = conversation.active_turn.as_ref() else {
        return Ok(());
    };
    let Some(run_id) = turn.trigger_run_id.as_ref() else {
        return Ok(());
    };
    let attachments: Vec<_> = turn
        .attachments
        .iter()
        .filter(|item| engine::chat_tool_image(item))
        .collect();
    if attachments.is_empty() {
        return Ok(());
    }
    let filter = doc! {"_id":run_id,"user_id":&conversation.user_id,"thread_id":&conversation.id,"turn_id":&turn.turn_id};
    let Some(document) = db
        .collection::<Document>(RUNS)
        .find_one(filter.clone())
        .projection(doc! {"user_id":1,"deliver_to":1,"image_refs":1,"image_refs_truncated":1})
        .session(&mut *session)
        .await?
    else {
        return Ok(());
    };
    let Ok(mut run) = bson::from_document::<RunImages>(document) else {
        tracing::warn!(run_id, "Automation image metadata is invalid");
        return Ok(());
    };
    let audience = audience_in_session(db, session, &run).await?;
    for attachment in attachments {
        if run
            .image_refs
            .iter()
            .any(|item| item.attachment.id == attachment.id)
        {
            continue;
        }
        if run.image_refs.len() >= MAX_RUN_IMAGES {
            let mut evict = None;
            if let Some(audience) = audience.as_ref() {
                for (index, item) in run.image_refs.iter().enumerate() {
                    let terminal=db.collection::<Document>(CLAIMS).find_one(doc! {
                        "_id":channel_image_delivery::image_delivery_id(&run.user_id,&item.attachment.id,audience),
                        "state":{"$in":["sent","failed","skipped","unknown"]},
                    }).projection(doc! {"_id":1}).session(&mut *session).await?.is_some();
                    if terminal {
                        evict = Some(index);
                        break;
                    }
                }
            }
            if let Some(index) = evict {
                run.image_refs.remove(index);
            } else {
                run.image_refs_truncated = run.image_refs_truncated.saturating_add(1);
                continue;
            }
        }
        run.image_refs.push(RunImageRef {
            conversation_id: conversation.id.clone(),
            turn_id: turn.turn_id.clone(),
            attachment: attachment.clone(),
        });
    }
    db.collection::<Document>(RUNS).update_one(filter,doc! {"$set":{
        "image_refs":bson::to_bson(&run.image_refs).map_err(|_|crate::errors::AppError::Internal("Automation image metadata unavailable".into()))?,"image_refs_truncated":i64::from(run.image_refs_truncated),
    }}).session(&mut *session).await?;
    Ok(())
}

#[cfg(test)]
#[path = "trigger_run_images_tests.rs"]
mod tests;

//! One org-owned public transcript per chat/topic; keys and private execution
//! history remain per actor. Reuses the org-group message shape and renderer.
use crate::{
    errors::{AppError, AppResult},
    models::{
        assistant_conversation::{AssistantConversation, ChannelOrigin},
        assistant_group::GroupMessage,
        org_channel::{LANES, MESSAGES},
    },
};
use bson::{Document, doc};
use futures::TryStreamExt;
use mongodb::{ClientSession, Database};

/// A verified gateway private chat adopts the relay partition atomically with
/// its actor-owned histories. Retain the relay alias for flag-off return. Never
/// merge an already established gateway conversation or another binding's lanes.
pub(crate) async fn adopt_private_partition(
    db: &Database,
    channel: &crate::models::nyxbot_channel::NyxbotChannel,
    relay: &str,
    gateway: &str,
) -> AppResult<()> {
    let context = (
        db.clone(),
        channel.clone(),
        relay.to_owned(),
        gateway.to_owned(),
    );
    crate::services::transaction_service::run_db_only(db, &context, |tx, (db, channel, relay, gateway)| Box::pin(async move {
        super::fence_management(db, tx, channel, &channel.user_id).await?;
        let binding = channel.org_specialist.as_ref().ok_or_else(super::denied)?;
        let threads = db.collection::<crate::models::nyxbot_channel::NyxbotThread>("nyxbot_threads");
        let Some(legacy) = threads.find_one(doc!{"channel_id":&channel.id,"user_id":&channel.user_id,"kind":"private",
            "$or":[{"partition":relay},{"relay_partition":relay}]}).session(&mut *tx).await? else {return Ok(())};
        if legacy.partition == *gateway {return Ok(())}
        if threads.find_one(doc!{"channel_id":&channel.id,"partition":gateway,"kind":{"$ne":null}})
            .session(&mut *tx).await?.is_some() {return Ok(())}
        threads.delete_many(doc!{"channel_id":&channel.id,"partition":gateway,"kind":null}).session(&mut *tx).await?;
        let changed = threads.update_one(doc!{"_id":&legacy.id,"partition":&legacy.partition},
            doc!{"$set":{"partition":gateway,"relay_partition":relay,"updated_at":bson::DateTime::now()}})
            .session(&mut *tx).await?;
        if changed.matched_count != 1 {return Err(super::denied())}
        for field in ["channel", "reply_channel"] {
            db.collection::<Document>("assistant_conversations").update_many(doc!{
                format!("{field}.nyxbot_channel_id"):&channel.id,
                format!("{field}.partition"):&legacy.partition,
                format!("{field}.org_execution.org_id"):&binding.org_id,
                format!("{field}.org_execution.revision"):&binding.revision,
            },doc!{"$set":{format!("{field}.partition"):gateway}}).session(&mut *tx).await?;
        }
        db.collection::<Document>(LANES).update_many(doc!{"channel_id":&channel.id,"partition":&legacy.partition,
            "org_id":&binding.org_id,"binding_revision":&binding.revision},
            doc!{"$set":{"partition":gateway}}).session(tx).await?;
        Ok(())
    })).await
}

pub(crate) async fn lane(
    db: &Database,
    origin: &ChannelOrigin,
    actor: &str,
    agent: &str,
) -> AppResult<(String, bool)> {
    let e = origin.org_execution.as_ref().ok_or_else(super::denied)?;
    let filter = doc! {"channel_id":&origin.nyxbot_channel_id,"partition":&origin.partition,"actor_id":actor,
    "agent_id":agent,"binding_revision":&e.revision,"guest_revision":&e.guest_revision,"sender_hash":if e.guest {e.sender_hash.as_str()} else {""}};
    let rows = db.collection::<Document>(LANES);
    let result=rows.update_one(filter.clone(),doc!{"$setOnInsert":{"_id":uuid::Uuid::new_v4().to_string(),
        "org_id":&e.org_id,"conversation_id":format!("nyxa-{}",uuid::Uuid::new_v4().simple()),"created_at":bson::DateTime::now()}}).upsert(true).await;
    if let Err(error) = result
        && !crate::services::assistant_team_service::is_duplicate(&error)
    {
        return Err(error.into());
    }
    let lane = rows.find_one(filter).await?.ok_or_else(super::denied)?;
    let id = lane
        .get_str("conversation_id")
        .map_err(|_| super::denied())?
        .to_owned();
    let exists = db
        .collection::<Document>("assistant_conversations")
        .find_one(doc! {"_id":&id,"user_id":actor})
        .await?
        .is_some();
    Ok((id, exists))
}

pub(crate) async fn context(db: &Database, origin: &ChannelOrigin) -> AppResult<String> {
    let e = origin.org_execution.as_ref().ok_or_else(super::denied)?;
    let mut rows: Vec<GroupMessage> = db
        .collection(MESSAGES)
        .find(doc! {"user_id":&e.org_id,"group_id":group_id(origin)})
        .sort(doc! {"seq":-1})
        .limit(40)
        .await?
        .try_collect()
        .await?;
    rows.reverse();
    Ok(format!(
        "\nEveryone in this chat sees only the public transcript below. Never publish private memory, tool traces or another actor's cards. Only the initiating verified member can decide their cards.\n{}",
        crate::services::org_group_service::render_transcript(&rows)
    ))
}

fn group_id(origin: &ChannelOrigin) -> String {
    crate::crypto::token::hash_token(&format!(
        "org-channel-transcript:{}:{}",
        origin.nyxbot_channel_id, origin.partition
    ))
}

pub(crate) async fn publish(
    db: &Database,
    tx: &mut ClientSession,
    row: &AssistantConversation,
    id: &str,
    text: &str,
    role: &str,
) -> AppResult<()> {
    if row.guest_model_setup {
        return Ok(());
    }
    let Some(origin) = super::origin(row) else {
        return Ok(());
    };
    let e = origin.org_execution.as_ref().ok_or_else(super::denied)?;
    // Direct/private history is never copied into a public group transcript.
    let Some(thread)=db.collection::<Document>("nyxbot_threads").find_one_and_update(doc!{
        "channel_id":&origin.nyxbot_channel_id,"partition":&origin.partition,"kind":{"$ne":"private"}},
        doc!{"$inc":{"public_message_seq":1_i64}}).return_document(mongodb::options::ReturnDocument::After).session(&mut *tx).await? else {return Ok(())};
    let message = GroupMessage {
        collaboration: None,
        activities: Vec::new(),
        org_group: true,
        author_user_id: (!e.guest && role == "user").then(|| e.actor_id.clone()),
        author_display_name: Some(
            if role == "agent" {
                "Assistant"
            } else if e.guest {
                "Visitor"
            } else {
                "Member"
            }
            .into(),
        ),
        request_id: row.active_turn.as_ref().map(|t| t.turn_id.clone()),
        attachments: Vec::new(),
        id: id.into(),
        group_id: group_id(origin),
        user_id: e.org_id.clone(),
        seq: thread.get_i64("public_message_seq").unwrap_or(1),
        role: role.into(),
        agent_id: (role == "agent").then(|| e.agent_id.clone()),
        agent_name: None,
        text: crate::services::assistant_nyxagent::excerpt(text, 12000),
        recipient_agent_ids: Vec::new(),
        mentioned_participant_ids: Vec::new(),
        created_at: chrono::Utc::now(),
    };
    let mut document = bson::to_document(&message)
        .map_err(|_| AppError::Internal("Channel transcript unavailable".into()))?;
    document.insert("channel_id", &origin.nyxbot_channel_id);
    document.insert("partition", &origin.partition);
    document.insert("conversation_id", &row.id);
    document.insert("actor_id", &e.actor_id);
    db.collection::<Document>(MESSAGES)
        .update_one(doc! {"_id":id}, doc! {"$setOnInsert":document})
        .upsert(true)
        .session(tx)
        .await?;
    Ok(())
}

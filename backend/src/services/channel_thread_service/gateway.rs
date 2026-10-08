//! Verified Agent Event Gateway facts. No opaque gateway refs enter metadata.
use super::*;
use crate::models::{
    channel_message::{COLLECTION_NAME as MESSAGES, ChannelMessage},
    channel_thread::ThreadSenderKind,
};
use serde_json::Value;

pub fn negotiated(channel: &NyxbotChannel) -> bool {
    channel.transport == "gateway"
        && channel.gateway_threads.supported
        && channel.gateway_threads.version == Some(1)
        && matches!(channel.platform.as_str(), "lark" | "feishu")
}
pub fn supports(channel: &NyxbotChannel) -> bool {
    channel.transport == "direct" || negotiated(channel)
}

/// Management-time projection keeps dormant and explicitly non-follow chats
/// out of all metadata, membership and follow database work.
pub fn candidate(channel: &NyxbotChannel, activity: &Value) -> bool {
    let chat = activity["conversation"]["id"].as_str().unwrap_or_default();
    negotiated(channel)
        && matches!(
            activity["conversation"]["kind"].as_str(),
            Some("group" | "channel")
        )
        && activity["thread"]["version"] == 1
        && (channel.gateway_threads.enabled
            || channel
                .gateway_threads
                .follow_chat_ids
                .iter()
                .any(|id| id == chat))
        && !channel
            .gateway_threads
            .excluded_chat_ids
            .iter()
            .any(|id| id == chat)
}

/// Correlate the authenticated provider projection to the original NyxID
/// ingress row before sharing direct-relay follow policy and delivery fences.
pub async fn retain(
    db: &mongodb::Database,
    channel: &NyxbotChannel,
    activity: &Value,
) -> AppResult<bool> {
    if !negotiated(channel) || activity["conversation"]["kind"] == "private" {
        return Ok(false);
    }
    let Ok(facts) = serde_json::from_value::<ChannelThreadFacts>(activity["thread"].clone()) else {
        return Ok(false);
    };
    if facts.version != 1
        || facts.kind != ThreadKind::Native
        || facts.sender_kind != ThreadSenderKind::Human
        || facts.parent_chat_id.is_some()
        || !facts.participant_hashes.is_empty()
        || facts.sender_hash.is_some()
        || activity["actor"]["kind"] != "human"
        || activity["source"]["type"] != "nyxid_relay"
        || activity["source"]["platform"].as_str() != Some(channel.platform.as_str())
        || activity["source"]["route_id"].as_str() != channel.route_id.as_deref()
        || activity["conversation"]["id"].as_str() != Some(facts.chat_id.as_str())
    {
        return Ok(false);
    }
    let Some(id) = activity["event_id"]
        .as_str()
        .filter(|id| uuid::Uuid::parse_str(id).is_ok())
    else {
        return Ok(false);
    };
    let Some(sender) = activity["actor"]["id"].as_str().filter(|s| !s.is_empty()) else {
        return Ok(false);
    };
    let owner = channel.bot_owner_id.as_deref().unwrap_or(&channel.user_id);
    let Some(source) = db.collection::<ChannelMessage>(MESSAGES).find_one(doc! {
        "_id":id,"direction":"inbound","channel_bot_id":&channel.channel_bot_id,
        "user_id":owner,"agent_api_key_id":&channel.route_api_key_id,
        "conversation_id":&channel.route_id,"platform":&channel.platform,"sender_platform_id":sender,
    }).await? else { return Ok(false); };
    if !super::resolution::source_matches(&facts, &source) {
        return Ok(false);
    }
    // Whole projection is replaced only on this source, before selection. Bodies,
    // mentioned identities and sealed references are deliberately not copied.
    db.collection::<ChannelMessage>(MESSAGES)
        .update_one(
            doc! {"_id":id},
            doc! {"$set":{"thread_context":bson::to_bson(&facts).map_err(|_| unavailable())?}},
        )
        .await?;
    Ok(true)
}

/// A gateway can request only a target reconstructed from original persisted
/// ingress and (for admitted work) its exact live child binding.
pub async fn reply_target(
    db: &mongodb::Database,
    adapter: &dyn PlatformAdapter,
    bot: &ChannelBot,
    credentials: &crate::services::channel_platform::BotCredentials<'_>,
    source: &ChannelMessage,
) -> AppResult<Option<ThreadReplyTarget>> {
    use crate::models::nyxbot_channel::{EVENTS_COLLECTION_NAME as EVENTS, NyxbotEvent};
    let Some(channel) = db
        .collection::<NyxbotChannel>(CHANNELS)
        .find_one(doc! {
            "channel_bot_id":&bot.id,"route_api_key_id":&source.agent_api_key_id,
            "status":"active","transport":"gateway","gateway_threads.version":1,
        })
        .await?
    else {
        return Ok(None);
    };
    let message = source.id.as_str();
    if !negotiated(&channel) {
        return Ok(None);
    }
    if let Some(event) = db.collection::<NyxbotEvent>(EVENTS).find_one(doc! {
        "channel_id":&channel.id,"event_id":message,"delivery.origin.thread.source_message_id":message,
    }).await?
        && let Some(delivery) = event.delivery
        && let Some(conversation) = event.resolved_conversation_id
    {
        return super::resolution::resolve_admitted(db, adapter, bot, credentials,
            &channel.user_id, &delivery.origin, &conversation).await;
    }
    super::resolution::resolve_retained(db, adapter, bot, credentials, message).await
}

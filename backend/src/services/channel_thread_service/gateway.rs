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
    channel.transport == "direct"
        || negotiated(channel)
        || (channel.platform == "discord" && crate::services::nyxbot_gateway_source::ready(channel))
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
        || !matches!(
            facts.sender_kind,
            ThreadSenderKind::Human | ThreadSenderKind::Bot
        )
        || facts.parent_chat_id.is_some()
        || !facts.participant_hashes.is_empty()
        || facts.sender_hash.is_some()
        || activity["actor"]["kind"].as_str()
            != Some(match facts.sender_kind {
                ThreadSenderKind::Human => "human",
                ThreadSenderKind::Bot => "bot",
                _ => "unknown",
            })
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
    if source
        .thread_context
        .as_ref()
        .is_some_and(|old| old.sender_kind != facts.sender_kind)
        || !super::resolution::source_matches(&facts, &source)
    {
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
pub(crate) async fn reply_target(
    db: &mongodb::Database,
    adapter: &dyn PlatformAdapter,
    bot: &ChannelBot,
    credentials: &crate::services::channel_platform::BotCredentials<'_>,
    source: &ChannelMessage,
) -> ReplyResult<ThreadReplyTarget> {
    use crate::models::nyxbot_channel::{EVENTS_COLLECTION_NAME as EVENTS, NyxbotEvent};
    let Some(channel) = db
        .collection::<NyxbotChannel>(CHANNELS)
        .find_one(doc! {
            "channel_bot_id":&bot.id,"route_api_key_id":&source.agent_api_key_id,
            "status":"active","transport":"gateway","gateway_threads.version":1,
        })
        .await?
    else {
        return Err(ReplyError::target(reply_unavailable()));
    };
    let message = source.id.as_str();
    if !negotiated(&channel) {
        return Err(ReplyError::target(reply_unavailable()));
    }
    if let Some(event) = db.collection::<NyxbotEvent>(EVENTS).find_one(doc! {
        "channel_id":&channel.id,"event_id":message,"delivery.origin.thread.source_message_id":message,
    }).await? {
        let (owner, origin, conversation) = admitted_reply(&event, &channel, message).map_err(ReplyError::target)?;
        crate::services::channel_thread_follow_service::validate_delivery(db, owner, origin, conversation)
            .await.map_err(ReplyError::authority)?;
        let mut target = super::resolution::resolve_for_reply(db, adapter, bot, credentials, message).await?;
        target.admitted = Some((owner.into(), origin.clone(), conversation.into()));
        return Ok(target);
    }
    super::resolution::resolve_for_reply(db, adapter, bot, credentials, message).await
}

/// The committed admission is authoritative, including events from old
/// replicas whose resolved ID still names an unused child reservation.
pub(crate) fn admitted_reply<'a>(
    event: &'a crate::models::nyxbot_channel::NyxbotEvent,
    channel: &NyxbotChannel,
    source: &str,
) -> AppResult<(
    &'a str,
    &'a crate::models::assistant_conversation::ChannelOrigin,
    &'a str,
)> {
    let origin = &event.delivery.as_ref().ok_or_else(unavailable)?.origin;
    let binding = origin.thread.as_deref().ok_or_else(unavailable)?;
    if event.channel_id != channel.id
        || event.user_id != channel.user_id
        || event.event_id != source
        || binding.source_message_id != source
        || event.resolved_thread_id.as_deref() != Some(&binding.child_id)
        || origin.nyxbot_channel_id != channel.id
        || origin.platform != channel.platform
    {
        return Err(unavailable());
    }
    let conversation = event
        .conversation_id
        .as_deref()
        .or_else(|| {
            origin
                .org_execution
                .is_none()
                .then_some(event.resolved_conversation_id.as_deref())
                .flatten()
        })
        .ok_or_else(unavailable)?;
    Ok((
        crate::services::org_channel_service::event_actor(event),
        origin,
        conversation,
    ))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ReplyFailure {
    Target,
    Send,
    Delayed,
    Expired,
    Transient,
    Authorization,
}

impl ReplyFailure {
    pub(crate) fn reason(self) -> Option<&'static str> {
        match self {
            Self::Target => Some("thread_target_rejected"),
            Self::Send => Some("thread_reply_failed"),
            Self::Delayed => Some("thread_delayed_reply_failed"),
            Self::Expired => Some("thread_reply_reference_expired"),
            Self::Transient | Self::Authorization => None,
        }
    }
}

#[derive(Debug)]
pub(crate) struct ReplyError {
    pub error: crate::errors::AppError,
    pub class: ReplyFailure,
}

pub(crate) type ReplyResult<T> = Result<T, ReplyError>;

impl From<crate::errors::AppError> for ReplyError {
    fn from(error: crate::errors::AppError) -> Self {
        let class = if matches!(
            error,
            crate::errors::AppError::Unauthorized(_) | crate::errors::AppError::Forbidden(_)
        ) {
            ReplyFailure::Authorization
        } else {
            ReplyFailure::Transient
        };
        Self { error, class }
    }
}

impl From<mongodb::error::Error> for ReplyError {
    fn from(error: mongodb::error::Error) -> Self {
        crate::errors::AppError::from(error).into()
    }
}

impl ReplyError {
    pub(crate) fn target(error: crate::errors::AppError) -> Self {
        Self {
            error,
            class: ReplyFailure::Target,
        }
    }

    /// Call only at a live target/authority validator, never at caller auth or
    /// provider lookup. These variants denote that validator's closed refusal.
    pub(crate) fn authority(error: crate::errors::AppError) -> Self {
        use crate::errors::AppError;
        match error {
            AppError::NotFound(_)
            | AppError::Forbidden(_)
            | AppError::Conflict(_)
            | AppError::ChannelBotNotFound(_)
            | AppError::ChannelBotInactive(_)
            | AppError::ChannelConversationNotAddressable
            | AppError::ChannelConversationNotReachable(_) => Self::target(error),
            _ => Self::from(error),
        }
    }
}

/// Preflight failures add diagnostics only for known target refusals. Used
/// capabilities and inconclusive lookups never alter bot health.
pub(crate) async fn record_failure(
    db: &mongodb::Database,
    bot: &str,
    owner: &str,
    source: &str,
    class: ReplyFailure,
    error: &crate::errors::AppError,
) {
    if class.reason().is_some() {
        record_outcome(db, bot, owner, source, class, error).await;
    }
}

/// The part sender always preserves main's outbound diagnostic. Thread detail
/// is additive; even an unclassified failure after the barrier is retained.
pub(crate) async fn record_outcome(
    db: &mongodb::Database,
    bot: &str,
    owner: &str,
    source: &str,
    class: ReplyFailure,
    error: &crate::errors::AppError,
) {
    crate::services::channel_bot_service::record_outbound_outcome_with_reason(
        db,
        bot,
        Some(error),
        Some(class),
    )
    .await;
    let Some(reason) = class.reason() else {
        return;
    };
    let actor = crate::services::audit_service::AuditActor {
        user_id: owner.into(),
        ip_address: None,
        user_agent: None,
        api_key_id: None,
        api_key_name: None,
    };
    let _ = crate::services::audit_service::log_actor_event(db.clone(), &actor,
        "nyxbot_thread_reply_failed", Some(serde_json::json!({
            "channel_bot_id":bot,"source_message_id":source,"reason":reason,"error_code":error.error_code()
        }))).await;
}

pub(super) fn reply_unavailable() -> crate::errors::AppError {
    crate::errors::AppError::Conflict("thread_target_unavailable".into())
}

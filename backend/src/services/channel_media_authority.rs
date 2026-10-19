//! Owner-only authority for native channel media. Provider handles come solely
//! from authenticated NyxID ingress metadata, never from a gateway attachment.
use crate::{
    errors::{AppError, AppResult},
    models::{
        assistant_conversation::ChannelOrigin,
        channel_bot::{COLLECTION_NAME as BOTS, ChannelBot},
        channel_message::{COLLECTION_NAME as MESSAGES, ChannelMessage},
        nyxbot_channel::{
            COLLECTION_NAME as CHANNELS, NyxbotChannel, NyxbotThread,
            THREADS_COLLECTION_NAME as THREADS,
        },
    },
};
use mongodb::{Database, bson::doc};

/// Captured routing facts. Reuse the original request on every fetch/retry so a
/// relink cannot silently replace the admitted authority.
#[derive(Clone)]
pub struct MediaRequest {
    pub channel: NyxbotChannel,
    pub source_message_id: String,
    pub chat_id: String,
    pub sender_id: String,
    pub native_thread_id: Option<String>,
    pub origin: Option<ChannelOrigin>,
    pub conversation_id: Option<String>,
    pub agent_id: Option<String>,
}
impl std::fmt::Debug for MediaRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("MediaRequest([REDACTED])")
    }
}

pub struct AuthorizedMedia {
    pub source: ChannelMessage,
    pub bot: ChannelBot,
}
impl std::fmt::Debug for AuthorizedMedia {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AuthorizedMedia([REDACTED])")
    }
}
fn refused() -> AppError {
    AppError::NotFound("Channel media unavailable".into())
}

/// Source presence is separate from owner eligibility: this lookup also works
/// for guests, so later admission cannot mistake a card-like caption for a
/// decision. It does not authorize downloads.
pub async fn source(db: &Database, request: &MediaRequest) -> AppResult<ChannelMessage> {
    let channel = &request.channel;
    if !matches!(channel.platform.as_str(), "lark" | "feishu") {
        return Err(refused());
    }
    let mut filter = doc! {
        "_id": &request.source_message_id, "direction": "inbound",
        "channel_bot_id": &channel.channel_bot_id,
        "user_id": channel.bot_owner_id.as_deref().unwrap_or(&channel.user_id),
        "platform": &channel.platform, "agent_api_key_id": &channel.route_api_key_id,
        "platform_conversation_id": &request.chat_id, "sender_platform_id": &request.sender_id,
        "thread_id": &request.native_thread_id,
    };
    if let Some(route) = &channel.route_id {
        filter.insert("conversation_id", route);
    }
    db.collection::<ChannelMessage>(MESSAGES)
        .find_one(filter)
        .await?
        .ok_or_else(refused)
}

/// All live checks precede credential materialization and provider I/O. Call
/// immediately before each fetch, including a larger-limit retry.
pub async fn resolve(db: &Database, request: &MediaRequest) -> AppResult<AuthorizedMedia> {
    let source = source(db, request).await?;
    let captured = &request.channel;
    let channel = db
        .collection::<NyxbotChannel>(CHANNELS)
        .find_one(doc! {"_id": &captured.id, "user_id": &captured.user_id, "status":"active"})
        .await?
        .ok_or_else(refused)?;
    if channel.channel_bot_id != captured.channel_bot_id
        || channel.platform != captured.platform
        || channel.bot_owner_id != captured.bot_owner_id
        || channel.route_api_key_id != captured.route_api_key_id
        || channel.route_id != captured.route_id
        || channel.agent_id != captured.agent_id
        || channel.follow_binding_generation != captured.follow_binding_generation
        || !channel.owner_sender_ids.contains(&request.sender_id)
    {
        return Err(refused());
    }
    if db
        .collection::<mongodb::bson::Document>(crate::models::user::COLLECTION_NAME)
        .find_one(doc! {"_id": &channel.user_id, "is_active":true})
        .await?
        .is_none()
    {
        return Err(refused());
    }
    if let Some(org) = &channel.bot_owner_id
        && (db
            .collection::<mongodb::bson::Document>(crate::models::user::COLLECTION_NAME)
            .find_one(doc! {"_id":org,"is_active":true})
            .await?
            .is_none()
            || !super::org_service::is_admin(db, &channel.user_id, org).await?)
    {
        return Err(refused());
    }
    super::platform_bot_authority::require_personal_routes(db, &channel.channel_bot_id).await?;
    let bot = db.collection::<ChannelBot>(BOTS).find_one(doc! {
        "_id": &channel.channel_bot_id, "user_id": channel.bot_owner_id.as_deref().unwrap_or(&channel.user_id),
        "platform": &channel.platform, "is_active":true, "status":"active"
    }).await?.ok_or_else(refused)?;
    let route = super::channel_routing_service::resolve_agent(
        db,
        &bot.id,
        &request.chat_id,
        Some(&request.sender_id),
        &bot.user_id,
    )
    .await?
    .ok_or_else(refused)?;
    if route.api_key_id != channel.route_api_key_id
        || route.conversation.id != source.conversation_id
    {
        return Err(refused());
    }
    let mut requires_nyxbot = false;
    if let Some(origin) = &request.origin {
        if origin.nyxbot_channel_id != channel.id || origin.platform != channel.platform {
            return Err(refused());
        }
        let chat = db.collection::<NyxbotThread>(THREADS).find_one(doc! {
            "channel_id": &channel.id, "user_id": &channel.user_id, "partition": &origin.partition,
            "platform_chat_id": &request.chat_id,
        }).await?.ok_or_else(refused)?;
        // Native children persist a canonical root and bound_agent_id. Their
        // agent_id is only a request projection; platform_thread_id belongs to
        // legacy partitions. The source already checked the exact native alias.
        if origin.thread.is_none() {
            if chat.platform_thread_id != request.native_thread_id {
                return Err(refused());
            }
            let effective = chat.agent_id.as_ref().or(channel.agent_id.as_ref());
            if effective.is_some() && effective != request.agent_id.as_ref() {
                return Err(refused());
            }
            requires_nyxbot = effective.is_none();
        }
        if let Some(binding) = origin.thread.as_deref() {
            if binding.guest
                || request.agent_id.as_deref() != Some(binding.agent_id.as_str())
                || binding.source_message_id != source.id
                || binding.sender_id != request.sender_id
            {
                return Err(refused());
            }
            super::channel_thread_follow_service::validate_delivery(
                db,
                &channel.user_id,
                origin,
                request.conversation_id.as_deref().ok_or_else(refused)?,
            )
            .await?;
        }
    }
    if let Some(id) = request.agent_id.as_ref().or(channel.agent_id.as_ref()) {
        let agent = super::assistant_team_service::agent(db, &channel.user_id, id).await?;
        if agent.destroyed_at.is_some()
            || (requires_nyxbot && !agent.is_nyxbot())
            || !super::org_agent_service::can_use(
                &super::org_agent_service::access(db, &channel.user_id, &agent.user_id).await?,
            )
        {
            return Err(refused());
        }
    }
    Ok(AuthorizedMedia { source, bot })
}

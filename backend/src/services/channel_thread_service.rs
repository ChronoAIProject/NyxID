//! Dormant direct-thread infrastructure. PR C will call these operations after
//! sender admission and the serial turn claim. No legacy caller supplies a target.
#![allow(dead_code)] // PR B contracts are deliberately dormant until PR C.
pub mod contracts;
pub mod delivery;
pub mod gateway;
pub mod history;
pub mod resolution;
use bson::doc;
pub use contracts::*;

use crate::{
    errors::AppResult,
    models::{
        channel_bot::ChannelBot,
        channel_thread::{ChannelThreadFacts, ThreadKind},
        nyxbot_channel::{COLLECTION_NAME as CHANNELS, NyxbotChannel},
    },
    services::{
        channel_platform::{InboundMessage, PlatformAdapter},
        feature_flag_service::{self, NYXBOT_THREAD_FOLLOW_FLAG_KEY},
    },
};

/// Never treat legacy interaction credentials, control text or empty IDs as roots.
pub(crate) fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 2048
        && !value.chars().any(char::is_control)
        && !value.starts_with("interaction:")
}

pub(crate) fn unavailable() -> crate::errors::AppError {
    crate::errors::AppError::ChannelConversationNotReachable(
        "Bound thread operation unavailable".into(),
    )
}

pub(crate) fn valid_facts(facts: &ChannelThreadFacts, inbound: &InboundMessage) -> bool {
    facts.version == 1
        && facts.chat_id == inbound.conversation_id
        && facts.message_id == inbound.platform_message_id
        && valid_id(&facts.chat_id)
        && valid_id(&facts.message_id)
        && [
            &facts.parent_chat_id,
            &facts.root_id,
            &facts.native_thread_id,
            &facts.parent_message_id,
        ]
        .into_iter()
        .all(|id| id.as_deref().is_none_or(valid_id))
        && facts.participant_hashes.len() <= 64
        && facts
            .participant_hashes
            .iter()
            .all(|hash| hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()))
        && facts
            .sender_hash
            .as_ref()
            .is_none_or(|hash| hash.len() == 64 && hash.bytes().all(|b| b.is_ascii_hexdigit()))
        && (facts.kind != ThreadKind::Unknown || facts.root_id.is_none())
}

#[derive(Default)]
pub(crate) struct InboundContext {
    pub thread: Option<ChannelThreadFacts>,
    pub telegram_group: Option<super::channel_inbound_diagnostics::TelegramRoute>,
}

pub(crate) async fn inbound_facts(
    db: &mongodb::Database,
    bot: &ChannelBot,
    route_api_key_id: &str,
    adapter: &dyn PlatformAdapter,
    inbound: &InboundMessage,
) -> AppResult<Option<ChannelThreadFacts>> {
    Ok(
        inbound_context(db, bot, route_api_key_id, None, adapter, inbound)
            .await?
            .thread,
    )
}

/// The existing active-channel lookup also identifies the receiver of an addressed
/// Telegram group message. Only direct links may put thread facts on the callback.
pub(crate) async fn inbound_context(
    db: &mongodb::Database,
    bot: &ChannelBot,
    route_api_key_id: &str,
    route_id: Option<&str>,
    adapter: &dyn PlatformAdapter,
    inbound: &InboundMessage,
) -> AppResult<InboundContext> {
    use super::channel_inbound_diagnostics::TelegramRoute;
    use crate::models::channel_thread::{ThreadAddress, ThreadSenderKind};
    let Some(facts) = adapter
        .thread_facts(inbound, bot, None)
        .filter(|facts| valid_facts(facts, inbound))
    else {
        return Ok(InboundContext::default());
    };
    let direct = doc! {"transport":"direct", "route_api_key_id":route_api_key_id};
    let mut filter = doc! {"channel_bot_id": &bot.id, "status":"active"};
    let telegram_group = route_id.is_some()
        && inbound.conversation_type == "group"
        && super::channel_platform::canonical_platform(&bot.platform) == "telegram";
    if telegram_group {
        let gateway = doc! {"transport":"gateway", "gateway_source":null, "route_id":{"$type":"string"},
        "platform":"telegram", "$or":[{"bot_owner_id":&bot.user_id},
            {"bot_owner_id":null,"user_id":&bot.user_id}]};
        filter.insert("$or", vec![direct, gateway]);
    } else {
        filter.extend(direct);
    }
    #[derive(serde::Deserialize)]
    struct Link {
        #[serde(rename = "_id")]
        id: String,
        user_id: String,
        transport: String,
        route_id: Option<String>,
        route_api_key_id: String,
    }
    let links = db.collection::<Link>(CHANNELS);
    let mut query = links
        .find_one(filter)
        .projection(doc! {"user_id":1,"transport":1,"route_id":1,"route_api_key_id":1});
    if telegram_group {
        // A matching direct link retains priority for the existing thread projection.
        query = query.sort(doc! {"transport":1, "created_at":-1, "_id":-1});
    }
    let Some(channel) = query.await? else {
        return Ok(InboundContext::default());
    };
    if channel.transport == "gateway" {
        let directed = facts.sender_kind == ThreadSenderKind::Human
            && matches!(
                facts.address,
                ThreadAddress::Mention | ThreadAddress::ReplyToBot
            );
        let route = if channel.route_api_key_id != route_api_key_id {
            TelegramRoute::OtherAgent { directed }
        } else if channel.route_id.as_deref() != route_id {
            TelegramRoute::OutsideSource { directed }
        } else {
            TelegramRoute::Bound
        };
        return Ok(InboundContext {
            thread: None,
            telegram_group: Some(route),
        });
    }
    if !feature_flag_service::personal_flag_enabled(
        db,
        &channel.user_id,
        NYXBOT_THREAD_FOLLOW_FLAG_KEY,
    )
    .await?
        && db
            .collection::<crate::models::nyxbot_channel::NyxbotThread>(
                crate::models::nyxbot_channel::THREADS_COLLECTION_NAME,
            )
            .find_one(doc! {"channel_id": &channel.id, "record_scope": "platform_thread"})
            .await?
            .is_none()
    {
        return Ok(InboundContext::default());
    }
    Ok(InboundContext {
        thread: Some(facts),
        telegram_group: None,
    })
}

#[cfg(test)]
pub(crate) mod operation_tests;
#[cfg(test)]
mod tests;

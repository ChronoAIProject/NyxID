//! Dormant direct-thread infrastructure. PR C will call these operations after
//! sender admission and the serial turn claim. No legacy caller supplies a target.
#![allow(dead_code)] // PR B contracts are deliberately dormant until PR C.
pub mod contracts;
pub mod delivery;
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

/// The flag belongs to the linked acting person, including for an org-owned bot.
/// No fallback to gateway/third-party routes, and no credential or provider I/O.
pub(crate) async fn inbound_facts(
    db: &mongodb::Database,
    bot: &ChannelBot,
    route_api_key_id: &str,
    adapter: &dyn PlatformAdapter,
    inbound: &InboundMessage,
) -> AppResult<Option<ChannelThreadFacts>> {
    let Some(facts) = adapter
        .thread_facts(inbound, bot, None)
        .filter(|facts| valid_facts(facts, inbound))
    else {
        return Ok(None);
    };
    let Some(channel) = db
        .collection::<NyxbotChannel>(CHANNELS)
        .find_one(doc! {
            "channel_bot_id": &bot.id, "route_api_key_id": route_api_key_id,
            "transport": "direct", "status": "active",
        })
        .await?
    else {
        return Ok(None);
    };
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
        return Ok(None);
    }
    Ok(Some(facts))
}

#[cfg(test)]
pub(crate) mod operation_tests;
#[cfg(test)]
mod tests;

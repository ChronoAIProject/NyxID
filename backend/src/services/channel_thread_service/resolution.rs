use super::*;
use crate::models::channel_message::{COLLECTION_NAME as MESSAGES, ChannelMessage};
use crate::services::channel_platform::BotCredentials;

/// Provider origins are adapter-owned. This client never follows redirects,
/// including redirects during tenant-token exchange.
pub(crate) fn client() -> AppResult<reqwest::Client> {
    reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(std::time::Duration::from_secs(HISTORY_SECONDS))
        .build()
        .map_err(|_| unavailable())
}

pub(super) async fn eligible_source_live(
    db: &mongodb::Database,
    bot: &ChannelBot,
    source: &ChannelMessage,
) -> AppResult<bool> {
    if !bot.is_active
        || bot.status != "active"
        || source.direction != "inbound"
        || source.channel_bot_id.as_deref() != Some(&bot.id)
        || source.user_id != bot.user_id
        || source.platform != bot.platform
    {
        return Ok(false);
    }
    crate::services::ownership_transfer_service::require_current_bot(db, bot).await?;
    let Some(key) = source.agent_api_key_id.as_deref() else {
        return Ok(false);
    };
    let Some(chat) = source.platform_conversation_id.as_deref() else {
        return Ok(false);
    };
    let Some(route) = crate::services::channel_routing_service::resolve_agent(
        db,
        &bot.id,
        chat,
        source.sender_platform_id.as_deref(),
        &bot.user_id,
    )
    .await?
    else {
        return Ok(false);
    };
    if route.api_key_id != key || route.conversation.id != source.conversation_id {
        return Ok(false);
    }
    let Some(link) = db
        .collection::<NyxbotChannel>(CHANNELS)
        .find_one(doc! {
            "channel_bot_id": &bot.id, "route_api_key_id": key,
            "transport": "direct", "status": "active",
        })
        .await?
    else {
        return Ok(false);
    };
    if link.bot_owner_id.as_deref().unwrap_or(&link.user_id) != bot.user_id {
        return Ok(false);
    }
    if let Some(org) = link.bot_owner_id.as_deref()
        && !crate::services::org_service::is_admin(db, &link.user_id, org).await?
    {
        return Ok(false);
    }
    Ok(true)
}

pub(super) async fn eligible_source(
    db: &mongodb::Database,
    bot: &ChannelBot,
    source: &ChannelMessage,
) -> AppResult<bool> {
    if !eligible_source_live(db, bot, source).await? {
        return Ok(false);
    }
    let Some(link) = db
        .collection::<NyxbotChannel>(CHANNELS)
        .find_one(doc! {
            "channel_bot_id": &bot.id, "route_api_key_id": &source.agent_api_key_id,
            "transport": "direct", "status": "active",
        })
        .await?
    else {
        return Ok(false);
    };
    feature_flag_service::personal_flag_enabled(db, &link.user_id, NYXBOT_THREAD_FOLLOW_FLAG_KEY)
        .await
}

/// Reconstruct only a server-persisted child delivery binding. Subscription
/// stop/expiry/flag rollback do not discard already admitted work.
pub async fn resolve_admitted(
    db: &mongodb::Database,
    adapter: &dyn PlatformAdapter,
    bot: &ChannelBot,
    credentials: &BotCredentials<'_>,
    owner: &str,
    origin: &crate::models::assistant_conversation::ChannelOrigin,
    conversation: &str,
) -> AppResult<Option<ThreadReplyTarget>> {
    crate::services::channel_thread_follow_service::validate_delivery(
        db,
        owner,
        origin,
        conversation,
    )
    .await?;
    let Some(binding) = origin.thread.as_deref() else {
        return Ok(None);
    };
    let mut target = tokio::time::timeout(
        std::time::Duration::from_secs(HISTORY_SECONDS),
        resolve_inner(
            db,
            adapter,
            bot,
            credentials,
            &binding.source_message_id,
            false,
        ),
    )
    .await
    .unwrap_or(Ok(None))?;
    if let Some(t) = &mut target {
        t.admitted = Some((owner.into(), origin.clone(), conversation.into()));
    }
    Ok(target)
}

/// Called after sender/route admission, never by an HTTP DTO. Re-load retained
/// metadata to prevent arbitrary caller-supplied facts from authorizing a send.
pub async fn resolve(
    db: &mongodb::Database,
    adapter: &dyn PlatformAdapter,
    bot: &ChannelBot,
    credentials: &BotCredentials<'_>,
    message_id: &str,
) -> AppResult<Option<ThreadReplyTarget>> {
    tokio::time::timeout(
        std::time::Duration::from_secs(HISTORY_SECONDS),
        resolve_inner(db, adapter, bot, credentials, message_id, true),
    )
    .await
    .unwrap_or(Ok(None))
}

async fn resolve_inner(
    db: &mongodb::Database,
    adapter: &dyn PlatformAdapter,
    bot: &ChannelBot,
    credentials: &BotCredentials<'_>,
    message_id: &str,
    require_flag: bool,
) -> AppResult<Option<ThreadReplyTarget>> {
    if adapter.platform_id() != bot.platform || !adapter.thread_capabilities().thread_reply {
        return Ok(None);
    }
    let Some(source) = db
        .collection::<ChannelMessage>(MESSAGES)
        .find_one(doc! {"_id": message_id, "channel_bot_id": &bot.id, "user_id": &bot.user_id})
        .await?
    else {
        return Ok(None);
    };
    if !(if require_flag {
        eligible_source(db, bot, &source).await?
    } else {
        eligible_source_live(db, bot, &source).await?
    }) {
        return Ok(None);
    }
    let Some(facts) = source
        .thread_context
        .as_ref()
        .filter(|f| source_matches(f, &source))
    else {
        return Ok(None);
    };
    let mut ancestors = Vec::new();
    let mut next = facts.parent_message_id.clone();
    let mut seen = std::collections::HashSet::from([facts.message_id.clone()]);
    // Exact bot/chat/owner/platform scope, finite traversal and ambiguity check.
    while let Some(id) = next {
        if ancestors.len() == ANCESTORS || !seen.insert(id.clone()) {
            return Ok(None);
        }
        use futures::TryStreamExt;
        let rows: Vec<ChannelMessage> = db
            .collection(MESSAGES)
            .find(doc! {
                "channel_bot_id": &bot.id, "user_id": &bot.user_id, "platform": &bot.platform,
                "platform_conversation_id": &facts.chat_id, "platform_message_id": &id,
            })
            .limit(2)
            .await?
            .try_collect()
            .await?;
        if rows.len() != 1 {
            break;
        }
        let Some(parent) = rows[0]
            .thread_context
            .as_ref()
            .filter(|f| source_matches(f, &rows[0]))
        else {
            break;
        };
        next = if parent.root_id.is_some() {
            None
        } else {
            parent.parent_message_id.clone()
        };
        ancestors.push(parent.clone());
    }
    let http = client()?;
    let resolved = tokio::time::timeout(
        std::time::Duration::from_secs(HISTORY_SECONDS),
        adapter.resolve_thread(&http, credentials, facts, &ancestors),
    )
    .await;
    let Ok(Ok(Some(resolved))) = resolved else {
        return Ok(None);
    };
    if !source_matches(&resolved, &source)
        || resolved.kind == ThreadKind::Unknown
        || resolved.root_id.as_deref().is_none_or(|id| !valid_id(id))
    {
        return Ok(None);
    }
    let updated = db.collection::<ChannelMessage>(MESSAGES).update_one(
        doc! {"_id": &source.id, "thread_context": bson::to_bson(facts).map_err(|_| unavailable())?},
        doc! {"$set": {"thread_context": bson::to_bson(&resolved).map_err(|_| unavailable())?}},
    ).await?;
    if updated.matched_count != 1 {
        return Ok(None);
    }
    Ok(Some(ThreadReplyTarget {
        bot_id: bot.id.clone(),
        owner_id: bot.user_id.clone(),
        message_id: source.id,
        route_key_id: source.agent_api_key_id.unwrap(),
        platform: bot.platform.clone(),
        facts: resolved,
        admitted: None,
    }))
}

pub(super) fn source_matches(facts: &ChannelThreadFacts, source: &ChannelMessage) -> bool {
    facts.version == 1
        && source.platform_conversation_id.as_deref() == Some(&facts.chat_id)
        && source.platform_message_id.as_deref() == Some(&facts.message_id)
        && valid_id(&facts.chat_id)
        && valid_id(&facts.message_id)
        && [
            &facts.root_id,
            &facts.parent_message_id,
            &facts.parent_chat_id,
            &facts.native_thread_id,
        ]
        .into_iter()
        .all(|id| id.as_deref().is_none_or(valid_id))
}

/// Callback-only resolution; selection still enforces activation/rollback rules.
pub(crate) async fn resolve_retained(
    db: &mongodb::Database,
    adapter: &dyn PlatformAdapter,
    bot: &ChannelBot,
    credentials: &BotCredentials<'_>,
    message: &str,
) -> AppResult<Option<ThreadReplyTarget>> {
    tokio::time::timeout(
        std::time::Duration::from_secs(HISTORY_SECONDS),
        resolve_inner(db, adapter, bot, credentials, message, false),
    )
    .await
    .unwrap_or(Ok(None))
}

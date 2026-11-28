//! Closed native Discord intake. Gateway admission never establishes addressing.
use super::*;
use crate::services::discord_gateway::valid_message_id as snowflake;

pub(super) struct Evidence {
    pub addressed: bool,
    pub source_message: Option<String>,
    pub guest_sender_verified: bool,
    pub guest_shared_room_verified: bool,
}

/// Resolve positive private audience evidence against the receiving Discord bot.
/// A normalized CMA activity is insufficient to establish a DM on its own.
pub(super) async fn private_guest_facts<'a>(
    state: &AppState,
    row: &NyxbotChannel,
    activity: &'a Value,
) -> AppResult<crate::services::channel_guest_access::transport::TransportFacts<'a>> {
    use crate::services::channel_guest_access::transport::TransportFacts;
    let unknown = TransportFacts::default();
    let chat = activity["conversation"]["id"].as_str().unwrap_or_default();
    let id = activity["event_id"]
        .as_str()
        .unwrap_or_default()
        .rsplit(':')
        .next()
        .unwrap_or_default();
    let sender = activity["actor"]["id"].as_str().unwrap_or_default();
    if activity["conversation"]["kind"] != "private"
        || !activity["conversation"]["thread_id"].is_null()
        || activity["actor"]["kind"] != "human"
        || !snowflake(chat)
        || !snowflake(id)
        || !snowflake(sender)
    {
        return Ok(unknown);
    }
    let bot = channel_bot_service::get_bot(&state.db, &row.channel_bot_id).await?;
    if sender == bot.platform_bot_id {
        return Ok(unknown);
    }
    let token =
        Zeroizing::new(channel_bot_service::decrypt_bot_token(&state.encryption_keys, &bot).await?);
    use crate::services::nyxbot_gateway_source::discord_read;
    let message_path = format!("/channels/{chat}/messages/{id}");
    let channel_path = format!("/channels/{chat}");
    let (message, channel) = tokio::try_join!(
        discord_read(&bot, &token, &message_path),
        discord_read(&bot, &token, &channel_path),
    )?;
    if message["id"] != id
        || message["channel_id"] != chat
        || message["author"]["id"] != sender
        || !matches!(
            message["author"].get("bot"),
            None | Some(Value::Bool(false))
        )
        || !message["webhook_id"].is_null()
        || !matches!(message["type"].as_u64(), Some(0 | 19))
        || message["content"] != activity["kind"]["text"]
        || !message["edited_timestamp"].is_null()
        || !message["guild_id"].is_null()
        || channel["id"] != chat
        || channel["type"] != 1
        || !channel["guild_id"].is_null()
    {
        return Ok(unknown);
    }
    Ok(TransportFacts {
        sender_id: Some(sender),
        audience: crate::services::channel_guest_access::AnswerAudience::Private,
        shared_room_verified: false,
    })
}

/// A maximum of four authenticated, capped Discord reads: original message,
/// its channel, (only when absent) its exact same-channel reply reference, and
/// guild roles only for a role mention on a bounded role-cache miss.
/// Errors confer no reply/follow authority. Display names never participate.
pub(super) async fn evidence(
    state: &AppState,
    row: &NyxbotChannel,
    activity: &Value,
    event: &str,
) -> AppResult<Evidence> {
    let mentioned = activity["kind"]["mentions_bot"] == true;
    let mut result = Evidence {
        addressed: mentioned,
        source_message: None,
        guest_sender_verified: false,
        guest_shared_room_verified: false,
    };
    if activity["conversation"]["kind"] == "private" {
        result.addressed = true;
        return Ok(result);
    }
    let bot = Box::pin(channel_bot_service::get_bot(&state.db, &row.channel_bot_id)).await?;
    let token = channel_bot_service::decrypt_bot_token(&state.encryption_keys, &bot).await?;
    let chat = activity["conversation"]["id"].as_str().unwrap_or_default();
    let id = activity["event_id"]
        .as_str()
        .unwrap_or_default()
        .rsplit(':')
        .next()
        .unwrap_or_default();
    if !snowflake(chat) || !snowflake(id) {
        return Ok(result);
    }
    use crate::services::nyxbot_gateway_source::discord_read;
    let read = async {
        let mut message =
            discord_read(&bot, &token, &format!("/channels/{chat}/messages/{id}")).await?;
        if message["id"] != id
            || message["channel_id"] != chat
            || message["author"]["id"] != activity["actor"]["id"]
            || message["content"] != activity["kind"]["text"]
            || !message["edited_timestamp"].is_null()
        {
            return Ok::<_, AppError>(None);
        }
        let channel = discord_read(&bot, &token, &format!("/channels/{chat}")).await?;
        // GET Message omits the Gateway-only guild_id envelope. Recover it
        // from the authenticated channel, without trusting provider metadata.
        if message["guild_id"].is_null()
            && channel["id"] == chat
            && channel["guild_id"].as_str().is_some_and(snowflake)
        {
            message["guild_id"] = channel["guild_id"].clone();
        }
        if let Some(parent) = message["message_reference"]["message_id"]
            .as_str()
            .filter(|s| snowflake(s))
            .map(str::to_owned)
            && message["referenced_message"].is_null()
            && message["message_reference"]["channel_id"]
                .as_str()
                .is_none_or(|v| v == chat)
            && let Ok(parent) =
                discord_read(&bot, &token, &format!("/channels/{chat}/messages/{parent}")).await
        {
            message["referenced_message"] = parent;
        }
        crate::services::discord_gateway::roles::ensure(&bot, &token, &message).await;
        let Some(facts) = crate::services::discord_gateway::facts::authenticated_facts(
            &bot,
            &json!({"d":message}),
            &channel,
        ) else {
            return Ok(None);
        };
        if facts.native_thread_id.as_deref() != activity["conversation"]["thread_id"].as_str() {
            return Ok(None);
        }
        let shared_room_verified =
            crate::services::channel_guest_access::transport::discord_shared_room(
                channel["type"].as_u64(),
                &channel["guild_id"],
            );
        Ok(Some((facts, shared_room_verified)))
    };
    let (facts, shared_room_verified) = match read.await {
        Ok(Some(facts)) => facts,
        Ok(None) => {
            // Conflicting authenticated platform evidence overrides a gateway
            // mention declaration. An unavailable lookup confers no new facts.
            result.addressed = false;
            return Ok(result);
        }
        Err(_) => return Ok(result),
    };
    use crate::models::channel_thread::{ThreadAddress, ThreadKind};
    result.addressed = matches!(
        facts.address,
        ThreadAddress::Mention | ThreadAddress::ReplyToBot
    );
    // Native thread/reply-chain metadata reuses the existing follow selector,
    // including members, reply_mode, limits, stop, expiry and turn-time fences.
    let follow_supported = matches!(facts.kind, ThreadKind::Native | ThreadKind::ReplyChain);
    result.guest_sender_verified = activity["actor"]["kind"] == "human"
        && facts.sender_kind == crate::models::channel_thread::ThreadSenderKind::Human;
    result.guest_shared_room_verified = result.guest_sender_verified && shared_room_verified;
    let Some(response) = Box::pin(async {
        state
            .db
            .collection::<crate::models::nyxbot_gateway_response::GatewayResponse>(
                crate::models::nyxbot_gateway_response::COLLECTION_NAME,
            )
            .find_one(doc! {"event_key":event,"channel_id":&row.id})
            .await
    })
    .await?
    else {
        return Ok(result);
    };
    let source_id = response.id;
    let source = doc! {"_id":&source_id,"channel_bot_id":&bot.id,"conversation_id":&row.route_id,
    "platform_conversation_id":chat,"user_id":&bot.user_id,"direction":"inbound","platform":"discord",
    "platform_message_id":id,"sender_platform_id":activity["actor"]["id"].as_str().unwrap_or_default(),
    "content_type":"text","thread_id":&facts.native_thread_id,"agent_api_key_id":&row.route_api_key_id,
    "thread_context":bson::to_bson(&facts).map_err(|_|AppError::OAuthAuthorityUnavailable)?,
    "guest_transport_audience":result.guest_sender_verified.then_some("shared"),
    "guest_transport_shared_room_verified":result.guest_shared_room_verified,
    "native_gateway":true,"created_at":bson::DateTime::now()};
    Box::pin(async {
        state
            .db
            .collection::<bson::Document>(crate::models::channel_message::COLLECTION_NAME)
            .update_one(doc! {"_id":&source_id}, doc! {"$set":source})
            .upsert(true)
            .await
    })
    .await?;
    crate::services::nyxbot_admission_service::begin(&state.db, row, &source_id, event).await?;
    result.source_message = follow_supported.then_some(source_id);
    Ok(result)
}

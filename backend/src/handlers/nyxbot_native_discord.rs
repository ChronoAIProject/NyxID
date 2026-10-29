//! Closed native Discord intake. Gateway admission never establishes addressing.
use super::*;
use crate::services::discord_gateway::valid_message_id as snowflake;

pub(super) struct Evidence {
    pub addressed: bool,
    pub source_message: Option<String>,
}

/// A maximum of three authenticated, capped Discord reads: original message,
/// its channel, and (only when absent) its exact same-channel reply reference.
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
    };
    if activity["conversation"]["kind"] == "private" {
        result.addressed = true;
        return Ok(result);
    }
    let bot = channel_bot_service::get_bot(&state.db, &row.channel_bot_id).await?;
    let token =
        Zeroizing::new(channel_bot_service::decrypt_bot_token(&state.encryption_keys, &bot).await?);
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
        let Some(facts) = crate::services::discord_gateway::facts::authenticated_facts(
            &bot,
            &json!({"d":message}),
            &channel,
        ) else {
            return Ok(None);
        };
        if facts.native_thread_id.is_some() == activity["conversation"]["thread_id"].is_null() {
            return Ok(None);
        }
        Ok(Some(facts))
    };
    let facts = match read.await {
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
    if !matches!(facts.kind, ThreadKind::Native | ThreadKind::ReplyChain) {
        return Ok(result);
    }
    let Some(response) = state
        .db
        .collection::<crate::models::nyxbot_gateway_response::GatewayResponse>(
            crate::models::nyxbot_gateway_response::COLLECTION_NAME,
        )
        .find_one(doc! {"event_key":event,"channel_id":&row.id})
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
    "native_gateway":true,"created_at":bson::DateTime::now()};
    state
        .db
        .collection::<bson::Document>(crate::models::channel_message::COLLECTION_NAME)
        .update_one(doc! {"_id":&source_id}, doc! {"$setOnInsert":source})
        .upsert(true)
        .await?;
    result.source_message = Some(source_id);
    Ok(result)
}

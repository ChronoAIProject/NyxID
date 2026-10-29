//! Native source extension point. Only verified identities are persisted;
//! credentials are assembled just before the authenticated management request.
use super::nyxbot_gateway_service as gateway;
use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::{
        channel_bot::ChannelBot,
        nyxbot_channel::{GatewaySource, NyxbotChannel},
    },
};
use serde_json::{Value, json};

pub const READINESS: &str = "nyxbot:native-gateway-v1";
pub const DISCORD_INTENTS: u64 = 37_377;
pub fn verified_bot_id(source: Option<&GatewaySource>) -> Option<&str> {
    match source {
        Some(GatewaySource::Discord { bot_id }) => Some(bot_id),
        None | Some(GatewaySource::Unsupported) => None,
    }
}
pub fn ready(row: &NyxbotChannel) -> bool {
    matches!(row.gateway_source, Some(GatewaySource::Discord { .. }))
        && row.transport == "gateway"
        && row.status == "active"
        && row.gateway_native_phase.as_deref() == Some("ready")
        && row.gateway_setup_release_id.is_none()
        && row.pending_agent_api_key_id.is_none()
}

/// C's WhatsApp implementation belongs here: select an eligible dedicated BYO
/// source, discover its capability, verify credentials against platform identity,
/// and return only that identity. Returning None keeps the exact relay policy.
pub async fn select(
    state: &AppState,
    row: &NyxbotChannel,
    bot: &ChannelBot,
) -> AppResult<Option<GatewaySource>> {
    if row.platform != "discord" {
        return Ok(None);
    }
    super::security_readiness_service::require(&state.db, READINESS).await?;
    if !super::discord_gateway::transport::gateway_released(&state.db, &bot.id).await? {
        return Err(AppError::Conflict("gateway_native_draining".into()));
    }
    let base = gateway::gateway_base(state).await?;
    let creator = gateway::creator_bearer(state, &row.user_id)?;
    let caps = gateway::gateway_call_at_base(
        &base,
        reqwest::Method::GET,
        "/channel-capabilities",
        &creator,
        None,
        None,
    )
    .await?;
    if caps.status != 200 || !discord_capable(&caps.body) {
        return Err(AppError::ValidationError(
            "gateway_native_unavailable".into(),
        ));
    }
    let token = zeroize::Zeroizing::new(
        super::channel_bot_service::decrypt_bot_token(&state.encryption_keys, bot).await?,
    );
    let identity = discord_read(bot, &token, "/users/@me").await?;
    if identity["id"].as_str() != Some(&bot.platform_bot_id)
        || identity["bot"] != true
        || !super::discord_gateway::valid_message_id(&bot.platform_bot_id)
    {
        return Err(AppError::Forbidden("gateway_bot_identity_mismatch".into()));
    }
    Ok(Some(GatewaySource::Discord {
        bot_id: bot.platform_bot_id.clone(),
    }))
}
pub fn discord_capable(caps: &Value) -> bool {
    let caps = &caps["relay_authoring"];
    caps["native_discord_writes_enabled"] == true
        && caps["native_discord_auto_activation_enabled"] == true
        && caps["multi_consumer_writes_enabled"] == true
}

/// Replace, never append: a native channel cannot retain a relay callback source.
pub fn apply_policy(row: &NyxbotChannel, policy: &mut Value) {
    if let Some(GatewaySource::Discord { bot_id }) = &row.gateway_source {
        // Local reply_mode/members/follow state is authoritative. Broad upstream
        // intake is needed for followed threads without fresh mentions.
        // Emit CMA's serialized shape (empty consumers/rules are omitted).
        // Disable textual routing directives: only Discord's structured mention
        // evidence and NyxID's own policy may address this single consumer.
        policy["routing"] = json!({"version":1,"default_consumer":{"name":"default","mentionable":false},
            "default_targets":["default"],"max_targets":1,"reply_label":"never",
            "addressing":{"mentions":false,"command":false,"event_field":false,"unknown_target_notice":false}});
        policy["sources"] = json!([{"type":"discord","bot_id":bot_id,"intents":DISCORD_INTENTS,
            "admission":{"type":"scoped","senders":{"type":"open"},"chats":{"type":"open"},"groups":"all"}}]);
        policy["reply"]
            .as_object_mut()
            .expect("policy reply")
            .remove("thread_contract");
    } else if row.gateway_source.is_some() {
        // A reader predating a new native source must not replace it with relay.
        policy["sources"] = json!([]);
    } else if matches!(row.platform.as_str(), "discord" | "whatsapp") {
        // Relay raw verification needs platform-specific raw callbacks which a
        // Discord MESSAGE_CREATE does not provide. Trusted normalized is CMA's
        // explicit legacy mode; it does not grant native addressing authority.
        policy["sources"][0]["trust_normalized"] = json!(true);
    }
}
pub async fn credentials(
    state: &AppState,
    row: &NyxbotChannel,
    bot: &ChannelBot,
    agent: &str,
    route: &str,
) -> AppResult<Value> {
    match &row.gateway_source {
        None => Ok(json!({"agent_key":agent,"channel_key":route})),
        Some(GatewaySource::Unsupported) => Err(AppError::OAuthAuthorityUnavailable),
        Some(GatewaySource::Discord { bot_id }) => {
            if bot_id != &bot.platform_bot_id {
                return Err(AppError::Forbidden("gateway_bot_identity_mismatch".into()));
            }
            let token = zeroize::Zeroizing::new(
                super::channel_bot_service::decrypt_bot_token(&state.encryption_keys, bot).await?,
            );
            Ok(json!({"agent_key":agent,"discord_bot_token":token.as_str()}))
        }
    }
}

/// Fixed origin, redirects disabled, 10-second deadline, 256 KiB response cap.
/// Neither errors nor logs include the credential, response body or message.
#[cfg(test)]
pub(crate) static TEST_DISCORD_BASES: std::sync::LazyLock<
    std::sync::Mutex<std::collections::HashMap<String, String>>,
> = std::sync::LazyLock::new(Default::default);
pub(crate) async fn discord_read(bot: &ChannelBot, token: &str, path: &str) -> AppResult<Value> {
    let _ = bot;
    #[cfg(test)]
    {
        let base = TEST_DISCORD_BASES.lock().unwrap().get(&bot.id).cloned();
        if let Some(base) = base {
            return discord_read_at(&base, token, path).await;
        }
    }
    discord_read_at("https://discord.com/api/v10", token, path).await
}
async fn discord_read_at(base: &str, token: &str, path: &str) -> AppResult<Value> {
    let request = gateway::GATEWAY_HTTP_CLIENT
        .get(format!("{base}{path}"))
        .header(reqwest::header::AUTHORIZATION, format!("Bot {token}"));
    tokio::time::timeout(std::time::Duration::from_secs(10), async {
        let mut response = request
            .send()
            .await
            .map_err(|_| AppError::OAuthAuthorityUnavailable)?;
        if !response.status().is_success() {
            return Err(AppError::OAuthAuthorityUnavailable);
        }
        let mut bytes = zeroize::Zeroizing::new(Vec::new());
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| AppError::OAuthAuthorityUnavailable)?
        {
            if bytes.len() + chunk.len() > 256 * 1024 {
                return Err(AppError::OAuthAuthorityUnavailable);
            }
            bytes.extend_from_slice(&chunk);
        }
        serde_json::from_slice(&bytes).map_err(|_| AppError::OAuthAuthorityUnavailable)
    })
    .await
    .map_err(|_| AppError::OAuthAuthorityUnavailable)?
}

pub(crate) fn discord_context(row: &NyxbotChannel, context: &Value) -> bool {
    use super::discord_gateway::valid_message_id as snowflake;
    let Some(GatewaySource::Discord { bot_id }) = &row.gateway_source else {
        return false;
    };
    let a = &context["activity"];
    let c = &a["conversation"];
    let event = a["event_id"].as_str().unwrap_or_default();
    let prefix = format!("discord:{bot_id}:");
    row.platform == "discord"
        && a["source"].as_object().is_some_and(|s| s.len() == 2)
        && context["schema_version"] == 1
        && a["schema_version"] == 1
        && context["channel_id"].as_str() == row.gateway_channel_id.as_deref()
        && row.binding_id.is_some()
        && row.gateway_bot_id.as_deref() == Some(bot_id)
        && a["source"]["type"] == "discord"
        && a["source"]["bot_id"].as_str() == Some(bot_id)
        && event.strip_prefix(&prefix).is_some_and(snowflake)
        && a["kind"]["type"] == "message"
        && a["kind"]["mentions_bot"].is_boolean()
        && a["kind"]["reply_to"].is_null()
        && a["thread"].is_null()
        && a["kind"]["text"]
            .as_str()
            .is_some_and(|s| s.len() <= 32 * 1024)
        && a["kind"]["attachments"]
            .as_array()
            .is_some_and(|s| s.len() <= 32)
        && a["actor"]["kind"] == "human"
        && a["actor"]["id"]
            .as_str()
            .is_some_and(|s| snowflake(s) && s != bot_id)
        && matches!(c["kind"].as_str(), Some("private" | "group"))
        && c["id"].as_str().is_some_and(snowflake)
        && (c["thread_id"].is_null() || (c["kind"] == "group" && c["thread_id"] == c["id"]))
        && context["event_ref"]
            .as_str()
            .is_some_and(|s| !s.is_empty() && s.len() <= 4096)
}

pub(crate) fn provider_context(row: &NyxbotChannel, context: &Value) -> bool {
    match &row.gateway_source {
        Some(GatewaySource::Discord { .. }) => discord_context(row, context),
        None | Some(GatewaySource::Unsupported) => false,
    }
}
pub(crate) fn message_identity<'a>(
    row: &NyxbotChannel,
    activity: &'a Value,
) -> AppResult<(&'static str, &'a str)> {
    match &row.gateway_source {
        Some(GatewaySource::Discord { bot_id }) => activity["event_id"]
            .as_str()
            .and_then(|id| id.strip_prefix(&format!("discord:{bot_id}:")))
            .filter(|id| super::discord_gateway::valid_message_id(id))
            .map(|id| ("discord-inbound", id))
            .ok_or_else(|| AppError::ValidationError("Invalid native message identity".into())),
        None | Some(GatewaySource::Unsupported) => {
            Err(AppError::ValidationError("Native source required".into()))
        }
    }
}

/// Stable status codes only; never gateway prose or credentials.
pub(crate) async fn record_activation(
    db: &mongodb::Database,
    row: &NyxbotChannel,
    body: &Value,
) -> AppResult<()> {
    if row.gateway_source.is_none() {
        return Ok(());
    }
    let status = body["native_discord_activation"]["state"]
        .as_str()
        .filter(|s| {
            matches!(
                *s,
                "disabled"
                    | "creation_pending"
                    | "routing_required"
                    | "provider_unavailable"
                    | "admissions_paused"
                    | "selected"
                    | "not_selected"
                    | "capacity_exceeded"
                    | "pending_discovery"
            )
        })
        .unwrap_or("unknown");
    db.collection::<NyxbotChannel>(crate::models::nyxbot_channel::COLLECTION_NAME).update_one(
        bson::doc! {"_id":&row.id,"gateway_source":bson::to_bson(&row.gateway_source).map_err(|_|AppError::OAuthAuthorityUnavailable)?},
        bson::doc! {"$set":{"gateway_native_activation":status,"delivery_checked_at":bson::DateTime::now()}}).await?;
    Ok(())
}
pub(crate) async fn check_status(state: &AppState, row: &NyxbotChannel) -> AppResult<()> {
    let Some(channel) = row.gateway_channel_id.as_deref() else {
        return Ok(());
    };
    let creator = gateway::creator_bearer(state, &row.user_id)?;
    let path = format!(
        "/channels/{}",
        url::form_urlencoded::byte_serialize(channel.as_bytes()).collect::<String>()
    );
    let result =
        gateway::gateway_call(state, reqwest::Method::GET, &path, &creator, None, None).await?;
    record_activation(&state.db, row, &result.body).await
}

pub(crate) fn matches_identity(source: &GatewaySource, value: &Value) -> bool {
    match source {
        GatewaySource::Discord { bot_id } => {
            value["type"] == "discord" && value["bot_id"].as_str() == Some(bot_id)
        }
        GatewaySource::Unsupported => false,
    }
}
pub(crate) fn validate_created(row: &NyxbotChannel, body: &Value) -> AppResult<()> {
    if let Some(source) = &row.gateway_source {
        if !matches!(
            body["native_discord_activation"]["state"].as_str(),
            Some("selected" | "pending_discovery")
        ) {
            return Err(AppError::ValidationError(
                "gateway_native_activation_unavailable".into(),
            ));
        }
        if !body["definition"]["sources"]
            .as_array()
            .is_some_and(|s| s.len() == 1 && matches_identity(source, &s[0]))
        {
            return Err(AppError::ValidationError(
                "gateway_native_source_mismatch".into(),
            ));
        }
    }
    Ok(())
}

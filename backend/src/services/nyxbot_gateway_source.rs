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
        None | Some(GatewaySource::WhatsApp { .. }) | Some(GatewaySource::Unsupported) => None,
    }
}
pub fn ready(row: &NyxbotChannel) -> bool {
    matches!(
        row.gateway_source,
        Some(GatewaySource::Discord { .. }) | Some(GatewaySource::WhatsApp { .. })
    ) && row.transport == "gateway"
        && row.status == "active"
        && row.gateway_native_phase.as_deref() == Some("ready")
        && row.gateway_setup_release_id.is_none()
        && row.pending_agent_api_key_id.is_none()
}

/// Select an eligible native source, discover its capability, verify credentials
/// against the platform identity, and return only that identity. Returning None
/// keeps the exact relay policy.
pub async fn select(
    state: &AppState,
    row: &NyxbotChannel,
    bot: &ChannelBot,
) -> AppResult<Option<GatewaySource>> {
    if row.platform == "whatsapp" {
        super::security_readiness_service::require(&state.db, READINESS).await?;
        if bot.credential_source != "user" {
            return Err(AppError::ValidationError(
                "dedicated_byo_app_required".into(),
            ));
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
        if caps.status != 200 || !whatsapp_capable(&caps.body) {
            return Err(AppError::ValidationError(
                "native_whatsapp_unavailable".into(),
            ));
        }
        let config = super::whatsapp_native::load(&state.db, bot)
            .await?
            .ok_or_else(|| AppError::ValidationError("native_whatsapp_not_configured".into()))?;
        if super::whatsapp_native::activation_backoff_active(&config) {
            return Err(AppError::ValidationError(
                "native_whatsapp_activation_backoff".into(),
            ));
        }
        if !super::whatsapp_native::matches_bot(&config, bot) {
            return Err(AppError::ValidationError(
                "native_whatsapp_identity_changed".into(),
            ));
        }
        super::whatsapp_native::verify_platform_identity(
            &state.http_client,
            &state.encryption_keys,
            bot,
            &config,
        )
        .await?;
        super::whatsapp_native::source(&config, bot)?;
        return Ok(Some(GatewaySource::WhatsApp {
            app_id: config.meta_app_id,
            waba_id: config.waba_id,
            phone_number_id: config.phone_number_id,
        }));
    }
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

pub fn whatsapp_capable(caps: &Value) -> bool {
    super::whatsapp_native::supported(caps)
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
    } else if let Some(GatewaySource::WhatsApp {
        app_id,
        waba_id,
        phone_number_id,
    }) = &row.gateway_source
    {
        policy["routing"] = json!({"version":1,"default_consumer":{"name":"default","mentionable":false,"non_human":true},
            "default_targets":["default"],"max_targets":1,"reply_label":"never",
            "addressing":{"mentions":false,"command":false,"event_field":false,"unknown_target_notice":false}});
        policy["sources"] = json!([{"type":"whatsapp","app_id":app_id,"waba_id":waba_id,
            "phone_number_id":phone_number_id,"admission":{"type":"all","groups":[]}}]);
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
        Some(GatewaySource::WhatsApp {
            app_id,
            waba_id,
            phone_number_id,
        }) => {
            if bot.platform != "whatsapp"
                || bot.credential_source != "user"
                || bot.platform_bot_id != *phone_number_id
            {
                return Err(AppError::Forbidden(
                    "native_whatsapp_identity_mismatch".into(),
                ));
            }
            let config = super::whatsapp_native::load(&state.db, bot)
                .await?
                .ok_or_else(|| AppError::OAuthAuthorityUnavailable)?;
            if config.meta_app_id != *app_id
                || config.waba_id != *waba_id
                || config.phone_number_id != *phone_number_id
            {
                return Err(AppError::Forbidden(
                    "native_whatsapp_identity_mismatch".into(),
                ));
            }
            let app_secret = state
                .encryption_keys
                .decrypt(
                    bot.app_secret_encrypted
                        .as_deref()
                        .ok_or_else(|| AppError::OAuthAuthorityUnavailable)?,
                )
                .await
                .map_err(|_| AppError::OAuthAuthorityUnavailable)?;
            let app_secret = zeroize::Zeroizing::new(
                String::from_utf8(app_secret).map_err(|_| AppError::OAuthAuthorityUnavailable)?,
            );
            let token = zeroize::Zeroizing::new(
                super::channel_bot_service::decrypt_bot_token(&state.encryption_keys, bot).await?,
            );
            // The relay Verify Token is hash-only. Native management receives a
            // separate, envelope-encrypted token generated on its first
            // management request; it is never returned by the settings API.
            let verify_token = super::whatsapp_native::native_verify_token(
                &state.db,
                &state.encryption_keys,
                &config,
            )
            .await?;
            let bundle = super::whatsapp_native::Credentials::new(
                app_secret.to_string(),
                token.to_string(),
                verify_token.to_string(),
            )
            .and_then(|bundle| {
                serde_json::to_value(bundle).map_err(|_| AppError::OAuthAuthorityUnavailable)
            })?;
            Ok(json!({"agent_key":agent,"credentials":{"whatsapp":bundle},
                "source":{"app_id":app_id,"waba_id":waba_id,"phone_number_id":phone_number_id}}))
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
        Some(GatewaySource::WhatsApp {
            app_id,
            waba_id,
            phone_number_id,
        }) => {
            let activity = &context["activity"];
            let source = &activity["source"];
            let actor = &activity["actor"];
            let conversation = &activity["conversation"];
            row.platform == "whatsapp"
                && context["schema_version"] == 1
                && activity["schema_version"] == 1
                && context["channel_id"].as_str() == row.gateway_channel_id.as_deref()
                && context["event_ref"]
                    .as_str()
                    .is_some_and(|v| !v.is_empty() && v.len() <= 4096)
                && source["type"] == "whatsapp"
                && source["app_id"].as_str() == Some(app_id)
                && source["waba_id"].as_str() == Some(waba_id)
                && source["phone_number_id"].as_str() == Some(phone_number_id)
                && actor["kind"] == "unknown"
                && super::whatsapp_native::numeric_id(actor["id"].as_str().unwrap_or_default())
                && conversation["kind"] == "private"
                && conversation["id"] == actor["id"]
                && conversation["thread_id"].is_null()
                && activity["kind"]["type"] == "message"
                && activity["kind"]["text"]
                    .as_str()
                    .is_some_and(|text| text.len() <= 32 * 1024)
                && activity["kind"]["attachments"]
                    .as_array()
                    .is_none_or(|items| items.len() <= 32)
                && activity["event_id"]
                    .as_str()
                    .is_some_and(|v| !v.is_empty() && v.len() <= 256)
        }
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
        Some(GatewaySource::WhatsApp {
            app_id,
            waba_id,
            phone_number_id,
        }) => activity["event_id"]
            .as_str()
            .filter(|id| super::channel_delivery_service::valid_message_id(id))
            .filter(|_| {
                activity["source"]["type"] == "whatsapp"
                    && activity["source"]["app_id"].as_str() == Some(app_id)
                    && activity["source"]["waba_id"].as_str() == Some(waba_id)
                    && activity["source"]["phone_number_id"].as_str() == Some(phone_number_id)
            })
            .map(|id| ("whatsapp-inbound", id))
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
    let state = match row.gateway_source.as_ref() {
        Some(GatewaySource::WhatsApp { .. }) => &body["native_whatsapp_activation"]["state"],
        _ => &body["native_discord_activation"]["state"],
    };
    let status = state
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
    record_activation(&state.db, row, &result.body).await?;
    if matches!(
        row.gateway_source.as_ref(),
        Some(GatewaySource::WhatsApp { .. })
    ) {
        let Some(bot) = state
            .db
            .collection::<crate::models::channel_bot::ChannelBot>(
                crate::models::channel_bot::COLLECTION_NAME,
            )
            .find_one(bson::doc! {"_id": &row.channel_bot_id})
            .await?
        else {
            return Ok(());
        };
        let Some(config) = super::whatsapp_native::load(&state.db, &bot).await? else {
            return Ok(());
        };
        let mut latest = None;
        let activities = result.body["activities"]
            .as_array()
            .into_iter()
            .flatten()
            .chain(result.body["activity"].as_array().into_iter().flatten())
            .take(50);
        for activity in activities {
            if let Some(summary) = super::whatsapp_native::delivery::summarize(&config, activity) {
                latest = summary
                    .observations
                    .into_iter()
                    .max_by_key(|observation| observation.confirmed_at);
            }
        }
        if let Some(observation) = latest {
            state
                .db
                .collection::<NyxbotChannel>(crate::models::nyxbot_channel::COLLECTION_NAME)
                .update_one(
                    bson::doc! {
                        "_id": &row.id,
                        "gateway_source": bson::to_bson(&row.gateway_source)
                            .map_err(|_| AppError::OAuthAuthorityUnavailable)?,
                    },
                    bson::doc! {
                        "$set": {
                            "gateway_native_delivery": format!("cma_receipt_{}", observation.status),
                            "delivery_checked_at": bson::DateTime::now(),
                        }
                    },
                )
                .await?;
        }
    }
    Ok(())
}

pub(crate) fn matches_identity(source: &GatewaySource, value: &Value) -> bool {
    match source {
        GatewaySource::Discord { bot_id } => {
            value["type"] == "discord" && value["bot_id"].as_str() == Some(bot_id)
        }
        GatewaySource::WhatsApp {
            app_id,
            waba_id,
            phone_number_id,
        } => {
            value["type"] == "whatsapp"
                && value["app_id"].as_str() == Some(app_id)
                && value["waba_id"].as_str() == Some(waba_id)
                && value["phone_number_id"].as_str() == Some(phone_number_id)
        }
        GatewaySource::Unsupported => false,
    }
}
pub(crate) fn validate_created(row: &NyxbotChannel, body: &Value) -> AppResult<()> {
    if let Some(source) = &row.gateway_source {
        let activation = match source {
            GatewaySource::WhatsApp { .. } => &body["native_whatsapp_activation"]["state"],
            _ => &body["native_discord_activation"]["state"],
        };
        if !matches!(activation.as_str(), Some("selected" | "pending_discovery")) {
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
        if matches!(source, GatewaySource::WhatsApp { .. })
            && body["endpoints"]["whatsapp_webhook_url"]
                .as_str()
                .is_none_or(str::is_empty)
        {
            return Err(AppError::ValidationError(
                "native_whatsapp_webhook_unavailable".into(),
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use mongodb::bson::{self, doc};

    fn row() -> NyxbotChannel {
        bson::from_document(doc! {
            "_id": "channel", "user_id": "owner", "channel_bot_id": "bot",
            "platform": "whatsapp", "bot_label": "WhatsApp", "transport": "gateway",
            "status": "active", "route_api_key_id": "route", "gateway_channel_id": "channel",
            "gateway_record_id": "record", "gateway_native_phase": "ready", "binding_id": "binding",
            "gateway_source": {"type":"whatsapp", "app_id":"111", "waba_id":"222", "phone_number_id":"333"},
            "created_at": bson::DateTime::now(), "updated_at": bson::DateTime::now(),
        })
        .expect("source test row")
    }

    fn source_value() -> Value {
        json!({"type":"whatsapp", "app_id":"111", "waba_id":"222", "phone_number_id":"333"})
    }

    #[test]
    fn whatsapp_capability_negotiation_requires_all_write_guards() {
        let base = json!({"relay_authoring": {
            "native_whatsapp_writes_enabled": true,
            "native_whatsapp_auto_activation_enabled": true,
            "multi_consumer_writes_enabled": true,
        }});
        assert!(whatsapp_capable(&base));
        for field in [
            "native_whatsapp_writes_enabled",
            "native_whatsapp_auto_activation_enabled",
            "multi_consumer_writes_enabled",
        ] {
            let mut unavailable = base.clone();
            unavailable["relay_authoring"][field] = json!(false);
            assert!(!whatsapp_capable(&unavailable), "{field}");
        }
    }

    #[test]
    fn whatsapp_policy_is_private_only_and_scoped_non_human() {
        let row = row();
        let mut policy = json!({"reply": {"thread_contract": {"version": 1}},
            "sources": [{}], "routing": {"old": true}});
        apply_policy(&row, &mut policy);
        assert_eq!(policy["routing"]["default_consumer"]["non_human"], true);
        assert_eq!(
            policy["sources"],
            json!([{
                "type":"whatsapp", "app_id":"111", "waba_id":"222", "phone_number_id":"333",
                "admission":{"type":"all", "groups":[]}
            }])
        );
        assert!(policy["reply"].get("thread_contract").is_none());
    }

    #[test]
    fn whatsapp_context_and_identity_require_exact_source_and_private_account() {
        let row = row();
        let context = json!({"schema_version":1, "channel_id":"channel", "event_ref":"opaque",
            "activity":{"schema_version":1, "event_id":"wa-native-message",
                "source":{"type":"whatsapp", "app_id":"111", "waba_id":"222", "phone_number_id":"333"},
                "actor":{"kind":"unknown", "id":"0015551234567"},
                "conversation":{"kind":"private", "id":"0015551234567", "thread_id":null},
                "kind":{"type":"message", "text":"hello", "attachments":[]}}});
        assert!(provider_context(&row, &context));
        let (namespace, id) = message_identity(&row, &context["activity"]).unwrap();
        assert_eq!(namespace, "whatsapp-inbound");
        assert_eq!(id, "wa-native-message");
        for (path, value) in [
            ("/activity/source/app_id", json!("999")),
            ("/activity/source/phone_number_id", json!("999")),
            ("/activity/actor/kind", json!("human")),
            ("/activity/conversation/kind", json!("group")),
            ("/activity/conversation/id", json!("group")),
            ("/activity/kind/type", json!("interaction")),
        ] {
            let mut changed = context.clone();
            *changed.pointer_mut(path).expect(path) = value;
            assert!(!provider_context(&row, &changed), "{path}");
        }
        assert!(message_identity(&row, &json!({"event_id":"bad\nvalue"})).is_err());
    }

    #[test]
    fn whatsapp_created_response_is_fenced_to_activation_source_and_webhook() {
        let row = row();
        let mut body = json!({
            "native_whatsapp_activation":{"state":"selected"},
            "definition":{"sources":[{}] },
            "endpoints":{"whatsapp_webhook_url":"https://gateway.example/whatsapp"}
        });
        body["definition"]["sources"][0] = source_value();
        assert!(validate_created(&row, &body).is_ok());
        body["definition"]["sources"][0]["waba_id"] = json!("999");
        assert!(validate_created(&row, &body).is_err());
        body["definition"]["sources"][0] = source_value();
        body["endpoints"]["whatsapp_webhook_url"] = json!(null);
        assert!(validate_created(&row, &body).is_err());
    }

    #[test]
    fn whatsapp_ready_requires_native_transport_fence() {
        let mut row = row();
        assert!(ready(&row));
        row.transport = "direct".into();
        assert!(!ready(&row));
        row.transport = "gateway".into();
        row.gateway_native_phase = Some("preparing".into());
        assert!(!ready(&row));
    }
}

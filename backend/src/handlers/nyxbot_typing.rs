//! Turn-scoped ephemeral indicators. Destinations and secrets are resolved once
//! from retained custody, then dropped with the task. No transcript messages.
use super::*;
use crate::models::assistant_conversation::AssistantConversation;

// Deliberately no Debug: keys and opaque event references remain task-local.
enum Destination {
    Gateway {
        base: String,
        path: Zeroizing<String>,
        key: Zeroizing<String>,
    },
    Telegram {
        adapter: Box<dyn crate::services::channel_platform::PlatformAdapter>,
        token: Zeroizing<String>,
        bot_id: String,
        chat: String,
        thread: Option<String>,
    },
}

struct Prepared {
    channel: NyxbotChannel,
    destination: Destination,
}

pub(crate) fn start(
    state: &AppState,
    row: &AssistantConversation,
) -> Option<tokio::task::JoinHandle<()>> {
    let turn = row.active_turn.as_ref()?;
    let event_id = turn.channel_event_id.clone()?;
    crate::services::assistant_checking_notice::destination(row)?;
    let state = state.clone();
    let row = row.clone();
    Some(
        crate::services::shutdown::process().spawn_worker(async move {
            let Ok(Some(prepared)) = resolve(&state, &row, &event_id).await else {
                return;
            };
            let mut interval = tokio::time::interval(Duration::from_secs(4));
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                interval.tick().await;
                if !live(&state, &row, &event_id, &prepared.channel)
                    .await
                    .unwrap_or(false)
                    || !prepared.destination.send(&state).await.unwrap_or(false)
                {
                    break;
                }
            }
        }),
    )
}

fn event_filter(row: &AssistantConversation, event_id: &str) -> bson::Document {
    doc! {
        "_id":event_id,"user_id":&row.user_id,"conversation_id":&row.id,
        "turn_id":&row.active_turn.as_ref().expect("claimed turn").turn_id,
        "expires_at":{"$gt":bson::DateTime::now()},
    }
}

/// Full authority and credential resolution happens once, before the first tick.
async fn resolve(
    state: &AppState,
    row: &AssistantConversation,
    event_id: &str,
) -> AppResult<Option<Prepared>> {
    let Some(event) = state
        .db
        .collection::<NyxbotEvent>(EVENTS)
        .find_one(event_filter(row, event_id))
        .await?
    else {
        return Ok(None);
    };
    let Some(delivery) = &event.delivery else {
        return Ok(None);
    };
    let Some((mut channel, _)) = late_delivery::eligible(state, &event, delivery, row).await?
    else {
        return Ok(None);
    };
    let destination = if channel.transport == "gateway" {
        let context = match event.event_context_ciphertext.as_ref() {
            Some(bytes) => {
                let plaintext = Zeroizing::new(state.encryption_keys.decrypt(bytes).await?);
                serde_json::from_slice::<Value>(&plaintext)
                    .map_err(|_| AppError::OAuthAuthorityUnavailable)?
            }
            None => Value::Null,
        };
        if context["capabilities"]["typing"] == true {
            let Some(reference) = context["event_ref"].as_str() else {
                return Ok(None);
            };
            let Some(key) = decrypt_agent_key(state, &channel).await? else {
                return Ok(None);
            };
            Some(Destination::Gateway {
                base: crate::services::nyxbot_gateway_service::gateway_base(state).await?,
                path: Zeroizing::new(format!("/events/{}/typing", urlencode(reference))),
                key,
            })
        } else if channel.gateway_source.is_some() {
            return Ok(None);
        } else {
            None
        }
    } else {
        None
    };
    let destination = match destination {
        Some(destination) => destination,
        None => {
            // CMA relay typing is unsupported. NyxID owns this bot's exact
            // authenticated inbound, including its original Telegram topic.
            if channel.platform != "telegram" {
                return Ok(None);
            }
            use crate::models::channel_message::{COLLECTION_NAME as MESSAGES, ChannelMessage};
            let Some(inbound) = state.db.collection::<ChannelMessage>(MESSAGES).find_one(doc! {
                "_id":&event.event_id,"channel_bot_id":&channel.channel_bot_id,
                "agent_api_key_id":&channel.route_api_key_id,"direction":"inbound","platform":"telegram",
            }).await? else { return Ok(None) };
            let Some(chat) = inbound.platform_conversation_id else {
                return Ok(None);
            };
            let bot = channel_bot_service::get_bot(&state.db, &channel.channel_bot_id).await?;
            Destination::Telegram {
                token: Zeroizing::new(
                    channel_bot_service::decrypt_bot_token(&state.encryption_keys, &bot).await?,
                ),
                adapter: crate::services::channel_adapters::resolve_adapter(
                    &channel.platform,
                    &state.token_exchange_cache,
                )?,
                bot_id: bot.platform_bot_id,
                chat,
                thread: inbound.thread_id,
            }
        }
    };
    channel.agent_key_ciphertext = None;
    Ok(Some(Prepared {
        channel,
        destination,
    }))
}

/// Three cheap reads per tick: live turn/Stop, retained event binding, then
/// channel eligibility. No credentials, KMS, gateway discovery or full target
/// reconciliation on refresh. Settlement/cancellation also aborts the task.
async fn live(
    state: &AppState,
    row: &AssistantConversation,
    event_id: &str,
    channel: &NyxbotChannel,
) -> AppResult<bool> {
    let turn = row.active_turn.as_ref().expect("claimed turn");
    if engine::stop_requested(&state.db, &row.user_id, &row.id, &turn.turn_id).await? {
        return Ok(false);
    }
    if state
        .db
        .collection::<bson::Document>(EVENTS)
        .find_one(event_filter(row, event_id))
        .projection(doc! {"_id":1})
        .await?
        .is_none()
    {
        return Ok(false);
    }
    let Some(current) = state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .find_one(doc! {"_id":&channel.id,"user_id":&row.user_id,"status":"active"})
        .projection(doc! {"agent_key_ciphertext":0})
        .await?
    else {
        return Ok(false);
    };
    Ok(current.transport == channel.transport
        && current.platform == channel.platform
        && current.channel_bot_id == channel.channel_bot_id
        && current.bot_owner_id == channel.bot_owner_id
        && current.route_id == channel.route_id
        && current.route_api_key_id == channel.route_api_key_id
        && current.agent_api_key_id == channel.agent_api_key_id
        && current.agent_id == channel.agent_id
        && current.binding_id == channel.binding_id
        && current.gateway_channel_id == channel.gateway_channel_id
        && current.gateway_source == channel.gateway_source
        && current.gateway_native_phase == channel.gateway_native_phase
        && current.owner_sender_ids == channel.owner_sender_ids
        && current.private_chats == channel.private_chats
        && current.follow_binding_generation == channel.follow_binding_generation)
}

impl Destination {
    async fn send(&self, state: &AppState) -> AppResult<bool> {
        match self {
            Self::Gateway { base, path, key } => {
                let response = tokio::time::timeout(
                    Duration::from_secs(3),
                    crate::services::nyxbot_gateway_service::gateway_call_at_base(
                        base,
                        reqwest::Method::POST,
                        path,
                        key,
                        Some(&json!({})),
                        None,
                    ),
                )
                .await
                .map_err(|_| AppError::OAuthAuthorityUnavailable)??;
                Ok(response.status == 200 && response.body["outcome"] == "sent")
            }
            Self::Telegram {
                adapter,
                token,
                bot_id,
                chat,
                thread,
            } => {
                adapter
                    .send_typing(
                        &state.http_client,
                        &crate::services::channel_platform::BotCredentials {
                            token,
                            platform_bot_id: Some(bot_id),
                            platform_secrets: None,
                            billing: None,
                        },
                        chat,
                        thread.as_deref(),
                    )
                    .await
            }
        }
    }
}

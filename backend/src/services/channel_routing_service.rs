//! Conversation-to-agent routing service.
//!
//! Resolves which agent (API key with callback URL) should receive an inbound
//! message based on the conversation, sender, or bot default. Also provides
//! CRUD for conversation routing rules.

use chrono::Utc;
use futures::TryStreamExt;
use mongodb::bson::{self, doc};

use crate::errors::{AppError, AppResult};
use crate::models::api_key::{ApiKey, COLLECTION_NAME as API_KEYS};
use crate::models::channel_conversation::{COLLECTION_NAME, ChannelConversation};

/// A resolved routing target: the conversation, the callback URL, and the
/// API key ID to sign callbacks with.
#[derive(Debug, Clone)]
pub struct AgentRoute {
    pub conversation: ChannelConversation,
    pub callback_url: String,
    pub api_key_id: String,
}

/// Resolve which agent should handle a message for the given bot + conversation.
///
/// Resolution order:
/// 1. Exact conversation match (`channel_bot_id` + `platform_conversation_id`)
/// 2. Sender-specific match in group contexts (`channel_bot_id` + `platform_sender_id`)
/// 3. Default agent for the bot (`channel_bot_id` + `default_agent: true`)
///
/// Returns `None` if no route is found or the matched API key lacks a callback URL.
pub async fn resolve_agent(
    db: &mongodb::Database,
    channel_bot_id: &str,
    platform_conversation_id: &str,
    platform_sender_id: Option<&str>,
) -> AppResult<Option<AgentRoute>> {
    let col = db.collection::<ChannelConversation>(COLLECTION_NAME);

    // Step 1: sender-specific match first (highest priority -- per-user routing in groups)
    let conversation = match platform_sender_id {
        Some(sender_id) if !sender_id.is_empty() => {
            col.find_one(doc! {
                "channel_bot_id": channel_bot_id,
                "platform_conversation_id": platform_conversation_id,
                "platform_sender_id": sender_id,
                "is_active": true,
            })
            .await?
        }
        _ => None,
    };

    // Step 2: exact conversation match (no sender filter)
    let conversation = match conversation {
        Some(c) => Some(c),
        None => {
            col.find_one(doc! {
                "channel_bot_id": channel_bot_id,
                "platform_conversation_id": platform_conversation_id,
                "platform_sender_id": null,
                "is_active": true,
            })
            .await?
        }
    };

    // Step 3: default agent for the bot
    let conversation = match conversation {
        Some(c) => Some(c),
        None => {
            col.find_one(doc! {
                "channel_bot_id": channel_bot_id,
                "default_agent": true,
                "is_active": true,
            })
            .await?
        }
    };

    let conversation = match conversation {
        Some(c) => c,
        None => return Ok(None),
    };

    // Look up the API key to get the callback URL
    let api_key = db
        .collection::<ApiKey>(API_KEYS)
        .find_one(doc! {
            "_id": &conversation.agent_api_key_id,
            "is_active": true,
        })
        .await?;

    let callback_url = match api_key.and_then(|k| k.callback_url) {
        Some(url) if !url.is_empty() => url,
        _ => return Ok(None),
    };

    Ok(Some(AgentRoute {
        api_key_id: conversation.agent_api_key_id.clone(),
        conversation,
        callback_url,
    }))
}

/// Create a new conversation routing rule.
///
/// `channel_bot_id` is `None` for device channels (platform="device"); the
/// caller is responsible for enforcing the `platform == "device" ⇔ bot_id is
/// None` invariant — see `handlers::channel_conversations::create_conversation`.
#[allow(clippy::too_many_arguments)]
pub async fn create_conversation(
    db: &mongodb::Database,
    user_id: &str,
    channel_bot_id: Option<&str>,
    platform: &str,
    platform_conversation_id: &str,
    platform_conversation_type: &str,
    platform_sender_id: Option<&str>,
    agent_api_key_id: &str,
    default_agent: bool,
) -> AppResult<ChannelConversation> {
    // Verify the API key exists and belongs to the user
    let api_key = db
        .collection::<ApiKey>(API_KEYS)
        .find_one(doc! { "_id": agent_api_key_id, "user_id": user_id, "is_active": true })
        .await?
        .ok_or_else(|| AppError::NotFound(format!("API key not found: {agent_api_key_id}")))?;

    if api_key.callback_url.is_none() {
        return Err(AppError::ValidationError(
            "API key must have a callback_url configured".to_string(),
        ));
    }

    // If setting as default, deactivate any existing default route for this
    // bot. We deactivate (not just clear default_agent) because the old route
    // with platform_conversation_id="*" would otherwise clash with the unique
    // partial index on active routes.
    //
    // Only applies to bot-backed conversations: device channels have no
    // default-agent concept (there's no webhook fan-out to disambiguate).
    if default_agent && let Some(bot_id) = channel_bot_id {
        let now = bson::DateTime::from_chrono(Utc::now());
        db.collection::<ChannelConversation>(COLLECTION_NAME)
            .update_many(
                doc! {
                    "channel_bot_id": bot_id,
                    "user_id": user_id,
                    "default_agent": true,
                    "is_active": true,
                },
                doc! { "$set": {
                    "default_agent": false,
                    "is_active": false,
                    "updated_at": now,
                }},
            )
            .await?;
    }

    let now = Utc::now();
    let conversation = ChannelConversation {
        id: uuid::Uuid::new_v4().to_string(),
        user_id: user_id.to_string(),
        channel_bot_id: channel_bot_id.map(String::from),
        platform: platform.to_string(),
        platform_conversation_id: platform_conversation_id.to_string(),
        platform_conversation_type: platform_conversation_type.to_string(),
        platform_sender_id: platform_sender_id.map(String::from),
        agent_api_key_id: agent_api_key_id.to_string(),
        default_agent,
        is_active: true,
        last_message_at: None,
        created_at: now,
        updated_at: now,
    };

    db.collection::<ChannelConversation>(COLLECTION_NAME)
        .insert_one(&conversation)
        .await?;

    Ok(conversation)
}

/// List active conversations for a user, optionally filtered by bot.
pub async fn list_conversations(
    db: &mongodb::Database,
    user_id: &str,
    channel_bot_id: Option<&str>,
) -> AppResult<Vec<ChannelConversation>> {
    let mut filter = doc! { "user_id": user_id, "is_active": true };
    if let Some(bot_id) = channel_bot_id {
        filter.insert("channel_bot_id", bot_id);
    }

    let conversations: Vec<ChannelConversation> = db
        .collection::<ChannelConversation>(COLLECTION_NAME)
        .find(filter)
        .sort(doc! { "created_at": -1 })
        .await?
        .try_collect()
        .await?;

    Ok(conversations)
}

/// Update a conversation routing rule.
pub async fn update_conversation(
    db: &mongodb::Database,
    conversation_id: &str,
    user_id: &str,
    agent_api_key_id: Option<&str>,
    default_agent: Option<bool>,
    is_active: Option<bool>,
) -> AppResult<ChannelConversation> {
    let mut set_doc = doc! {
        "updated_at": bson::DateTime::from_chrono(Utc::now()),
    };

    if let Some(key_id) = agent_api_key_id {
        // Verify the API key belongs to the user and has a callback URL
        let api_key = db
            .collection::<ApiKey>(API_KEYS)
            .find_one(doc! { "_id": key_id, "user_id": user_id, "is_active": true })
            .await?
            .ok_or_else(|| AppError::NotFound(format!("API key not found: {key_id}")))?;

        if api_key.callback_url.is_none() {
            return Err(AppError::ValidationError(
                "API key must have a callback_url configured".to_string(),
            ));
        }
        set_doc.insert("agent_api_key_id", key_id);
    }

    if let Some(active) = is_active {
        set_doc.insert("is_active", active);
    }

    // Handle default_agent toggle: clear other defaults first
    if let Some(true) = default_agent {
        // Look up the conversation to get its bot ID
        let conv = db
            .collection::<ChannelConversation>(COLLECTION_NAME)
            .find_one(doc! { "_id": conversation_id, "user_id": user_id })
            .await?
            .ok_or_else(|| {
                AppError::NotFound(format!("Conversation not found: {conversation_id}"))
            })?;

        // default_agent is a bot-only concept: it disambiguates which agent
        // answers an inbound webhook when no exact conversation match exists.
        // Device channels have no such fan-out, so reject the toggle for them
        // rather than silently no-op'ing.
        let Some(bot_id) = conv.channel_bot_id.as_deref() else {
            return Err(AppError::ValidationError(
                "default_agent cannot be set on device conversations".to_string(),
            ));
        };

        let now = bson::DateTime::from_chrono(Utc::now());
        db.collection::<ChannelConversation>(COLLECTION_NAME)
            .update_many(
                doc! {
                    "channel_bot_id": bot_id,
                    "user_id": user_id,
                    "default_agent": true,
                    "is_active": true,
                    "_id": { "$ne": conversation_id },
                },
                doc! { "$set": { "default_agent": false, "updated_at": now }},
            )
            .await?;

        set_doc.insert("default_agent", true);
    } else if let Some(false) = default_agent {
        set_doc.insert("default_agent", false);
    }

    let updated = db
        .collection::<ChannelConversation>(COLLECTION_NAME)
        .find_one_and_update(
            doc! { "_id": conversation_id, "user_id": user_id },
            doc! { "$set": set_doc },
        )
        .return_document(mongodb::options::ReturnDocument::After)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("Conversation not found: {conversation_id}")))?;

    Ok(updated)
}

/// Soft-delete a conversation routing rule.
pub async fn delete_conversation(
    db: &mongodb::Database,
    conversation_id: &str,
    user_id: &str,
) -> AppResult<()> {
    let now = bson::DateTime::from_chrono(Utc::now());
    let result = db
        .collection::<ChannelConversation>(COLLECTION_NAME)
        .update_one(
            doc! { "_id": conversation_id, "user_id": user_id },
            doc! { "$set": {
                "is_active": false,
                "updated_at": now,
            }},
        )
        .await?;

    if result.matched_count == 0 {
        return Err(AppError::NotFound(format!(
            "Conversation not found: {conversation_id}"
        )));
    }

    Ok(())
}

/// Update the `last_message_at` timestamp on a conversation.
pub async fn touch_conversation(db: &mongodb::Database, conversation_id: &str) -> AppResult<()> {
    let now = bson::DateTime::from_chrono(Utc::now());
    db.collection::<ChannelConversation>(COLLECTION_NAME)
        .update_one(
            doc! { "_id": conversation_id },
            doc! { "$set": {
                "last_message_at": now,
                "updated_at": now,
            }},
        )
        .await?;
    Ok(())
}

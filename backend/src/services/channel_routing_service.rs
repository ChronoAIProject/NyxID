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
    owner_id: &str,
) -> AppResult<Option<AgentRoute>> {
    if !super::platform_bot_authority::permits_personal_routes(db, channel_bot_id).await? {
        return Ok(None);
    }
    let col = db.collection::<ChannelConversation>(COLLECTION_NAME);

    // Step 1: sender-specific match first (highest priority -- per-user routing in groups)
    let conversation = match platform_sender_id {
        Some(sender_id) if !sender_id.is_empty() => {
            col.find_one(doc! {
                "channel_bot_id": channel_bot_id,
                "user_id": owner_id,
                "platform_conversation_id": platform_conversation_id,
                "platform_sender_id": sender_id,
                "is_active": true,
                "retired_by_transfer": { "$ne": true },
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
                "user_id": owner_id,
                "platform_conversation_id": platform_conversation_id,
                "platform_sender_id": null,
                "is_active": true,
                "retired_by_transfer": { "$ne": true },
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
                "user_id": owner_id,
                "default_agent": true,
                "is_active": true,
                "retired_by_transfer": { "$ne": true },
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
            "user_id": owner_id,
            "is_active": true,
        })
        .await?;

    let callback_url = match api_key
        .filter(|key| ensure_route_agent(key, owner_id).is_ok())
        .and_then(|k| k.callback_url)
    {
        Some(url) if !url.is_empty() => url,
        _ => return Ok(None),
    };

    Ok(Some(AgentRoute {
        api_key_id: conversation.agent_api_key_id.clone(),
        conversation,
        callback_url,
    }))
}

pub fn ensure_route_agent(key: &ApiKey, owner_id: &str) -> AppResult<()> {
    if key.user_id != owner_id {
        return Err(AppError::ValidationError(
            "Agent key and conversation must have the same owner".into(),
        ));
    }
    if !key.is_active || key.expires_at.is_some_and(|expiry| expiry <= Utc::now()) {
        return Err(AppError::ValidationError(
            "Agent key is inactive or expired".into(),
        ));
    }
    if key.platform.as_deref()
        == Some(super::assistant_agent_credential_service::ASSISTANT_PLATFORM)
    {
        return Err(AppError::ValidationError(
            "Assistant chat keys cannot be used as channel route agents".into(),
        ));
    }
    if key
        .callback_url
        .as_deref()
        .is_none_or(|url| url.trim().is_empty())
    {
        return Err(AppError::ValidationError(
            "API key must have a callback_url configured".into(),
        ));
    }
    Ok(())
}

pub(crate) async fn load_callback_key(
    db: &mongodb::Database,
    route: &AgentRoute,
    owner_id: &str,
) -> AppResult<ApiKey> {
    if let Some(bot_id) = route.conversation.channel_bot_id.as_deref() {
        super::platform_bot_authority::require_personal_routes(db, bot_id).await?;
    }
    let key = db
        .collection::<ApiKey>(API_KEYS)
        .find_one(doc! { "_id": &route.api_key_id })
        .await?
        .ok_or_else(|| AppError::NotFound("Route agent key is unavailable".into()))?;
    ensure_route_agent(&key, owner_id)?;
    if route.conversation.user_id != owner_id
        || key.callback_url.as_deref() != Some(route.callback_url.as_str())
    {
        return Err(AppError::Conflict(
            "Route agent callback or owner changed before dispatch".into(),
        ));
    }
    Ok(key)
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
    allow_agent_initiated: bool,
) -> AppResult<ChannelConversation> {
    // Verify the API key exists and belongs to the user
    let api_key = db
        .collection::<ApiKey>(API_KEYS)
        .find_one(doc! { "_id": agent_api_key_id, "user_id": user_id, "is_active": true })
        .await?
        .ok_or_else(|| AppError::NotFound(format!("API key not found: {agent_api_key_id}")))?;

    ensure_route_agent(&api_key, user_id)?;
    if let Some(bot_id) = channel_bot_id {
        super::platform_bot_authority::refuse_platform_bot(db, bot_id).await?;
    }

    let now = Utc::now();
    let conversation = ChannelConversation {
        activity_callback: None,
        id: uuid::Uuid::new_v4().to_string(),
        user_id: user_id.to_string(),
        channel_bot_id: channel_bot_id.map(String::from),
        platform: platform.to_string(),
        platform_conversation_id: platform_conversation_id.to_string(),
        platform_conversation_type: platform_conversation_type.to_string(),
        platform_sender_id: platform_sender_id.map(String::from),
        agent_api_key_id: agent_api_key_id.to_string(),
        default_agent,
        allow_agent_initiated,
        is_active: true,
        last_message_at: None,
        created_at: now,
        updated_at: now,
    };

    if channel_bot_id.is_some() {
        insert_bot_conversation(db, &conversation).await?;
    } else {
        insert_device_conversation(db, &conversation).await?;
    }

    Ok(conversation)
}

/// Write-fence a route's agent key inside the route transaction, so binding a
/// key conflicts with a concurrent revocation (for example manager conversion)
/// instead of leaving an active route on a revoked key.
pub(crate) async fn fence_route_key(
    db: &mongodb::Database,
    key_id: &str,
    session: &mut mongodb::ClientSession,
) -> AppResult<()> {
    let fenced = db
        .collection::<bson::Document>(API_KEYS)
        .update_one(
            doc! {"_id": key_id, "is_active": true},
            doc! {"$inc": {"route_binding_revision": 1_i64}},
        )
        .session(&mut *session)
        .await?;
    if fenced.matched_count != 1 {
        return Err(AppError::Conflict(
            "Route agent key is no longer active".into(),
        ));
    }
    Ok(())
}

async fn insert_device_conversation(
    db: &mongodb::Database,
    conversation: &ChannelConversation,
) -> AppResult<()> {
    use super::api_key_mutation_service::{map_transaction_error, transaction_result};
    let db = db.clone();
    let conversation = conversation.clone();
    let mut session = db.client().start_session().await?;
    session
        .start_transaction()
        .and_run2(async move |session| {
            let result: AppResult<()> = async {
                fence_route_key(&db, &conversation.agent_api_key_id, session).await?;
                db.collection::<ChannelConversation>(COLLECTION_NAME)
                    .insert_one(&conversation)
                    .session(&mut *session)
                    .await?;
                Ok(())
            }
            .await;
            transaction_result(result)
        })
        .await
        .map_err(map_transaction_error)
}

async fn insert_bot_conversation(
    db: &mongodb::Database,
    conversation: &ChannelConversation,
) -> AppResult<()> {
    use super::api_key_mutation_service::{map_transaction_error, transaction_result};
    let db = db.clone();
    let conversation = conversation.clone();
    let mut session = db.client().start_session().await?;
    session
        .start_transaction()
        .and_run2(async move |session| {
            let result: AppResult<()> =
                async {
                    let bot_id = conversation.channel_bot_id.as_deref().expect("bot route");
                    // Serialize route creation against ownership transfer on the bot row.
                    let bot = db
                        .collection::<bson::Document>(crate::models::channel_bot::COLLECTION_NAME)
                        .update_one(
                            doc! { "_id": bot_id, "user_id": &conversation.user_id,
                            "platform": &conversation.platform, "is_active": true,
                            "platform_profile_id": {"$exists": false}, "platform_installation_id": {"$exists": false},
                            "platform_entry_version":{"$exists":false},"platform_entry_pending":{"$exists":false} },
                            doc! { "$inc": { "route_revision": 1_i64 } },
                        )
                        .session(&mut *session)
                        .await?;
                    if bot.matched_count != 1 {
                        return Err(AppError::Conflict(
                            "Channel bot changed owner or is no longer active".into(),
                        ));
                    }
                    super::platform_bot_authority::fence_personal_routes(
                        &db,
                        bot_id,
                        &conversation.user_id,
                        session,
                    )
                    .await?;
                    fence_route_key(&db, &conversation.agent_api_key_id, session).await?;
                    if conversation.default_agent {
                        db.collection::<ChannelConversation>(COLLECTION_NAME).update_many(
                    doc! { "channel_bot_id": bot_id, "user_id": &conversation.user_id,
                        "default_agent": true, "is_active": true },
                    doc! { "$set": { "default_agent": false, "is_active": false,
                        "updated_at": bson::DateTime::from_chrono(conversation.updated_at) } },
                ).session(&mut *session).await?;
                    }
                    db.collection::<ChannelConversation>(COLLECTION_NAME)
                        .insert_one(&conversation)
                        .session(&mut *session)
                        .await?;
                    Ok(())
                }
                .await;
            transaction_result(result)
        })
        .await
        .map_err(map_transaction_error)
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
    allow_agent_initiated: Option<bool>,
) -> AppResult<ChannelConversation> {
    let current = db
        .collection::<ChannelConversation>(COLLECTION_NAME)
        .find_one(doc! {"_id": conversation_id, "user_id": user_id, "retired_by_transfer": { "$ne": true }})
        .await?
        .ok_or_else(|| AppError::NotFound("Conversation not found".into()))?;
    if let Some(bot_id) = current.channel_bot_id.as_deref() {
        super::platform_bot_authority::require_personal_routes(db, bot_id).await?;
    }
    // Permission reductions remain available even when the route's key is unusable.
    if agent_api_key_id.is_some()
        || is_active == Some(true)
        || default_agent == Some(true)
        || allow_agent_initiated == Some(true)
    {
        let key = db
            .collection::<ApiKey>(API_KEYS)
            .find_one(doc! {"_id": agent_api_key_id.unwrap_or(&current.agent_api_key_id)})
            .await?
            .ok_or_else(|| AppError::NotFound("Route agent key is unavailable".into()))?;
        ensure_route_agent(&key, user_id)?;
    }
    let mut set_doc = doc! {
        "updated_at": bson::DateTime::from_chrono(Utc::now()),
    };

    if let Some(key_id) = agent_api_key_id {
        set_doc.insert("agent_api_key_id", key_id);
        set_doc.insert("activity_callback", bson::Bson::Null);
    }

    if let Some(active) = is_active {
        set_doc.insert("is_active", active);
    }

    if let Some(allowed) = allow_agent_initiated {
        set_doc.insert("allow_agent_initiated", allowed);
    }

    if default_agent == Some(true) && current.channel_bot_id.is_none() {
        return Err(AppError::ValidationError(
            "default_agent cannot be set on device conversations".into(),
        ));
    }
    if let Some(default) = default_agent {
        set_doc.insert("default_agent", default);
    }
    if current.channel_bot_id.is_some() {
        return mutate_bot_conversation(db, &current, set_doc, default_agent == Some(true)).await;
    }

    if let Some(key) = widened_route_key(&set_doc, &current).map(str::to_owned) {
        use super::api_key_mutation_service::{map_transaction_error, transaction_result};
        let db = db.clone();
        let (conversation_id, user_id) = (conversation_id.to_owned(), user_id.to_owned());
        let mut session = db.client().start_session().await?;
        return session
            .start_transaction()
            .and_run2(async move |session| {
                let result: AppResult<ChannelConversation> = async {
                    fence_route_key(&db, &key, session).await?;
                    let mut filter = doc! { "_id": &conversation_id, "user_id": &user_id, "retired_by_transfer": { "$ne": true } };
                    pin_fenced_key(&mut filter, &set_doc, &key);
                    db.collection::<ChannelConversation>(COLLECTION_NAME)
                        .find_one_and_update(filter, doc! { "$set": set_doc.clone() })
                        .return_document(mongodb::options::ReturnDocument::After)
                        .session(&mut *session)
                        .await?
                        .ok_or_else(|| AppError::NotFound(format!("Conversation not found: {conversation_id}")))
                }
                .await;
                transaction_result(result)
            })
            .await
            .map_err(map_transaction_error);
    }

    let updated = db
        .collection::<ChannelConversation>(COLLECTION_NAME)
        .find_one_and_update(
            doc! { "_id": conversation_id, "user_id": user_id, "retired_by_transfer": { "$ne": true } },
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
    let current = db
        .collection::<ChannelConversation>(COLLECTION_NAME)
        .find_one(doc! {"_id": conversation_id, "user_id": user_id})
        .await?
        .ok_or_else(|| AppError::NotFound("Conversation not found".into()))?;
    // Deleting an already inactive route changes nothing, so account and
    // channel cleanup succeed even after the bot came under platform control.
    if !current.is_active {
        return Ok(());
    }
    if let Some(bot_id) = current.channel_bot_id.as_deref() {
        // Removing a route only narrows access: orphaned and transferred routes
        // keep the unconditional soft-delete; platform bots stay fenced.
        let fenced = db
            .collection::<bson::Document>(crate::models::channel_bot::COLLECTION_NAME)
            .find_one(doc! {"_id": bot_id, "user_id": user_id})
            .projection(doc! {"_id": 1})
            .await?
            .is_some()
            && db
                .collection::<bson::Document>(COLLECTION_NAME)
                .find_one(doc! {"_id": conversation_id, "retired_by_transfer": {"$ne": true}})
                .projection(doc! {"_id": 1})
                .await?
                .is_some();
        if fenced {
            super::platform_bot_authority::require_personal_routes(db, bot_id).await?;
            mutate_bot_conversation(
                db,
                &current,
                doc! {"is_active": false, "updated_at": bson::DateTime::now()},
                false,
            )
            .await?;
            return Ok(());
        }
    }
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

/// The key a route update binds or re-enables; reductions need no key fence.
/// Re-enabling keeps the route's key, so require the route to still hold the
/// key that was fenced; a concurrent rebind must not be re-enabled unfenced.
fn pin_fenced_key(filter: &mut bson::Document, set: &bson::Document, key: &str) {
    if !set.contains_key("agent_api_key_id") {
        filter.insert("agent_api_key_id", key);
    }
}

fn widened_route_key<'a>(
    set: &'a bson::Document,
    current: &'a ChannelConversation,
) -> Option<&'a str> {
    let rebinds = set.get_str("agent_api_key_id").ok();
    if rebinds.is_some() || set.get_bool("is_active") == Ok(true) {
        return Some(rebinds.unwrap_or(&current.agent_api_key_id));
    }
    None
}

async fn mutate_bot_conversation(
    db: &mongodb::Database,
    current: &ChannelConversation,
    set: bson::Document,
    default: bool,
) -> AppResult<ChannelConversation> {
    use super::api_key_mutation_service::{map_transaction_error, transaction_result};
    let db = db.clone();
    let current = current.clone();
    let mut session = db.client().start_session().await?;
    session.start_transaction().and_run2(async move |session| {
        let result: AppResult<_> = async {
            let bot = current.channel_bot_id.as_deref().expect("bot conversation");
            super::platform_bot_authority::fence_personal_routes(&db, bot, &current.user_id, session).await?;
            let mut filter = doc! {"_id": &current.id, "user_id": &current.user_id, "channel_bot_id": bot, "retired_by_transfer": {"$ne": true}};
            if let Some(key) = widened_route_key(&set, &current) {
                fence_route_key(&db, key, session).await?;
                pin_fenced_key(&mut filter, &set, key);
            }
            if default {
                db.collection::<ChannelConversation>(COLLECTION_NAME).update_many(
                    doc! {"channel_bot_id": bot, "user_id": &current.user_id, "default_agent": true, "is_active": true, "_id": {"$ne": &current.id}},
                    doc! {"$set": {"default_agent": false, "updated_at": bson::DateTime::now()}}
                ).session(&mut *session).await?;
            }
            db.collection::<ChannelConversation>(COLLECTION_NAME).find_one_and_update(
                filter,
                doc! {"$set": set.clone()}
            ).return_document(mongodb::options::ReturnDocument::After).session(&mut *session).await?
                .ok_or_else(|| AppError::Conflict("Conversation changed before update".into()))
        }.await;
        transaction_result(result)
    }).await.map_err(map_transaction_error)
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

/// Retire rollback routes by the dedicated credential, including routes whose
/// creation succeeded before the channel stored the returned route id.
pub(crate) async fn retire_key_routes_in_session(
    db: &mongodb::Database,
    owner: &str,
    bot: &str,
    key: &str,
    session: &mut mongodb::ClientSession,
) -> AppResult<()> {
    db.collection::<ChannelConversation>(COLLECTION_NAME).update_many(
        doc! {"user_id":owner,"channel_bot_id":bot,"agent_api_key_id":key},
        doc! {"$set":{"is_active":false,"default_agent":false,"updated_at":bson::DateTime::now()}},
    ).session(session).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::*;

    fn make_api_key(key_id: &str, user_id: &str, callback_url: Option<&str>) -> ApiKey {
        ApiKey {
            id: key_id.to_string(),
            user_id: user_id.to_string(),
            name: "test-agent".to_string(),
            key_prefix: "nyxid_ag".to_string(),
            key_hash: "deadbeef".repeat(8),
            scopes: "read write".to_string(),
            last_used_at: None,
            expires_at: None,
            is_active: true,
            created_at: Utc::now(),
            rotation_predecessor_id: None,
            machine_external_grants_present: false,
            state_version: 1,
            updated_at: Some(Utc::now()),
            description: None,
            allowed_service_ids: vec![],
            allowed_platform_service_ids: Vec::new(),
            assistant_group_id: None,
            assistant_agent_owner_id: None,
            assistant_operation_scopes: Default::default(),
            allowed_node_ids: vec![],
            allow_all_services: true,
            allow_auto_connected_services: false,
            allow_all_nodes: true,
            rate_limit_per_second: None,
            rate_limit_burst: None,
            platform: None,
            callback_url: callback_url.map(String::from),
            purpose: Default::default(),
            scheduled_write_enabled: false,
        }
    }

    async fn seed_bot(db: &mongodb::Database, bot_id: &str, user_id: &str, platform: &str) {
        db.collection::<bson::Document>(crate::models::channel_bot::COLLECTION_NAME)
            .insert_one(doc! {
                "_id": bot_id,
                "user_id": user_id,
                "platform": platform,
                "is_active": true,
                "ownership_version": 0_i64,
            })
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn platform_onboarding_manager_refuses_alternate_setters_and_cached_dispatch() {
        let db = connect_transaction_test_database("platform_manager_routes").await;
        let owner = uuid::Uuid::new_v4().to_string();
        let bot = uuid::Uuid::new_v4().to_string();
        let key = uuid::Uuid::new_v4().to_string();
        seed_bot(&db, &bot, &owner, "telegram").await;
        db.collection::<bson::Document>(crate::models::channel_bot::COLLECTION_NAME)
            .update_one(
                doc! {"_id": &bot},
                doc! {"$set": {"credential_source": "telegram_manager"}},
            )
            .await
            .unwrap();
        db.collection::<bson::Document>(crate::models::platform_bot_config::COLLECTION_NAME)
            .insert_one(doc! {"_id": uuid::Uuid::new_v4().to_string(), "channel_bot_id": &bot, "manager_bot_id": "123", "enabled": false}).await.unwrap();
        db.collection::<bson::Document>(crate::models::channel_bot::COLLECTION_NAME)
            .update_one(
                doc! {"_id": &bot},
                doc! {"$set": {"platform_profile_id": "profile"}},
            )
            .await
            .unwrap();

        db.collection::<ApiKey>(API_KEYS)
            .insert_one(make_api_key(
                &key,
                &owner,
                Some("https://agent.test/callback"),
            ))
            .await
            .unwrap();

        for default in [false, true] {
            let error = create_conversation(
                &db,
                &owner,
                Some(&bot),
                "telegram",
                "chat",
                "private",
                None,
                &key,
                default,
                true,
            )
            .await
            .unwrap_err();
            assert!(matches!(error, AppError::Forbidden(_)));
        }
        assert_eq!(
            db.collection::<ChannelConversation>(COLLECTION_NAME)
                .count_documents(doc! {})
                .await
                .unwrap(),
            0
        );

        // Persist an old-writer route to prove ingress and late dispatch do not
        // trust previously authorized exact, sender or default routes.
        for (chat, sender, default) in [
            ("chat", None, false),
            ("group", Some("sender"), false),
            ("*", None, true),
        ] {
            let now = Utc::now();
            let conversation = ChannelConversation {
                activity_callback: None,
                id: uuid::Uuid::new_v4().to_string(),
                user_id: owner.clone(),
                channel_bot_id: Some(bot.clone()),
                platform: "telegram".into(),
                platform_conversation_id: chat.into(),
                platform_conversation_type: "private".into(),
                platform_sender_id: sender.map(str::to_owned),
                agent_api_key_id: key.clone(),
                default_agent: default,
                allow_agent_initiated: true,
                is_active: true,
                last_message_at: None,
                created_at: now,
                updated_at: now,
            };
            db.collection::<ChannelConversation>(COLLECTION_NAME)
                .insert_one(&conversation)
                .await
                .unwrap();
            assert!(
                resolve_agent(&db, &bot, chat, sender, &owner)
                    .await
                    .unwrap()
                    .is_none()
            );
            let route = AgentRoute {
                conversation: conversation.clone(),
                callback_url: "https://agent.test/callback".into(),
                api_key_id: key.clone(),
            };
            assert!(matches!(
                load_callback_key(&db, &route, &owner).await,
                Err(AppError::Forbidden(_))
            ));
            for active in [false, true] {
                assert!(matches!(
                    update_conversation(
                        &db,
                        &conversation.id,
                        &owner,
                        Some(&key),
                        Some(true),
                        Some(active),
                        Some(true)
                    )
                    .await,
                    Err(AppError::Forbidden(_))
                ));
            }
            assert!(matches!(
                delete_conversation(&db, &conversation.id, &owner).await,
                Err(AppError::Forbidden(_))
            ));
            let persisted = db
                .collection::<ChannelConversation>(COLLECTION_NAME)
                .find_one(doc! {"_id": &conversation.id})
                .await
                .unwrap()
                .unwrap();
            assert!(persisted.is_active);
            assert_eq!(persisted.default_agent, default);
        }
        assert!(
            resolve_agent(&db, &bot, "unmatched", Some("administrator"), &owner)
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn platform_onboarding_preserves_personal_route_creation_and_default_dispatch() {
        let db = connect_transaction_test_database("platform_personal_routes").await;
        let owner = uuid::Uuid::new_v4().to_string();
        let bot = uuid::Uuid::new_v4().to_string();
        let key = uuid::Uuid::new_v4().to_string();
        seed_bot(&db, &bot, &owner, "telegram").await;
        db.collection::<bson::Document>(crate::models::channel_bot::COLLECTION_NAME)
            .update_one(
                doc! {"_id": &bot},
                doc! {"$set": {"credential_source": "telegram_manager"}},
            )
            .await
            .unwrap();

        db.collection::<ApiKey>(API_KEYS)
            .insert_one(make_api_key(
                &key,
                &owner,
                Some("https://agent.test/callback"),
            ))
            .await
            .unwrap();
        let conversation = create_conversation(
            &db,
            &owner,
            Some(&bot),
            "telegram",
            "*",
            "private",
            None,
            &key,
            true,
            false,
        )
        .await
        .unwrap();
        let route = resolve_agent(&db, &bot, "unmatched", Some("owner"), &owner)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(route.conversation.id, conversation.id);
        assert_eq!(
            load_callback_key(&db, &route, &owner).await.unwrap().id,
            key
        );
        update_conversation(
            &db,
            &conversation.id,
            &owner,
            None,
            Some(false),
            Some(false),
            None,
        )
        .await
        .unwrap();
        delete_conversation(&db, &conversation.id, &owner)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn platform_onboarding_route_creation_conflicts_with_concurrent_key_revocation() {
        let db = connect_transaction_test_database("platform_route_key_fence").await;
        let owner = uuid::Uuid::new_v4().to_string();
        let bot = uuid::Uuid::new_v4().to_string();
        seed_bot(&db, &bot, &owner, "telegram").await;
        let key = make_api_key(
            &uuid::Uuid::new_v4().to_string(),
            &owner,
            Some("https://agent.test/callback"),
        );
        db.collection::<ApiKey>(API_KEYS)
            .insert_one(&key)
            .await
            .unwrap();
        // Revocation (as in manager conversion) is written but not yet committed.
        let mut revocation = db.client().start_session().await.unwrap();
        revocation.start_transaction().await.unwrap();
        db.collection::<bson::Document>(API_KEYS)
            .update_one(doc! {"_id": &key.id}, doc! {"$set": {"is_active": false}})
            .session(&mut revocation)
            .await
            .unwrap();
        let creation = {
            let (db, owner, bot, key) = (db.clone(), owner.clone(), bot.clone(), key.id.clone());
            tokio::spawn(async move {
                create_conversation(
                    &db,
                    &owner,
                    Some(&bot),
                    "telegram",
                    "chat",
                    "private",
                    None,
                    &key,
                    false,
                    false,
                )
                .await
            })
        };
        tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        revocation.commit_transaction().await.unwrap();
        assert!(creation.await.unwrap().is_err());
        assert_eq!(
            db.collection::<ChannelConversation>(COLLECTION_NAME)
                .count_documents(doc! {"channel_bot_id": &bot, "is_active": true})
                .await
                .unwrap(),
            0
        );
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn platform_onboarding_stale_reactivation_cannot_enable_a_rebound_revoked_key() {
        let db = connect_transaction_test_database("platform_route_stale_reactivation").await;
        let owner = uuid::Uuid::new_v4().to_string();
        let bot = uuid::Uuid::new_v4().to_string();
        seed_bot(&db, &bot, &owner, "telegram").await;
        let first = make_api_key(
            &uuid::Uuid::new_v4().to_string(),
            &owner,
            Some("https://agent.test/callback"),
        );
        let mut second = make_api_key(
            &uuid::Uuid::new_v4().to_string(),
            &owner,
            Some("https://agent.test/callback"),
        );
        second.is_active = false;
        let keys = db.collection::<ApiKey>(API_KEYS);
        keys.insert_one(&first).await.unwrap();
        keys.insert_one(&second).await.unwrap();
        let route = create_conversation(
            &db,
            &owner,
            Some(&bot),
            "telegram",
            "chat",
            "private",
            None,
            &first.id,
            false,
            false,
        )
        .await
        .unwrap();
        let mut stale = route.clone();
        stale.is_active = false;
        // Meanwhile the route was paused, rebound to a key that was then revoked.
        db.collection::<ChannelConversation>(COLLECTION_NAME)
            .update_one(
                doc! {"_id": &route.id},
                doc! {"$set": {"is_active": false, "agent_api_key_id": &second.id}},
            )
            .await
            .unwrap();
        assert!(
            mutate_bot_conversation(
                &db,
                &stale,
                doc! {"is_active": true, "updated_at": bson::DateTime::now()},
                false,
            )
            .await
            .is_err()
        );
        let persisted = db
            .collection::<ChannelConversation>(COLLECTION_NAME)
            .find_one(doc! {"_id": &route.id})
            .await
            .unwrap()
            .unwrap();
        assert!(!persisted.is_active);
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn platform_onboarding_inactive_route_on_converted_bot_can_still_be_deleted() {
        let db = connect_transaction_test_database("platform_inactive_route_delete").await;
        let owner = uuid::Uuid::new_v4().to_string();
        let bot = uuid::Uuid::new_v4().to_string();
        seed_bot(&db, &bot, &owner, "telegram").await;
        let key = make_api_key(
            &uuid::Uuid::new_v4().to_string(),
            &owner,
            Some("https://agent.test/callback"),
        );
        db.collection::<ApiKey>(API_KEYS)
            .insert_one(&key)
            .await
            .unwrap();
        let conversation = create_conversation(
            &db,
            &owner,
            Some(&bot),
            "telegram",
            "chat",
            "private",
            None,
            &key.id,
            true,
            true,
        )
        .await
        .unwrap();
        // Conversion marks the bot and deactivates its routes; account and
        // channel cleanup must still be able to remove them.
        db.collection::<bson::Document>(crate::models::channel_bot::COLLECTION_NAME)
            .update_one(
                doc! {"_id": &bot},
                doc! {"$set": {"platform_profile_id": "profile"}},
            )
            .await
            .unwrap();
        db.collection::<ChannelConversation>(COLLECTION_NAME)
            .update_one(
                doc! {"_id": &conversation.id},
                doc! {"$set": {"is_active": false}},
            )
            .await
            .unwrap();
        delete_conversation(&db, &conversation.id, &owner)
            .await
            .unwrap();
        let persisted = db
            .collection::<ChannelConversation>(COLLECTION_NAME)
            .find_one(doc! {"_id": &conversation.id})
            .await
            .unwrap()
            .unwrap();
        assert!(!persisted.is_active);
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn platform_onboarding_deleted_bot_rejects_orphan_routes_and_cached_callback() {
        let db = connect_transaction_test_database("platform_orphan_route").await;
        let owner = uuid::Uuid::new_v4().to_string();
        let bot = uuid::Uuid::new_v4().to_string();
        seed_bot(&db, &bot, &owner, "telegram").await;
        let key = make_api_key(
            &uuid::Uuid::new_v4().to_string(),
            &owner,
            Some("https://agent.test/callback"),
        );
        db.collection::<ApiKey>(API_KEYS)
            .insert_one(&key)
            .await
            .unwrap();
        let conversation = create_conversation(
            &db,
            &owner,
            Some(&bot),
            "telegram",
            "chat",
            "private",
            None,
            &key.id,
            true,
            true,
        )
        .await
        .unwrap();
        let route = resolve_agent(&db, &bot, "chat", None, &owner)
            .await
            .unwrap()
            .unwrap();
        db.collection::<bson::Document>(crate::models::channel_bot::COLLECTION_NAME)
            .delete_one(doc! {"_id": &bot})
            .await
            .unwrap();
        assert!(
            resolve_agent(&db, &bot, "chat", None, &owner)
                .await
                .unwrap()
                .is_none()
        );
        assert!(load_callback_key(&db, &route, &owner).await.is_err());
        assert!(
            update_conversation(&db, &conversation.id, &owner, None, Some(true), None, None)
                .await
                .is_err()
        );
        delete_conversation(&db, &conversation.id, &owner)
            .await
            .unwrap();
        let persisted = db
            .collection::<ChannelConversation>(COLLECTION_NAME)
            .find_one(doc! {"_id": &conversation.id})
            .await
            .unwrap()
            .unwrap();
        assert!(!persisted.is_active);
        db.drop().await.unwrap();
    }

    #[tokio::test]
    async fn test_create_conversation() {
        let Some(db) = connect_test_database("chan_route_create").await else {
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();
        let bot_id = uuid::Uuid::new_v4().to_string();
        db.collection::<bson::Document>(crate::models::channel_bot::COLLECTION_NAME)
            .insert_one(doc! { "_id": &bot_id, "user_id": &user_id, "platform": "telegram", "is_active": true })
            .await.unwrap();
        let key_id = uuid::Uuid::new_v4().to_string();

        db.collection::<ApiKey>(API_KEYS)
            .insert_one(make_api_key(
                &key_id,
                &user_id,
                Some("https://agent.test/callback"),
            ))
            .await
            .unwrap();

        let conv = create_conversation(
            &db,
            &user_id,
            Some(&bot_id),
            "telegram",
            "chat_123",
            "private",
            None,
            &key_id,
            false,
            false,
        )
        .await
        .unwrap();

        assert_eq!(conv.user_id, user_id);
        assert_eq!(conv.channel_bot_id.as_deref(), Some(bot_id.as_str()));
        assert_eq!(conv.platform, "telegram");
        assert_eq!(conv.platform_conversation_id, "chat_123");
        assert!(conv.is_active);
        assert!(!conv.default_agent);
    }

    #[tokio::test]
    async fn test_create_conversation_no_callback_url_rejected() {
        let Some(db) = connect_test_database("chan_route_nocb").await else {
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();
        let bot_id = uuid::Uuid::new_v4().to_string();
        db.collection::<bson::Document>(crate::models::channel_bot::COLLECTION_NAME)
            .insert_one(doc! { "_id": &bot_id, "user_id": &user_id, "platform": "telegram", "is_active": true })
            .await.unwrap();
        let key_id = uuid::Uuid::new_v4().to_string();

        db.collection::<ApiKey>(API_KEYS)
            .insert_one(make_api_key(&key_id, &user_id, None))
            .await
            .unwrap();

        let err = create_conversation(
            &db,
            &user_id,
            Some(&bot_id),
            "telegram",
            "chat_456",
            "private",
            None,
            &key_id,
            false,
            false,
        )
        .await
        .unwrap_err();
        assert!(matches!(err, AppError::ValidationError(_)));
    }

    #[tokio::test]
    async fn test_resolve_agent_exact_conversation_match() {
        let Some(db) = connect_test_database("chan_route_exact").await else {
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();
        let bot_id = uuid::Uuid::new_v4().to_string();
        db.collection::<bson::Document>(crate::models::channel_bot::COLLECTION_NAME)
            .insert_one(doc! { "_id": &bot_id, "user_id": &user_id, "platform": "telegram", "is_active": true })
            .await.unwrap();
        let key_id = uuid::Uuid::new_v4().to_string();

        db.collection::<ApiKey>(API_KEYS)
            .insert_one(make_api_key(
                &key_id,
                &user_id,
                Some("https://agent.test/hook"),
            ))
            .await
            .unwrap();

        let conv = create_conversation(
            &db,
            &user_id,
            Some(&bot_id),
            "telegram",
            "chat_789",
            "group",
            None,
            &key_id,
            false,
            false,
        )
        .await
        .unwrap();

        let route = resolve_agent(&db, &bot_id, "chat_789", None, &user_id)
            .await
            .unwrap();
        assert!(route.is_some());
        let route = route.unwrap();
        assert_eq!(route.conversation.id, conv.id);
        assert_eq!(route.callback_url, "https://agent.test/hook");
        assert_eq!(route.api_key_id, key_id);
    }

    #[tokio::test]
    async fn test_resolve_agent_default_fallback() {
        let Some(db) = connect_test_database("chan_route_default").await else {
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();
        let bot_id = uuid::Uuid::new_v4().to_string();
        db.collection::<bson::Document>(crate::models::channel_bot::COLLECTION_NAME)
            .insert_one(doc! { "_id": &bot_id, "user_id": &user_id, "platform": "telegram", "is_active": true })
            .await.unwrap();
        let key_id = uuid::Uuid::new_v4().to_string();

        db.collection::<ApiKey>(API_KEYS)
            .insert_one(make_api_key(
                &key_id,
                &user_id,
                Some("https://default.test/cb"),
            ))
            .await
            .unwrap();

        create_conversation(
            &db,
            &user_id,
            Some(&bot_id),
            "telegram",
            "*",
            "private",
            None,
            &key_id,
            true,
            false,
        )
        .await
        .unwrap();

        let route = resolve_agent(&db, &bot_id, "unknown_chat", None, &user_id)
            .await
            .unwrap();
        assert!(route.is_some());
        assert_eq!(route.unwrap().callback_url, "https://default.test/cb");
    }

    #[tokio::test]
    async fn test_resolve_agent_no_route() {
        let Some(db) = connect_test_database("chan_route_none").await else {
            return;
        };
        let bot_id = uuid::Uuid::new_v4().to_string();

        let route = resolve_agent(&db, &bot_id, "nonexistent", None, "nobody")
            .await
            .unwrap();
        assert!(route.is_none());
    }

    #[tokio::test]
    async fn test_list_and_delete_conversation() {
        let Some(db) = connect_test_database("chan_route_list_del").await else {
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();
        let bot_id = uuid::Uuid::new_v4().to_string();
        db.collection::<bson::Document>(crate::models::channel_bot::COLLECTION_NAME)
            .insert_one(doc! { "_id": &bot_id, "user_id": &user_id, "platform": "discord", "is_active": true })
            .await.unwrap();
        let key_id = uuid::Uuid::new_v4().to_string();

        db.collection::<ApiKey>(API_KEYS)
            .insert_one(make_api_key(&key_id, &user_id, Some("https://x.test/cb")))
            .await
            .unwrap();

        let conv = create_conversation(
            &db,
            &user_id,
            Some(&bot_id),
            "discord",
            "guild_1",
            "group",
            None,
            &key_id,
            false,
            false,
        )
        .await
        .unwrap();

        let list = list_conversations(&db, &user_id, Some(&bot_id))
            .await
            .unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].id, conv.id);

        delete_conversation(&db, &conv.id, &user_id).await.unwrap();

        let list_after = list_conversations(&db, &user_id, Some(&bot_id))
            .await
            .unwrap();
        assert!(list_after.is_empty());
    }

    #[tokio::test]
    async fn test_touch_conversation() {
        let Some(db) = connect_test_database("chan_route_touch").await else {
            return;
        };
        let user_id = uuid::Uuid::new_v4().to_string();
        let bot_id = uuid::Uuid::new_v4().to_string();
        db.collection::<bson::Document>(crate::models::channel_bot::COLLECTION_NAME)
            .insert_one(
                doc! { "_id": &bot_id, "user_id": &user_id, "platform": "lark", "is_active": true },
            )
            .await
            .unwrap();
        let key_id = uuid::Uuid::new_v4().to_string();

        db.collection::<ApiKey>(API_KEYS)
            .insert_one(make_api_key(&key_id, &user_id, Some("https://t.test/cb")))
            .await
            .unwrap();

        let conv = create_conversation(
            &db,
            &user_id,
            Some(&bot_id),
            "lark",
            "chat_t",
            "private",
            None,
            &key_id,
            false,
            false,
        )
        .await
        .unwrap();
        assert!(conv.last_message_at.is_none());

        touch_conversation(&db, &conv.id).await.unwrap();

        let updated = db
            .collection::<ChannelConversation>(COLLECTION_NAME)
            .find_one(doc! { "_id": &conv.id })
            .await
            .unwrap()
            .unwrap();
        assert!(updated.last_message_at.is_some());
    }
    #[tokio::test]
    async fn route_writes_resolution_and_final_callback_read_enforce_eligibility() {
        let db = crate::test_utils::connect_transaction_test_database("wa_route_eligibility").await;
        for (name, mutation) in [
            (
                "expired",
                doc! {"expires_at": bson::DateTime::from_chrono(Utc::now()-chrono::Duration::seconds(1))},
            ),
            ("inactive", doc! {"is_active":false}),
            ("owner", doc! {"user_id":"other-owner"}),
            ("assistant", doc! {"platform":"nyxid-assistant"}),
            ("missing_callback", doc! {"callback_url":bson::Bson::Null}),
            ("empty_callback", doc! {"callback_url":"  "}),
        ] {
            let owner = uuid::Uuid::new_v4().to_string();
            let bot = uuid::Uuid::new_v4().to_string();
            seed_bot(&db, &bot, &owner, "whatsapp").await;
            let key = make_api_key(
                &uuid::Uuid::new_v4().to_string(),
                &owner,
                Some("https://agent.test/callback"),
            );
            db.collection::<ApiKey>(API_KEYS)
                .insert_one(&key)
                .await
                .unwrap();
            let route = create_conversation(
                &db,
                &owner,
                Some(&bot),
                "whatsapp",
                "123",
                "private",
                None,
                &key.id,
                false,
                false,
            )
            .await
            .unwrap();
            let resolved = resolve_agent(&db, &bot, "123", None, &owner)
                .await
                .unwrap()
                .unwrap();
            assert!(load_callback_key(&db, &resolved, &owner).await.is_ok());
            db.collection::<ApiKey>(API_KEYS)
                .update_one(doc! {"_id":&key.id}, doc! {"$set":mutation})
                .await
                .unwrap();
            assert!(
                load_callback_key(&db, &resolved, &owner).await.is_err(),
                "final read accepted {name}"
            );
            assert!(
                resolve_agent(&db, &bot, "123", None, &owner)
                    .await
                    .unwrap()
                    .is_none(),
                "resolution accepted {name}"
            );
            assert!(
                create_conversation(
                    &db,
                    &owner,
                    Some(&bot),
                    "whatsapp",
                    "456",
                    "private",
                    None,
                    &key.id,
                    false,
                    false
                )
                .await
                .is_err(),
                "create accepted {name}"
            );
            assert!(
                update_conversation(&db, &route.id, &owner, None, None, Some(true), None)
                    .await
                    .is_err(),
                "enable accepted {name}"
            );
            assert!(
                update_conversation(&db, &route.id, &owner, Some(&key.id), None, None, None)
                    .await
                    .is_err(),
                "replacement accepted {name}"
            );
            assert!(
                update_conversation(&db, &route.id, &owner, None, None, Some(false), None)
                    .await
                    .is_ok(),
                "disable rejected {name}"
            );
        }
    }

    #[tokio::test]
    async fn callback_url_swap_and_wrong_owner_route_are_rejected_at_dispatch() {
        let db = crate::test_utils::connect_transaction_test_database("wa_callback_swap").await;
        seed_bot(&db, "bot", "owner", "whatsapp").await;
        let key = make_api_key("agent", "owner", Some("https://agent.test/original"));
        db.collection::<ApiKey>(API_KEYS)
            .insert_one(&key)
            .await
            .unwrap();
        let route = create_conversation(
            &db,
            "owner",
            Some("bot"),
            "whatsapp",
            "123",
            "private",
            None,
            "agent",
            false,
            false,
        )
        .await
        .unwrap();
        let resolved = resolve_agent(&db, "bot", "123", None, "owner")
            .await
            .unwrap()
            .unwrap();
        db.collection::<ApiKey>(API_KEYS)
            .update_one(
                doc! {"_id":"agent"},
                doc! {"$set":{"callback_url":"https://agent.test/replaced"}},
            )
            .await
            .unwrap();
        assert!(matches!(
            load_callback_key(&db, &resolved, "owner").await,
            Err(AppError::Conflict(_))
        ));
        db.collection::<ChannelConversation>(COLLECTION_NAME)
            .update_one(
                doc! {"_id":route.id},
                doc! {"$set":{"user_id":"another-owner"}},
            )
            .await
            .unwrap();
        assert!(
            resolve_agent(&db, "bot", "123", None, "owner")
                .await
                .unwrap()
                .is_none()
        );
    }

    #[tokio::test]
    async fn broken_route_allows_permission_reductions_without_a_usable_key() {
        let db = crate::test_utils::connect_transaction_test_database("wa_route_reductions").await;
        for missing in [false, true] {
            let owner = uuid::Uuid::new_v4().to_string();
            let bot = uuid::Uuid::new_v4().to_string();
            seed_bot(&db, &bot, &owner, "whatsapp").await;
            let key = make_api_key(
                &uuid::Uuid::new_v4().to_string(),
                &owner,
                Some("https://agent.test/callback"),
            );
            db.collection::<ApiKey>(API_KEYS)
                .insert_one(&key)
                .await
                .unwrap();
            let route = create_conversation(
                &db,
                &owner,
                Some(&bot),
                "whatsapp",
                "123",
                "private",
                None,
                &key.id,
                true,
                true,
            )
            .await
            .unwrap();
            if missing {
                db.collection::<ApiKey>(API_KEYS)
                    .delete_one(doc! {"_id": &key.id})
                    .await
                    .unwrap();
            } else {
                db.collection::<ApiKey>(API_KEYS).update_one(doc! {"_id": &key.id}, doc! {"$set": {"expires_at": bson::DateTime::from_chrono(Utc::now()-chrono::Duration::seconds(1))}}).await.unwrap();
            }
            let narrowed =
                update_conversation(&db, &route.id, &owner, None, None, None, Some(false))
                    .await
                    .expect("turning off proactive sends must remain available");
            assert!(!narrowed.allow_agent_initiated);
            assert!(narrowed.is_active);
            let narrowed =
                update_conversation(&db, &route.id, &owner, None, Some(false), None, None)
                    .await
                    .expect("removing the default must remain available");
            assert!(!narrowed.default_agent);
            assert!(
                update_conversation(&db, &route.id, &owner, None, Some(true), None, None)
                    .await
                    .is_err()
            );
            assert!(
                update_conversation(&db, &route.id, &owner, None, None, None, Some(true))
                    .await
                    .is_err()
            );
            assert!(
                update_conversation(
                    &db,
                    &route.id,
                    &owner,
                    Some(&key.id),
                    None,
                    Some(false),
                    None
                )
                .await
                .is_err()
            );
            let disabled = update_conversation(
                &db,
                &route.id,
                &owner,
                None,
                Some(false),
                Some(false),
                Some(false),
            )
            .await
            .unwrap();
            assert!(!disabled.is_active);
            assert!(
                update_conversation(&db, &route.id, &owner, None, None, Some(true), None)
                    .await
                    .is_err()
            );
        }
    }
}

//! External callback admission and exact-source reply authority.

use super::{
    AnswerAudience, GuestAdmission, GuestMode, GuestTurn, VerifiedGuestInput, policy, transport,
};
use crate::config::AppConfig;
use crate::crypto::aes::{EncryptionKeys, RecordContext};
use crate::crypto::jwt::{self, JwtKeys};
use crate::errors::{AppError, AppResult};
use crate::models::{
    api_key::{ApiKey, COLLECTION_NAME as API_KEYS},
    channel_bot::ChannelBot,
    channel_conversation::{COLLECTION_NAME as ROUTES, ChannelConversation},
    channel_message::ChannelMessage,
    nyxbot_channel::COLLECTION_NAME as HOSTED,
};
use crate::services::{channel_platform::InboundMessage, channel_relay_service};
use base64::{Engine, engine::general_purpose::URL_SAFE_NO_PAD};
use bson::{Document, doc};
use mongodb::Database;

pub(crate) async fn is_hosted_route(
    db: &Database,
    bot_id: &str,
    route_id: &str,
    key_id: &str,
) -> AppResult<bool> {
    Ok(db
        .collection::<Document>(HOSTED)
        .find_one(doc! {
            "channel_bot_id": bot_id, "route_id": route_id, "status": "active",
            "route_api_key_id": key_id,
        })
        .projection(doc! {"_id": 1})
        .await?
        .is_some())
}

/// Prepare immutable provider evidence and the work to commit with the original
/// channel metadata. The stable ID recognizes old add-on work across off/on.
pub(crate) async fn prepare(
    db: &Database,
    keys: &EncryptionKeys,
    bot: &ChannelBot,
    route: &ChannelConversation,
    inbound: &InboundMessage,
    metadata: &mut ChannelMessage,
) -> AppResult<bool> {
    let mode = super::select_mode_for_bot(db, bot).await?;
    if mode == GuestMode::Legacy && !bot.diagnostics.guest_relay_work_present {
        return Ok(true);
    }
    let conversation_id = destination_for(metadata)?;
    let event_id = match channel_relay_service::native_event_id(&bot.platform, &inbound.raw_data) {
        Some(id) => format!("relay-update:{id}"),
        None => format!(
            "relay-message:{conversation_id}:{}",
            inbound.platform_message_id
        ),
    };
    let stable_source = uuid::Uuid::new_v5(
        &uuid::Uuid::NAMESPACE_OID,
        &serde_json::to_vec(&(
            "nyxid:channel-guest-relay-source:v1",
            &bot.id,
            &bot.user_id,
            &bot.platform,
            &event_id,
        ))
        .map_err(|_| refused())?,
    )
    .to_string();
    let existing = db
        .collection::<Document>(crate::services::channel_activity_service::collection(
            metadata,
        ))
        .find_one(doc! {"_id": &stable_source})
        .projection(doc! {"_id": 1})
        .await?;
    if existing.is_some() {
        return Ok(false);
    }
    if mode == GuestMode::Legacy {
        return Ok(true);
    }
    let mut facts = transport::inbound_facts(bot, inbound, metadata.thread_context.as_ref());
    let meta_grant = if crate::services::channel_adapters::meta::is_meta(&bot.platform) {
        let authority = metadata.meta_authority.as_ref().ok_or_else(refused)?;
        let grant = validate_meta_source(bot, metadata)?;
        facts.sender_id = Some(authority.sender_id.as_str());
        facts.audience = AnswerAudience::Private;
        Some(grant)
    } else {
        None
    };
    metadata.guest_transport_audience = facts.sender_id.map(|_| facts.audience);
    if is_hosted_route(db, &bot.id, &route.id, &route.agent_api_key_id).await? {
        return Ok(true);
    }
    if external_route_status(db, route).await? != ExternalRouteStatus::Ready {
        return Ok(true);
    }
    let prior = db
        .collection::<Document>(crate::services::channel_activity_service::collection(
            metadata,
        ))
        .find_one(doc! {
            "channel_bot_id": &bot.id, "user_id": &bot.user_id, "platform": &bot.platform,
            "direction": "inbound", "platform_conversation_id": &inbound.conversation_id,
            "platform_message_id": &inbound.platform_message_id,
            "native_event_id": &metadata.native_event_id,
        })
        .projection(doc! {"_id": 1})
        .await?;
    if prior.is_some() {
        return Ok(false);
    }
    let GuestAdmission::AddOn(turn) = super::classify_with_meta(
        db,
        mode,
        VerifiedGuestInput {
            bot_id: &bot.id,
            binding_id: &route.id,
            platform: &bot.platform,
            conversation_id: &conversation_id,
            source_event_id: &event_id,
            sender_id: facts.sender_id,
            audience: facts.audience,
            meta_grant_id: meta_grant.as_ref().map(|grant| grant.grant_id.as_str()),
        },
        meta_grant.as_ref(),
    )
    .await?
    else {
        return Err(refused());
    };
    metadata.id = stable_source;
    metadata.guest_relay = Some(turn);
    if let Some(thread) = metadata.thread_id.as_deref().filter(|thread| {
        metadata.platform == "discord" && thread.starts_with("interaction:")
    }) {
        metadata.thread_id = Some(seal_reply_handle(keys, metadata, thread.as_bytes()).await?);
    }
    Ok(true)
}

fn validate_meta_source(
    bot: &ChannelBot,
    source: &ChannelMessage,
) -> AppResult<crate::models::channel_meta::GrantRef> {
    use crate::services::{channel_adapters::meta, channel_meta_service::scopes};
    let authority = source.meta_authority.as_ref().ok_or_else(refused)?;
    let grant = scopes::reference(bot)?;
    let now = chrono::Utc::now();
    if authority.grant_id.as_deref() != Some(&grant.grant_id)
        || authority.generation != grant.generation
        || authority.asset_id != bot.platform_bot_id
        || source.sender_platform_id.as_deref() != Some(&authority.sender_id)
        || source.platform_conversation_id.as_deref() != Some(meta::conversation(
            &bot.platform, &bot.id, &authority.asset_id, &authority.sender_id,
        ).as_str())
        || authority.event_at > now
        || now >= authority.event_at + chrono::Duration::hours(24)
        || now >= authority.reply_expires_at
    {
        return Err(refused());
    }
    Ok(grant)
}

pub(crate) async fn prove_meta_tool_participant(
    state: &crate::AppState,
    bot: &ChannelBot,
    source: &ChannelMessage,
) -> AppResult<()> {
    use crate::services::channel_adapters::{meta, resolve_adapter};
    let inbox = match bot.platform.as_str() {
        "messenger" => meta::Inbox::Messenger,
        "instagram" => meta::Inbox::Instagram,
        _ => return Ok(()),
    };
    let authority = source.meta_authority.as_ref().ok_or_else(refused)?;
    validate_meta_source(bot, source)?;
    let adapter = resolve_adapter(&bot.platform, &state.token_exchange_cache)?;
    let token = crate::services::channel_credentials::resolve_bot_token(
        &state.db, &state.encryption_keys, adapter.as_ref(), bot,
    ).await?;
    meta::MetaAdapter::new(inbox).prove_control(
        &state.db, &state.http_client, bot, &authority.sender_id, &token,
    ).await
}

pub(crate) fn reply_token(
    keys: &JwtKeys,
    config: &AppConfig,
    source: &ChannelMessage,
    route: &ChannelConversation,
) -> AppResult<String> {
    if source.guest_relay.is_some() {
        let bounded = guest_token_config(config, source)?;
        return jwt::generate_guest_relay_reply_token(
            keys,
            &bounded,
            &route.agent_api_key_id,
            &route.id,
            &source.id,
            &route.platform,
        );
    }
    jwt::generate_relay_reply_token(
        keys,
        config,
        &route.agent_api_key_id,
        &route.id,
        &source.id,
        &route.platform,
    )
}

pub(crate) fn guest_data_token(
    keys: &JwtKeys,
    config: &AppConfig,
    source: &ChannelMessage,
    route: &ChannelConversation,
) -> AppResult<Option<String>> {
    source
        .guest_relay
        .as_ref()
        .map(|_| {
            let bounded = guest_token_config(config, source)?;
            jwt::generate_guest_relay_data_token(
                keys,
                &bounded,
                &route.agent_api_key_id,
                &route.id,
                &source.id,
                &route.platform,
            )
        })
        .transpose()
}

fn guest_token_config(config: &AppConfig, source: &ChannelMessage) -> AppResult<AppConfig> {
    let remaining = source
        .created_at
        .timestamp()
        .saturating_add(config.jwt_relay_reply_ttl_secs)
        .saturating_sub(chrono::Utc::now().timestamp());
    let remaining = source.meta_authority.as_ref().map_or(remaining, |meta| {
        let now = chrono::Utc::now();
        let expires = meta.reply_expires_at.min(meta.event_at + chrono::Duration::hours(24));
        remaining.min((expires - now).num_seconds())
    });
    if remaining <= 0 {
        return Err(AppError::TokenExpired);
    }
    let mut bounded = config.clone();
    bounded.jwt_relay_reply_ttl_secs = config.jwt_relay_reply_ttl_secs.min(remaining);
    Ok(bounded)
}

/// Fence the original route, source, and durable key in the same transaction
/// that accesses guest records. Every matching write modifies a live row.
pub(crate) async fn fence_data_in_session(
    db: &Database,
    session: &mut mongodb::ClientSession,
    source: &ChannelMessage,
    route: &ChannelConversation,
    bot: &ChannelBot,
    request_id: &str,
) -> AppResult<()> {
    let turn = source.guest_relay.as_ref().ok_or_else(refused)?;
    super::fence_in_session(db, session, turn).await?;
    if turn.meta_grant_id.is_some() {
        let grant = validate_meta_source(bot, source)?;
        if turn.meta_grant_id.as_deref() != Some(&grant.grant_id) {
            return Err(refused());
        }
        crate::services::channel_meta_service::scopes::fence_reference(
            db, session, &grant, true,
        ).await?;
        let binding = bot.meta_binding.as_ref().ok_or_else(refused)?;
        let chat = source.platform_conversation_id.as_deref().ok_or_else(refused)?;
        if db.collection::<Document>(crate::services::channel_adapters::meta::CONTROLS)
            .update_one(
                doc! {"identity_key":format!("{}:{chat}",bot.id),"controlled":true,
                    "generation": grant.generation,
                    "$and":[crate::services::channel_adapters::meta::control_scope(binding)]},
                doc! {"$inc":{"admission_revision":1_i64}},
            ).session(&mut *session).await?.matched_count != 1
        {
            return Err(refused());
        }
    }
    let marker = doc! {"$inc":{"guest_data_fence_revision":1_i64}};
    let bot_fence = db.collection::<Document>(crate::models::channel_bot::COLLECTION_NAME)
        .update_one(doc! {"_id":&bot.id, "user_id":&bot.user_id, "is_active":true,
            "platform_bot_id": &bot.platform_bot_id,
            "$expr": {"$eq": [{"$ifNull": ["$ownership_version", 0_i64]}, bot.ownership_version]},
        }, marker.clone()).session(&mut *session).await?;
    let route_fence = db.collection::<Document>(ROUTES).update_one(
        doc! {"_id":&route.id, "channel_bot_id":&bot.id, "user_id":&bot.user_id,
            "is_active":true, "retired_by_transfer":{"$ne":true}, "agent_api_key_id":&route.agent_api_key_id},
        marker.clone(),
    ).session(&mut *session).await?;
    let key_fence = db
        .collection::<Document>(API_KEYS)
        .update_one(
            doc! {"_id":&route.agent_api_key_id, "user_id":&bot.user_id,
            "is_active":true, "guest_relay_bot_id":&bot.id,
            "guest_relay_dedicated_route_id": &route.id,
            "guest_relay_activation_epoch": turn.activation_epoch,
            "guest_relay_bot_revision": turn.bot_revision,
            "$or":[{"expires_at":null}, {"expires_at":{"$gt":bson::DateTime::now()}}]},
            marker.clone(),
        )
        .session(&mut *session)
        .await?;
    let source_fence = db
        .collection::<Document>(crate::models::channel_message::COLLECTION_NAME)
        .update_one(
            doc! {"_id":&source.id, "direction":"inbound", "channel_bot_id":&bot.id,
                "conversation_id":&route.id, "user_id":&bot.user_id,
                "agent_api_key_id":&route.agent_api_key_id, "platform":&source.platform,
                "platform_conversation_id":&source.platform_conversation_id,
                "sender_platform_id":&source.sender_platform_id, "thread_id":&source.thread_id,
                "guest_relay":bson::to_bson(turn).map_err(|_|refused())?,
                "guest_data_calls": {"$ne": request_id},
                "$expr": {"$lt": [{"$size": {"$ifNull": ["$guest_data_calls", []]}}, 128]},
            },
            doc! {"$addToSet":{"guest_data_calls":request_id}},
        )
        .session(&mut *session)
        .await?;
    if [bot_fence, route_fence, key_fence, source_fence]
        .into_iter()
        .any(|outcome| outcome.matched_count != 1)
    {
        return Err(refused());
    }
    Ok(())
}

fn refused() -> AppError {
    AppError::Forbidden("Channel guest reply authority is unavailable".into())
}

/// Persist the one bot for which a retained relay key can act during
/// enrollment. Key erasure removes the binding along with the credential.
pub(crate) async fn bind_in_session(
    db: &Database,
    session: &mut mongodb::ClientSession,
    source: &ChannelMessage,
) -> AppResult<()> {
    let Some(turn) = source.guest_relay.as_ref() else {
        return Ok(());
    };
    let key = source.agent_api_key_id.as_deref().ok_or_else(refused)?;
    let route = db.collection::<Document>(ROUTES).update_one(
        doc! {"_id": &turn.binding_id, "channel_bot_id": &turn.bot_id, "user_id":&source.user_id,
            "is_active": true, "retired_by_transfer": {"$ne":true}, "agent_api_key_id": key},
        doc! {"$set":{"guest_relay_activated": true}},
    ).session(&mut *session).await?;
    let retained = db.collection::<Document>(API_KEYS).update_one(
        doc! {
            "_id": key, "user_id": &source.user_id, "is_active": true,
            "guest_relay_dedicated_route_id": &turn.binding_id,
            "guest_relay_dedicated_bot_id": &turn.bot_id,
            "callback_url": {"$type":"string","$ne":""},
            "$or":[{"expires_at":null},{"expires_at":{"$gt":bson::DateTime::now()}}],
        },
        doc! {"$set":{"guest_relay_bot_id": &turn.bot_id,
            "guest_relay_activation_epoch": turn.activation_epoch, "guest_relay_bot_revision":turn.bot_revision},
            "$inc": {"guest_relay_binding_fence": 1_i64}},
    ).session(&mut *session).await?;
    let bot = db.collection::<Document>(crate::models::channel_bot::COLLECTION_NAME).update_one(
        doc! {"_id":&turn.bot_id,"user_id":&source.user_id,"is_active":true},
        doc! {"$set":{"channel_guest_data_present":true,"diagnostics.guest_relay_work_present":true}},
    ).session(&mut *session).await?;
    if [route, retained, bot].iter().any(|row| row.matched_count != 1) {
        return Err(refused());
    }
    super::bind_work_in_session(db, session, turn).await
}

#[derive(Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum ExternalRouteStatus {
    Ready,
    DedicatedKeyRequired,
    LegacyCallbackPending,
}

pub(crate) async fn external_route_status(
    db: &Database,
    route: &ChannelConversation,
) -> AppResult<ExternalRouteStatus> {
    let bot_id = route.channel_bot_id.as_deref().ok_or_else(refused)?;
    let key = db.collection::<RetainedRelayKey>(API_KEYS).find_one(doc! {
        "_id": &route.agent_api_key_id, "guest_relay_dedicated_route_id": &route.id,
        "guest_relay_dedicated_bot_id": bot_id,
    }).await?;
    let route_state = db.collection::<Document>(ROUTES).find_one(doc! {"_id": &route.id})
        .projection(doc! {"guest_relay_activated": 1}).await?;
    let activated = route_state.as_ref().and_then(|row| row.get_bool("guest_relay_activated").ok())
        == Some(true);
    if !key.is_some_and(|key| compatible_key(&key.key, &route.user_id)) {
        return if activated { Err(refused()) } else { Ok(ExternalRouteStatus::DedicatedKeyRequired) };
    }
    if route.platform == "aurinko"
        && has_pending_legacy_callback(db, bot_id, &route.user_id).await?
    {
        return Ok(ExternalRouteStatus::LegacyCallbackPending);
    }
    Ok(ExternalRouteStatus::Ready)
}

async fn has_pending_legacy_callback(db: &Database, bot_id: &str, owner: &str) -> AppResult<bool> {
    Ok(db.collection::<Document>(crate::models::channel_message::COLLECTION_NAME)
        .find_one(doc! {"channel_bot_id":bot_id,"user_id":owner,"direction":"inbound",
            "guest_relay":null,"callback_status":{"$in":["pending","failed"]},
            "callback_failure_reason":{"$ne":crate::services::webhook_delivery_service::DESTINATION_REFUSED},
        }).projection(doc! {"_id":1}).await?.is_some())
}

fn compatible_key(key: &ApiKey, owner: &str) -> bool {
    key.purpose == crate::models::api_key::ApiKeyPurpose::General
        && key.permission_policy_id.is_none()
        && key.assistant_group_id.is_none()
        && key.assistant_agent_owner_id.is_none()
        && crate::services::channel_routing_service::ensure_route_agent(key, owner).is_ok()
}

pub(crate) async fn dedicate_key_in_session(
    db: &Database,
    session: &mut mongodb::ClientSession,
    route: &ChannelConversation,
    key_id: &str,
) -> AppResult<()> {
    let bot_id = route.channel_bot_id.as_deref().ok_or_else(refused)?;
    let key = db.collection::<ApiKey>(API_KEYS).find_one(doc! {"_id":key_id})
        .session(&mut *session).await?.filter(|key| compatible_key(key, &route.user_id))
        .ok_or_else(|| AppError::Conflict("Choose a compatible ordinary callback key for this route".into()))?;
    let shared = db.collection::<Document>(ROUTES).find_one(doc! {
        "agent_api_key_id":key_id,"is_active":true,"_id":{"$ne":&route.id},
    }).projection(doc! {"_id":1}).session(&mut *session).await?.is_some();
    let hosted = db.collection::<Document>(HOSTED).find_one(doc! {"$or":[
        {"route_api_key_id":key_id}, {"pending_route_api_key_id":key_id}, {"agent_api_key_id":key_id},
    ]}).projection(doc! {"_id":1}).session(&mut *session).await?.is_some();
    if shared || hosted {
        return Err(AppError::Conflict("Choose an agent key that is unused by other routes".into()));
    }
    let result = db.collection::<Document>(API_KEYS).update_one(
        doc! {"_id":&key.id,"user_id":&route.user_id,"is_active":true,
            "$and":[
                {"$or":[{"guest_relay_dedicated_route_id":null},{"guest_relay_dedicated_route_id":&route.id}]},
                {"$or":[{"guest_relay_dedicated_bot_id":null},{"guest_relay_dedicated_bot_id":bot_id}]},
            ],
        },
        doc! {"$set":{"guest_relay_dedicated_route_id":&route.id,"guest_relay_dedicated_bot_id":bot_id}},
    ).session(&mut *session).await?;
    if result.matched_count != 1 {
        return Err(AppError::Conflict("Choose an agent key dedicated to this route".into()));
    }
    Ok(())
}

/// The restriction follows the durable credential even if the user clears
/// its callback URL or the original route goes away.
#[derive(serde::Deserialize)]
pub(crate) struct RetainedRelayKey {
    #[serde(flatten)]
    pub key: ApiKey,
    guest_relay_bot_id: Option<String>,
    guest_relay_dedicated_bot_id: Option<String>,
    guest_relay_dedicated_route_id: Option<String>,
    guest_relay_activation_epoch: Option<i64>,
    guest_relay_bot_revision: Option<i64>,
}

impl RetainedRelayKey {
    pub(crate) async fn require_broad_access(&self, db: &Database) -> AppResult<()> {
        self.broad_route(db).await.map(drop)
    }

    async fn broad_route(&self, db: &Database) -> AppResult<Option<(ChannelBot, ChannelConversation)>> {
        let current = self.live_route(db).await?;
        if let Some((bot, route)) = &current {
            if super::select_mode_for_bot(db, bot).await? == GuestMode::Legacy
                || (route.platform == "aurinko"
                    && has_pending_legacy_callback(db, &bot.id, &route.user_id).await?)
                || (self.guest_relay_bot_id.is_none() && !compatible_key(&self.key, &route.user_id))
            {
                return Ok(current);
            }
        } else if self.guest_relay_bot_id.is_none() {
            return Ok(None);
        }
        Err(refused())
    }

    async fn live_route(&self, db: &Database) -> AppResult<Option<(ChannelBot, ChannelConversation)>> {
        let Some((bot_id, route_id)) = self.guest_relay_dedicated_bot_id.as_deref()
            .zip(self.guest_relay_dedicated_route_id.as_deref()) else {
            return Ok(None);
        };
        let bot = db.collection::<ChannelBot>(crate::models::channel_bot::COLLECTION_NAME)
            .find_one(doc! {"_id":bot_id,"user_id":&self.key.user_id,"is_active":true,
                "status":{"$nin":["suspended","transferring"]},
            }).await?;
        let Some(bot) = bot else { return Ok(None); };
        let route = db.collection::<ChannelConversation>(ROUTES).find_one(doc! {
            "_id": route_id, "channel_bot_id": bot_id, "user_id": &self.key.user_id,
            "agent_api_key_id": &self.key.id, "is_active":true, "retired_by_transfer":{"$ne":true},
        }).await?;
        Ok(route.map(|route| (bot, route)))
    }

    async fn broad_assignment(&self, db: &Database) -> AppResult<Option<String>> {
        let current = self.broad_route(db).await?;
        if self.guest_relay_bot_id.is_none() {
            return Ok(None);
        }
        let (bot, route) = current.ok_or_else(refused)?;
        let enrolled_epoch = self.guest_relay_activation_epoch.ok_or_else(refused)?;
        let enrolled_revision = self.guest_relay_bot_revision.ok_or_else(refused)?;
        let rollout_epoch = super::activation_collection(db)
            .find_one(doc! {"_id":super::FLAG}).await?.ok_or_else(refused)?.epoch;
        Ok(Some(format!(
            "{}.{enrolled_epoch}.{enrolled_revision}.{rollout_epoch}.{}.{}",
            route.id, bot.guest_access_v1_revision, route.updated_at.timestamp_millis(),
        )))
    }

    pub(crate) async fn require_relay_bearer(
        &self,
        db: &Database,
        claim: Option<&str>,
    ) -> AppResult<()> {
        if self.broad_assignment(db).await?.as_deref() != claim {
            return Err(refused());
        }
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) async fn mint_ordinary_bearer(
        &self,
        db: &Database,
        keys: &JwtKeys,
        config: &AppConfig,
        owner: &uuid::Uuid,
        route: &ChannelConversation,
        scope: &str,
        rbac: Option<&jwt::RbacClaimData>,
        agent: &jwt::RelayAgentScope,
    ) -> AppResult<String> {
        let assignment = self.broad_assignment(db).await?;
        if self.key.user_id != route.user_id || agent.api_key_id != self.key.id
            || (assignment.is_some() && self.guest_relay_dedicated_route_id.as_deref() != Some(&route.id))
        {
            return Err(refused());
        }
        jwt::generate_relay_access_token_with_assignment(
            keys, config, owner, scope, rbac, agent, assignment,
        )
    }
}

pub(crate) async fn release_unused_key_in_session(
    db: &Database,
    session: &mut mongodb::ClientSession,
    route: &ChannelConversation,
) -> AppResult<()> {
    let keys = db.collection::<Document>(API_KEYS);
    let filter = doc! {"_id":&route.agent_api_key_id,"user_id":&route.user_id,
        "guest_relay_dedicated_route_id":&route.id};
    if keys.find_one(filter.clone()).session(&mut *session).await?
        .is_some_and(|key| key.get_str("guest_relay_bot_id").is_ok())
    {
        return Err(AppError::Conflict("Rotate the previously enrolled key to reuse it on another route".into()));
    }
    keys.update_one(filter, doc! {"$unset":{
        "guest_relay_dedicated_route_id":"","guest_relay_dedicated_bot_id":"",
    }}).session(&mut *session).await?;
    Ok(())
}

pub(crate) async fn require_assignment_in_session(
    db: &Database,
    session: &mut mongodb::ClientSession,
    key: &str,
    requested_route: Option<&str>,
) -> AppResult<()> {
    let retained = db.collection::<Document>(API_KEYS).find_one(doc! {"_id":key})
        .projection(doc! {"guest_relay_bot_id":1,"guest_relay_dedicated_route_id":1})
        .session(&mut *session).await?;
    if let Some(retained) = retained {
        if let Ok(route) = retained.get_str("guest_relay_dedicated_route_id") {
            if Some(route) != requested_route {
                return Err(AppError::Conflict("This key is dedicated to another channel route".into()));
            }
        } else if retained.get_str("guest_relay_bot_id").is_ok() {
            return Err(AppError::Conflict("This key is reserved for guest relay".into()));
        }
    }
    Ok(())
}

pub(crate) const REPLY_HANDLE_PREFIX: &str = "guest-interaction.v3:";

fn reply_handle_fields(source: &ChannelMessage) -> AppResult<[&str; 7]> {
    let turn = source.guest_relay.as_ref().ok_or_else(refused)?;
    let bot = source.channel_bot_id.as_deref().ok_or_else(refused)?;
    let event = source.platform_message_id.as_deref().ok_or_else(refused)?;
    if source.direction != "inbound" || source.platform != "discord"
        || turn.bot_id != bot || turn.binding_id != source.conversation_id
        || turn.platform != source.platform
        || turn.conversation_id != destination_for(source)?
    {
        return Err(refused());
    }
    Ok(["thread_id", "guest-relay-discord", bot, &source.platform,
        &turn.binding_id, event, &turn.conversation_id])
}

fn reply_handle_context<'a>(
    source: &'a ChannelMessage,
    fields: &'a [&'a str],
) -> AppResult<RecordContext<'a>> {
    RecordContext::new(
        crate::models::channel_message::COLLECTION_NAME, &source.id, &source.user_id, fields,
    )
}

async fn seal_reply_handle(
    keys: &EncryptionKeys,
    source: &ChannelMessage,
    plaintext: &[u8],
) -> AppResult<String> {
    if plaintext.len() > 2048 || !plaintext.starts_with(b"interaction:") {
        return Err(refused());
    }
    let fields = reply_handle_fields(source)?;
    let ciphertext = keys.encrypt_record(plaintext, &reply_handle_context(source, &fields)?).await?;
    Ok(format!("{REPLY_HANDLE_PREFIX}{}", URL_SAFE_NO_PAD.encode(ciphertext)))
}

fn decode_reply_handle(source: &ChannelMessage) -> AppResult<Vec<u8>> {
    let encoded = source.thread_id.as_deref()
        .and_then(|thread| thread.strip_prefix(REPLY_HANDLE_PREFIX))
        .filter(|encoded| encoded.len() <= 8192).ok_or_else(refused)?;
    URL_SAFE_NO_PAD.decode(encoded).map_err(|_| refused())
}

pub(crate) async fn open_reply_handle(
    keys: &EncryptionKeys,
    source: &ChannelMessage,
) -> AppResult<Option<zeroize::Zeroizing<String>>> {
    let Some(thread) = source.thread_id.as_deref().filter(|_| source.guest_relay.is_some()) else {
        return Ok(None);
    };
    if !thread.starts_with(REPLY_HANDLE_PREFIX) {
        if source.platform == "discord" && thread.starts_with("interaction:") {
            return Err(refused());
        }
        return Ok(None);
    }
    if chrono::Utc::now() >= source.created_at + chrono::Duration::minutes(14) {
        return Err(AppError::TokenExpired);
    }
    let fields = reply_handle_fields(source)?;
    let bytes = keys.decrypt_record(&decode_reply_handle(source)?, &reply_handle_context(source, &fields)?)
        .await?;
    let opened = std::str::from_utf8(&bytes).map_err(|_| refused())?;
    if !opened.starts_with("interaction:") || opened.len() > 2048 {
        return Err(refused());
    }
    Ok(Some(zeroize::Zeroizing::new(opened.to_owned())))
}

pub(crate) async fn verify_reply_handle_record(keys: &EncryptionKeys, row: &Document) -> AppResult<()> {
    let source: ChannelMessage = bson::from_document(row.clone()).map_err(|_| refused())?;
    let fields = reply_handle_fields(&source)?;
    keys.authenticate_current_record(&decode_reply_handle(&source)?, &reply_handle_context(&source, &fields)?)
        .await
}

pub(crate) async fn migrate_reply_handle_record(
    db: &Database,
    keys: &EncryptionKeys,
    row: &Document,
) -> AppResult<bool> {
    let source: ChannelMessage = bson::from_document(row.clone()).map_err(|_| refused())?;
    let ciphertext = decode_reply_handle(&source)?;
    let fields = reply_handle_fields(&source)?;
    let context = reply_handle_context(&source, &fields)?;
    if keys.authenticate_current_record(&ciphertext, &context).await.is_ok() {
        return Ok(false);
    }
    let bytes = keys.decrypt_record(&ciphertext, &context).await?;
    let sealed = seal_reply_handle(keys, &source, &bytes).await?;
    let applied = db.collection::<Document>(crate::models::channel_message::COLLECTION_NAME)
        .update_one(
            doc! {"_id": &source.id, "user_id":&source.user_id,
                "channel_bot_id": &source.channel_bot_id, "conversation_id": &source.conversation_id,
                "platform_conversation_id": &source.platform_conversation_id,
                "platform_message_id": &source.platform_message_id,
                "thread_id": &source.thread_id},
            doc! {"$set":{"thread_id":sealed}},
        ).await?;
    Ok(applied.modified_count == 1)
}

fn destination_for(source: &ChannelMessage) -> AppResult<String> {
    let chat = source
        .platform_conversation_id
        .as_deref()
        .filter(|id| !id.is_empty() && *id != "*")
        .ok_or_else(refused)?;
    let mut thread = source.thread_id.as_deref();
    let interaction;
    if source.platform == "discord" && thread.is_some_and(|thread| {
        thread.starts_with(REPLY_HANDLE_PREFIX) || thread.starts_with("interaction:")
    }) {
        let event = source.platform_message_id.as_deref().filter(|id| !id.is_empty())
            .ok_or_else(refused)?;
        interaction = format!("discord-interaction:{event}");
        thread = Some(&interaction);
    }
    Ok(transport::destination(chat, thread))
}

pub(crate) async fn revalidate_delivery(
    db: &Database,
    source: &ChannelMessage,
    route: &ChannelConversation,
    bot: &ChannelBot,
    key_id: &str,
    guest_token: Option<bool>,
) -> AppResult<()> {
    if let Some(guest_token) = guest_token
        && guest_token != source.guest_relay.is_some()
    {
        return Err(refused());
    }
    // Legacy JWTs prove their original source without newly consulting rollout
    // state. The existing route, owner and adapter checks govern the delivery.
    let Some(turn) = source.guest_relay.as_ref() else {
        return Ok(());
    };

    if source.direction != "inbound"
        || source.channel_bot_id.as_deref() != Some(&bot.id)
        || source.platform != bot.platform
        || source.user_id != bot.user_id
        || source.conversation_id != route.id
        || source.agent_api_key_id.as_deref() != Some(key_id)
        || route.agent_api_key_id != key_id
        || route.channel_bot_id.as_deref() != Some(&bot.id)
        || route.user_id != bot.user_id
        || route.platform != bot.platform
    {
        return Err(refused());
    }
    let chat = source
        .platform_conversation_id
        .as_deref()
        .ok_or_else(refused)?;
    let active = crate::services::channel_routing_service::resolve_agent(
        db,
        &bot.id,
        chat,
        source.sender_platform_id.as_deref(),
        &bot.user_id,
    )
    .await?
    .is_some_and(|current| current.conversation.id == route.id && current.api_key_id == key_id);
    if !active || external_route_status(db, route).await? != ExternalRouteStatus::Ready {
        return Err(refused());
    }
    if crate::services::channel_adapters::meta::is_meta(&bot.platform) {
        let grant = validate_meta_source(bot, source)?;
        if turn.meta_grant_id.as_deref() != Some(&grant.grant_id) {
            return Err(refused());
        }
        crate::services::channel_adapters::meta::require_ingress(db, bot, chat).await?;
        crate::services::channel_meta_service::scopes::require_current_operation(db, bot).await?;
    } else if turn.meta_grant_id.is_some() {
        return Err(refused());
    }
    check_turn(db, turn, source, &bot.id, &route.id).await
}

pub(crate) async fn revalidate_effect(
    db: &Database,
    source: &ChannelMessage,
    route: &ChannelConversation,
    bot: &ChannelBot,
    key_id: &str,
) -> AppResult<()> {
    if source.guest_relay.is_some() {
        current_key(db, key_id, &bot.user_id).await?;
        revalidate_delivery(db, source, route, bot, key_id, None).await?;
    }
    Ok(())
}

pub(crate) async fn revalidate_edit(
    db: &Database,
    outbound: &ChannelMessage,
    route: &ChannelConversation,
    bot: &ChannelBot,
    key_id: &str,
    chat_id: &str,
    has_metadata: bool,
) -> AppResult<()> {
    let Some(turn) = &outbound.guest_relay else {
        return revalidate_delivery(db, outbound, route, bot, key_id, None).await;
    };
    if has_metadata || outbound.platform_conversation_id.as_deref() != Some(chat_id) {
        return Err(refused());
    }
    let inbound_id = outbound
        .reply_to_message_id
        .as_deref()
        .ok_or_else(refused)?;
    let source = channel_relay_service::get_message(db, inbound_id).await?;
    if source.guest_relay.as_ref() != Some(turn)
        || source.platform_conversation_id.as_deref() != Some(chat_id)
        || outbound.conversation_id != route.id
        || outbound.agent_api_key_id.as_deref() != Some(key_id)
    {
        return Err(refused());
    }
    revalidate_effect(db, &source, route, bot, key_id).await
}

async fn check_turn(
    db: &Database,
    turn: &GuestTurn,
    source: &ChannelMessage,
    bot_id: &str,
    route_id: &str,
) -> AppResult<()> {
    if super::admitted_turn(
        db,
        bot_id,
        route_id,
        &source.platform,
        &turn.source_event_id,
    )
    .await?
    .as_ref()
        != Some(turn)
    {
        return Err(refused());
    }
    policy::revalidate_delivery(
        db,
        turn,
        bot_id,
        route_id,
        &source.platform,
        &destination_for(source)?,
        source
            .guest_transport_audience
            .unwrap_or(AnswerAudience::Shared),
    )
    .await
}

pub(crate) async fn revalidate_retry(
    db: &Database,
    bot: &ChannelBot,
    route: &ChannelConversation,
    key_id: &str,
    inbound: &InboundMessage,
    source: &ChannelMessage,
) -> AppResult<()> {
    if source.guest_relay.is_some() {
        if source.sender_platform_id.as_deref() != Some(&inbound.sender_platform_id)
            || source.platform_message_id.as_deref() != Some(&inbound.platform_message_id)
        {
            return Err(refused());
        }
        return revalidate_delivery(db, source, route, bot, key_id, Some(true)).await;
    }
    // The original Aurinko receipt is allowed to finish in its original lane.
    // Until it settles, external_route_status keeps new events ordinary.

    Ok(())
}

pub(crate) async fn current_key(db: &Database, id: &str, owner: &str) -> AppResult<()> {
    if db
        .collection::<ApiKey>(API_KEYS)
        .find_one(doc! {"_id": id, "user_id": owner, "is_active": true})
        .await?
        .is_some_and(|key| {
            key.expires_at
                .is_none_or(|expiry| expiry > chrono::Utc::now())
        })
    {
        return Ok(());
    }
    Err(refused())
}

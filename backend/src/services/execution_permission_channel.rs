//! Verified owner channel sources. No provider capabilities or result text are copied here.
use bson::{Document, doc};
use chrono::Utc;
use mongodb::{ClientSession, Database};
use serde_json::json;

use crate::{
    errors::{AppError, AppResult},
    models::{
        assistant_conversation::AssistantConversation,
        execution_permission::{Audience, TaskAdmission},
        execution_permission_channel::ChannelSource,
        nyxbot_channel::{NyxbotChannel, NyxbotEvent, NyxbotThread},
    },
};

pub(crate) mod deliveries;
pub(crate) const READINESS: &str = "permissions:execution-authoring-channel-v1";

/// Inputs come only from the channel provider's verified sender/admission path.
/// This proof is not a substitute for live source authorization.
pub(crate) fn verified_owner(
    channel: &NyxbotChannel,
    kind: crate::models::channel_thread::ThreadSenderKind,
    owner_eligible: bool,
    original_message: bool,
    guest: bool,
) -> bool {
    !guest
        && owner_eligible
        && original_message
        && kind == crate::models::channel_thread::ThreadSenderKind::Human
        && channel.bot_owner_id.is_none()
        && channel.org_specialist.is_none()
}

pub(crate) fn refused() -> AppError {
    AppError::Forbidden(
        "Original channel permission source is no longer available; review the result in NyxID"
            .into(),
    )
}
fn encoding(_: impl std::fmt::Display) -> AppError {
    AppError::Internal("Invalid channel permission metadata".into())
}
fn fingerprint(domain: &[u8], value: &impl serde::Serialize) -> AppResult<String> {
    super::audit_service::keyed_fingerprint(
        super::audit_service::audit_chain_hmac_key(),
        domain,
        &serde_json::to_vec(value).map_err(encoding)?,
    )
    .ok_or_else(|| AppError::Internal("Audit chain unavailable".into()))
}

pub(crate) struct SourceRows {
    pub event: NyxbotEvent,
    pub channel: NyxbotChannel,
    pub thread: NyxbotThread,
    pub settings: NyxbotThread,
}

async fn load<T: serde::de::DeserializeOwned>(
    db: &Database,
    tx: &mut ClientSession,
    collection: &str,
    filter: Document,
    fence: bool,
) -> AppResult<T> {
    let rows = db.collection::<Document>(collection);
    let row = if fence {
        rows.find_one_and_update(filter, doc! {"$inc":{"execution_source_fence":1_i64}})
            .session(tx)
            .await?
    } else {
        rows.find_one(filter).session(tx).await?
    }
    .ok_or_else(refused)?;
    bson::from_document(row).map_err(encoding)
}

async fn rows(
    db: &Database,
    tx: &mut ClientSession,
    owner: &str,
    conversation: &str,
    event: &str,
    fence: bool,
) -> AppResult<SourceRows> {
    let event: NyxbotEvent = load(
        db,
        tx,
        "nyxbot_events",
        doc! {
            "_id":event,"user_id":owner,"conversation_id":conversation,
            "custom_binding_id":null,"delivery.version":1,"delivery.execution_owner_verified":true,
            "delivery.guest":false,"delivery.sender_kind":"human","media.interrupted":{"$ne":true},
            "expires_at":{"$gt":bson::DateTime::now()}
        },
        fence,
    )
    .await?;
    let d = event.delivery.as_ref().ok_or_else(refused)?;
    if d.origin.org_execution.is_some() {
        return Err(refused());
    }
    let channel: NyxbotChannel = load(db,tx,"nyxbot_channels",doc! {
        "_id":&event.channel_id,"user_id":owner,"bot_owner_id":null,"org_specialist":null,"status":"active"
    },fence).await?;
    let thread: NyxbotThread = load(db,tx,"nyxbot_threads",doc! {
        "channel_id":&channel.id,"user_id":owner,"partition":&d.origin.partition,"conversation_id":conversation
    },fence).await?;
    if channel.gateway_source.is_some()
        && d.origin.native_event_id.as_deref() != Some(event.id.as_str())
    {
        return Err(refused());
    }
    let settings = if let Some(id) = &thread.follow.settings_chat_id {
        load(
            db,
            tx,
            "nyxbot_threads",
            doc! {"_id":id,"channel_id":&channel.id,"user_id":owner},
            fence,
        )
        .await?
    } else {
        thread.clone()
    };
    if d.origin.nyxbot_channel_id != channel.id
        || d.origin.platform != channel.platform
        || (super::channel_thread_follow_service::is_child(&thread) && d.origin.thread.is_none())
        || channel.transport != d.transport
        || channel.route_id != d.source_route_id
        || Some(channel.route_api_key_id.as_str()) != d.source_route_api_key_id.as_deref()
        || !matches!(
            super::channel_turn_delivery::admission(
                &channel,
                &settings,
                &d.sender_id,
                Some(d.addressed)
            ),
            super::channel_turn_delivery::Admission::Owner
        )
        || (channel.gateway_source.is_some() && !super::nyxbot_gateway_source::ready(&channel))
    {
        return Err(refused());
    }
    if channel.transport == "gateway" {
        if event.event_context_ciphertext.is_none() && d.target_ciphertext.is_none() {
            return Err(refused());
        }
    } else {
        let _: Document = load(
            db,
            tx,
            "channel_messages",
            doc! {"_id":&event.event_id,
                "direction":"inbound","user_id":owner,"channel_bot_id":&channel.channel_bot_id,
                "agent_api_key_id":&channel.route_api_key_id,"sender_platform_id":&d.sender_id
            },
            fence,
        )
        .await?;
    }
    if let Some(binding) = d.origin.thread.as_deref() {
        if fence {
            super::channel_thread_follow_service::validate_in_session(
                db,
                owner,
                &d.origin,
                conversation,
                true,
                tx,
            )
            .await?;
        }
        if binding.guest
            || binding.child_id != thread.id
            || binding.sender_id != d.sender_id
            || thread.follow.binding_generation != binding.generation
            || settings.follow.binding_generation != binding.generation
            || thread.follow.binding_channel_generation != binding.channel_generation
            || thread.follow.bound_agent_id.as_deref() != Some(binding.agent_id.as_str())
        {
            return Err(refused());
        }
    }
    Ok(SourceRows {
        event,
        channel,
        thread,
        settings,
    })
}

/// Exact source projection excludes delivery state, newest-message pointers,
/// titles and traffic timestamps, so later activity cannot retarget this proof.
fn policy_digest(r: &SourceRows) -> AppResult<String> {
    let c = &r.channel;
    let t = &r.thread;
    let s = &r.settings;
    let d = r.event.delivery.as_ref().ok_or_else(refused)?;
    fingerprint(
        b"execution-channel-source-v4",
        &json!({
            "event":r.event.id,"provider_event":r.event.event_id,"relay":r.event.relay_message_id,
            "created":r.event.created_at,"turn":r.event.turn_id,"origin":d.origin,
            "sender":d.sender_id,"kind":d.sender_kind,"verified":d.execution_owner_verified,"addressed":d.addressed,
            "channel":c.id,"owner":c.user_id,"bot":c.channel_bot_id,"platform":c.platform,
            "transport":c.transport,"route":c.route_id,"route_key":c.route_api_key_id,
            "gateway_key":c.agent_api_key_id,"binding":c.binding_id,"gateway":c.gateway_channel_id,
            "source":c.gateway_source,"native_phase":c.gateway_native_phase,
            "owners":c.owner_sender_ids,"agent":c.agent_id,"private":c.private_chats,"posts":c.private_chat_posts,
            "generation":c.execution_policy_revision,"follow_generation":c.follow_binding_generation,
            "thread":t.id,"partition":t.partition,"chat":t.platform_chat_id,"topic":t.platform_thread_id,
            "kind":t.kind,"owner_chat":t.owner_chat,"settings_kind":s.kind,
            "relay_partition":t.relay_partition,"conversation":t.conversation_id,
            "child_generation":t.follow.binding_generation,"follow_revision":t.follow.follow_revision,
            "settings":s.id,"settings_generation":s.execution_policy_revision,
            "settings_binding":s.follow.binding_generation,"threads":s.follow.threads,
            "reply":s.reply_mode,"members":s.members,"posts_allowed":s.allow_posts,"target_agent":s.agent_id
        }),
    )
}

pub(crate) fn audience(source: &ChannelSource) -> Audience {
    Audience::Channel {
        binding_id: source.audience_id.clone(),
        // Policy counters start at zero for legacy rows; permission contract
        // revisions start at one. Overflow is refused by source validation.
        revision: source.thread_revision.checked_add(1).unwrap_or(0),
    }
}

pub(crate) async fn admit(
    db: &Database,
    tx: &mut ClientSession,
    row: &AssistantConversation,
    event: &str,
) -> AppResult<ChannelSource> {
    let r = rows(db, tx, &row.user_id, &row.id, event, true).await?;
    let agent = row.agent_id.as_deref().ok_or_else(refused)?;
    let expires = r
        .event
        .expires_at
        .min(r.event.created_at + chrono::Duration::hours(24));
    let expires = if r.channel.transport == "gateway"
        && (r.channel.gateway_source.is_some()
            || r.event
                .delivery
                .as_ref()
                .is_none_or(|d| d.target_expires_at.is_none()))
    {
        expires.min(r.event.created_at + chrono::Duration::minutes(29))
    } else {
        r.event
            .delivery
            .as_ref()
            .and_then(|d| d.target_expires_at)
            .map_or(expires, |target| expires.min(target))
    };
    let source = ChannelSource {
        version: 4,
        event_id: event.into(),
        actor_user_id: row.user_id.clone(),
        agent_id: agent.into(),
        conversation_id: row.id.clone(),
        key_id: row.credential_api_key_id.clone(),
        original_turn_id: r.event.turn_id.clone().ok_or_else(refused)?,
        channel_id: r.channel.id.clone(),
        thread_id: r.thread.id.clone(),
        settings_thread_id: r.settings.id.clone(),
        channel_revision: r.channel.execution_policy_revision,
        thread_revision: r.settings.execution_policy_revision,
        audience_id: fingerprint(
            b"execution-channel-audience-v4",
            &(
                r.channel.id.as_str(),
                r.channel.execution_policy_revision,
                r.settings.id.as_str(),
            ),
        )?,
        audience_label: format!(
            "{} · {} · {}",
            r.channel.platform,
            r.channel.bot_label,
            r.thread
                .title
                .as_deref()
                .or(r.settings.title.as_deref())
                .unwrap_or("original chat/thread")
        )
        .chars()
        .filter(|c| !c.is_control())
        .take(240)
        .collect(),
        policy_digest: policy_digest(&r)?,
        expires_at: expires,
    };
    validate_inner(db, tx, &source, true, false).await?;
    db.collection::<Document>("nyxbot_events")
        .update_one(
            doc! {"_id":event},
            doc! {"$set":{"delivery.execution_source_bound":true}},
        )
        .session(tx)
        .await?;
    Ok(source)
}

pub(crate) async fn validate(
    db: &Database,
    tx: &mut ClientSession,
    s: &ChannelSource,
    fence: bool,
) -> AppResult<SourceRows> {
    validate_inner(db, tx, s, fence, true).await
}

async fn validate_inner(
    db: &Database,
    tx: &mut ClientSession,
    s: &ChannelSource,
    fence: bool,
    conversation_exists: bool,
) -> AppResult<SourceRows> {
    if s.version != 4
        || s.expires_at <= Utc::now()
        || s.channel_revision < 0
        || !(0..i64::MAX).contains(&s.thread_revision)
    {
        return Err(refused());
    }
    super::security_readiness_service::require(db, READINESS).await?;
    let r = rows(
        db,
        tx,
        &s.actor_user_id,
        &s.conversation_id,
        &s.event_id,
        fence,
    )
    .await?;
    if (conversation_exists
        && r.event
            .delivery
            .as_ref()
            .is_none_or(|d| !d.execution_source_bound))
        || r.channel.id != s.channel_id
        || r.thread.id != s.thread_id
        || r.settings.id != s.settings_thread_id
        || r.channel.execution_policy_revision != s.channel_revision
        || r.settings.execution_policy_revision != s.thread_revision
        || r.event.turn_id.as_deref() != Some(s.original_turn_id.as_str())
        || policy_digest(&r)? != s.policy_digest
        || r.settings
            .agent_id
            .as_ref()
            .or(r.channel.agent_id.as_ref())
            .is_some_and(|id| id != &s.agent_id)
    {
        return Err(refused());
    }
    let _:Document=load(db,tx,"users",doc!{"_id":&s.actor_user_id,"user_type":"person","is_active":true,"platform_personal_deleting":{"$ne":true}},fence).await?;
    let _: Document = load(
        db,
        tx,
        "assistant_agents",
        doc! {"_id":&s.agent_id,"user_id":&s.actor_user_id,"destroyed_at":null},
        fence,
    )
    .await?;
    if conversation_exists {
        let _:Document=load(db,tx,"assistant_conversations",doc!{"_id":&s.conversation_id,"user_id":&s.actor_user_id,"agent_id":&s.agent_id,"credential_api_key_id":&s.key_id,"agent_owner_id":null,"shared_agent_thread":{"$ne":true},"guest_turn":{"$ne":true}},fence).await?;
    }
    let c = &r.channel;
    if fence {
        super::platform_bot_authority::fence_personal_routes(
            db,
            &c.channel_bot_id,
            &s.actor_user_id,
            tx,
        )
        .await?;
    } else if !super::platform_bot_authority::permits_personal_routes(db, &c.channel_bot_id).await?
    {
        return Err(refused());
    }
    let _:Document=load(db,tx,"channel_bots",doc!{"_id":&c.channel_bot_id,"user_id":&s.actor_user_id,"is_active":true,"status":{"$ne":"suspended"}},fence).await?;
    let route = c.route_id.as_deref().ok_or_else(refused)?;
    let _:Document=load(db,tx,"channel_conversations",doc!{"_id":route,"user_id":&s.actor_user_id,"channel_bot_id":&c.channel_bot_id,"agent_api_key_id":&c.route_api_key_id,"is_active":true,"retired_by_transfer":{"$ne":true}},fence).await?;
    for key in std::iter::once(s.key_id.as_str())
        .chain(std::iter::once(c.route_api_key_id.as_str()))
        .chain(c.agent_api_key_id.as_deref())
    {
        let _:Document=load(db,tx,"api_keys",doc!{"_id":key,"user_id":&s.actor_user_id,"is_active":true,"$or":[{"expires_at":null},{"expires_at":{"$gt":bson::DateTime::now()}}]},fence).await?;
    }
    Ok(r)
}

pub(crate) async fn check(db: &Database, task: &TaskAdmission) -> AppResult<()> {
    if let Some(source) = &task.channel_source {
        if task.audience != audience(source) || task.actor_user_id != source.actor_user_id {
            return Err(refused());
        }
        let mut session = db.client().start_session().await?;
        validate(db, &mut session, source, false).await?;
    }
    Ok(())
}

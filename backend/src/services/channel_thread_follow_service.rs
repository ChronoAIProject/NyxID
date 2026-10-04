//! Direct follow authority and lifecycle. No message bodies or provider I/O.
use super::{api_key_mutation_service as transactions, channel_thread_service::ThreadReplyTarget};
use crate::{
    errors::{AppError, AppResult},
    models::{
        assistant_conversation::{AgentEvent, AssistantConversation, ChannelOrigin},
        channel_thread::{ChannelThreadFacts, ThreadKind, ThreadSenderKind},
        channel_thread_follow::ThreadTurnBinding,
        nyxbot_channel::{
            COLLECTION_NAME as CHANNELS, NyxbotChannel, NyxbotThread,
            THREADS_COLLECTION_NAME as THREADS,
        },
    },
};
use chrono::{Duration, Utc};
use futures::TryStreamExt;
use mongodb::{
    ClientSession, Database,
    bson::{self, doc},
    options::ReturnDocument,
};

pub mod discovery;
pub use discovery::*;
pub mod lifecycle;
pub mod turns;
pub use lifecycle::*;
pub use turns::*;
pub const SCOPE: &str = "platform_thread";
pub const IDLE_HOURS: i64 = 24;
pub const OPENING_SECONDS: i64 = 60;
pub const CHAT_CAP: u64 = 32;
pub const BOT_CAP: u64 = 256;

pub fn channel_generation(channel: &NyxbotChannel, settings: &NyxbotThread) -> i64 {
    if settings.agent_id.is_some() {
        0
    } else {
        channel.follow_binding_generation
    }
}

pub fn is_child(chat: &NyxbotThread) -> bool {
    chat.follow.record_scope.as_deref() == Some(SCOPE)
}
fn supported_child(chat: &NyxbotThread) -> bool {
    chat.follow.thread_identity_version == Some(1)
        && matches!(
            chat.follow.thread_kind,
            Some(
                ThreadKind::Native | ThreadKind::Topic | ThreadKind::ReplyChain | ThreadKind::Email,
            )
        )
        && matches!(
            chat.follow.follow_state.as_deref(),
            Some("opening" | "active" | "stopped" | "expired" | "unavailable")
        )
}
pub fn follows(chat: &NyxbotThread) -> bool {
    chat.follow.follow_state.as_deref() == Some("active")
        && chat
            .follow
            .follow_expires_at
            .is_some_and(|t| t > Utc::now())
}
pub fn eligible(channel: &NyxbotChannel, settings: &NyxbotThread, sender: &str) -> Option<bool> {
    if sender.is_empty() {
        return None;
    }
    if channel.owner_sender_ids.iter().any(|id| id == sender) {
        return Some(false);
    }
    let allowed = match settings.members.as_deref() {
        Some("everyone") => true,
        None => settings.owner_seen,
        _ => false,
    };
    allowed.then_some(true)
}

pub fn eligible_for_facts(
    channel: &NyxbotChannel,
    settings: &NyxbotThread,
    facts: &ChannelThreadFacts,
    sender: &str,
) -> Option<bool> {
    if facts.kind == ThreadKind::Email {
        if channel.owner_sender_ids.iter().any(|id| id == sender) {
            return Some(false);
        }
        return (channel.private_chats.as_deref() == Some("everyone")).then_some(true);
    }
    eligible(channel, settings, sender)
}
pub fn not_found() -> AppError {
    AppError::NotFound("Channel thread not found".into())
}

pub async fn access(db: &Database, owner: &str, channel_id: &str) -> AppResult<NyxbotChannel> {
    let row = db
        .collection::<NyxbotChannel>(CHANNELS)
        .find_one(doc! {"_id": channel_id, "user_id": owner, "status": "active"})
        .await?
        .ok_or_else(not_found)?;
    if let Some(org) = &row.bot_owner_id
        && !super::org_service::is_admin(db, owner, org).await?
    {
        return Err(not_found());
    }
    Ok(row)
}

async fn fence(
    db: &Database,
    channel: &str,
    owner: &str,
    session: &mut ClientSession,
) -> AppResult<NyxbotChannel> {
    db.collection::<NyxbotChannel>(CHANNELS)
        .find_one_and_update(
            doc! {"_id":channel,"user_id":owner,"status":{"$in":["active","pending"]}},
            doc! {"$inc":{"follow_capacity_revision":1}},
        )
        .return_document(ReturnDocument::After)
        .session(session)
        .await?
        .ok_or_else(not_found)
}

fn partition(
    channel: &NyxbotChannel,
    settings: &NyxbotThread,
    target: &ThreadReplyTarget,
    agent: &str,
) -> String {
    let f = target.facts();
    let kind = match f.kind {
        ThreadKind::Native => "native",
        ThreadKind::Topic => "topic",
        ThreadKind::ReplyChain => "reply_chain",
        ThreadKind::Email => "email",
        _ => "unknown",
    };
    let generations = format!(
        "{}:{}",
        channel_generation(channel, settings),
        settings.follow.binding_generation
    );
    let mut bytes = Vec::new();
    for part in [
        channel.platform.as_str(),
        &channel.channel_bot_id,
        if f.kind == ThreadKind::Email {
            "email_shared"
        } else {
            settings.id.as_str()
        },
        kind,
        f.root_id.as_deref().unwrap_or_default(),
        &generations,
        agent,
    ] {
        bytes.extend_from_slice(&(part.len() as u64).to_be_bytes());
        bytes.extend_from_slice(part.as_bytes());
    }
    format!("thread_v1_{}", super::internal_auth::sha256_hex(&bytes))
}

pub enum Selection {
    Legacy,
    Quiet,
    Child(Box<NyxbotThread>, ThreadTurnBinding),
    Stopped(Box<NyxbotThread>, ThreadTurnBinding),
}

/// Reserve metadata only; activation and renewal happen with the turn/queue
/// transaction. Every allocator/stopping operation writes the channel fence.
#[allow(clippy::too_many_arguments)]
pub async fn select(
    db: &Database,
    owner: &str,
    channel_id: &str,
    parent_id: &str,
    settings_id: &str,
    target: &ThreadReplyTarget,
    source: &str,
    sender: &str,
    addressed: bool,
    stop: bool,
    agent: &str,
    enabled: bool,
) -> AppResult<Selection> {
    access(db, owner, channel_id).await?;
    let enabled = enabled
        && super::feature_flag_service::personal_flag_enabled(
            db,
            owner,
            super::feature_flag_service::NYXBOT_THREAD_FOLLOW_FLAG_KEY,
        )
        .await?;
    if target.facts().sender_kind != ThreadSenderKind::Human {
        return Ok(Selection::Quiet);
    }
    let db = db.clone();
    let owner = owner.to_owned();
    let channel_id = channel_id.to_owned();
    let parent_id = parent_id.to_owned();
    let settings_id = settings_id.to_owned();
    let source = source.to_owned();
    let sender = sender.to_owned();
    let agent = agent.to_owned();
    let target = target.clone();
    let mut session = db.client().start_session().await?;
    session.start_transaction().and_run2(async move |session| {
            let db=&db;
            let owner=owner.as_str();
            let channel_id=channel_id.as_str();
            let parent_id=parent_id.as_str();
            let settings_id=settings_id.as_str();
            let source=source.as_str();
            let sender=sender.as_str();
            let agent=agent.as_str();
            let target=&target;

        let work: AppResult<Selection> = Box::pin(async {
            let channel = fence(db, channel_id, owner, session).await?;
            if channel.status != "active" || channel.transport != "direct" { return Ok(Selection::Legacy); }
            let settings = db.collection::<NyxbotThread>(THREADS)
                .find_one(doc! {"_id":settings_id,"channel_id":channel_id,"user_id":owner,"record_scope":{"$ne":SCOPE}})
                .session(&mut *session).await?.ok_or_else(not_found)?;
            if settings.follow.threads.as_deref().is_some_and(|s| s != "follow") { return Ok(Selection::Legacy); }
            if settings.agent_id.as_deref().or(channel.agent_id.as_deref()).is_some_and(|a| a != agent) { return Err(not_found()); }
            let Some(guest) = eligible_for_facts(&channel, &settings, target.facts(), sender) else { return Ok(Selection::Quiet); };
            if stop && guest {return Ok(Selection::Quiet);}
            if settings_id!=parent_id {
                db.collection::<NyxbotThread>(THREADS).update_one(doc! {"_id":settings_id,"user_id":owner,"channel_id":channel_id},
                    doc! {"$set":{"parent_chat_id":parent_id}}).session(&mut *session).await?;
            }
            let key = partition(&channel, &settings, target, agent);
            let rows = db.collection::<NyxbotThread>(THREADS);
            let mut child = rows.find_one(doc! {"channel_id":channel_id,"partition":&key})
                .session(&mut *session).await?;
            if child.as_ref().is_some_and(|child| !supported_child(child)) {
                return Ok(Selection::Quiet);
            }
            if !enabled && child.is_none() { return Ok(Selection::Legacy); }
            let active = enabled && child.as_ref().is_some_and(follows);
            // Aurinko remains a legacy private chat until an addressed message
            // opens a shared child or an existing child is actively followed.
            if target.facts().kind == ThreadKind::Email && !active && !addressed {
                return Ok(Selection::Legacy);
            }
            if stop && !guest && (active || addressed) {
                if let Some(mut child) = child {
                    stop_in_session(db, &mut child, "owner", session).await?;
                    let binding = binding(&child, source, sender, guest, agent);
                    return Ok(Selection::Stopped(Box::new(child), binding));
                }
                return Ok(Selection::Quiet);
            }
            if !(active || addressed || child.is_some() && settings.reply_mode.as_deref()==Some("all")) {
                return Ok(if child.is_none() && settings.reply_mode.as_deref()==Some("all") {Selection::Legacy}else{Selection::Quiet});
            }
            if !active && addressed && enabled {
                reserve(db, &channel, &settings, parent_id, &key, target, sender, agent, &mut child, session).await?;
            }
            let mut child = child.ok_or_else(not_found)?;
            if child.conversation_id.is_none() && (addressed || settings.reply_mode.as_deref()==Some("all")) {
                child=rows.find_one_and_update(doc! {"_id":&child.id,"conversation_id":bson::Bson::Null},
                    doc! {"$set":{"conversation_id":format!("nyxa-{}",uuid::Uuid::new_v4().simple()),"context_status":"pending","context_message_count":0}})
                    .return_document(ReturnDocument::After).session(&mut *session).await?.ok_or_else(not_found)?;
            }

            let binding = binding(&child, source, sender, guest, agent);
            // Policy is a request-lifetime projection, never a child snapshot.
            child.agent_id = Some(agent.into());
            child.members = settings.members;
            child.owner_seen = settings.owner_seen;
            child.reply_mode = settings.reply_mode;
            Ok(Selection::Child(Box::new(child), binding))
        }).await;
        transactions::transaction_result(work)
    }).await.map_err(transactions::map_transaction_error)
}

fn binding(
    child: &NyxbotThread,
    source: &str,
    sender: &str,
    guest: bool,
    agent: &str,
) -> ThreadTurnBinding {
    ThreadTurnBinding {
        child_id: child.id.clone(),
        source_message_id: source.into(),
        sender_id: sender.into(),
        guest,
        revision: child.follow.follow_revision,
        generation: child.follow.binding_generation,
        channel_generation: child.follow.binding_channel_generation,
        agent_id: agent.into(),
        queued: false,
    }
}

#[cfg(test)]
mod tests;

/// Metadata-only overload counters. Never retain an unadmitted body.
pub async fn note_busy(db: &Database, binding: &ThreadTurnBinding, busy: bool, dropped: bool) {
    let _=db.collection::<NyxbotThread>(THREADS).update_one(doc! {"_id":&binding.child_id,"record_scope":SCOPE},
        doc! {"$inc":{"follow_busy_count":i64::from(busy),"follow_drop_count":i64::from(dropped)}}).await;
}

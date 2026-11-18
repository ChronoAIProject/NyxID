//! A fixed, terminal org refusal can answer only its original inbound message.
use super::{ReplyError, ReplyResult, negotiated, reply_unavailable};
use crate::{
    crypto::jwt::RelayReplyClaims,
    errors::{AppError, AppResult},
    models::{
        assistant_conversation::ChannelOrigin,
        channel_bot::ChannelBot,
        channel_message::{COLLECTION_NAME as MESSAGES, ChannelMessage},
        channel_thread::ThreadKind,
        nyxbot_channel::{EVENTS_COLLECTION_NAME as EVENTS, NyxbotChannel, NyxbotEvent},
        nyxbot_response_read::OrgNotice,
        reply_token_use::ReplyTokenUse,
    },
    services::{
        channel_platform::{
            BotCredentials, OutboundReply, PlatformAdapter, PostEffectGuard, SendOutcome,
        },
        channel_reply_delivery::Delivery,
        channel_thread_follow_service as follow,
        channel_thread_service::{
            ReplyBinding, ThreadReplyTarget, delivery::ThreadReplyToken, resolution,
        },
        feature_flag_service as flags,
        nyxbot_response_read::{notice_target_digest, org_notice},
        org_channel_service as org,
    },
};
use bson::{Document, doc};
use chrono::Utc;
use mongodb::{ClientSession, Database};
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub(in crate::services::channel_thread_service) struct NoticeTarget {
    event_id: String,
    origin: ChannelOrigin,
    notice: OrgNotice,
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn target(
    db: &Database,
    adapter: &dyn PlatformAdapter,
    bot: &ChannelBot,
    credentials: &BotCredentials<'_>,
    source: &ChannelMessage,
    channel: &NyxbotChannel,
    raw: &Document,
    event: &NyxbotEvent,
    plain_text: Option<&str>,
) -> ReplyResult<ThreadReplyTarget> {
    let rejected = || ReplyError::target(reply_unavailable());
    let notice = org_notice(raw, event)
        .map_err(|_| rejected())?
        .ok_or_else(rejected)?;
    let delivery = event.delivery.as_ref().ok_or_else(rejected)?;
    let binding = delivery.origin.thread.as_ref().ok_or_else(rejected)?;
    if event.channel_id != channel.id
        || event.user_id != channel.user_id
        || event.relay_message_id.as_deref() != Some(source.id.as_str())
        || binding.source_message_id != source.id
        || event.resolved_thread_id.as_deref() != Some(binding.child_id.as_str())
        || delivery.origin.nyxbot_channel_id != channel.id
        || delivery.origin.platform != channel.platform
        || delivery.transport != "gateway"
        || delivery.sender_id != binding.sender_id
        || delivery.guest != binding.guest
        || delivery.source_route_id != channel.route_id
        || delivery.source_route_api_key_id.as_deref() != Some(&channel.route_api_key_id)
        || notice.bot_id != bot.id
        || channel.binding_id.as_deref() != Some(&notice.binding_id)
        || !plain_text
            .is_some_and(|text| crate::crypto::token::hash_token(text) == notice.text_sha256)
    {
        return Err(rejected());
    }
    if !notice.pending {
        return Err(already_claimed().into());
    }
    let proof = NoticeTarget {
        event_id: event.id.clone(),
        origin: delivery.origin.clone(),
        notice,
    };
    validate(db, bot, source, &proof).await?;
    let mut target =
        resolution::resolve_for_reply(db, adapter, bot, credentials, &source.id).await?;
    if notice_target_digest(target.facts()) != proof.notice.target_sha256 {
        return Err(rejected());
    }
    target.authority = ReplyBinding::Notice(Box::new(proof));
    Ok(target)
}

pub(in crate::services::channel_thread_service) fn validate<'a>(
    db: &'a Database,
    bot: &'a ChannelBot,
    source: &'a ChannelMessage,
    proof: &'a NoticeTarget,
) -> futures::future::BoxFuture<'a, ReplyResult<()>> {
    Box::pin(async move {
        let context = (db, bot, source, proof);
        crate::services::transaction_service::run_db_only(
            db,
            &context,
            |tx, (db, bot, source, proof)| Box::pin(check_in_session(db, tx, bot, source, proof)),
        )
        .await
        .map_err(ReplyError::authority)
    })
}

fn already_claimed() -> AppError {
    AppError::Unauthorized("Notice already claimed".into())
}

/// Only Lark/Feishu fixed text reaches this guard. Their adapter invokes it
/// after tenant-token exchange, immediately before the message POST.
pub(in crate::services::channel_thread_service) fn dispatch<'a>(
    db: &Database,
    bot: &ChannelBot,
    source: &ChannelMessage,
    proof: &NoticeTarget,
    token: Option<&ThreadReplyToken<'_>>,
    part: &OutboundReply,
    send: futures::future::BoxFuture<'a, AppResult<SendOutcome>>,
) -> futures::future::BoxFuture<'a, ReplyResult<(Delivery, AppResult<SendOutcome>)>> {
    let exact = part.metadata.is_none()
        && part.attachments.is_empty()
        && part
            .text
            .as_deref()
            .is_some_and(|text| crate::crypto::token::hash_token(text) == proof.notice.text_sha256);
    let token = token.map(|t| (t.usage.clone(), t.claims.clone()));
    let context = Arc::new((
        db.clone(),
        bot.clone(),
        source.clone(),
        proof.clone(),
        token,
    ));
    Box::pin(async move {
        if !exact {
            return Err(ReplyError::target(reply_unavailable()));
        }
        let (receipt, mut dispatched) = tokio::sync::oneshot::channel();
        let receipt = Mutex::new(Some(receipt));
        let guard: PostEffectGuard = Arc::new(move || {
            let receipt = receipt.lock().unwrap().take();
            let context = Arc::clone(&context);
            Box::pin(async move {
                let receipt = receipt.ok_or_else(already_claimed)?;
                let claim = crate::services::transaction_service::run_db_only(
                    &context.0,
                    context.as_ref(),
                    |tx, (db, bot, source, proof, token)| {
                        Box::pin(claim_in_session(db, tx, bot, source, proof, token.as_ref()))
                    },
                )
                .await;
                match claim {
                    Ok(mut delivery) => {
                        delivery.trace_attempt(0);
                        receipt
                            .send(Ok(delivery))
                            .map_err(|_| crate::services::channel_reply_delivery::unconfirmed())
                    }
                    Err(error) => {
                        let failure = ReplyError::authority(error);
                        let _ = receipt.send(Err(failure.class));
                        Err(failure.error)
                    }
                }
            })
        });
        let sent = crate::services::channel_turn_preparation::with_effect(guard, send).await;
        match dispatched.try_recv() {
            Ok(Ok(delivery)) => Ok((delivery, sent)),
            Ok(Err(class)) => Err(ReplyError {
                class,
                error: sent.err().unwrap_or_else(reply_unavailable),
            }),
            Err(_) => Err(sent
                .err()
                .unwrap_or_else(crate::services::channel_reply_delivery::unconfirmed)
                .into()),
        }
    })
}

async fn claim_in_session(
    db: &Database,
    tx: &mut ClientSession,
    bot: &ChannelBot,
    source: &ChannelMessage,
    proof: &NoticeTarget,
    token: Option<&(ReplyTokenUse, RelayReplyClaims)>,
) -> AppResult<Delivery> {
    // Check the previous flag first. A losing callback cannot journal a
    // subsequent authority change against the winning delivery.
    let previous = db
        .collection::<Document>(EVENTS)
        .find_one_and_update(
            doc! {"_id": &proof.event_id, "event_id": &source.id, "org_notice.bot_id": &bot.id},
            doc! {"$set": {"org_notice.pending": false}},
        )
        .return_document(mongodb::options::ReturnDocument::Before)
        .projection(doc! {"org_notice.pending": 1})
        .session(&mut *tx)
        .await?
        .ok_or_else(reply_unavailable)?;
    let pending = previous
        .get_document("org_notice")
        .and_then(|notice| notice.get_bool("pending"))
        .map_err(|_| reply_unavailable())?;
    if !pending {
        return Err(already_claimed());
    }
    check_in_session(db, tx, bot, source, proof).await?;
    if let Some((_, claims)) = token {
        claims.require_unexpired()?;
    }
    Delivery::claim_org_notice(db, tx, bot, source, token.map(|(usage, _)| usage)).await
}

async fn check_in_session(
    db: &Database,
    session: &mut ClientSession,
    bot: &ChannelBot,
    source: &ChannelMessage,
    proof: &NoticeTarget,
) -> AppResult<()> {
    let origin = &proof.origin;
    let b = origin.thread.as_ref().ok_or_else(reply_unavailable)?;
    let e = origin
        .org_execution
        .as_ref()
        .ok_or_else(reply_unavailable)?;
    let notice = &proof.notice;
    let event = db.collection::<Document>(EVENTS).find_one(doc! {
        "_id":&proof.event_id,"event_id":&source.id,"relay_message_id":&source.id,
        "channel_id":&origin.nyxbot_channel_id,"status":"refused","turn_id":null,"conversation_id":null,
        "expires_at":{"$gt":bson::DateTime::now()},"resolved_thread_id":&b.child_id,
        "org_notice.version":1,"org_notice.response_id":&notice.response_id,
        "org_notice.expires_at":{"$gt":bson::DateTime::now()},"org_notice.text_sha256":&notice.text_sha256,
        "org_notice.target_sha256":&notice.target_sha256,"org_notice.binding_id":&notice.binding_id,
        "org_notice.bot_id":&bot.id,
        "delivery.source_route_api_key_id":&source.agent_api_key_id,"delivery.source_route_id":&source.conversation_id,
        "delivery.origin":bson::to_bson(origin).map_err(|_| reply_unavailable())?,
    }).projection(doc!{"_id":1}).session(&mut *session).await?;
    if event.is_none() {
        return Err(reply_unavailable());
    }
    let (mut channel, child, settings) =
        org::notice_audience_in_session(db, session, origin).await?;
    if !negotiated(&channel)
        || channel.gateway_source.is_some()
        || channel.binding_id.as_deref() != Some(&notice.binding_id)
        || channel.route_id.as_deref() != Some(&source.conversation_id)
        || source.agent_api_key_id.as_deref() != Some(&channel.route_api_key_id)
        || channel.channel_bot_id != bot.id
        || channel.bot_owner_id.as_deref() != Some(&bot.user_id)
        || child.id != b.child_id
        || !follow::is_child(&child)
        || follow::is_child(&settings)
        || child.follow.thread_identity_version != Some(1)
        || child.follow.thread_kind != Some(ThreadKind::Native)
        || !matches!(settings.kind.as_deref(), Some("group" | "channel"))
        || child.follow.binding_generation != b.generation
        || settings.follow.binding_generation != b.generation
        || child.follow.binding_channel_generation != b.channel_generation
        || follow::channel_generation(&channel, &settings) != b.channel_generation
        || child.follow.follow_revision != b.revision
        || child.follow.bound_agent_id.as_deref() != Some(&b.agent_id)
        || e.agent_id != b.agent_id
        || e.guest != b.guest
        || org::sender_identity(&channel, &b.sender_id) != e.sender_hash
        || settings
            .follow
            .threads
            .as_deref()
            .is_some_and(|mode| mode != "follow")
    {
        return Err(reply_unavailable());
    }
    let available = follow::follows(&child)
        || (child.follow.follow_state.as_deref() == Some("opening")
            && child
                .follow
                .follow_opening_expires_at
                .is_some_and(|t| t > Utc::now()));
    if !available
        || !flags::personal_flag_enabled(db, &channel.user_id, flags::NYXBOT_THREAD_FOLLOW_FLAG_KEY)
            .await?
    {
        return Err(reply_unavailable());
    }
    let live = db.collection::<ChannelMessage>(MESSAGES).find_one(doc! {
        "_id":&source.id,"direction":"inbound","channel_bot_id":&bot.id,"user_id":&bot.user_id,"platform":&origin.platform,
        "sender_platform_id":&b.sender_id,"agent_api_key_id":&channel.route_api_key_id,"conversation_id":&channel.route_id,
        "nyxbot_admission.requires_org_notice":true,
    }).session(&mut *session).await?.ok_or_else(reply_unavailable)?;
    let facts = live.thread_context.as_ref().ok_or_else(reply_unavailable)?;
    channel.owner_sender_ids = if b.guest {
        Vec::new()
    } else {
        vec![b.sender_id.clone()]
    };
    if facts.kind != ThreadKind::Native
        || !resolution::source_matches(facts, &live)
        || facts.root_id.is_none()
        || facts.root_id != child.follow.thread_root_id
        || child.platform_chat_id.as_deref() != Some(&facts.chat_id)
        || b.sender_kind != Some(facts.sender_kind)
        || notice_target_digest(facts) != notice.target_sha256
        || follow::eligible_for_facts(&channel, &settings, facts, &b.sender_id) != Some(b.guest)
    {
        return Err(reply_unavailable());
    }
    if !resolution::eligible_source_live(db, bot, &live).await? {
        return Err(reply_unavailable());
    }
    Ok(())
}

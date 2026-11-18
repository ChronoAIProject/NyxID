//! Bounded management-time negotiation. No discovery or membership reads run
//! for ordinary gateway events without follow.
use super::*;
use crate::models::nyxbot_channel::GatewayThreadSupport;

pub(super) async fn discover(state: &AppState, row: &mut NyxbotChannel) -> AppResult<()> {
    if !matches!(row.platform.as_str(), "lark" | "feishu")
        || row.gateway_bot_id.is_none()
        || row
            .gateway_threads
            .checked_at
            .is_some_and(|at| at + ChronoDuration::minutes(10) > Utc::now())
    {
        return Ok(());
    }
    let Some(id) = &row.gateway_channel_id else {
        return Ok(());
    };
    let creator = creator_bearer(state, &row.user_id)?;
    let response = gateway_call(
        state,
        reqwest::Method::GET,
        &format!("/channels/{}", urlencode(id)),
        &creator,
        None,
        None,
    )
    .await;
    row.gateway_threads.checked_at = Some(Utc::now());
    if let Ok(response) = response
        && response.status == 200
    {
        row.gateway_threads.supported = response.body["thread_contract_versions"]
            .as_array()
            .is_some_and(|v| v.contains(&json!(1)));
        if let Some(version) = response.body["version"].as_i64() {
            row.gateway_version = Some(version);
        }
    }
    state.db.collection::<NyxbotChannel>(CHANNELS).update_one(doc! {"_id":&row.id,"transport":"gateway"},
        doc! {"$set":{"gateway_threads.checked_at":bson::DateTime::from_chrono(row.gateway_threads.checked_at.unwrap()),
            "gateway_threads.supported":row.gateway_threads.supported, "gateway_version":row.gateway_version}}).await?;
    Ok(())
}

pub(super) async fn desired(
    state: &AppState,
    row: &NyxbotChannel,
) -> AppResult<GatewayThreadSupport> {
    let mut next = row.gateway_threads.clone();
    if !next.supported {
        next.version = None;
        next.enabled = false;
        next.follow_chat_ids.clear();
        next.excluded_chat_ids.clear();
        return Ok(next);
    }
    next.enabled = thread_follow::enabled(state, &row.user_id).await?;
    if !next.enabled && next.version.is_none() {
        return Ok(next);
    }
    next.version = Some(1);
    let parents: Vec<NyxbotThread> = state
        .db
        .collection(THREADS)
        .find(doc! {
            "channel_id":&row.id,"user_id":&row.user_id,"kind":{"$in":["group","channel"]},
            "record_scope":{"$ne":crate::services::channel_thread_follow_service::SCOPE},
        })
        .sort(doc! {"_id":1})
        .limit(257)
        .await?
        .try_collect()
        .await?;
    let active: Vec<NyxbotThread> = state.db.collection(THREADS).find(doc! {
        "channel_id":&row.id,"user_id":&row.user_id,"record_scope":crate::services::channel_thread_follow_service::SCOPE,
        "follow_state":"active","follow_expires_at":{"$gt":bson::DateTime::now()},
    }).limit(256).await?.try_collect().await?;
    // If the bounded snapshot cannot represent every explicit exclusion,
    // process only the positively selected chats. Never add follow work to a
    // non-follow chat whose settings fell outside the snapshot.
    let flag_enabled = next.enabled;
    next.enabled &= parents.len() <= 256;
    // Active children have priority over default-enabled chats at the cap.
    next.follow_chat_ids = active
        .iter()
        .filter_map(|p| p.platform_chat_id.clone())
        .collect();
    next.follow_chat_ids.sort();
    next.follow_chat_ids.dedup();
    for parent in parents
        .iter()
        .filter(|p| flag_enabled && p.follow.threads.as_deref().is_none_or(|m| m == "follow"))
    {
        if let Some(chat) = &parent.platform_chat_id
            && next.follow_chat_ids.len() < 256
            && !next.follow_chat_ids.contains(chat)
        {
            next.follow_chat_ids.push(chat.clone());
        }
    }
    next.follow_chat_ids.sort();
    next.excluded_chat_ids = parents
        .iter()
        .filter(|p| {
            p.platform_thread_id.is_none()
                && p.follow.threads.as_deref().is_some_and(|m| m != "follow")
        })
        .filter_map(|p| p.platform_chat_id.clone())
        .collect();
    next.excluded_chat_ids.sort();
    next.excluded_chat_ids.dedup();
    Ok(next)
}

pub(super) fn policy(support: &GatewayThreadSupport) -> Value {
    json!({"version":1,"follow_chat_ids":support.follow_chat_ids})
}

/// Asynchronous updates use their original admitted event, never the chat's
/// newest reference. The normal late-answer worker owns final-answer retries.
pub(super) async fn send(
    state: &AppState,
    channel: &NyxbotChannel,
    origin: &ChannelOrigin,
    conversation: &str,
    text: &str,
) -> AppResult<()> {
    let result = Box::pin(send_inner(state, channel, origin, conversation, text)).await;
    if let Err(failure) = &result {
        crate::services::channel_thread_service::gateway::record_failure(
            &state.db,
            &channel.channel_bot_id,
            &channel.user_id,
            origin
                .thread
                .as_ref()
                .map_or("", |b| b.source_message_id.as_str()),
            failure.class,
            &failure.error,
        )
        .await;
    }
    result.map_err(|failure| failure.error)
}

fn missing_reply_source() -> crate::services::channel_thread_service::gateway::ReplyError {
    crate::services::channel_thread_service::gateway::ReplyError::target(
        crate::services::channel_thread_follow_service::not_found(),
    )
}

async fn send_inner(
    state: &AppState,
    channel: &NyxbotChannel,
    origin: &ChannelOrigin,
    conversation: &str,
    text: &str,
) -> crate::services::channel_thread_service::gateway::ReplyResult<()> {
    use crate::services::channel_thread_service::gateway::{ReplyError, ReplyFailure};
    let binding = origin.thread.as_ref().ok_or_else(missing_reply_source)?;
    let event = state.db.collection::<NyxbotEvent>(EVENTS).find_one(doc! {
        "channel_id":&channel.id,"user_id":&channel.user_id,"event_id":&binding.source_message_id,
        "resolved_thread_id":&binding.child_id,
    }).await?.ok_or_else(missing_reply_source)?;
    let (actor, admitted_origin, admitted_conversation) =
        crate::services::channel_thread_service::gateway::admitted_reply(
            &event,
            channel,
            &binding.source_message_id,
        )
        .map_err(ReplyError::target)?;
    if admitted_conversation != conversation || !same_reply_identity(admitted_origin, origin) {
        return Err(missing_reply_source());
    }
    crate::services::channel_thread_follow_service::validate_delivery(
        &state.db,
        actor,
        admitted_origin,
        admitted_conversation,
    )
    .await
    .map_err(ReplyError::authority)?;
    if event.created_at + ChronoDuration::minutes(29) <= Utc::now() {
        return Err(ReplyError {
            error: crate::services::channel_thread_follow_service::not_found(),
            class: ReplyFailure::Expired,
        });
    }
    let Some(ciphertext) = event.event_context_ciphertext.as_ref() else {
        return Err(missing_reply_source());
    };
    let context: Value = serde_json::from_slice(&Zeroizing::new(
        state.encryption_keys.decrypt(ciphertext).await?,
    ))
    .map_err(|_| missing_reply_source())?;
    let reference = context["event_ref"]
        .as_str()
        .ok_or_else(missing_reply_source)?;
    let key = decrypt_agent_key(state, channel)
        .await?
        .ok_or_else(missing_reply_source)?;
    let reply = gateway_call(
        state,
        reqwest::Method::POST,
        &format!("/events/{}/replies", urlencode(reference)),
        &key,
        Some(&json!({"text":bounded_reply(text),"thread_reply":true})),
        None,
    )
    .await?;
    if !(200..300).contains(&reply.status) {
        // Preserve main's outward error and caller retry decisions for every
        // non-success status; classify the observed gateway refusal separately.
        return Err(ReplyError {
            error: crate::services::channel_thread_follow_service::not_found(),
            class: ReplyFailure::Delayed,
        });
    }
    Ok(())
}

// Queue bookkeeping and the native transport reference are not reply authority.
// Keep every source, actor, generation and policy identity comparison exact.
pub(super) fn same_reply_identity(a: &ChannelOrigin, b: &ChannelOrigin) -> bool {
    let (Some(a_thread), Some(b_thread)) = (&a.thread, &b.thread) else {
        return false;
    };
    let mut a_thread = a_thread.clone();
    a_thread.queued = b_thread.queued;
    a.nyxbot_channel_id == b.nyxbot_channel_id
        && a.partition == b.partition
        && a.platform == b.platform
        && a.org_execution == b.org_execution
        && a_thread == *b_thread
}

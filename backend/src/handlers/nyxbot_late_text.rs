//! Recoverable preparation of late text. The event holds only plan metadata.
use super::*;
use crate::{
    models::{
        channel_text_delivery::TextDeliveryNotice,
        channel_turn_preparation::{Preparation, Receipt},
    },
    services::{
        channel_platform::OutboundReply, channel_reply_plan as plan,
        channel_turn_preparation as preparation,
    },
};

#[allow(clippy::too_many_arguments)]
pub(super) async fn send(
    state: &AppState,
    event: &NyxbotEvent,
    d: &ChannelTurnDelivery,
    channel: &NyxbotChannel,
    thread: &NyxbotThread,
    conversation: &AssistantConversation,
    claim: &str,
    saved: Preparation,
    text: &str,
    report: &super::super::images::ImageReport,
) -> AppResult<Option<bool>> {
    let notice = saved.notice.unwrap_or(TextDeliveryNotice {
        attempted: report.attempted,
        sent: report.sent,
        failed: report.failed,
        unknown: report.unknown,
        skipped: report.skipped,
        unsupported: report.unsupported,
        app_visible: report.app_visible,
        automation: report.automation,
        omitted: 0,
    });
    let report = super::super::images::ImageReport {
        attempted: notice.attempted,
        sent: notice.sent,
        failed: notice.failed,
        unknown: notice.unknown,
        skipped: notice.skipped,
        unsupported: notice.unsupported,
        app_visible: notice.app_visible,
        automation: notice.automation,
        ..Default::default()
    };
    let text = super::super::images::append_failure(text, &report);
    if text.trim().is_empty() {
        let confirmed = report.unknown == 0 && report.failed == 0;
        let finished = preparation::empty(
            &state.db,
            &event.id,
            claim,
            if confirmed { "sent" } else { "unknown" },
        )
        .await?;
        return Ok(finished.then_some(confirmed));
    }
    // Gateway/email/public-X keep their existing single-operation semantics.
    // We do not infer a recoverable prefix inside an upstream operation.
    let outbound = OutboundReply {
        text: Some(text),
        metadata: plan::assistant_metadata(&channel.platform),
        attachments: vec![],
        reply_to_platform_message_id: None,
    };
    let parts = if d.transport == "gateway" {
        vec![outbound]
    } else {
        plan::ordinary(
            &outbound,
            &channel.platform,
            thread.platform_chat_id.as_deref().unwrap_or_default(),
        )?
        .unwrap_or_else(|| vec![outbound])
    };
    let fingerprint = plan::fingerprint(&parts);
    let total = parts.len() as u32;
    if let Some(previous) = saved.fingerprint {
        if previous != fingerprint || saved.total != total {
            preparation::empty(&state.db, &event.id, claim, "refused").await?;
            return Ok(None);
        }
    } else {
        preparation::plan(&state.db, &event.id, claim, &fingerprint, total, &notice).await?;
    }
    for (index, part) in parts.into_iter().enumerate().skip(saved.next as usize) {
        let index = index as u32;
        let guard = {
            let bot_id = channel.channel_bot_id.clone();
            let state = state.clone();
            let event = event.clone();
            let d = d.clone();
            let conversation = conversation.clone();
            let claim = claim.to_owned();
            std::sync::Arc::new(move || {
                let bot_id = bot_id.clone();
                let state = state.clone();
                let event = event.clone();
                let d = d.clone();
                let conversation = conversation.clone();
                let claim = claim.clone();
                Box::pin(async move {
                    let checked = crate::services::platform_bot_authority::read_personal_routes(
                        &state.db, &bot_id,
                    )
                    .await?;
                    if eligible_with(&state, &event, &d, &conversation, Some(&checked))
                        .await?
                        .is_none()
                    {
                        return Err(AppError::Forbidden(
                            "Channel answer authority changed".into(),
                        ));
                    }
                    preparation::before_text_effect(
                        &state.db,
                        &event.id,
                        Some(&event),
                        &claim,
                        index,
                    )
                    .await
                }) as futures::future::BoxFuture<'static, AppResult<()>>
            })
        };
        if d.transport == "gateway" {
            let hash = request_hash(d, part.text.as_deref().unwrap_or_default());
            preparation::gateway::plan(&state.db, &event.id, claim, &hash).await?;
            let deadline = d
                .target_expires_at
                .unwrap_or(event.expires_at)
                .min(event.expires_at)
                .min(Utc::now() + ChronoDuration::hours(24));
            if deadline <= Utc::now()
                || (d.target_ciphertext.is_none()
                    && event.created_at + ChronoDuration::minutes(29) <= Utc::now())
            {
                preparation::gateway::record(
                    &state.db,
                    &event.id,
                    claim,
                    &channel.channel_bot_id,
                    &crate::models::gateway_reply::Reply {
                        outcome: crate::models::gateway_reply::Outcome::Rejected,
                        observation: Default::default(),
                    },
                    None,
                    deadline,
                    "gateway_reply_local_refusal",
                )
                .await?;
                return Ok(None);
            }
            let result = preparation::with_effect(
                guard,
                dispatch_occurrence(
                    state,
                    event,
                    d,
                    channel,
                    conversation,
                    part.text.as_deref().unwrap_or_default(),
                    deadline,
                ),
            )
            .await;
            // HTTP failures are classified at the transport boundary. Local
            // preparation/authority failures must not authorize a re-ask.
            let reply = match result {
                Ok(reply) => reply,
                Err(error) if error.is_forbidden() => crate::models::gateway_reply::Reply {
                    outcome: crate::models::gateway_reply::Outcome::Rejected,
                    observation: Default::default(),
                },
                _ => preparation::gateway::unknown(Default::default()),
            };
            #[cfg(test)]
            crate::services::durable_wake_tests::after("late_text_accepted_before_receipt").await;
            preparation::gateway::record(
                &state.db,
                &event.id,
                claim,
                &channel.channel_bot_id,
                &reply,
                None,
                deadline,
                "gateway_reply_observed",
            )
            .await?;
            #[cfg(test)]
            crate::services::durable_wake_tests::after("late_text_receipt_committed").await;
            return Ok(None);
        }
        let result = preparation::with_effect(
            guard,
            crate::services::channel_reply_notice::supplemental(tokio::time::timeout(
                Duration::from_secs(preparation::SEND_SECS as u64),
                Box::pin(send_part(state, event, d, channel, conversation, part)),
            )),
        )
        .await;
        match result {
            Ok(Ok(receipt)) => {
                #[cfg(test)]
                crate::services::durable_wake_tests::after("late_text_accepted_before_receipt")
                    .await;
                preparation::accepted(&state.db, &event.id, claim, index, total, &receipt).await?;
                #[cfg(test)]
                crate::services::durable_wake_tests::after("late_text_receipt_committed").await;
            }
            other => {
                if preparation::unconfirmed(&state.db, &event.id, Some(claim)).await? {
                    return Ok(Some(false));
                }
                // Nothing crossed this claim's boundary. Keep transient
                // preparation failures recoverable; authority refusals retire.
                if let Ok(Err(error)) = other {
                    if error.is_forbidden()
                        || matches!(
                            error,
                            AppError::NotFound(_)
                                | AppError::ValidationError(_)
                                | AppError::Conflict(_)
                        )
                    {
                        preparation::empty(&state.db, &event.id, claim, "refused").await?;
                    }
                    return Err(error);
                }
                return Ok(None);
            }
        }
    }
    Ok(Some(true))
}

async fn send_part(
    state: &AppState,
    event: &NyxbotEvent,
    d: &ChannelTurnDelivery,
    channel: &NyxbotChannel,
    conversation: &AssistantConversation,
    part: OutboundReply,
) -> AppResult<Receipt> {
    if d.origin.thread.is_some() {
        let result = thread_follow::send_bound_reply(
            state,
            channel,
            crate::services::org_channel_service::event_actor(event),
            &d.origin,
            &conversation.id,
            &part,
        )
        .await?;
        if let Some(error) = result.error {
            return Err(error);
        }
        return Ok(Receipt {
            platform_message_ids: result.message_ids,
            outbound_message_id: None,
        });
    }
    use crate::handlers::channel_relay as relay;
    crate::services::platform_bot_authority::require_personal_routes(
        &state.db,
        &channel.channel_bot_id,
    )
    .await?;
    let auth = route_key_auth(state, channel).await?;
    let headers = HeaderMap::new();
    let body = relay::AsyncReplyRequest {
        message_id: event.event_id.clone(),
        thread_reply: false,
        reply: relay::AsyncReplyBody {
            text: part.text.clone(),
            metadata: part.metadata.clone(),
            attachments: vec![],
        },
    };
    let context = relay::resolve_reply_request_context(state, &headers, Some(&auth), &body).await?;
    let adapter = relay::reply_adapter(state, &context)?;
    let Json(result) =
        relay::deliver_prepared_reply(state, &headers, context, part, false, adapter.as_ref())
            .await?;
    Ok(Receipt {
        platform_message_ids: result.platform_message_id.into_iter().collect(),
        outbound_message_id: Some(result.message_id),
    })
}

fn request_hash(d: &ChannelTurnDelivery, text: &str) -> String {
    let mut body = json!({"text":text});
    if d.origin.thread.is_some() {
        body["thread_reply"] = json!(true);
    }
    sha256_hex(body.to_string())
}

pub(super) async fn resume_receipt(state: &AppState, event: &NyxbotEvent) -> AppResult<()> {
    use crate::models::gateway_reply::Outcome;
    let claim = Uuid::new_v4().to_string();
    let Some(saved) = preparation::gateway::acquire(&state.db, &event.id, &claim).await? else {
        return Ok(());
    };
    let wait = saved
        .gateway_receipt
        .as_ref()
        .ok_or_else(preparation::lost)?;
    let d = event.delivery.as_ref().ok_or_else(preparation::lost)?;
    let channel = state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .find_one(doc! {"_id":&event.channel_id})
        .await?;
    let bot = channel.as_ref().map_or("", |c| c.channel_bot_id.as_str());
    let terminal = |reason: &'static str| {
        let claim = &claim;
        async move {
            preparation::gateway::record(
                &state.db,
                &event.id,
                claim,
                bot,
                &preparation::gateway::unknown(wait.observation.clone()),
                Some(wait),
                wait.deadline,
                reason,
            )
            .await
            .map(|_| ())
        }
    };
    if wait.deadline <= Utc::now() {
        return terminal("gateway_receipt_expired").await;
    }
    let (Some(channel), Some(conversation_id)) = (&channel, &event.conversation_id) else {
        return terminal("gateway_reply_authority_changed").await;
    };
    let conversation = match engine::get(
        &state.db,
        crate::services::org_channel_service::event_actor(event),
        conversation_id,
    )
    .await
    {
        Ok(c) => c,
        Err(AppError::NotFound(_) | AppError::Forbidden(_)) => {
            return terminal("gateway_reply_authority_changed").await;
        }
        Err(e) => return Err(e),
    };
    if eligible(state, event, d, &conversation).await?.is_none() {
        return terminal("gateway_reply_authority_changed").await;
    }
    let Some(message) = state.db.collection::<AssistantMessage>(MESSAGES).find_one(doc! {
        "_id":&saved.message_id,"user_id":crate::services::org_channel_service::event_actor(event),
        "conversation_id":conversation_id,"turn_id":&event.turn_id,"role":"assistant",
    }).await? else { return terminal("gateway_reply_source_unavailable").await; };
    let Some(notice) = &saved.notice else {
        return terminal("gateway_reply_plan_changed").await;
    };
    let report = super::super::images::ImageReport {
        attempted: notice.attempted,
        sent: notice.sent,
        failed: notice.failed,
        unknown: notice.unknown,
        skipped: notice.skipped,
        unsupported: notice.unsupported,
        app_visible: notice.app_visible,
        automation: notice.automation,
        ..Default::default()
    };
    let text = super::super::images::append_failure(&settled_answer(&message, d), &report);
    if saved.gateway_request_version != Some(1)
        || d.target_ciphertext.is_none()
        || saved.gateway_request_hash.as_deref() != Some(request_hash(d, &text).as_str())
    {
        return terminal("gateway_reply_plan_changed").await;
    }
    let guard = {
        let state = state.clone();
        let event = event.clone();
        let d = d.clone();
        let conversation = conversation.clone();
        let claim = claim.clone();
        std::sync::Arc::new(move || {
            let state = state.clone();
            let event = event.clone();
            let d = d.clone();
            let conversation = conversation.clone();
            let claim = claim.clone();
            Box::pin(async move {
                if eligible(&state, &event, &d, &conversation).await?.is_none() {
                    return Err(AppError::Forbidden(
                        "Channel answer authority changed".into(),
                    ));
                }
                preparation::gateway::before_gateway_effect(
                    &state.db,
                    &event.id,
                    Some(&event),
                    &claim,
                )
                .await
            }) as futures::future::BoxFuture<'static, AppResult<()>>
        })
    };
    let reply = preparation::with_effect(
        guard,
        dispatch_occurrence(
            state,
            event,
            d,
            channel,
            &conversation,
            &text,
            wait.deadline,
        ),
    )
    .await
    .unwrap_or_else(|_| preparation::gateway::unknown(wait.observation.clone()));
    let reason = if reply.outcome == Outcome::Pending {
        "gateway_reply_waiting"
    } else {
        "gateway_reply_observed"
    };
    preparation::gateway::record(
        &state.db,
        &event.id,
        &claim,
        bot,
        &reply,
        Some(wait),
        wait.deadline,
        reason,
    )
    .await?;
    Ok(())
}

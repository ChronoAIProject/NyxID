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
            let state = state.clone();
            let event = event.clone();
            let d = d.clone();
            let conversation = conversation.clone();
            let claim = claim.to_owned();
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
                    preparation::before_effect(&state.db, &event.id, &claim, index).await
                }) as futures::future::BoxFuture<'static, AppResult<()>>
            })
        };
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
    if d.transport == "gateway" {
        return match dispatch_occurrence(
            state,
            event,
            d,
            channel,
            conversation,
            part.text.as_deref().unwrap_or_default(),
            None,
        )
        .await?
        {
            "confirmed" => Ok(Receipt::default()),
            "rejected" => Err(AppError::Forbidden(
                "Channel answer target unavailable".into(),
            )),
            _ => Err(AppError::ChannelPlatformError(
                "Channel answer unconfirmed".into(),
            )),
        };
    }
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

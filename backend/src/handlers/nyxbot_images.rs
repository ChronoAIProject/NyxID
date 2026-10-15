//! Durable, ordered tool-image delivery through NyxID's own relay adapters.
use std::time::{Duration, Instant};

use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::{
        assistant_conversation::{ChannelOrigin, TurnAttachment},
        nyxbot_channel::{NyxbotChannel, NyxbotThread},
        nyxbot_image_delivery::NyxbotImageDelivery,
    },
    services::{
        assistant_nyxagent as engine, channel_media_service as media,
        channel_platform::{MaterializedAttachment, MediaKind, OutboundReply},
        channel_turn_delivery as claims,
    },
};
use axum::http::HeaderMap;

const MIN_SEND_WINDOW: Duration = Duration::from_secs(10);

#[derive(Clone)]
pub struct ImageSendPlan {
    pub attachments: Vec<TurnAttachment>,
}
impl std::fmt::Debug for ImageSendPlan {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImageSendPlan")
            .field("count", &self.attachments.len())
            .finish()
    }
}

pub enum ImageAnchor {
    Inbound { message_id: String },
    Thread { origin: ChannelOrigin },
    Initiated { chat_id: String },
    NoAnchor,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ImageOutcome {
    Sent,
    Failed(&'static str),
    Unknown,
    Skipped(&'static str),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ImageReport {
    pub attempted: u32,
    pub sent: u32,
    pub failed: u32,
    pub unknown: u32,
    pub skipped: u32,
    pub unsupported: bool,
    pub app_visible: bool,
    pub automation: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct ImageAudience {
    pub owner_private: bool,
}

pub struct ImageSendContext<'a> {
    pub owner: &'a str,
    pub conversation_id: &'a str,
    pub turn_id: &'a str,
    pub channel: &'a NyxbotChannel,
    pub thread: &'a NyxbotThread,
    pub anchor: ImageAnchor,
    pub audience: ImageAudience,
    pub deadline: Option<Instant>,
    pub claim_key: &'a str,
    pub initiated: Option<&'a super::chats::PostTarget>,
}
impl std::fmt::Debug for ImageSendContext<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImageSendContext")
            .field("conversation_id", &self.conversation_id)
            .field("turn_id", &self.turn_id)
            .finish_non_exhaustive()
    }
}

pub fn classify_outcome(error: &AppError) -> ImageOutcome {
    if media::is_delivery_uncertain(error) || matches!(error, AppError::DatabaseError(_)) {
        return ImageOutcome::Unknown;
    }
    if crate::services::channel_billing_service::blocks_channel(error) {
        return ImageOutcome::Failed("billing");
    }
    ImageOutcome::Failed(match error {
        AppError::ChannelPlatformError(_) => "platform_error",
        AppError::ChannelConversationNotReachable(_) => "not_reachable",
        AppError::ChannelMediaUnsupported | AppError::ChannelPlatformSendUnsupported => {
            "unsupported"
        }
        AppError::ChannelMediaTooLarge => "too_large",
        AppError::RateLimited => "rate_limited",
        AppError::Forbidden(_) | AppError::Unauthorized(_) => "refused",
        AppError::ValidationError(_) => "invalid",
        AppError::ChannelBotInactive(_) => "bot_inactive",
        AppError::Conflict(_) => "conflict",
        _ => "unavailable",
    })
}

impl ImageReport {
    fn record(&mut self, outcome: ImageOutcome) {
        match outcome {
            ImageOutcome::Sent => {
                self.attempted += 1;
                self.sent += 1;
            }
            ImageOutcome::Failed(code) => {
                self.attempted += 1;
                self.failed += 1;
                self.unsupported |= code == "unsupported";
            }
            ImageOutcome::Unknown => {
                self.attempted += 1;
                self.unknown += 1;
            }
            ImageOutcome::Skipped(_) => self.skipped += 1,
        }
    }
}

fn existing_report(row: &NyxbotImageDelivery, mut report: ImageReport) -> ImageReport {
    for item in &row.items {
        let outcome = match item.state.as_str() {
            "sent" => ImageOutcome::Sent,
            "failed" => {
                report.unsupported |= item.code.as_deref() == Some("unsupported");
                ImageOutcome::Failed("previous_refusal")
            }
            "skipped" | "pending" => ImageOutcome::Skipped("previous_attempt"),
            _ => ImageOutcome::Unknown,
        };
        report.record(outcome);
    }
    report
}

/// Byte ownership survives reqwest clones and zeroizes when the last body drops.
struct ImageBytes(zeroize::Zeroizing<Vec<u8>>);
impl AsRef<[u8]> for ImageBytes {
    fn as_ref(&self) -> &[u8] {
        self.0.as_slice()
    }
}

pub async fn send_turn_images(
    state: &AppState,
    ctx: ImageSendContext<'_>,
    plan: &ImageSendPlan,
) -> AppResult<ImageReport> {
    let mut report = ImageReport {
        app_visible: ctx.audience.owner_private,
        automation: matches!(ctx.anchor, ImageAnchor::Initiated { .. }),
        ..Default::default()
    };
    let attachments: Vec<_> = plan
        .attachments
        .iter()
        .filter(|attachment| engine::chat_tool_image(attachment))
        .take(engine::MAX_TURN_ATTACHMENTS)
        .cloned()
        .collect();
    if attachments.is_empty() {
        return Ok(report);
    }
    let started = Instant::now();
    let budget = ctx.deadline.map_or(Duration::from_secs(120), |deadline| {
        deadline
            .saturating_duration_since(started)
            .saturating_sub(Duration::from_secs(15))
            .min(Duration::from_secs(120))
    });
    let end = started + budget;
    let ids: Vec<_> = attachments
        .iter()
        .map(|attachment| attachment.id.clone())
        .collect();
    let Some(claim) = claims::claim_images(
        &state.db,
        ctx.owner,
        ctx.conversation_id,
        ctx.turn_id,
        &ctx.channel.id,
        ctx.claim_key,
        &ids,
    )
    .await?
    else {
        return Ok(
            match claims::read_images(&state.db, ctx.conversation_id, ctx.turn_id, ctx.claim_key)
                .await?
            {
                Some(row) => existing_report(&row, report),
                None => {
                    report.skipped = ids.len() as u32;
                    report
                }
            },
        );
    };
    let supported = crate::services::channel_adapters::outbound_capabilities(
        &ctx.channel.platform,
        &state.token_exchange_cache,
    )
    .media
    .outbound
    .contains(&MediaKind::Image)
        && !(ctx.channel.platform == "x"
            && ctx
                .thread
                .platform_chat_id
                .as_deref()
                .is_some_and(crate::services::channel_adapters::x::is_public_conversation));
    let loaded = if supported && !matches!(ctx.anchor, ImageAnchor::NoAnchor) {
        engine::turn_tool_images(
            &state.db,
            &state.encryption_keys,
            ctx.owner,
            ctx.conversation_id,
            ctx.turn_id,
            &attachments,
        )
        .await?
    } else {
        // Claim outcomes still record every item, without loading private bytes
        // for a destination that cannot send them.
        attachments
            .into_iter()
            .map(|attachment| (attachment, Err("unavailable")))
            .collect()
    };
    let paced = matches!(ctx.channel.platform.as_str(), "telegram" | "telegram-new")
        && ctx.thread.kind.as_deref() != Some("private");
    let mut last_send: Option<Instant> = None;
    let mut stop = None;
    for (index, (attachment, bytes)) in loaded.into_iter().enumerate() {
        let claimed =
            claims::claim_image_item(&state.db, &claim.id, &claim.claim_id, &attachment.id).await;
        if !matches!(claimed, Ok(true)) {
            report.record(stop.map_or(ImageOutcome::Unknown, ImageOutcome::Skipped));
            stop = Some("after_unknown");
            continue;
        }
        let mut code = None;
        let mut outcome = if let Some(reason) = stop {
            ImageOutcome::Skipped(reason)
        } else if !supported {
            ImageOutcome::Failed("unsupported")
        } else if matches!(ctx.anchor, ImageAnchor::NoAnchor) {
            ImageOutcome::Failed("no_anchor")
        } else if end.saturating_duration_since(Instant::now()) < MIN_SEND_WINDOW {
            ImageOutcome::Skipped("deadline")
        } else if let Ok(bytes) = bytes {
            if paced && let Some(last) = last_send {
                tokio::time::sleep(
                    (last + Duration::from_secs(1))
                        .saturating_duration_since(Instant::now())
                        .min(end.saturating_duration_since(Instant::now())),
                )
                .await;
            }
            if end.saturating_duration_since(Instant::now()) < MIN_SEND_WINDOW {
                ImageOutcome::Skipped("deadline")
            } else {
                let ext = match attachment.content_type.as_str() {
                    "image/jpeg" => "jpg",
                    "image/png" => "png",
                    "image/gif" => "gif",
                    "image/webp" => "webp",
                    _ => "img",
                };
                let materialized = MaterializedAttachment {
                    kind: MediaKind::Image,
                    bytes: bytes::Bytes::from_owner(ImageBytes(bytes)),
                    filename: Some(format!("image-{}.{ext}", index + 1)),
                    mime_type: Some(attachment.content_type),
                    caption: None,
                };
                let timeout =
                    Duration::from_secs(35).min(end.saturating_duration_since(Instant::now()));
                last_send = Some(Instant::now());
                match tokio::time::timeout(timeout, send_one(state, &ctx, materialized)).await {
                    Ok(Ok(sent_code)) => {
                        code = sent_code;
                        ImageOutcome::Sent
                    }
                    Ok(Err(error)) => {
                        if ctx.channel.platform == "whatsapp"
                            && crate::services::channel_adapters::whatsapp::image_delivery_uncertain(
                                &error,
                            )
                        {
                            ImageOutcome::Unknown
                        } else {
                            classify_outcome(&error)
                        }
                    }
                    Err(_) => ImageOutcome::Unknown,
                }
            }
        } else {
            ImageOutcome::Failed("unavailable")
        };
        let (item_state, item_code) = match outcome {
            ImageOutcome::Sent => ("sent", code),
            ImageOutcome::Failed(reason) => ("failed", Some(reason)),
            ImageOutcome::Unknown => ("unknown", None),
            ImageOutcome::Skipped(reason) => ("skipped", Some(reason)),
        };
        if claims::finish_image_item(
            &state.db,
            &claim.id,
            &claim.claim_id,
            &attachment.id,
            item_state,
            item_code,
        )
        .await
        .is_err()
        {
            outcome = ImageOutcome::Unknown;
        }
        report.record(outcome);
        match outcome {
            ImageOutcome::Unknown => stop = Some("after_unknown"),
            ImageOutcome::Failed("billing") => stop = Some("billing"),
            ImageOutcome::Skipped("deadline") => stop = Some("deadline"),
            _ => {}
        }
    }
    // Failure leaves the non-reclaimable sending row as the crash barrier.
    let _ = claims::finish_images(&state.db, &claim.id, &claim.claim_id).await;
    let destination_kind = match ctx.anchor {
        ImageAnchor::Inbound { .. } | ImageAnchor::NoAnchor => "inbound",
        ImageAnchor::Thread { .. } => "thread",
        ImageAnchor::Initiated { .. } => "initiated",
    };
    super::audit(state, ctx.owner, "nyxbot_channel_image_delivery", serde_json::json!({
        "conversation_id": ctx.conversation_id, "turn_id": ctx.turn_id, "channel_agent_id":ctx.channel.id,
        "destination_kind":destination_kind, "transport":ctx.channel.transport,
        "attempted":report.attempted, "sent":report.sent, "failed":report.failed,
        "unknown":report.unknown, "skipped":report.skipped,
    })).await;
    Ok(report)
}

async fn send_one(
    state: &AppState,
    ctx: &ImageSendContext<'_>,
    attachment: MaterializedAttachment,
) -> AppResult<Option<&'static str>> {
    let outbound = OutboundReply {
        text: None,
        metadata: None,
        attachments: vec![attachment],
        reply_to_platform_message_id: None,
    };
    match &ctx.anchor {
        ImageAnchor::NoAnchor => Err(AppError::NotFound("Image anchor unavailable".into())),
        ImageAnchor::Inbound { message_id } => {
            let auth = super::route_key_auth(state, ctx.channel).await?;
            let body = crate::handlers::channel_relay::AsyncReplyRequest {
                message_id: message_id.clone(),
                thread_reply: false,
                reply: crate::handlers::channel_relay::AsyncReplyBody {
                    text: None,
                    metadata: None,
                    attachments: Vec::new(),
                },
            };
            let headers = HeaderMap::new();
            let context = crate::handlers::channel_relay::resolve_reply_request_context(
                state,
                &headers,
                Some(&auth),
                &body,
            )
            .await?;
            let adapter = crate::handlers::channel_relay::reply_adapter(state, &context)?;
            let response = crate::handlers::channel_relay::deliver_prepared_reply(
                state,
                &headers,
                context,
                outbound,
                false,
                adapter.as_ref(),
            )
            .await?
            .0;
            response
                .platform_message_id
                .ok_or_else(media::upload_failed)?;
            Ok(response.delivered_as.as_ref().and_then(|kinds| {
                (kinds.first().map(String::as_str) == Some("file")).then_some("as_document")
            }))
        }
        ImageAnchor::Thread { origin } => {
            let result = super::thread_follow::send_bound_reply(
                state,
                ctx.channel,
                ctx.owner,
                origin,
                ctx.conversation_id,
                &outbound,
            )
            .await?;
            if let Some(error) = result.error {
                return Err(error);
            }
            if result.message_ids.is_empty() {
                return Err(media::upload_failed());
            }
            Ok((result.delivered_kinds.first() == Some(&MediaKind::File)).then_some("as_document"))
        }
        ImageAnchor::Initiated { chat_id } => {
            let target = ctx
                .initiated
                .filter(|target| target.thread.id == *chat_id)
                .ok_or_else(|| AppError::Conflict("Image post target unavailable".into()))?;
            let response = super::chats::send_post(state, target, outbound).await?;
            response
                .platform_message_id
                .ok_or_else(media::upload_failed)?;
            Ok(response.delivered_as.as_ref().and_then(|kinds| {
                (kinds.first().map(String::as_str) == Some("file")).then_some("as_document")
            }))
        }
    }
}

pub fn failure_note(report: &ImageReport) -> String {
    let total = report.attempted.saturating_add(report.skipped);
    let missing = report
        .failed
        .saturating_add(report.unknown)
        .saturating_add(report.skipped);
    if missing == 0 {
        return String::new();
    }
    let location = if report.automation {
        "in the automation's thread in the NyxID app"
    } else {
        "under this reply in the NyxID app"
    };
    if report.unsupported {
        return if report.app_visible {
            format!("\n\n[This chat cannot receive images; {total} image(s) are {location}.]")
        } else {
            format!("\n\n[This chat cannot receive images; {total} image(s) could not be sent.]")
        };
    }
    if report.sent == 0 {
        return if report.app_visible {
            format!(
                "\n\n[The {total} image(s) from this reply could not be sent to this chat; they are {location}.]"
            )
        } else {
            format!("\n\n[The {total} image(s) from this reply could not be sent to this chat.]")
        };
    }
    if report.app_visible {
        format!(
            "\n\n[{missing} of {total} image(s) could not be sent to this chat; they are {location}.]"
        )
    } else {
        format!("\n\n[{missing} of {total} image(s) could not be sent to this chat.]")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn failure_note_counts_and_audience_cover_partial_all_unsupported_and_automation() {
        assert!(failure_note(&ImageReport::default()).is_empty());
        for app_visible in [false, true] {
            for automation in [false, true] {
                for unsupported in [false, true] {
                    for sent in [0, 1] {
                        let report = ImageReport {
                            attempted: sent + 2,
                            sent,
                            failed: 1,
                            unknown: 1,
                            skipped: 1,
                            unsupported,
                            app_visible,
                            automation,
                        };
                        let note = failure_note(&report);
                        assert_eq!(note.contains("NyxID app"), app_visible);
                        assert_eq!(
                            note.contains("automation's thread"),
                            app_visible && automation
                        );
                        assert!(note.len() < 8000);
                        if unsupported {
                            assert!(note.contains("This chat cannot receive images"));
                        } else if sent > 0 {
                            assert!(note.contains("3 of 4 image(s)"));
                        } else {
                            assert!(note.contains("The 3 image(s)"));
                        }
                    }
                }
            }
        }
    }
    #[test]
    fn image_outcome_distinguishes_uncertainty_from_refusal_and_billing() {
        assert_eq!(
            classify_outcome(&media::upload_failed()),
            ImageOutcome::Unknown
        );
        for (error, code) in [
            (
                AppError::ChannelPlatformError("definite refusal".into()),
                "platform_error",
            ),
            (
                AppError::ChannelConversationNotReachable("refusal".into()),
                "not_reachable",
            ),
            (AppError::ChannelMediaUnsupported, "unsupported"),
            (AppError::ChannelPlatformSendUnsupported, "unsupported"),
            (AppError::ChannelMediaTooLarge, "too_large"),
            (AppError::RateLimited, "rate_limited"),
            (AppError::Forbidden("refused".into()), "refused"),
            (AppError::ValidationError("invalid".into()), "invalid"),
            (
                AppError::ChannelBotInactive("inactive".into()),
                "bot_inactive",
            ),
            (AppError::NotFound("missing".into()), "unavailable"),
            (AppError::Conflict("conflict".into()), "conflict"),
            (AppError::InsufficientCredits, "billing"),
            (
                AppError::BillingProviderUnavailable("unavailable".into()),
                "billing",
            ),
        ] {
            assert_eq!(classify_outcome(&error), ImageOutcome::Failed(code));
        }
    }
}

#[derive(Clone, Copy)]
pub struct ImageTurn<'a> {
    pub turn_id: &'a str,
    pub plan: &'a ImageSendPlan,
    pub app_visible: bool,
}

pub fn event_anchor(
    event: &crate::models::nyxbot_channel::NyxbotEvent,
    delivery: &crate::models::nyxbot_channel::ChannelTurnDelivery,
) -> ImageAnchor {
    if delivery.origin.thread.is_some() {
        return ImageAnchor::Thread {
            origin: delivery.origin.clone(),
        };
    }
    let message = if delivery.transport == "direct" {
        event
            .relay_message_id
            .clone()
            .or_else(|| Some(event.event_id.clone()))
    } else {
        event.relay_message_id.clone()
    };
    message.map_or(ImageAnchor::NoAnchor, |message_id| ImageAnchor::Inbound {
        message_id,
    })
}

pub fn thread_anchor(channel: &NyxbotChannel, thread: &NyxbotThread) -> ImageAnchor {
    let relay_era = thread.event_ref_ciphertext.is_none() && thread.last_message_id.is_some();
    let direct_text_path = channel.transport != "gateway" || relay_era;
    let message = if direct_text_path {
        thread
            .last_relay_message_id
            .clone()
            .or_else(|| thread.last_message_id.clone())
    } else {
        thread.last_relay_message_id.clone()
    };
    message.map_or(ImageAnchor::NoAnchor, |message_id| ImageAnchor::Inbound {
        message_id,
    })
}

pub async fn report_or_failure(
    state: &AppState,
    ctx: ImageSendContext<'_>,
    plan: &ImageSendPlan,
) -> ImageReport {
    let app_visible = ctx.audience.owner_private;
    let automation = matches!(ctx.anchor, ImageAnchor::Initiated { .. });
    match send_turn_images(state, ctx, plan).await {
        Ok(report) => report,
        Err(error) => {
            tracing::warn!(
                code = error.error_code(),
                "Tool image delivery preparation failed"
            );
            let count = plan
                .attachments
                .iter()
                .filter(|attachment| engine::chat_tool_image(attachment))
                .take(engine::MAX_TURN_ATTACHMENTS)
                .count() as u32;
            ImageReport {
                attempted: count,
                failed: count,
                app_visible,
                automation,
                ..Default::default()
            }
        }
    }
}

pub fn append_failure(text: &str, report: &ImageReport) -> String {
    let note = failure_note(report);
    let budget = super::MAX_CHANNEL_REPLY_CHARS.saturating_sub(note.chars().count());
    format!("{}{note}", super::excerpt(text, budget))
}

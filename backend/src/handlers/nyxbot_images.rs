//! Durable, ordered tool-image delivery through NyxID's own relay adapters.
use std::time::{Duration, Instant};

use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::{
        assistant_conversation::{ChannelOrigin, TurnAttachment},
        nyxbot_channel::{NyxbotChannel, NyxbotThread},
        nyxbot_image_delivery::{ImageDeliveryState, NyxbotImageDelivery},
    },
    services::{
        assistant_nyxagent as engine, channel_image_delivery as claims,
        channel_media_service as media,
        channel_platform::{MaterializedAttachment, MediaKind, OutboundReply},
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
    pub outcomes: Vec<ImageItemResult>,
    pub claimed: u32,
    pub attempted: u32,
    pub sent: u32,
    pub failed: u32,
    pub unknown: u32,
    pub skipped: u32,
    pub unsupported: bool,
    pub app_visible: bool,
    pub automation: bool,
}

#[derive(Clone, Debug, serde::Serialize, PartialEq, Eq)]
pub struct ImageItemResult {
    pub attachment_id: String,
    pub status: String,
    pub code: Option<String>,
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
    pub initiated: Option<&'a super::chats::PostTarget>,
    pub fence: Option<&'a crate::services::assistant_acknowledgement_service::ChatAuthority>,
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

pub(super) fn existing_item(row: &NyxbotImageDelivery) -> ImageItemResult {
    existing_outcome(&row.attachment_id, row.state, row.code.clone())
}

pub(super) fn legacy_item(
    attachment: &str,
    outcome: claims::LegacyImageOutcome,
) -> ImageItemResult {
    existing_outcome(attachment, outcome.state, outcome.code)
}

fn existing_outcome(
    attachment: &str,
    state: ImageDeliveryState,
    code: Option<String>,
) -> ImageItemResult {
    let status = match state {
        ImageDeliveryState::Sent => "previously_delivered",
        ImageDeliveryState::Failed => "failed",
        ImageDeliveryState::Skipped => "skipped",
        ImageDeliveryState::Unknown => "unknown",
        ImageDeliveryState::Sending | ImageDeliveryState::Pending => "in_progress_or_unknown",
    };
    ImageItemResult {
        attachment_id: attachment.into(),
        status: status.into(),
        code,
    }
}

struct ImageBytes(zeroize::Zeroizing<Vec<u8>>);
impl AsRef<[u8]> for ImageBytes {
    fn as_ref(&self) -> &[u8] {
        self.0.as_slice()
    }
}

pub fn image_result(report: &ImageReport) -> serde_json::Value {
    let mut result = serde_json::Map::new();
    for status in [
        "sent_now",
        "previously_delivered",
        "failed",
        "skipped",
        "unknown",
        "in_progress_or_unknown",
        "cancelled",
    ] {
        result.insert(
            status.into(),
            serde_json::json!(
                report
                    .outcomes
                    .iter()
                    .filter(|item| item.status == status)
                    .count()
            ),
        );
    }
    result.insert("items".into(), serde_json::json!(report.outcomes));
    serde_json::Value::Object(result)
}

async fn audience_key(state: &AppState, ctx: &ImageSendContext<'_>) -> AppResult<String> {
    match &ctx.anchor {
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
            let context = crate::handlers::channel_relay::resolve_reply_request_context(
                state,
                &HeaderMap::new(),
                Some(&auth),
                &body,
            )
            .await?;
            let adapter = crate::handlers::channel_relay::reply_adapter(state, &context)?;
            crate::handlers::channel_relay::image_reply_audience(&context, adapter.as_ref())
        }
        ImageAnchor::Thread { origin } => {
            super::thread_follow::bound_image_audience(
                state,
                ctx.channel,
                ctx.owner,
                origin,
                ctx.conversation_id,
            )
            .await
        }
        ImageAnchor::Initiated { .. } => {
            let target = ctx
                .initiated
                .ok_or_else(|| AppError::NotFound("Image post target unavailable".into()))?;
            let mut reply = OutboundReply {
                text: None,
                metadata: None,
                attachments: Vec::new(),
                reply_to_platform_message_id: None,
            };
            target.adapter.reply_context(
                crate::services::channel_image_audience::chat_thread(&target.thread),
                target.thread.created_at,
                &mut reply.metadata,
            );
            Ok(crate::services::channel_image_audience::resolve(
                &target.bot.id,
                &target.bot.platform,
                &target.conversation.platform_conversation_id,
                &reply,
            ))
        }
        ImageAnchor::NoAnchor => {
            let reply = OutboundReply {
                text: None,
                metadata: None,
                attachments: Vec::new(),
                reply_to_platform_message_id: None,
            };
            Ok(crate::services::channel_image_audience::resolve(
                &ctx.channel.channel_bot_id,
                &ctx.channel.platform,
                ctx.thread.platform_chat_id.as_deref().unwrap_or_default(),
                &reply,
            ))
        }
    }
}

async fn live(state: &AppState, ctx: &ImageSendContext<'_>) -> AppResult<bool> {
    match ctx.fence {
        Some(chat) => Ok(super::chats::post_turn_is_live(state, chat)
            .await
            .unwrap_or(false)),
        None => Ok(true),
    }
}

pub async fn send_turn_images(
    state: &AppState,
    ctx: ImageSendContext<'_>,
    plan: &ImageSendPlan,
) -> AppResult<ImageReport> {
    Box::pin(send_turn_images_with_stop(state, ctx, plan, None)).await
}

pub(super) async fn send_turn_images_with_stop(
    state: &AppState,
    ctx: ImageSendContext<'_>,
    plan: &ImageSendPlan,
    initial_stop: Option<&'static str>,
) -> AppResult<ImageReport> {
    let mut report = ImageReport {
        app_visible: ctx.audience.owner_private,
        automation: matches!(ctx.anchor, ImageAnchor::Initiated { .. }),
        ..Default::default()
    };
    let mut seen = std::collections::HashSet::new();
    let attachments: Vec<_> = plan
        .attachments
        .iter()
        .filter(|attachment| {
            engine::chat_tool_image(attachment) && seen.insert(attachment.id.as_str())
        })
        .take(engine::MAX_TURN_ATTACHMENTS)
        .cloned()
        .collect();
    if attachments.is_empty() {
        return Ok(report);
    }
    if ctx.channel.gateway_source.is_some() {
        // A native source never borrows relay anchors, credentials or sends.
        return Err(AppError::ChannelMediaUnsupported);
    }
    let key = audience_key(state, &ctx).await?;
    let started = Instant::now();
    let budget = ctx.deadline.map_or(Duration::from_secs(120), |deadline| {
        deadline
            .saturating_duration_since(started)
            .saturating_sub(Duration::from_secs(15))
            .min(Duration::from_secs(120))
    });
    let end = started + budget;
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
    let paced = matches!(ctx.channel.platform.as_str(), "telegram" | "telegram-new")
        && ctx.thread.kind.as_deref() != Some("private");
    let mut last_send: Option<Instant> = None;
    let mut stop = initial_stop;
    let mut cancelled = false;
    for (index, attachment) in attachments.into_iter().enumerate() {
        if cancelled || !live(state, &ctx).await? {
            cancelled = true;
            report.outcomes.push(ImageItemResult {
                attachment_id: attachment.id,
                status: "cancelled".into(),
                code: None,
            });
            continue;
        }
        let (id, attempt) = match claims::claim_for_turn(
            &state.db,
            ctx.owner,
            ctx.conversation_id,
            ctx.turn_id,
            &ctx.channel.id,
            &attachment.id,
            &key,
        )
        .await
        .unwrap_or(claims::ImageClaim::Unknown)
        {
            claims::ImageClaim::Dispatch { id, attempt } => (id, attempt),
            claims::ImageClaim::Existing(row) => {
                report.outcomes.push(existing_item(&row));
                continue;
            }
            claims::ImageClaim::Legacy(outcome) => {
                report.outcomes.push(legacy_item(&attachment.id, outcome));
                continue;
            }
            claims::ImageClaim::Unknown => {
                report.record(ImageOutcome::Unknown);
                report.outcomes.push(ImageItemResult {
                    attachment_id: attachment.id,
                    status: "unknown".into(),
                    code: None,
                });
                stop = Some("after_unknown");
                continue;
            }
        };
        report.claimed += 1;
        let source_exists = claims::source_exists(
            &state.db,
            ctx.owner,
            ctx.conversation_id,
            ctx.turn_id,
            &attachment.id,
        )
        .await;
        if matches!(source_exists, Ok(false)) {
            let _ = claims::remove(&state.db, &id).await;
            report.record(ImageOutcome::Failed("unavailable"));
            report.outcomes.push(ImageItemResult {
                attachment_id: attachment.id,
                status: "failed".into(),
                code: Some("unavailable".into()),
            });
            continue;
        }
        let progress = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let mut code = None;
        let mut outcome = if source_exists.is_err() {
            ImageOutcome::Unknown
        } else if let Some(reason) = stop {
            ImageOutcome::Skipped(reason)
        } else if !supported {
            ImageOutcome::Failed("unsupported")
        } else if matches!(ctx.anchor, ImageAnchor::NoAnchor) {
            ImageOutcome::Failed("no_anchor")
        } else if end.saturating_duration_since(Instant::now()) < MIN_SEND_WINDOW {
            ImageOutcome::Skipped("deadline")
        } else {
            let loaded = engine::turn_tool_images(
                &state.db,
                &state.encryption_keys,
                ctx.owner,
                ctx.conversation_id,
                ctx.turn_id,
                std::slice::from_ref(&attachment),
            )
            .await;
            if let Err(error) = loaded {
                classify_outcome(&error)
            } else if let Some((_, Ok(bytes))) = loaded.unwrap().into_iter().next() {
                if paced && let Some(last) = last_send {
                    tokio::time::sleep(
                        (last + Duration::from_secs(1))
                            .saturating_duration_since(Instant::now())
                            .min(end.saturating_duration_since(Instant::now())),
                    )
                    .await;
                }
                if !live(state, &ctx).await? {
                    cancelled = true;
                    ImageOutcome::Skipped("cancelled")
                } else if end.saturating_duration_since(Instant::now()) < MIN_SEND_WINDOW {
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
                        mime_type: Some(attachment.content_type.clone()),
                        caption: None,
                    };
                    last_send = Some(Instant::now());
                    match tokio::time::timeout(
                        Duration::from_secs(35).min(end.saturating_duration_since(Instant::now())),
                        crate::services::channel_platform::track_provider_attempt(
                            progress.clone(),
                            send_one(state, &ctx, materialized),
                        ),
                    )
                    .await
                    {
                        Ok(Ok(sent_code)) => {
                            code = sent_code;
                            ImageOutcome::Sent
                        }
                        Ok(Err(AppError::AssistantTurnRequired)) => {
                            cancelled = true;
                            ImageOutcome::Skipped("cancelled")
                        }
                        Ok(Err(error)) => if ctx.channel.platform == "whatsapp"
                            && crate::services::channel_adapters::whatsapp::image_delivery_uncertain(
                                &error,
                            ) {
                            ImageOutcome::Unknown
                        } else {
                            classify_outcome(&error)
                        },
                        Err(_) => ImageOutcome::Unknown,
                    }
                }
            } else {
                ImageOutcome::Failed("unavailable")
            }
        };
        let (durable_state, item_code, status) = match outcome {
            ImageOutcome::Sent => (ImageDeliveryState::Sent, code, "sent_now"),
            ImageOutcome::Failed(reason) => (ImageDeliveryState::Failed, Some(reason), "failed"),
            ImageOutcome::Unknown => (ImageDeliveryState::Unknown, None, "unknown"),
            ImageOutcome::Skipped("cancelled") => {
                (ImageDeliveryState::Skipped, Some("cancelled"), "cancelled")
            }
            ImageOutcome::Skipped(reason) => (ImageDeliveryState::Skipped, Some(reason), "skipped"),
        };
        let permanent = matches!(
            outcome,
            ImageOutcome::Sent | ImageOutcome::Failed("unsupported" | "no_anchor" | "unavailable")
        );
        let completion = if !permanent && !progress.load(std::sync::atomic::Ordering::Relaxed) {
            claims::release(&state.db, &id, &attempt).await
        } else {
            claims::finish(&state.db, &id, &attempt, durable_state, item_code).await
        };
        match completion.unwrap_or(claims::ImageCompletion::Unknown) {
            claims::ImageCompletion::Recorded => {
                report.record(outcome);
                report.outcomes.push(ImageItemResult {
                    attachment_id: attachment.id,
                    status: status.into(),
                    code: item_code.map(str::to_owned),
                });
            }
            claims::ImageCompletion::Existing(row) => report.outcomes.push(existing_item(&row)),
            claims::ImageCompletion::Unknown => {
                outcome = ImageOutcome::Unknown;
                report.record(outcome);
                report.outcomes.push(ImageItemResult {
                    attachment_id: attachment.id,
                    status: "unknown".into(),
                    code: None,
                });
            }
        }
        match outcome {
            ImageOutcome::Unknown => stop = Some("after_unknown"),
            ImageOutcome::Failed("billing") => stop = Some("billing"),
            ImageOutcome::Skipped("deadline") => stop = Some("deadline"),
            _ => {}
        }
    }
    super::audit(
        state,
        ctx.owner,
        "nyxbot_channel_image_delivery",
        serde_json::json!({
            "conversation_id": ctx.conversation_id,
            "turn_id": ctx.turn_id,
            "channel_agent_id": ctx.channel.id,
            "transport": ctx.channel.transport,
            "attempted": report.attempted,
            "sent": report.sent,
            "failed": report.failed,
            "unknown": report.unknown,
            "skipped": report.skipped,
        }),
    )
    .await;
    Ok(report)
}

async fn send_one(
    state: &AppState,
    ctx: &ImageSendContext<'_>,
    attachment: MaterializedAttachment,
) -> AppResult<Option<&'static str>> {
    // Keep this multi-stage delivery future on the heap. Its callers include
    // Tokio workers and nested image/automation delivery on normal stacks.
    Box::pin(async move {
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
                Ok((result.delivered_kinds.first() == Some(&MediaKind::File))
                    .then_some("as_document"))
            }
            ImageAnchor::Initiated { chat_id } => {
                let target = ctx
                    .initiated
                    .filter(|target| target.thread.id == *chat_id)
                    .ok_or_else(|| AppError::Conflict("Image post target unavailable".into()))?;
                let response =
                    super::chats::send_post_with_fence(state, target, outbound, ctx.fence).await?;
                response
                    .platform_message_id
                    .ok_or_else(media::upload_failed)?;
                Ok(response.delivered_as.as_ref().and_then(|kinds| {
                    (kinds.first().map(String::as_str) == Some("file")).then_some("as_document")
                }))
            }
        }
    })
    .await
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
                            ..Default::default()
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
    if delivery.state == "replay" {
        return ImageAnchor::NoAnchor;
    }
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
    if channel.transport == "gateway" && channel.gateway_source.is_some() {
        return ImageAnchor::NoAnchor;
    }
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
    report_or_failure_with_stop(state, ctx, plan, None).await
}

pub(super) async fn report_or_failure_with_stop(
    state: &AppState,
    ctx: ImageSendContext<'_>,
    plan: &ImageSendPlan,
    stop: Option<&'static str>,
) -> ImageReport {
    let app_visible = ctx.audience.owner_private;
    let automation = matches!(ctx.anchor, ImageAnchor::Initiated { .. });
    let result = match stop {
        None => send_turn_images(state, ctx, plan).await,
        Some(reason) => Box::pin(send_turn_images_with_stop(state, ctx, plan, Some(reason))).await,
    };
    match result {
        Ok(report) => report,
        Err(error) => {
            tracing::warn!(
                code = error.error_code(),
                "Tool image delivery preparation failed"
            );
            let mut seen = std::collections::HashSet::new();
            let outcome = classify_outcome(&error);
            let (status, code) = match outcome {
                ImageOutcome::Unknown => ("unknown", None),
                ImageOutcome::Failed(code) | ImageOutcome::Skipped(code) => {
                    ("failed", Some(code.to_owned()))
                }
                ImageOutcome::Sent => unreachable!(),
            };
            let outcomes: Vec<_> = plan
                .attachments
                .iter()
                .filter(|attachment| {
                    engine::chat_tool_image(attachment) && seen.insert(attachment.id.as_str())
                })
                .take(engine::MAX_TURN_ATTACHMENTS)
                .map(|attachment| ImageItemResult {
                    attachment_id: attachment.id.clone(),
                    status: status.into(),
                    code: code.clone(),
                })
                .collect();
            let count = outcomes.len() as u32;
            ImageReport {
                outcomes,
                attempted: count,
                failed: if status == "failed" { count } else { 0 },
                unknown: if status == "unknown" { count } else { 0 },
                app_visible,
                automation,
                ..Default::default()
            }
        }
    }
}

pub fn append_failure(text: &str, report: &ImageReport) -> String {
    let note = failure_note(report);
    format!("{text}{note}")
}

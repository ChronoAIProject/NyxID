//! Late source-authorized media ingestion. Admission and queueing never fetch provider bytes.
mod binding;
pub mod guest;
pub mod queue;
#[cfg(test)]
pub(crate) mod test_hooks;
#[cfg(test)]
mod tests;

use super::{assistant_upload_service as uploads, channel_media_authority as authority};
use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::{
        assistant_channel_media::{MediaBound, MediaOutcome, UploadSource},
        assistant_conversation::{AssistantConversation, COLLECTION_NAME as CONVERSATIONS},
        assistant_message::{AssistantMessage, COLLECTION_NAME as MESSAGES},
        nyxbot_channel::{EVENTS_COLLECTION_NAME as EVENTS, NyxbotChannel, NyxbotEvent},
    },
};
pub(crate) use binding::{finalize_in_session, media_finalize};
use mongodb::{
    Database,
    bson::{self, doc},
};
use std::{collections::HashMap, sync::Arc, time::Duration};

pub const FAILED_NOTE: &str =
    "The attachments in this message could not be processed; please ask the owner to resend them.";
pub const GUEST_NOTE: &str = "attachments from guests are not shared with the assistant";

#[cfg(test)]
pub(crate) static TEST_ADAPTERS: std::sync::LazyLock<
    std::sync::Mutex<
        std::collections::HashMap<String, Arc<dyn super::channel_platform::PlatformAdapter>>,
    >,
> = std::sync::LazyLock::new(Default::default);

fn adapter(
    _state: &AppState,
    bot: &crate::models::channel_bot::ChannelBot,
) -> AppResult<Arc<dyn super::channel_platform::PlatformAdapter>> {
    #[cfg(test)]
    {
        TEST_ADAPTERS
            .lock()
            .unwrap()
            .get(&bot.id)
            .cloned()
            .ok_or_else(|| AppError::Internal("Channel media test adapter missing".into()))
    }
    #[cfg(not(test))]
    {
        super::channel_adapters::resolve_adapter(&bot.platform, &_state.token_exchange_cache)
            .map(Arc::from)
    }
}

pub async fn ensure_indexes(db: &Database) -> mongodb::error::Result<()> {
    use mongodb::{IndexModel, options::IndexOptions};
    db.collection::<bson::Document>(crate::models::assistant_upload::COLLECTION_NAME)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"user_id":1,"source.event_key":1,"source.index":1,"source.embedded":1})
                .options(
                    IndexOptions::builder()
                        .name("assistant_channel_source_unique".to_owned())
                        .unique(true)
                        .partial_filter_expression(doc! {"source.event_key":{"$type":"string"}})
                        .build(),
                )
                .build(),
        )
        .await?;
    db.collection::<bson::Document>(MESSAGES)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"media_pending":1,"created_at":1})
                .options(
                    IndexOptions::builder()
                        .partial_filter_expression(doc! {"media_pending":{"$exists":true}})
                        .build(),
                )
                .build(),
        )
        .await?;
    db.collection::<bson::Document>(MESSAGES)
        .create_index(
            IndexModel::builder()
                .keys(doc! {"user_id":1,"conversation_id":1,"media_queue_notice_id":1})
                .options(
                    IndexOptions::builder()
                        .name("assistant_media_queue_notice_unique".to_owned())
                        .unique(true)
                        .partial_filter_expression(
                            doc! {"media_queue_notice_id":{"$type":"string"}},
                        )
                        .build(),
                )
                .build(),
        )
        .await?;
    Ok(())
}

async fn message(
    db: &Database,
    row: &AssistantConversation,
    turn: &str,
) -> AppResult<Option<AssistantMessage>> {
    Ok(db
        .collection::<AssistantMessage>(MESSAGES)
        .find_one(doc! {
            "user_id":&row.user_id,"conversation_id":&row.id,"turn_id":turn,
            "role":{"$ne":"assistant"},"media_event_ids.0":{"$exists":true}
        })
        .await?)
}

fn outcome(
    event: &NyxbotEvent,
    index: Option<usize>,
    embedded: bool,
    status: &str,
    upload_id: Option<String>,
) -> MediaOutcome {
    MediaOutcome {
        event_id: event.id.clone(),
        event_key: event
            .media
            .as_ref()
            .map(|m| m.key.clone())
            .unwrap_or_default(),
        index,
        embedded,
        status: status.into(),
        upload_id,
    }
}

type ItemOrder = HashMap<(String, usize, bool), (usize, usize)>;

fn source_filter(row: &AssistantConversation, event: &str, turn: Option<&str>) -> bson::Document {
    let mut filter = doc! {"_id":event,"conversation_id":&row.id};
    if let Some(origin) = super::org_channel_service::origin(row) {
        filter.insert("channel_id", &origin.nyxbot_channel_id);
        filter.insert("delivery.origin.org_execution.actor_id", &row.user_id);
    } else {
        filter.insert("user_id", &row.user_id);
    }
    if let Some(turn) = turn {
        filter.insert("turn_id", turn);
    }
    filter
}

fn remember_order(order: &mut ItemOrder, event: &NyxbotEvent) {
    if let Some(media) = &event.media {
        for (position, item) in media.items.iter().enumerate() {
            order.insert(
                (event.id.clone(), item.index, item.embedded),
                (position + 1, media.declared),
            );
        }
    }
}

pub(crate) async fn pending_receipt(
    db: &Database,
    ids: &[String],
    status: &str,
) -> AppResult<MediaBound> {
    let mut receipt = MediaBound {
        event_ids: ids.to_vec(),
        ok: Vec::new(),
        failed: Vec::new(),
        context: String::new(),
    };
    let mut order = ItemOrder::new();
    for id in ids {
        if let Some(event) = db
            .collection::<NyxbotEvent>(EVENTS)
            .find_one(doc! {"_id":id})
            .await?
            && let Some(media) = &event.media
        {
            remember_order(&mut order, &event);
            for item in &media.items {
                receipt.failed.push(outcome(
                    &event,
                    Some(item.index),
                    item.embedded,
                    status,
                    None,
                ));
            }
            if media.declared > media.items.len() {
                receipt.failed.push(outcome(
                    &event,
                    None,
                    true,
                    &format!(
                        "{} more embedded media omitted",
                        media.declared - media.items.len()
                    ),
                    None,
                ));
            }
        } else {
            receipt.failed.push(MediaOutcome {
                event_id: id.clone(),
                event_key: String::new(),
                index: None,
                embedded: false,
                status: status.into(),
                upload_id: None,
            });
        }
    }
    receipt.context = context(&receipt, &order);
    Ok(receipt)
}

fn context(receipt: &MediaBound, order: &ItemOrder) -> String {
    let mut text = String::new();
    let originals = receipt
        .ok
        .iter()
        .filter(|item| item.status == "stored_original")
        .count();
    if originals > 0 {
        text.push_str(&format!("{originals} attachment(s) are saved for download in this chat, but their contents cannot be read by the assistant."));
    }
    if receipt.ok.is_empty() && !receipt.failed.is_empty() {
        text.push_str(FAILED_NOTE);
    }
    for item in receipt.failed.iter().take(100) {
        if item.index.is_none() && item.status.ends_with(" more embedded media omitted") {
            text.push_str(&format!("\n{}.", item.status));
            continue;
        }
        let phrase = match item.status.as_str() {
            "too_large" => "too large",
            "source_unavailable" => "source unavailable",
            "unsupported_or_invalid" => "unsupported or invalid file",
            "rate_limited" => "upload limit reached",
            "fetch_or_store_failed" => "download or storage failed",
            "timeout" | "not_processed: timeout" => "timed out",
            "not_processed: stopped" => "turn stopped",
            "not_processed: expired" => "source expired",
            _ => "processing failed",
        };
        let ordinal = item
            .index
            .and_then(|index| order.get(&(item.event_id.clone(), index, item.embedded)));
        if let Some((position, total)) = ordinal {
            text.push_str(&format!(
                "\nAttachment {position} of {total} could not be read ({phrase})."
            ));
        } else {
            text.push_str(&format!("\nAn attachment could not be read ({phrase})."));
        }
    }
    text
}

async fn request(
    db: &Database,
    row: &AssistantConversation,
    event: &NyxbotEvent,
) -> AppResult<authority::MediaRequest> {
    if super::org_channel_service::event_actor(event) != row.user_id {
        return Err(AppError::NotFound("Channel media unavailable".into()));
    }
    let mut channel = db
        .collection::<NyxbotChannel>(crate::models::nyxbot_channel::COLLECTION_NAME)
        .find_one(doc! {"_id":&event.channel_id,"user_id":&event.user_id})
        .await?
        .ok_or_else(|| AppError::NotFound("Channel media unavailable".into()))?;
    let source = db
        .collection::<crate::models::channel_message::ChannelMessage>(
            crate::models::channel_message::COLLECTION_NAME,
        )
        .find_one(doc! {"_id":&event.relay_message_id,"direction":"inbound"})
        .await?
        .ok_or_else(|| AppError::NotFound("Channel media unavailable".into()))?;
    if let Some(captured) = event.media.as_ref().and_then(|m| m.authority.as_ref()) {
        channel.route_api_key_id = captured.route_api_key_id.clone();
        channel.route_id = captured.route_id.clone();
        channel.agent_id = captured.agent_id.clone();
        channel.bot_owner_id = captured.bot_owner_id.clone();
        channel.follow_binding_generation = captured.follow_binding_generation;
    }
    Ok(authority::MediaRequest {
        channel,
        source_message_id: source.id,
        chat_id: source
            .platform_conversation_id
            .ok_or(AppError::ChannelAttachmentNotFound)?,
        sender_id: source.sender_platform_id.unwrap_or_default(),
        native_thread_id: source.thread_id,
        origin: event.delivery.as_ref().map(|d| d.origin.clone()),
        conversation_id: Some(row.id.clone()),
        agent_id: row.agent_id.clone(),
    })
}

/// The returned bytes remain covered until extraction/encryption finishes.
async fn fetch_item(
    state: &AppState,
    row: &AssistantConversation,
    event: &NyxbotEvent,
    item: &crate::models::nyxbot_channel::EventMediaItem,
) -> AppResult<(String, bool)> {
    let request = request(&state.db, row, event).await?;
    let guest_media = event.media.as_ref().is_some_and(|m| m.guest);
    if guest_media != row.guest_turn {
        return Err(AppError::ChannelAttachmentNotFound);
    }
    let maximum = state
        .config
        .channel_media_max_bytes
        .min(super::attachment_extraction::MAX_BYTES as u64) as usize;
    let initial = uploads::byte_budget::initial_channel_limit(item.size_bytes, maximum);
    for (attempt, limit) in [initial, maximum].into_iter().enumerate() {
        uploads::admit(&state.db, &row.user_id).await?;
        let reservation = uploads::byte_budget::reserve(limit).await?;
        if guest_media {
            let live = super::assistant_nyxagent::get(&state.db, &row.user_id, &row.id).await?;
            guest::validate_event(&state.db, &live, event).await?;
        }
        let authorized = Box::pin(authority::resolve(&state.db, &request)).await?;
        if authorized.guest != guest_media {
            return Err(AppError::ChannelAttachmentNotFound);
        }
        let selector = if item.embedded {
            &authorized.source.embedded_attachments
        } else {
            &authorized.source.attachments
        }
        .get(item.index)
        .ok_or(AppError::ChannelAttachmentNotFound)?;
        if selector.size_bytes.is_some_and(|n| n > maximum as u64) {
            return Err(AppError::ChannelMediaTooLarge);
        }
        let adapter = adapter(state, &authorized.bot)?;
        let fetched = super::channel_media_fetch::fetch(
            &state.db,
            &state.encryption_keys,
            &state.http_client,
            &authorized.bot,
            selector,
            adapter.as_ref(),
            reservation.payload_limit as u64,
        )
        .await;
        match fetched {
            Err(AppError::ChannelMediaTooLarge) if attempt == 0 && limit < maximum => {
                drop(reservation);
                continue;
            }
            Err(error) => return Err(error),
            Ok(fetched) => {
                if guest_media {
                    let live =
                        super::assistant_nyxagent::get(&state.db, &row.user_id, &row.id).await?;
                    guest::validate_event(&state.db, &live, event).await?;
                    if !Box::pin(authority::resolve(&state.db, &request))
                        .await?
                        .guest
                    {
                        return Err(AppError::ChannelAttachmentNotFound);
                    }
                }
                let name = selector.filename.as_deref().unwrap_or("attachment");
                let bytes = fetched.bytes.to_vec();
                drop(fetched);
                let source = UploadSource {
                    guest: guest_media.then(|| {
                        crate::models::assistant_channel_media::GuestSource {
                            version: 1,
                            channel_id: event.channel_id.clone(),
                            event_id: event.id.clone(),
                            sender_id: request.sender_id.clone(),
                        }
                    }),
                    turn_id: row
                        .active_turn
                        .as_ref()
                        .expect("claimed turn")
                        .turn_id
                        .clone(),
                    event_key: event.media.as_ref().expect("media event").key.clone(),
                    index: item.index,
                    embedded: item.embedded,
                    declared: event.media.as_ref().expect("media event").declared,
                };
                let upload = if authority::expanded_relay(&authorized.bot.platform) || guest_media {
                    // Custody or membership can change while the network is in flight.
                    Box::pin(authority::resolve(&state.db, &request)).await?;
                    Box::pin(uploads::upload_channel_original_authorized(
                        &state.db,
                        &state.encryption_keys,
                        &row.user_id,
                        &row.id,
                        name,
                        bytes,
                        source,
                        selector.mime_type.as_deref(),
                    ))
                    .await?
                } else {
                    Box::pin(uploads::upload_channel_authorized(
                        &state.db,
                        &state.encryption_keys,
                        &row.user_id,
                        &row.id,
                        name,
                        bytes,
                        source,
                    ))
                    .await?
                };

                drop(reservation);
                let original_only = upload.content_type == "application/octet-stream"
                    || upload.content_type.starts_with("audio/")
                    || upload.content_type.starts_with("video/");
                return Ok((upload.id, original_only));
            }
        }
    }
    Err(AppError::ChannelMediaTooLarge)
}

async fn collect(
    state: &AppState,
    row: &AssistantConversation,
    message: &AssistantMessage,
) -> AppResult<MediaBound> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(90);
    let mut order = ItemOrder::new();
    let mut rechecks = Vec::new();
    let mut receipt = MediaBound {
        event_ids: message.media_event_ids.clone(),
        ok: Vec::new(),
        failed: Vec::new(),
        context: String::new(),
    };
    for id in &message.media_event_ids {
        let Some(event) = state
            .db
            .collection::<NyxbotEvent>(EVENTS)
            .find_one(source_filter(row, id, Some(&message.turn_id)))
            .await?
        else {
            receipt.failed.push(MediaOutcome {
                event_id: id.clone(),
                event_key: String::new(),
                index: None,
                embedded: false,
                status: "source_unavailable".into(),
                upload_id: None,
            });
            continue;
        };
        let Some(media) = &event.media else { continue };
        remember_order(&mut order, &event);
        for item in media.items.iter().take(10) {
            let fetched = if tokio::time::Instant::now() >= deadline {
                None
            } else {
                let result = tokio::time::timeout_at(
                    deadline,
                    Box::pin(fetch_item(state, row, &event, item)),
                )
                .await
                .ok();
                // A fetch's own timer may wake in the same poll as the budget.
                // Once the budget is exhausted its terminal class is timeout.
                result.filter(|_| tokio::time::Instant::now() < deadline)
            };
            match fetched {
                Some(Ok((id, original_only))) => receipt.ok.push(outcome(
                    &event,
                    Some(item.index),
                    item.embedded,
                    if original_only {
                        "stored_original"
                    } else {
                        "bound"
                    },
                    Some(id),
                )),
                Some(Err(error)) => {
                    let status = match error {
                        AppError::ChannelMediaTooLarge => "too_large",
                        AppError::NotFound(_)
                        | AppError::Forbidden(_)
                        | AppError::SecurityReadinessUnavailable => "source_unavailable",
                        AppError::BadRequest(_) => "unsupported_or_invalid",
                        AppError::RateLimited => "rate_limited",
                        _ => "fetch_or_store_failed",
                    };
                    receipt.failed.push(outcome(
                        &event,
                        Some(item.index),
                        item.embedded,
                        status,
                        None,
                    ));
                }
                None => receipt.failed.push(outcome(
                    &event,
                    Some(item.index),
                    item.embedded,
                    "timeout",
                    None,
                )),
            }
        }
        if media.declared > media.items.len() {
            receipt.failed.push(outcome(
                &event,
                None,
                true,
                &format!(
                    "{} more embedded media omitted",
                    media.declared - media.items.len()
                ),
                None,
            ));
        }
        if authority::expanded_relay(&media.platform) {
            rechecks.push(event);
        }
    }
    // Revalidate new adapters immediately before binding, after every fetch and
    // extraction has completed. Unbound uploads are purged by settlement.
    for event in rechecks {
        let valid = match request(&state.db, row, &event).await {
            Ok(request) => Box::pin(authority::resolve(&state.db, &request))
                .await
                .is_ok(),
            Err(_) => false,
        };
        if !valid {
            let (kept, refused): (Vec<_>, Vec<_>) = receipt
                .ok
                .into_iter()
                .partition(|item| item.event_id != event.id);
            receipt.ok = kept;
            receipt.failed.extend(refused.into_iter().map(|mut item| {
                item.status = "source_unavailable".into();
                item.upload_id = None;
                item
            }));
        }
    }
    // Revocation during extraction/storage must not bind guest bytes. The
    // existing finalizer removes unbound roots, including late unknown commits.
    if row.guest_turn {
        let mut kept = Vec::new();
        for item in receipt.ok.drain(..) {
            let authorized = async {
                let live = super::assistant_nyxagent::get(&state.db, &row.user_id, &row.id).await?;
                let event = state
                    .db
                    .collection::<NyxbotEvent>(EVENTS)
                    .find_one(source_filter(row, &item.event_id, Some(&message.turn_id)))
                    .await?
                    .ok_or(AppError::ChannelAttachmentNotFound)?;
                if event.media.as_ref().is_some_and(|m| m.guest) {
                    guest::validate_event(&state.db, &live, &event).await?;
                    if !Box::pin(authority::resolve(
                        &state.db,
                        &request(&state.db, &live, &event).await?,
                    ))
                    .await?
                    .guest
                    {
                        return Err(AppError::ChannelAttachmentNotFound);
                    }
                }
                Ok::<_, AppError>(())
            }
            .await;
            if authorized.is_ok() {
                kept.push(item);
            } else {
                receipt.failed.push(MediaOutcome {
                    status: "source_unavailable".into(),
                    upload_id: None,
                    ..item
                });
            }
        }
        receipt.ok = kept;
    }
    receipt.context = context(&receipt, &order);
    if row.guest_turn && !guest::enabled(&state.db, &row.user_id).await? {
        receipt.context.push('\n');
        receipt.context.push_str(GUEST_NOTE);
    }
    // Preserve the parser's cap signal even when input was independently rendered.
    if message.text.contains("[post truncated]") {
        receipt.context.push_str("\n[post truncated]");
    }
    Ok(receipt)
}

// The collection deadline is an item failure, so successful siblings still bind
// and the model receives the failure context. Bind is outside that deadline.
// Database/transport failures before a receipt exists may fail the turn; shared
// settlement then records not_processed: failed (or stopped on a missed fence).
pub async fn ingest_and_bind(state: &AppState, row: &AssistantConversation) -> AppResult<String> {
    let Some(turn) = row.active_turn.as_ref() else {
        return Ok(String::new());
    };
    let Some(message) = message(&state.db, row, &turn.turn_id).await? else {
        return Ok(String::new());
    };
    if let Some(receipt) = &message.media_bound {
        binding::reconcile(&state.db, receipt).await?;
        return Ok(receipt.context.clone());
    }
    if message.media_pending.is_none() {
        return Ok(String::new());
    }
    let receipt = Box::pin(collect(state, row, &message)).await?;
    let receipt = Box::pin(binding::bind(&state.db, &message, &receipt)).await?;
    binding::reconcile(&state.db, &receipt).await?;
    Ok(receipt.context)
}

//! Explicit owner-turn image selection. Inventory is never delivery intent.
//! Bytes remain in the existing encrypted store; channel storage stays metadata-only.
use base64::Engine;
use bson::doc;
use chrono::Utc;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::{
    assistant_nyxagent as engine,
    channel_platform::{MediaKind, OutboundAttachment, OutboundMediaSource, PlatformAdapter},
};
use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::{
        assistant_attachment::{AssistantAttachment, COLLECTION_NAME as ATTACHMENTS},
        assistant_conversation::{AssistantConversation, COLLECTION_NAME as CONVERSATIONS},
        assistant_message::{AssistantMessage, COLLECTION_NAME as MESSAGES},
        channel_message::ChannelMessage,
        nyxbot_channel::{COLLECTION_NAME as CHANNELS, NyxbotChannel},
    },
};

pub const MAX_CHANNEL_REPLY_CHARS: usize = 8000;

pub fn unavailable() -> AppError {
    AppError::NotFound("Reply image unavailable".into())
}

/// Metadata-only observation of one NyxID-owned send, with no provider-specific
/// inference. A successful adapter call confirms acceptance, not recipient viewing.
#[derive(Debug, serde::Serialize)]
pub struct DeliveryReport {
    status: &'static str,
    images_requested: usize,
    images_omitted: usize,
    message: &'static str,
}

impl DeliveryReport {
    pub fn status(&self) -> &'static str {
        self.status
    }

    pub fn observed(confirmed: bool, requested: usize, omitted: usize) -> Self {
        let (status, message) = if !confirmed {
            (
                "not_confirmed",
                "Delivery not confirmed; parts may have been delivered. NyxID will not retry this send.",
            )
        } else if omitted > 0 {
            (
                "images_unavailable",
                "The text and image-unavailability notice were sent; selected images were not sent.",
            )
        } else {
            (
                "confirmed",
                "The send succeeded; recipient viewing is not confirmed.",
            )
        };
        Self {
            status,
            images_requested: requested,
            images_omitted: omitted,
            message,
        }
    }
}

/// Observe the existing call once. Text-only sends retain their original errors;
/// selected-image sends never imply a complete success after any call failure.
pub fn observe<T>(
    result: AppResult<T>,
    requested: usize,
    omitted: usize,
) -> AppResult<(Option<T>, Option<DeliveryReport>)> {
    if requested == 0 {
        return result.map(|value| (Some(value), None));
    }
    let report = DeliveryReport::observed(result.is_ok(), requested, omitted);
    Ok((result.ok(), Some(report)))
}

pub async fn relay_attachments(
    state: &AppState,
    original: &ChannelMessage,
    route_key: &str,
    attachments: Vec<OutboundAttachment<super::channel_platform::RelayMediaSource>>,
) -> AppResult<Vec<OutboundAttachment>> {
    use super::channel_platform::{AssistantImageSource, RelayMediaSource};
    let mut resolved = Vec::with_capacity(attachments.len());
    for mut attachment in attachments {
        if let RelayMediaSource::Assistant(AssistantImageSource::AssistantImage {
            conversation_id,
            turn_id,
            image_ref,
        }) = &attachment.source
        {
            if attachment.kind != MediaKind::Image {
                return Err(unavailable());
            }
            let image = relay_source(
                state,
                original,
                route_key,
                conversation_id,
                turn_id,
                image_ref,
            )
            .await?;
            attachment.source = RelayMediaSource::Media(image.source);
            attachment.mime_type = image.mime_type;
        }
        resolved.push(attachment.try_into()?);
    }
    Ok(resolved)
}

pub fn stored_note(image_ref: &str) -> String {
    format!(
        "Stored for the owner's NyxID conversation. Image handle: {image_ref}. No chat receives this image unless you explicitly select it with nyxid__set_reply_images, or include it in image_refs on nyxid__post_to_chat. NyxID can show it to you after this response when image input is supported. Do not describe its contents before viewing it."
    )
}

pub fn refs(args: &Value, required: bool) -> AppResult<Vec<String>> {
    let Some(value) = args.get("image_refs") else {
        return if required {
            Err(unavailable())
        } else {
            Ok(Vec::new())
        };
    };
    let refs: Vec<String> = serde_json::from_value(value.clone()).map_err(|_| unavailable())?;
    if refs.len() > engine::MAX_TURN_ATTACHMENTS
        || refs.iter().any(|id| id.is_empty() || id.len() > 64)
        || refs.iter().collect::<std::collections::HashSet<_>>().len() != refs.len()
    {
        return Err(unavailable());
    }
    Ok(refs)
}

async fn image(
    state: &AppState,
    owner: &str,
    conversation: &str,
    turn: &str,
    id: &str,
) -> AppResult<AssistantAttachment> {
    let filter = doc! {"_id":id,"user_id":owner,"conversation_id":conversation,"turn_id":turn};
    let row = state
        .db
        .collection::<AssistantAttachment>(ATTACHMENTS)
        .find_one(filter.clone())
        .await?
        .filter(|r| {
            r.origin == "tool"
                && super::mcp_service::TOOL_IMAGE_TYPES.contains(&r.content_type.as_str())
        })
        .ok_or_else(unavailable)?;
    super::assistant_upload_retention::require_available(&state.db, filter)
        .await
        .map_err(|_| unavailable())?;
    Ok(row)
}

pub async fn validate_live(
    state: &AppState,
    owner: &str,
    conversation: &str,
    expected_turn: Option<&str>,
    refs: &[String],
) -> AppResult<AssistantConversation> {
    let row = engine::get(&state.db, owner, conversation)
        .await
        .map_err(|_| unavailable())?;
    let turn = engine::live_turn(&row, Utc::now())
        .filter(|t| {
            !t.stop_requested && !row.guest_turn && Some(t.turn_id.as_str()) == expected_turn
        })
        .ok_or_else(unavailable)?;
    for id in refs {
        if !turn
            .attachments
            .iter()
            .any(|a| a.id == *id && a.origin == "tool")
        {
            return Err(unavailable());
        }
        image(state, owner, conversation, &turn.turn_id, id).await?;
    }
    Ok(row)
}

pub async fn select(
    state: &AppState,
    owner: &str,
    conversation: &str,
    expected_turn: Option<&str>,
    args: &Value,
) -> AppResult<Value> {
    let refs = refs(args, true)?;
    let row = validate_live(state, owner, conversation, expected_turn, &refs).await?;
    let turn = row.active_turn.as_ref().ok_or_else(unavailable)?;
    let changed = state.db.collection::<bson::Document>(CONVERSATIONS).update_one(
        doc! {"_id":conversation,"user_id":owner,"guest_turn":{"$ne":true},"active_turn.turn_id":&turn.turn_id,"active_turn.stop_requested":{"$ne":true}, "$expr":{"$gt":[{"$ifNull":["$active_turn.lease_expires_at",{"$add":["$active_turn.started_at",engine::ACTIVE_TURN_TTL_SECS*1000]}]}, bson::DateTime::now()]}},
        doc! {"$set":{"active_turn.reply_image_refs":&refs}},
    ).await?;
    if changed.matched_count != 1 {
        return Err(unavailable());
    }
    let capability = expected_capability(state, &row).await.unwrap_or("unknown");
    Ok(
        json!({"status":"selected_not_sent","image_refs":refs,"destination_capability":capability,"message":"Selected, not yet sent. The final reply supplies the text."}),
    )
}

async fn expected_capability(
    state: &AppState,
    row: &AssistantConversation,
) -> AppResult<&'static str> {
    use crate::models::{
        nyxbot_channel::{NyxbotThread, THREADS_COLLECTION_NAME},
        trigger_run::{COLLECTION_NAME as RUNS, TriggerRun},
        trigger_schedule::DeliverTo,
    };
    let turn = row.active_turn.as_ref().ok_or_else(unavailable)?;
    let (chat, initiated) = if let Some(run_id) = &turn.trigger_run_id {
        let run = state
            .db
            .collection::<TriggerRun>(RUNS)
            .find_one(doc! {"_id":run_id,"user_id":&row.user_id,"turn_id":&turn.turn_id})
            .await?;
        let Some(DeliverTo::Chat { chat_id }) = run.map(|r| r.deliver_to) else {
            return Ok("unknown");
        };
        (
            state
                .db
                .collection::<NyxbotThread>(THREADS_COLLECTION_NAME)
                .find_one(doc! {"_id":chat_id,"user_id":&row.user_id})
                .await?,
            true,
        )
    } else {
        let Some(origin) = turn
            .asked_from
            .as_ref()
            .or(row.channel.as_ref())
            .or(row.reply_channel.as_ref())
        else {
            return Ok("unknown");
        };
        // Native-thread routing also depends on a live bound target and gateway
        // negotiation. Do not guess from the channel transport alone.
        if origin.thread.is_some() {
            return Ok("unknown");
        }
        (state.db.collection::<NyxbotThread>(THREADS_COLLECTION_NAME).find_one(doc! {"channel_id":&origin.nyxbot_channel_id,"partition":&origin.partition,"user_id":&row.user_id}).await?,false)
    };
    let Some(chat) = chat else {
        return Ok("unknown");
    };
    let Some(channel) = state
        .db
        .collection::<NyxbotChannel>(CHANNELS)
        .find_one(doc! {"_id":&chat.channel_id,"user_id":&row.user_id})
        .await?
    else {
        return Ok("unknown");
    };
    if !initiated {
        if super::channel_turn_delivery::reply_uses_gateway(&channel, &chat) {
            return Ok(
                if chat.event_ref_ciphertext.is_some()
                    && chat
                        .event_ref_expires_at
                        .is_some_and(|expiry| expiry > Utc::now())
                {
                    "text_with_image_notice"
                } else {
                    "unknown"
                },
            );
        }
        if chat.last_message_id.is_none() {
            return Ok("unknown");
        }
    }
    let Some(address) = chat.platform_chat_id.as_deref() else {
        return Ok("unknown");
    };
    let supported = super::channel_adapters::conversation_capabilities(
        &channel.platform,
        address,
        &state.token_exchange_cache,
    )
    .media
    .outbound
    .contains(&MediaKind::Image);
    Ok(if supported {
        "images"
    } else {
        "text_with_image_notice"
    })
}

pub async fn settled(
    state: &AppState,
    owner: &str,
    conversation: &str,
    turn: &str,
) -> AppResult<Option<AssistantMessage>> {
    Ok(state
        .db
        .collection::<AssistantMessage>(MESSAGES)
        .find_one(
            doc! {"user_id":owner,"conversation_id":conversation,"turn_id":turn,"role":"assistant"},
        )
        .await?)
}

/// One resolver for all initiated sends and replies. Never accepts inventory implicitly.
pub async fn resolve(
    state: &AppState,
    owner: &str,
    conversation: &str,
    turn: &str,
    refs: &[String],
) -> AppResult<Vec<OutboundAttachment>> {
    if refs.len() > engine::MAX_TURN_ATTACHMENTS {
        return Err(unavailable());
    }
    let mut result = Vec::with_capacity(refs.len());
    for id in refs {
        let row = image(state, owner, conversation, turn, id).await?;
        let (mime, bytes) = engine::read_attachment(
            &state.db,
            &state.encryption_keys,
            owner,
            conversation,
            &row.id,
        )
        .await
        .map_err(|_| unavailable())?;
        if bytes.len() as u64 > state.config.channel_media_max_bytes
            || !super::mcp_service::image_magic_matches(&mime, &bytes)
        {
            return Err(unavailable());
        }
        result.push(OutboundAttachment {
            kind: MediaKind::Image,
            source: OutboundMediaSource::Base64 {
                data: base64::engine::general_purpose::STANDARD.encode(bytes),
            },
            filename: None,
            mime_type: Some(mime),
            caption: None,
        });
    }
    Ok(result)
}

pub fn supports_images(adapter: &dyn PlatformAdapter) -> bool {
    adapter
        .media_capabilities()
        .outbound
        .contains(&MediaKind::Image)
}

pub fn notice(text: &str, frontend: &str, conversation: &str) -> String {
    let suffix = format!(
        "\n\nThe selected images could not be shown in this chat. Owner-only NyxID thread: {}/assistant?c={conversation}",
        frontend.trim_end_matches('/')
    );
    let budget = MAX_CHANNEL_REPLY_CHARS.saturating_sub(suffix.chars().count());
    let body: String = text.chars().take(budget).collect();
    format!("{body}{suffix}")
}

/// Hash exact routing identity; interaction credentials must never be copied into transcripts.
pub fn destination(message: &ChannelMessage) -> String {
    let value = json!([
        message.channel_bot_id,
        message.conversation_id,
        message.platform,
        message.platform_conversation_id,
        message.thread_id,
        message
            .thread_context
            .as_ref()
            .map(|t| (&t.kind, &t.root_id, &t.native_thread_id))
    ]);
    hex::encode(Sha256::digest(value.to_string().as_bytes()))
}

pub async fn bind(
    state: &AppState,
    message: &AssistantMessage,
    source: &ChannelMessage,
) -> AppResult<()> {
    if message.reply_image_refs.is_empty() {
        return Ok(());
    }
    state.db.collection::<bson::Document>(MESSAGES).update_one(
        doc! {"_id":&message.id,"user_id":&message.user_id,"reply_image_refs":&message.reply_image_refs},
        doc! {"$addToSet":{"reply_image_destinations":destination(source)}},
    ).await?;
    Ok(())
}

/// Initiated results bind the addressed chat, without a reply/thread anchor.
pub async fn bind_initiated(
    state: &AppState,
    owner: &str,
    selection: Selection<'_>,
    target: &crate::models::channel_conversation::ChannelConversation,
) -> AppResult<()> {
    if selection.refs.is_empty() {
        return Ok(());
    }
    let digest = hex::encode(Sha256::digest(
        json!([
            target.channel_bot_id,
            target.id,
            target.platform,
            target.platform_conversation_id,
            null,
            null
        ])
        .to_string()
        .as_bytes(),
    ));
    state.db.collection::<bson::Document>(MESSAGES).update_one(doc! {
        "user_id":owner,"conversation_id":selection.conversation,"turn_id":selection.turn,"role":"assistant","reply_image_refs":selection.refs,
    },doc! {"$addToSet":{"reply_image_destinations":digest}}).await?;
    Ok(())
}

/// Gateway references remain opaque. Bind only an existing exact relay source;
/// an event without that source cannot grant the reserved relay reader access.
pub async fn bind_original(
    state: &AppState,
    message: &AssistantMessage,
    channel: &NyxbotChannel,
    source_id: &str,
) -> AppResult<()> {
    if message.reply_image_refs.is_empty() {
        return Ok(());
    }
    let source=state.db.collection::<ChannelMessage>(crate::models::channel_message::COLLECTION_NAME).find_one(doc! {
        "_id":source_id,"channel_bot_id":&channel.channel_bot_id,"conversation_id":&channel.route_id,"agent_api_key_id":&channel.route_api_key_id,"direction":"inbound",
    }).await?;
    if let Some(source) = source {
        bind(state, message, &source).await?;
    }
    Ok(())
}

/// Reserved reader for CMA. The caller has already authenticated the relay credential
/// and resolved its inbound message + live route. Never accepts a caller-supplied owner.
pub async fn relay_source(
    state: &AppState,
    source: &ChannelMessage,
    route_key: &str,
    conversation: &str,
    turn: &str,
    image_ref: &str,
) -> AppResult<OutboundAttachment> {
    let channel = state.db.collection::<NyxbotChannel>(CHANNELS).find_one(doc! {
        "channel_bot_id":&source.channel_bot_id,"route_id":&source.conversation_id,"route_api_key_id":route_key,"status":"active",
    }).await?.ok_or_else(unavailable)?;
    if channel.bot_owner_id.as_deref().unwrap_or(&channel.user_id) != source.user_id
        || source
            .platform_conversation_id
            .as_deref()
            .is_none_or(|id| id.is_empty() || id == "*")
        || source.platform != channel.platform
    {
        return Err(unavailable());
    }
    // Linking person owns assistant images even when the route key is org-owned.
    super::channel_thread_follow_service::access(&state.db, &channel.user_id, &channel.id)
        .await
        .map_err(|_| unavailable())?;
    let message = settled(state, &channel.user_id, conversation, turn)
        .await?
        .ok_or_else(unavailable)?;
    if !message.reply_image_refs.iter().any(|id| id == image_ref)
        || !message
            .reply_image_destinations
            .contains(&destination(source))
    {
        return Err(unavailable());
    }
    resolve(
        state,
        &channel.user_id,
        conversation,
        turn,
        &[image_ref.to_owned()],
    )
    .await?
    .pop()
    .ok_or_else(unavailable)
}

#[derive(Clone, Copy)]
pub struct Selection<'a> {
    pub conversation: &'a str,
    pub turn: &'a str,
    pub refs: &'a [String],
}
impl<'a> Selection<'a> {
    pub fn from_message(message: &'a AssistantMessage) -> Self {
        Self {
            conversation: &message.conversation_id,
            turn: &message.turn_id,
            refs: &message.reply_image_refs,
        }
    }
}

/// Check capability before composing. Retention races degrade honestly as well.
pub async fn prepare(
    state: &AppState,
    owner: &str,
    text: &str,
    selection: Option<Selection<'_>>,
    supported: bool,
) -> (String, Vec<OutboundAttachment>, usize) {
    let Some(selection) = selection.filter(|s| !s.refs.is_empty()) else {
        return (text.to_owned(), Vec::new(), 0);
    };
    if supported
        && let Ok(images) = resolve(
            state,
            owner,
            selection.conversation,
            selection.turn,
            selection.refs,
        )
        .await
    {
        return (text.to_owned(), images, 0);
    }
    (
        notice(text, &state.config.frontend_url, selection.conversation),
        Vec::new(),
        selection.refs.len(),
    )
}

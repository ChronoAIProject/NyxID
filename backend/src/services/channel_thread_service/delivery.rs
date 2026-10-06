use super::*;
use crate::models::channel_message::{COLLECTION_NAME as MESSAGES, ChannelMessage};
use crate::models::channel_thread::{ThreadAddress, ThreadSenderKind};
use crate::services::channel_platform::{BotCredentials, OutboundReply};

/// Every accepted component is recorded immediately. On a later failure the
/// caller must not retry the whole reply or send a second error notice.
pub struct ThreadDelivery {
    pub message_ids: Vec<String>,
    pub error: Option<crate::errors::AppError>,
}

/// No target can arrive from agent reply JSON. This path is dormant until the
/// follow service explicitly resolves one after admission.
pub async fn send_reply(
    db: &mongodb::Database,
    adapter: &dyn PlatformAdapter,
    bot: &ChannelBot,
    credentials: &BotCredentials<'_>,
    target: &ThreadReplyTarget,
    reply: &crate::services::channel_platform::OutboundReply,
) -> AppResult<ThreadDelivery> {
    let source = db
        .collection::<ChannelMessage>(MESSAGES)
        .find_one(doc! {"_id": &target.message_id})
        .await?
        .ok_or_else(unavailable)?;
    if !target.matches(bot, &source, &target.facts.chat_id)
        || adapter.platform_id() != target.platform
    {
        return Err(unavailable());
    }
    let components = components(reply)?;
    let http = resolution::client()?;
    let mut result = ThreadDelivery {
        message_ids: Vec::new(),
        error: None,
    };
    for part in components {
        let sent = async {
            let eligible = if let Some((owner, origin, conversation)) = &target.admitted {
                crate::services::channel_thread_follow_service::validate_delivery(
                    db,
                    owner,
                    origin,
                    conversation,
                )
                .await?;
                resolution::eligible_source_live(db, bot, &source).await?
            } else {
                resolution::eligible_source(db, bot, &source).await?
            };
            if !eligible {
                return Err(unavailable());
            }
            adapter
                .send_bound_reply_outcome(
                    db,
                    &http,
                    bot,
                    &source,
                    credentials,
                    &target.facts.chat_id,
                    &part,
                    Some(target),
                )
                .await?
                .into_result()
        }
        .await;
        let id = match sent {
            Ok(Some(id)) if valid_id(&id) => id,
            Ok(_) => {
                result.error = Some(unavailable());
                break;
            }
            Err(error) => {
                result.error = Some(error);
                break;
            }
        };
        // Preserve known acceptance even if recording fails. Never retry sends.
        result.message_ids.push(id.clone());
        let mut facts = target.facts.clone();
        facts.message_id = id.clone();
        facts.parent_message_id = Some(target.facts.message_id.clone());
        facts.sender_kind = ThreadSenderKind::Bot;
        facts.address = ThreadAddress::NotAddressed;
        let now = chrono::Utc::now();
        let mut record = source.clone();
        record.id = uuid::Uuid::new_v4().to_string();
        record.direction = "outbound".into();
        record.platform_message_id = Some(id);
        record.sender_platform_id = Some(bot.platform_bot_id.clone());
        record.sender_display_name = None;
        record.attachments.clear();
        record.activity = None;
        record.platform_send = None;
        record.thread_id = None;
        record.thread_context = Some(facts);
        record.content_type = part
            .attachments
            .first()
            .map_or("text", |a| a.kind.as_str())
            .into();
        record.reply_to_message_id = Some(source.id.clone());
        record.reply_to_platform_message_id = source.platform_message_id.clone();
        record.platform_reply_message_id = None;
        record.callback_status = None;
        record.callback_http_status = None;
        record.created_at = now;
        record.updated_at = Some(now);
        if let Err(error) = db
            .collection::<ChannelMessage>(MESSAGES)
            .insert_one(record)
            .await
        {
            result.error = Some(error.into());
            break;
        }
    }
    Ok(result)
}

fn components(reply: &OutboundReply) -> AppResult<Vec<OutboundReply>> {
    if reply.attachments.len() > 64 {
        return Err(unavailable());
    }
    let mut parts = Vec::new();
    let mut push_text = |text: &str, metadata: Option<serde_json::Value>| -> AppResult<()> {
        let mut rest = text;
        while !rest.is_empty() {
            if parts.len() >= 64 {
                return Err(unavailable());
            }
            let mut end = rest.len().min(2000);
            while !rest.is_char_boundary(end) {
                end -= 1;
            }
            parts.push(OutboundReply {
                text: Some(rest[..end].into()),
                attachments: Vec::new(),
                reply_to_platform_message_id: None,
                metadata: metadata.clone(),
            });
            rest = &rest[end..];
        }
        Ok(())
    };
    push_text(
        reply.text.as_deref().unwrap_or_default(),
        reply.metadata.clone(),
    )?;
    for attachment in &reply.attachments {
        if let Some(caption) = &attachment.caption {
            push_text(caption, None)?;
        }
    }
    if parts.is_empty() && reply.metadata.is_some() {
        parts.push(OutboundReply {
            text: None,
            attachments: Vec::new(),
            reply_to_platform_message_id: None,
            metadata: reply.metadata.clone(),
        });
    }
    for attachment in &reply.attachments {
        let mut attachment = attachment.clone();
        attachment.caption = None;
        parts.push(OutboundReply {
            text: None,
            attachments: vec![attachment],
            reply_to_platform_message_id: None,
            metadata: None,
        });
    }
    if parts.is_empty() || parts.len() > 64 {
        return Err(unavailable());
    }
    Ok(parts)
}

use super::*;
use crate::models::channel_message::{COLLECTION_NAME as MESSAGES, ChannelMessage};
use crate::models::channel_thread::{ThreadAddress, ThreadSenderKind};
use crate::services::channel_platform::{BotCredentials, OutboundReply};
use crate::services::channel_reply_delivery::Delivery;

/// Every accepted component is recorded immediately. On a later failure the
/// caller must not retry the whole reply or send a second error notice.
pub struct ThreadDelivery {
    pub message_ids: Vec<String>,
    pub delivered_kinds: Vec<crate::services::channel_platform::MediaKind>,
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
    let components = components(reply, adapter.platform_id())?;
    let http = resolution::client()?;
    let mut result = ThreadDelivery {
        message_ids: Vec::new(),
        delivered_kinds: Vec::new(),
        error: None,
    };
    let mut delivery = Delivery::begin(db, bot, &source, components.len()).await?;
    for (index, part) in components.into_iter().enumerate() {
        let sent = Box::pin(async {
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
            delivery.attempting(db, index).await?;
            let outcome = adapter
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
                .await?;
            let kinds = outcome
                .delivered_kinds
                .clone()
                .unwrap_or_else(|| part.attachments.iter().map(|a| a.kind).collect());
            Ok((outcome.into_result()?, kinds))
        })
        .await;
        let (id, kinds) = match sent {
            Ok((Some(id), kinds)) if valid_id(&id) => (id, kinds),
            Ok(_) => {
                let error = crate::services::channel_reply_delivery::unconfirmed();
                result.error = Some(
                    delivery
                        .failed(db, index, &error)
                        .await
                        .err()
                        .unwrap_or(error),
                );
                break;
            }
            Err(error) => {
                let error = delivery
                    .failed(db, index, &error)
                    .await
                    .err()
                    .unwrap_or(error);
                crate::services::channel_bot_service::record_outbound_outcome(
                    db,
                    &bot.id,
                    Some(&error),
                )
                .await;
                result.error =
                    Some(crate::services::channel_reply_delivery::failure_after_dispatch(error));
                break;
            }
        };
        // Preserve known acceptance even if recording fails. Never retry sends.
        result.message_ids.push(id.clone());
        result.delivered_kinds.extend(&kinds);
        crate::services::channel_bot_service::record_outbound_outcome(db, &bot.id, None).await;
        let mut facts = target.facts.clone();
        facts.message_id = id.clone();
        facts.parent_message_id = Some(target.facts.message_id.clone());
        facts.sender_kind = ThreadSenderKind::Bot;
        facts.address = ThreadAddress::NotAddressed;
        let now = chrono::Utc::now();
        let mut record = source.clone();
        record.id = uuid::Uuid::new_v4().to_string();
        record.direction = "outbound".into();
        record.platform_message_id = Some(id.clone());
        record.sender_platform_id = Some(bot.platform_bot_id.clone());
        record.sender_display_name = None;
        record.attachments.clear();
        record.activity = None;
        record.platform_send = None;
        record.thread_id = None;
        record.thread_context = Some(facts);
        record.content_type = kinds.first().map_or("text", |kind| kind.as_str()).into();
        record.reply_to_message_id = Some(source.id.clone());
        record.reply_to_platform_message_id = source.platform_message_id.clone();
        record.platform_reply_message_id = None;
        record.callback_status = None;
        record.callback_http_status = None;
        record.created_at = now;
        record.updated_at = Some(now);
        if let Err(error) = db
            .collection::<ChannelMessage>(MESSAGES)
            .insert_one(&record)
            .await
        {
            result.error = Some(error.into());
            break;
        }
        if let Err(error) = delivery.accepted(db, index, vec![id], &record.id).await {
            result.error = Some(error);
            break;
        }
    }
    if result.error.is_none() {
        result.error = delivery.complete(db).await.err();
    }
    Ok(result)
}

fn components(reply: &OutboundReply, platform: &str) -> AppResult<Vec<OutboundReply>> {
    if reply.attachments.len() > 64 {
        return Err(unavailable());
    }
    let mut parts = Vec::new();
    let mut push_text = |text: &str, metadata: Option<serde_json::Value>| -> AppResult<()> {
        let text_reply = OutboundReply {
            text: Some(text.into()),
            attachments: vec![],
            reply_to_platform_message_id: None,
            metadata,
        };
        for part in crate::services::channel_reply_plan::text_parts(&text_reply, platform)? {
            if part.text.as_deref().is_none_or(str::is_empty) {
                continue;
            }
            if parts.len() >= 64 {
                return Err(unavailable());
            }
            parts.push(part);
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

#[cfg(test)]
mod tests {
    use super::*;

    fn reply(text: String) -> OutboundReply {
        OutboundReply {
            text: Some(text),
            attachments: Vec::new(),
            metadata: None,
            reply_to_platform_message_id: None,
        }
    }

    #[test]
    fn long_lark_thread_reply_stays_whole_when_it_fits() {
        let text = format!(
            "{}\n\n[Event details](https://example.com/events/agent-meetup)",
            "活动安排。".repeat(500)
        );
        let parts = components(&reply(text.clone()), "lark").unwrap();
        assert_eq!(parts.len(), 1);
        assert_eq!(parts[0].text.as_deref(), Some(text.as_str()));
    }

    #[test]
    fn lark_parts_fit_serialized_content_even_with_json_escaping() {
        let text = "\"\\\n\t\u{01}🦉活动".repeat(4000);
        let parts = components(&reply(text.clone()), "lark").unwrap();
        assert!(parts.len() > 1);
        let mut reassembled = String::new();
        for part in parts {
            let text = part.text.unwrap();
            assert!(serde_json::json!({"text": text}).to_string().len() <= 30 * 1024);
            reassembled.push_str(&text);
        }
        assert_eq!(reassembled, text);
    }

    #[test]
    fn rejects_too_many_parts_before_sending_any_content() {
        assert!(components(&reply("x".repeat(4096 * 64 + 1)), "telegram").is_err());
    }
}

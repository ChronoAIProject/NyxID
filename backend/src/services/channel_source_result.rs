//! Metadata read for an already authorized channel conversation.
use crate::{
    errors::AppResult,
    models::{
        assistant_message,
        channel_conversation::ChannelConversation,
        channel_message,
        nyxbot_channel::{COLLECTION_NAME as CHANNELS, EVENTS_COLLECTION_NAME as EVENTS},
    },
};
use bson::{Document, doc};
use mongodb::Database;

pub(crate) struct SourceResult {
    pub inbound_message_id: String,
    pub platform_message_id: Option<String>,
    pub sender_platform_id: Option<String>,
    pub response_id: Option<String>,
    pub turn_status: &'static str,
    pub reply_status: &'static str,
    pub reply_evidence: Option<&'static str>,
    pub reply_kind: Option<&'static str>,
}

pub(crate) async fn lookup(
    db: &Database,
    conversation: &ChannelConversation,
    alias: &str,
) -> AppResult<Option<SourceResult>> {
    let Some(bot_id) = &conversation.channel_bot_id else {
        return Ok(None);
    };
    let now = bson::DateTime::now();
    let Some(event) = db
        .collection::<Document>(EVENTS)
        .find_one(doc! {"_id": alias, "custom_binding_id": null, "expires_at": {"$gt": now}})
        .projection(doc! {
            "event_id": 1, "relay_message_id": 1, "admission_message_id": 1, "channel_id": 1, "user_id": 1,
            "turn_id": 1, "conversation_id": 1, "status": 1, "gateway_response_id": 1,
            "response_read.version": 1, "response_read.expires_at": 1,
            "response_read.commands.response_id": 1,
            "delivery.origin.org_execution.actor_id": 1, "delivery.version": 1, "delivery.state": 1,
            "delivery.source_route_id": 1, "delivery.source_route_api_key_id": 1,
            "delivery.gateway_observation.status": 1, "delivery.preparation.gateway_receipt.pending": 1,
            "delivery.preparation.gateway_receipt.version": 1, "delivery.preparation.version": 1,
            "delivery.preparation.total": 1, "delivery.preparation.receipts.platform_message_ids": 1,
        })
        .await?
    else {
        return Ok(None);
    };
    let (Ok(inbound_id), Ok(channel_id), Ok(user_id)) = (
        event.get_str("relay_message_id"),
        event.get_str("channel_id"),
        event.get_str("user_id"),
    ) else {
        return Ok(None);
    };
    if event.get_str("event_id") != Ok(inbound_id) {
        return Ok(None);
    }
    // Organization channel provider records can belong to the linking person.
    // Scope them by the same bot installation and its resource owner.
    let Some(channel) = db.collection::<Document>(CHANNELS)
        .find_one(doc! {
            "_id": channel_id, "user_id": user_id, "channel_bot_id": bot_id, "platform": &conversation.platform,
            "$or": [
                {"bot_owner_id": &conversation.user_id},
                {"bot_owner_id": null, "user_id": &conversation.user_id},
            ],
        })
        .projection(doc! {"route_id": 1, "route_api_key_id": 1})
        .await?
    else {
        return Ok(None);
    };
    let delivery = event.get_document("delivery").ok();
    let captured_route = delivery.and_then(|d| d.get_str("source_route_id").ok());
    let captured_key = delivery.and_then(|d| d.get_str("source_route_api_key_id").ok());
    let admission_message = event.get_str("admission_message_id").ok();
    if captured_route.is_some_and(|route| route != conversation.id)
        || admission_message.is_some_and(|id| id != inbound_id)
        || (captured_route.is_none()
            && admission_message.is_none()
            && channel.get_str("route_id") != Ok(conversation.id.as_str()))
    {
        return Ok(None);
    }
    let Some(source_key) = captured_key.or_else(|| channel.get_str("route_api_key_id").ok()) else {
        return Ok(None);
    };
    // Match the original inbound by its stored ID and route key. A bot or
    // native message ID alone can appear on more than one conversation route.
    let Some(source) = db
        .collection::<Document>(channel_message::COLLECTION_NAME)
        .find_one(doc! {
            "_id": inbound_id, "direction": "inbound", "user_id": &conversation.user_id,
            "conversation_id": &conversation.id, "platform": &conversation.platform,
            "channel_bot_id": bot_id, "agent_api_key_id": source_key,
        })
        .projection(doc! {"platform_message_id": 1, "sender_platform_id": 1})
        .await?
    else {
        return Ok(None);
    };
    let actor_id = delivery
        .and_then(|doc| doc.get_document("origin").ok())
        .and_then(|doc| doc.get_document("org_execution").ok())
        .and_then(|doc| doc.get_str("actor_id").ok())
        .unwrap_or(user_id);
    let pending = matches!(
        event.get_str("status"),
        Ok("queued" | "running" | "resuming" | "retryable")
    );
    let turn_status = match (event.get_str("conversation_id"), event.get_str("turn_id")) {
        (Ok(conversation_id), Ok(turn_id)) => {
            let message = db
                .collection::<Document>(assistant_message::COLLECTION_NAME)
                .find_one(doc! {
                    "conversation_id": conversation_id, "turn_id": turn_id,
                    "user_id": actor_id, "role": "assistant",
                })
                .projection(doc! {"status": 1})
                .await?;
            match message.as_ref().and_then(|row| row.get_str("status").ok()) {
                Some("completed") => "completed",
                Some("failed") => "failed",
                Some("cancelled") => "cancelled",
                Some("interrupted") => "interrupted",
                Some("pending" | "running" | "in_progress" | "streaming") => "in_progress",
                None if pending => "in_progress",
                _ => "unknown",
            }
        }
        (Err(_), Err(_)) if pending => "pending",
        (Err(_), Err(_)) => "no_turn",
        _ => "unknown",
    };
    let mapped_response = event
        .get_document("response_read")
        .ok()
        .and_then(|mapping| {
            if mapping.get_i32("version") != Ok(1)
                || mapping.get_datetime("expires_at").ok()? <= &now
            {
                return None;
            }
            mapping
                .get_array("commands")
                .ok()?
                .first()?
                .as_document()?
                .get_str("response_id")
                .ok()
        });
    let (reply_status, reply_evidence) = reply_result(delivery);
    let reply_kind = reply_evidence.map(|_| match turn_status {
        "completed" => "completed_turn_output",
        "failed" | "cancelled" | "interrupted" => "failed_turn_output",
        _ => "unclassified",
    });
    Ok(Some(SourceResult {
        inbound_message_id: inbound_id.to_owned(),
        platform_message_id: source
            .get_str("platform_message_id")
            .ok()
            .map(str::to_owned),
        sender_platform_id: source.get_str("sender_platform_id").ok().map(str::to_owned),
        response_id: event
            .get_str("gateway_response_id")
            .ok()
            .or(mapped_response)
            .map(str::to_owned),
        turn_status,
        reply_status,
        reply_evidence,
        reply_kind,
    }))
}

fn reply_result(delivery: Option<&Document>) -> (&'static str, Option<&'static str>) {
    let Some(delivery) = delivery.filter(|d| d.get_i32("version") == Ok(1)) else {
        return ("unknown", None);
    };
    let preparation = delivery
        .get_document("preparation")
        .ok()
        .filter(|p| p.get_i32("version") == Ok(1));
    let receipt_pending = preparation
        .and_then(|p| p.get_document("gateway_receipt").ok())
        .is_some_and(|receipt| {
            matches!(receipt.get_i32("version"), Ok(1 | 2))
                && receipt.get_bool("pending") == Ok(true)
        });
    let gateway_confirmed = delivery
        .get_document("gateway_observation")
        .is_ok_and(|receipt| matches!(receipt.get_i32("status"), Ok(200..=299)));
    let platform_confirmed = preparation
        .and_then(|p| {
            Some((
                p.get_i64("total")
                    .or_else(|_| p.get_i32("total").map(i64::from))
                    .ok()?,
                p.get_array("receipts").ok()?,
            ))
        })
        .is_some_and(|(total, receipts)| {
            total > 0
                && total == receipts.len() as i64
                && receipts.iter().all(|receipt| {
                    receipt
                        .as_document()
                        .and_then(|part| part.get_array("platform_message_ids").ok())
                        .is_some_and(|ids| !ids.is_empty())
                })
        });
    let status = match delivery.get_str("state") {
        Ok("waiting" | "pending" | "preparing_text" | "sending") => "pending",
        Ok("unknown") if receipt_pending => "pending",
        Ok("unknown") => "unconfirmed",
        Ok("sent") if gateway_confirmed => return ("confirmed", Some("gateway_reply_receipt")),
        Ok("sent") if platform_confirmed => return ("confirmed", Some("platform_part_receipts")),
        Ok("streamed" | "replay") => "handed_to_gateway",
        Ok("silent") => "silent",
        Ok("refused") => "refused",
        _ => "unknown",
    };
    (status, None)
}

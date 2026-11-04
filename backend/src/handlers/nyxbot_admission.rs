//! Project admission into the existing owner-visible activity record.
use super::*;
use crate::services::nyxbot_admission_service as diagnostics;

pub(super) async fn outcome(
    state: &AppState,
    row: &NyxbotChannel,
    source: &str,
    result: Result<&Inbound, ()>,
) -> AppResult<()> {
    let code = match result {
        Ok(Inbound::Silent("handled")) => return Ok(()),
        Ok(Inbound::Silent(reason)) => reason,
        Ok(Inbound::Busy) => "refused_busy",
        Ok(Inbound::Queued) => "queued",
        Ok(Inbound::Reply(_)) => "reply_without_turn",
        Ok(Inbound::Turn(_)) => return Ok(()), // Bound in the turn transaction.
        Err(()) => "refused_processing_error",
    };
    if diagnostics::finish(&state.db, row, source, code)
        .await
        .is_err()
    {
        // An accepted queue or reply must survive a diagnostics-only failure.
        tracing::warn!(channel_id=%row.id, message_id=source, code="admission_diagnostic_unavailable", "NyxBot admission diagnostic could not be updated");
    }
    Ok(())
}

/// Native events have a persisted recovery identity. Reuse it for metadata;
/// never retain the opaque event reference or message body in this record.
pub(super) async fn gateway_source(
    state: &AppState,
    row: &NyxbotChannel,
    activity: &Value,
    event: &str,
) -> AppResult<Option<String>> {
    if activity["conversation"]["kind"] == "private" {
        return Ok(None);
    }
    if row.gateway_source.is_none() {
        return Ok((activity["source"]["type"] == "nyxid_relay")
            .then(|| activity["event_id"].as_str().map(str::to_owned))
            .flatten());
    }
    if row.platform != "discord" {
        return Ok(None);
    }
    let Some(response) = state
        .db
        .collection::<crate::models::nyxbot_gateway_response::GatewayResponse>(
            crate::models::nyxbot_gateway_response::COLLECTION_NAME,
        )
        .find_one(doc! {"event_key":event,"channel_id":&row.id})
        .await?
    else {
        return Ok(None);
    };
    let source = response.id;
    state.db.collection::<bson::Document>(crate::models::channel_message::COLLECTION_NAME).update_one(doc! {"_id":&source},doc! {"$setOnInsert":{
        "_id":&source,"channel_bot_id":&row.channel_bot_id,"conversation_id":&row.route_id,
        "user_id":bot_owner(row),"direction":"inbound","platform":"discord",
        "platform_conversation_id":activity["conversation"]["id"].as_str(),
        "sender_platform_id":activity["actor"]["id"].as_str(),"content_type":"text",
        "agent_api_key_id":&row.route_api_key_id,"native_gateway":true,"created_at":bson::DateTime::now()
    }}).upsert(true).await?;
    Ok(Some(source))
}

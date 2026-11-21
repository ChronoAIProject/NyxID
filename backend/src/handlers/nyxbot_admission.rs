//! Project admission into the existing owner-visible activity record.
use super::*;
use crate::services::nyxbot_admission_service as diagnostics;

pub(super) async fn outcome(
    state: &AppState,
    row: &NyxbotChannel,
    source: &str,
    result: Result<&Inbound, ()>,
) -> AppResult<Option<&'static str>> {
    if matches!(row.platform.as_str(), "x" | "whatsapp") {
        let observed = match &result {
            Ok(Inbound::Turn(_)) => Some(XOutcome::TurnStarted),
            Ok(Inbound::Queued) => Some(XOutcome::Queued),
            Ok(Inbound::Busy) => None, // finish() records the refusal.
            Ok(Inbound::Silent("handled")) | Ok(Inbound::Reply(_)) => None,
            // finish() records these after the per-message update.
            Ok(Inbound::Silent(_)) | Err(()) => None,
        };
        if let Some(outcome) = observed {
            x_diagnostics::for_channel(&state.db, row, outcome).await;
        }
    }
    let Some(code) = disposition(row, result) else {
        return Ok(None);
    };
    match diagnostics::finish(&state.db, row, source, code).await {
        Ok(true) => Ok(Some(code)),
        Ok(false) => Ok(None),
        Err(_) => {
            tracing::warn!(channel_id=%row.id, message_id=source, code="admission_diagnostic_unavailable", "NyxBot admission diagnostic could not be updated");
            Ok(None)
        }
    }
}

fn disposition(row: &NyxbotChannel, result: Result<&Inbound, ()>) -> Option<&'static str> {
    Some(match result {
        Ok(Inbound::Silent("handled")) => return None,
        Ok(Inbound::Silent(reason)) => reason,
        Ok(Inbound::Busy) => "refused_busy",
        Ok(Inbound::Queued) => "queued",
        Ok(Inbound::Reply(reply))
            if row.platform == "whatsapp" && matches!(reply.kind, Some(NoticeKind::GuestBusy)) =>
        {
            "refused_busy"
        }
        Ok(Inbound::Reply(_)) => "reply_without_turn",
        Ok(Inbound::Turn(_)) => return None, // Bound in the turn transaction.
        Err(()) => "refused_processing_error",
    })
}

pub(super) async fn native_whatsapp_outcome(
    state: &AppState,
    row: &NyxbotChannel,
    result: Result<&Inbound, ()>,
) {
    let observed = match result {
        Ok(Inbound::Turn(_)) => XOutcome::TurnStarted,
        Ok(Inbound::Queued) => XOutcome::Queued,
        _ => match disposition(row, result) {
            Some("reply_without_turn") | None => return,
            Some(code) => XOutcome::admission(code),
        },
    };
    x_diagnostics::for_channel(&state.db, row, observed).await;
}

pub(super) async fn telegram_group_outcome(
    state: &AppState,
    row: &NyxbotChannel,
    source: &str,
    activity: &Value,
    disposition: &str,
) {
    let code = match disposition {
        "refused_members" => "telegram_group_members_refused",
        "refused_owner_absent" => "telegram_group_owner_absent",
        "refused_sender" | "refused_self_bot" => "telegram_group_sender_refused",
        "refused_bot_budget" => "telegram_group_bot_budget",
        "refused_processing_error" => "telegram_group_processing_failed",
        "refused_busy" | "refused_empty_content" => return,
        reason if reason.starts_with("refused_") => "telegram_group_policy_refused",
        _ => return,
    };
    // The legacy gateway can imply addressing for admission. A bot-wide policy
    // diagnostic needs an actual mention or a reply to this bot in this chat.
    if disposition != "refused_processing_error" && activity["kind"]["mentions_bot"] != true {
        let Some(chat_id) = activity["conversation"]["id"].as_str() else {
            return;
        };
        let reply = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            chats::gateway_replies_to_bot(state, row, chat_id, source),
        )
        .await;
        if !matches!(reply, Ok(Ok(true))) {
            return;
        }
    }
    crate::services::channel_inbound_diagnostics::telegram_outcome(
        &state.db,
        &row.channel_bot_id,
        bot_owner(row),
        code,
        None,
    )
    .await;
}

/// Native events have a persisted recovery identity. Reuse it for metadata;
/// never retain the opaque event reference or message body in this record.
pub(super) async fn gateway_source(
    state: &AppState,
    row: &NyxbotChannel,
    activity: &Value,
    event: &str,
) -> AppResult<Option<String>> {
    if activity["conversation"]["kind"] == "private" && row.platform != "whatsapp" {
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

//! Correlate model generation/compaction refusals with the executing upstream
//! turn. This is server-owned correlation, not a model-only provenance claim.
//! Compaction retains exact action consent; stateless inference has turn consent.
use super::*;

pub async fn begin_model_attempt(
    db: &Database,
    row: &crate::models::assistant_conversation::AssistantConversation,
) -> AppResult<String> {
    let attempt = uuid::Uuid::new_v4().to_string();
    db.collection::<Document>(COLLECTION).insert_one(doc! {
        "_id": &attempt, "kind":"model_attempt", "active":true,
        "user_id":&row.user_id,"conversation_id":&row.id,"api_key_id":&row.credential_api_key_id,
        "turn_id":&row.active_turn.as_ref().ok_or(AppError::AssistantTurnRequired)?.turn_id,
        "expires_at":bson::DateTime::from_chrono(Utc::now()+Duration::minutes(35)),
    }).await?;
    Ok(attempt)
}

pub(super) async fn observe_model_callback(
    db: &Database,
    chat: &acks::ChatAuthority,
    refusal: Option<&Value>,
) -> AppResult<()> {
    let mut filter = scope(chat)?;
    filter.extend(doc! {"kind":"model_attempt","active":true});
    let update = if let Some(id) = refusal
        .filter(|value| value["error"] == "acknowledgement_required")
        .and_then(|value| value["acknowledgement_id"].as_str())
    {
        doc! {"$set":{"refused_card_id":id}}
    } else {
        doc! {"$unset":{"refused_card_id":""}}
    };
    db.collection::<Document>(COLLECTION)
        .update_one(filter, update)
        .await?;
    Ok(())
}

pub async fn finish_model_attempt(db: &Database, attempt: &str) -> AppResult<Option<String>> {
    let Some(marker) = db.collection::<Document>(COLLECTION).find_one_and_update(
        doc! {"_id":attempt,"kind":"model_attempt","active":true,"expires_at":{"$gt":bson::DateTime::now()}},
        doc! {"$set":{"active":false}},
    ).await? else { return Ok(None); };
    let live = acks::for_key(
        db,
        marker
            .get_str("user_id")
            .map_err(|_| AppError::AssistantTurnRequired)?,
        Some(
            marker
                .get_str("api_key_id")
                .map_err(|_| AppError::AssistantTurnRequired)?,
        ),
    )
    .await?;
    if live.as_ref().is_none_or(|chat| {
        !chat.turn_live
            || chat.turn_stopped
            || chat.turn_id.as_deref() != marker.get_str("turn_id").ok()
    }) {
        return Ok(None);
    }
    let Ok(card_id) = marker.get_str("refused_card_id") else {
        return Ok(None);
    };
    let card = db.collection::<Document>("assistant_acknowledgements").find_one(doc! {
        "_id":card_id,"user_id":marker.get_str("user_id").ok(),
        "conversation_id":marker.get_str("conversation_id").ok(),"api_key_id":marker.get_str("api_key_id").ok(),
        "requested_turn_id":marker.get_str("turn_id").ok(),"tool_name":"nyxid.service_execution",
        "status":"pending","expires_at":{"$gt":bson::DateTime::now()},
    }).await?;
    Ok(card.map(|card| {
        card.get_str("summary")
            .unwrap_or("use the organization connection for this turn")
            .to_owned()
    }))
}

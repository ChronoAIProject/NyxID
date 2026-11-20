//! Correlate model generation/compaction refusals with the executing upstream
//! turn. This is server-owned correlation, not a model-only provenance claim.
//! Compaction retains exact action consent; stateless inference has turn consent.
use super::*;

pub async fn begin_model_attempt(
    db: &Database,
    row: &crate::models::assistant_conversation::AssistantConversation,
) -> AppResult<String> {
    let active = row
        .active_turn
        .as_ref()
        .ok_or(AppError::AssistantTurnRequired)?;
    let turn = &active.turn_id;
    let attempt = active.restart_admission.as_ref().map_or_else(
        || uuid::Uuid::new_v4().to_string(),
        |source| source.reply_id.clone(),
    );
    let context = (db.clone(), row.clone(), turn.clone(), attempt.clone());
    crate::services::transaction_service::run_db_only(db,&context,|tx,(db,row,turn,attempt)| {
        Box::pin(async move {
        crate::services::erasure_admission::fence_in_session(db,&row.user_id,tx).await?;
        let enrolled = row.active_turn.as_ref().is_some_and(|turn| turn.restart_admission.is_some());
        let filter = if enrolled {
            crate::services::assistant_nyxagent::WorkerFence::matching(row, doc! {"credential_api_key_id":&row.credential_api_key_id, "active_turn.stop_requested":false})?
        } else {
            doc! {"_id":&row.id,"user_id":&row.user_id,"credential_api_key_id":&row.credential_api_key_id,"active_turn.turn_id":turn}
        };
        let live=db.collection::<Document>(crate::models::assistant_conversation::COLLECTION_NAME).update_one(
            filter,
            doc! {"$inc":{"model_attempt_fence":1_i64}}
        ).session(&mut *tx).await?;
        if live.matched_count!=1 {return Err(AppError::AssistantTurnRequired);}
        let mut marker = doc! {
            "_id":&attempt,"kind":"model_attempt","user_id":&row.user_id,
            "conversation_id":&row.id,"api_key_id":&row.credential_api_key_id,"turn_id":turn,"key_version":row.credential_version_floor(),
            "expires_at":bson::DateTime::from_chrono(Utc::now()+Duration::minutes(35))
        };
        let markers = db.collection::<Document>(COLLECTION);
        if enrolled {
            let claim = row.active_turn.as_ref().and_then(|turn| turn.execution_attempt.as_ref()).ok_or(AppError::AssistantTurnRequired)?;
            markers.update_one(doc! {"_id":attempt, "user_id":&row.user_id,"conversation_id":&row.id,"turn_id":turn},doc! {"$setOnInsert":marker, "$set":{"active":true,"claim_id":&claim.claim_id}}).upsert(true).session(&mut *tx).await?;
        } else {
            marker.insert("active", true);
            markers.insert_one(marker).session(&mut *tx).await?;
        }
        Ok(())
        })
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

pub async fn finish_model_attempt(
    db: &Database,
    attempt: &str,
    row: &crate::models::assistant_conversation::AssistantConversation,
) -> AppResult<Option<String>> {
    let markers = db.collection::<Document>(COLLECTION);
    let mut filter = doc! {"_id":attempt,"kind":"model_attempt","active":true,"expires_at":{"$gt":bson::DateTime::now()}};
    if let Some(active) = row
        .active_turn
        .as_ref()
        .filter(|turn| turn.restart_admission.is_some())
    {
        let claim = active
            .execution_attempt
            .as_ref()
            .ok_or(AppError::AssistantTurnRequired)?;
        filter.insert("claim_id", &claim.claim_id);
    }
    let marker = markers
        .find_one_and_update(filter, doc! {"$set":{"active":false}})
        .await?;
    let Some(marker) = marker else {
        return Ok(None);
    };
    let live = acks::chat_for_key_version(
        db,
        marker
            .get_str("user_id")
            .map_err(|_| AppError::AssistantTurnRequired)?,
        Some(
            marker
                .get_str("api_key_id")
                .map_err(|_| AppError::AssistantTurnRequired)?,
        ),
        marker.get_i64("key_version").unwrap_or_default(),
        None,
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

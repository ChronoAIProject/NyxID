//! Cross-replica admission, controls and generation fencing.
use crate::{
    errors::{AppError, AppResult},
    models::{
        assistant_voice::{FINALIZATION_HOURS, MAX_CALL_SECONDS, VoicePreferences},
        assistant_voice_session::{COLLECTION_NAME, LEASE_SECONDS, SessionState, VoiceSession},
    },
};
use chrono::{Duration, Utc};
use futures::TryStreamExt;
use mongodb::{
    Database, IndexModel,
    bson::{self, Document, doc},
    options::{IndexOptions, ReturnDocument},
};

pub async fn ensure_indexes(db: &Database) -> mongodb::error::Result<()> {
    db.collection::<VoiceSession>(COLLECTION_NAME)
        .create_indexes([
            IndexModel::builder()
                .keys(doc! {"user_id":1})
                .options(
                    IndexOptions::builder()
                        .unique(true)
                        .partial_filter_expression(doc! {"live_slot":true})
                        .build(),
                )
                .build(),
            IndexModel::builder()
                .keys(doc! {"user_id":1,"client_request_id":1})
                .options(IndexOptions::builder().unique(true).build())
                .build(),
            IndexModel::builder()
                .keys(doc! {"live_slot":1,"lease_until":1})
                .build(),
            IndexModel::builder()
                .keys(doc! {"user_id":1,"created_at":1})
                .build(),
            IndexModel::builder()
                .keys(doc! {"conversation_id":1})
                .build(),
            IndexModel::builder()
                .keys(doc! {"final_usage_confirmed":1,"reconcile_deadline":1})
                .build(),
        ])
        .await?;
    Ok(())
}

pub async fn admit(
    db: &Database,
    user: &str,
    conversation: &str,
    client_request_id: &str,
    preferences: VoicePreferences,
    credential_identity: String,
    worker: &str,
) -> AppResult<VoiceSession> {
    super::super::assistant_voice::require_enabled(db, user)
        .await
        .map_err(|e| crate::errors::voice_start::Stage::Flag.error(e))?;
    super::super::assistant_voice::thread(db, user, conversation).await?;
    if uuid::Uuid::parse_str(client_request_id).is_err() {
        return Err(AppError::ValidationError(
            "Invalid voice request identity".into(),
        ));
    }
    let rows = db.collection::<VoiceSession>(COLLECTION_NAME);
    let now = Utc::now();
    if rows
        .find_one(doc! {"user_id":user,"client_request_id":client_request_id})
        .await?
        .is_some()
    {
        return Err(AppError::Conflict(
            "Voice start already attempted; do not replay the offer".into(),
        ));
    }
    for (seconds, limit) in [(60, 5), (3600, 30)] {
        if rows.count_documents(doc!{"user_id":user,"created_at":{"$gte":bson::DateTime::from_chrono(now-Duration::seconds(seconds))}}).await? >= limit {
            return Err(AppError::RateLimited);
        }
    }
    let row = VoiceSession {
        id: uuid::Uuid::new_v4().to_string(),
        user_id: user.into(),
        conversation_id: conversation.into(),
        client_request_id: client_request_id.into(),
        notify_on_completion: preferences.notify_on_completion,
        preferences,
        receipt_message_id: None,
        purge_requested: false,
        protocol: None,
        measured_ms: 0,
        state: SessionState::Starting,
        live_slot: true,
        generation: 1,
        lease_owner: worker.into(),
        lease_until: now + Duration::seconds(LEASE_SECONDS),
        created_at: now,
        deadline: now + Duration::seconds(MAX_CALL_SECONDS),
        last_user_at: now,
        heartbeat_at: now,
        closed_at: None,
        provider_session_id: None,
        credential_identity,
        desired_muted: false,
        input_muted: false,
        end_requested: false,
        end_reason: None,
        observed_seconds: 0,
        reserved_until: 0,
        final_usage_confirmed: false,
        billing_finalized: false,
        playback_ms: 0,
        inaudible_until_ms: 0,
        playback_revision: 0,
        control_revision: 0,
        control_socket_id: None,
        control_socket_until: None,
        last_command_id: None,
        last_command_action: None,
        reconcile_deadline: now + Duration::hours(FINALIZATION_HOURS),
    };
    let mut tx = db.client().start_session().await?;
    let db = db.clone();
    let saved = row.clone();
    tx.start_transaction()
        .and_run2(async move |tx| {
            let result: AppResult<VoiceSession> = Box::pin(async {
                if db
                    .collection::<Document>(crate::models::assistant_conversation::COLLECTION_NAME)
                    .update_one(
                        doc! {"_id":&saved.conversation_id,"user_id":&saved.user_id},
                        doc! {"$inc":{"voice_session_fence":1}},
                    )
                    .session(&mut *tx)
                    .await?
                    .matched_count
                    != 1
                {
                    return Err(AppError::NotFound("Voice conversation unavailable".into()));
                }
                match db
                    .collection::<VoiceSession>(COLLECTION_NAME)
                    .insert_one(&saved)
                    .session(&mut *tx)
                    .await
                {
                    Ok(_) => Ok(saved.clone()),
                    Err(e) if super::super::assistant_team_service::is_duplicate(&e) => {
                        Err(AppError::Conflict("End the current call first".into()))
                    }
                    Err(e) => Err(e.into()),
                }
            })
            .await;
            super::super::api_key_mutation_service::transaction_result(result)
        })
        .await
        .map_err(super::super::api_key_mutation_service::map_transaction_error)
}

pub fn fence(row: &VoiceSession) -> Document {
    doc! {"_id":&row.id,"generation":row.generation,"lease_owner":&row.lease_owner,"live_slot":row.live_slot,
    "lease_until":{"$gt":bson::DateTime::from_chrono(Utc::now())}}
}
pub async fn write(db: &Database, row: &VoiceSession, set: Document) -> AppResult<VoiceSession> {
    db.collection::<VoiceSession>(COLLECTION_NAME)
        .find_one_and_update(fence(row), doc! {"$set":set})
        .return_document(ReturnDocument::After)
        .await?
        .ok_or_else(|| AppError::Conflict("Voice session ownership changed".into()))
}
pub async fn refresh(db: &Database, row: &VoiceSession) -> AppResult<VoiceSession> {
    write(db,row,doc!{"lease_until":bson::DateTime::from_chrono(Utc::now()+Duration::seconds(LEASE_SECONDS))}).await
}
pub async fn get(
    db: &Database,
    user: &str,
    conversation: &str,
    id: &str,
) -> AppResult<VoiceSession> {
    super::super::assistant_voice::thread(db, user, conversation).await?;
    db.collection::<VoiceSession>(COLLECTION_NAME)
        .find_one(doc! {"_id":id,"user_id":user,"conversation_id":conversation})
        .await?
        .ok_or_else(|| AppError::NotFound("Voice session not found".into()))
}

/// Return the caller's live call without requiring the voice feature flag.
/// This is intentionally metadata-only so a user can recover or end a call
/// after an operator changes rollout configuration.
pub async fn active_for_conversation(
    db: &Database,
    user: &str,
    conversation: &str,
) -> AppResult<Option<VoiceSession>> {
    super::super::assistant_voice::thread(db, user, conversation).await?;
    Ok(db
        .collection::<VoiceSession>(COLLECTION_NAME)
        .find_one(doc! {
            "user_id": user,
            "conversation_id": conversation,
            "live_slot": true,
        })
        .sort(doc! {"created_at": -1})
        .await?)
}

pub async fn close(
    db: &Database,
    row: &VoiceSession,
    reason: &str,
    confirmed: bool,
) -> AppResult<()> {
    let db = db.clone();
    let row = row.clone();
    let reason = reason.to_string();
    let mut tx = db.client().start_session().await?;
    tx.start_transaction()
        .and_run2(async move |tx| {
            let result: AppResult<()> = Box::pin(async {
                let live = !confirmed && !row.final_usage_confirmed && Utc::now() < row.deadline;
            let closed = db.collection::<VoiceSession>(COLLECTION_NAME).find_one_and_update(
                fence(&row), doc! {"$set":{"state":if live {"closing"} else {"closed"},"live_slot":live,"end_reason":&reason,
                    "closed_at":bson::DateTime::from_chrono(row.closed_at.unwrap_or_else(Utc::now)),
                    "final_usage_confirmed":row.final_usage_confirmed || confirmed,
                    "lease_until":bson::DateTime::from_chrono(Utc::now()+Duration::seconds(30))}})
                .return_document(ReturnDocument::After).session(&mut *tx).await?
                .ok_or_else(||AppError::Conflict("Voice session ownership changed".into()))?;
                let user_exists=db.collection::<Document>(crate::models::user::COLLECTION_NAME)
                    .find_one(doc! {"_id":&closed.user_id}).session(&mut *tx).await?.is_some();
                let purge=closed.purge_requested || !user_exists;
                if purge && !closed.purge_requested {
                    db.collection::<Document>(COLLECTION_NAME).update_one(doc! {"_id":&closed.id},doc! {"$set":{"purge_requested":true}}).session(&mut *tx).await?;
                }
                if closed.receipt_message_id.is_none() && !purge {
                    super::receipt::append(&db, tx, &closed).await?;
                }
                if purge
                    && closed.billing_finalized
                    && (closed.final_usage_confirmed || Utc::now() >= closed.reconcile_deadline)
                {
                    db.collection::<VoiceSession>(COLLECTION_NAME)
                        .delete_one(doc! {"_id":&closed.id})
                        .session(&mut *tx)
                        .await?;
                }
                Ok(())
            })
            .await;
            super::super::api_key_mutation_service::transaction_result(result)
        })
        .await
        .map_err(super::super::api_key_mutation_service::map_transaction_error)
}

/// Recovery acquires a fresh fence. OpenAI calls are reattached by the runtime;
/// calls that were explicitly ended retain that intent and are closed there.
pub async fn claim_orphans(db: &Database, worker: &str) -> AppResult<Vec<VoiceSession>> {
    let rows = db.collection::<VoiceSession>(COLLECTION_NAME);
    let now = Utc::now();
    let candidates:Vec<VoiceSession>=rows.find(doc!{"lease_until":{"$lte":bson::DateTime::from_chrono(now)},"$or":[{"live_slot":true},{"purge_requested":true,"reconcile_deadline":{"$lte":bson::DateTime::from_chrono(now)}},{"billing_finalized":false,"reconcile_deadline":{"$lte":bson::DateTime::from_chrono(now)}},{"final_usage_confirmed":true,"billing_finalized":false},{"final_usage_confirmed":false,"provider_session_id":{"$type":"string"},"reconcile_deadline":{"$gt":bson::DateTime::from_chrono(now)}}]})
        .sort(doc!{"lease_until":1}).limit(2).await?.try_collect().await?;
    let mut claimed = Vec::new();
    for row in candidates {
        if let Some(row)=rows.find_one_and_update(doc!{"_id":row.id,"generation":row.generation,"live_slot":row.live_slot,
            "lease_until":{"$lte":bson::DateTime::from_chrono(now)}},doc!{"$inc":{"generation":1},"$set":{
            "lease_owner":worker,"lease_until":bson::DateTime::from_chrono(now+Duration::seconds(LEASE_SECONDS))}})
            .return_document(ReturnDocument::After).await? {claimed.push(row);}
    }
    Ok(claimed)
}

pub async fn control(
    db: &Database,
    user: &str,
    conversation: &str,
    id: &str,
    command_id: &str,
    expected_revision: i64,
    action: &str,
) -> AppResult<VoiceSession> {
    if uuid::Uuid::parse_str(command_id).is_err() || !matches!(action, "mute" | "unmute" | "end") {
        return Err(AppError::ValidationError("Invalid voice control".into()));
    }
    let row = get(db, user, conversation, id).await?;
    if row.last_command_id.as_deref() == Some(command_id) {
        if row.last_command_action.as_deref() != Some(action) {
            return Err(AppError::Conflict("Voice control identity reused".into()));
        }
        return Ok(row);
    }
    if !row.live_slot || row.end_requested {
        return Ok(row);
    }
    let mut set = doc! {"last_command_id":command_id,"last_command_action":action};
    if action == "end" {
        set.insert("end_requested", true);
        set.insert("state", "closing");
    } else {
        set.insert("desired_muted", action == "mute");
    }
    let mut filter = doc! {"_id":id,"user_id":user,"live_slot":true,"end_requested":false};
    // End is terminal and may safely win a concurrent mute/unmute revision.
    if action != "end" {
        filter.insert("control_revision", expected_revision);
    }
    db.collection::<VoiceSession>(COLLECTION_NAME)
        .find_one_and_update(filter, doc! {"$set":set,"$inc":{"control_revision":1}})
        .return_document(ReturnDocument::After)
        .await?
        .ok_or_else(|| AppError::Conflict("Voice controls changed; refresh the session".into()))
}

pub async fn heartbeat(
    db: &Database,
    row: &VoiceSession,
    generation: i64,
    playback_ms: i64,
    playback_audible: bool,
    revision: i64,
) -> AppResult<()> {
    let elapsed = (Utc::now() - row.created_at).num_milliseconds().max(0);
    if generation != row.generation || !(0..=elapsed).contains(&playback_ms) || revision < 0 {
        return Err(AppError::ValidationError(
            "Invalid voice playback watermark".into(),
        ));
    }
    let mut maxima = doc! {"playback_ms":playback_ms};
    if !playback_audible {
        maxima.insert("inaudible_until_ms", elapsed);
    }
    // Only a newer report may advance playback. Reports never grant a decision.
    db.collection::<VoiceSession>(COLLECTION_NAME).update_one(doc!{"_id":&row.id,"user_id":&row.user_id,
        "generation":generation,"live_slot":true,"playback_revision":{"$lt":revision}},
        doc!{"$set":{"heartbeat_at":bson::DateTime::from_chrono(Utc::now()),"playback_revision":revision},
            "$max":maxima}).await?;
    Ok(())
}

pub async fn claim_stream(db: &Database, row: &VoiceSession, id: &str) -> AppResult<()> {
    let now = Utc::now();
    let changed=db.collection::<Document>(COLLECTION_NAME).update_one(doc! {"_id":&row.id,"user_id":&row.user_id,
        "generation":row.generation,"live_slot":true,"end_requested":false,"$or":[{"control_socket_until":bson::Bson::Null},
        {"control_socket_until":{"$lte":bson::DateTime::from_chrono(now)}}]},
        doc! {"$set":{"control_socket_id":id,"control_socket_until":bson::DateTime::from_chrono(now+Duration::seconds(30))}}).await?;
    if changed.matched_count != 1 {
        return Err(AppError::Conflict("Voice control is already open".into()));
    }
    Ok(())
}
pub async fn refresh_stream(db: &Database, row: &VoiceSession, id: &str) -> AppResult<()> {
    let changed=db.collection::<Document>(COLLECTION_NAME).update_one(doc! {"_id":&row.id,"user_id":&row.user_id,
        "generation":row.generation,"control_socket_id":id,"control_socket_until":{"$gt":bson::DateTime::now()}},
        doc! {"$set":{"control_socket_until":bson::DateTime::from_chrono(Utc::now()+Duration::seconds(30))}}).await?;
    if changed.matched_count != 1 {
        return Err(AppError::ClientDisconnected);
    }
    Ok(())
}
pub async fn release_stream(db: &Database, row: &VoiceSession, id: &str) -> AppResult<()> {
    // A browser transport disconnect is recoverable. Only release the socket
    // lease; an explicit End command owns end_requested.
    db.collection::<Document>(COLLECTION_NAME)
        .update_one(
            doc! {"_id":&row.id,"user_id":&row.user_id,"control_socket_id":id},
            doc! {"$set":{"control_socket_until":bson::DateTime::now()}},
        )
        .await?;
    Ok(())
}

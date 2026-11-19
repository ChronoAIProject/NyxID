use super::*;
use crate::{
    models::assistant_conversation::{EngineWorkerInstance, RestartSource},
    services::assistant_turn_restart,
};

pub(crate) async fn cancel(state: &AppState, row: &AssistantConversation) {
    let Some(admission) = row
        .active_turn
        .as_ref()
        .and_then(|turn| turn.restart_admission.as_deref())
        .filter(|source| source.window.is_some())
    else {
        return;
    };
    let Ok(auth) = crate::handlers::assistant_team::owner_auth(&row.user_id) else {
        return;
    };
    let budget = tokio::time::timeout(Duration::from_secs(3), async {
        let credential = credentials::load_for_conversation(
            &state.db,
            &state.encryption_keys,
            &row.user_id,
            &row.id,
        )
        .await
        .ok()
        .flatten()
        .filter(|credential| credential.api_key_id == admission.credential_id)?;
        let transport = http_egress(state.clone(), auth.clone());
        crate::services::assistant_nyxagent_runtime::cancel_background(
            state,
            transport.as_ref(),
            row,
            &credential,
            Some(SERVER_TURN_POLICY),
        )
        .await;
        Some(())
    })
    .await;
    if !matches!(budget, Ok(Some(()))) {
        crate::services::audit_service::log_for_user(
            state.db.clone(),
            &auth,
            "assistant_background_cancel",
            Some(
                json!({"conversation_id":row.id,"turn_id":row.active_turn.as_ref().map(|turn|&turn.turn_id),"outcome":"cancel_unconfirmed"}),
            ),
        );
    }
}

pub(super) async fn take_one(
    state: &AppState,
    offered: &AssistantConversation,
    worker: &EngineWorkerInstance,
) -> AppResult<bool> {
    let Some(_work) = state.shutdown.background() else {
        return Ok(false);
    };
    let Some(active) = offered.active_turn.as_ref() else {
        return Ok(false);
    };
    let Some(admission) = active.restart_admission.as_deref() else {
        return Ok(false);
    };
    if admission.actor_id != offered.user_id
        || admission.model != offered.model
        || Some(&admission.agent_id) != offered.agent_id.as_ref()
        || admission.credential_id != offered.credential_api_key_id
        || admission.credential_ended
    {
        return Ok(false);
    }
    let authorized = engine::get(&state.db, &offered.user_id, &offered.id).await?;
    if authorized
        .active_turn
        .as_ref()
        .map(|turn| &turn.execution_attempt)
        != Some(&active.execution_attempt)
    {
        return Ok(false);
    }
    let credential = credentials::load_for_conversation(
        &state.db,
        &state.encryption_keys,
        &offered.user_id,
        &offered.id,
    )
    .await?
    .filter(|key| key.api_key_id == admission.credential_id)
    .ok_or(AppError::AssistantTurnRequired)?;
    let message = state.db.collection::<crate::models::assistant_message::AssistantMessage>(crate::models::assistant_message::COLLECTION_NAME).find_one(doc!{"_id":&admission.initiating_message_id,"user_id":&admission.actor_id,"conversation_id":&offered.id,"turn_id":&active.turn_id,"seq":admission.initiating_seq,"role":{"$in":["system","user"]}}).await?.ok_or(AppError::AssistantTurnRequired)?;
    let auth = crate::handlers::assistant_team::owner_auth(&admission.actor_id)?;
    let permit = match admission.source {
        RestartSource::FirstPartyOwner => {
            state
                .direct_chat_limiter
                .recover_owner_slot(&admission.actor_id)
                .await?
        }
        _ => {
            let limit =
                crate::handlers::assistant_team::team_pool_limit(state, &admission.actor_id).await
                    + u32::from(!offered.is_subagent());
            state
                .direct_chat_limiter
                .try_acquire_pool("assistant_team", &admission.actor_id, limit)
                .await?
        }
    };
    let Some(permit) = permit else {
        return Ok(false);
    };
    if state.shutdown.is_draining() {
        return Ok(false);
    }
    let Some(row) = assistant_turn_restart::claim(&state.db, offered, worker).await? else {
        return Ok(false);
    };
    credentials::validate_claimed(&state.db, &row, &credential).await?;
    let cursor = active
        .reply
        .as_ref()
        .map_or(0, |reply| reply.cursor.max(0) as u64);
    let (sender, _) = broadcast::channel(256);
    let events = Events {
        sender,
        cursor,
        reset_notified: false,
        checkpoint: None,
    };
    state.shutdown.spawn(run_turn(
        state.clone(),
        auth,
        row,
        message.text,
        credential,
        Some(SERVER_TURN_POLICY),
        permit,
        events,
    ));
    Ok(true)
}

pub(crate) fn spawn(state: AppState) {
    let wake = std::sync::Arc::new(tokio::sync::Notify::new());
    let listener = wake.clone();
    let collection = state.db.collection::<mongodb::bson::Document>(
        crate::models::assistant_conversation::COLLECTION_NAME,
    );
    let shutdown = state.shutdown.clone();
    state.shutdown.spawn_worker(async move {
        loop {
            let listen = async {
                let mut stream = collection.watch().pipeline([doc!{"$match":{"$or":[{"updateDescription.updatedFields.active_turn.restart_offer":{"$exists":true}},{"operationType":{"$in":["replace","insert"]},"fullDocument.active_turn.restart_offer.due_at":{"$type":"date"}}]}}]).await?;
                while let Some(change) = stream.next().await {change?; listener.notify_one();}
                Ok::<_,mongodb::error::Error>(())
            };
            tokio::select!{() = shutdown.intake_stopped() => return, _ = listen => {}}
            tokio::select!{() = shutdown.intake_stopped() => return, () = tokio::time::sleep(Duration::from_secs(5)) => {}}
        }
    });
    state.shutdown.clone().spawn_worker(async move {
        let worker = crate::services::assistant_engine_worker::instance();
        let mut interval = tokio::time::interval(Duration::from_secs(3));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select!{() = state.shutdown.intake_stopped() => break,_ = interval.tick() => {},() = wake.notified() => {}}
            let Ok(rows) = assistant_turn_restart::due(&state.db,&worker.id).await else {continue;};
            for row in rows { if state.shutdown.is_draining() {return;}
                if let Err(error) = take_one(&state,&row,worker).await { tracing::warn!(conversation_id=%row.id,error_code=error.error_code(),"Assistant subscriber claim deferred"); }
            }
        }
    });
}

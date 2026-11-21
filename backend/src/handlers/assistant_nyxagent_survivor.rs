use super::*;
use crate::{
    models::assistant_conversation::{EngineWorkerInstance, RestartSource, TeamRestartSource},
    services::assistant_turn_restart,
};

pub(crate) async fn cancel_before_revocation(
    state: &AppState,
    owner: &str,
    mut target: mongodb::bson::Document,
) {
    use futures::TryStreamExt;
    let running = doc! {
        "active_turn.restart_admission.window": {"$type":"object"},
        "active_turn.restart_admission.window.terminal": {"$ne":true},
    };
    target.insert(
        "$or",
        vec![running, doc! {"stopped_window":{"$type":"object"}}],
    );
    let cancellation = async {
        state
            .db
            .collection::<AssistantConversation>(
                crate::models::assistant_conversation::COLLECTION_NAME,
            )
            .find(target)
            .await?
            .try_for_each_concurrent(16, |row| async move {
                cancel(state, &row).await;
                Ok(())
            })
            .await
    };
    if !matches!(
        tokio::time::timeout(Duration::from_secs(3), cancellation).await,
        Ok(Ok(()))
    ) {
        crate::services::audit_service::log_async(
            state.db.clone(),
            Some(owner.to_owned()),
            "assistant_background_cancel".into(),
            Some(json!({"outcome":"cancel_unconfirmed","reason":"credential_revocation"})),
            None,
            None,
            None,
            None,
        );
    }
}

pub(crate) async fn cancel(state: &AppState, row: &AssistantConversation) {
    if let Some(stopped) = row
        .stopped_window
        .as_ref()
        .filter(|_| row.active_turn.is_none())
    {
        let Ok(auth) = crate::handlers::assistant_team::owner_auth(&row.user_id) else {
            return;
        };
        let confirmed = tokio::time::timeout(Duration::from_secs(3), async {
            let destination = stopped.destination_hash.as_ref()?;
            let credential = credentials::load_for_conversation(
                &state.db,
                &state.encryption_keys,
                &row.user_id,
                &row.id,
            )
            .await
            .ok()??;
            if credential.api_key_id != stopped.credential_id {
                return None;
            }
            let result = proxy_raw(
                state,
                &auth,
                &credential.raw_key,
                "POST",
                "v1/responses/cancel",
                None,
                Some(&stopped.window_key),
                Some(
                    crate::services::platform_manager_contract::NyxAgentDestination(
                        destination.clone(),
                    ),
                ),
                Some(SERVER_TURN_POLICY),
            )
            .await
            .ok()?;
            if !result.status().is_success() {
                return None;
            }
            let body = axum::body::to_bytes(result.into_body(), engine::MAX_STREAM_BYTES)
                .await
                .ok()?;
            let value: Value = serde_json::from_slice(&body).ok()?;
            Some(
                matches!(
                    value["status"].as_str(),
                    Some("completed" | "cancelled" | "failed")
                ) && value["response"]["error"]["code"] != "executor_lost",
            )
        })
        .await;
        let outcome = if matches!(confirmed, Ok(Some(true))) {
            "terminal"
        } else {
            "cancel_unconfirmed"
        };
        crate::services::audit_service::log_for_user(
            state.db.clone(),
            &auth,
            "assistant_background_cancel",
            Some(
                json!({"conversation_id":row.id,"turn_id":stopped.turn_id,"outcome":outcome,"reason":"credential_revocation"}),
            ),
        );
        return;
    }
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

pub(in crate::handlers) async fn take_one(
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
    let review = if admission.credential_ended {
        Some(engine::TurnError::internal(
            "credential_invalid",
            "credential_revocation",
        ))
    } else {
        crate::services::assistant_nyxagent_runtime::expired_handoff(offered).map(|deadline| {
            if admission
                .window
                .as_ref()
                .is_some_and(|window| !window.terminal)
            {
                engine::TurnError::internal("outcome_unknown", "claim_deadline")
            } else {
                deadline
            }
        })
    };
    if let Some(reason) = review {
        let Some(row) = assistant_turn_restart::claim(&state.db, offered, worker).await? else {
            return Ok(false);
        };
        let text = active
            .reply
            .as_ref()
            .map(|reply| reply.text.clone())
            .unwrap_or_default();
        if !admission.credential_ended {
            cancel(state, &row).await;
        }
        let outcome = engine::TurnResult {
            text: text.clone(),
            session_id: None,
            response_id: None,
            error: Some(reason),
        };
        let error = engine::finish_turn_inner(
            &state.db,
            &row,
            &admission.credential_id,
            &admission.reply_id,
            &outcome,
            None,
            engine::SettlementCondition::Worker,
        )
        .await?;
        crate::handlers::assistant_team::after_turn_boxed(state.clone(), row, text, error).await;
        return Ok(true);
    }
    if admission.actor_id != offered.user_id
        || admission.model != offered.model
        || Some(&admission.agent_id) != offered.agent_id.as_ref()
        || admission.credential_id != offered.credential_api_key_id
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
    use crate::models::assistant_conversation::TurnOrigin;
    let role = match active.origin {
        TurnOrigin::User => "user",
        TurnOrigin::Trigger | TurnOrigin::Event => "event",
        TurnOrigin::Group => "group",
        TurnOrigin::Orchestrator => "orchestrator",
        TurnOrigin::Channel => return Ok(false),
    };
    let message = state
        .db
        .collection::<crate::models::assistant_message::AssistantMessage>(
            crate::models::assistant_message::COLLECTION_NAME,
        )
        .find_one(doc! {
            "_id": &admission.initiating_message_id, "user_id": &admission.actor_id,
            "conversation_id": &offered.id, "turn_id": &active.turn_id,
            "seq": admission.initiating_seq, "role": role,
        })
        .await?
        .ok_or(AppError::AssistantTurnRequired)?;
    let auth = crate::handlers::assistant_team::owner_auth(&admission.actor_id)?;
    if state.shutdown.is_draining() {
        return Ok(false);
    }
    let limiter = &state.direct_chat_limiter;
    let owner = &admission.actor_id;
    let team_limit = match (&admission.source, &admission.team_source) {
        (RestartSource::FirstPartyOwner, None) => None,
        _ => Some(
            crate::handlers::assistant_team::team_pool_limit(state, owner).await
                + u32::from(
                    !offered.is_subagent()
                        || matches!(
                            admission.team_source,
                            Some(TeamRestartSource::GroupMember { .. })
                        ),
                ),
        ),
    };
    let (row, permit) = if let Some(token) = &admission.capacity {
        let context = (
            state.db.clone(),
            limiter.clone(),
            token.clone(),
            owner.clone(),
            team_limit,
            offered.clone(),
            worker.clone(),
        );
        let claimed = crate::services::transaction_service::run_db_only(
            &state.db,
            &context,
            |session, (db, limiter, token, owner, team_limit, offered, worker)| {
                Box::pin(async move {
                    let transferred = limiter
                        .transfer_capacity(session, token, owner, *team_limit)
                        .await?;
                    if transferred.is_none()
                        && crate::services::assistant_nyxagent_runtime::expired_handoff(offered)
                            .is_none()
                    {
                        return Err(AppError::AssistantTurnActive);
                    }
                    let row = assistant_turn_restart::claim_capacity(
                        db,
                        offered,
                        worker,
                        transferred.as_ref().map(|grant| &grant.token),
                        session,
                    )
                    .await?
                    .ok_or(AppError::AssistantTurnActive)?;
                    Ok((row, transferred))
                })
            },
        )
        .await;
        let (row, transferred) = match claimed {
            Ok(claimed) => claimed,
            Err(AppError::AssistantTurnActive) => return Ok(false),
            Err(error) => return Err(error),
        };
        let permit = match transferred {
            Some(grant) => limiter.transferred_permit(owner, grant),
            None => DirectChatPermit::settlement_only(),
        };
        (row, permit)
    } else {
        let permit = match team_limit {
            None => limiter.recover_owner_slot(owner).await?,
            Some(limit) => {
                limiter
                    .try_acquire_pool("assistant_team", owner, limit)
                    .await?
            }
        };
        let Some(permit) = permit else {
            return Ok(false);
        };
        let Some(row) = assistant_turn_restart::claim(&state.db, offered, worker).await? else {
            return Ok(false);
        };
        (row, permit)
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

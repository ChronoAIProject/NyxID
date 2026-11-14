//! Delivery adapters only; the service owns durable source/claim authorization.
use super::*;
use crate::{
    models::{
        assistant_message::AssistantMessage,
        execution_permission_channel::{COLLECTION_NAME as DELIVERIES, Delivery},
    },
    services::execution_permission_channel::{self as channel, deliveries},
};

#[cfg(test)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum TestProcessFailure {
    Preparation,
    Claim,
}

#[cfg(test)]
tokio::task_local! {
    static TEST_PROCESS_FAILURE: TestProcessFailure;
}

#[cfg(test)]
pub(crate) async fn process_with_test_failure(
    state: &AppState,
    id: &str,
    failure: TestProcessFailure,
) -> AppResult<()> {
    TEST_PROCESS_FAILURE
        .scope(failure, process(state, id))
        .await
}

#[cfg(test)]
fn injected_failure(failure: TestProcessFailure) -> bool {
    TEST_PROCESS_FAILURE
        .try_with(|current| *current == failure)
        .unwrap_or(false)
}

pub(crate) async fn process(state: &AppState, id: &str) -> AppResult<()> {
    let Some(d) = deliveries::lease(&state.db, id).await? else {
        return Ok(());
    };
    let mut decision = supplemental::Decision::new(supplemental::Purpose::PermissionResult);
    decision.skip(supplemental::Reason::AuthorityRefused);
    decision.event = d
        .task
        .channel_source
        .as_ref()
        .map(|source| source.event_id.clone());
    let preparation=async {
        #[cfg(test)]
        if injected_failure(TestProcessFailure::Preparation) {
            return Err(AppError::Internal("test preparation failure".into()));
        }
        let source=d.task.channel_source.as_ref().ok_or_else(channel::refused)?;
        let mut session=state.db.client().start_session().await?;
        let rows=channel::validate(&state.db,&mut session,source,false).await?;
        let history=engine::get(&state.db,&d.user_id,&d.conversation_id).await?;
        let message=state.db.collection::<AssistantMessage>("assistant_messages").find_one(doc!{
            "_id":&d.message_id,"user_id":&d.user_id,"conversation_id":&d.conversation_id,"turn_id":&d.turn_id,"role":"assistant"
        }).await?.ok_or_else(channel::refused)?;
        // Text-only receipt. The web thread retains attachment access under its ACL.
        let mut text=late_delivery::answer(&message.text,message.status!="completed");
        if !message.attachments.is_empty() {text.push_str("\n\nAttachments are available in the NyxID thread.");}
        let delivery=rows.event.delivery.as_ref().ok_or_else(channel::refused)?;
        if rows.channel.transport=="gateway"
            && delivery.target_expires_at.is_none_or(|t|t<=Utc::now())
            && rows.event.created_at+ChronoDuration::minutes(29)<=Utc::now()
        {return Err(channel::refused());}
        let anchor = if rows.channel.transport == "gateway" && rows.channel.gateway_source.is_none() {
            let anchor = supplemental::event_anchor(state, &rows.event, &rows.channel, &rows.thread, &mut decision).await?
                .ok_or_else(channel::refused)?;
            if let Some(binding) = &delivery.origin.thread {
                if binding.source_message_id != anchor { return Err(channel::refused()); }
                decision.anchor = supplemental::Anchor::ThreadBinding;
            }
            if decrypt_agent_key(state, &rows.channel).await?.is_none() { return Err(channel::refused()); }
            Some(anchor)
        } else { None };
        Ok((rows,history,text,anchor))
    }.await;
    let (rows, history, text, anchor) = match preparation {
        Ok(p) => p,
        Err(
            AppError::Forbidden(_) | AppError::NotFound(_) | AppError::SecurityReadinessUnavailable,
        ) => {
            deliveries::finish(&state.db, &d, "refused").await?;
            decision
                .record_ids(state, &d.user_id, &d.conversation_id, Some(&d.turn_id))
                .await;
            return Ok(());
        }
        Err(error) => {
            return Err(error); // recoverable no-send preparation lease
        }
    };
    let claim = {
        #[cfg(test)]
        if injected_failure(TestProcessFailure::Claim) {
            Err(AppError::Internal("test claim failure".into()))
        } else {
            deliveries::claim(&state.db, &d).await
        }
        #[cfg(not(test))]
        {
            deliveries::claim(&state.db, &d).await
        }
    };
    match claim {
        Ok(true) => {}
        Ok(false) => return Ok(()),
        Err(
            AppError::Forbidden(_) | AppError::NotFound(_) | AppError::SecurityReadinessUnavailable,
        ) => {
            deliveries::finish(&state.db, &d, "refused").await?;
            decision
                .record_ids(state, &d.user_id, &d.conversation_id, Some(&d.turn_id))
                .await;
            return Ok(());
        }
        Err(e) => {
            return Err(e);
        }
    }
    let delivery = rows.event.delivery.as_ref().ok_or_else(channel::refused)?;
    let outcome = if let Some(anchor) = anchor {
        decision.dispatch(crate::handlers::nyxbot::supplemental::Egress::LocalRelay);
        let sent = crate::services::channel_reply_notice::supplemental(Box::pin(async {
            if delivery.origin.thread.is_some() {
                thread_follow::send_bound_reply(
                    state,
                    &rows.channel,
                    &d.user_id,
                    &delivery.origin,
                    &history.id,
                    &crate::services::channel_platform::OutboundReply {
                        text: Some(text.clone()),
                        attachments: Vec::new(),
                        reply_to_platform_message_id: None,
                        metadata: crate::services::channel_reply_plan::assistant_metadata(
                            &rows.channel.platform,
                        ),
                    },
                )
                .await
                .and_then(|outcome| outcome.error.map_or(Ok(()), Err))
            } else {
                direct_reply(state, &rows.channel, &anchor, &text, None).await
            }
        }))
        .await;
        decision.result(&sent);
        match sent {
            Ok(()) => "confirmed",
            Err(error) => match images::classify_outcome(&error) {
                images::ImageOutcome::Failed(_) => "rejected",
                _ => "unknown",
            },
        }
    } else if rows.channel.gateway_source.is_some() {
        decision.anchor = supplemental::Anchor::EventSource;
        decision.dispatch(crate::handlers::nyxbot::supplemental::Egress::GatewayEvent);
        native_responses::send_occurrence(
            state,
            &rows.channel,
            &delivery.origin,
            &history.id,
            &text,
            &d.id,
        )
        .await
        .unwrap_or("unknown")
    } else {
        decision.anchor = supplemental::Anchor::EventSource;
        decision.dispatch(crate::handlers::nyxbot::supplemental::Egress::LocalRelay);
        late_delivery::dispatch_occurrence(
            state,
            &rows.event,
            delivery,
            &rows.channel,
            &history,
            &text,
            Some(&d.id),
        )
        .await
        .unwrap_or("unknown")
    };
    deliveries::finish(&state.db, &d, outcome).await?;
    decision.outcome(outcome);
    decision
        .record_ids(state, &d.user_id, &d.conversation_id, Some(&d.turn_id))
        .await;
    tracing::info!(
        outcome,
        transport = rows.channel.transport,
        "Execution permission channel result delivery settled"
    );
    Ok(())
}

pub(crate) async fn for_turn(state: &AppState, turn: &str) {
    let result: AppResult<()> = async {
        let rows: Vec<Delivery> = state
            .db
            .collection::<Delivery>(DELIVERIES)
            .find(doc! {"turn_id":turn,"state":"pending"})
            .limit(1)
            .await?
            .try_collect()
            .await?;
        for row in rows {
            process(state, &row.id).await?;
        }
        Ok(())
    }
    .await;
    if let Err(e) = result {
        tracing::warn!(
            code = e.error_code(),
            "Execution permission channel result delivery deferred"
        );
    }
}

pub(crate) fn spawn_sweep(state: AppState) {
    state.shutdown.clone().spawn_worker(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(15));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            let Some(_work) = state.shutdown.tick(&mut interval).await else {
                break;
            };
            let result: AppResult<()> = async {
                for id in deliveries::due(&state.db).await? {
                    let Some(_claim) = state.shutdown.background() else {
                        break;
                    };
                    process(&state, &id).await?;
                }
                Ok(())
            }
            .await;
            if let Err(e) = result {
                tracing::warn!(
                    code = e.error_code(),
                    "Execution permission channel result sweep deferred"
                );
            }
        }
    });
}

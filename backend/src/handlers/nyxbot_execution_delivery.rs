//! Delivery adapters only; the service owns durable source/claim authorization.
use super::*;
use crate::{
    models::channel_turn_preparation::Receipt,
    models::{
        assistant_conversation::AssistantConversation,
        assistant_message::AssistantMessage,
        execution_permission_channel::{COLLECTION_NAME as DELIVERIES, Delivery},
    },
    services::{
        channel_platform::OutboundReply,
        channel_reply_plan as plan, channel_turn_preparation as boundary,
        execution_permission_channel::{self as channel, deliveries},
    },
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
            if !deliveries::finish_owned(&state.db, &d, "refused").await? {
                return Ok(());
            }
            decision
                .record_ids(state, &d.user_id, &d.conversation_id, Some(&d.turn_id))
                .await;
            return Ok(());
        }
        Err(error) => {
            return Err(error); // recoverable no-send preparation lease
        }
    };
    #[cfg(test)]
    if injected_failure(TestProcessFailure::Claim) {
        return Err(AppError::Internal("test claim failure".into()));
    }
    #[cfg(test)]
    crate::services::durable_wake_tests::after("execution_result_transport_preparation").await;
    let delivery = rows.event.delivery.as_ref().ok_or_else(channel::refused)?;
    let outbound = OutboundReply {
        text: Some(text),
        attachments: vec![],
        reply_to_platform_message_id: None,
        metadata: if rows.channel.gateway_source.is_none() {
            plan::assistant_metadata(&rows.channel.platform)
        } else {
            None
        },
    };
    // Native WhatsApp exposes its own text parts; other native gateways retain
    // their single-operation receipt. Never infer a prefix inside that operation.
    let parts: AppResult<Vec<OutboundReply>> = (|| {
        if rows.channel.gateway_source.is_some() {
            if matches!(
                rows.channel.gateway_source.as_ref(),
                Some(crate::models::nyxbot_channel::GatewaySource::WhatsApp { .. })
            ) {
                plan::text_parts(&outbound, "whatsapp")
            } else {
                Ok(vec![outbound])
            }
        } else {
            Ok(plan::ordinary(
                &outbound,
                &rows.channel.platform,
                rows.thread.platform_chat_id.as_deref().unwrap_or_default(),
            )?
            .unwrap_or_else(|| vec![outbound]))
        }
    })();
    let parts = match parts {
        Ok(parts) => parts,
        Err(error) => {
            if deliveries::finish_owned(&state.db, &d, "refused").await? {
                decision.result(&Err(error));
                decision
                    .record_ids(state, &d.user_id, &d.conversation_id, Some(&d.turn_id))
                    .await;
            }
            return Ok(());
        }
    };
    let total = parts.len() as u32;
    let prepared = match deliveries::plan(&state.db, &d, &plan::fingerprint(&parts), total).await {
        Ok(plan) => plan,
        Err(error @ AppError::Conflict(_)) => {
            // A changed plan cannot safely continue an accepted prefix. An old
            // claimant cannot retire a survivor because finish is claim-fenced.
            deliveries::finish_owned(&state.db, &d, "refused").await?;
            return Err(error);
        }
        Err(error) => return Err(error),
    };
    for (index, part) in parts.into_iter().enumerate().skip(prepared.next as usize) {
        let index = index as u32;
        let boundary_error = std::sync::Arc::new(tokio::sync::Mutex::new(None));
        let guard = {
            let db = state.db.clone();
            let d = d.clone();
            let error = boundary_error.clone();
            std::sync::Arc::new(move || {
                let db = db.clone();
                let d = d.clone();
                let error = error.clone();
                Box::pin(async move {
                    match deliveries::before_effect(&db, &d, index).await {
                        Ok(()) => Ok(()),
                        Err(cause) => {
                            *error.lock().await = Some(cause);
                            Err(channel::refused())
                        }
                    }
                }) as futures::future::BoxFuture<'static, AppResult<()>>
            })
        };
        decision.anchor = if delivery.origin.thread.is_some() {
            supplemental::Anchor::ThreadBinding
        } else {
            supplemental::Anchor::EventSource
        };
        decision.dispatch(if rows.channel.gateway_source.is_some() {
            supplemental::Egress::GatewayEvent
        } else {
            supplemental::Egress::LocalRelay
        });
        let result = boundary::with_effect(
            guard,
            crate::services::channel_reply_notice::supplemental(tokio::time::timeout(
                std::time::Duration::from_secs(deliveries::SEND_SECS as u64),
                Box::pin(send_part(
                    state,
                    &d,
                    &rows,
                    &history,
                    anchor.as_deref(),
                    part,
                    index,
                )),
            )),
        )
        .await;
        let result = if let Some(error) = boundary_error.lock().await.take() {
            Err(error)
        } else {
            result.unwrap_or_else(|_| {
                Err(AppError::ChannelPlatformError(
                    crate::services::channel_text_delivery::UNCERTAIN.into(),
                ))
            })
        };
        let outcome = match result {
            Ok(receipt) => {
                #[cfg(test)]
                crate::services::durable_wake_tests::after(
                    "execution_result_accepted_before_receipt",
                )
                .await;
                if !deliveries::accepted(&state.db, &d, index, total, &receipt).await? {
                    return Ok(());
                }
                #[cfg(test)]
                crate::services::durable_wake_tests::after("execution_result_receipt_committed")
                    .await;
                if index + 1 < total {
                    continue;
                }
                "confirmed"
            }
            Err(error) => {
                let outcome = match images::classify_outcome(&error) {
                    images::ImageOutcome::Failed(_) => "rejected",
                    _ => "unknown",
                };
                if deliveries::finish_owned(&state.db, &d, outcome).await? {
                    outcome
                } else if error.is_forbidden()
                    || matches!(
                        error,
                        AppError::NotFound(_)
                            | AppError::ValidationError(_)
                            | AppError::SecurityReadinessUnavailable
                    )
                {
                    if !deliveries::finish_owned(&state.db, &d, "refused").await? {
                        return Ok(());
                    }
                    "refused"
                } else {
                    return Err(error);
                }
            }
        };
        decision.outcome(outcome);
        decision
            .record_ids(state, &d.user_id, &d.conversation_id, Some(&d.turn_id))
            .await;
        tracing::info!(
            outcome,
            transport = rows.channel.transport,
            "Execution permission channel result delivery settled"
        );
        return Ok(());
    }
    Ok(())
}

async fn send_part(
    state: &AppState,
    d: &Delivery,
    rows: &channel::SourceRows,
    history: &AssistantConversation,
    anchor: Option<&str>,
    part: OutboundReply,
    index: u32,
) -> AppResult<Receipt> {
    let origin = &rows
        .event
        .delivery
        .as_ref()
        .ok_or_else(channel::refused)?
        .origin;
    if rows.channel.gateway_source.is_some() {
        let occurrence = if index == 0 {
            d.id.clone()
        } else {
            format!("{}:part:{index}", d.id)
        };
        return match native_responses::send_occurrence(
            state,
            &rows.channel,
            origin,
            &history.id,
            part.text.as_deref().unwrap_or_default(),
            &occurrence,
        )
        .await?
        {
            "confirmed" => Ok(Receipt::default()),
            "rejected" => Err(channel::refused()),
            _ => Err(AppError::ChannelPlatformError(
                crate::services::channel_text_delivery::UNCERTAIN.into(),
            )),
        };
    }
    if origin.thread.is_some() {
        let result = thread_follow::send_bound_reply(
            state,
            &rows.channel,
            &d.user_id,
            origin,
            &history.id,
            &part,
        )
        .await?;
        if let Some(error) = result.error {
            return Err(error);
        }
        return Ok(Receipt {
            platform_message_ids: result.message_ids,
            outbound_message_id: None,
        });
    }
    // Gateway-era and sealed sources retain the original local relay message.
    // Carry the planned metadata so rendered entities are not parsed twice.
    use crate::handlers::channel_relay as relay;
    let auth = route_key_auth(state, &rows.channel).await?;
    let headers = HeaderMap::new();
    let body = relay::AsyncReplyRequest {
        message_id: anchor.unwrap_or(&rows.event.event_id).to_owned(),
        thread_reply: false,
        reply: relay::AsyncReplyBody {
            text: part.text.clone(),
            metadata: part.metadata.clone(),
            attachments: vec![],
        },
    };
    let context = relay::resolve_reply_request_context(state, &headers, Some(&auth), &body).await?;
    let adapter = relay::reply_adapter(state, &context)?;
    let Json(result) =
        relay::deliver_prepared_reply(state, &headers, context, part, false, adapter.as_ref())
            .await?;
    Ok(Receipt {
        platform_message_ids: result.platform_message_id.into_iter().collect(),
        outbound_message_id: Some(result.message_id),
    })
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

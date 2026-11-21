//! Channel answers outlive their provider HTTP stream. The event owns a durable
//! barrier, while answer text stays exclusively in the ordinary assistant history.
use super::*;
use crate::{
    models::{
        assistant_conversation::AssistantConversation,
        assistant_message::{AssistantMessage, COLLECTION_NAME as MESSAGES},
        nyxbot_channel::ChannelTurnDelivery,
    },
    services::{channel_turn_delivery as delivery, channel_turn_preparation as preparation},
};

#[path = "nyxbot_late_text.rs"]
mod text;

// CMA's default hard limit is 600 s from ingress, not from provider admission.
// Finish early enough for its terminal frames and normal delivery pacing. An
// earlier CMA deadline/close is handled by Drop; this does not extend its limit.
pub(super) const STREAM_SECS: i64 = 570;
const INCOMPLETE: &str = "\n\n[Incomplete — I could not finish this turn.]";

pub(super) fn answer(text: &str, failed: bool) -> String {
    if failed {
        if text.trim().is_empty() {
            engine::TurnError::new("outcome_unknown").message
        } else {
            format!("{text}{INCOMPLETE}")
        }
    } else {
        bounded_reply(text)
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn prepare(
    state: &AppState,
    event: &str,
    channel: &NyxbotChannel,
    origin: &ChannelOrigin,
    sender: &Sender<'_>,
    guest: bool,
    addressed: bool,
    notice_language: Option<NoticeLanguage>,
) -> AppResult<()> {
    let events = state.db.collection::<NyxbotEvent>(EVENTS);
    let row = events
        .find_one(doc! {"_id": event, "channel_id": &channel.id,
        "user_id": &channel.user_id, "media.interrupted": {"$ne":true}, "turn_id": bson::Bson::Null})
        .await?
        .ok_or_else(|| AppError::Conflict("Channel event already admitted".into()))?;
    let mut metadata = ChannelTurnDelivery {
        restart_owner_verified: sender.owner_eligible
            && sender.original_message
            && !guest
            && channel.bot_owner_id.is_none()
            && channel.org_specialist.is_none()
            && origin.native_event_id.as_deref() == Some(event)
            && origin.thread.is_none()
            && channel.gateway_source.is_some()
            && sender.execution_kind() != crate::models::channel_thread::ThreadSenderKind::Bot
            && channel
                .owner_sender_ids
                .iter()
                .any(|owner| owner == sender.id),
        notice_language,
        execution_source_bound: false,
        execution_owner_verified: crate::services::execution_permission_channel::verified_owner(
            channel,
            sender.execution_kind(),
            sender.owner_eligible,
            sender.original_message,
            guest,
        ),
        sender_kind: Some(sender.execution_kind()),
        version: 1,
        state: if channel.gateway_source.is_some() && channel.transport == "gateway" {
            "replay"
        } else if channel.transport == "gateway" {
            "waiting"
        } else {
            "pending"
        }
        .into(),
        origin: origin.clone(),
        sender_id: sender.id.into(),
        guest,
        addressed,
        transport: channel.transport.clone(),
        source_route_id: channel.route_id.clone(),
        source_route_api_key_id: Some(channel.route_api_key_id.clone()),
        stream_deadline: row.created_at + ChronoDuration::seconds(STREAM_SECS),
        checked_at: row.created_at,
        claim_id: None,
        target_ciphertext: None,
        target_expires_at: None,
    };
    if metadata.execution_owner_verified
        && channel.transport == "gateway"
        && channel.gateway_source.is_none()
        && crate::services::execution_permission_enrollment::snapshot::channel_candidate(
            &state.db,
            &channel.user_id,
        )
        && crate::services::security_readiness_service::serving_report(
            &state.db,
            crate::services::execution_permission_channel::READINESS,
        )
        .await?
        .serving_ready
        && crate::services::execution_permission_enrollment::live_enabled(
            &state.db,
            &channel.user_id,
            None,
        )
        .await?
        && let Err(error) = fetch_target(state, &row, channel, &mut metadata).await
    {
        // Minting has no message effect. A gateway without this capability keeps
        // the original short event bound, never a new destination or a resend.
        tracing::debug!(
            code = error.error_code(),
            "Channel permission reply target unavailable"
        );
    }
    let metadata = bson::to_bson(&metadata)
        .map_err(|_| AppError::Internal("Channel delivery unavailable".into()))?;
    let prepared = events.update_one(doc! {"_id": event, "media.interrupted": {"$ne":true}, "turn_id": bson::Bson::Null, "org_notice": null},
        vec![doc! {"$set": {"delivery": {"$mergeObjects": [
            {"$literal": metadata},
            {"$cond": [{"$eq": ["$gateway_replay_claimed", true]}, {"state":"replay"},
                {"$cond":[{"$eq":["$gateway_observer_detached",true]},{"state":"pending"},{}]}]},
        ]}}}]).await?;
    if prepared.matched_count != 1 {
        return Err(AppError::Conflict("Channel event already admitted".into()));
    }
    Ok(())
}

struct StreamGuard {
    state: AppState,
    event: String,
    armed: bool,
}
impl StreamGuard {
    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for StreamGuard {
    fn drop(&mut self) {
        if self.armed {
            let state = self.state.clone();
            let event = self.event.clone();
            crate::services::shutdown::process().spawn_cleanup(async move {
                let _ = delivery::detach(&state.db, &event).await;
                if !state.shutdown.is_stopped() {
                    process(&state, &event).await;
                }
            });
        }
    }
}

pub(super) fn provider_stream(
    state: AppState,
    event_key: String,
    response_id: String,
    created: Value,
    receiver: broadcast::Receiver<Value>,
    admitted_at: chrono::DateTime<Utc>,
) -> Response {
    provider_stream_inner(
        state,
        event_key,
        response_id,
        created,
        receiver,
        admitted_at,
        false,
    )
}

/// Stream a custom-event response while retaining the direct relay delivery
/// barrier. Custom CMA channels use an existing direct chat as their approved
/// sink: the provider stream must expose the completion and the durable
/// worker must still send the same completion to that chat.
pub(super) fn provider_stream_direct(
    state: AppState,
    event_key: String,
    response_id: String,
    created: Value,
    receiver: broadcast::Receiver<Value>,
    admitted_at: chrono::DateTime<Utc>,
) -> Response {
    provider_stream_inner(
        state,
        event_key,
        response_id,
        created,
        receiver,
        admitted_at,
        true,
    )
}

fn provider_stream_inner(
    state: AppState,
    event_key: String,
    response_id: String,
    created: Value,
    receiver: broadcast::Receiver<Value>,
    admitted_at: chrono::DateTime<Utc>,
    direct_delivery: bool,
) -> Response {
    // Construct outside the generator so an unpolled/dropped body detaches too.
    let mut guard = StreamGuard {
        state: state.clone(),
        event: event_key.clone(),
        armed: true,
    };
    let deadline = admitted_at + ChronoDuration::seconds(STREAM_SECS);
    let wait = (deadline - Utc::now()).to_std().unwrap_or_default();
    let deadline = tokio::time::Instant::now() + wait;
    sse(async_stream::stream! {
        yield created;
        match tokio::time::timeout_at(deadline, final_reply(receiver)).await {
            Ok(Err(error)) if error.stage == "subscriber_closed" && may_detach(&state, &event_key).await.unwrap_or(true) => {
                let _ = delivery::detach(&state.db, &event_key).await;
                guard.disarm();
                for frame in message_frames(&response_id, None) { yield frame; }
                process(&state, &event_key).await;
            }
            Ok(outcome) => {
                let (allowed,org_guest,language)=org_stream_authority(&state,&event_key).await.unwrap_or((false,false,None));
                if !allowed {
                    let _=refuse(&state,&event_key).await;
                    guard.disarm();
                    for frame in message_frames(&response_id,None) {yield frame;}
                    return;
                }
                let outcome=outcome.map_err(|mut error| {
                    if org_guest {
                        let notice = Notice::org_guest_failure(Some(error.code));
                        *error = engine::TurnError::new("org_channel_guest_unavailable");
                        error.message = notice.text(language).into();
                    } else if error.code == "channel_retry_attachment_unavailable" {
                        error.message = Notice::RetryAttachmentUnavailable.text(language).into();
                    } else if error.code == "assistant_start_unavailable" {
                        error.message = Notice::DependencyUnavailable.text(language).into();
                    } else if error.code == "capacity_exceeded" {
                        error.message = crate::services::channel_notices::capacity(language,error.capacity_wait_ms());
                    }
                    error
                });
                let status = if outcome.is_ok() { "completed" } else { "failed" };
                // A completed empty result still owns a terminal delivery
                // decision. Never acquire the stream/send barrier for silence.
                if outcome.as_ref().is_ok_and(|text| text.trim().is_empty())
                    && silent_result(&state, &event_key).await.unwrap_or(false)
                {
                    guard.disarm();
                    for frame in message_frames(&response_id, None) { yield frame; }
                } else if direct_delivery {
                    // Direct custom destinations remain `pending`, so the
                    // durable worker owns the chat send. Do not claim the
                    // normal gateway stream barrier here: claiming it would
                    // prevent that worker from dispatching the approved relay
                    // receipt while still leaving the provider without a
                    // completion frame.
                    guard.disarm();
                    match outcome {
                        Ok(text) => {
                            let text = stream_images(&state, &event_key, &text, deadline.into_std()).await;
                            let wake_state = state.clone();
                            let wake_event = event_key.clone();
                            crate::services::shutdown::process().spawn(async move { process(&wake_state, &wake_event).await; });
                            for frame in message_frames(&response_id, Some(&text)) { yield frame; }
                        },
                        Err(error) if matches!(error.code, "channel_retry_attachment_unavailable" | "capacity_exceeded" | "outcome_unknown" | "guest_model_route_required" | "assistant_start_unavailable" | "org_channel_guest_unavailable" | "model_approval_required" | "model_policy_denied" | "model_org_approval_required") => {
                            let wake_state = state.clone();
                            let wake_event = event_key.clone();
                            crate::services::shutdown::process().spawn(async move { process(&wake_state, &wake_event).await; });
                            for frame in message_frames(&response_id, Some(&error.message)) { yield frame; }
                        },
                        Err(error) => {
                            let wake_state = state.clone();
                            let wake_event = event_key.clone();
                            crate::services::shutdown::process().spawn(async move { process(&wake_state, &wake_event).await; });
                            yield json!({"type": "response.failed", "response": {
                                "id": &response_id, "status": "failed",
                                "error": {"code": identifier(error.code), "message": "NyxBot could not finish this turn."}}});
                        }
                    }
                } else if delivery::claim_stream(&state.db, &event_key, status).await.unwrap_or(false) {
                    guard.disarm();
                    if let Ok(Some(event)) = state.db.collection::<NyxbotEvent>(EVENTS).find_one(doc! {"_id":&event_key}).await {
                        if let Some(turn) = &event.turn_id {
                            crate::turn_trace::milestone(turn, crate::turn_trace::Stage::Channel, None, Some(crate::turn_trace::Outcome::Streamed));
                        }
                        audit(&state, &event.user_id, "nyxbot_channel_answer_delivery", json!({"event_id":event.id,
                            "conversation_id":event.conversation_id,"turn_id":event.turn_id,"transport":"gateway",
                            "outcome":"streamed","reason":"platform_delivery_unconfirmed"})).await;
                    }
                    match outcome {
                        Ok(text) => {
                            let text = stream_images(&state, &event_key, &text, deadline.into_std()).await;
                            for frame in message_frames(&response_id, Some(&text)) { yield frame; }
                        },
                        Err(error) if matches!(error.code, "channel_retry_attachment_unavailable" | "capacity_exceeded" | "outcome_unknown" | "guest_model_route_required" | "assistant_start_unavailable" | "org_channel_guest_unavailable" | "model_approval_required" | "model_policy_denied" | "model_org_approval_required") => {
                            for frame in message_frames(&response_id, Some(&error.message)) { yield frame; }
                        },
                        Err(error) => yield json!({"type": "response.failed", "response": {
                            "id": &response_id, "status": "failed",
                            "error": {"code": identifier(error.code), "message": "NyxBot could not finish this turn."}}}),
                    }
                } else {
                    // The deadline/sweeper won: the final answer belongs to the
                    // asynchronous path, never to both writers.
                    let _ = delivery::detach(&state.db, &event_key).await;
                    guard.disarm();
                    for frame in message_frames(&response_id, None) { yield frame; }
                    process(&state, &event_key).await;
                }
            }
            Err(_) => {
                let _ = delivery::detach(&state.db, &event_key).await;
                guard.disarm();
                // Accepted work continues through its original durable target.
                // A deadline is not a failure and emits no progress message.
                for frame in message_frames(&response_id, None) { yield frame; }
            }
        }
    })
}

async fn may_detach(state: &AppState, event_key: &str) -> AppResult<bool> {
    let Some(event) = state
        .db
        .collection::<NyxbotEvent>(EVENTS)
        .find_one(doc! {"_id":event_key})
        .await?
    else {
        return Ok(false);
    };
    let (Some(conversation), Some(turn)) = (event.conversation_id, event.turn_id) else {
        return Ok(false);
    };
    if state.db.collection::<bson::Document>(crate::models::assistant_conversation::COLLECTION_NAME)
        .find_one(doc! {"_id":&conversation,"user_id":&event.user_id,"active_turn.turn_id":&turn,"active_turn.restart_admission.version":4}).projection(doc! {"_id":1}).await?.is_some() {return Ok(true)}
    Ok(state.db.collection::<bson::Document>(MESSAGES).find_one(doc! {"conversation_id":conversation,"user_id":event.user_id,"turn_id":turn,"role":"assistant","settled_by_attempt":{"$exists":true}}).projection(doc! {"_id":1}).await?.is_some())
}

/// A separate bounded worker: provider I/O must never stall the shared live
/// change stream or the team wake-up sweep. Old events have no delivery version
/// and are deliberately never picked up (old stream writers cannot claim it).
pub(crate) fn spawn_sweep(state: AppState) {
    state.shutdown.clone().spawn_worker(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(15));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            let Some(_work) = state.shutdown.tick(&mut interval).await else {
                break;
            };
            if sweep(&state).await.is_err() {
                tracing::warn!(stage = "sweep", "Channel answer delivery deferred");
            }
            if super::handoff_delivery::sweep(&state).await.is_err() {
                tracing::warn!(stage = "sweep", "Saved channel reply delivery deferred");
            }
            if super::images::recovery::sweep(&state).await.is_err() {
                tracing::warn!(stage = "sweep", "Attachment recovery deferred");
            }
        }
    });
}

pub(super) async fn sweep(state: &AppState) -> AppResult<()> {
    use crate::models::nyxbot_gateway_response::{COLLECTION_NAME as RESPONSES, GatewayResponse};
    let pending: Vec<GatewayResponse> = state
        .db
        .collection(RESPONSES)
        .find(doc! {"terminal":false,"expires_at":{"$gt":bson::DateTime::now()}})
        .sort(doc! {"checked_at":1})
        .limit(32)
        .await?
        .try_collect()
        .await?;
    for response in pending {
        let Some(_claim) = state.shutdown.background() else {
            break;
        };
        let event_key = &response.event_key;
        crate::services::nyxbot_response_recovery::refresh(
            &state.db,
            &state.encryption_keys,
            &response,
            |text| async move {
                stream_images(
                    state,
                    event_key,
                    &text,
                    std::time::Instant::now() + Duration::from_secs(135),
                )
                .await
            },
        )
        .await?;
    }
    let receipt_rows: Vec<NyxbotEvent> = state
        .db
        .collection(EVENTS)
        .find(preparation::gateway::due(Utc::now()))
        .hint(mongodb::options::Hint::Name(
            preparation::gateway::DUE_INDEX.into(),
        ))
        .sort(doc! {"delivery.receipt_due_at":1})
        .limit(32)
        .await?
        .try_collect()
        .await?;
    use futures::StreamExt;
    futures::stream::iter(receipt_rows)
        .for_each_concurrent(4, |event| async move {
            let Some(_work) = state.shutdown.background() else {
                return;
            };
            if text::resume_receipt(state, &event).await.is_err() {
                tracing::warn!(event_id=%event.id, "Gateway receipt observation deferred");
            }
        })
        .await;
    let rows: Vec<NyxbotEvent> = state
        .db
        .collection(EVENTS)
        .find(doc! {"$and":[preparation::due(), {
            "$or": [{"turn_id": {"$type": "string"}}, {"status": "queued"}],
        }]})
        .sort(doc! {"delivery.checked_at": 1})
        .limit(32)
        .await?
        .try_collect()
        .await?;
    futures::stream::iter(rows)
        .for_each_concurrent(4, |event| async move {
            let Some(_work) = state.shutdown.background() else {
                return;
            };
            process(state, &event.id).await;
        })
        .await;
    Ok(())
}

pub(crate) async fn process(state: &AppState, event_id: &str) {
    process_selected(state, event_id, false).await;
}

/// Direct callbacks already await settlement and deliver their own reply. Only
/// gateway streams can disappear while that detached turn keeps running. Avoid
/// racing the normal direct callback; the sweep still recovers a lost callback.
pub(crate) async fn settled(state: &AppState, event_id: &str) {
    process_selected(state, event_id, true).await;
}

async fn process_selected(state: &AppState, event_id: &str, gateway_only: bool) {
    match Box::pin(process_inner(state, event_id, gateway_only)).await {
        Ok(()) => {}
        Err(error) if error.is_forbidden() || matches!(error, AppError::NotFound(_)) => {
            let _ = refuse(state, event_id).await;
        }
        Err(_) => {
            x_diagnostics::for_event(&state.db, event_id, XOutcome::DeliveryDeferred).await;
            // No upstream error display: it may contain URLs, capabilities or text.
            tracing::warn!(
                event_id,
                stage = "prepare",
                "Channel answer delivery deferred"
            );
        }
    }
}

async fn refuse(state: &AppState, event_id: &str) -> AppResult<()> {
    if let Some(event) = state
        .db
        .collection::<NyxbotEvent>(EVENTS)
        .find_one_and_update(
            doc! {"_id": event_id, "$or":[
                {"delivery.state":{"$in":["waiting","pending"]}},
                {"delivery.state":"preparing_text","delivery.preparation.version":1,
                 "$expr":{"$lte":["$delivery.preparation.lease_until","$$NOW"]}},
            ]},
            doc! {"$set": {"delivery.state": "refused"}},
        )
        .await?
    {
        x_diagnostics::for_event(&state.db, event_id, XOutcome::AuthorityChanged).await;
        audit(
            state,
            &event.user_id,
            "nyxbot_channel_answer_refused",
            json!({
                "event_id": event.id, "turn_id": event.turn_id, "reason": "authority_changed",
            }),
        )
        .await;
    }
    Ok(())
}

async fn process_inner(state: &AppState, event_id: &str, gateway_only: bool) -> AppResult<()> {
    let events = state.db.collection::<NyxbotEvent>(EVENTS);
    let mut filter = preparation::due();
    filter.insert("_id", event_id);
    if gateway_only {
        filter.insert("delivery.transport", "gateway");
    }
    let Some(event) = events.find_one(filter).await? else {
        return Ok(());
    };
    let trace = event
        .turn_id
        .as_deref()
        .and_then(crate::turn_trace::TurnContext::for_turn);
    crate::turn_trace::scope(trace, async {
    let Some(mut d) = event.delivery.clone() else {
        return Ok(());
    };
    if d.state == "sending" {
        if d.transport == "gateway"
            && let Some(recovered) = preparation::gateway::recover(&state.db,event_id).await?
        {
            return Box::pin(text::resume_receipt(state, &recovered)).await;
        }
        if preparation::unconfirmed(&state.db,event_id,None).await? {
            x_diagnostics::for_event(&state.db,event_id,XOutcome::ReplyUnconfirmed).await;
            audit(state,&event.user_id,"nyxbot_channel_answer_delivery",
                json!({"event_id":event.id,"conversation_id":event.conversation_id,"turn_id":event.turn_id,
                    "transport":d.transport,"outcome":"unknown","reason":"interrupted_dispatch"})).await;
        }
        return Ok(());
    }
    let Some(conversation_id) = &event.conversation_id else {
        return Ok(());
    };
    let now = Utc::now();
    let checked = doc! {"$set": {"delivery.checked_at": bson::DateTime::from_chrono(now)}};
    if d.state == "waiting" {
        // Older observers could persist the disconnect and exit before detach.
        // The stream and replay claims still arbitrate against the release.
        let previous = state.db.collection::<bson::Document>(EVENTS)
            .find_one_and_update(doc! {"_id": event_id, "delivery.state": "waiting"}, checked)
            .projection(doc! {"_id": 0, "gateway_observer_detached": 1})
            .await?;
        let Some(previous) = previous else { return Ok(()); };
        if d.stream_deadline > now && previous.get_bool("gateway_observer_detached") != Ok(true) {
            return Ok(());
        }
        delivery::detach(&state.db, event_id).await?;
        d.state = "pending".into();
    } else {
        events.update_one(doc! {"_id": event_id}, checked).await?;
    }
    let conversation = engine::get(
        &state.db,
        crate::services::org_channel_service::event_actor(&event),
        conversation_id,
    )
    .await?;
    let Some((channel, thread)) = eligible(state, &event, &d, &conversation).await? else {
        refuse(state, event_id).await?;
        return Ok(());
    };
    if d.transport == "gateway" && d.target_ciphertext.is_none() {
        mint_target(state, &event, &channel, &mut d).await?;
    }
    let Some(turn_id) = &event.turn_id else {
        return Ok(());
    };
    let Some(message) = state.db.collection::<AssistantMessage>(MESSAGES).find_one(doc! {
        "user_id": crate::services::org_channel_service::event_actor(&event), "conversation_id": conversation_id, "turn_id": turn_id, "role": "assistant",
    }).await? else { return Ok(()); };
    if successful_silence(&message) {
        if delivery::silence(&state.db, event_id).await? {
            x_diagnostics::for_channel(&state.db, &channel, XOutcome::AgentSilent).await;
            crate::turn_trace::milestone(turn_id, crate::turn_trace::Stage::Channel, None, Some(crate::turn_trace::Outcome::Silent));
            audit(
                state,
                &event.user_id,
                "nyxbot_channel_answer_delivery",
                json!({"event_id": event.id, "turn_id": turn_id, "outcome": "silent"}),
            )
            .await;
        }
        return Ok(());
    }
    let text = settled_answer(&message, &d);
    // The first check authorizes preparation; each provider boundary revalidates
    // after credential and attachment loading.
    let claim = Uuid::new_v4().to_string();
    let Some(saved) = preparation::acquire(&state.db,event_id,&message.id,&claim).await? else {
        return Ok(());
    };
    let report = crate::services::channel_reply_notice::supplemental(event_images(
        state,
        &event,
        &d,
        &channel,
        &thread,
        &conversation,
        &message,
        None,
    ))
    .await;
    if d.transport == "gateway" {
        events.update_one(doc! {"_id":event_id}, doc! {"$set":{"status":if message.status=="completed" {"completed"} else {"failed"}}}).await?;
    }
    let Some(confirmed) = Box::pin(text::send(state,&event,&d,&channel,&thread,&conversation,&claim,saved,&text,&report)).await? else { return Ok(()); };
    let status=if confirmed { "sent" } else { "unknown" };
    crate::turn_trace::milestone(turn_id, crate::turn_trace::Stage::Channel, None, Some(if confirmed { crate::turn_trace::Outcome::Confirmed } else { crate::turn_trace::Outcome::Unknown }));
    x_diagnostics::for_channel(&state.db, &channel, if status == "sent" { XOutcome::ReplySent } else { XOutcome::ReplyUnconfirmed }).await;
    events.update_one(doc! {"_id": event_id}, doc! {"$set": {"status": if message.status == "completed" { "completed" } else { "failed" }}}).await?;
    audit(
        state,
        &event.user_id,
        "nyxbot_channel_answer_delivery",
        json!({
            "event_id": event.id, "conversation_id": conversation_id, "turn_id": turn_id,
            "transport": d.transport, "outcome": status,
            "images_attempted": report.attempted, "images_sent": report.sent, "images_failed": report.failed,
            "images_unknown": report.unknown, "images_skipped": report.skipped,
        }),
    )
    .await;
    Ok(())
    }).await
}

#[allow(clippy::too_many_arguments)]
async fn event_images(
    state: &AppState,
    event: &NyxbotEvent,
    delivery: &ChannelTurnDelivery,
    channel: &NyxbotChannel,
    thread: &NyxbotThread,
    conversation: &AssistantConversation,
    message: &AssistantMessage,
    deadline: Option<std::time::Instant>,
) -> super::images::ImageReport {
    use super::images::*;
    if message.status != "completed" && message.text.trim().is_empty() {
        return ImageReport::default();
    }
    let anchor = event_anchor(event, delivery);
    report_or_failure(
        state,
        ImageSendContext {
            source: crate::models::image_preparation::Source::Event {
                event_id: event.id.clone(),
            },
            owner: crate::services::org_channel_service::event_actor(event),
            conversation_id: &conversation.id,
            turn_id: &message.turn_id,
            channel,
            thread,
            anchor,
            audience: ImageAudience {
                owner_private: !delivery.guest
                    && (conversation.channel.is_none()
                        || (thread.kind.as_deref() == Some("private") && thread.owner_chat)),
            },
            deadline,

            initiated: None,
            fence: None,
        },
        &ImageSendPlan {
            delegated: message.delegated_images.references.clone(),
            attachments: message.attachments.clone(),
        },
    )
    .await
}

pub(super) async fn stream_images(
    state: &AppState,
    event_id: &str,
    text: &str,
    deadline: std::time::Instant,
) -> String {
    let result: AppResult<String> = async {
        let Some(event) = state.db.collection::<NyxbotEvent>(EVENTS).find_one(doc! {"_id":event_id}).await? else {
            return Ok(bounded_reply(text));
        };
        let (Some(delivery), Some(conversation_id), Some(turn_id)) = (&event.delivery, &event.conversation_id, &event.turn_id) else {
            return Ok(bounded_reply(text));
        };
        let conversation = engine::get(&state.db, crate::services::org_channel_service::event_actor(&event), conversation_id).await?;
        let Some((channel, thread)) = eligible(state, &event, delivery, &conversation).await? else {
            return Ok(bounded_reply(text));
        };
        let Some(message) = state.db.collection::<AssistantMessage>(MESSAGES).find_one(doc! {
            "user_id": crate::services::org_channel_service::event_actor(&event), "conversation_id": conversation_id, "turn_id": turn_id, "role":"assistant",
        }).await? else { return Ok(bounded_reply(text)); };
        let report = event_images(state, &event, delivery, &channel, &thread, &conversation, &message, Some(deadline)).await;
        Ok(super::images::append_failure(&bounded_reply(text), &report))
    }.await;
    match result {
        Ok(text) => text,
        Err(error) => {
            tracing::warn!(
                code = error.error_code(),
                event_id,
                "Stream image preparation failed"
            );
            bounded_reply(text)
        }
    }
}

pub(crate) async fn retry_expiry(
    state: &AppState,
    conversation: &AssistantConversation,
    source: &str,
) -> AppResult<chrono::DateTime<Utc>> {
    let turn = conversation
        .active_turn
        .as_ref()
        .ok_or(AppError::AssistantTurnRequired)?;
    let event = state.db.collection::<NyxbotEvent>(EVENTS)
        .find_one(doc! {"_id":source,"conversation_id":&conversation.id,"turn_id":&turn.turn_id,"delivery.version":1})
        .await?.ok_or(AppError::AssistantTurnRequired)?;
    let d = event
        .delivery
        .as_ref()
        .ok_or(AppError::AssistantTurnRequired)?;
    if crate::services::org_channel_service::event_actor(&event) != conversation.user_id {
        return Err(AppError::AssistantTurnRequired);
    }
    let mut expiry = event.expires_at;
    if let Some(target) = d.target_expires_at {
        expiry = expiry.min(target);
    } else if d.transport == "gateway" {
        expiry = expiry.min(event.created_at + ChronoDuration::minutes(29));
    }
    if expiry > Utc::now() {
        let (channel, _) = eligible(state, &event, d, conversation)
            .await?
            .ok_or(AppError::AssistantTurnRequired)?;
        if !crate::services::nyxbot_gateway_source::ready(&channel)
            && channel.gateway_source.is_some()
        {
            return Err(AppError::AssistantTurnRequired);
        }
        if channel.route_id != d.source_route_id
            || d.source_route_api_key_id.as_deref() != Some(&channel.route_api_key_id)
        {
            return Err(AppError::AssistantTurnRequired);
        }
        let route = channel
            .route_id
            .as_deref()
            .ok_or(AppError::AssistantTurnRequired)?;
        if state.db.collection::<bson::Document>(crate::models::channel_conversation::COLLECTION_NAME)
            .find_one(doc! {"_id":route,"user_id":bot_owner(&channel),"channel_bot_id":&channel.channel_bot_id,
                "agent_api_key_id":&channel.route_api_key_id,"is_active":true,"retired_by_transfer":{"$ne":true}})
            .projection(doc! {"_id":1}).await?.is_none() {
            return Err(AppError::AssistantTurnRequired);
        }
    }
    Ok(expiry)
}

pub(super) async fn eligible(
    state: &AppState,
    event: &NyxbotEvent,
    d: &ChannelTurnDelivery,
    conversation: &AssistantConversation,
) -> AppResult<Option<(NyxbotChannel, NyxbotThread)>> {
    eligible_with(state, event, d, conversation, None).await
}

pub(super) async fn eligible_with(
    state: &AppState,
    event: &NyxbotEvent,
    d: &ChannelTurnDelivery,
    conversation: &AssistantConversation,
    checked: Option<&crate::services::platform_bot_authority::PersonalRouteCheck>,
) -> AppResult<Option<(NyxbotChannel, NyxbotThread)>> {
    let (channel, thread) = if d.origin.thread.is_some() {
        // Each admitted native-thread turn has its own source binding. A newer
        // turn can update conversation.channel; never compare that full binding
        // with the older answer or replace the answer's original source.
        let thread = crate::services::channel_thread_follow_service::validate_delivery(
            &state.db,
            crate::services::org_channel_service::event_actor(event),
            &d.origin,
            &conversation.id,
        )
        .await?;
        (
            load_channel(state, &event.user_id, &d.origin.nyxbot_channel_id).await?,
            thread,
        )
    } else {
        let Some(target) = crate::services::channel_turn_delivery::target_with(
            &state.db,
            conversation,
            &d.origin,
            checked,
        )
        .await?
        else {
            return Ok(None);
        };
        target
    };
    if event.expires_at <= Utc::now() || channel.transport != d.transport {
        return Ok(None);
    }
    // An unavailable rollout/dependency leaves pending delivery retryable.
    require_org_access_with(state, &channel, checked).await?;
    if state
        .db
        .collection::<bson::Document>(crate::models::user::COLLECTION_NAME)
        .find_one(doc! {"_id": &event.user_id, "is_active": true})
        .await?
        .is_none()
    {
        return Ok(None);
    }
    let agent =
        crate::services::assistant_team_service::agent_for_conversation(&state.db, conversation)
            .await?;
    if agent.destroyed_at.is_some()
        || thread
            .agent_id
            .as_ref()
            .or(channel.agent_id.as_ref())
            .is_some_and(|id| id != &agent.id)
    {
        return Ok(None);
    }
    let custom_binding = if let Some(binding_id) = event.custom_binding_id.as_deref() {
        let Some(binding) = state
            .db
            .collection::<crate::models::nyxbot_event_binding::NyxbotEventBinding>(
                crate::models::nyxbot_event_binding::COLLECTION_NAME,
            )
            .find_one(doc! {"_id": binding_id, "user_id": &event.user_id})
            .await?
        else {
            return Ok(None);
        };
        let configured_destination = state
            .db
            .collection::<NyxbotThread>(THREADS)
            .find_one(doc! {
                "channel_id": &binding.destination_channel_id,
                "user_id": &binding.user_id,
                "partition": &binding.destination_partition,
            })
            .await?;
        let destination_matches = configured_destination.as_ref().is_some_and(|configured| {
            configured.last_message_id.is_some()
                && configured.platform_chat_id == thread.platform_chat_id
                && configured.platform_thread_id == thread.platform_thread_id
                && configured.relay_partition == thread.relay_partition
        });
        if binding.destination_channel_id != channel.id
            || event.partition != thread.partition
            || !event.partition.starts_with("custom:")
            || !destination_matches
            || crate::services::nyxbot_event_binding_service::authorize_provider(state, &binding)
                .await
                .is_err()
        {
            return Ok(None);
        }
        true
    } else {
        false
    };
    let admitted = if let Some(e) = &d.origin.org_execution {
        crate::services::org_channel_service::validate(
            &state.db,
            &d.origin,
            &conversation.user_id,
            &e.agent_id,
        )
        .await?;
        if e.guest != d.guest
            || channel.route_id != d.source_route_id
            || d.source_route_api_key_id.as_deref() != Some(channel.route_api_key_id.as_str())
        {
            return Ok(None);
        }
        let mut projected = channel.clone();
        projected.owner_sender_ids = if e.guest {
            Vec::new()
        } else {
            vec![d.sender_id.clone()]
        };
        if d.origin.thread.is_some() {
            if e.guest {
                chats::Admission::Guest
            } else {
                chats::Admission::Owner
            }
        } else {
            if d.sender_kind == Some(crate::models::channel_thread::ThreadSenderKind::Bot)
                && !chats::bot_guest_allowed(&thread, Some(d.addressed))
            {
                return Ok(None);
            }
            chats::admission(&projected, &thread, &d.sender_id, Some(d.addressed))
        }
    } else if d.sender_kind == Some(crate::models::channel_thread::ThreadSenderKind::Bot) {
        // Followed children inherit live consent from their settings chat,
        // checked by validate_delivery above. Their own default member fields
        // are not that policy. Every bot answer still retains guest authority.
        if !d.guest
            || (d.origin.thread.is_none() && !chats::bot_guest_allowed(&thread, Some(d.addressed)))
        {
            return Ok(None);
        }
        chats::Admission::Guest
    } else if custom_binding {
        if !d.guest || !d.addressed {
            return Ok(None);
        }
        chats::Admission::Guest
    } else {
        chats::admission(&channel, &thread, &d.sender_id, Some(d.addressed))
    };
    if d.origin.thread.is_none()
        && !matches!(
            (d.guest, admitted),
            (false, chats::Admission::Owner) | (true, chats::Admission::Guest)
        )
    {
        return Ok(None);
    }
    let bot = crate::services::platform_bot_authority::bot_with(
        &state.db,
        &channel.channel_bot_id,
        checked,
    )
    .await?;
    if !bot.is_active || bot.status == "suspended" {
        return Ok(None);
    }
    let key =
        key_service::get_api_key(&state.db, bot_owner(&channel), &channel.route_api_key_id).await?;
    if !key.is_active || key.expires_at.is_some_and(|t| t <= Utc::now()) {
        return Ok(None);
    }
    // This is a solicited reply, not a proactive post. Never use allow_posts
    // (or an owner identity) to bypass a guest/member/reply-mode refusal.
    Ok(Some((channel, thread)))
}

async fn event_reference(state: &AppState, event: &NyxbotEvent) -> AppResult<Zeroizing<String>> {
    let ciphertext = event
        .event_context_ciphertext
        .as_ref()
        .ok_or_else(|| AppError::NotFound("Channel event unavailable".into()))?;
    let bytes = Zeroizing::new(state.encryption_keys.decrypt(ciphertext).await?);
    let context: Value = serde_json::from_slice(&bytes)
        .map_err(|_| AppError::Internal("Channel event unavailable".into()))?;
    context["event_ref"]
        .as_str()
        .filter(|s| !s.is_empty())
        .map(|s| Zeroizing::new(s.to_owned()))
        .ok_or_else(|| AppError::NotFound("Channel event unavailable".into()))
}

async fn mint_target(
    state: &AppState,
    event: &NyxbotEvent,
    channel: &NyxbotChannel,
    d: &mut ChannelTurnDelivery,
) -> AppResult<()> {
    fetch_target(state, event, channel, d).await?;
    if let (Some(sealed), Some(expires)) = (&d.target_ciphertext, d.target_expires_at) {
        state.db.collection::<NyxbotEvent>(EVENTS).update_one(doc! {"_id": &event.id, "delivery.state":{"$in":["pending","preparing_text"]}, "delivery.target_ciphertext": bson::Bson::Null},
            doc! {"$set": {"delivery.target_ciphertext": bson::Binary {subtype: bson::spec::BinarySubtype::Generic, bytes: sealed.clone()}, "delivery.target_expires_at": bson::DateTime::from_chrono(expires)}}).await?;
    }
    let canonical = state
        .db
        .collection::<NyxbotEvent>(EVENTS)
        .find_one(doc! {
            "_id":&event.id,
        })
        .await?
        .and_then(|e| e.delivery)
        .ok_or_else(preparation::lost)?;
    d.target_ciphertext = canonical.target_ciphertext;
    d.target_expires_at = canonical.target_expires_at;
    Ok(())
}

async fn fetch_target(
    state: &AppState,
    event: &NyxbotEvent,
    channel: &NyxbotChannel,
    d: &mut ChannelTurnDelivery,
) -> AppResult<()> {
    if event.created_at + ChronoDuration::minutes(29) <= Utc::now() {
        return Ok(());
    }
    let reference = event_reference(state, event).await?;
    let Some(key) = decrypt_agent_key(state, channel).await? else {
        return Ok(());
    };
    let response = gateway_call(
        state,
        reqwest::Method::POST,
        &format!("/events/{}/reply-target", urlencode(&reference)),
        &key,
        Some(&json!({})),
        None,
    )
    .await?;
    if !(200..300).contains(&response.status) {
        return Ok(());
    }
    let (Some(target), Some(expires)) = (
        response.body["reply_target_ref"]
            .as_str()
            .filter(|s| !s.is_empty() && s.len() <= 4096),
        response.body["expires_at"]
            .as_i64()
            .and_then(|t| chrono::DateTime::from_timestamp(t, 0)),
    ) else {
        return Ok(());
    };
    if expires <= Utc::now() || expires > Utc::now() + ChronoDuration::hours(24) {
        return Ok(());
    }
    let sealed = state.encryption_keys.encrypt(target.as_bytes()).await?;
    d.target_ciphertext = Some(sealed);
    d.target_expires_at = Some(expires);
    Ok(())
}

pub(super) async fn dispatch_occurrence(
    state: &AppState,
    event: &NyxbotEvent,
    d: &ChannelTurnDelivery,
    channel: &NyxbotChannel,
    conversation: &AssistantConversation,
    text: &str,
    deadline: chrono::DateTime<Utc>,
) -> AppResult<crate::models::gateway_reply::Reply> {
    use crate::models::gateway_reply::{Observation, Outcome, Reply};
    let classified = |outcome| Reply {
        outcome,
        observation: Observation::default(),
    };
    let mut operation = crate::turn_trace::Operation::start(
        event
            .turn_id
            .as_deref()
            .and_then(crate::turn_trace::TurnContext::for_turn),
        crate::turn_trace::Stage::Channel,
    );
    let result: AppResult<crate::models::gateway_reply::Reply> =
        crate::turn_trace::scope(operation.context(), async {
            if d.origin.thread.is_some() && d.transport == "direct" {
                thread_follow::send(
                    state,
                    &event.user_id,
                    &d.origin,
                    &conversation.id,
                    text,
                    None,
                )
                .await?;
                return Ok(classified(Outcome::Confirmed));
            }
            if d.transport == "direct" {
                // Route-key auth rechecks the current route; an expired callback token
                // never extends the sender's authority. The exact inbound preserves topics.
                direct_reply(state, channel, &event.event_id, text, None).await?;
                return Ok(classified(Outcome::Confirmed));
            }
            let Some(key) = decrypt_agent_key(state, channel).await? else {
                return Ok(classified(Outcome::Rejected));
            };
            let (path, idempotency) = if let Some(ciphertext) = &d.target_ciphertext {
                if d.target_expires_at.is_none_or(|t| t <= Utc::now()) {
                    return Ok(classified(Outcome::Rejected));
                }
                let reference = Zeroizing::new(
                    String::from_utf8(state.encryption_keys.decrypt(ciphertext).await?)
                        .map_err(|_| AppError::Internal("Channel target unavailable".into()))?,
                );
                (
                    Zeroizing::new(format!("/reply-targets/{}/messages", urlencode(&reference))),
                    Some(event.id.as_str()),
                )
            } else {
                // Compatibility with gateways predating reply targets: the original
                // short-lived event is safe once, but never replaced by a newer event.
                if event.created_at + ChronoDuration::minutes(29) <= Utc::now() {
                    return Ok(classified(Outcome::Rejected));
                }
                let reference = event_reference(state, event).await?;
                (
                    Zeroizing::new(format!("/events/{}/replies", urlencode(&reference))),
                    None,
                )
            };
            let mut message = json!({"text": text});
            if d.origin.thread.is_some() {
                message["thread_reply"] = json!(true);
            }
            crate::services::nyxbot_gateway_service::reply_call(
                state,
                &path,
                &key,
                &message,
                idempotency,
                d.target_ciphertext.is_some(),
                deadline,
            )
            .await
        })
        .await;
    operation.finish(
        match &result {
            Ok(reply) if reply.outcome == Outcome::Confirmed => {
                crate::turn_trace::Outcome::Confirmed
            }
            Ok(reply) if reply.outcome == Outcome::Rejected => crate::turn_trace::Outcome::Rejected,
            _ => crate::turn_trace::Outcome::Unknown,
        },
        None,
        None,
    );
    result
}

fn settled_answer(message: &AssistantMessage, d: &ChannelTurnDelivery) -> String {
    if let Some(code) = message.error_code.as_deref() {
        let failure = if d.origin.org_execution.as_ref().is_some_and(|e| e.guest) {
            Notice::org_guest_failure(Some(code))
                .text(d.notice_language)
                .into()
        } else if code == "channel_retry_attachment_unavailable" {
            Notice::RetryAttachmentUnavailable
                .text(d.notice_language)
                .into()
        } else if code == "assistant_start_unavailable" {
            Notice::DependencyUnavailable.text(d.notice_language).into()
        } else if code == "capacity_exceeded" {
            crate::services::channel_notices::capacity(
                d.notice_language,
                message
                    .error_detail
                    .as_ref()
                    .and_then(|detail| detail.capacity_wait_ms),
            )
        } else {
            engine::TurnError::new(code)
                .with_model_route_reason(
                    message
                        .turn_metadata
                        .as_ref()
                        .and_then(|data| data.failure_reason.as_ref())
                        .and_then(|reason| reason.model_route.as_ref())
                        .map(|route| route.reason.as_str()),
                )
                .message
        };
        if message.text.trim().is_empty() {
            failure
        } else {
            format!("{}\n\n{}", message.text.trim(), failure)
        }
    } else {
        answer(&message.text, message.status != "completed")
    }
}

fn successful_silence(message: &AssistantMessage) -> bool {
    message.status == "completed"
        && message.error_code.is_none()
        && message.text.trim().is_empty()
        && !message.attachments.iter().any(engine::chat_tool_image)
}

async fn silent_result(state: &AppState, event_id: &str) -> AppResult<bool> {
    let Some(event) = state
        .db
        .collection::<NyxbotEvent>(EVENTS)
        .find_one(doc! {"_id": event_id})
        .await?
    else {
        return Ok(false);
    };
    let (Some(conversation), Some(turn)) = (&event.conversation_id, &event.turn_id) else {
        return Ok(false);
    };
    let Some(message) = state
        .db
        .collection::<AssistantMessage>(MESSAGES)
        .find_one(doc! {
            "user_id": crate::services::org_channel_service::event_actor(&event), "conversation_id": conversation,
            "turn_id": turn, "role": "assistant",
        })
        .await?
    else {
        return Ok(false);
    };
    if !successful_silence(&message) {
        return Ok(false);
    }
    delivery::silence(&state.db, event_id).await
}

// Ordinary channels retain their existing response path. Org channel answers
// must recheck the original actor and source immediately before streaming.
async fn org_stream_authority(
    state: &AppState,
    id: &str,
) -> AppResult<(bool, bool, Option<NoticeLanguage>)> {
    let Some(event) = state
        .db
        .collection::<NyxbotEvent>(EVENTS)
        .find_one(doc! {"_id":id})
        .await?
    else {
        return Ok((true, false, None));
    };
    let Some(d) = &event.delivery else {
        return Ok((true, false, None));
    };
    if d.origin.org_execution.is_none() {
        return Ok((true, false, d.notice_language));
    }
    let Some(conversation) = event.conversation_id.as_deref() else {
        return Ok((false, d.guest, d.notice_language));
    };
    let row = engine::get(
        &state.db,
        crate::services::org_channel_service::event_actor(&event),
        conversation,
    )
    .await?;
    Ok((
        eligible(state, &event, d, &row).await?.is_some(),
        d.guest,
        d.notice_language,
    ))
}

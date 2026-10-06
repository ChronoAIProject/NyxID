//! Channel answers outlive their provider HTTP stream. The event owns a durable
//! barrier, while answer text stays exclusively in the ordinary assistant history.
use super::*;
use crate::{
    models::{
        assistant_conversation::AssistantConversation,
        assistant_message::{AssistantMessage, COLLECTION_NAME as MESSAGES},
        nyxbot_channel::ChannelTurnDelivery,
    },
    services::channel_turn_delivery as delivery,
};

// CMA's default hard limit is 600 s from ingress, not from provider admission.
// Finish early enough for its terminal frames and normal delivery pacing. An
// earlier CMA deadline/close is handled by Drop; this does not extend its limit.
pub(super) const STREAM_SECS: i64 = 570;
const STILL_WORKING: &str =
    "I'm still working on this. I'll post the answer here when it is ready.";
const INCOMPLETE: &str = "\n\n[Incomplete — I could not finish this turn.]";

pub(super) fn answer(text: &str, failed: bool) -> String {
    if failed {
        if text.trim().is_empty() {
            "I could not finish that. Please try again.".into()
        } else {
            format!(
                "{}{INCOMPLETE}",
                excerpt(
                    text.trim(),
                    MAX_CHANNEL_REPLY_CHARS - INCOMPLETE.chars().count() - 2
                )
            )
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
    sender: &str,
    guest: bool,
    addressed: bool,
) -> AppResult<()> {
    let events = state.db.collection::<NyxbotEvent>(EVENTS);
    let row = events
        .find_one(doc! {"_id": event, "channel_id": &channel.id,
        "user_id": &channel.user_id, "turn_id": bson::Bson::Null})
        .await?
        .ok_or_else(|| AppError::Conflict("Channel event already admitted".into()))?;
    let metadata = ChannelTurnDelivery {
        version: 1,
        state: if channel.transport == "gateway" {
            "waiting"
        } else {
            "pending"
        }
        .into(),
        origin: origin.clone(),
        sender_id: sender.into(),
        guest,
        addressed,
        transport: channel.transport.clone(),
        stream_deadline: row.created_at + ChronoDuration::seconds(STREAM_SECS),
        checked_at: row.created_at,
        claim_id: None,
        target_ciphertext: None,
        target_expires_at: None,
    };
    events.update_one(doc! {"_id": event, "turn_id": bson::Bson::Null},
        doc! {"$set": {"delivery": bson::to_bson(&metadata).map_err(|_| AppError::Internal("Channel delivery unavailable".into()))?}}).await?;
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
            tokio::spawn(async move {
                let _ = delivery::detach(&state.db, &event).await;
                process(&state, &event).await;
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
            Ok(outcome) => {
                let status = if outcome.is_ok() { "completed" } else { "failed" };
                if delivery::claim_stream(&state.db, &event_key, status).await.unwrap_or(false) {
                    guard.disarm();
                    match outcome {
                        Ok(text) => for frame in message_frames(&response_id, Some(&bounded_reply(&text))) {
                            yield frame;
                        },
                        Err(code) => yield json!({"type": "response.failed", "response": {
                            "id": &response_id, "status": "failed",
                            "error": {"code": identifier(&code), "message": "NyxBot could not finish this turn."}}}),
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
                let text = if running(&state, &event_key).await.unwrap_or(false) { Some(STILL_WORKING) } else { None };
                for frame in message_frames(&response_id, text) { yield frame; }
            }
        }
    })
}

/// A separate bounded worker: provider I/O must never stall the shared live
/// change stream or the team wake-up sweep. Old events have no delivery version
/// and are deliberately never picked up (old stream writers cannot claim it).
pub(crate) fn spawn_sweep(state: AppState) {
    tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(15));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            interval.tick().await;
            if sweep(&state).await.is_err() {
                tracing::warn!(stage = "sweep", "Channel answer delivery deferred");
            }
        }
    });
}

pub(super) async fn sweep(state: &AppState) -> AppResult<()> {
    let rows: Vec<NyxbotEvent> = state.db.collection(EVENTS).find(doc! {
        "delivery.version": 1, "delivery.state": {"$in": ["waiting", "pending"]},
        "turn_id": {"$type": "string"}, "expires_at": {"$gt": bson::DateTime::from_chrono(Utc::now())},
    }).sort(doc! {"delivery.checked_at": 1}).limit(32).await?.try_collect().await?;
    use futures::StreamExt;
    futures::stream::iter(rows)
        .for_each_concurrent(4, |event| async move {
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
        Err(AppError::NotFound(_) | AppError::Forbidden(_)) => {
            let _ = refuse(state, event_id).await;
        }
        Err(_) => {
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
            doc! {"_id": event_id, "delivery.state": {"$in": ["waiting", "pending"]}},
            doc! {"$set": {"delivery.state": "refused"}},
        )
        .await?
    {
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
    let mut filter = doc! {"_id": event_id, "delivery.version": 1,
    "delivery.state": {"$in": ["waiting", "pending"]}};
    if gateway_only {
        filter.insert("delivery.transport", "gateway");
    }
    let Some(event) = events.find_one(filter).await? else {
        return Ok(());
    };
    let Some(mut d) = event.delivery.clone() else {
        return Ok(());
    };
    let (Some(conversation_id), Some(turn_id)) = (&event.conversation_id, &event.turn_id) else {
        return Ok(());
    };
    let now = Utc::now();
    events
        .update_one(
            doc! {"_id": event_id},
            doc! {"$set": {"delivery.checked_at": bson::DateTime::from_chrono(now)}},
        )
        .await?;
    if d.state == "waiting" && d.stream_deadline <= now {
        delivery::detach(&state.db, event_id).await?;
        d.state = "pending".into();
    }
    if d.state == "waiting" {
        return Ok(());
    }
    let conversation = engine::get(&state.db, &event.user_id, conversation_id).await?;
    let Some((channel, _)) = eligible(state, &event, &d, &conversation).await? else {
        refuse(state, event_id).await?;
        return Ok(());
    };
    // Mint while the original reference is live; this has no message effect.
    // Stored encrypted and bound to the original event, never the newest chat.
    if d.transport == "gateway" && d.target_ciphertext.is_none() {
        mint_target(state, &event, &channel, &mut d).await?;
    }
    let Some(message) = state.db.collection::<AssistantMessage>(MESSAGES).find_one(doc! {
        "user_id": &event.user_id, "conversation_id": conversation_id, "turn_id": turn_id, "role": "assistant",
    }).await? else { return Ok(()); };
    let text = answer(&message.text, message.status != "completed");
    // Preflight above can be slow: re-resolve authority immediately before the
    // irreversible barrier. Concurrent losers never reach a provider.
    let Some((channel, _)) = eligible(state, &event, &d, &conversation).await? else {
        return Ok(());
    };
    let claim = Uuid::new_v4().to_string();
    if !delivery::claim_send(&state.db, event_id, &claim).await? {
        return Ok(());
    }
    let result = dispatch(state, &event, &d, &channel, &conversation, &text).await;
    let status = match result {
        Ok(true) => "sent",
        Ok(false) | Err(_) => "unknown",
    };
    delivery::finish(&state.db, event_id, &claim, status).await?;
    events.update_one(doc! {"_id": event_id}, doc! {"$set": {"status": if message.status == "completed" { "completed" } else { "failed" }}}).await?;
    audit(
        state,
        &event.user_id,
        "nyxbot_channel_answer_delivery",
        json!({
            "event_id": event.id, "conversation_id": conversation_id, "turn_id": turn_id,
            "transport": d.transport, "outcome": status,
        }),
    )
    .await;
    Ok(())
}

async fn eligible(
    state: &AppState,
    event: &NyxbotEvent,
    d: &ChannelTurnDelivery,
    conversation: &AssistantConversation,
) -> AppResult<Option<(NyxbotChannel, NyxbotThread)>> {
    let (channel, thread) = if d.origin.thread.is_some() {
        // Each admitted native-thread turn has its own source binding. A newer
        // turn can update conversation.channel; never compare that full binding
        // with the older answer or replace the answer's original source.
        let thread = crate::services::channel_thread_follow_service::validate_delivery(
            &state.db,
            &event.user_id,
            &d.origin,
            &conversation.id,
        )
        .await?;
        (
            load_channel(state, &event.user_id, &d.origin.nyxbot_channel_id).await?,
            thread,
        )
    } else {
        let Some(target) = delivery_target(state, conversation, &d.origin).await? else {
            return Ok(None);
        };
        target
    };
    if event.expires_at <= Utc::now()
        || channel.transport != d.transport
        || !org_access_holds(state, &channel).await?
    {
        return Ok(None);
    }
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
    let admitted = chats::admission(&channel, &thread, &d.sender_id, Some(d.addressed));
    if d.origin.thread.is_none()
        && !matches!(
            (d.guest, admitted),
            (false, chats::Admission::Owner) | (true, chats::Admission::Guest)
        )
    {
        return Ok(None);
    }
    let bot = channel_bot_service::get_bot(&state.db, &channel.channel_bot_id).await?;
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
    state.db.collection::<NyxbotEvent>(EVENTS).update_one(doc! {"_id": &event.id, "delivery.target_ciphertext": bson::Bson::Null},
        doc! {"$set": {"delivery.target_ciphertext": bson::Binary {subtype: bson::spec::BinarySubtype::Generic, bytes: sealed.clone()}, "delivery.target_expires_at": bson::DateTime::from_chrono(expires)}}).await?;
    d.target_ciphertext = Some(sealed);
    d.target_expires_at = Some(expires);
    Ok(())
}

async fn dispatch(
    state: &AppState,
    event: &NyxbotEvent,
    d: &ChannelTurnDelivery,
    channel: &NyxbotChannel,
    conversation: &AssistantConversation,
    text: &str,
) -> AppResult<bool> {
    if d.origin.thread.is_some() {
        thread_follow::send(state, &event.user_id, &d.origin, &conversation.id, text).await?;
        return Ok(true);
    }
    if d.transport == "direct" {
        // Route-key auth rechecks the current route; an expired callback token
        // never extends the sender's authority. The exact inbound preserves topics.
        direct_reply(state, channel, &event.event_id, text, None).await?;
        return Ok(true);
    }
    let Some(key) = decrypt_agent_key(state, channel).await? else {
        return Ok(false);
    };
    let (path, idempotency) = if let Some(ciphertext) = &d.target_ciphertext {
        if d.target_expires_at.is_none_or(|t| t <= Utc::now()) {
            return Ok(false);
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
            return Ok(false);
        }
        let reference = event_reference(state, event).await?;
        (
            Zeroizing::new(format!("/events/{}/replies", urlencode(&reference))),
            None,
        )
    };
    let response = gateway_call(
        state,
        reqwest::Method::POST,
        &path,
        &key,
        Some(&json!({"text": text})),
        idempotency,
    )
    .await?;
    if !(200..300).contains(&response.status) {
        return Ok(false);
    }
    let outcome = response
        .body
        .pointer("/delivery/outcome")
        .or_else(|| response.body.pointer("/message/delivery/outcome"))
        .and_then(Value::as_str);
    Ok(outcome == Some("confirmed"))
}

async fn running(state: &AppState, event_id: &str) -> AppResult<bool> {
    let Some(event) = state
        .db
        .collection::<NyxbotEvent>(EVENTS)
        .find_one(doc! {"_id": event_id})
        .await?
    else {
        return Ok(false);
    };
    let (Some(conversation), Some(turn)) = (event.conversation_id, event.turn_id) else {
        return Ok(false);
    };
    let row = engine::get(&state.db, &event.user_id, &conversation).await?;
    Ok(engine::live_turn(&row, Utc::now())
        .is_some_and(|active| active.turn_id == turn && !active.stop_requested))
}

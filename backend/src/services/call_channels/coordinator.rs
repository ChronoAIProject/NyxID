//! One coordinator admits external guests into the normal specialist turn engine.
use super::{
    billing, binding, carrier,
    profiles::{self, Adapter},
    protocol::{self, Ingress, RelayFrame},
    session,
};
use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::channel_adapter::*,
    services::{
        assistant_nyxagent as engine, assistant_nyxagent_runtime as runtime, channel_adapters,
    },
};
use axum::{
    extract::ws::{Message, WebSocket},
    http::HeaderMap,
};
use chrono::{Duration, Utc};
use futures::{SinkExt, future::BoxFuture};
use mongodb::bson::{self, doc};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::sync::{broadcast, mpsc};
use zeroize::Zeroizing;

pub fn webhook_url(state: &AppState, b: &Binding) -> String {
    format!(
        "{}/api/v1/webhooks/call-adapters/{}",
        state.config.base_url.trim_end_matches('/'),
        b.id
    )
}
fn websocket_url(
    state: &AppState,
    b: &Binding,
    s: &CallSession,
    ticket: &str,
) -> AppResult<String> {
    let mut url = url::Url::parse(&format!("{}/socket/{}", webhook_url(state, b), s.id))
        .map_err(|_| AppError::VoiceProviderUnavailable)?;
    url.set_scheme(if url.scheme() == "https" { "wss" } else { "ws" })
        .map_err(|_| AppError::VoiceProviderUnavailable)?;
    url.query_pairs_mut().append_pair("ticket", ticket);
    Ok(url.into())
}
async fn ticket(state: &AppState, s: &CallSession) -> AppResult<Zeroizing<String>> {
    // Reproduce the same bearer for a retry of the same admitted TwiML response.
    // Only its digest is stored; it grants no additional socket claim.
    let raw = Zeroizing::new(
        crate::services::audit_service::keyed_fingerprint(
            crate::services::audit_service::audit_chain_hmac_key(),
            b"channel-relay-ticket",
            format!("{}:{}:{}", s.id, s.binding_id, s.binding_revision).as_bytes(),
        )
        .ok_or(AppError::VoiceProviderUnavailable)?,
    );
    let hash = hex::encode(Sha256::digest(raw.as_bytes()));
    let updated = state
        .db
        .collection::<CallSession>(SESSIONS)
        .update_one(
            doc! {"_id":&s.id,"socket_ticket_hash":bson::Bson::Null},
            doc! {"$set":{"socket_ticket_hash":&hash}},
        )
        .await?;
    if updated.modified_count != 1 {
        let saved = session::load(state, &s.id).await?;
        if saved.socket_ticket_hash.as_deref() != Some(hash.as_str()) {
            return Err(AppError::AssistantTurnRequired);
        }
    }
    Ok(raw)
}
pub fn ingress<'a>(
    state: AppState,
    id: &'a str,
    headers: &'a HeaderMap,
    body: &'a [u8],
) -> BoxFuture<'a, AppResult<String>> {
    ingress_correlated(state, id, headers, body, None)
}
// Construct admission on the heap before a handler polls it. Boxing only at
// an await site still embeds the unboxed construction temporary in that frame.
pub fn ingress_correlated<'a>(
    state: AppState,
    id: &'a str,
    headers: &'a HeaderMap,
    body: &'a [u8],
    send: Option<&'a str>,
) -> BoxFuture<'a, AppResult<String>> {
    Box::pin(ingress_correlated_inner(state, id, headers, body, send))
}
async fn ingress_correlated_inner(
    state: AppState,
    id: &str,
    headers: &HeaderMap,
    body: &[u8],
    send: Option<&str>,
) -> AppResult<String> {
    let b = binding::load(&state, id).await?;
    let authentication = binding::resolve(&state, &b.user_id, &b.credential, b.profile).await;
    match profiles::provider(b.profile) {
        "telnyx" => protocol::verify_telnyx(
            headers,
            body,
            binding::verification_key(&state, &b).await?.as_str(),
            Utc::now().timestamp(),
        )?,
        "twilio" => {
            let (_, secret) = protocol::twilio_key(
                &authentication
                    .as_ref()
                    .map_err(|_| {
                        AppError::ChannelWebhookVerificationFailed(
                            "Provider authentication unavailable".into(),
                        )
                    })?
                    .key,
                &b.account_id,
            )?;
            protocol::verify_twilio(
                headers,
                &send
                    .map(|id| format!("{}/sms-receipts/{id}", webhook_url(&state, &b)))
                    .unwrap_or_else(|| webhook_url(&state, &b)),
                &protocol::form(body)?,
                secret,
            )?;
        }
        _ => return Err(AppError::ChannelPlatformSendUnsupported),
    }
    let adapter = channel_adapters::resolve_nyxbot_profile(
        profiles::id(b.profile),
        &state.token_exchange_cache,
    )?;
    let event = match &adapter {
        Adapter::Messaging(a) => a.receive(&b, body)?,
        Adapter::Conversation(a) => a.receive(&b, body)?,
        _ => return Err(AppError::ChannelPlatformSendUnsupported),
    };
    if send.is_some() && !matches!(&event, Ingress::Receipt { .. }) {
        return Err(AppError::ChannelWebhookVerificationFailed(
            "Receipt event required".into(),
        ));
    }
    match event {
        Ingress::Call {
            event_id,
            call_id,
            from,
            to,
        } => {
            let r = binding::live(&state, &b).await?;
            billing::validate(&state, &b, &r).await?;
            let active = carrier::prove_call(&state, &b, &r, &call_id, &from, &to).await?;
            let (s, fresh) = session::prepare(&state, &b, &call_id, &from, &to).await?;
            session::receipt(&state, &b, &event_id, "call_started").await?;
            if !fresh && b.profile == Profile::TwilioRelay && s.cap_confirmed {
                session::fence(&state, &s).await?;
                let raw = ticket(&state, &s).await?;
                return Ok(protocol::twiml(
                    &websocket_url(&state, &b, &s, &raw)?,
                    &b.voice,
                    &b.language,
                ));
            }
            let resume_bootstrap = b.profile == Profile::TwilioRelay
                && s.bootstrap_dispatched
                && !s.cap_dispatched
                && active;
            if !(fresh || resume_bootstrap) {
                if b.profile == Profile::TwilioRelay {
                    return Err(AppError::Conflict(
                        "Call admission is already in progress; no command was replayed".into(),
                    ));
                }
                return Ok(String::new());
            }
            let meter = if fresh {
                Some(billing::reserve(&state, &b, &r, &s.billing_request_id).await?)
            } else {
                None
            };
            if b.profile == Profile::TwilioRelay && !active {
                // The bootstrap answers before the active-call TimeLimit update.
                // Recheck mutable provider fallback routing at that boundary.
                let live = binding::live(&state, &b).await?;
                for path in binding::resource_paths(&b) {
                    binding::authorize(&state, &b, &live, "GET", &path, None).await?;
                }
                binding::verify_resources(
                    crate::services::proxy_service::no_redirect_http_client(),
                    &b,
                    &live.key,
                    None,
                )
                .await?;
                if !session::command(&state, &s, "bootstrap_dispatched").await? {
                    return Ok("<Response><Hangup /></Response>".into());
                }
                session::fence(&state, &s).await?;
                if let Some(meter) = &meter {
                    state.billing.mark_forwarded(meter).await?;
                }
                // Pause answers the inbound leg. The following bounded, no-retry
                // Redirect reaches admission only once the call is active.
                return Ok(format!(
                    "<Response><Pause length=\"1\"/><Redirect method=\"POST\">{}#rc=0&amp;rt=5000&amp;tt=5000</Redirect><Hangup/></Response>",
                    protocol::xml(&webhook_url(&state, &b))
                ));
            }
            if let Adapter::Conversation(a) = adapter {
                if b.profile == Profile::TelnyxRelay {
                    if session::command(&state, &s, "answer_dispatched").await? {
                        if let Some(meter) = &meter {
                            state.billing.mark_forwarded(meter).await?;
                        }
                        if a.command(&state, &b, &r, &s, "answer", json!({}))
                            .await
                            .is_err()
                        {
                            mark_uncertain(&state, &s).await?;
                        }
                    }
                } else {
                    if !session::command(&state, &s, "cap_dispatched").await? {
                        return Err(AppError::AssistantTurnRequired);
                    }
                    if let Some(meter) = &meter {
                        state.billing.mark_forwarded(meter).await?;
                    }
                    if carrier::twilio_update(&state, &b, &r, &s, &[("TimeLimit", "1800")])
                        .await
                        .is_err()
                    {
                        mark_uncertain(&state, &s).await?;
                        return Err(AppError::VoiceProviderUnavailable);
                    }
                    state
                        .db
                        .collection::<CallSession>(SESSIONS)
                        .update_one(doc! {"_id":&s.id}, doc! {"$set":{"cap_confirmed":true}})
                        .await?;
                    let raw = ticket(&state, &s).await?;
                    state.db.collection::<CallSession>(SESSIONS).update_one(doc!{"_id":&s.id},doc!{"$set":{"answered":true,"connected_at":bson::DateTime::from_chrono(Utc::now())}}).await?;
                    let current = session::load(&state, &s.id).await?;
                    session::fence(&state, &current).await?;
                    let live = binding::live(&state, &b).await?;
                    if live.identity != s.credential_identity {
                        return Err(AppError::AssistantTurnRequired);
                    }
                    return Ok(protocol::twiml(
                        &websocket_url(&state, &b, &s, &raw)?,
                        &b.voice,
                        &b.language,
                    ));
                }
            }
        }
        Ingress::CallAnswered {
            event_id,
            call_id,
            at,
        } => {
            let r = binding::live(&state, &b).await?;
            let s = session::by_call(&state, &b, &call_id).await?;
            if session::receipt(&state, &b, &event_id, "answered").await? {
                state.db.collection::<CallSession>(SESSIONS).update_one(doc!{"_id":&s.id,"answered":false},doc!{"$set":{"answered":true,"connected_at":bson::DateTime::from_chrono(at.unwrap_or_else(Utc::now))}}).await?;
            }
            let s = session::load(&state, &s.id).await?;
            if session::command(&state, &s, "relay_dispatched").await? {
                let raw = ticket(&state, &s).await?;
                if let Adapter::Conversation(a) = adapter {
                    let payload = json!({"url":websocket_url(&state,&b,&s,&raw)?,"voice":b.voice,"tts_provider":"telnyx","voice_settings":{"type":"telnyx"},"language":b.language,"dtmf_detection":false,"interruptible":"any","transcription_engine":"Deepgram","transcription_engine_config":{"transcription_model":"deepgram/nova-3"}});
                    if a.command(&state, &b, &r, &s, "conversation_relay_start", payload)
                        .await
                        .is_err()
                    {
                        mark_uncertain(&state, &s).await?;
                    }
                }
            }
        }
        Ingress::RelayEnded { event_id, call_id } => {
            let s = session::by_call(&state, &b, &call_id).await?;
            session::receipt(&state, &b, &event_id, "relay_ended").await?;
            close(&state, &b, &s, false).await?;
        }
        Ingress::CallEnded {
            event_id,
            call_id,
            at,
            seconds,
        } => {
            let s = session::by_call(&state, &b, &call_id).await?;
            session::receipt(&state, &b, &event_id, "call_ended").await?;
            // A BYOK Telnyx verification key is owner supplied. It can stop
            // work but cannot attest charge quantities; query the provider API.
            let (at, seconds) = if b.profile == Profile::TelnyxRelay
                && matches!(b.credential, CredentialBinding::Connection { .. })
            {
                let _ = engine::request_stop(&state.db, &s.user_id, &s.conversation_id).await;
                let r = binding::resolve(&state, &b.user_id, &b.credential, b.profile).await?;
                let Some((at, seconds)) = carrier::terminal_status(&state, &b, &r, &s).await?
                else {
                    return Err(AppError::VoiceProviderUnavailable);
                };
                (Some(at), Some(seconds))
            } else {
                (at, seconds)
            };
            let frozen = session::terminal(
                &state,
                &s,
                at.unwrap_or_else(Utc::now),
                seconds,
                "authenticated_provider_terminal",
            )
            .await?;
            finalize_call(&state, &frozen).await?;
        }
        Ingress::Sms {
            event_id,
            message_id,
            from,
            to,
            text,
            parts,
        } => {
            let r = binding::live(&state, &b).await?;
            let parts = if b.profile == Profile::TelnyxSms
                && matches!(b.credential, CredentialBinding::Connection { .. })
            {
                carrier::message_parts(&state, &b, &r, &message_id, &from, &to, true).await?
            } else {
                parts
            };
            let (s, fresh) =
                session::prepare(&state, &b, &format!("sms:{message_id}"), &from, &to).await?;
            if !fresh {
                return Ok("<Response />".into());
            }
            // Each signed inbound part has its own normal request meter.
            for part in 0..parts {
                let m =
                    billing::reserve(&state, &b, &r, &format!("adapter-sms-in:{}:{part}", s.id))
                        .await?;
                state.billing.mark_forwarded(&m).await?;
                billing::settle(&state, &m, 0).await?;
            }
            let task_state = state.clone();
            let sms_binding = b.clone();
            tokio::spawn(async move {
                let sid = s.id.clone();
                let _ = sms_turn(task_state.clone(), sms_binding, s, event_id, text).await;
                let _ = session::finish_noncall(&task_state, &sid, "sms_turn_finished").await;
            });
        }
        Ingress::Receipt {
            event_id,
            message_id,
            from,
            to,
            status,
            parts,
        } => {
            super::sms_delivery::record_correlated(
                &state,
                &b,
                super::sms_delivery::Receipt {
                    event: &event_id,
                    message: &message_id,
                    from: &from,
                    to: &to,
                    status: &status,
                    parts,
                    send,
                },
            )
            .await?;
        }
        Ingress::Ignore => {}
    }
    Ok(if profiles::provider(b.profile) == "twilio" {
        "<Response />".into()
    } else {
        String::new()
    })
}
async fn mark_uncertain(state: &AppState, s: &CallSession) -> AppResult<()> {
    state
        .db
        .collection::<CallSession>(SESSIONS)
        .update_one(
            doc! {"_id":&s.id},
            doc! {"$set":{"state":"uncertain","delivery":"unknown"}},
        )
        .await?;
    Ok(())
}

pub struct Output {
    pub turn_id: String,
    pub generation: i64,
    pub text: String,
    pub last: bool,
    pub error: bool,
}
pub(crate) async fn turn(
    state: AppState,
    b: &Binding,
    s: &CallSession,
    event: &str,
    text: &str,
) -> AppResult<(
    mpsc::Receiver<Output>,
    tokio::task::JoinHandle<Option<runtime::OwnerTurnSettlement>>,
)> {
    turn_with_history(state, b, s, event, text, None).await
}
pub(crate) async fn turn_with_history(
    state: AppState,
    b: &Binding,
    s: &CallSession,
    event: &str,
    text: &str,
    history: Option<Vec<String>>,
) -> AppResult<(
    mpsc::Receiver<Output>,
    tokio::task::JoinHandle<Option<runtime::OwnerTurnSettlement>>,
)> {
    let permit = state
        .direct_chat_limiter
        .try_acquire_pool(
            "assistant_channel",
            &b.user_id,
            crate::services::assistant_team_service::CHANNEL_TURNS_PER_OWNER,
        )
        .await?
        .ok_or(AppError::VoiceQueueFull)?;
    let claim = session::admit(&state, b, s, event, text, history).await?;
    let (events, mut receiver) = broadcast::channel::<Value>(256);
    let (sender, output) = mpsc::channel(64);
    let row = claim.conversation;
    let admitted_turn_id = row.active_turn.as_ref().map(|t| t.turn_id.clone());
    let stream_turn_id = admitted_turn_id
        .clone()
        .ok_or(AppError::AssistantTurnRequired)?;
    let terminal_turn_id = stream_turn_id.clone();
    let text = text.to_owned();
    let credential = claim.credential;
    let stream_state = state.clone();
    let stream_session = s.clone();
    let terminal = sender.clone();
    let relay = tokio::spawn(async move {
        loop {
            match receiver.recv().await {
                Ok(event) => {
                    if event["event"] != "block.delta" {
                        continue;
                    }
                    let Some(text) = event["text"].as_str() else {
                        continue;
                    };
                    if session::fence(&stream_state, &stream_session)
                        .await
                        .is_err()
                        || !matches!(
                            tokio::time::timeout(
                                std::time::Duration::from_secs(5),
                                sender.send(Output {
                                    turn_id: stream_turn_id.clone(),
                                    generation: stream_session.generation,
                                    text: text.into(),
                                    last: false,
                                    error: false,
                                })
                            )
                            .await,
                            Ok(Ok(()))
                        )
                    {
                        let _ = engine::request_stop(
                            &stream_state.db,
                            &stream_session.user_id,
                            &stream_session.conversation_id,
                        )
                        .await;
                        return false;
                    }
                }
                Err(broadcast::error::RecvError::Closed) => return true,
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    let _ = engine::request_stop(
                        &stream_state.db,
                        &stream_session.user_id,
                        &stream_session.conversation_id,
                    )
                    .await;
                    return false;
                }
            }
        }
    });
    let worker_session = s.clone();
    let egress = crate::services::assistant_nyxagent_egress::channel(state.clone(), s.clone());
    let worker = tokio::spawn(async move {
        let result = runtime::run_owner_turn(
            state.clone(),
            egress,
            row,
            text,
            credential,
            Some(
                crate::services::billing::route_inventory::BillingRoutePolicy::Metered(
                    crate::services::billing::BillingIngress::Proxy,
                ),
            ),
            permit,
            runtime::Events {
                checkpoint: None,
                sender: events,
                cursor: 0,
                reset_notified: false,
            },
        )
        .await;
        let complete =
            relay.await.unwrap_or(false) && result.as_ref().is_some_and(|s| s.error.is_none());
        let _ = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            terminal.send(Output {
                turn_id: terminal_turn_id,
                generation: worker_session.generation,
                text: String::new(),
                last: complete,
                error: !complete,
            }),
        )
        .await;
        if let Some(settled) = result.as_ref() {
            let current = session::load(&state, &worker_session.id)
                .await
                .ok()
                .filter(|s| s.reply_turn_id == admitted_turn_id);
            let evidence = crate::models::assistant_message::ChannelDelivery {
                status: current
                    .as_ref()
                    .map(|s| s.delivery.clone())
                    .unwrap_or_else(|| "generated_not_confirmed".into()),
                heard_prefix_chars: current.as_ref().and_then(|s| s.heard_prefix_chars),
                heard_prefix_matches_sent: current
                    .as_ref()
                    .is_some_and(|s| s.heard_prefix_matches_sent),
            };
            let _=state.db.collection::<bson::Document>(crate::models::assistant_message::COLLECTION_NAME).update_one(doc!{"_id":&settled.message_id,"conversation_id":&worker_session.conversation_id},doc!{"$set":{"turn_metadata.channel_delivery":bson::to_bson(&evidence).unwrap_or(bson::Bson::Null)}}).await;
        }
        let _ = state
            .db
            .collection::<CallSession>(SESSIONS)
            .update_one(
                doc! {"_id":&worker_session.id,"active_turn_id":admitted_turn_id},
                doc! {"$set":{"active_turn_id":bson::Bson::Null}},
            )
            .await;
        result
    });
    Ok((output, worker))
}
async fn sms_turn(
    state: AppState,
    b: Binding,
    s: CallSession,
    event: String,
    text: String,
) -> AppResult<()> {
    let s = session::activate_noncall(&state, &b, &s, None).await?;
    let (mut output, worker) = turn(state.clone(), &b, &s, &format!("sms:{event}"), &text).await?;
    while output.recv().await.is_some() {}
    let Some(settled) = worker.await.map_err(|_| AppError::AssistantTurnRequired)? else {
        return Ok(());
    };
    if settled.error.is_some() {
        return Ok(());
    }
    let current = session::load(&state, &s.id).await?;
    session::fence(&state, &current).await?;
    let r = binding::live(&state, &b).await?;
    let text = if profiles::sms_parts(b.profile, &settled.text).is_ok() {
        settled.text
    } else {
        "This answer exceeds the SMS limit. Please ask a more specific question.".into()
    };
    let reserved_parts = profiles::sms_parts(b.profile, &text)?;
    let send_id = uuid::Uuid::new_v4().to_string();
    let callback = format!("{}/sms-receipts/{send_id}", webhook_url(&state, &b));
    let payload = profiles::sms_payload(&b, &s.from, &text, &callback)?;
    let path = if b.profile == Profile::TelnyxSms {
        "/v2/messages".into()
    } else {
        format!("/2010-04-01/Accounts/{}/Messages.json", b.account_id)
    };
    binding::authorize(&state, &b, &r, "POST", &path, Some(&payload)).await?;
    let mut meters = Vec::new();
    for part in 0..reserved_parts {
        match billing::reserve(&state, &b, &r, &format!("adapter-sms-out:{}:{part}", s.id)).await {
            Ok(meter) => meters.push(meter),
            Err(error) => {
                for meter in &meters {
                    state
                        .billing
                        .settle(
                            meter,
                            crate::models::service_billing::PlatformUsage::default(),
                            None,
                            None,
                        )
                        .await?;
                }
                return Err(error);
            }
        }
    }
    let row = EventReceipt {
        id: send_id,
        identity_key: session::identity(&[&s.id, "sms_send"]),
        user_id: b.user_id.clone(),
        binding_id: b.id.clone(),
        binding_revision: b.revision,
        session_id: Some(s.id.clone()),
        event_kind: "sms_send".into(),
        status: "dispatching".into(),
        provider_message_id: None,
        delivery: "unknown".into(),
        billing_request_id: Some(format!("adapter-sms-out:{}:0", s.id)),
        observed_parts: None,
        reserved_parts: Some(reserved_parts),
        billing_finalized: false,
        from: Some(b.phone_number.clone()),
        to: Some(s.from.clone()),
        created_at: Utc::now(),
    };
    if let Err(error) = session::insert_sms_send(&state, &b, &current, &row).await {
        // No provider dispatch occurred. Release every opened part reservation.
        for meter in &meters {
            state
                .billing
                .settle(
                    meter,
                    crate::models::service_billing::PlatformUsage::default(),
                    None,
                    None,
                )
                .await?;
        }
        return Err(error);
    }
    let Adapter::Messaging(a) = channel_adapters::resolve_nyxbot_profile(
        profiles::id(b.profile),
        &state.token_exchange_cache,
    )?
    else {
        return Err(AppError::ChannelPlatformSendUnsupported);
    };
    for meter in &meters {
        if let Err(error) = state.billing.mark_forwarded(meter).await {
            for meter in &meters {
                state
                    .billing
                    .settle(
                        meter,
                        crate::models::service_billing::PlatformUsage::default(),
                        None,
                        None,
                    )
                    .await?;
            }
            state.db.collection::<EventReceipt>(EVENTS).update_one(doc!{"_id":&row.id},doc!{"$set":{"status":"rejected_before_dispatch","billing_finalized":true,"delivery":"failed"}}).await?;
            return Err(error);
        }
    }
    // Billing admission can wait; revalidate the full turn/connection/policy
    // after reservations and immediately before the irreversible provider POST.
    let dispatch = sms_dispatch_authority(&state, &b, &s, &r.identity, &path, &payload).await;
    let live = match dispatch {
        Ok(live) => live,
        Err(error) => {
            for meter in &meters {
                state
                    .billing
                    .settle(
                        meter,
                        crate::models::service_billing::PlatformUsage::default(),
                        None,
                        None,
                    )
                    .await?;
            }
            state.db.collection::<EventReceipt>(EVENTS).update_one(doc!{"_id":&row.id},doc!{"$set":{"status":"rejected_before_dispatch","billing_finalized":true,"delivery":"failed"}}).await?;
            return Err(error);
        }
    };
    match a
        .send(
            crate::services::proxy_service::no_redirect_http_client(),
            &b,
            &live.key,
            &s.from,
            &text,
            &callback,
        )
        .await
    {
        Ok(accepted) => {
            state.db.collection::<EventReceipt>(EVENTS).update_one(doc!{"_id":&row.id,"status":"dispatching"},doc!{"$set":{"status":"accepted","delivery":"accepted","provider_message_id":accepted.id,"observed_parts":accepted.parts}}).await?;
            super::sms_delivery::reconcile(&state, &row.id).await?;
        }
        Err(profiles::SmsFailure::Rejected(error)) => {
            state
                .db
                .collection::<EventReceipt>(EVENTS)
                .update_one(
                    doc! {"_id":&row.id},
                    doc! {"$set":{"status":"rejected","delivery":"failed","observed_parts":0}},
                )
                .await?;
            super::sms_delivery::reconcile(&state, &row.id).await?;
            return Err(error);
        }
        Err(profiles::SmsFailure::Uncertain) => {
            state
                .db
                .collection::<EventReceipt>(EVENTS)
                .update_one(
                    doc! {"_id":&row.id,"status":"dispatching"},
                    doc! {"$set":{"status":"uncertain"}},
                )
                .await?;
        }
    }
    session::finish_noncall(&state, &s.id, "sms_turn_finished").await?;
    Ok(())
}

pub(super) async fn sms_dispatch_authority(
    state: &AppState,
    b: &Binding,
    s: &CallSession,
    admitted_identity: &str,
    path: &str,
    payload: &[u8],
) -> AppResult<binding::Resolved> {
    let current = session::load(state, &s.id).await?;
    session::fence(state, &current).await?;
    let live = binding::live(state, b).await?;
    if live.identity != admitted_identity {
        return Err(AppError::AssistantTurnRequired);
    }
    binding::authorize(state, b, &live, "POST", path, Some(payload)).await?;
    let row = state.db.collection::<crate::models::assistant_conversation::AssistantConversation>(crate::models::assistant_conversation::COLLECTION_NAME)
        .find_one(doc! {"_id":&s.conversation_id,"user_id":&s.user_id,"guest_turn":true,"active_turn":bson::Bson::Null}).await?;
    if row.is_none() {
        return Err(AppError::AssistantTurnRequired);
    }
    Ok(live)
}

pub async fn authenticate_socket(
    state: &AppState,
    b: &Binding,
    s: &CallSession,
    headers: &HeaderMap,
    raw: &str,
) -> AppResult<()> {
    if raw.len() != 64
        || s.socket_ticket_hash.as_deref()
            != Some(hex::encode(Sha256::digest(raw.as_bytes())).as_str())
        || s.claim_id.is_some()
        || s.state != SessionState::Prepared
        || Utc::now() > s.lease_expires_at
    {
        return Err(AppError::ChannelWebhookVerificationFailed(
            "Socket verification failed".into(),
        ));
    }
    let r = binding::live(state, b).await?;
    if b.profile == Profile::TwilioRelay {
        let (_, secret) = protocol::twilio_key(&r.key, &b.account_id)?;
        protocol::verify_twilio_upgrade(headers, &websocket_url(state, b, s, raw)?, secret)?;
    }
    carrier::prove_call(state, b, &r, &s.provider_call_id, &s.from, &s.to).await?;
    Ok(())
}
pub async fn socket(state: AppState, b: Binding, s: CallSession, mut socket: WebSocket) {
    let claim = uuid::Uuid::new_v4().to_string();
    let won = state
        .db
        .collection::<CallSession>(SESSIONS)
        .update_one(
            doc! {"_id":&s.id,"claim_id":bson::Bson::Null,"state":"prepared"},
            doc! {"$set":{"claim_id":&claim,"state":"active"}},
        )
        .await
        .is_ok_and(|r| r.modified_count == 1);
    if !won {
        let _ = tokio::time::timeout(std::time::Duration::from_secs(1), socket.close()).await;
        return;
    }
    let Ok(owned) = session::load(&state, &s.id).await else {
        return;
    };
    let result = socket_inner(&state, &b, &owned, &mut socket).await;
    let end = channel_adapters::resolve_nyxbot_profile(
        profiles::id(b.profile),
        &state.token_exchange_cache,
    )
    .ok()
    .and_then(|a| match a {
        Adapter::Conversation(a) => Some(a.end()),
        _ => None,
    })
    .unwrap_or_else(protocol::end_frame);
    let _ = tokio::time::timeout(
        std::time::Duration::from_secs(1),
        socket.send(Message::Text(end.to_string().into())),
    )
    .await;
    let _ = tokio::time::timeout(std::time::Duration::from_secs(1), socket.close()).await;
    if session::load(&state, &s.id)
        .await
        .is_ok_and(|row| row.claim_id.as_deref() == Some(claim.as_str()))
    {
        let _ = close(&state, &b, &owned, false).await;
    }
    if result.is_err() {
        tracing::info!(binding_id=%b.id,session_id=%s.id,"Carrier relay ended");
    }
}
async fn socket_inner(
    state: &AppState,
    b: &Binding,
    initial: &CallSession,
    socket: &mut WebSocket,
) -> AppResult<()> {
    let claim = initial
        .claim_id
        .as_deref()
        .ok_or(AppError::AssistantTurnRequired)?
        .to_owned();
    let mut s = session::load(state, &initial.id).await?;
    let Adapter::Conversation(adapter) = channel_adapters::resolve_nyxbot_profile(
        profiles::id(b.profile),
        &state.token_exchange_cache,
    )?
    else {
        return Err(AppError::ChannelPlatformSendUnsupported);
    };
    let first = tokio::time::timeout(std::time::Duration::from_secs(10), socket.recv())
        .await
        .map_err(|_| AppError::AssistantTurnRequired)?
        .ok_or(AppError::AssistantTurnRequired)?
        .map_err(|_| AppError::AssistantTurnRequired)?;
    let RelayFrame::Setup {
        session_id,
        account_id,
        call_id,
        from,
        to,
        direction,
    } = adapter.frame(&first.into_data())?
    else {
        return Err(AppError::AssistantTurnRequired);
    };
    if !protocol::handle(&session_id)
        || account_id != b.account_id
        || call_id != s.provider_call_id
        || from != s.from
        || to != s.to
        || direction != "inbound"
    {
        return Err(AppError::ChannelWebhookVerificationFailed(
            "Relay setup mismatch".into(),
        ));
    }
    state.db.collection::<CallSession>(SESSIONS).update_one(doc!{"_id":&s.id,"claim_id":&claim},doc!{"$set":{"provider_session_id":session_id,"lease_expires_at":bson::DateTime::from_chrono(Utc::now()+Duration::seconds(30))}}).await?;
    let mut output: Option<mpsc::Receiver<Output>> = None;
    let mut worker: Option<tokio::task::JoinHandle<Option<runtime::OwnerTurnSettlement>>> = None;
    let mut playback = Playback::default();
    let mut queued = std::collections::VecDeque::<String>::new();
    let mut ticker = tokio::time::interval(std::time::Duration::from_secs(5));
    loop {
        // Every final enters the same FIFO. A completed producer does not imply
        // its bounded output has drained; both must finish before the next turn.
        if output.is_none()
            && worker.is_none()
            && let Some(text) = queued.pop_front()
        {
            s = session::load(state, &s.id).await?;
            session::fence(state, &s).await?;
            let event = format!(
                "relay:{}:{}",
                s.provider_session_id.as_deref().unwrap_or_default(),
                s.next_utterance
            );
            let (rx, task) = turn(state.clone(), b, &s, &event, &text).await?;
            output = Some(rx);
            worker = Some(task);
        }
        tokio::select! {
            item=async {match output.as_mut(){Some(rx)=>rx.recv().await,None=>std::future::pending().await}}=>{
                if let Some(item)=item {
                    if item.error{return Err(AppError::ChannelRelayFailed("Agent response failed or stream was lost".into()));}
                    session::fence(state,&s).await?;
                    if item.generation==s.generation {
                        if playback.bytes()+item.text.len()>262144{return Err(AppError::RequestBodyTooLarge{max_bytes:262144,context:"Relay playback window".into()});}
                        tokio::time::timeout(std::time::Duration::from_secs(5),socket.send(Message::Text(adapter.text(&item.text,item.last).to_string().into()))).await.map_err(|_|AppError::AssistantTurnRequired)?.map_err(|_|AppError::AssistantTurnRequired)?;
                        playback.append(&item.turn_id,&item.text)?;
                    }
                    if item.last{output=None;}
                } else {return Err(AppError::ChannelRelayFailed("Agent stream ended without terminal evidence".into()));}
            }
            done=async {match worker.as_mut(){Some(task)=>task.await,None=>std::future::pending().await}}=>{
                done.map_err(|_|AppError::AssistantTurnRequired)?;worker=None;
            }
            incoming=socket.recv()=>{
                let Some(Ok(message))=incoming else{break};
                if matches!(message,Message::Close(_)){break;}
                if matches!(message,Message::Ping(_)|Message::Pong(_)){continue;}
                match adapter.frame(&message.into_data())? {
                    RelayFrame::Prompt{text,last:true}=>{
                        if queued.len()>=4 || text.is_empty() || text.len()>8192{return Err(AppError::VoiceQueueFull);}
                        queued.push_back(text);
                    }
                    RelayFrame::Prompt{last:false,..}|RelayFrame::Eager{}|RelayFrame::Resumed{}=>{},
                    RelayFrame::Interrupt{heard_prefix,duration_ms}=>{
                        let reply=playback.correlate(&heard_prefix);
                        session::record_interruption(state,&s,&heard_prefix,reply.map(|(_,text)|text).unwrap_or(""),duration_ms,reply.map(|(turn,_)|turn)).await?;
                        s=session::load(state,&s.id).await?;s=session::interrupt(state,&s).await?;output=None;
                        if let Some(task)=worker.take(){tokio::time::timeout(std::time::Duration::from_secs(10),task).await.map_err(|_|AppError::AssistantTurnRequired)?.map_err(|_|AppError::AssistantTurnRequired)?;}
                    }
                    RelayFrame::Dtmf{digit}=>{if digit.len()!=1 || !"0123456789*#ABCD".contains(&digit){return Err(AppError::BadRequest("Invalid DTMF".into()));}},
                    _=>break,
                }
            }
            _=ticker.tick()=>{
                session::fence(state,&s).await?;
                state.db.collection::<CallSession>(SESSIONS).update_one(doc!{"_id":&s.id,"claim_id":&claim,"state":"active"},doc!{"$set":{"lease_expires_at":bson::DateTime::from_chrono(Utc::now()+Duration::seconds(30))}}).await?;
            }
        }
    }
    Ok(())
}
pub async fn finalize_call(state: &AppState, s: &CallSession) -> AppResult<()> {
    if s.billing_finalized || s.ended_at.is_none() {
        return Ok(());
    }
    billing::settle_admitted(
        state,
        &s.billing_request_id,
        &s.user_id,
        s.observed_seconds,
        1,
    )
    .await?;
    state
        .db
        .collection::<CallSession>(SESSIONS)
        .update_one(
            doc! {"_id":&s.id,"ended_at":{"$ne":bson::Bson::Null}},
            doc! {"$set":{"billing_finalized":true}},
        )
        .await?;
    Ok(())
}
// Socket tasks and ingress share cleanup. Keep its billing and credential
// state machine out of both callers' poll frames, including future creation.
pub fn close<'a>(
    state: &'a AppState,
    b: &'a Binding,
    s: &'a CallSession,
    authoritative: bool,
) -> BoxFuture<'a, AppResult<()>> {
    Box::pin(close_inner(state, b, s, authoritative))
}
async fn close_inner(
    state: &AppState,
    b: &Binding,
    s: &CallSession,
    authoritative: bool,
) -> AppResult<()> {
    let current = session::load(state, &s.id).await?;
    let _ = engine::request_stop(&state.db, &s.user_id, &s.conversation_id).await;
    if current.ended_at.is_some() {
        return finalize_call(state, &current).await;
    }
    if authoritative {
        let frozen = session::terminal(
            state,
            &current,
            Utc::now(),
            None,
            "server_terminal_observation",
        )
        .await?;
        return finalize_call(state, &frozen).await;
    }
    state
        .db
        .collection::<CallSession>(SESSIONS)
        .update_one(
            doc! {"_id":&s.id,"state":{"$ne":"closed"}},
            doc! {"$inc":{"generation":1},"$set":{"state":"uncertain"}},
        )
        .await?;
    // Revocation stops new turns. Cleanup may only address the recorded admitted call.
    // A missing credential cannot prevent settlement of already-frozen evidence.
    let Ok(r) = binding::resolve(state, &b.user_id, &b.credential, b.profile).await else {
        return Ok(());
    };
    if session::command(state, &current, "hangup_dispatched").await? {
        if s.profile == Profile::TelnyxRelay {
            let _ = carrier::telnyx_command(state, b, &r, &current, "hangup", json!({})).await;
        } else if s.profile == Profile::TwilioRelay {
            let _ =
                carrier::twilio_update(state, b, &r, &current, &[("Status", "completed")]).await;
        }
    }
    // Command acceptance is not terminal evidence. Only a provider read or signed
    // terminal callback may freeze duration; an ambiguous command is never replayed.
    if let Ok(Some((at, seconds))) = carrier::terminal_status(state, b, &r, &current).await {
        let frozen = session::terminal(
            state,
            &current,
            at,
            Some(seconds),
            "authenticated_provider_status",
        )
        .await?;
        finalize_call(state, &frozen).await?;
    }
    Ok(())
}
pub fn spawn_sweep(state: AppState) {
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(15));
        loop {
            tick.tick().await;
            if let Ok(rows) = session::expired(&state).await {
                for s in rows {
                    let _ = state
                        .db
                        .collection::<bson::Document>(SESSIONS)
                        .update_one(
                            doc! {"_id":&s.id},
                            doc! {"$set":{"recovery_checked_at":bson::DateTime::now()}},
                        )
                        .await;
                    if s.ended_at.is_some() {
                        let _ = finalize_call(&state, &s).await;
                    } else if !profiles::relay(s.profile) {
                        let _ = session::finish_noncall(&state, &s.id, "expired").await;
                    } else if let Ok(b) = binding::load(&state, &s.binding_id).await {
                        let _ = close(&state, &b, &s, false).await;
                    }
                }
            }
            let _ = super::sms_delivery::sweep(&state).await;
        }
    });
}

#[derive(Default)]
pub(crate) struct Playback {
    replies: std::collections::VecDeque<(String, String)>,
}
impl Playback {
    fn bytes(&self) -> usize {
        self.replies.iter().map(|(_, text)| text.len()).sum()
    }
    pub(crate) fn append(&mut self, turn: &str, text: &str) -> AppResult<()> {
        if self.replies.back().is_none_or(|(id, _)| id != turn) {
            if self.replies.len() == 4 {
                self.replies.pop_front();
            }
            self.replies.push_back((turn.into(), String::new()));
        }
        let (_, sent) = self
            .replies
            .back_mut()
            .ok_or(AppError::AssistantTurnRequired)?;
        if sent.len() + text.len() > 65536 {
            return Err(AppError::RequestBodyTooLarge {
                max_bytes: 65536,
                context: "Relay reply".into(),
            });
        }
        sent.push_str(text);
        Ok(())
    }
    pub(crate) fn correlate(&self, prefix: &str) -> Option<(&str, &str)> {
        if prefix.is_empty() {
            return None;
        }
        let mut found = self
            .replies
            .iter()
            .filter(|(_, text)| text.starts_with(prefix));
        let (id, text) = found.next()?;
        if found.next().is_some() {
            None
        } else {
            Some((id, text))
        }
    }
}

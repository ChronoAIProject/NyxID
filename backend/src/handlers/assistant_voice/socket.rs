//! Human-only control and bounded PCM relay. Never accepts transcripts or provider frames.
use crate::{
    AppState,
    errors::{AppError, AppResult},
    models::{
        assistant_message::{AssistantMessage, COLLECTION_NAME as MESSAGES},
        assistant_voice::{REQUESTS, VoiceRequest},
        assistant_voice_session::VoiceSession,
    },
    mw::auth::AuthUser,
};
use axum::{
    extract::{
        Path, State, WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    http::HeaderMap,
    response::Response,
};
use futures::{StreamExt, TryStreamExt};
use mongodb::bson::doc;
use serde::Deserialize;
use serde_json::json;

#[derive(Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum ClientEvent {
    PttBegin,
    PttCommit,
    Heartbeat {
        generation: i64,
        playback_ms: i64,
        playback_audible: bool,
        revision: i64,
    },
}

pub async fn stream(
    State(state): State<AppState>,
    auth: AuthUser,
    Path((id, sid)): Path<(String, String)>,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
) -> AppResult<Response> {
    super::super::login_client_context::require_first_party_human(&auth)?;
    super::require_origin(&state, &headers)?;
    let human_session = auth
        .session_id
        .ok_or_else(|| AppError::Forbidden("An active browser session is required".into()))?
        .to_string();
    let call =
        crate::services::voice::session::get(&state.db, &auth.user_id.to_string(), &id, &sid)
            .await?;
    let socket_id = uuid::Uuid::new_v4().to_string();
    crate::services::voice::session::claim_stream(&state.db, &call, &socket_id).await?;
    Ok(upgrade
        .max_message_size(4096)
        .max_frame_size(4096)
        .on_upgrade(move |socket| {
            Box::pin(async move {
                let (input, output) = if call.protocol
                    == Some(crate::models::downstream_service::VoiceProtocol::XaiRealtime)
                {
                    let (tx, rx) = tokio::sync::mpsc::channel(64);
                    let (out, stream) = tokio::sync::mpsc::channel(64);
                    tokio::spawn(Box::pin(crate::services::voice::grok_runtime::serve(
                        state.clone(),
                        call.clone(),
                        socket_id.clone(),
                        rx,
                        out,
                    )));
                    (Some(tx), Some(stream))
                } else {
                    (None, None)
                };
                let _ = serve(
                    &state,
                    call.clone(),
                    socket,
                    &human_session,
                    &socket_id,
                    input,
                    output,
                )
                .await;
                let _ =
                    crate::services::voice::session::release_stream(&state.db, &call, &socket_id)
                        .await;
            })
        }))
}

async fn serve(
    state: &AppState,
    mut call: VoiceSession,
    mut socket: WebSocket,
    human_session: &str,
    socket_id: &str,
    input: Option<tokio::sync::mpsc::Sender<crate::services::voice::grok::ClientInput>>,
    mut output: Option<tokio::sync::mpsc::Receiver<crate::services::voice::grok::Output>>,
) -> AppResult<()> {
    use crate::services::voice::grok::{ClientInput, Output};
    let mut tick = tokio::time::interval(std::time::Duration::from_secs(1));
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut frame_window = tokio::time::Instant::now();
    let mut frames = 0;
    loop {
        tokio::select! {
            frame=socket.next()=>{
                if frame_window.elapsed()>=std::time::Duration::from_secs(1) {frame_window=tokio::time::Instant::now();frames=0;}
                frames+=1;if frames>if input.is_some() {80} else {20} {return Err(AppError::RateLimited);}
                let text = match frame {
                    Some(Ok(Message::Binary(bytes))) if input.is_some() && call.state == crate::models::assistant_voice_session::SessionState::Active => {
                        if bytes.is_empty() || bytes.len()>1920 || bytes.len()%2!=0 {return Err(AppError::ValidationError("Invalid voice audio frame".into()));}
                        input.as_ref().expect("checked relay").try_send(ClientInput::Pcm(bytes.to_vec())).map_err(|_|AppError::ClientDisconnected)?;
                        continue;
                    }
                    Some(Ok(Message::Text(text)))=>text,
                    _=>return Ok(()),
                };
                let event:ClientEvent=serde_json::from_str(&text).map_err(|_|AppError::ValidationError("Invalid voice control frame".into()))?;
                match event {
                    ClientEvent::PttBegin | ClientEvent::PttCommit => {
                        let tx=input.as_ref().ok_or_else(||AppError::ValidationError("Unexpected voice media control".into()))?;
                        tx.try_send(if matches!(event,ClientEvent::PttBegin) {ClientInput::Begin} else {ClientInput::Commit}).map_err(|_|AppError::ClientDisconnected)?;
                    }
                    ClientEvent::Heartbeat{generation,playback_ms,playback_audible,revision}=> {
                        crate::services::voice::session::heartbeat(&state.db,&call,generation,playback_ms,playback_audible,revision).await?;
                        if let Some(tx)=&input {tx.try_send(ClientInput::Playback{ms:playback_ms,audible:playback_audible}).map_err(|_|AppError::ClientDisconnected)?;}
                    }
                }
            },
            audio=async {match &mut output {Some(rx)=>rx.recv().await,None=>std::future::pending().await}}=>{
                let Some(audio)=audio else {return Ok(());};
                let result=tokio::time::timeout(std::time::Duration::from_secs(2),async {
                    match audio {
                        Output::Audio{bytes,end_ms}=>{
                            socket.send(Message::Text(json!({"type":"audio","generation":call.generation,"end_ms":end_ms}).to_string().into())).await?;
                            socket.send(Message::Binary(bytes.into())).await
                        }
                        Output::Flush=>socket.send(Message::Text(json!({"type":"audio_flush","generation":call.generation}).to_string().into())).await,
                    }
                }).await;
                result.map_err(|_|AppError::ClientDisconnected)?.map_err(|_|AppError::ClientDisconnected)?;
            },
            _=tick.tick()=>{
                crate::services::voice::session::refresh_stream(&state.db,&call,socket_id).await?;
                if state.db.collection::<mongodb::bson::Document>(crate::models::session::COLLECTION_NAME)
                    .find_one(doc! {"_id":human_session,"user_id":&call.user_id,"revoked":false,"expires_at":{"$gt":mongodb::bson::DateTime::now()}})
                    .await?.is_none() { return Err(AppError::Unauthorized("Voice session expired".into())); }
                crate::services::assistant_voice::require_enabled(&state.db,&call.user_id).await?;
                call=crate::services::voice::session::get(&state.db,&call.user_id,&call.conversation_id,&call.id).await?;
                let captions:Vec<AssistantMessage>=state.db.collection(MESSAGES).find(doc!{"user_id":&call.user_id,
                    "conversation_id":&call.conversation_id,"voice.session_id":&call.id}).sort(doc!{"seq":-1}).limit(16).await?.try_collect().await?;
                let tasks:Vec<VoiceRequest>=state.db.collection(REQUESTS).find(doc!{"user_id":&call.user_id,
                    "conversation_id":&call.conversation_id,"session_id":&call.id}).sort(doc!{"message_seq":-1}).limit(64).await?.try_collect().await?;
                let value=json!({"type":"snapshot","session":super::SessionResponse::from(call.clone()),
                    "captions":captions.into_iter().rev().map(|m|json!({"id":m.id,"speaker":m.role,"text":m.text,
                        "sealed":m.voice.as_ref().is_some_and(|v|v.sealed),"complete":m.voice.as_ref().is_some_and(|v|v.complete)})).collect::<Vec<_>>(),
                    "tasks":tasks.into_iter().rev().map(|r|json!({"id":r.id,"state":r.state,"turn_id":r.turn_id,
                        "pending_acknowledgement_ids":r.pending_acknowledgement_ids})).collect::<Vec<_>>()});
                tokio::time::timeout(std::time::Duration::from_secs(2),socket.send(Message::Text(value.to_string().into())))
                    .await.map_err(|_|AppError::ClientDisconnected)?.map_err(|_|AppError::ClientDisconnected)?;
                if !call.live_slot {return Ok(());}
            }
        }
    }
}

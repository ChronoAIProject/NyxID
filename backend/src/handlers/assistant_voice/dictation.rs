//! Browser-only dictation ingress. No provider commands/transcripts accepted.
use super::*;
use crate::services::voice::dictation::{self, Input};
use axum::{
    extract::{
        WebSocketUpgrade,
        ws::{Message, WebSocket},
    },
    http::HeaderMap,
    response::Response,
};
use futures::{SinkExt, StreamExt};

pub async fn dictation_options(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(q): Query<OptionsQuery>,
) -> AppResult<Json<serde_json::Value>> {
    super::super::login_client_context::require_first_party_human(&auth)?;
    let user = auth.user_id.to_string();
    voice::require_enabled(&state.db, &user).await?;
    let thread = voice::thread(&state.db, &user, &q.conversation_id).await?;
    let options = voice::dictation_options(&state.db, &state.encryption_keys, &thread).await?;
    Ok(Json(
        serde_json::json!({"options":options,"transcription_model":"gpt-live-transcribe","max_seconds":dictation::MAX_SECONDS}),
    ))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Start {
    client_request_id: String,
    preferences: Preferences,
}
pub async fn dictation_stream(
    State(state): State<AppState>,
    auth: AuthUser,
    Path(id): Path<String>,
    headers: HeaderMap,
    upgrade: WebSocketUpgrade,
) -> AppResult<Response> {
    super::super::login_client_context::require_first_party_human(&auth)?;
    super::require_origin(&state, &headers)?;
    let human_session = auth
        .session_id
        .ok_or_else(|| AppError::Forbidden("An active browser session is required".into()))?
        .to_string();
    let user = auth.user_id.to_string();
    voice::require_enabled(&state.db, &user).await?;
    voice::thread(&state.db, &user, &id).await?;
    Ok(upgrade
        .max_frame_size(4096)
        .max_message_size(4096)
        .on_upgrade(move |socket| async move {
            let _ = serve(state, socket, user, id, human_session).await;
        }))
}
async fn serve(
    state: AppState,
    mut socket: WebSocket,
    user: String,
    thread: String,
    human_session: String,
) -> AppResult<()> {
    let first = tokio::time::timeout(std::time::Duration::from_secs(5), socket.next())
        .await
        .map_err(|_| AppError::ClientDisconnected)?;
    let Some(Ok(Message::Text(text))) = first else {
        return Err(AppError::ClientDisconnected);
    };
    let start: Start = serde_json::from_str(&text)
        .map_err(|_| AppError::ValidationError("Invalid dictation request".into()))?;
    let (input, receive) = tokio::sync::mpsc::channel(8);
    let (output, mut events) = tokio::sync::mpsc::channel(8);
    let task = tokio::spawn(dictation::serve(
        state,
        user,
        thread,
        human_session,
        start.client_request_id,
        start.preferences.into(),
        receive,
        output,
    ));
    let (mut sender, mut reader) = socket.split();
    let mut ingress_frames = 0usize;
    loop {
        tokio::select! {
            event=events.recv()=> {
                let Some(event)=event else {break};
                let terminal=event["type"]=="error" || event["type"]=="stopped";
                if !matches!(tokio::time::timeout(std::time::Duration::from_secs(2),sender.send(Message::Text(event.to_string().into()))).await,Ok(Ok(()))) {break;}
                if terminal {break;}
            }
            event=reader.next()=> {
                ingress_frames += 1;
                if ingress_frames > 4096 { break; }
                let message=match event {
                    Some(Ok(Message::Binary(bytes))) => Input::Audio(bytes.to_vec()),
                    Some(Ok(Message::Text(text))) if text.as_str()==r#"{"type":"stop"}"# => Input::Stop,
                    Some(Ok(Message::Ping(_)|Message::Pong(_)))=>continue,
                    _=>break,
                };
                if input.try_send(message).is_err() {break;}
            }
        }
    }
    // Dropping ingress requests commit/finalization of forwarded audio. Do not abort
    // the server actor: it owns durable confirmed usage and bounded provider drain.
    drop(input);
    drop(events);
    let _ = tokio::time::timeout(std::time::Duration::from_secs(15), task).await;
    Ok(())
}

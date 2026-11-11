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

// The shared PCM worklet sends 480 mono samples at 24 kHz: 20 ms / 50 frames
// per second. Leave a small finite allowance for Stop and transport Ping/Pong.
const PCM_FRAMES_PER_SECOND: usize = 50;
const CONTROL_FRAME_ALLOWANCE: usize = 32;
const MAX_INGRESS_FRAMES: usize =
    dictation::MAX_SECONDS as usize * PCM_FRAMES_PER_SECOND + CONTROL_FRAME_ALLOWANCE;

// Two seconds of normal 20 ms PCM frames (at most 400 KiB at the wire bound).
const INPUT_CAPACITY: usize = 2 * PCM_FRAMES_PER_SECOND;
const BACKPRESSURE_WAIT: std::time::Duration = std::time::Duration::from_secs(2);

#[derive(Default)]
struct Ingress {
    frames: usize,
}
impl Ingress {
    fn parse(&mut self, message: Message) -> Result<Option<Input>, &'static str> {
        if self.frames >= MAX_INGRESS_FRAMES {
            return Err("dictation_frame_limit");
        }
        self.frames += 1;
        match message {
            Message::Binary(bytes) => Ok(Some(Input::Audio(bytes.to_vec()))),
            Message::Text(text) if text.as_str() == r#"{"type":"stop"}"# => Ok(Some(Input::Stop)),
            Message::Ping(_) | Message::Pong(_) => Ok(None),
            _ => Err("dictation_invalid_frame"),
        }
    }
}

fn terminal(code: &'static str) -> serde_json::Value {
    let message = match code {
        "dictation_backpressure" => {
            "Dictation could not keep up. Review the captured text and try again."
        }
        "dictation_invalid_frame" | "dictation_frame_limit" => {
            "Dictation audio was not accepted. Review the captured text and start again."
        }
        "dictation_start_timeout" => "Dictation did not start in time. Try again.",
        _ => "Dictation was interrupted. Review the captured text and try again.",
    };
    serde_json::json!({"type":"error","stage":"transport","code":code,
        "error":{"error":code,"message":message}})
}
fn terminal_close(event: &serde_json::Value) -> axum::extract::ws::CloseFrame {
    use axum::extract::ws::close_code;
    let (code, reason) = if event["type"] == "stopped" {
        (close_code::NORMAL, "dictation_stopped")
    } else {
        match event["code"].as_str() {
            Some("dictation_backpressure") => (close_code::AGAIN, "dictation_backpressure"),
            Some("dictation_invalid_frame" | "dictation_frame_limit") => {
                (close_code::POLICY, "dictation_invalid_audio")
            }
            _ => (close_code::ERROR, "dictation_failed"),
        }
    };
    axum::extract::ws::CloseFrame {
        code,
        reason: reason.into(),
    }
}
async fn send_event<S>(sender: &mut S, event: serde_json::Value) -> bool
where
    S: futures::Sink<Message> + Unpin,
{
    let terminal = matches!(event["type"].as_str(), Some("error" | "stopped"));
    let close = terminal.then(|| terminal_close(&event));
    if !matches!(
        tokio::time::timeout(
            std::time::Duration::from_secs(2),
            sender.send(Message::Text(event.to_string().into()))
        )
        .await,
        Ok(Ok(()))
    ) {
        return false;
    }
    if let Some(close) = close {
        let _ = tokio::time::timeout(
            std::time::Duration::from_secs(2),
            sender.send(Message::Close(Some(close))),
        )
        .await;
    }
    !terminal
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DictationOptionsQuery {
    conversation_id: String,
}

pub async fn dictation_options(
    State(state): State<AppState>,
    auth: AuthUser,
    Query(q): Query<DictationOptionsQuery>,
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
    let first = tokio::time::timeout(std::time::Duration::from_secs(5), socket.next()).await;
    let start = match first {
        Ok(Some(Ok(Message::Text(text)))) => serde_json::from_str::<Start>(&text).ok(),
        Err(_) => {
            send_event(&mut socket, terminal("dictation_start_timeout")).await;
            return Ok(());
        }
        _ => None,
    };
    let Some(start) = start else {
        send_event(&mut socket, terminal("dictation_invalid_frame")).await;
        return Ok(());
    };
    let (input, receive) = tokio::sync::mpsc::channel(INPUT_CAPACITY);
    let (output, events) = tokio::sync::mpsc::channel(8);
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
    relay(socket, input, events).await;
    let _ = tokio::time::timeout(std::time::Duration::from_secs(15), task).await;
    Ok(())
}

async fn relay(
    socket: WebSocket,
    input: tokio::sync::mpsc::Sender<Input>,
    mut events: tokio::sync::mpsc::Receiver<serde_json::Value>,
) {
    let (mut sender, mut reader) = socket.split();
    let mut ingress = Ingress::default();
    let mut pending = None;
    let mut deadline = tokio::time::Instant::now();
    let mut stopped = false;
    loop {
        tokio::select! { biased;
            event = events.recv() => {
                let event = event.unwrap_or_else(|| terminal("dictation_worker_ended"));
                if !send_event(&mut sender, event).await { break; }
            }
            _ = tokio::time::sleep_until(deadline), if pending.is_some() => {
                send_event(&mut sender, terminal("dictation_backpressure")).await;
                break;
            }
            permit = input.reserve(), if pending.is_some() => {
                match permit {
                    Ok(permit) => { permit.send(pending.take().expect("pending input")); }
                    Err(_) => {
                        // A finishing actor can close ingress before its terminal output.
                        pending = None;
                        stopped = true;
                    }
                }
            }
            event = reader.next(), if pending.is_none() && !stopped => {
                match event {
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(message)) => match ingress.parse(message) {
                        Ok(Some(message)) => {
                            stopped = matches!(message, Input::Stop);
                            pending = Some(message);
                            deadline = tokio::time::Instant::now() + BACKPRESSURE_WAIT;
                        }
                        Ok(None) => {},
                        Err(code) => { send_event(&mut sender, terminal(code)).await; break; }
                    },
                    Some(Err(_)) => { send_event(&mut sender, terminal("dictation_transport_failed")).await; break; }
                }
            }
        }
    }
    // Dropping ingress requests commit/finalization of forwarded audio. Do not abort
    // the server actor: it owns durable confirmed usage and bounded provider drain.
    drop(input);
    drop(events);
    // Complete the close handshake while discarding buffered ingress. Dropping
    // TCP with unread audio can reset the peer before it receives our close frame.
    let _ = tokio::time::timeout(std::time::Duration::from_secs(1), async {
        for _ in 0..MAX_INGRESS_FRAMES {
            match reader.next().await {
                Some(Ok(Message::Close(_))) | Some(Err(_)) | None => break,
                _ => {}
            }
        }
    })
    .await;
}

#[cfg(test)]
#[path = "dictation_tests.rs"]
mod tests;

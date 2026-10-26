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

#[derive(Default)]
struct Ingress {
    frames: usize,
}
impl Ingress {
    fn forward(&mut self, message: Message, input: &tokio::sync::mpsc::Sender<Input>) -> bool {
        if self.frames >= MAX_INGRESS_FRAMES {
            return false;
        }
        self.frames += 1;
        let message = match message {
            Message::Binary(bytes) => Input::Audio(bytes.to_vec()),
            Message::Text(text) if text.as_str() == r#"{"type":"stop"}"# => Input::Stop,
            Message::Ping(_) | Message::Pong(_) => return true,
            _ => return false,
        };
        input.try_send(message).is_ok()
    }
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
    let mut ingress = Ingress::default();
    loop {
        tokio::select! {
            event=events.recv()=> {
                let Some(event)=event else {break};
                let terminal=event["type"]=="error" || event["type"]=="stopped";
                if !matches!(tokio::time::timeout(std::time::Duration::from_secs(2),sender.send(Message::Text(event.to_string().into()))).await,Ok(Ok(()))) {break;}
                if terminal {break;}
            }
            event=reader.next()=> {
                let Some(Ok(message))=event else {break};
                if !ingress.forward(message,&input) {break;}
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dictation_handler_ingress_forwards_two_minutes_of_worklet_pcm_and_stop() {
        let (input, mut receive) = tokio::sync::mpsc::channel(8);
        let mut ingress = Ingress::default();
        let samples_per_frame = 480;
        let sample_rate = 24_000;
        let duration_seconds = 120;
        let mut forwarded_bytes = 0;
        // Exercise the actual handler dispatch/channel path with 6,000 normal
        // 20 ms packets, consuming each as the relay does during capture.
        for _ in 0..duration_seconds * sample_rate / samples_per_frame {
            let bytes = vec![0x42; samples_per_frame * 2];
            assert!(ingress.forward(Message::Binary(bytes.clone().into()), &input));
            match receive.try_recv().unwrap() {
                Input::Audio(forwarded) => {
                    assert_eq!(forwarded, bytes);
                    forwarded_bytes += forwarded.len();
                }
                Input::Stop => panic!("PCM was dispatched as Stop"),
            }
        }
        assert_eq!(forwarded_bytes, duration_seconds * sample_rate * 2);
        // Transport controls and Stop fit the explicit control allowance;
        // Ping/Pong must not enter the PCM channel.
        assert!(ingress.forward(Message::Ping(Vec::new().into()), &input));
        assert!(ingress.forward(Message::Pong(Vec::new().into()), &input));
        assert!(ingress.forward(Message::Text(r#"{"type":"stop"}"#.into()), &input));
        assert!(matches!(receive.try_recv().unwrap(), Input::Stop));
        assert!(matches!(
            receive.try_recv(),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty)
        ));
    }

    #[test]
    fn dictation_handler_ingress_bounds_control_floods_and_channel_backpressure() {
        let (input, mut receive) = tokio::sync::mpsc::channel(8);
        let mut ingress = Ingress::default();
        let mut refused = false;
        for index in 0..10_000 {
            let message = if index % 2 == 0 {
                Message::Ping(Vec::new().into())
            } else {
                Message::Pong(Vec::new().into())
            };
            if !ingress.forward(message, &input) {
                refused = true;
                break;
            }
        }
        assert!(refused, "control-only traffic must exhaust a finite budget");
        assert!(!ingress.forward(Message::Binary(vec![0; 960].into()), &input));
        assert!(!ingress.forward(Message::Text(r#"{"type":"stop"}"#.into()), &input));
        assert!(matches!(
            receive.try_recv(),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty)
        ));

        // Enlarging the lifetime frame count must not enlarge the eight-frame
        // transient channel or replace its immediate backpressure refusal.
        let mut ingress = Ingress::default();
        for _ in 0..8 {
            assert!(ingress.forward(Message::Binary(vec![0; 960].into()), &input));
        }
        assert!(!ingress.forward(Message::Binary(vec![0; 960].into()), &input));
    }
}

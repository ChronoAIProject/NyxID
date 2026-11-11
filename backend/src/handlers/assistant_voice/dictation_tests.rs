use super::*;
use tokio_tungstenite::tungstenite::Message as ClientMessage;
type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

async fn fixture() -> (
    Socket,
    tokio::sync::mpsc::Receiver<Input>,
    tokio::sync::mpsc::Sender<serde_json::Value>,
    tokio::task::JoinHandle<()>,
) {
    let (input, receive) = tokio::sync::mpsc::channel(INPUT_CAPACITY);
    let (output, events) = tokio::sync::mpsc::channel(8);
    let channels = std::sync::Arc::new(std::sync::Mutex::new(Some((input, events))));
    let app = axum::Router::new().route(
        "/",
        axum::routing::get(move |ws: WebSocketUpgrade| {
            let (input, events) = channels.lock().unwrap().take().unwrap();
            async move { ws.on_upgrade(move |socket| relay(socket, input, events)) }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let (socket, _) = tokio_tungstenite::connect_async(format!("ws://{address}/"))
        .await
        .unwrap();
    (socket, receive, output, server)
}
async fn event(socket: &mut Socket) -> serde_json::Value {
    let frame = tokio::time::timeout(std::time::Duration::from_secs(5), socket.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    serde_json::from_str(frame.to_text().unwrap()).unwrap()
}

#[tokio::test]
async fn dictation_2138_short_stall_preserves_continuous_audio_and_manual_stop() {
    let (mut socket, mut input, output, server) = fixture().await;
    for _ in 0..INPUT_CAPACITY + 25 {
        socket
            .send(ClientMessage::Binary(vec![42; 960].into()))
            .await
            .unwrap();
    }
    tokio::time::sleep(std::time::Duration::from_millis(250)).await;
    for _ in 0..INPUT_CAPACITY + 25 {
        assert!(matches!(input.recv().await, Some(Input::Audio(bytes)) if bytes == vec![42;960]));
    }
    socket
        .send(ClientMessage::Text(r#"{"type":"stop"}"#.into()))
        .await
        .unwrap();
    assert!(matches!(input.recv().await, Some(Input::Stop)));
    output
        .send(serde_json::json!({"type":"stopped","confirmed":true}))
        .await
        .unwrap();
    assert_eq!(event(&mut socket).await["type"], "stopped");
    assert!(
        matches!(socket.next().await.unwrap().unwrap(), ClientMessage::Close(Some(close)) if close.code == tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode::Normal)
    );
    server.abort();
}

#[tokio::test]
async fn dictation_2138_full_queue_reports_terminal_error_before_retryable_close() {
    let (mut socket, _input, _output, server) = fixture().await;
    for _ in 0..INPUT_CAPACITY + 1 {
        socket
            .send(ClientMessage::Binary(vec![0; 960].into()))
            .await
            .unwrap();
    }
    let error = event(&mut socket).await;
    assert_eq!(error["type"], "error");
    assert_eq!(error["code"], "dictation_backpressure");
    assert_eq!(error["stage"], "transport");
    assert!(
        matches!(socket.next().await.unwrap().unwrap(), ClientMessage::Close(Some(close)) if u16::from(close.code) == 1013 && close.reason == "dictation_backpressure")
    );
    server.abort();
}

#[tokio::test]
async fn dictation_2138_terminal_output_is_delivered_while_audio_is_backpressured() {
    let (mut socket, _input, output, server) = fixture().await;
    for _ in 0..INPUT_CAPACITY + 1 {
        socket
            .send(ClientMessage::Binary(vec![0; 960].into()))
            .await
            .unwrap();
    }
    output
        .send(
            serde_json::json!({"type":"error","code":"dictation_provider_failed","stage":"provider",
        "error":{"error":"voice_provider_unavailable","message":"Transcription ended"}}),
        )
        .await
        .unwrap();
    assert_eq!(
        event(&mut socket).await["code"],
        "dictation_provider_failed"
    );
    assert!(
        matches!(socket.next().await.unwrap().unwrap(), ClientMessage::Close(Some(close)) if u16::from(close.code) == 1011)
    );
    server.abort();
}

#[tokio::test]
async fn dictation_2138_all_120_seconds_and_stop_fit_the_ingress_budget() {
    let (mut socket, mut input, output, server) = fixture().await;
    let consumer = tokio::spawn(async move {
        let mut bytes = 0;
        while let Some(message) = input.recv().await {
            match message {
                Input::Audio(audio) => bytes += audio.len(),
                Input::Stop => break,
            }
        }
        output
            .send(serde_json::json!({"type":"stopped","confirmed":true}))
            .await
            .unwrap();
        bytes
    });
    for _ in 0..dictation::MAX_SECONDS as usize * PCM_FRAMES_PER_SECOND {
        socket
            .send(ClientMessage::Binary(vec![0; 960].into()))
            .await
            .unwrap();
    }
    socket
        .send(ClientMessage::Text(r#"{"type":"stop"}"#.into()))
        .await
        .unwrap();
    assert_eq!(event(&mut socket).await["type"], "stopped");
    assert_eq!(consumer.await.unwrap(), 120 * 48_000);
    server.abort();
}

#[tokio::test]
async fn dictation_2138_worker_exit_and_invalid_frames_are_explicit() {
    let (mut socket, _input, output, server) = fixture().await;
    drop(output);
    assert_eq!(event(&mut socket).await["code"], "dictation_worker_ended");
    server.abort();
    let (mut socket, _input, _output, server) = fixture().await;
    socket
        .send(ClientMessage::Text("private-unrecognized-command".into()))
        .await
        .unwrap();
    let error = event(&mut socket).await;
    assert_eq!(error["code"], "dictation_invalid_frame");
    assert!(!error.to_string().contains("private-unrecognized"));
    server.abort();
}

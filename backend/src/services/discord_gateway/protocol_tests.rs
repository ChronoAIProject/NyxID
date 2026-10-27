use super::*;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::{
    WebSocketStream, accept_async,
    tungstenite::protocol::{CloseFrame, frame::coding::CloseCode},
};

type Socket = WebSocketStream<TcpStream>;
async fn send(ws: &mut Socket, value: Value) {
    ws.send(Message::Text(value.to_string().into()))
        .await
        .unwrap();
}
async fn read(ws: &mut Socket) -> Value {
    loop {
        match timeout(Duration::from_secs(3), ws.next())
            .await
            .unwrap()
            .unwrap()
            .unwrap()
        {
            Message::Text(text) => return serde_json::from_str(&text).unwrap(),
            Message::Ping(bytes) => ws.send(Message::Pong(bytes)).await.unwrap(),
            _ => {}
        }
    }
}
async fn server_socket() -> (TcpListener, String) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}/", listener.local_addr().unwrap());
    (listener, url)
}
async fn accept(listener: TcpListener) -> Socket {
    let mut ws = accept_async(listener.accept().await.unwrap().0)
        .await
        .unwrap();
    send(&mut ws, json!({"op":10,"d":{"heartbeat_interval":100}})).await;
    ws
}
fn ready(url: &str) -> Value {
    json!({"op":0,"t":"READY","s":1,"d":{"user":{"id":"100"},"session_id":"session-one","resume_gateway_url":url}})
}

#[tokio::test]
async fn identify_and_heartbeat_continue_while_dispatch_is_slow() {
    let (listener, url) = server_socket().await;
    let server_url = url.clone();
    let completed = Arc::new(AtomicUsize::new(0));
    let seen = completed.clone();
    let server = tokio::spawn(async move {
        let mut ws = accept(listener).await;
        let auth = read(&mut ws).await;
        assert_eq!(auth["op"], 2);
        assert_eq!(auth["d"]["intents"], INTENTS);
        assert_eq!(auth["d"]["token"], "fixture-token");
        send(&mut ws, ready(&server_url)).await;
        send(
            &mut ws,
            json!({"op":0,"t":"MESSAGE_CREATE","s":2,"d":{"id":"200"}}),
        )
        .await;
        let mut heartbeats = 0;
        while seen.load(Ordering::SeqCst) < 2 {
            let heartbeat = read(&mut ws).await;
            assert_eq!(heartbeat["op"], 1);
            assert!(heartbeat["d"].is_null() || matches!(heartbeat["d"].as_u64(), Some(1 | 2)));
            heartbeats += 1;
            send(&mut ws, json!({"op":11})).await;
        }
        send(&mut ws, json!({"op":7})).await;
        assert!(heartbeats >= 2);
    });
    let mut resume = Resume::default();
    let failure = connection(&url, "fixture-token", "100", &mut resume, |_, event| {
        let completed = completed.clone();
        async move {
            if event["t"] == "MESSAGE_CREATE" {
                tokio::time::sleep(Duration::from_millis(250)).await;
            }
            completed.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    })
    .await;
    server.await.unwrap();
    assert_eq!(failure, Failure::Reconnect);
    assert_eq!(resume.sequence, Some(2));
    assert_eq!(resume.session_id, "session-one");
}

#[tokio::test]
async fn resume_uses_committed_sequence_and_accepts_replay_before_resumed() {
    let (listener, url) = server_socket().await;
    let server = tokio::spawn(async move {
        let mut ws = accept(listener).await;
        let auth = read(&mut ws).await;
        assert_eq!(
            auth,
            json!({"op":6,"d":{"token":"fixture-token","session_id":"retained","seq":42}})
        );
        send(&mut ws, json!({"op":1})).await;
        let heartbeat = read(&mut ws).await;
        assert_eq!(heartbeat["d"], 42);
        send(&mut ws, json!({"op":11})).await;
        send(
            &mut ws,
            json!({"op":0,"t":"MESSAGE_CREATE","s":43,"d":{"id":"300"}}),
        )
        .await;
        send(&mut ws, json!({"op":0,"t":"RESUMED","s":44,"d":{}})).await;
        send(&mut ws, json!({"op":7})).await;
    });
    let mut resume = Resume {
        session_id: "retained".into(),
        url: url.clone(),
        sequence: Some(42),
    };
    let processed = Arc::new(AtomicUsize::new(0));
    let result = connection(&url, "fixture-token", "100", &mut resume, |_, _| {
        processed.fetch_add(1, Ordering::SeqCst);
        async { Ok(()) }
    })
    .await;
    server.await.unwrap();
    assert_eq!(result, Failure::Reconnect);
    assert_eq!(processed.load(Ordering::SeqCst), 2);
    assert_eq!(resume.sequence, Some(44));
}

#[tokio::test]
async fn invalid_session_controls_fresh_identify_versus_resume() {
    for resumable in [false, true] {
        let (listener, url) = server_socket().await;
        let server = tokio::spawn(async move {
            let mut ws = accept(listener).await;
            assert_eq!(read(&mut ws).await["op"], 6);
            send(&mut ws, json!({"op":9,"d":resumable})).await;
        });
        let mut resume = Resume {
            session_id: "retained".into(),
            url: url.clone(),
            sequence: Some(42),
        };
        assert_eq!(
            connection(&url, "token", "100", &mut resume, |_, _| async { Ok(()) }).await,
            Failure::InvalidSession(resumable)
        );
        server.await.unwrap();
        assert_eq!(resume.session_id.is_empty(), !resumable);
        let (listener, next) = server_socket().await;
        resume.url = next.clone();
        let server = tokio::spawn(async move {
            let mut ws = accept(listener).await;
            assert_eq!(read(&mut ws).await["op"], if resumable { 6 } else { 2 });
            send(&mut ws, json!({"op":7})).await;
        });
        connection(&next, "token", "100", &mut resume, |_, _| async { Ok(()) }).await;
        server.await.unwrap();
    }
}

#[tokio::test]
async fn missed_heartbeat_ack_reconnects() {
    let (listener, url) = server_socket().await;
    let server_url = url.clone();
    let server = tokio::spawn(async move {
        let mut ws = accept(listener).await;
        read(&mut ws).await;
        send(&mut ws, ready(&server_url)).await;
        assert_eq!(read(&mut ws).await["op"], 1);
        // Keep the peer open, but never acknowledge the heartbeat.
        let _ = timeout(Duration::from_secs(2), ws.next()).await;
    });
    let mut resume = Resume::default();
    assert_eq!(
        connection(&url, "token", "100", &mut resume, |_, _| async { Ok(()) }).await,
        Failure::Heartbeat
    );
    server.await.unwrap();
}

#[tokio::test]
async fn failed_dispatch_keeps_resume_before_failed_and_queued_messages() {
    let (listener, url) = server_socket().await;
    let server_url = url.clone();
    let server = tokio::spawn(async move {
        let mut ws = accept(listener).await;
        read(&mut ws).await;
        send(&mut ws, ready(&server_url)).await;
        for sequence in 2..=3 {
            send(
                &mut ws,
                json!({"op":0,"t":"MESSAGE_CREATE","s":sequence,"d":{}}),
            )
            .await;
        }
        send(&mut ws, json!({"op":7})).await;
    });
    let mut resume = Resume::default();
    let processed = Arc::new(AtomicUsize::new(0));
    let result = connection(&url, "token", "100", &mut resume, |cursor, _| {
        let processed = processed.clone();
        async move {
            processed.fetch_add(1, Ordering::SeqCst);
            if cursor.sequence == Some(2) {
                Err(Failure::Ingress)
            } else {
                Ok(())
            }
        }
    })
    .await;
    server.await.unwrap();
    assert_eq!(result, Failure::Ingress);
    assert_eq!(resume.sequence, Some(1));
    assert_eq!(processed.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn close_codes_surface_token_intents_and_invalidate_expired_sessions() {
    for (code, expected) in [
        (4004, Failure::InvalidToken),
        (4014, Failure::Intents),
        (4013, Failure::Intents),
        (4011, Failure::Sharding),
        (4007, Failure::InvalidSession(false)),
        (4009, Failure::InvalidSession(false)),
    ] {
        let (listener, url) = server_socket().await;
        let server = tokio::spawn(async move {
            let mut ws = accept(listener).await;
            read(&mut ws).await;
            ws.close(Some(CloseFrame {
                code: CloseCode::from(code),
                reason: "ignored provider prose".into(),
            }))
            .await
            .unwrap();
        });
        let mut resume = Resume {
            session_id: "retained".into(),
            url: url.clone(),
            sequence: Some(42),
        };
        assert_eq!(
            connection(&url, "token", "100", &mut resume, |_, _| async { Ok(()) }).await,
            expected
        );
        server.await.unwrap();
        if code == 4007 || code == 4009 {
            assert!(resume.session_id.is_empty());
        }
    }
}

#[tokio::test]
async fn bounded_queue_reconnects_without_advancing_past_dispatched_events() {
    let (listener, url) = server_socket().await;
    let server_url = url.clone();
    let server = tokio::spawn(async move {
        let mut ws = accept(listener).await;
        read(&mut ws).await;
        send(&mut ws, ready(&server_url)).await;
        for sequence in 2..100 {
            send(
                &mut ws,
                json!({"op":0,"t":"MESSAGE_CREATE","s":sequence,"d":{"id":"300"}}),
            )
            .await;
        }
        while let Some(Ok(message)) = ws.next().await {
            if matches!(message, Message::Close(_)) {
                break;
            }
            if let Message::Text(_) = message {
                send(&mut ws, json!({"op":11})).await;
            }
        }
    });
    let mut resume = Resume::default();
    let result = connection(&url, "token", "100", &mut resume, |_, _| async {
        tokio::time::sleep(Duration::from_millis(10)).await;
        Ok(())
    })
    .await;
    server.await.unwrap();
    assert_eq!(result, Failure::Backpressure);
    assert!(resume.sequence.unwrap() < 99);
}

#[test]
fn urls_and_backoff_are_bounded() {
    assert!(gateway_url("wss://gateway.discord.gg/").is_ok());
    assert!(
        gateway_url("wss://gateway-us-east1-b.discord.gg/?v=9")
            .unwrap()
            .ends_with("v=10&encoding=json")
    );
    for url in [
        "wss://discord.gg.attacker.test/",
        "wss://user:secret@gateway.discord.gg/",
        "https://gateway.discord.gg/",
        "wss://gateway.discord.gg/other",
        "wss://gateway.discord.gg:444/",
    ] {
        assert!(gateway_url(url).is_err());
    }
    for n in 0..100 {
        let delay = retry_delay(n, Failure::Transport);
        assert!((Duration::from_millis(500)..=Duration::from_secs(60)).contains(&delay));
        assert!(
            (Duration::from_secs(1)..=Duration::from_secs(5))
                .contains(&retry_delay(n, Failure::InvalidSession(false)))
        );
    }
}

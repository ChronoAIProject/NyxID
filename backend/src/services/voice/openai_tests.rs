use super::openai::*;
use axum::{
    Json, Router,
    extract::{WebSocketUpgrade, ws::Message},
    http::HeaderMap,
    routing::{get, post},
};
use serde_json::{Value, json};
use zeroize::Zeroizing;

#[tokio::test]
async fn fixture_session_restricts_browser_and_finalizes_only_on_provider_close() {
    let app=Router::new().route("/v1/live/sessions",post(|headers:HeaderMap,Json(body):Json<Value>| async move {
        assert_eq!(headers["authorization"],"Bearer fixture-secret");
        assert_eq!(body["session"]["model"],"catalog-live-model");
        assert_eq!(body["session"]["store"],false);
        assert_eq!(body["session"]["delegation"],json!({"type":"client"}));
        assert_eq!(body["session"]["client"]["data_channel"]["allowed_client_events"],json!([]));
        assert!(body["session"].get("tools").is_none());
        Json(json!({"session":{"id":"live_fixture","expires_at":2_000_000_000_i64},"transport":{"type":"webrtc","sdp":"v=0\r\nm=audio 9 UDP/TLS/RTP/SAVPF 111\r\n"}}))
    })).route("/v1/live/sessions/live_fixture/attach",get(|headers:HeaderMap,ws:WebSocketUpgrade| async move {
        assert_eq!(headers["authorization"],"Bearer fixture-secret");
        ws.on_upgrade(|mut socket| async move {
            socket.send(Message::Text(json!({"type":"session.usage.updated","usage":{"seconds":32.5}}).to_string().into())).await.unwrap();
            let Some(Ok(Message::Text(text)))=socket.recv().await else {panic!("expected close")};
            assert_eq!(serde_json::from_str::<Value>(&text).unwrap()["type"],"session.close");
            socket.send(Message::Text(json!({"type":"session.closed","usage":{"seconds":33.2}}).to_string().into())).await.unwrap();
        })
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let provider = OpenAi::fixture(Zeroizing::new("fixture-secret".into()), address);
    let created = provider
        .create(
            "v=0\r\nm=audio 9 UDP/TLS/RTP/SAVPF 111\r\n",
            "fixture instructions",
            "marin",
            "catalog-live-model",
        )
        .await
        .unwrap();
    let mut socket = provider.attach(&created.provider_id).await.unwrap();
    assert_eq!(
        receive(&mut socket).await.unwrap().unwrap()["usage"]["seconds"],
        32.5
    );
    send(&mut socket, json!({"type":"session.close"}))
        .await
        .unwrap();
    assert_eq!(
        receive(&mut socket).await.unwrap().unwrap()["type"],
        "session.closed"
    );
    server.abort();
}
#[test]
fn provider_locator_cannot_redirect_credentials() {
    for value in [
        "",
        "../other",
        "https://attacker.invalid",
        "live?a=b",
        "live#x",
        "live%2fother",
    ] {
        assert!(!valid_provider_id(value));
    }
    assert!(valid_provider_id("live_u0_a-123"));
}

#[test]
fn usage_decimal_never_rounds_up_across_a_second_or_window() {
    for (raw, expected) in [
        ("0.999999999999999999999", 0),
        ("29.9999999999999999999", 29),
        ("3.012e1", 30),
        ("0", 0),
        ("1e-7", 0),
        ("1800.1", 1800),
    ] {
        assert_eq!(super::openai::completed_seconds(raw).unwrap(), expected);
    }
    for raw in ["-1", "NaN", "true", "86400.1", "1e100000"] {
        assert!(super::openai::completed_seconds(raw).is_err())
    }
}

#[tokio::test]
async fn initialization_rejections_release_but_malformed_sdp_keeps_the_closable_id() {
    use axum::{http::StatusCode, response::IntoResponse};
    for malformed in [false, true] {
        let app=Router::new().route("/v1/live/sessions",post(move || async move {
            if malformed {
                Json(json!({"session":{"id":"live_orphan","expires_at":2_000_000_000_i64},"transport":{"sdp":"invalid"}})).into_response()
            } else {StatusCode::UNAUTHORIZED.into_response()}
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let provider = OpenAi::fixture(Zeroizing::new("fixture-secret".into()), address);
        let error = provider
            .create("v=0", "instructions", "marin", "gpt-live-1")
            .await
            .err()
            .unwrap();
        assert_eq!(error.not_created, !malformed);
        assert_eq!(
            error.provider_id.as_deref(),
            malformed.then_some("live_orphan")
        );
        assert!(!format!("{error:?}").contains("live_orphan"));
        server.abort();
    }
}

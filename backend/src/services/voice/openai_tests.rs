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
        assert_eq!(body["session"]["audio"]["output"]["voice"], "marin");
        assert_eq!(body["transport"]["type"], "webrtc");
        assert_eq!(body["session"]["client"]["data_channel"]["allowed_server_events"], json!([{ "type":"session.started" }, { "type":"session.closed" }, { "type":"error" }]));
        (axum::http::StatusCode::CREATED, Json(json!({"session":{"id":"live_fixture"},"transport":{"type":"webrtc","sdp":"v=0\r\nm=audio 9 UDP/TLS/RTP/SAVPF 111\r\n"}})))
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
    assert_eq!(
        created.expires_at, None,
        "Official create response omits expiry"
    );
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
fn provider_locator_encodes_opaque_ids_without_redirecting_credentials() {
    let adapter = OpenAi::new(Zeroizing::new("fixture-secret".into())).unwrap();
    for id in [
        "live_u0_a-123",
        "live:id.v2",
        "../other",
        "https://attacker.invalid",
        "live?a=b",
        "live#x",
        "live%2fother",
        "live/a",
    ] {
        let url = adapter.attach_url(id).unwrap();
        assert_eq!(url.origin().ascii_serialization(), "wss://api.openai.com");
        assert!(url.query().is_none() && url.fragment().is_none());
        assert_eq!(url.path_segments().unwrap().count(), 5);
    }
    for id in ["", ".", "..", "live\ninvalid"] {
        assert!(!valid_provider_id(id));
    }
    assert!(!valid_provider_id(&"x".repeat(1025)));
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

#[tokio::test]
async fn voice_create_rejection_retains_only_safe_provider_metadata() {
    let app = Router::new().route("/v1/live/sessions", post(|| async {
        (axum::http::StatusCode::BAD_REQUEST, Json(json!({"error": {
            "type":"invalid_request_error", "code":"invalid_value", "param":"session.delegation",
            "message":"DO-NOT-EXPOSE sk-secret SDP instructions", "key":"sk-secret"
        }})))
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let provider = OpenAi::fixture(Zeroizing::new("fixture-secret".into()), address);
    let failure = provider
        .create("v=0", "secret instructions", "marin", "gpt-live-1")
        .await
        .err()
        .unwrap();
    assert!(failure.not_created);
    let response = failure.error.response_body();
    assert_eq!(response.error_code, 12501);
    assert_eq!(
        response.details.as_ref().unwrap()["reason"],
        "provider_create:400:invalid_value:session.delegation"
    );
    assert_eq!(
        response.details.as_ref().unwrap()["provider_type"],
        "invalid_request_error"
    );
    let rendered = serde_json::to_string(&response).unwrap() + &format!("{:?}", failure.error);
    for secret in [
        "DO-NOT-EXPOSE",
        "sk-secret",
        "instructions",
        "SDP",
        "fixture-secret",
    ] {
        assert!(!rendered.contains(secret));
    }
    server.abort();
}

#[tokio::test]
async fn voice_create_expiry_is_optional_metadata_not_an_answer_requirement() {
    for expiry in [
        Value::Null,
        json!("2026-10-05T14:00:00Z"),
        json!(2_000_000_000.5),
        json!(2_000_000_000_i64),
    ] {
        let expected = expiry.as_i64();
        let app = Router::new().route("/v1/live/sessions",post(move || {let expiry=expiry.clone();async move {
            Json(json!({"session":{"id":"live:opaque.v2","expires_at":expiry},"transport":{"type":"webrtc","sdp":"v=0\r\nm=audio 9 UDP/TLS/RTP/SAVPF 111\r\n"}}))
        }}));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let provider = OpenAi::fixture(Zeroizing::new("fixture-secret".into()), address);
        let created = provider
            .create("v=0", "context", "marin", "gpt-live-1")
            .await
            .unwrap();
        assert_eq!(created.expires_at, expected);
        assert_eq!(created.provider_id, "live:opaque.v2");
        server.abort();
    }
}

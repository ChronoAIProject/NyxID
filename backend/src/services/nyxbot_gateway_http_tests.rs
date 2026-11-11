use super::*;
use crate::{
    models::downstream_service::{COLLECTION_NAME as SERVICES, DownstreamService},
    test_utils::{connect_transaction_test_database, test_app_state},
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use wiremock::{Mock, MockServer, ResponseTemplate, matchers::path};

async fn state_for_base(base: &str) -> AppState {
    let state = test_app_state(connect_transaction_test_database("gateway_http").await);
    let mut service = crate::models::downstream_service::test_helpers::dummy_service();
    service.id = Uuid::new_v4().to_string();
    service.slug = GATEWAY_SLUG.into();
    service.base_url = base.into();
    service.requires_user_credential = false;
    state
        .db
        .collection::<DownstreamService>(SERVICES)
        .insert_one(service)
        .await
        .unwrap();
    state
}

async fn call(state: &AppState, path: &str) -> AppResult<GatewayResponse> {
    gateway_call(
        state,
        reqwest::Method::POST,
        path,
        "fixture-creator-bearer",
        Some(&serde_json::json!({"credentials":{"agent_key":"fixture-provider-key"}})),
        None,
    )
    .await
}

#[tokio::test]
async fn gateway_never_follows_same_origin_or_cross_origin_redirects() {
    let source = MockServer::start().await;
    let target = MockServer::start().await;
    let state = state_for_base(&source.uri()).await;
    for (name, destination) in [
        ("same", format!("{}/target", source.uri())),
        ("cross", format!("{}/target", target.uri())),
    ] {
        for status in [301, 302, 303, 307, 308] {
            let url = format!("/{name}/{status}");
            Mock::given(path(&url))
                .respond_with(
                    ResponseTemplate::new(status).insert_header("location", destination.as_str()),
                )
                .mount(&source)
                .await;
            let response = call(&state, &url).await.unwrap();
            assert_eq!(response.status, status);
        }
    }
    assert!(target.received_requests().await.unwrap().is_empty());
    let requests = source.received_requests().await.unwrap();
    assert_eq!(requests.len(), 10);
    assert!(
        requests
            .iter()
            .all(|request| request.url.path() != "/target")
    );
    state.db.drop().await.unwrap();
}

// Raw HTTP keeps deliberate framing failures intact. The socket stays open
// after advertised/chunked overflow to prove the reader stops immediately.
async fn raw_response(
    head: &str,
    chunks: Vec<Vec<u8>>,
    hold_open: bool,
) -> (String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let head = head.to_owned();
    let server = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = vec![0; 8192];
        let _ = socket.read(&mut request).await.unwrap();
        if socket.write_all(head.as_bytes()).await.is_err() {
            return;
        }
        for chunk in chunks {
            if socket.write_all(&chunk).await.is_err() {
                return;
            }
        }
        if hold_open {
            std::future::pending::<()>().await;
        }
    });
    (base, server)
}

#[tokio::test]
async fn gateway_bounds_advertised_and_chunked_bodies_and_rejects_broken_streams() {
    let chunk = vec![b'x'; 64 * 1024];
    let mut oversized_chunked = Vec::new();
    for _ in 0..33 {
        let mut frame = b"10000\r\n".to_vec();
        frame.extend_from_slice(&chunk);
        frame.extend_from_slice(b"\r\n");
        oversized_chunked.push(frame);
    }
    for (head, chunks, hold) in [
        (
            "HTTP/1.1 200 OK\r\nContent-Length: 2097153\r\n\r\n",
            vec![],
            true,
        ),
        (
            "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n",
            oversized_chunked,
            true,
        ),
        (
            "HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\n",
            vec![b"{\"version\":1}".to_vec()],
            false,
        ),
        (
            "HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n",
            vec![b"7\r\n{\"v\":1}\r\n".to_vec()],
            false,
        ),
    ] {
        let (base, server) = raw_response(head, chunks, hold).await;
        let state = state_for_base(&base).await;
        assert!(
            tokio::time::timeout(Duration::from_secs(3), call(&state, "/channels"))
                .await
                .expect("bounded reader must not wait for overflow tail")
                .is_err()
        );
        server.abort();
        state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn gateway_parses_small_json_and_preserves_http_errors_but_rejects_malformed_success() {
    let server = MockServer::start().await;
    let state = state_for_base(&server.uri()).await;
    Mock::given(path("/small"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"version":1})))
        .mount(&server)
        .await;
    assert_eq!(call(&state, "/small").await.unwrap().body["version"], 1);
    for status in [401, 403, 502] {
        let path_name = format!("/error/{status}");
        Mock::given(path(&path_name))
            .respond_with(ResponseTemplate::new(status).set_body_string("non-JSON upstream error"))
            .mount(&server)
            .await;
        let result = call(&state, &path_name).await.unwrap();
        assert_eq!(result.status, status);
        assert!(result.body.is_null());
    }
    let (base, raw) = raw_response(
        "HTTP/1.1 403 Forbidden\r\nContent-Length: 100\r\n\r\n",
        vec![b"{\"code\":\"not_authenticated\"}".to_vec()],
        false,
    )
    .await;
    let error_state = state_for_base(&base).await;
    let response = call(&error_state, "/channels").await.unwrap();
    assert_eq!(response.status, 403);
    assert!(response.body.is_null());
    raw.await.unwrap();
    error_state.db.drop().await.unwrap();
    Mock::given(path("/broken"))
        .respond_with(ResponseTemplate::new(200).set_body_string("{\"version\":"))
        .mount(&server)
        .await;
    assert!(call(&state, "/broken").await.is_err());
    Mock::given(path("/empty"))
        .respond_with(ResponseTemplate::new(204))
        .mount(&server)
        .await;
    assert_eq!(call(&state, "/empty").await.unwrap().status, 204);
    Mock::given(path("/deleted"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    assert_eq!(
        gateway_call(
            &state,
            reqwest::Method::DELETE,
            "/deleted",
            "fixture-bearer",
            None,
            None
        )
        .await
        .unwrap()
        .status,
        200
    );
    // The gateway's larger cap is independent of setup's existing 64 KiB cap.
    let body = serde_json::json!({"data":"x".repeat(70*1024)});
    Mock::given(path("/setup-large"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&server)
        .await;
    assert!(call(&state, "/setup-large").await.is_ok());
    assert!(
        super::super::platform_entry_admin_service::provider_json(
            GATEWAY_HTTP_CLIENT.post(format!("{}/setup-large", server.uri()))
        )
        .await
        .is_err()
    );
    state.db.drop().await.unwrap();
}

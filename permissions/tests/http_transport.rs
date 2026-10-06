use std::sync::{Arc, Mutex};

use axum::{
    Router,
    extract::State,
    http::{HeaderMap, StatusCode, Uri},
    routing::get,
};
use nyxid_permissions::{Error, Request, Transport, http_transport::NyxIdTransport};
use uuid::Uuid;
use zeroize::Zeroizing;

const UPSTREAM_KEY: &str = "synthetic-test-upstream-key";
type CapturedRequests = Arc<Mutex<Vec<(Uri, HeaderMap)>>>;

#[tokio::test]
async fn transport_pins_catalog_connection_and_credential() {
    let seen = Arc::new(Mutex::new(Vec::<(Uri, HeaderMap)>::new()));
    let app = Router::new()
        .route(
            "/{*path}",
            get(
                |State(seen): State<CapturedRequests>, uri: Uri, headers: HeaderMap| async move {
                    seen.lock().unwrap().push((uri, headers));
                    axum::Json(serde_json::json!({"id":"report"}))
                },
            ),
        )
        .with_state(seen.clone());
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let service = Uuid::new_v4();
    let connection = Uuid::new_v4();
    let transport = NyxIdTransport::new(
        &base,
        service,
        connection,
        Zeroizing::new(UPSTREAM_KEY.into()),
    )
    .unwrap();
    let mut request = Request::get("/drive/v3/files/report");
    request.query.insert("fields".into(), "id,name".into());
    assert_eq!(transport.send(&request).await.unwrap().status, 200);
    {
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert_eq!(
            seen[0].0.path(),
            format!("/api/v1/proxy/{service}/drive/v3/files/report")
        );
        let query = nyxid_permissions::parse_query(seen[0].0.query().unwrap()).unwrap();
        assert_eq!(query["_nyxid_via"], connection.to_string());
        assert_eq!(seen[0].1["authorization"], format!("Bearer {UPSTREAM_KEY}"));
        assert_eq!(seen[0].1["accept-encoding"], "identity");
    }
    request
        .query
        .insert("_nyxid_via".into(), Uuid::new_v4().to_string());
    assert!(transport.send(&request).await.is_err());
    assert_eq!(seen.lock().unwrap().len(), 1);
    server.abort();
}

#[tokio::test]
async fn redirects_are_never_followed() {
    let count = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let app = Router::new()
        .route(
            "/stolen",
            get({
                let count = count.clone();
                move || {
                    count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    async { "stolen" }
                }
            }),
        )
        .route(
            "/{*path}",
            get(|| async { (StatusCode::TEMPORARY_REDIRECT, [("location", "/stolen")]) }),
        );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let transport = NyxIdTransport::new(
        &base,
        Uuid::new_v4(),
        Uuid::new_v4(),
        Zeroizing::new(UPSTREAM_KEY.into()),
    )
    .unwrap();
    assert!(matches!(
        transport
            .send(&Request::get("/drive/v3/files/report"))
            .await,
        Err(Error::Upstream)
    ));
    assert_eq!(count.load(std::sync::atomic::Ordering::SeqCst), 0);
    server.abort();
}

#[test]
fn invalid_origins_are_rejected_without_network_access() {
    for base in [
        "http://google.com",
        "https://user:password@example.com",
        "https://example.com/path",
        "https://example.com?x=1",
        "https://example.com#fragment",
        "file:///tmp/file",
    ] {
        assert!(
            NyxIdTransport::new(
                base,
                Uuid::new_v4(),
                Uuid::new_v4(),
                Zeroizing::new(UPSTREAM_KEY.into())
            )
            .is_err(),
            "{base}"
        );
    }
}

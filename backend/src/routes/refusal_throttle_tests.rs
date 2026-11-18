use std::sync::{Arc, Mutex};

use axum::{
    Extension, Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
    middleware,
    routing::get,
};
use mongodb::{
    bson::doc,
    event::{EventHandler, command::CommandEvent},
};
use tower::ServiceExt;

use crate::{
    models::user::{COLLECTION_NAME as USERS, User, UserType},
    mw::{auth::AuthUser, rate_limit},
    test_utils,
};

#[tokio::test]
async fn refusal_throttle_full_router_preserves_twenty_then_only_authentication_commands() {
    let commands = Arc::new(Mutex::new(Vec::<(String, String)>::new()));
    let captured = commands.clone();
    let db = test_utils::connect_test_database_with_command_handler(
        "refusal_throttle",
        EventHandler::callback(move |event| {
            if let CommandEvent::Started(event) = event {
                let collection = event
                    .command
                    .get_str(&event.command_name)
                    .unwrap_or("")
                    .to_owned();
                captured
                    .lock()
                    .unwrap()
                    .push((event.command_name, collection));
            }
        }),
    )
    .await
    .unwrap();
    let user = uuid::Uuid::new_v4().to_string();
    db.collection::<User>(USERS)
        .insert_one(test_utils::test_user(&user, UserType::Person))
        .await
        .unwrap();
    let key = crate::services::key_service::create_api_key(
        &db,
        &user,
        "refusal test",
        "proxy",
        None,
        None,
        Some(&[]),
        Some(&[]),
        Some(true),
        Some(true),
        Some(false),
        None,
        None,
        None,
        None,
    )
    .await
    .unwrap();
    let mut state = test_utils::test_app_state(db.clone());
    let now = Arc::new(Mutex::new(std::time::Instant::now()));
    let clock = now.clone();
    state.refusal_throttle = Arc::new(crate::mw::refusal_throttle::RefusalThrottle::with_clock(
        Arc::new(move || *clock.lock().unwrap()),
        4096,
    ));
    let (_, private) = super::build_router_with_state(state.clone());
    let app: Router = private
        .route(
            "/api/v1/refusal-throttle-baseline",
            get(|auth: AuthUser| async { (Extension(auth), StatusCode::NO_CONTENT) }),
        )
        .with_state(state.clone())
        .layer(middleware::from_fn(rate_limit::rate_limit_middleware))
        .layer(Extension(rate_limit::create_per_ip_rate_limiter(
            db.clone(),
            "refusal_test_ip",
            100_000,
            60,
        )))
        .layer(Extension(rate_limit::create_rate_limiter(
            db.clone(),
            100_000,
            100_000,
        )))
        .layer(Extension(Arc::new(
            Vec::<crate::config::TrustedProxyRange>::new(),
        )))
        .layer(Extension(rate_limit::RateLimitExemptIps::default()));
    let send = |path: &str| {
        app.clone().oneshot(
            Request::builder()
                .uri(path)
                .header("x-api-key", &key.full_key)
                .body(Body::empty())
                .unwrap(),
        )
    };
    let path = "/api/v1/proxy/s/refusal-missing/items";
    let mut first_body = None;
    let mut counted = 0;
    for _ in 0..20 {
        commands.lock().unwrap().clear();
        let response = send(path).await.unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(
            body.as_ref(),
            br#"{"error":"not_found","error_code":1003,"message":"Not found: Service not found"}"#
        );
        if let Some(first) = &first_body {
            assert_eq!(&body, first);
        } else {
            first_body = Some(body);
        }
        crate::services::shutdown::process().tasks_stopped().await;
        counted = commands.lock().unwrap().len();
    }
    commands.lock().unwrap().clear();
    let baseline_response = send("/api/v1/refusal-throttle-baseline").await.unwrap();
    assert_eq!(baseline_response.status(), StatusCode::NO_CONTENT);
    let real_auth = baseline_response
        .extensions()
        .get::<AuthUser>()
        .unwrap()
        .clone();
    crate::services::shutdown::process().tasks_stopped().await;
    let baseline = std::mem::take(&mut *commands.lock().unwrap());
    let audit_before = db
        .collection::<mongodb::bson::Document>(crate::models::audit_log::COLLECTION_NAME)
        .count_documents(doc! {})
        .await
        .unwrap();
    let usage_before = db
        .collection::<mongodb::bson::Document>(crate::models::usage_meter::COLLECTION_NAME)
        .count_documents(doc! {})
        .await
        .unwrap();
    commands.lock().unwrap().clear();
    let response = send(path).await.unwrap();
    crate::services::shutdown::process().tasks_stopped().await;
    let throttled = std::mem::take(&mut *commands.lock().unwrap());
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(response.headers()["retry-after"], "30");
    assert_eq!(
        throttled, baseline,
        "no commands beyond outer limits and real authentication"
    );
    assert_eq!(
        db.collection::<mongodb::bson::Document>(crate::models::audit_log::COLLECTION_NAME)
            .count_documents(doc! {})
            .await
            .unwrap(),
        audit_before
    );
    assert_eq!(
        db.collection::<mongodb::bson::Document>(crate::models::usage_meter::COLLECTION_NAME)
            .count_documents(doc! {})
            .await
            .unwrap(),
        usage_before
    );
    for _ in 0..100 {
        commands.lock().unwrap().clear();
        let response = send(path).await.unwrap();
        assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
        crate::services::shutdown::process().tasks_stopped().await;
        assert_eq!(*commands.lock().unwrap(), baseline);
    }
    // The LLM pool entrance calls this same handler internally without the route marker.
    for _ in 0..21 {
        let result = crate::handlers::proxy::proxy_request_by_slug(
            axum::extract::State(state.clone()),
            real_auth.clone(),
            Default::default(),
            axum::extract::Path(("refusal-missing".into(), "items".into())),
            Request::builder().uri(path).body(Body::empty()).unwrap(),
        )
        .await;
        assert!(matches!(result, Err(crate::errors::AppError::NotFound(_))));
    }
    // HTTP upgrades and CONNECT retain their existing handling even on an armed path.
    for request in [
        Request::builder()
            .uri(path)
            .header("upgrade", "websocket")
            .header("connection", "upgrade"),
        Request::builder().uri(path).method("CONNECT"),
    ] {
        let response = app
            .clone()
            .oneshot(
                request
                    .header("x-api-key", &key.full_key)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
    *now.lock().unwrap() += std::time::Duration::from_secs(30);
    assert_eq!(send(path).await.unwrap().status(), StatusCode::NOT_FOUND);
    assert_eq!(
        send(path).await.unwrap().status(),
        StatusCode::TOO_MANY_REQUESTS
    );

    test_utils::set_test_approval_preference(&db, &user, false).await;
    let remote_status = Arc::new(std::sync::atomic::AtomicU16::new(404));
    let status = remote_status.clone();
    let remote = Router::new().fallback(move || {
        let status = status.load(std::sync::atomic::Ordering::SeqCst);
        async move { (StatusCode::from_u16(status).unwrap(), axum::Json(serde_json::json!({
            "error": "not_found", "error_code": 1003, "message": "Not found: Service not found"
        }))) }
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let remote_url = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, remote).await.unwrap() });
    let endpoint = test_utils::test_user_endpoint(
        &uuid::Uuid::new_v4().to_string(),
        &user,
        "refusal-missing",
        &remote_url,
        None,
        None,
    );
    let service = test_utils::test_user_service(
        &uuid::Uuid::new_v4().to_string(),
        &user,
        "refusal-missing",
        &endpoint.id,
        None,
        None,
    );
    db.collection::<crate::models::user_endpoint::UserEndpoint>(
        crate::models::user_endpoint::COLLECTION_NAME,
    )
    .insert_one(endpoint)
    .await
    .unwrap();
    db.collection::<crate::models::user_service::UserService>(
        crate::models::user_service::COLLECTION_NAME,
    )
    .insert_one(service)
    .await
    .unwrap();
    *now.lock().unwrap() += std::time::Duration::from_secs(30);
    let response = send(path).await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::NOT_FOUND,
        "the probe is now a downstream 404"
    );
    to_bytes(response.into_body(), usize::MAX).await.unwrap();
    crate::services::shutdown::process().tasks_stopped().await;
    remote_status.store(200, std::sync::atomic::Ordering::SeqCst);
    commands.lock().unwrap().clear();
    let response = send(path).await.unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "downstream outcome cleared the throttle"
    );
    to_bytes(response.into_body(), usize::MAX).await.unwrap();
    crate::services::shutdown::process().tasks_stopped().await;
    let ordinary = commands.lock().unwrap().len();
    eprintln!(
        "refusal throttle command counts: ordinary={ordinary} counted_refusal={counted} throttled={} auth_baseline={}",
        throttled.len(),
        baseline.len()
    );
    eprintln!("refusal throttle baseline commands: {baseline:?}");
    remote_status.store(404, std::sync::atomic::Ordering::SeqCst);
    for _ in 0..100 {
        let response = send(path).await.unwrap();
        assert_eq!(
            response.status(),
            StatusCode::NOT_FOUND,
            "downstream 404 must never count, even with a NyxID-shaped error body"
        );
        to_bytes(response.into_body(), usize::MAX).await.unwrap();
        crate::services::shutdown::process().tasks_stopped().await;
    }
    // Every HTTP spelling, including both root forwards, has its own exact-path key.
    for route in [
        "/api/v1/proxy/s/refusal-absent",
        "/api/v1/proxy/s/refusal-absent/",
        "/api/v1/proxy/00000000-0000-4000-8000-000000000001/items",
        "/api/v1/proxy/00000000-0000-4000-8000-000000000001",
        "/api/v1/proxy/00000000-0000-4000-8000-000000000001/",
    ] {
        for _ in 0..20 {
            assert_eq!(
                send(route).await.unwrap().status(),
                StatusCode::NOT_FOUND,
                "{route}"
            );
        }
        assert_eq!(
            send(route).await.unwrap().status(),
            StatusCode::TOO_MANY_REQUESTS,
            "{route}"
        );
    }
    server.abort();
}

async fn round2_fixture() -> (Router, crate::AppState, String, String) {
    let db = test_utils::connect_test_database("refusal_round2")
        .await
        .unwrap();
    let user = uuid::Uuid::new_v4().to_string();
    db.collection::<User>(USERS)
        .insert_one(test_utils::test_user(&user, UserType::Person))
        .await
        .unwrap();
    let key = crate::services::key_service::create_api_key(
        &db,
        &user,
        "refusal round2",
        "proxy",
        None,
        None,
        Some(&[]),
        Some(&[]),
        Some(true),
        Some(true),
        Some(false),
        None,
        None,
        None,
        None,
    )
    .await
    .unwrap();
    test_utils::set_test_approval_preference(&db, &user, false).await;
    let mut state = test_utils::test_app_state(db);
    let now = std::time::Instant::now();
    state.refusal_throttle = Arc::new(crate::mw::refusal_throttle::RefusalThrottle::with_clock(
        Arc::new(move || now),
        4096,
    ));
    let (_, private) = super::build_router_with_state(state.clone());
    let app = private.with_state(state.clone());
    (app, state, key.full_key, user)
}

fn round2_request(path: &str, key: &str, bytes: &'static [u8]) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(path)
        .header("x-api-key", key)
        .header("content-type", "application/json")
        .header("content-length", bytes.len())
        .body(Body::from(bytes))
        .unwrap()
}

async fn round2_connect(state: &crate::AppState, user: &str, slug: &str, url: &str) {
    let endpoint = test_utils::test_user_endpoint(
        &uuid::Uuid::new_v4().to_string(),
        user,
        slug,
        url,
        None,
        None,
    );
    let service = test_utils::test_user_service(
        &uuid::Uuid::new_v4().to_string(),
        user,
        slug,
        &endpoint.id,
        None,
        None,
    );
    state
        .db
        .collection::<crate::models::user_endpoint::UserEndpoint>(
            crate::models::user_endpoint::COLLECTION_NAME,
        )
        .insert_one(endpoint)
        .await
        .unwrap();
    state
        .db
        .collection::<crate::models::user_service::UserService>(
            crate::models::user_service::COLLECTION_NAME,
        )
        .insert_one(service)
        .await
        .unwrap();
}

#[tokio::test]
async fn refusal_throttle_round2_body_b_is_forwarded_while_body_a_stays_throttled() {
    let (app, state, key, user) = round2_fixture().await;
    let path = "/api/v1/proxy/s/round2-body/items";
    const A: &[u8] = br#"{"workspace_id":"refused"}"#;
    const B: &[u8] = br#"{"workspace_id":"allowed"}"#;
    for _ in 0..20 {
        assert_eq!(
            app.clone()
                .oneshot(round2_request(path, &key, A))
                .await
                .unwrap()
                .status(),
            StatusCode::NOT_FOUND
        );
    }
    let downstream = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::any())
        .respond_with(wiremock::ResponseTemplate::new(200))
        .mount(&downstream)
        .await;
    round2_connect(&state, &user, "round2-body", &downstream.uri()).await;
    let response = app
        .clone()
        .oneshot(round2_request(path, &key, B))
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "a different body must pass"
    );
    to_bytes(response.into_body(), usize::MAX).await.unwrap();
    crate::services::shutdown::process().tasks_stopped().await;
    let received = downstream.received_requests().await.unwrap();
    assert_eq!(received.len(), 1);
    assert_eq!(received[0].body, B);
    assert_eq!(received[0].headers["content-length"], B.len().to_string());
    let mut identical = round2_request(path, &key, A);
    identical
        .headers_mut()
        .insert("idempotency-key", "new-attempt".parse().unwrap());
    assert_eq!(
        app.oneshot(identical).await.unwrap().status(),
        StatusCode::TOO_MANY_REQUESTS
    );
}

#[tokio::test]
async fn refusal_throttle_round2_raw_query_b_is_forwarded_while_query_a_stays_throttled() {
    let (app, state, key, user) = round2_fixture().await;
    let a = "/api/v1/proxy/s/round2-query/items?value=1&value=2";
    let b = "/api/v1/proxy/s/round2-query/items?value=2&value=1";
    for _ in 0..20 {
        assert_eq!(
            app.clone()
                .oneshot(round2_request(a, &key, b"{}"))
                .await
                .unwrap()
                .status(),
            StatusCode::NOT_FOUND
        );
    }
    let downstream = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::any())
        .respond_with(wiremock::ResponseTemplate::new(200))
        .mount(&downstream)
        .await;
    round2_connect(&state, &user, "round2-query", &downstream.uri()).await;
    let response = app
        .clone()
        .oneshot(round2_request(b, &key, b"{}"))
        .await
        .unwrap();
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "raw query strings must be independent"
    );
    to_bytes(response.into_body(), usize::MAX).await.unwrap();
    crate::services::shutdown::process().tasks_stopped().await;
    let received = downstream.received_requests().await.unwrap();
    assert_eq!(received.len(), 1);
    assert_eq!(received[0].url.query(), Some("value=2&value=1"));
    assert_eq!(received[0].body, b"{}");
    assert_eq!(
        app.oneshot(round2_request(a, &key, b"{}"))
            .await
            .unwrap()
            .status(),
        StatusCode::TOO_MANY_REQUESTS
    );
}

#[tokio::test]
async fn refusal_throttle_round2_large_body_never_counts() {
    let (app, _, key, _) = round2_fixture().await;
    let bytes = bytes::Bytes::from(vec![b'x'; 65 * 1024]);
    for _ in 0..100 {
        let request = Request::builder()
            .method("POST")
            .uri("/api/v1/proxy/s/round2-large/items")
            .header("x-api-key", &key)
            .header("content-length", bytes.len())
            .body(Body::from(bytes.clone()))
            .unwrap();
        assert_eq!(
            app.clone().oneshot(request).await.unwrap().status(),
            StatusCode::NOT_FOUND
        );
    }
}

#[tokio::test]
async fn refusal_throttle_round2_streamed_body_never_counts_or_polls_early() {
    let (app, _, key, _) = round2_fixture().await;
    let polls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    for _ in 0..100 {
        let observed = polls.clone();
        let body = Body::from_stream(futures::stream::once(async move {
            observed.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok::<_, std::io::Error>(bytes::Bytes::from_static(b"streamed"))
        }));
        let request = Request::builder()
            .method("POST")
            .uri("/api/v1/proxy/s/round2-stream/items")
            .header("x-api-key", &key)
            .header("transfer-encoding", "chunked")
            .body(body)
            .unwrap();
        assert_eq!(
            app.clone().oneshot(request).await.unwrap().status(),
            StatusCode::NOT_FOUND
        );
    }
    assert_eq!(polls.load(std::sync::atomic::Ordering::SeqCst), 0);
}

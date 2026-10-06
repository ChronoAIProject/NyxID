use std::sync::Arc;

use axum::{
    Router,
    body::{Body, to_bytes},
    extract::{Path, State},
    http::{HeaderMap, Method, Request, StatusCode},
    response::{IntoResponse, Response},
    routing::any,
};
use futures::{StreamExt, TryStreamExt};
use mongodb::bson::{Document, doc};
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::models::user::UserType;
use crate::services::billing::{BillingIngress, route_inventory::BillingRoutePolicy};
use crate::test_utils::{
    connect_transaction_test_database, test_app_state, test_auth_user,
    test_auto_connected_catalog_service, test_user, test_user_endpoint, test_user_service,
};
use crate::{AppState, mw::auth::AuthUser};

#[path = "service_pool_billing_tests.rs"]
mod billing;

#[path = "service_pool_inspection_tests.rs"]
mod inspection;

#[path = "service_pool_runtime_tests.rs"]
mod runtime;

#[path = "service_pool_operation_scope_tests.rs"]
mod operation_scopes;

#[derive(Clone, Debug)]
struct ReceivedRequest {
    method: Method,
    body: bytes::Bytes,
    headers: HeaderMap,
}

struct Upstream {
    url: String,
    requests: Arc<Mutex<Vec<ReceivedRequest>>>,
    server: tokio::task::JoinHandle<()>,
}

impl Drop for Upstream {
    fn drop(&mut self) {
        self.server.abort();
    }
}

async fn upstream(status: StatusCode, label: &'static str) -> Upstream {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let captured = requests.clone();
    let app = Router::new().route(
        "/{*path}",
        any(move |request: Request<Body>| {
            let captured = captured.clone();
            async move {
                let (parts, body) = request.into_parts();
                captured.lock().await.push(ReceivedRequest {
                    method: parts.method,
                    headers: parts.headers,
                    body: to_bytes(body, 1024 * 1024).await.unwrap(),
                });
                let mut response = Response::builder()
                    .status(status)
                    .header("content-type", "application/json");
                if status == StatusCode::TOO_MANY_REQUESTS {
                    response = response.header("retry-after", "120");
                }
                response
                    .body(Body::from(format!("{{\"member\":\"{label}\"}}")))
                    .unwrap()
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    Upstream {
        url,
        requests,
        server,
    }
}

struct Fixture {
    state: AppState,
    auth: AuthUser,
    pool_id: String,
    second_member_id: String,
    first: Upstream,
    second: Upstream,
}

async fn fixture(
    label: &str,
    first_status: StatusCode,
    strategy: &str,
    retry_ambiguous: bool,
) -> Fixture {
    crate::services::audit_service::init_audit_chain_hmac_key(zeroize::Zeroizing::new([73; 32]));
    let db = connect_transaction_test_database(label).await;
    crate::db::ensure_indexes(&db).await.unwrap();
    let owner = Uuid::new_v4().to_string();
    db.collection::<crate::models::user::User>("users")
        .insert_one(test_user(&owner, UserType::Person))
        .await
        .unwrap();
    let first = upstream(first_status, "first").await;
    let second = upstream(StatusCode::OK, "second").await;
    let mut catalog = test_auto_connected_catalog_service();
    catalog.slug = "pool-review-api".into();
    catalog.base_url = first.url.clone();
    db.collection::<crate::models::downstream_service::DownstreamService>("downstream_services")
        .insert_one(&catalog)
        .await
        .unwrap();
    let mut member_ids = Vec::new();
    for (slug, url) in [("review-first", &first.url), ("review-second", &second.url)] {
        let endpoint_id = Uuid::new_v4().to_string();
        let member_id = Uuid::new_v4().to_string();
        let endpoint = test_user_endpoint(&endpoint_id, &owner, slug, url, None, Some(&catalog.id));
        let service = test_user_service(
            &member_id,
            &owner,
            slug,
            &endpoint_id,
            Some(&catalog.id),
            None,
        );
        db.collection::<crate::models::user_endpoint::UserEndpoint>("user_endpoints")
            .insert_one(endpoint)
            .await
            .unwrap();
        db.collection::<crate::models::user_service::UserService>("user_services")
            .insert_one(service)
            .await
            .unwrap();
        member_ids.push(member_id);
    }
    let pool_id = Uuid::new_v4().to_string();
    let mut pool = doc! {
        "_id": &pool_id, "user_id": &owner,
        "slug": "review-route", "name": "Review route", "strategy": strategy,
        "member_contract": "same_api", "config_revision": 0_i64, "is_active": true,
        "members": [
            { "user_service_id": &member_ids[0], "enabled": true, "weight": 1, "priority": 0 },
            { "user_service_id": &member_ids[1], "enabled": true, "weight": 1, "priority": if strategy == "priority" { 1 } else { 0 } },
        ],
        "created_at": mongodb::bson::DateTime::now(), "updated_at": mongodb::bson::DateTime::now(),
    };
    if strategy == "priority" {
        pool.insert(
            "failover",
            doc! {
                "max_attempts": 2, "retry_on": ["http_429", "http_503"],
                "retry_ambiguous_dispatch": retry_ambiguous,
            },
        );
    }
    db.collection::<Document>("service_pools")
        .insert_one(pool)
        .await
        .unwrap();
    Fixture {
        state: test_app_state(db),
        auth: test_auth_user(&owner),
        pool_id,
        second_member_id: member_ids[1].clone(),
        first,
        second,
    }
}

async fn call(fixture: &Fixture, body: &'static str) -> Response {
    try_call(fixture, body)
        .await
        .expect("pool request response")
}

async fn try_call(fixture: &Fixture, body: &'static str) -> crate::errors::AppResult<Response> {
    let mut request = Request::builder()
        .method(Method::POST)
        .uri("/api/v1/proxy/s/review-route/perform")
        .header("content-type", "application/json")
        .header("x-trace-id", "review-request")
        .body(Body::from(body))
        .unwrap();
    request
        .extensions_mut()
        .insert(BillingRoutePolicy::Metered(BillingIngress::Proxy));
    super::proxy::proxy_request_by_slug(
        State(fixture.state.clone()),
        fixture.auth.clone(),
        crate::telemetry::TelemetryContext::default(),
        Path(("review-route".into(), "perform".into())),
        request,
    )
    .await
}

#[tokio::test]
async fn pool_proxy_retries_429_replays_body_and_respects_cooldown() {
    let fixture = fixture(
        "pool_proxy_429",
        StatusCode::TOO_MANY_REQUESTS,
        "priority",
        false,
    )
    .await;
    let response = call(&fixture, "{\"value\":7}").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["x-nyxid-pool-attempts"], "2");
    assert_eq!(response.headers()["x-nyxid-pool-member"], "review-second");
    assert_eq!(
        to_bytes(response.into_body(), 1024).await.unwrap(),
        "{\"member\":\"second\"}"
    );
    for upstream in [&fixture.first, &fixture.second] {
        let requests = upstream.requests.lock().await;
        assert_eq!(requests.len(), 1);
        assert_eq!(requests[0].method, Method::POST);
        assert_eq!(requests[0].body, "{\"value\":7}");
        assert_eq!(requests[0].headers["x-trace-id"], "review-request");
        assert_eq!(requests[0].headers["content-type"], "application/json");
    }
    let events = tokio::time::timeout(std::time::Duration::from_secs(10), async {
        loop {
            let events: Vec<crate::models::audit_log::AuditLog> = fixture
                .state
                .db
                .collection("audit_log")
                .find(doc! {"event_type":"service_pool_attempt"})
                .sort(doc! {"event_data.attempt":1})
                .await
                .unwrap()
                .try_collect()
                .await
                .unwrap();
            if events.len() >= 2 {
                break events;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("429 fallback records both attempted members");
    assert_eq!(events.len(), 2);
    for (index, event) in events.iter().enumerate() {
        let data = event.event_data.as_ref().unwrap();
        assert_eq!(data["pool_id"], fixture.pool_id);
        assert_eq!(data["attempt"], index + 1);
        assert_eq!(data["priority"], index);
        assert_eq!(data["upstream_status"], if index == 0 { 429 } else { 200 });
        assert!(event.seq.is_some() && event.entry_hash.is_some());
        assert!(!data.to_string().contains("127.0.0.1"));
    }
    assert_ne!(
        events[0].event_data.as_ref().unwrap()["user_service_id"],
        events[1].event_data.as_ref().unwrap()["user_service_id"]
    );
    let response = call(&fixture, "{\"value\":8}").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["x-nyxid-pool-attempts"], "1");
    to_bytes(response.into_body(), 1024).await.unwrap();
    assert_eq!(fixture.first.requests.lock().await.len(), 1);
    assert_eq!(fixture.second.requests.lock().await.len(), 2);
    fixture.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn pool_proxy_bad_request_does_not_try_backup() {
    let fixture = fixture("pool_proxy_400", StatusCode::BAD_REQUEST, "priority", true).await;
    let response = call(&fixture, "{}").await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(
        to_bytes(response.into_body(), 1024).await.unwrap(),
        "{\"member\":\"first\"}"
    );
    assert_eq!(fixture.first.requests.lock().await.len(), 1);
    assert!(fixture.second.requests.lock().await.is_empty());
    fixture.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn pool_proxy_ambiguous_post_needs_explicit_replay_permission() {
    for enabled in [false, true] {
        let fixture = fixture(
            "pool_proxy_503",
            StatusCode::SERVICE_UNAVAILABLE,
            "priority",
            enabled,
        )
        .await;
        let response = call(&fixture, "{}").await;
        assert_eq!(
            response.status(),
            if enabled {
                StatusCode::OK
            } else {
                StatusCode::SERVICE_UNAVAILABLE
            }
        );
        to_bytes(response.into_body(), 1024).await.unwrap();
        assert_eq!(fixture.first.requests.lock().await.len(), 1);
        assert_eq!(
            fixture.second.requests.lock().await.len(),
            usize::from(enabled)
        );
        fixture.state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn pool_proxy_legacy_round_robin_remains_single_attempt() {
    let fixture = fixture(
        "pool_proxy_legacy",
        StatusCode::TOO_MANY_REQUESTS,
        "round_robin",
        false,
    )
    .await;
    fixture
        .state
        .db
        .collection::<Document>("service_pools")
        .update_one(
            doc! { "_id": &fixture.pool_id },
            doc! { "$unset": {
                "config_revision": "", "member_contract": "", "members.$[].priority": "",
            } },
        )
        .await
        .unwrap();
    let response = call(&fixture, "{}").await;
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    to_bytes(response.into_body(), 1024).await.unwrap();
    assert!(fixture.second.requests.lock().await.is_empty());
    let response = call(&fixture, "{}").await;
    assert_eq!(response.status(), StatusCode::OK);
    to_bytes(response.into_body(), 1024).await.unwrap();
    assert_eq!(fixture.first.requests.lock().await.len(), 1);
    assert_eq!(fixture.second.requests.lock().await.len(), 1);
    let response = call(&fixture, "{}").await;
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    to_bytes(response.into_body(), 1024).await.unwrap();
    assert_eq!(fixture.first.requests.lock().await.len(), 2);
    assert_eq!(fixture.second.requests.lock().await.len(), 1);
    fixture.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn pool_proxy_legacy_weighted_remains_single_attempt_without_cooldown() {
    let fixture = fixture(
        "pool_proxy_legacy_weighted",
        StatusCode::TOO_MANY_REQUESTS,
        "weighted",
        false,
    )
    .await;
    fixture
        .state
        .db
        .collection::<Document>("service_pools")
        .update_one(
            doc! {"_id": &fixture.pool_id},
            doc! {"$set":{"members.0.weight":2}},
        )
        .await
        .unwrap();
    for expected in [429, 429, 200, 429] {
        let response = call(&fixture, "{}").await;
        assert_eq!(response.status().as_u16(), expected);
        to_bytes(response.into_body(), 1024).await.unwrap();
    }
    assert_eq!(fixture.first.requests.lock().await.len(), 3);
    assert_eq!(fixture.second.requests.lock().await.len(), 1);
    assert_eq!(
        fixture
            .state
            .db
            .collection::<Document>("service_pool_member_health")
            .count_documents(doc! {})
            .await
            .unwrap(),
        0
    );
    fixture.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn pool_proxy_restricted_scope_filters_before_attempt_bound() {
    let mut fixture = fixture("pool_proxy_scope", StatusCode::OK, "priority", false).await;
    bind_first_credential(&fixture).await;
    let decrypts_before = fixture.state.encryption_keys.decrypt_stats();
    fixture.auth.allow_all_services = false;
    fixture.auth.allowed_service_ids = vec![fixture.second_member_id.clone()];
    fixture
        .state
        .db
        .collection::<Document>("service_pools")
        .update_one(
            doc! { "_id": &fixture.pool_id },
            doc! { "$set": { "failover.max_attempts": 1 } },
        )
        .await
        .unwrap();
    let response = call(&fixture, "{}").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["x-nyxid-pool-member"], "review-second");
    to_bytes(response.into_body(), 1024).await.unwrap();
    assert!(fixture.first.requests.lock().await.is_empty());
    assert_eq!(fixture.second.requests.lock().await.len(), 1);
    assert_eq!(
        fixture.state.encryption_keys.decrypt_stats(),
        decrypts_before
    );
    fixture.state.db.drop().await.unwrap();
}

async fn replace_first_with_failing_stream(
    fixture: &mut Fixture,
    before_first: bool,
) -> tokio::sync::oneshot::Sender<()> {
    let (release, pending) = tokio::sync::oneshot::channel();
    let pending = Arc::new(Mutex::new(Some(pending)));
    let requests = Arc::new(Mutex::new(Vec::new()));
    let captured = requests.clone();
    let app = Router::new().route(
        "/{*path}",
        any(move |request: Request<Body>| {
            let pending = pending.clone();
            let captured = captured.clone();
            async move {
                let (parts, body) = request.into_parts();
                captured.lock().await.push(ReceivedRequest {
                    method: parts.method,
                    headers: parts.headers,
                    body: to_bytes(body, 1024 * 1024).await.unwrap(),
                });
                let pending = pending.lock().await.take().unwrap();
                let stream = async_stream::stream! {
                    if !before_first {
                        yield Ok::<_, std::io::Error>(bytes::Bytes::from_static(b"first"));
                        let _ = pending.await;
                    }
                    yield Err(std::io::Error::other("review upstream stream failed"));
                };
                Response::builder()
                    .status(StatusCode::OK)
                    .header("content-type", "application/octet-stream")
                    .body(Body::from_stream(stream))
                    .unwrap()
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    fixture
        .state
        .db
        .collection::<Document>("user_endpoints")
        .update_one(
            doc! { "label": "review-first" },
            doc! { "$set": { "url": &url } },
        )
        .await
        .unwrap();
    fixture
        .state
        .db
        .collection::<Document>("service_pools")
        .update_one(
            doc! { "_id": &fixture.pool_id },
            doc! { "$set": {
                "failover.retry_on": ["transport_error"],
                "failover.retry_ambiguous_dispatch": true,
            } },
        )
        .await
        .unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    fixture.first = Upstream {
        url,
        requests,
        server,
    };
    release
}

#[tokio::test]
async fn pool_proxy_pre_first_body_failure_can_retry_with_explicit_replay() {
    let mut fixture = fixture("pool_proxy_first_frame", StatusCode::OK, "priority", true).await;
    let _release = replace_first_with_failing_stream(&mut fixture, true).await;
    let response = tokio::time::timeout(std::time::Duration::from_secs(10), call(&fixture, "{}"))
        .await
        .expect("fallback must complete within the attempt budget");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["x-nyxid-pool-attempts"], "2");
    assert_eq!(
        to_bytes(response.into_body(), 1024).await.unwrap(),
        "{\"member\":\"second\"}"
    );
    assert_eq!(fixture.first.requests.lock().await.len(), 1);
    assert_eq!(fixture.second.requests.lock().await.len(), 1);
    fixture.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn pool_proxy_mid_stream_failure_never_switches_members() {
    let mut fixture = fixture("pool_proxy_mid_stream", StatusCode::OK, "priority", true).await;
    let release = replace_first_with_failing_stream(&mut fixture, false).await;
    let response = tokio::time::timeout(std::time::Duration::from_secs(10), call(&fixture, "{}"))
        .await
        .expect("first member must commit its available data");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["x-nyxid-pool-attempts"], "1");
    let mut body = response.into_body().into_data_stream();
    let first = tokio::time::timeout(std::time::Duration::from_secs(5), body.next())
        .await
        .unwrap()
        .unwrap()
        .unwrap();
    assert_eq!(first, "first");
    release.send(()).unwrap();
    let next = tokio::time::timeout(std::time::Duration::from_secs(5), body.next())
        .await
        .expect("committed stream must report its upstream failure");
    assert!(
        matches!(next, Some(Err(_))),
        "stream failure must not masquerade as successful EOF"
    );
    assert!(fixture.second.requests.lock().await.is_empty());
    fixture.state.db.drop().await.unwrap();
}

async fn replace_second_status(fixture: &mut Fixture, status: StatusCode) {
    let replacement = upstream(status, "second").await;
    fixture
        .state
        .db
        .collection::<Document>("user_endpoints")
        .update_one(
            doc! { "label": "review-second" },
            doc! { "$set": { "url": &replacement.url } },
        )
        .await
        .unwrap();
    fixture.second = replacement;
}

#[tokio::test]
async fn pool_proxy_replay_body_limit_keeps_the_first_attempt() {
    let fixture = fixture(
        "pool_proxy_replay_limit",
        StatusCode::TOO_MANY_REQUESTS,
        "priority",
        true,
    )
    .await;
    fixture
        .state
        .db
        .collection::<Document>("service_pools")
        .update_one(
            doc! { "_id": &fixture.pool_id },
            doc! { "$set": { "failover.max_replay_body_bytes": 3 } },
        )
        .await
        .unwrap();
    let response = call(&fixture, "{\"value\":7}").await;
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    assert_eq!(response.headers()["x-nyxid-pool-attempts"], "1");
    to_bytes(response.into_body(), 1024).await.unwrap();
    let requests = fixture.first.requests.lock().await;
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].body, "{\"value\":7}");
    assert!(fixture.second.requests.lock().await.is_empty());
    fixture.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn pool_proxy_exhaustion_preserves_the_last_upstream_response() {
    let mut fixture = fixture(
        "pool_proxy_exhausted",
        StatusCode::TOO_MANY_REQUESTS,
        "priority",
        true,
    )
    .await;
    replace_second_status(&mut fixture, StatusCode::SERVICE_UNAVAILABLE).await;
    let response = call(&fixture, "{}").await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(response.headers()["x-nyxid-pool-attempts"], "2");
    assert_eq!(response.headers()["x-nyxid-pool-member"], "review-second");
    assert_eq!(
        to_bytes(response.into_body(), 1024).await.unwrap(),
        "{\"member\":\"second\"}"
    );
    assert_eq!(fixture.first.requests.lock().await.len(), 1);
    assert_eq!(fixture.second.requests.lock().await.len(), 1);
    fixture.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn pool_proxy_all_members_cooling_returns_retry_information_without_dispatch() {
    let mut fixture = fixture(
        "pool_proxy_all_cooled",
        StatusCode::TOO_MANY_REQUESTS,
        "priority",
        false,
    )
    .await;
    replace_second_status(&mut fixture, StatusCode::TOO_MANY_REQUESTS).await;
    let response = call(&fixture, "{}").await;
    assert_eq!(response.status(), StatusCode::TOO_MANY_REQUESTS);
    to_bytes(response.into_body(), 1024).await.unwrap();
    let response = match try_call(&fixture, "{}").await {
        Ok(response) => response,
        Err(error) => error.into_response(),
    };
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let retry_after = response
        .headers()
        .get("retry-after")
        .expect("all-cooled response must expose when it can be retried")
        .to_str()
        .unwrap()
        .parse::<u64>()
        .unwrap();
    let earliest = fixture
        .state
        .db
        .collection::<Document>(crate::models::service_pool_member_health::COLLECTION_NAME)
        .find_one(doc! { "pool_id": &fixture.pool_id, "cooldown_until": { "$type": "date" } })
        .sort(doc! { "cooldown_until": 1 })
        .await
        .unwrap()
        .unwrap()
        .get_datetime("cooldown_until")
        .unwrap()
        .to_chrono();
    let expected_seconds = (earliest - chrono::Utc::now()).num_seconds().max(1) as u64;
    assert!(
        retry_after.abs_diff(expected_seconds) <= 2,
        "Retry-After {retry_after} must describe the actual earliest recovery, approximately {expected_seconds}s"
    );
    to_bytes(response.into_body(), 16 * 1024).await.unwrap();
    assert_eq!(fixture.first.requests.lock().await.len(), 1);
    assert_eq!(fixture.second.requests.lock().await.len(), 1);
    fixture.state.db.drop().await.unwrap();
}

#[derive(Clone, Copy)]
enum StallAt {
    Headers,
    FirstBody,
    RejectionFirstBody,
    ForbiddenFirstBody,
    RejectionBody,
}

async fn replace_first_with_stalled_response(
    fixture: &mut Fixture,
    stall_at: StallAt,
) -> tokio::sync::oneshot::Sender<()> {
    let (release, pending) = tokio::sync::oneshot::channel();
    let pending = Arc::new(Mutex::new(Some(pending)));
    let requests = Arc::new(Mutex::new(Vec::new()));
    let captured = requests.clone();
    let app = Router::new().route(
        "/{*path}",
        any(move |request: Request<Body>| {
            let captured = captured.clone();
            let pending = pending.clone();
            async move {
                let (parts, body) = request.into_parts();
                captured.lock().await.push(ReceivedRequest {
                    method: parts.method,
                    headers: parts.headers,
                    body: to_bytes(body, 1024 * 1024).await.unwrap(),
                });
                let pending = pending.lock().await.take().unwrap();
                if matches!(stall_at, StallAt::Headers) {
                    let _ = pending.await;
                    return Response::new(Body::from("late response"));
                }
                let stream = async_stream::stream! {
                    if matches!(stall_at, StallAt::RejectionBody) {
                        yield Ok::<_, std::io::Error>(bytes::Bytes::from_static(b"partial rejection"));
                    }
                    let _ = pending.await;
                    yield Ok(bytes::Bytes::from_static(b"late response"));
                };
                let rejection = matches!(stall_at, StallAt::RejectionBody | StallAt::RejectionFirstBody);
                let mut response = Response::builder()
                    .status(if matches!(stall_at, StallAt::ForbiddenFirstBody) {
                        StatusCode::FORBIDDEN
                    } else if rejection {
                        StatusCode::TOO_MANY_REQUESTS
                    } else {
                        StatusCode::OK
                    })
                    .header("content-type", "application/octet-stream");
                if rejection {
                    response = response.header("retry-after", "120");
                }
                response
                    .body(Body::from_stream(stream))
                    .unwrap()
            }
        }),
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    fixture
        .state
        .db
        .collection::<Document>("user_endpoints")
        .update_one(
            doc! { "label": "review-first" },
            doc! { "$set": { "url": &url } },
        )
        .await
        .unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    fixture.first = Upstream {
        url,
        requests,
        server,
    };
    release
}

#[tokio::test]
async fn pool_proxy_attempt_deadline_bounds_headers_first_body_and_rejection_drain() {
    for stall_at in [StallAt::Headers, StallAt::FirstBody, StallAt::RejectionBody] {
        let mut fixture = fixture(
            "pool_proxy_attempt_deadline",
            StatusCode::OK,
            "priority",
            true,
        )
        .await;
        let _release = replace_first_with_stalled_response(&mut fixture, stall_at).await;
        fixture
            .state
            .db
            .collection::<Document>("service_pools")
            .update_one(
                doc! { "_id": &fixture.pool_id },
                doc! { "$set": {
                    "failover.per_attempt_timeout_ms": 1000,
                    // The 1 s per-attempt bound is what this test proves. The
                    // overall budget only needs slack for slow (coverage)
                    // runs: a broken per-attempt bound still stalls into it.
                    "failover.overall_deadline_ms": 30000,
                    "failover.retry_on": ["http_429", "timeout"],
                } },
            )
            .await
            .unwrap();
        let response =
            tokio::time::timeout(std::time::Duration::from_secs(35), call(&fixture, "{}"))
                .await
                .expect("stalled attempt must leave time to try the backup");
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()["x-nyxid-pool-attempts"], "2");
        assert_eq!(
            to_bytes(response.into_body(), 1024).await.unwrap(),
            "{\"member\":\"second\"}"
        );
        assert_eq!(fixture.first.requests.lock().await.len(), 1);
        assert_eq!(fixture.second.requests.lock().await.len(), 1);
        fixture.state.db.drop().await.unwrap();
    }
}

#[tokio::test]
async fn pool_proxy_overall_deadline_stops_before_backup_dispatch() {
    let mut fixture = fixture(
        "pool_proxy_overall_deadline",
        StatusCode::OK,
        "priority",
        true,
    )
    .await;
    let _release = replace_first_with_stalled_response(&mut fixture, StallAt::Headers).await;
    fixture
        .state
        .db
        .collection::<Document>("service_pools")
        .update_one(
            doc! { "_id": &fixture.pool_id },
            doc! { "$set": {
                "failover.per_attempt_timeout_ms": 5000,
                "failover.overall_deadline_ms": 1000,
                "failover.retry_on": ["timeout"],
            } },
        )
        .await
        .unwrap();
    let outcome = tokio::time::timeout(std::time::Duration::from_secs(4), try_call(&fixture, "{}"))
        .await
        .expect("overall deadline must bound a pending transport request");
    let response = match outcome {
        Ok(response) => response,
        Err(error) => error.into_response(),
    };
    assert_eq!(response.status(), StatusCode::GATEWAY_TIMEOUT);
    let error: serde_json::Value =
        serde_json::from_slice(&to_bytes(response.into_body(), 4096).await.unwrap()).unwrap();
    assert_eq!(error["error"], "service_pool_deadline_exceeded");
    assert_eq!(error["details"]["attempts"][0]["attempt"], 1);
    assert_eq!(error["details"]["attempts"][0]["reason"], "timeout");
    assert_eq!(fixture.first.requests.lock().await.len(), 1);
    assert!(fixture.second.requests.lock().await.is_empty());
    fixture.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn pool_proxy_upgrade_rejected_before_any_upstream_dispatch() {
    let fixture = fixture("pool_proxy_upgrade", StatusCode::OK, "priority", true).await;
    let mut request = Request::builder()
        .method(Method::GET)
        .uri("/api/v1/proxy/s/review-route/perform")
        .header("connection", "upgrade")
        .header("upgrade", "websocket")
        .header("sec-websocket-version", "13")
        .header("sec-websocket-key", "dGhlIHNhbXBsZSBub25jZQ==")
        .body(Body::empty())
        .unwrap();
    request
        .extensions_mut()
        .insert(BillingRoutePolicy::Metered(BillingIngress::Proxy));
    let outcome = super::proxy::proxy_request_by_slug(
        State(fixture.state.clone()),
        fixture.auth.clone(),
        crate::telemetry::TelemetryContext::default(),
        Path(("review-route".into(), "perform".into())),
        request,
    )
    .await;
    let response = match outcome {
        Ok(response) => response,
        Err(error) => error.into_response(),
    };
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert!(fixture.first.requests.lock().await.is_empty());
    assert!(fixture.second.requests.lock().await.is_empty());
    fixture.state.db.drop().await.unwrap();
}

async fn use_org_member(
    fixture: &mut Fixture,
    role: crate::models::org_membership::OrgRole,
    allowed_service_ids: Option<Vec<String>>,
) {
    let org_id = fixture.auth.user_id.to_string();
    let actor = Uuid::new_v4().to_string();
    fixture
        .state
        .db
        .collection::<Document>("users")
        .update_one(
            doc! { "_id": &org_id },
            doc! { "$set": {
                "user_type": "org", "slug": "pool-review-org",
            } },
        )
        .await
        .unwrap();
    fixture
        .state
        .db
        .collection::<crate::models::user::User>("users")
        .insert_one(test_user(&actor, UserType::Person))
        .await
        .unwrap();
    fixture
        .state
        .db
        .collection::<crate::models::org_membership::OrgMembership>("org_memberships")
        .insert_one(crate::test_utils::test_membership(
            &org_id,
            &actor,
            role,
            allowed_service_ids,
        ))
        .await
        .unwrap();
    fixture.auth = test_auth_user(&actor);
}

#[tokio::test]
async fn pool_proxy_org_role_scope_filters_before_attempt_bound() {
    let mut fixture = fixture("pool_proxy_org_scope", StatusCode::OK, "priority", false).await;
    bind_first_credential(&fixture).await;
    let decrypts_before = fixture.state.encryption_keys.decrypt_stats();
    let allowed = vec![fixture.second_member_id.clone()];
    use_org_member(
        &mut fixture,
        crate::models::org_membership::OrgRole::Member,
        Some(allowed),
    )
    .await;
    fixture
        .state
        .db
        .collection::<Document>("service_pools")
        .update_one(
            doc! { "_id": &fixture.pool_id },
            doc! { "$set": {
                "failover.max_attempts": 1,
            } },
        )
        .await
        .unwrap();
    let response = call(&fixture, "{}").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(response.headers()["x-nyxid-pool-member"], "review-second");
    assert_eq!(response.headers()["x-nyxid-pool-attempts"], "1");
    to_bytes(response.into_body(), 1024).await.unwrap();
    assert!(fixture.first.requests.lock().await.is_empty());
    assert_eq!(fixture.second.requests.lock().await.len(), 1);
    assert_eq!(
        fixture.state.encryption_keys.decrypt_stats(),
        decrypts_before
    );
    fixture.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn pool_proxy_org_viewer_cannot_execute_any_member() {
    let mut fixture = fixture("pool_proxy_org_viewer", StatusCode::OK, "priority", true).await;
    bind_first_credential(&fixture).await;
    let decrypts_before = fixture.state.encryption_keys.decrypt_stats();
    use_org_member(
        &mut fixture,
        crate::models::org_membership::OrgRole::Viewer,
        None,
    )
    .await;
    let response = match try_call(&fixture, "{}").await {
        Ok(response) => response,
        Err(error) => error.into_response(),
    };
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert!(fixture.first.requests.lock().await.is_empty());
    assert!(fixture.second.requests.lock().await.is_empty());
    assert_eq!(
        fixture.state.encryption_keys.decrypt_stats(),
        decrypts_before
    );
    fixture.state.db.drop().await.unwrap();
}

async fn bind_first_credential(fixture: &Fixture) {
    let service = fixture
        .state
        .db
        .collection::<Document>("user_services")
        .find_one(doc! { "slug": "review-first" })
        .await
        .unwrap()
        .unwrap();
    let key_id = Uuid::new_v4().to_string();
    let encrypted = fixture
        .state
        .encryption_keys
        .encrypt(b"pool-review-fixture")
        .await
        .unwrap();
    fixture.state.db.collection::<Document>("user_api_keys")
        .insert_one(doc! {
            "_id": &key_id, "user_id": service.get_str("user_id").unwrap(),
            "label": "Review credential", "credential_type": "bearer", "status": "active",
            "credential_encrypted": mongodb::bson::Binary {
                subtype: mongodb::bson::spec::BinarySubtype::Generic, bytes: encrypted,
            },
            "credential_epoch": 1_i64,
            "created_at": mongodb::bson::DateTime::now(), "updated_at": mongodb::bson::DateTime::now(),
        }).await.unwrap();
    fixture
        .state
        .db
        .collection::<Document>("user_services")
        .update_one(
            doc! { "_id": service.get_str("_id").unwrap() },
            doc! { "$set": {
                "api_key_id": key_id, "auth_method": "bearer", "auth_key_name": "Authorization",
            } },
        )
        .await
        .unwrap();
}

#[tokio::test]
async fn pool_proxy_materializes_the_selected_members_credential() {
    let fixture = fixture(
        "pool_proxy_selected_credential",
        StatusCode::OK,
        "priority",
        false,
    )
    .await;
    bind_first_credential(&fixture).await;
    let decrypts_before = fixture.state.encryption_keys.decrypt_stats();
    let response = call(&fixture, "{}").await;
    assert_eq!(response.status(), StatusCode::OK);
    to_bytes(response.into_body(), 1024).await.unwrap();
    assert!(fixture.state.encryption_keys.decrypt_stats().v2_current > decrypts_before.v2_current);
    assert_eq!(
        fixture.first.requests.lock().await[0].headers["authorization"],
        "Bearer pool-review-fixture"
    );
    assert!(fixture.second.requests.lock().await.is_empty());
    fixture.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn pool_proxy_org_pool_does_not_shadow_a_personal_service() {
    let mut fixture = fixture(
        "pool_proxy_personal_precedence",
        StatusCode::OK,
        "priority",
        true,
    )
    .await;
    use_org_member(
        &mut fixture,
        crate::models::org_membership::OrgRole::Member,
        None,
    )
    .await;
    let personal = upstream(StatusCode::OK, "personal").await;
    let actor = fixture.auth.user_id.to_string();
    let endpoint_id = Uuid::new_v4().to_string();
    let service_id = Uuid::new_v4().to_string();
    let catalog = fixture
        .state
        .db
        .collection::<Document>("downstream_services")
        .find_one(doc! { "slug": "pool-review-api" })
        .await
        .unwrap()
        .unwrap();
    let catalog_id = catalog.get_str("_id").unwrap();
    fixture
        .state
        .db
        .collection::<crate::models::user_endpoint::UserEndpoint>("user_endpoints")
        .insert_one(test_user_endpoint(
            &endpoint_id,
            &actor,
            "review-route",
            &personal.url,
            None,
            Some(catalog_id),
        ))
        .await
        .unwrap();
    fixture
        .state
        .db
        .collection::<crate::models::user_service::UserService>("user_services")
        .insert_one(test_user_service(
            &service_id,
            &actor,
            "review-route",
            &endpoint_id,
            Some(catalog_id),
            None,
        ))
        .await
        .unwrap();
    let response = call(&fixture, "{}").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        to_bytes(response.into_body(), 1024).await.unwrap(),
        "{\"member\":\"personal\"}"
    );
    assert_eq!(personal.requests.lock().await.len(), 1);
    assert!(fixture.first.requests.lock().await.is_empty());
    assert!(fixture.second.requests.lock().await.is_empty());
    fixture.state.db.drop().await.unwrap();
}

#[tokio::test]
async fn pool_proxy_legacy_approval_preflight_matches_the_selected_member() {
    let fixture = fixture(
        "pool_proxy_legacy_approval",
        StatusCode::OK,
        "round_robin",
        false,
    )
    .await;
    bind_first_credential(&fixture).await;
    let decrypts_before = fixture.state.encryption_keys.decrypt_stats();
    let first_catalog = fixture
        .state
        .db
        .collection::<crate::models::downstream_service::DownstreamService>("downstream_services")
        .find_one(doc! { "slug": "pool-review-api" })
        .await
        .unwrap()
        .unwrap();
    let mut second_catalog = first_catalog.clone();
    second_catalog.id = Uuid::new_v4().to_string();
    second_catalog.slug = "pool-review-backup-api".into();
    second_catalog.base_url = fixture.second.url.clone();
    fixture
        .state
        .db
        .collection::<crate::models::downstream_service::DownstreamService>("downstream_services")
        .insert_one(&second_catalog)
        .await
        .unwrap();
    fixture
        .state
        .db
        .collection::<Document>("user_services")
        .update_one(
            doc! { "_id": &fixture.second_member_id },
            doc! { "$set": {
                "catalog_service_id": &second_catalog.id,
            } },
        )
        .await
        .unwrap();
    fixture
        .state
        .db
        .collection::<Document>("user_endpoints")
        .update_one(
            doc! { "label": "review-second" },
            doc! { "$set": {
                "catalog_service_id": &second_catalog.id,
            } },
        )
        .await
        .unwrap();
    fixture.state.db.collection::<Document>("service_approval_configs").insert_one(doc! {
        "_id": Uuid::new_v4().to_string(), "user_id": fixture.auth.user_id.to_string(),
        "service_id": first_catalog.id, "service_name": "Denied first member",
        "approval_required": false, "rules": [], "default_effect": "deny",
        "created_at": mongodb::bson::DateTime::now(), "updated_at": mongodb::bson::DateTime::now(),
    }).await.unwrap();
    fixture
        .state
        .db
        .collection::<Document>("service_pools")
        .update_one(
            doc! { "_id": &fixture.pool_id },
            doc! { "$set": { "rr_counter": 1_i64 } },
        )
        .await
        .unwrap();
    let response = call(&fixture, "{}").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        to_bytes(response.into_body(), 1024).await.unwrap(),
        "{\"member\":\"second\"}"
    );
    assert!(fixture.first.requests.lock().await.is_empty());
    assert_eq!(fixture.second.requests.lock().await.len(), 1);
    assert_eq!(
        fixture.state.encryption_keys.decrypt_stats(),
        decrypts_before
    );
    fixture.state.db.drop().await.unwrap();
}

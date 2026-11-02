//! Proxy root spellings (#2127): `/proxy/s/{slug}` and `/proxy/s/{slug}/`
//! (and the UUID equivalents) must resolve the same service and forward `/`.
//! A catch-all never matches an empty tail, so the trailing-slash form used to
//! fall through to `/proxy/{service_id}/{*path}` with service id `s`.
use super::build_router_with_state;
use crate::models::downstream_service::{COLLECTION_NAME as CATALOG, DownstreamService};
use crate::models::user::{COLLECTION_NAME as USERS, User, UserType};
use crate::models::user_endpoint::{COLLECTION_NAME as ENDPOINTS, UserEndpoint};
use crate::models::user_service::{COLLECTION_NAME as SERVICES, UserService};
use crate::test_utils::{
    connect_test_database, test_app_state, test_user, test_user_endpoint, test_user_service,
};
use axum::Router;
use axum::body::{Body, to_bytes};
use axum::http::{Request, StatusCode};
use axum::routing::any;
use serde_json::Value;
use tower::ServiceExt;
use uuid::Uuid;

async fn start_echo_downstream() -> (String, tokio::task::JoinHandle<()>) {
    async fn echo(request: Request<Body>) -> axum::Json<Value> {
        axum::Json(serde_json::json!({ "path": request.uri().path() }))
    }
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind echo downstream");
    let address = listener.local_addr().expect("echo downstream address");
    let app = Router::new().fallback(any(echo));
    let server = tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("serve echo downstream");
    });
    (format!("http://{address}"), server)
}

async fn forwarded_path(app: &Router, token: &str, uri: &str) -> (StatusCode, Value) {
    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(uri)
                .header("authorization", format!("Bearer {token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let status = response.status();
    let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
    (
        status,
        serde_json::from_slice(&body)
            .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(&body).into_owned())),
    )
}

#[tokio::test]
async fn proxy_root_with_and_without_trailing_slash_forwards_root() {
    let db = connect_test_database("proxy_root_trailing_slash")
        .await
        .expect("proxy root router tests require MongoDB; tests must not skip");
    let owner = Uuid::new_v4().to_string();
    db.collection::<User>(USERS)
        .insert_one(test_user(&owner, UserType::Person))
        .await
        .unwrap();
    let (downstream_url, downstream) = start_echo_downstream().await;
    let slug = format!("root-slash-{}", &Uuid::new_v4().simple().to_string()[..8]);
    // The UUID route addresses the catalog service; the caller's connection
    // to it supplies the endpoint, as in the billing route smoke.
    let mut catalog = crate::models::downstream_service::test_helpers::dummy_service();
    catalog.id = Uuid::new_v4().to_string();
    catalog.slug = format!("{slug}-catalog");
    catalog.base_url = downstream_url.clone();
    db.collection::<DownstreamService>(CATALOG)
        .insert_one(&catalog)
        .await
        .unwrap();
    let endpoint = test_user_endpoint(
        &Uuid::new_v4().to_string(),
        &owner,
        &slug,
        &downstream_url,
        None,
        Some(&catalog.id),
    );
    let service = test_user_service(
        &Uuid::new_v4().to_string(),
        &owner,
        &slug,
        &endpoint.id,
        Some(&catalog.id),
        None,
    );
    db.collection::<UserEndpoint>(ENDPOINTS)
        .insert_one(&endpoint)
        .await
        .unwrap();
    db.collection::<UserService>(SERVICES)
        .insert_one(&service)
        .await
        .unwrap();

    let state = test_app_state(db);
    let token = crate::crypto::jwt::generate_access_token(
        &state.jwt_keys,
        &state.config,
        &Uuid::parse_str(&owner).unwrap(),
        "proxy",
        None,
        None,
        None,
        None,
        None,
    )
    .unwrap();
    let (_, private) = build_router_with_state(state.clone());
    let app = private.with_state(state);

    for (uri, expected) in [
        (format!("/api/v1/proxy/s/{slug}"), "/"),
        (format!("/api/v1/proxy/s/{slug}/"), "/"),
        (format!("/api/v1/proxy/s/{slug}/healthz"), "/healthz"),
        (format!("/api/v1/proxy/{}", catalog.id), "/"),
        (format!("/api/v1/proxy/{}/", catalog.id), "/"),
        (format!("/api/v1/proxy/{}/healthz", catalog.id), "/healthz"),
    ] {
        let (status, body) = forwarded_path(&app, &token, &uri).await;
        assert_eq!(status, StatusCode::OK, "{uri}: {body}");
        assert_eq!(body["path"], expected, "{uri} forwarded the wrong path");
    }

    // The bare `s` segment is still not a slug lookup.
    let (status, _) = forwarded_path(&app, &token, "/api/v1/proxy/s/").await;
    assert_ne!(status, StatusCode::OK);
    downstream.abort();
}

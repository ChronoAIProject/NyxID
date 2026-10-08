use super::curation_tests::{fixture, request};
use crate::models::{
    downstream_service::{COLLECTION_NAME as CATALOG, DownstreamService, OfferingKind},
    service_endpoint::{COLLECTION_NAME as ENDPOINTS, ServiceEndpoint},
};
use axum::http::StatusCode;
use mongodb::bson::doc;
use serde_json::json;

#[tokio::test]
async fn tool_publication_gates_all_discovery_and_proxy() {
    let mut f = fixture("tools_end_to_end", false).await;
    f.service.offering_kind = OfferingKind::Tool;
    f.service.service_category = "internal".into();
    f.service.auth_method = "none".into();
    f.service.requires_user_credential = false;
    f.service.proxy_operation_policy = None;
    f.service.visibility = "public".into();
    f.state
        .db
        .collection::<DownstreamService>(CATALOG)
        .replace_one(doc! {"_id":&f.service.id}, &f.service)
        .await
        .unwrap();
    let path = format!("/api/v1/services/{}/endpoints", f.service.id);
    let (status, endpoint) = request(
        &f.state,
        "POST",
        &path,
        &f.human_token,
        Some(json!({"name":"draft_operation","method":"GET","path":"/draft"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{endpoint}");
    assert_eq!(endpoint["publication"], "draft");
    assert_eq!(endpoint["is_active"], false);
    let mcp_session = f
        .state
        .mcp_sessions
        .create_with_proxy_access(&f.owner, true)
        .await
        .unwrap()
        .unwrap();
    for (method, path, body) in [
        ("GET", "/api/v1/mcp/config", None),
        (
            "POST",
            "/mcp",
            Some(json!({"jsonrpc":"2.0","id":1,"method":"tools/list"})),
        ),
        (
            "POST",
            "/mcp",
            Some(
                json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"nyx__search_tools","arguments":{"query":f.service.slug}}}),
            ),
        ),
        (
            "POST",
            "/mcp",
            Some(
                json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"nyx__discover_services","arguments":{}}}),
            ),
        ),
    ] {
        let (status, result) = if path == "/mcp" {
            mcp_request(&f.state, &f.human_token, &mcp_session, body.unwrap()).await
        } else {
            request(&f.state, method, path, &f.human_token, body).await
        };
        assert_eq!(status, StatusCode::OK, "{path}: {result}");
        assert!(!result.to_string().contains("draft_operation"), "{result}");
    }
    let (status, result) = request(
        &f.state,
        "GET",
        &format!("/api/v1/proxy/s/{}/draft", f.service.slug),
        &f.human_token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{result}");
    assert_eq!(result["error_code"], 12600, "{result}");
    let (_,result)=mcp_request(&f.state,&f.human_token,&mcp_session,json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"nyx__call_tool","arguments":{"tool_name":format!("{}__draft_operation",f.service.slug),"arguments_json":"{}"}}})).await;
    assert_eq!(result["result"]["isError"], true, "{result}");
    assert!(result.to_string().contains("12600"), "{result}");
    let (status, published) = request(
        &f.state,
        "POST",
        &format!("/api/v1/services/{}/publication", f.service.id),
        &f.human_token,
        Some(json!({"endpoint_names":["draft_operation"],"state":"published"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{published}");
    let rows = f
        .state
        .db
        .collection::<ServiceEndpoint>(ENDPOINTS)
        .find_one(doc! {"_id":endpoint["id"].as_str().unwrap()})
        .await
        .unwrap()
        .unwrap();
    assert!(rows.is_active);
    assert_eq!(rows.operation_generation, 2);
    f.state.db.drop().await.unwrap();
}

async fn mcp_request(
    state: &crate::AppState,
    bearer: &str,
    session: &str,
    body: serde_json::Value,
) -> (StatusCode, serde_json::Value) {
    use tower::ServiceExt;
    let req = axum::http::Request::builder()
        .method("POST")
        .uri("/mcp")
        .header("authorization", format!("Bearer {bearer}"))
        .header("mcp-session-id", session)
        .header("content-type", "application/json")
        .body(axum::body::Body::from(body.to_string()))
        .unwrap();
    let response = super::curation_tests::router(state)
        .oneshot(req)
        .await
        .unwrap();
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), 1_000_000)
        .await
        .unwrap();
    (status, serde_json::from_slice(&bytes).unwrap())
}

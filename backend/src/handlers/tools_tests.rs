use super::curation_tests::{fixture, request, token};
use crate::models::{
    downstream_service::{COLLECTION_NAME as CATALOG, DownstreamService, OfferingKind},
    service_account::COLLECTION_NAME as ACCOUNTS,
    service_endpoint::{COLLECTION_NAME as ENDPOINTS, ServiceEndpoint},
};
use axum::http::StatusCode;
use mongodb::bson::doc;
use serde_json::json;

#[tokio::test]
async fn tool_publication_gates_all_discovery_proxy_and_editor_fields() {
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
        let (status, result) = request(&f.state, method, path, &f.human_token, body).await;
        assert_eq!(status, StatusCode::OK, "{path}: {result}");
        assert!(!result.to_string().contains("draft_operation"), "{result}");
    }
    let (status, result) = request(
        &f.state,
        "GET",
        &format!("/api/v1/proxy/{}/draft", f.service.slug),
        &f.human_token,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{result}");
    assert_eq!(result["error_code"], 12600, "{result}");
    let (_,result)=request(&f.state,"POST","/mcp",&f.human_token,Some(json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"nyx__call_tool","arguments":{"tool_name":format!("{}__draft_operation",f.service.slug),"arguments":"{}"}}}))).await;
    assert_eq!(result["result"]["isError"], true, "{result}");
    assert!(result.to_string().contains("12600"), "{result}");
    let (status,role)=request(&f.state,"POST","/api/v1/admin/roles",&f.human_token,Some(json!({"name":"Tools editor","slug":"tools-editor","permissions":["nyxid:catalog:services:read","nyxid:catalog:services:write"]}))).await;
    assert_eq!(status, StatusCode::OK, "{role}");
    let (status,saved)=request(&f.state,"PUT",&format!("/api/v1/admin/service-accounts/{}",f.sa.id),&f.human_token,Some(json!({"allowed_scopes":"catalog:services:read catalog:services:write","role_ids":[role["id"]]}))).await;
    assert_eq!(status, StatusCode::OK, "{saved}");
    f.sa = f
        .state
        .db
        .collection(ACCOUNTS)
        .find_one(doc! {"_id":&f.sa.id})
        .await
        .unwrap()
        .unwrap();
    let bearer = token(&f, None).await;
    let (status, published) = request(
        &f.state,
        "POST",
        &format!("/api/v1/services/{}/publication", f.service.id),
        &bearer,
        Some(json!({"endpoint_names":["draft_operation"],"state":"published"})),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{published}");
    for forbidden in [
        json!({"credential":null}),
        json!({"billing":null}),
        json!({"base_url":"https://evil.invalid"}),
    ] {
        let (status, result) = request(
            &f.state,
            "PUT",
            &format!("/api/v1/services/{}", f.service.id),
            &bearer,
            Some(forbidden),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{result}");
    }
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

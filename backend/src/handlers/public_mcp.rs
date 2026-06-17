use axum::{
    body::{Body, to_bytes},
    extract::{ConnectInfo, State},
    http::{Request, Response, StatusCode, header},
    response::IntoResponse,
};
use serde::Deserialize;
use std::net::SocketAddr;

use crate::AppState;
use crate::errors::{AppError, AppResult};
use crate::services::mcp_service;

#[derive(Debug, Deserialize)]
struct JsonRpcRequest {
    id: Option<serde_json::Value>,
    method: String,
}

pub async fn public_mcp_post(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    request: Request<Body>,
) -> Response<Body> {
    match handle_public_mcp_post(state, Some(peer), request).await {
        Ok(response) => response,
        Err(AppError::RateLimited) => rpc_error(None, -32005, "Rate limit exceeded"),
        Err(error) => {
            tracing::warn!(%error, "Public MCP request failed");
            rpc_error(None, -32603, "Internal error")
        }
    }
}

async fn handle_public_mcp_post(
    state: AppState,
    peer: Option<SocketAddr>,
    request: Request<Body>,
) -> AppResult<Response<Body>> {
    let path = request.uri().path().to_string();
    crate::mw::rate_limit::enforce_public_ip_rate_limit(
        &state.public_mcp_limiter,
        request.headers(),
        peer,
        &state.config.trusted_proxy_ips,
        &path,
    )?;

    let body = to_bytes(request.into_body(), state.config.public_proxy_max_body_size)
        .await
        .map_err(|_| AppError::BadRequest("Public MCP request body is too large".to_string()))?;

    let parsed: JsonRpcRequest = match serde_json::from_slice(&body) {
        Ok(request) => request,
        Err(_) => return Ok(rpc_error(None, -32700, "Parse error")),
    };

    match parsed.method.as_str() {
        "initialize" => Ok(rpc_success(
            parsed.id,
            serde_json::json!({
                "protocolVersion": "2025-03-26",
                "capabilities": {
                    "tools": {
                        "listChanged": false
                    }
                },
                "serverInfo": {
                    "name": "nyxid-public-mcp",
                    "version": env!("CARGO_PKG_VERSION")
                }
            }),
        )),
        "notifications/initialized" => Ok(StatusCode::ACCEPTED.into_response()),
        "tools/list" => {
            let services = mcp_service::load_public_tools(&state.db).await?;
            let tools = mcp_service::generate_public_tool_definitions(&services)
                .into_iter()
                .map(|tool| {
                    serde_json::json!({
                        "name": tool.name,
                        "description": tool.description,
                        "inputSchema": tool.input_schema,
                    })
                })
                .collect::<Vec<_>>();
            Ok(rpc_success(
                parsed.id,
                serde_json::json!({ "tools": tools }),
            ))
        }
        "tools/call" => Ok(rpc_error(
            parsed.id,
            -32601,
            "Public MCP tool execution is not supported",
        )),
        _ => Ok(rpc_error(parsed.id, -32601, "Method not found")),
    }
}

fn rpc_success(id: Option<serde_json::Value>, result: serde_json::Value) -> Response<Body> {
    json_response(serde_json::json!({
        "jsonrpc": "2.0",
        "id": id.unwrap_or(serde_json::Value::Null),
        "result": result,
    }))
}

fn rpc_error(id: Option<serde_json::Value>, code: i64, message: &str) -> Response<Body> {
    json_response(serde_json::json!({
        "jsonrpc": "2.0",
        "id": id.unwrap_or(serde_json::Value::Null),
        "error": {
            "code": code,
            "message": message,
        }
    }))
}

fn json_response(value: serde_json::Value) -> Response<Body> {
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(value.to_string()))
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::downstream_service::{AnonymousEndpointRule, DownstreamService};
    use crate::test_utils::{connect_test_database, test_app_state};
    use axum::body::to_bytes;
    use chrono::Utc;
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};
    use uuid::Uuid;

    async fn json_body(response: Response<Body>) -> serde_json::Value {
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&body).unwrap()
    }

    fn peer() -> Option<SocketAddr> {
        Some(SocketAddr::new(
            IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)),
            40000,
        ))
    }

    /// Build a JSON-RPC POST request body for the public MCP endpoint.
    fn rpc_request(body: serde_json::Value) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri("/public/mcp")
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    }

    fn raw_request(raw: &str) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri("/public/mcp")
            .body(Body::from(raw.to_owned()))
            .unwrap()
    }

    fn safe_anonymous_service() -> DownstreamService {
        DownstreamService {
            id: Uuid::new_v4().to_string(),
            name: "Public Catalog".to_string(),
            slug: "public-catalog".to_string(),
            description: Some("Public docs".to_string()),
            base_url: "https://example.test".to_string(),
            service_type: "http".to_string(),
            visibility: "public".to_string(),
            auth_method: "none".to_string(),
            auth_key_name: String::new(),
            credential_encrypted: vec![],
            auth_type: None,
            openapi_spec_url: None,
            asyncapi_spec_url: None,
            streaming_supported: false,
            ssh_config: None,
            oauth_client_id: None,
            service_category: "internal".to_string(),
            requires_user_credential: false,
            is_active: true,
            created_by: "admin".to_string(),
            identity_propagation_mode: "none".to_string(),
            identity_include_user_id: false,
            identity_include_email: false,
            identity_include_name: false,
            identity_jwt_audience: None,
            forward_access_token: false,
            inject_delegation_token: false,
            delegation_token_scope: "llm:proxy".to_string(),
            provider_config_id: None,
            homepage_url: None,
            repository_url: None,
            issues_url: None,
            capabilities: None,
            auth_notes: None,
            known_limitations: None,
            required_permissions: None,
            examples_url: None,
            recommended_skills: None,
            custom_user_agent: None,
            default_request_headers: None,
            ws_frame_injections: Vec::new(),
            developer_app_ids: None,
            token_exchange_config: None,
            anonymous_endpoints: vec![AnonymousEndpointRule {
                id: Uuid::new_v4().to_string(),
                enabled: true,
                method: "GET".to_string(),
                path_pattern: "/public/**".to_string(),
                daily_quota: 100,
            }],
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[tokio::test]
    async fn initialize_returns_server_info() {
        let Some(db) = connect_test_database("pmcp_initialize").await else {
            return;
        };
        let state = test_app_state(db);
        let response = handle_public_mcp_post(
            state,
            peer(),
            rpc_request(serde_json::json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "initialize"
            })),
        )
        .await
        .expect("initialize handled");

        let body = json_body(response).await;
        assert_eq!(body["id"], 1);
        assert_eq!(body["result"]["serverInfo"]["name"], "nyxid-public-mcp");
        assert!(body["result"]["protocolVersion"].is_string());
    }

    #[tokio::test]
    async fn notifications_initialized_returns_accepted() {
        let Some(db) = connect_test_database("pmcp_notifications").await else {
            return;
        };
        let state = test_app_state(db);
        let response = handle_public_mcp_post(
            state,
            peer(),
            rpc_request(serde_json::json!({
                "jsonrpc": "2.0",
                "method": "notifications/initialized"
            })),
        )
        .await
        .expect("notification handled");

        assert_eq!(response.status(), StatusCode::ACCEPTED);
    }

    #[tokio::test]
    async fn parse_error_for_invalid_json() {
        let Some(db) = connect_test_database("pmcp_parse_error").await else {
            return;
        };
        let state = test_app_state(db);
        let response = handle_public_mcp_post(state, peer(), raw_request("{not json"))
            .await
            .expect("parse-error handled gracefully");

        let body = json_body(response).await;
        assert_eq!(body["error"]["code"], -32700);
        assert_eq!(body["id"], serde_json::Value::Null);
    }

    #[tokio::test]
    async fn tools_list_exposes_only_safe_enabled_public_tools() {
        let Some(db) = connect_test_database("pmcp_tools_list").await else {
            return;
        };

        // A safe service with an enabled anonymous endpoint -> should appear.
        let safe = safe_anonymous_service();
        // An identity-propagating service with an enabled rule -> filtered out.
        let mut unsafe_svc = safe_anonymous_service();
        unsafe_svc.id = Uuid::new_v4().to_string();
        unsafe_svc.slug = "identity-service".to_string();
        unsafe_svc.identity_propagation_mode = "headers".to_string();
        unsafe_svc.forward_access_token = true;
        // A safe service whose only rule is disabled -> omitted.
        let mut disabled_svc = safe_anonymous_service();
        disabled_svc.id = Uuid::new_v4().to_string();
        disabled_svc.slug = "disabled-service".to_string();
        disabled_svc.anonymous_endpoints[0].enabled = false;

        db.collection::<DownstreamService>(crate::models::downstream_service::COLLECTION_NAME)
            .insert_many([safe.clone(), unsafe_svc, disabled_svc])
            .await
            .expect("insert services");

        let state = test_app_state(db);
        let response = handle_public_mcp_post(
            state,
            peer(),
            rpc_request(serde_json::json!({
                "jsonrpc": "2.0",
                "id": 2,
                "method": "tools/list"
            })),
        )
        .await
        .expect("tools/list handled");

        let body = json_body(response).await;
        let tools = body["result"]["tools"].as_array().expect("tools array");
        assert_eq!(tools.len(), 1, "only the safe enabled service is exposed");
        let name = tools[0]["name"].as_str().expect("tool name");
        assert!(
            name.starts_with("public__public_catalog__"),
            "unexpected tool name: {name}"
        );
        assert!(tools[0]["inputSchema"].is_object());
    }

    #[tokio::test]
    async fn tools_call_is_rejected_via_handler() {
        let Some(db) = connect_test_database("pmcp_tools_call").await else {
            return;
        };
        let state = test_app_state(db);
        let response = handle_public_mcp_post(
            state,
            peer(),
            rpc_request(serde_json::json!({
                "jsonrpc": "2.0",
                "id": 3,
                "method": "tools/call",
                "params": { "name": "public__x", "arguments": {} }
            })),
        )
        .await
        .expect("tools/call handled");

        let body = json_body(response).await;
        assert_eq!(body["id"], 3);
        assert_eq!(body["error"]["code"], -32601);
    }

    #[tokio::test]
    async fn unknown_method_returns_method_not_found() {
        let Some(db) = connect_test_database("pmcp_unknown").await else {
            return;
        };
        let state = test_app_state(db);
        let response = handle_public_mcp_post(
            state,
            peer(),
            rpc_request(serde_json::json!({
                "jsonrpc": "2.0",
                "id": 4,
                "method": "resources/list"
            })),
        )
        .await
        .expect("unknown method handled");

        let body = json_body(response).await;
        assert_eq!(body["id"], 4);
        assert_eq!(body["error"]["code"], -32601);
        assert_eq!(body["error"]["message"], "Method not found");
    }

    #[tokio::test]
    async fn tools_call_is_rejected_on_public_mcp_projection() {
        let response = rpc_error(
            Some(serde_json::json!(1)),
            -32601,
            "Public MCP tool execution is not supported",
        );
        let body = json_body(response).await;
        assert_eq!(body["id"], 1);
        assert_eq!(body["error"]["code"], -32601);
    }
}

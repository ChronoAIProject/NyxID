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
    use axum::body::to_bytes;

    async fn json_body(response: Response<Body>) -> serde_json::Value {
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        serde_json::from_slice(&body).unwrap()
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

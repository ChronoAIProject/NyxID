use std::{collections::BTreeMap, sync::Arc};

use axum::{
    Router,
    body::Body,
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, Method, StatusCode, Uri, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{any, post},
};
use bytes::Bytes;
use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;
use zeroize::Zeroizing;

use crate::{Engine, Error, MAX_BODY_BYTES, Request, parse_json, parse_query};

#[derive(Clone)]
struct ServerState {
    engine: Arc<Engine>,
    key_hash: [u8; 32],
}

pub fn router(engine: Arc<Engine>, client_key: Zeroizing<String>) -> Result<Router, Error> {
    if client_key.len() < 32 || client_key.chars().any(char::is_control) {
        return Err(Error::Policy(
            "client key must contain at least 32 bytes and no control characters",
        ));
    }
    let state = ServerState {
        engine,
        key_hash: Sha256::digest(client_key.as_bytes()).into(),
    };
    Ok(Router::new()
        .route("/mcp", post(mcp))
        .route("/{*path}", any(rest))
        .layer(DefaultBodyLimit::max(MAX_BODY_BYTES))
        .layer(middleware::from_fn_with_state(state.clone(), authenticate))
        .with_state(state))
}

async fn authenticate(
    State(state): State<ServerState>,
    request: axum::extract::Request,
    next: Next,
) -> Response {
    let auth = request.headers().get_all(header::AUTHORIZATION);
    let mut values = auth.iter();
    let token = values
        .next()
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));
    if values.next().is_some()
        || token.is_none_or(|s| {
            let candidate: [u8; 32] = Sha256::digest(s.as_bytes()).into();
            !bool::from(candidate.ct_eq(&state.key_hash))
        })
    {
        return (
            StatusCode::UNAUTHORIZED,
            axum::Json(json!({"error":"invalid POC credential"})),
        )
            .into_response();
    }
    if request.headers().contains_key(header::ORIGIN)
        || request.headers().contains_key(header::CONTENT_ENCODING)
        || request.headers().contains_key(header::UPGRADE)
    {
        return error_response(Error::Denied(
            "browser origins, compressed requests and upgrades are unsupported",
        ));
    }
    next.run(request).await
}

fn error_status(error: &Error) -> StatusCode {
    match error {
        Error::Policy(_) => StatusCode::BAD_REQUEST,
        Error::Denied(_) => StatusCode::FORBIDDEN,
        Error::HookDenied { .. } => StatusCode::FORBIDDEN,
        Error::HookUnavailable { .. } => StatusCode::SERVICE_UNAVAILABLE,
        Error::Timeout => StatusCode::GATEWAY_TIMEOUT,
        Error::Busy => StatusCode::TOO_MANY_REQUESTS,
        _ => StatusCode::BAD_GATEWAY,
    }
}

fn error_response(error: Error) -> Response {
    (
        error_status(&error),
        axum::Json(json!({"error":error.to_string()})),
    )
        .into_response()
}

async fn execute(
    state: &ServerState,
    request: Request,
    channel: &str,
) -> Result<crate::Response, Error> {
    let result = state.engine.execute(request).await;
    eprintln!(
        "{}",
        json!({"event":"permission_poc_decision", "policy":state.engine.policy().name,
        "channel":channel, "status":result.as_ref().map(|r| r.status).unwrap_or_else(|e| error_status(e).as_u16()),
        "reason":result.as_ref().err().map(ToString::to_string)})
    );
    result
}

async fn rest(
    State(state): State<ServerState>,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let query = match parse_query(uri.query().unwrap_or("")) {
        Ok(q) => q,
        Err(e) => return error_response(e),
    };
    let request = Request {
        method,
        path: uri.path().into(),
        query,
        body,
        content_type: headers
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned),
    };
    match execute(&state, request, "rest").await {
        Ok(response) => {
            let mut result = (
                StatusCode::from_u16(response.status).unwrap_or(StatusCode::BAD_GATEWAY),
                Body::from(response.body),
            )
                .into_response();
            result.headers_mut().insert(
                header::CONTENT_TYPE,
                response
                    .content_type
                    .parse()
                    .unwrap_or(header::HeaderValue::from_static("application/octet-stream")),
            );
            result.headers_mut().insert(
                "x-content-type-options",
                header::HeaderValue::from_static("nosniff"),
            );
            result.headers_mut().insert(
                header::CACHE_CONTROL,
                header::HeaderValue::from_static("no-store"),
            );
            if response.content_type == "application/octet-stream" {
                result.headers_mut().insert(
                    header::CONTENT_DISPOSITION,
                    header::HeaderValue::from_static("attachment"),
                );
            }
            result
        }
        Err(error) => error_response(error),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ToolRequest {
    method: String,
    path: String,
    #[serde(default)]
    query: BTreeMap<String, String>,
    #[serde(default, deserialize_with = "present_body")]
    body: Option<Value>,
}

fn present_body<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<Value>, D::Error> {
    Value::deserialize(deserializer).map(Some)
}

async fn mcp(State(state): State<ServerState>, body: Bytes) -> Response {
    let response = handle_mcp(&state.engine, body).await;
    eprintln!(
        "{}",
        json!({"event":"permission_poc_decision", "policy": state.engine.policy().name,
        "channel":"mcp", "status":response.status().as_u16(),
        "tool_error":response.extensions().get::<McpToolOutcome>().map(|v| v.is_error)})
    );
    response
}

#[derive(Clone, Copy)]
pub struct McpToolOutcome {
    pub is_error: bool,
}

/// Stateless MCP dispatch for hosts that already authenticate and bind an engine.
/// The host must bound the body and reject unsupported encodings and upgrades.
pub async fn handle_mcp(engine: &Engine, body: Bytes) -> Response {
    let message = match parse_json(&body) {
        Ok(v) if v.is_object() => v,
        _ => return rpc_error(Value::Null, -32700, "invalid JSON-RPC message"),
    };
    let id = message.get("id").cloned();
    if message["jsonrpc"] != "2.0"
        || id
            .as_ref()
            .is_some_and(|id| !(id.is_string() || id.is_number()))
    {
        return rpc_error(Value::Null, -32600, "invalid JSON-RPC envelope");
    }
    let method = message["method"].as_str().unwrap_or("");
    if id.is_none() {
        return if method == "notifications/initialized" {
            StatusCode::ACCEPTED.into_response()
        } else {
            rpc_error(Value::Null, -32600, "requests require an ID")
        };
    }
    let id = id.unwrap();
    let google_api = engine.policy().provider == "google";
    let tool_name = if google_api {
        "google_request"
    } else {
        "drive_request"
    };
    let result = match method {
        "initialize" => {
            json!({"protocolVersion":"2025-03-26", "capabilities":{"tools":{}}, "serverInfo":{"name":"nyxid-permissions","version":"0.1.0"}})
        }
        "ping" => json!({}),
        "tools/list" => {
            let description = if google_api {
                // Operation definitions contain policy rules, never credentials.
                format!(
                    "Google API permission rules: {}",
                    serde_json::to_string(&engine.policy().resource).expect("policy serialization")
                )
            } else {
                format!(
                    "Google Drive permission POC. Allowed operations: {}. List q accepts only 'FOLDER_ID' in parents. Metadata is projected to id/name/mimeType. Binary downloads use REST.",
                    engine.policy().allowed_operations.join(", ")
                )
            };
            json!({"tools":[{"name":tool_name, "description":description,
            "inputSchema":{"type":"object", "properties":{"method":{"type":"string","enum":["GET","HEAD","POST","PUT","PATCH","DELETE"]},"path":{"type":"string"},"query":{"type":"object","additionalProperties":{"type":"string"}},"body":{}},"required":["method","path"],"additionalProperties":false},
            "annotations":{"readOnlyHint":false,"destructiveHint":true,"openWorldHint":true}}]})
        }
        "tools/call" => {
            if message["params"]["name"] != tool_name {
                return rpc_error(id, -32602, "unknown tool");
            }
            let args: ToolRequest =
                match serde_json::from_value(message["params"]["arguments"].clone()) {
                    Ok(args) => args,
                    Err(_) => return rpc_error(id, -32602, "invalid tool arguments"),
                };
            let method = match Method::from_bytes(args.method.as_bytes()) {
                Ok(m) => m,
                Err(_) => return rpc_error(id, -32602, "invalid HTTP method"),
            };
            if args.path.ends_with("/export") || args.query.get("alt").is_some_and(|v| v == "media")
            {
                return rpc_error(id, -32602, "use REST for binary downloads");
            }
            let request = Request {
                method,
                path: args.path,
                query: args.query,
                body: args
                    .body
                    .map(|v| {
                        serde_json::to_vec(&v)
                            .expect("JSON Value serialization")
                            .into()
                    })
                    .unwrap_or_default(),
                content_type: Some("application/json".into()),
            };
            match engine.execute(request).await {
                Ok(response) => {
                    json!({"content":[{"type":"text","text":String::from_utf8_lossy(&response.body)}],"isError":false})
                }
                Err(error) => {
                    json!({"content":[{"type":"text","text":error.to_string()}],"isError":true})
                }
            }
        }
        _ => return rpc_error(id, -32601, "method not found"),
    };
    let outcome = (method == "tools/call").then(|| McpToolOutcome {
        is_error: result["isError"].as_bool().unwrap_or(true),
    });
    let mut response =
        axum::Json(json!({"jsonrpc":"2.0", "id":id, "result":result})).into_response();
    if let Some(outcome) = outcome {
        response.extensions_mut().insert(outcome);
    }
    response
}

fn rpc_error(id: Value, code: i32, message: &str) -> Response {
    axum::Json(json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}}))
        .into_response()
}

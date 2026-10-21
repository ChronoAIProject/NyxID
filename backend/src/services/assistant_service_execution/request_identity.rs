use super::*;
use sha2::{Digest, Sha256};

fn body_digest(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// The legacy model transport scope exception includes ordinary generation and
/// the runtime's exact compaction callback. It grants no account fungibility or
/// turn consent, and never applies to arbitrary response/file/session routes.
pub(super) fn legacy_model_generation(
    target: &crate::services::proxy_service::ProxyTarget,
    method: &str,
    path: &str,
    body: &[u8],
) -> bool {
    let path = path.trim_matches('/');
    let path = path.strip_prefix("v1/").unwrap_or(path);
    target.service.inference.is_some()
        && method == "POST"
        && matches!(
            path,
            "responses" | "responses/compact" | "chat/completions" | "messages" | "embeddings"
        )
        && serde_json::from_slice::<Value>(body)
            .ok()
            .is_some_and(|body| {
                body.get("model")
                    .and_then(Value::as_str)
                    .is_some_and(|model| !model.trim().is_empty())
            })
}

/// Only catalog-declared inference on a closed model generation endpoint is
/// account-fungible. Stateful response/file handles retain account identity.
pub fn http_operation(
    target: &crate::services::proxy_service::ProxyTarget,
    method: &str,
    path: &str,
    query: Option<&str>,
    body: &[u8],
) -> Value {
    let parsed = if target.service.inference.is_some() && method == "POST" {
        serde_json::from_slice::<Value>(body).unwrap_or(Value::Null)
    } else {
        Value::Null
    };
    let inference = target.service.inference.is_some()
        && method == "POST"
        && stateless_inference(path, &parsed);
    if inference {
        // MCP actions keep their entire semantic payload. Raw stateless model
        // consent is separately bounded to a server-admitted turn below.
        json!({"kind": "inference", "method": method, "path": path, "query": query,
            "model": parsed.get("model"), "provider": target.service.id, "body": body_digest(body)})
    } else {
        json!({"kind": "http", "method": method, "path": path, "query": query, "body": body_digest(body)})
    }
}
/// Recognize only supported, account-independent generation requests. Unknown
/// provider extensions retain exact action consent. Function tools describe
/// callbacks to NyxAgent; provider-hosted tools can execute effects themselves.
pub(crate) fn stateless_inference(path: &str, body: &Value) -> bool {
    let path = path
        .trim_matches('/')
        .strip_prefix("v1/")
        .unwrap_or(path.trim_matches('/'));
    if !matches!(
        path,
        "chat/completions" | "responses" | "messages" | "embeddings"
    ) {
        return false;
    }
    let Some(object) = body.as_object() else {
        return false;
    };
    if object
        .get("model")
        .and_then(Value::as_str)
        .is_none_or(|model| model.trim().is_empty())
    {
        return false;
    }
    if path == "responses" && object.get("store") != Some(&Value::Bool(false)) {
        return false;
    }
    const FIELDS: &[&str] = &[
        "model",
        "messages",
        "input",
        "instructions",
        "system",
        "stream",
        "stream_options",
        "max_tokens",
        "max_completion_tokens",
        "max_output_tokens",
        "temperature",
        "top_p",
        "top_k",
        "frequency_penalty",
        "presence_penalty",
        "stop",
        "stop_sequences",
        "seed",
        "n",
        "tools",
        "tool_choice",
        "parallel_tool_calls",
        "response_format",
        "text",
        "reasoning",
        "reasoning_effort",
        "thinking",
        "store",
        "background",
        "metadata",
        "user",
        "service_tier",
        "include",
        "truncation",
        "encoding_format",
        "dimensions",
        "logprobs",
        "top_logprobs",
        "prediction",
        "verbosity",
        "safety_identifier",
        "prompt_cache_key",
        "prompt_cache_retention",
    ];
    if object.keys().any(|key| !FIELDS.contains(&key.as_str()))
        || object
            .get("store")
            .is_some_and(|value| value != &Value::Bool(false))
        || object
            .get("background")
            .is_some_and(|value| value != &Value::Bool(false))
    {
        return false;
    }
    fn handles(value: &Value) -> bool {
        match value {
            Value::Object(map) => map.iter().any(|(key, value)| {
                matches!(
                    key.as_str(),
                    "file_id"
                        | "previous_response_id"
                        | "conversation"
                        | "assistant_id"
                        | "thread_id"
                        | "vector_store_ids"
                        | "container"
                        | "prompt"
                        | "mcp_servers"
                        | "server_url"
                        | "server_label"
                        | "connector_id"
                ) || handles(value)
            }),
            Value::Array(items) => items.iter().any(handles),
            _ => false,
        }
    }
    let mut executable = body.clone();
    executable
        .as_object_mut()
        .expect("model request")
        .remove("tools");
    if handles(&executable) {
        return false;
    }
    if let Some(tools) = object.get("tools") {
        let Some(tools) = tools.as_array() else {
            return false;
        };
        if tools.iter().any(|tool| {
            !tool.is_object()
                || if path == "messages" {
                    tool.get("type").is_some()
                        || tool.get("name").and_then(Value::as_str).is_none()
                        || tool
                            .get("input_schema")
                            .and_then(Value::as_object)
                            .is_none()
                } else {
                    !matches!(
                        tool.get("type").and_then(Value::as_str),
                        Some("function" | "web_search")
                    )
                }
        }) {
            return false;
        }
    }
    true
}

/// Bind sanitized semantic headers. Correlation metadata can change at each
/// transport hop and carries no account/action authority. Account/project and
/// idempotency values remain bound. Cards expose only this private digest.
pub fn bind_headers(operation: &mut Value, headers: &[(String, String)]) {
    let mut semantic: Vec<_> = headers
        .iter()
        .filter(|(name, _)| {
            !matches!(
                name.to_ascii_lowercase().as_str(),
                ACK_HEADER
                    | "traceparent"
                    | "tracestate"
                    | "baggage"
                    | "x-request-id"
                    | "x-correlation-id"
            )
        })
        .map(|(name, value)| (name.to_ascii_lowercase(), value.clone()))
        .collect();
    semantic.sort_by(|a, b| a.0.cmp(&b.0));
    operation["headers"] = json!(digest(json!(semantic)));
}

/// Raw model payloads vary within a turn; fallback still preserves exact
/// provider/model/endpoint/query/semantic headers and the bucket's turn fence.
pub(super) fn selection_fingerprint(operation: &Value) -> String {
    let mut identity = operation.clone();
    if identity["kind"] == "raw_inference" {
        identity
            .as_object_mut()
            .expect("operation")
            .remove("input_admission");
    }
    digest(identity)
}

pub(super) fn raw_identity(
    target: &crate::services::proxy_service::ProxyTarget,
    method: &str,
    path: &str,
    query: Option<&str>,
    body: &[u8],
    headers: &axum::http::HeaderMap,
    projection: &HttpProjection,
) -> Option<Value> {
    let mut operation = http_operation(target, method, path, query, body);
    if operation["kind"] != "inference"
        || mcp_service::http_carries_method_override(method, headers, query, body)
    {
        return None;
    }
    operation["kind"] = json!("raw_inference");
    operation.as_object_mut().expect("operation").remove("body");
    let mut forwarded = projection.headers(target, headers);
    forwarded.retain(|(name, _)| {
        !matches!(
            name.to_ascii_lowercase().as_str(),
            "traceparent" | "tracestate" | "baggage" | "x-request-id" | "x-correlation-id"
        )
    });
    bind_headers(&mut operation, &forwarded);
    Some(operation)
}

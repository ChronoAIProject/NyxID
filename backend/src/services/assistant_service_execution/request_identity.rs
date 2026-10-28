use super::*;
use sha2::{Digest, Sha256};

fn body_digest(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// Codex correlation metadata is inert text, never provider resource identity.
/// Keep this contract closed and measure the sorted compact UTF-8 encoding,
/// including JSON escaping, independently of serde_json's map feature flags.
fn valid_client_metadata(value: &Value) -> bool {
    fn ascii_id(id: &str) -> bool {
        !id.is_empty()
            && id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    }
    const IDS: &[&str] = &[
        "root_turn_id",
        "session_id",
        "thread_id",
        "turn_id",
        "x-codex-installation-id",
        "x-codex-window-id",
    ];
    let Some(object) = value.as_object() else {
        return false;
    };
    if object.len() != IDS.len() + 1
        || !IDS.iter().all(|key| {
            object.get(*key).and_then(Value::as_str).is_some_and(|id| {
                (1..=128).contains(&id.len())
                    && (ascii_id(id)
                        || (*key == "x-codex-window-id"
                            && id.split_once(':').is_some_and(|(id, index)| {
                                ascii_id(id)
                                    && !index.is_empty()
                                    && index.bytes().all(|byte| byte.is_ascii_digit())
                            })))
            })
        })
        || object
            .get("x-codex-turn-metadata")
            .and_then(Value::as_str)
            .is_none_or(|metadata| metadata.len() > 1024)
    {
        return false;
    }
    let sorted: std::collections::BTreeMap<_, _> = object.iter().collect();
    serde_json::to_vec(&sorted).is_ok_and(|bytes| bytes.len() <= 2048)
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
    // Only the bounded seven-field Codex metadata contract shares owner inference consent.
    // Wider transport telemetry keeps exact owner approval and account identity.
    body.get("client_metadata")
        .is_none_or(valid_client_metadata)
        && stateless_model_transport(path, body)
}

/// Presentation only. Never include this diagnosis in an operation identity,
/// consent digest or dispatch key. Messages contain no caller-supplied values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum InferenceFallbackReason {
    MetadataShape,
    MetadataUnknownField,
    MetadataIdentifier,
    MetadataTextLimit,
    MetadataObjectLimit,
    CompactProtocolUnverified,
    ProviderStateOrEffect,
    UnsupportedGeneration,
}

impl InferenceFallbackReason {
    pub(crate) fn code(self) -> &'static str {
        match self {
            Self::MetadataShape => "metadata_shape",
            Self::MetadataUnknownField => "metadata_unknown_field",
            Self::MetadataIdentifier => "metadata_identifier",
            Self::MetadataTextLimit => "metadata_text_limit",
            Self::MetadataObjectLimit => "metadata_object_limit",
            Self::CompactProtocolUnverified => "compact_protocol_unverified",
            Self::ProviderStateOrEffect => "provider_state_or_effect",
            Self::UnsupportedGeneration => "unsupported_generation",
        }
    }
    pub(crate) fn message(self) -> &'static str {
        match self {
            Self::MetadataShape => {
                "Automatic model Read requires the complete seven-field metadata contract. Review this exact request or use a supported client shape."
            }
            Self::MetadataUnknownField => {
                "Unrecognized metadata has no reviewed inert meaning. Approval covers this exact request only."
            }
            Self::MetadataIdentifier => {
                "A correlation identifier is outside the bounded client grammar. Use a supported client shape or review this exact request."
            }
            Self::MetadataTextLimit => {
                "Automatic model Read allows at most 1024 UTF-8 bytes of turn metadata. Approval covers this exact request only."
            }
            Self::MetadataObjectLimit => {
                "Automatic model Read allows at most 2048 bytes of compact canonical client metadata. Approval covers this exact request only."
            }
            Self::CompactProtocolUnverified => {
                "Compaction has no reviewed reusable protocol. Approval covers this exact body; changed compaction requires separate review."
            }
            Self::ProviderStateOrEffect => {
                "Provider state or executable configuration is outside automatic model Read. Review the exact request and its effects."
            }
            Self::UnsupportedGeneration => {
                "This generation shape has no reviewed automatic model contract. Review this exact request or use a supported shape."
            }
        }
    }
}

pub(super) async fn record_fallback_reason(
    db: &Database,
    chat: &acks::ChatAuthority,
    operation: &Value,
    reason: InferenceFallbackReason,
) -> AppResult<()> {
    // Retain an actionable diagnosis even when an existing policy permits the
    // exact request without a card. Only the operation digest and static text
    // are stored; none of the request's raw metadata or body is copied here.
    db.collection::<Document>(COLLECTION)
        .update_one(
            doc! {"kind":"model_fallback_diagnostic", "user_id":&chat.user_id,
            "conversation_id":&chat.conversation_id, "api_key_id":&chat.api_key_id,
            "turn_id":&chat.turn_id, "operation":digest(operation.clone())},
            doc! {"$setOnInsert": {"_id":uuid::Uuid::new_v4().to_string(),
            "reason":reason.code(), "explanation":reason.message(),
            "expires_at":bson::DateTime::from_chrono(Utc::now()+Duration::minutes(35))}},
        )
        .upsert(true)
        .await?;
    Ok(())
}

pub(crate) fn inference_fallback_reason(
    path: &str,
    body: &Value,
) -> Option<InferenceFallbackReason> {
    use InferenceFallbackReason as Reason;
    if stateless_inference(path, body) {
        return None;
    }
    let path = path.trim_matches('/');
    let path = path.strip_prefix("v1/").unwrap_or(path);
    if !matches!(
        path,
        "responses" | "responses/compact" | "chat/completions" | "messages" | "embeddings"
    ) {
        return Some(Reason::UnsupportedGeneration);
    }
    let mut without_metadata = body.clone();
    let Some(object) = without_metadata.as_object_mut() else {
        return Some(Reason::UnsupportedGeneration);
    };
    object.remove("client_metadata");
    // Diagnose executable/stateful changes first. Oversized telemetry must not
    // disguise an unrelated effect or provider resource as a metadata problem.
    let transport_path = if path == "responses/compact" {
        "responses"
    } else {
        path
    };
    if !stateless_model_transport(transport_path, &without_metadata) {
        return Some(Reason::ProviderStateOrEffect);
    }
    if path == "responses/compact" {
        return Some(Reason::CompactProtocolUnverified);
    }
    let Some(metadata) = body.get("client_metadata").and_then(Value::as_object) else {
        return Some(Reason::MetadataShape);
    };
    const KEYS: &[&str] = &[
        "root_turn_id",
        "session_id",
        "thread_id",
        "turn_id",
        "x-codex-installation-id",
        "x-codex-window-id",
        "x-codex-turn-metadata",
    ];
    if metadata.keys().any(|key| !KEYS.contains(&key.as_str())) {
        return Some(Reason::MetadataUnknownField);
    }
    if metadata.len() != KEYS.len() || metadata.values().any(|value| !value.is_string()) {
        return Some(Reason::MetadataShape);
    }
    if metadata["x-codex-turn-metadata"]
        .as_str()
        .is_some_and(|text| text.len() > 1024)
    {
        return Some(Reason::MetadataTextLimit);
    }
    // The existing strict validator remains the sole grammar authority. A
    // failed object under its size ceiling can only be an identifier failure.
    let sorted: std::collections::BTreeMap<_, _> = metadata.iter().collect();
    if serde_json::to_vec(&sorted).is_ok_and(|bytes| bytes.len() > 2048) {
        return Some(Reason::MetadataObjectLimit);
    }
    Some(Reason::MetadataIdentifier)
}

/// Closed generation contract used only for guest model transport and recording
/// an already-admitted owner route. It never selects an owner's admission path.
pub(crate) fn stateless_model_transport(path: &str, body: &Value) -> bool {
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
        "client_metadata",
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
    // Codex correlation fields are client telemetry, not stored provider
    // resource handles. NyxAgent forwards this object on ordinary model calls.
    // Keep the extension closed and bounded; unknown/nested values still deny.
    if let Some(metadata) = object.get("client_metadata") {
        const CORRELATION: &[&str] = &[
            "root_turn_id",
            "session_id",
            "thread_id",
            "turn_id",
            "x-codex-installation-id",
            "x-codex-turn-metadata",
            "x-codex-window-id",
        ];
        if path != "responses"
            || metadata.as_object().is_none_or(|fields| {
                fields.len() > CORRELATION.len()
                    || fields.iter().any(|(key, value)| {
                        !CORRELATION.contains(&key.as_str())
                            || value.as_str().is_none_or(|text| text.len() > 8192)
                    })
            })
        {
            return false;
        }
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
    executable
        .as_object_mut()
        .expect("model request")
        .remove("client_metadata");
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

pub(super) fn model_route_operation(
    target: &crate::services::proxy_service::ProxyTarget,
    method: &str,
    path: &str,
    query: Option<&str>,
    body: &[u8],
) -> Option<Value> {
    if target.service.inference.is_none() || method != "POST" {
        return None;
    }
    let parsed = serde_json::from_slice::<Value>(body).ok()?;
    stateless_model_transport(path, &parsed).then(|| {
        json!({"kind":"raw_inference", "method":method, "path":path, "query":query,
            "model":parsed["model"], "provider":target.service.id})
    })
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

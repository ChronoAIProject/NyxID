//! Translator between OpenAI Chat Completions format and the Responses API
//! format used by `chatgpt.com/backend-api/codex`.
//!
//! The `openai-codex` device code flow produces OIDC tokens that are only
//! valid at the ChatGPT backend (not `api.openai.com`). The ChatGPT backend
//! speaks the Responses API wire format, so this translator bridges the gap.
//!
//! Supports both directions:
//! - Chat Completions request (has `messages`) → translated to Responses API
//! - Responses API request (has `input`) → passed through with minimal enrichment

use tracing;

use crate::errors::{AppError, AppResult};
use crate::services::llm_gateway_service::{
    LlmTranslator, SseEvent, StreamTranslationState, TranslatedRequest,
};

/// Returns `true` if the body is in Chat Completions format (has `messages`).
/// Returns `false` if already in Responses API format (has `input`).
pub fn is_chat_completions_format(body: &serde_json::Value) -> bool {
    body.get("messages").is_some()
}

pub struct ChatgptTranslator;

impl ChatgptTranslator {
    /// Translate a Chat Completions request (has `messages`) to Responses API format.
    fn translate_chat_completions_request(
        &self,
        path: &str,
        body: &serde_json::Value,
    ) -> AppResult<TranslatedRequest> {
        let mut translated = serde_json::Map::new();

        // Passthrough fields
        for key in &[
            "model",
            "temperature",
            "top_p",
            "stream",
            "tools",
            "tool_choice",
        ] {
            if let Some(val) = body.get(*key) {
                translated.insert(key.to_string(), val.clone());
            }
        }

        // Convert messages -> instructions + input
        if let Some(messages) = body.get("messages").and_then(|m| m.as_array()) {
            let (instructions, input) = convert_messages_to_input(messages);
            translated.insert(
                "instructions".to_string(),
                serde_json::Value::String(instructions.unwrap_or_default()),
            );
            translated.insert("input".to_string(), serde_json::Value::Array(input));
        }

        // Do not store responses in the user's ChatGPT history
        translated.insert("store".to_string(), serde_json::Value::Bool(false));

        // Codex backend requires streaming
        translated.insert("stream".to_string(), serde_json::Value::Bool(true));

        // Path: chat/completions -> responses
        let translated_path = path.replace("chat/completions", "responses");

        let extra_headers = vec![];

        Ok(TranslatedRequest {
            path: translated_path,
            body: serde_json::Value::Object(translated),
            extra_headers,
        })
    }

    /// Enrich a Responses API request (has `input`) with defaults, pass through as-is.
    fn enrich_responses_api_request(
        &self,
        path: &str,
        body: &serde_json::Value,
    ) -> AppResult<TranslatedRequest> {
        let mut enriched = body.as_object().cloned().unwrap_or_default();

        // Ensure store=false so responses don't pollute ChatGPT history
        enriched
            .entry("store".to_string())
            .or_insert(serde_json::Value::Bool(false));

        // Strip token limit params not supported by the Codex backend
        enriched.remove("max_tokens");
        enriched.remove("max_output_tokens");
        enriched.remove("max_completion_tokens");

        // Codex backend requires instructions even if empty
        enriched
            .entry("instructions".to_string())
            .or_insert(serde_json::Value::String(String::new()));

        // Codex backend requires streaming
        enriched.insert("stream".to_string(), serde_json::Value::Bool(true));

        Ok(TranslatedRequest {
            path: path.to_string(),
            body: serde_json::Value::Object(enriched),
            extra_headers: vec![],
        })
    }
}

impl LlmTranslator for ChatgptTranslator {
    fn needs_translation(&self) -> bool {
        true
    }

    fn gateway_base_url(&self) -> Option<&str> {
        Some("https://chatgpt.com/backend-api/codex")
    }

    fn translate_request(
        &self,
        path: &str,
        body: &serde_json::Value,
    ) -> AppResult<TranslatedRequest> {
        if is_chat_completions_format(body) {
            self.translate_chat_completions_request(path, body)
        } else {
            // Already Responses API format -- pass through with enrichment
            self.enrich_responses_api_request(path, body)
        }
    }

    fn translate_response(&self, body: serde_json::Value) -> AppResult<serde_json::Value> {
        let output = body
            .get("output")
            .and_then(|o| o.as_array())
            .cloned()
            .unwrap_or_default();

        let mut text_parts = Vec::new();
        let mut tool_calls = Vec::new();

        for item in &output {
            let item_type = item.get("type").and_then(|t| t.as_str()).unwrap_or("");
            match item_type {
                "message" => {
                    if let Some(content_arr) = item.get("content").and_then(|c| c.as_array()) {
                        for block in content_arr {
                            if block.get("type").and_then(|t| t.as_str()) == Some("output_text") {
                                if let Some(text) = block.get("text").and_then(|t| t.as_str()) {
                                    text_parts.push(text.to_string());
                                }
                            }
                        }
                    }
                }
                "function_call" => {
                    let id = item.get("id").and_then(|v| v.as_str()).unwrap_or("unknown");
                    let name = item
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown");
                    let arguments = item
                        .get("arguments")
                        .and_then(|v| v.as_str())
                        .unwrap_or("{}");
                    tool_calls.push(serde_json::json!({
                        "id": id,
                        "type": "function",
                        "function": {
                            "name": name,
                            "arguments": arguments,
                        }
                    }));
                }
                _ => {}
            }
        }

        let content_text = text_parts.join("");

        let status = body
            .get("status")
            .and_then(|s| s.as_str())
            .unwrap_or("completed");

        let finish_reason = if !tool_calls.is_empty() {
            "tool_calls"
        } else {
            match status {
                "completed" => "stop",
                "incomplete" => "length",
                _ => "stop",
            }
        };

        let id = body.get("id").and_then(|v| v.as_str()).unwrap_or("unknown");
        let model = body
            .get("model")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
        let created = body
            .get("created_at")
            .and_then(|v| v.as_i64())
            .unwrap_or_else(|| chrono::Utc::now().timestamp());

        let input_tokens = body
            .pointer("/usage/input_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let output_tokens = body
            .pointer("/usage/output_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);

        let mut message = serde_json::json!({
            "role": "assistant",
            "content": if content_text.is_empty() {
                serde_json::Value::Null
            } else {
                serde_json::Value::String(content_text)
            },
        });
        if !tool_calls.is_empty() {
            message["tool_calls"] = serde_json::Value::Array(tool_calls);
        }

        Ok(serde_json::json!({
            "id": format!("chatcmpl-{id}"),
            "object": "chat.completion",
            "created": created,
            "model": model,
            "choices": [{
                "index": 0,
                "message": message,
                "finish_reason": finish_reason,
            }],
            "usage": {
                "prompt_tokens": input_tokens,
                "completion_tokens": output_tokens,
                "total_tokens": input_tokens + output_tokens,
            },
        }))
    }

    fn translate_stream_event(
        &self,
        event: &SseEvent,
        state: &mut StreamTranslationState,
    ) -> Option<String> {
        let data: serde_json::Value = serde_json::from_str(&event.data).ok()?;

        // Use event: header if present, fall back to type field in data
        let event_type = event
            .event_type
            .as_deref()
            .or_else(|| data.get("type").and_then(|t| t.as_str()))?;

        match event_type {
            "response.created" => {
                let response = data.get("response")?;
                state.id = response
                    .get("id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_string();
                state.model = response
                    .get("model")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown")
                    .to_string();
                if let Some(ts) = response.get("created_at").and_then(|v| v.as_i64()) {
                    state.created = ts;
                }

                let chunk = serde_json::json!({
                    "id": format!("chatcmpl-{}", state.id),
                    "object": "chat.completion.chunk",
                    "created": state.created,
                    "model": &state.model,
                    "choices": [{
                        "index": 0,
                        "delta": { "role": "assistant", "content": "" },
                        "finish_reason": serde_json::Value::Null,
                    }]
                });
                Some(format!("data: {}\n\n", chunk))
            }

            "response.output_item.added" => {
                let item = data.get("item")?;
                let item_type = item.get("type").and_then(|t| t.as_str())?;

                if item_type == "function_call" {
                    let tool_index = state.next_tool_index;
                    state.next_tool_index += 1;

                    let output_index = data
                        .get("output_index")
                        .and_then(|v| v.as_u64())
                        .unwrap_or(0) as usize;
                    state.tool_call_indices.push((output_index, tool_index));

                    let tool_id = item.get("id").and_then(|v| v.as_str()).unwrap_or("unknown");
                    let tool_name = item
                        .get("name")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown");

                    let chunk = serde_json::json!({
                        "id": format!("chatcmpl-{}", state.id),
                        "object": "chat.completion.chunk",
                        "created": state.created,
                        "model": &state.model,
                        "choices": [{
                            "index": 0,
                            "delta": {
                                "tool_calls": [{
                                    "index": tool_index,
                                    "id": tool_id,
                                    "type": "function",
                                    "function": {
                                        "name": tool_name,
                                        "arguments": "",
                                    }
                                }]
                            },
                            "finish_reason": serde_json::Value::Null,
                        }]
                    });
                    Some(format!("data: {}\n\n", chunk))
                } else {
                    None
                }
            }

            "response.output_text.delta" => {
                let delta = data.get("delta").and_then(|d| d.as_str()).unwrap_or("");

                let chunk = serde_json::json!({
                    "id": format!("chatcmpl-{}", state.id),
                    "object": "chat.completion.chunk",
                    "created": state.created,
                    "model": &state.model,
                    "choices": [{
                        "index": 0,
                        "delta": { "content": delta },
                        "finish_reason": serde_json::Value::Null,
                    }]
                });
                Some(format!("data: {}\n\n", chunk))
            }

            "response.function_call_arguments.delta" => {
                let delta = data.get("delta").and_then(|d| d.as_str()).unwrap_or("");
                let output_index = data
                    .get("output_index")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0) as usize;

                let tool_index = state
                    .tool_call_indices
                    .iter()
                    .find(|(oi, _)| *oi == output_index)
                    .map(|(_, ti)| *ti)
                    .unwrap_or(0);

                let chunk = serde_json::json!({
                    "id": format!("chatcmpl-{}", state.id),
                    "object": "chat.completion.chunk",
                    "created": state.created,
                    "model": &state.model,
                    "choices": [{
                        "index": 0,
                        "delta": {
                            "tool_calls": [{
                                "index": tool_index,
                                "function": {
                                    "arguments": delta,
                                }
                            }]
                        },
                        "finish_reason": serde_json::Value::Null,
                    }]
                });
                Some(format!("data: {}\n\n", chunk))
            }

            "response.completed" => {
                let response = data.get("response")?;
                let input_tokens = response
                    .pointer("/usage/input_tokens")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);
                let output_tokens = response
                    .pointer("/usage/output_tokens")
                    .and_then(|v| v.as_u64())
                    .unwrap_or(0);

                let finish_reason = if state.next_tool_index > 0 {
                    "tool_calls"
                } else {
                    "stop"
                };

                // OpenAI spec: finish_reason in one chunk, usage in a
                // separate chunk with empty choices[], then [DONE].
                let finish_chunk = serde_json::json!({
                    "id": format!("chatcmpl-{}", state.id),
                    "object": "chat.completion.chunk",
                    "created": state.created,
                    "model": &state.model,
                    "choices": [{
                        "index": 0,
                        "delta": {},
                        "finish_reason": finish_reason,
                    }]
                });
                let usage_chunk = serde_json::json!({
                    "id": format!("chatcmpl-{}", state.id),
                    "object": "chat.completion.chunk",
                    "created": state.created,
                    "model": &state.model,
                    "choices": [],
                    "usage": {
                        "prompt_tokens": input_tokens,
                        "completion_tokens": output_tokens,
                        "total_tokens": input_tokens + output_tokens,
                    }
                });
                Some(format!(
                    "data: {}\n\ndata: {}\n\ndata: [DONE]\n\n",
                    finish_chunk, usage_chunk
                ))
            }

            "response.incomplete" => {
                let finish_chunk = serde_json::json!({
                    "id": format!("chatcmpl-{}", state.id),
                    "object": "chat.completion.chunk",
                    "created": state.created,
                    "model": &state.model,
                    "choices": [{
                        "index": 0,
                        "delta": {},
                        "finish_reason": "length",
                    }]
                });
                Some(format!("data: {}\n\ndata: [DONE]\n\n", finish_chunk))
            }

            // Skip: response.output_item.done, response.output_text.done,
            // response.content_part.added, response.in_progress, etc.
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// Message conversion helpers
// ---------------------------------------------------------------------------

/// Convert Chat Completions `messages` array into Responses API `instructions`
/// (from system messages) and `input` array (from user/assistant/tool messages).
fn convert_messages_to_input(
    messages: &[serde_json::Value],
) -> (Option<String>, Vec<serde_json::Value>) {
    let mut instructions_parts = Vec::new();
    let mut input = Vec::new();

    for msg in messages {
        let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("");
        match role {
            "system" | "developer" => {
                if let Some(content) = msg.get("content").and_then(|c| c.as_str()) {
                    instructions_parts.push(content.to_string());
                }
            }
            "user" => {
                // Support both string content and array content (multimodal)
                input.push(serde_json::json!({
                    "role": "user",
                    "content": msg.get("content").cloned()
                        .unwrap_or(serde_json::Value::Null),
                }));
            }
            "assistant" => {
                if let Some(tool_calls) = msg.get("tool_calls").and_then(|tc| tc.as_array()) {
                    // Emit text content as a message if present
                    if let Some(content) = msg.get("content").and_then(|c| c.as_str()) {
                        if !content.is_empty() {
                            input.push(serde_json::json!({
                                "role": "assistant",
                                "content": content,
                            }));
                        }
                    }
                    // Each tool_call becomes a separate function_call input item
                    for tc in tool_calls {
                        let id = tc.get("id").and_then(|v| v.as_str()).unwrap_or("unknown");
                        let name = tc
                            .pointer("/function/name")
                            .and_then(|v| v.as_str())
                            .unwrap_or("unknown");
                        let arguments = tc
                            .pointer("/function/arguments")
                            .and_then(|v| v.as_str())
                            .unwrap_or("{}");
                        input.push(serde_json::json!({
                            "type": "function_call",
                            "id": id,
                            "name": name,
                            "arguments": arguments,
                        }));
                    }
                } else {
                    input.push(serde_json::json!({
                        "role": "assistant",
                        "content": msg.get("content").cloned()
                            .unwrap_or(serde_json::Value::Null),
                    }));
                }
            }
            "tool" => {
                let call_id = msg
                    .get("tool_call_id")
                    .and_then(|v| v.as_str())
                    .unwrap_or("unknown");
                let output = msg.get("content").and_then(|c| c.as_str()).unwrap_or("");
                input.push(serde_json::json!({
                    "type": "function_call_output",
                    "call_id": call_id,
                    "output": output,
                }));
            }
            _ => {}
        }
    }

    let instructions = if instructions_parts.is_empty() {
        None
    } else {
        Some(instructions_parts.join("\n"))
    };

    (instructions, input)
}

// ---------------------------------------------------------------------------
// HTTP SSE transport for ChatGPT backend (matching default codex-rs approach)
// ---------------------------------------------------------------------------

/// Codex CLI version to impersonate. Should track a recent stable release.
const CODEX_VERSION: &str = "0.101.0";
const CHATGPT_RESPONSES_API_URL: &str = "https://chatgpt.com/backend-api/codex/responses";

/// Build a User-Agent string matching the codex-rs format:
/// `codex_cli_rs/{version} ({os_type} {os_version}; {arch})`
fn codex_user_agent() -> String {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    // Map Rust OS names to codex-rs os_info style names
    let os_name = match os {
        "linux" => "Linux",
        "macos" => "Mac OS",
        "windows" => "Windows",
        other => other,
    };
    format!("codex_cli_rs/{CODEX_VERSION} ({os_name}; {arch})")
}

/// Build a `reqwest::Client` for ChatGPT backend requests.
/// Configures proxy from environment if set.
fn chatgpt_http_client() -> AppResult<reqwest::Client> {
    let mut builder = reqwest::Client::builder()
        .use_rustls_tls()
        .connect_timeout(std::time::Duration::from_secs(10));

    if let Ok(proxy_url) = std::env::var("CHATGPT_PROXY_URL")
        .or_else(|_| std::env::var("HTTPS_PROXY"))
        .or_else(|_| std::env::var("https_proxy"))
    {
        tracing::debug!("ChatGPT HTTP via proxy: {proxy_url}");
        let proxy = reqwest::Proxy::https(&proxy_url)
            .map_err(|e| AppError::Internal(format!("Invalid proxy URL: {e}")))?;
        builder = builder.proxy(proxy);
    }

    builder
        .build()
        .map_err(|e| AppError::Internal(format!("Failed to build HTTP client: {e}")))
}

fn build_chatgpt_request(
    client: &reqwest::Client,
    api_url: &str,
    request_text: String,
    bearer_token: &str,
) -> AppResult<reqwest::Request> {
    client
        .post(api_url)
        .header("Authorization", format!("Bearer {bearer_token}"))
        .header("Content-Type", "application/json")
        .header("Accept", "text/event-stream")
        .header("originator", "codex_cli_rs")
        .header("User-Agent", codex_user_agent())
        .body(request_text)
        .build()
        .map_err(|e| AppError::Internal(format!("Failed to build HTTP request: {e}")))
}

fn build_chatgpt_api_url(api_url: &str, query: Option<&str>) -> String {
    let mut api_url = api_url.to_string();
    if let Some(query) = query.filter(|query| !query.is_empty()) {
        api_url.push('?');
        api_url.push_str(query);
    }
    api_url
}

/// Send a Responses API request via HTTP POST to `chatgpt.com/backend-api/codex`.
///
/// Uses `reqwest` with SSE streaming, matching the default codex CLI behavior
/// (codex-rs uses HTTP by default; WebSocket is an opt-in feature).
///
/// When `translate_response` is `true`, Responses API SSE events are
/// translated back to Chat Completions format. When `false`,
/// the raw Responses API SSE events are forwarded to the client.
pub async fn send_to_chatgpt(
    translated_body: &serde_json::Value,
    bearer_token: &str,
    is_streaming: bool,
    translate_response: bool,
    query: Option<&str>,
) -> AppResult<axum::response::Response> {
    send_to_chatgpt_with_api_url(
        translated_body,
        bearer_token,
        is_streaming,
        translate_response,
        query,
        CHATGPT_RESPONSES_API_URL,
    )
    .await
}

async fn send_to_chatgpt_with_api_url(
    translated_body: &serde_json::Value,
    bearer_token: &str,
    is_streaming: bool,
    translate_response: bool,
    query: Option<&str>,
    api_url: &str,
) -> AppResult<axum::response::Response> {
    use axum::body::Body;
    use axum::http::StatusCode;
    use futures::StreamExt;

    let api_url = build_chatgpt_api_url(api_url, query);

    let request_text = serde_json::to_string(translated_body)
        .map_err(|e| AppError::Internal(format!("Failed to serialize request: {e}")))?;

    tracing::debug!(
        translate_response,
        is_streaming,
        request_len = request_text.len(),
        api_url,
        "ChatGPT HTTP request body: {}",
        &request_text,
    );

    let client = chatgpt_http_client()?;

    let request = build_chatgpt_request(&client, &api_url, request_text, bearer_token)?;

    let response = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        client.execute(request),
    )
    .await
    .map_err(|_| {
        tracing::error!("ChatGPT HTTP request timed out waiting for response headers (30s)");
        AppError::Internal(
            "ChatGPT backend did not respond within 30 seconds".to_string(),
        )
    })?
    .map_err(|e| {
        tracing::error!("ChatGPT HTTP request failed: {e}");
        AppError::Internal(format!("ChatGPT HTTP request failed: {e}"))
    })?;

    let status = response.status();
    tracing::debug!("ChatGPT HTTP response status: {status}");

    // Forward upstream errors to the client as-is so they can see what
    // the ChatGPT backend rejected (e.g. unsupported parameter for a model).
    if !status.is_success() {
        let error_body = response.text().await.unwrap_or_default();
        tracing::warn!(
            "ChatGPT backend returned HTTP {status}: {}",
            truncate_for_log(&error_body, 1000),
        );

        let content_type = if error_body.starts_with('{') {
            "application/json"
        } else {
            "text/plain"
        };

        return axum::http::Response::builder()
            .status(status.as_u16())
            .header("content-type", content_type)
            .body(Body::from(error_body))
            .map_err(|e| AppError::Internal(format!("Failed to build error response: {e}")));
    }

    if is_streaming {
        let (tx, rx) = tokio::sync::mpsc::channel::<Result<bytes::Bytes, std::io::Error>>(32);

        tokio::spawn(async move {
            let mut received_any_event = false;

            if translate_response {
                // Chat Completions mode: translate SSE events → OpenAI chunks
                let translator = ChatgptTranslator;
                let mut state = StreamTranslationState::default();
                let mut sse_buf = String::new();
                let mut byte_stream = response.bytes_stream();

                while let Some(chunk_result) = byte_stream.next().await {
                    let chunk_bytes = match chunk_result {
                        Ok(b) => b,
                        Err(e) => {
                            tracing::warn!("ChatGPT SSE read error: {e}");
                            break;
                        }
                    };

                    sse_buf.push_str(&String::from_utf8_lossy(&chunk_bytes));

                    // Process complete SSE events (delimited by double newline)
                    while let Some(event) = extract_next_sse_event(&mut sse_buf) {
                        received_any_event = true;

                        tracing::debug!(
                            event_type = %event.event_type.as_deref().unwrap_or(""),
                            "ChatGPT SSE recv: {}",
                            truncate_for_log(&event.data, 500),
                        );

                        if let Some(ref translated) =
                            translator.translate_stream_event(&event, &mut state)
                        {
                            tracing::debug!(
                                "ChatGPT SSE emit: {}",
                                truncate_for_log(translated, 500),
                            );
                            if tx
                                .send(Ok(bytes::Bytes::from(translated.clone())))
                                .await
                                .is_err()
                            {
                                tracing::debug!("ChatGPT SSE client disconnected");
                                return;
                            }
                        }

                        let etype = event.event_type.as_deref().unwrap_or("");
                        if etype == "response.completed" || etype == "response.incomplete" {
                            return;
                        }
                    }
                }
            } else {
                // Responses API passthrough: forward SSE events directly
                let mut sse_buf = String::new();
                let mut byte_stream = response.bytes_stream();

                while let Some(chunk_result) = byte_stream.next().await {
                    let chunk_bytes = match chunk_result {
                        Ok(b) => b,
                        Err(e) => {
                            tracing::warn!("ChatGPT SSE read error: {e}");
                            break;
                        }
                    };

                    sse_buf.push_str(&String::from_utf8_lossy(&chunk_bytes));

                    while let Some(event) = extract_next_sse_event(&mut sse_buf) {
                        received_any_event = true;

                        let event_type = event.event_type.as_deref().unwrap_or("");
                        tracing::debug!(
                            event_type = %event_type,
                            "ChatGPT SSE passthrough: {}",
                            truncate_for_log(&event.data, 500),
                        );

                        let sse = format!("event: {event_type}\ndata: {}\n\n", event.data,);
                        if tx.send(Ok(bytes::Bytes::from(sse))).await.is_err() {
                            tracing::debug!("ChatGPT SSE client disconnected (passthrough)");
                            return;
                        }

                        if event_type == "response.completed" || event_type == "response.incomplete"
                        {
                            return;
                        }
                    }
                }
            }

            if !received_any_event {
                let error_msg = "ChatGPT backend returned empty SSE stream";
                tracing::error!("{error_msg}");

                if translate_response {
                    let error_chunk = serde_json::json!({
                        "error": {
                            "message": error_msg,
                            "type": "server_error",
                            "code": "upstream_error",
                        }
                    });
                    let _ = tx
                        .send(Ok(bytes::Bytes::from(format!(
                            "data: {}\n\ndata: [DONE]\n\n",
                            error_chunk,
                        ))))
                        .await;
                } else {
                    let error_event = serde_json::json!({
                        "type": "error",
                        "error": { "message": error_msg },
                    });
                    let _ = tx
                        .send(Ok(bytes::Bytes::from(format!(
                            "event: error\ndata: {}\n\n",
                            error_event,
                        ))))
                        .await;
                }
            }

            tracing::debug!("ChatGPT SSE stream ended");
        });

        let body = Body::from_stream(tokio_stream::wrappers::ReceiverStream::new(rx));
        axum::http::Response::builder()
            .status(StatusCode::OK)
            .header("content-type", "text/event-stream")
            .header("cache-control", "no-cache")
            .body(body)
            .map_err(|e| AppError::Internal(format!("Failed to build response: {e}")))
    } else {
        // Non-streaming: collect SSE events until response.completed/incomplete
        let mut final_response: Option<serde_json::Value> = None;
        let mut sse_buf = String::new();
        let mut byte_stream = response.bytes_stream();

        while let Some(chunk_result) = byte_stream.next().await {
            let chunk_bytes = match chunk_result {
                Ok(b) => b,
                Err(e) => {
                    tracing::warn!("ChatGPT SSE read error: {e}");
                    break;
                }
            };

            sse_buf.push_str(&String::from_utf8_lossy(&chunk_bytes));

            while let Some(event) = extract_next_sse_event(&mut sse_buf) {
                let etype = event.event_type.as_deref().unwrap_or("");

                tracing::debug!(
                    event_type = %etype,
                    "ChatGPT SSE recv (non-stream): {}",
                    truncate_for_log(&event.data, 500),
                );

                if etype == "response.completed" || etype == "response.incomplete" {
                    if let Ok(data) = serde_json::from_str::<serde_json::Value>(&event.data) {
                        final_response = data.get("response").cloned();
                    }
                    break;
                }
            }

            if final_response.is_some() {
                break;
            }
        }

        let resp_json = final_response.unwrap_or_else(|| {
            tracing::warn!("ChatGPT SSE: no response.completed event received");
            serde_json::json!({"error": "No response received from ChatGPT"})
        });

        let body_bytes = if translate_response {
            let translator = ChatgptTranslator;
            let translated = translator.translate_response(resp_json)?;
            tracing::debug!(
                "ChatGPT response (translated): {}",
                truncate_for_log(
                    &serde_json::to_string(&translated).unwrap_or_default(),
                    2000,
                ),
            );
            serde_json::to_vec(&translated)
        } else {
            tracing::debug!(
                "ChatGPT response (passthrough): {}",
                truncate_for_log(&serde_json::to_string(&resp_json).unwrap_or_default(), 2000,),
            );
            serde_json::to_vec(&resp_json)
        }
        .map_err(|e| AppError::Internal(format!("Failed to serialize response: {e}")))?;

        axum::http::Response::builder()
            .status(StatusCode::OK)
            .header("content-type", "application/json")
            .body(Body::from(body_bytes))
            .map_err(|e| AppError::Internal(format!("Failed to build response: {e}")))
    }
}

/// Extract the next complete SSE event from a buffer.
///
/// SSE events are delimited by double newlines (`\n\n`). Each event may have:
/// - `event: <type>` line
/// - `data: <payload>` line(s)
///
/// Returns `None` if no complete event is available yet.
fn extract_next_sse_event(buf: &mut String) -> Option<SseEvent> {
    let delimiter = "\n\n";
    let pos = buf.find(delimiter)?;

    let raw = buf[..pos].to_string();
    *buf = buf[pos + delimiter.len()..].to_string();

    let mut event_type = None;
    let mut data_parts = Vec::new();

    for line in raw.lines() {
        if let Some(val) = line.strip_prefix("event: ") {
            event_type = Some(val.to_string());
        } else if let Some(val) = line.strip_prefix("data: ") {
            data_parts.push(val.to_string());
        } else if line.starts_with("event:") {
            event_type = Some(line["event:".len()..].trim().to_string());
        } else if line.starts_with("data:") {
            data_parts.push(line["data:".len()..].trim().to_string());
        }
    }

    if data_parts.is_empty() && event_type.is_none() {
        return None;
    }

    let data = data_parts.join("\n");

    // If no explicit event: header, try to extract type from the JSON data
    if event_type.is_none() {
        if let Ok(parsed) = serde_json::from_str::<serde_json::Value>(&data) {
            event_type = parsed
                .get("type")
                .and_then(|t| t.as_str())
                .map(String::from);
        }
    }

    Some(SseEvent { event_type, data })
}

/// Truncate a string for logging, appending "..." if cut.
fn truncate_for_log(s: &str, max: usize) -> &str {
    if s.len() <= max {
        s
    } else {
        // Find a valid UTF-8 boundary
        let mut end = max;
        while end > 0 && !s.is_char_boundary(end) {
            end -= 1;
        }
        &s[..end]
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // --- Trait method tests ---

    #[test]
    fn chatgpt_needs_translation_true() {
        let translator = ChatgptTranslator;
        assert!(translator.needs_translation());
    }

    #[test]
    fn chatgpt_gateway_base_url() {
        let translator = ChatgptTranslator;
        assert_eq!(
            translator.gateway_base_url(),
            Some("https://chatgpt.com/backend-api/codex")
        );
    }

    // --- Request translation tests ---

    #[test]
    fn chatgpt_translate_request_extracts_system() {
        let translator = ChatgptTranslator;
        let body = serde_json::json!({
            "model": "gpt-5.2",
            "messages": [
                {"role": "system", "content": "You are helpful."},
                {"role": "user", "content": "Hello"}
            ],
            "max_tokens": 1024,
            "temperature": 0.7
        });

        let result = translator
            .translate_request("chat/completions", &body)
            .unwrap();

        assert_eq!(result.path, "responses");
        assert_eq!(result.body["instructions"], "You are helpful.");
        assert_eq!(result.body["model"], "gpt-5.2");
        assert_eq!(result.body["temperature"], 0.7);
        assert_eq!(result.body["store"], false);
        // Token limit params are stripped (Codex backend rejects them)
        assert!(result.body.get("max_tokens").is_none());
        assert!(result.body.get("max_output_tokens").is_none());
        assert!(result.body.get("messages").is_none());

        let input = result.body["input"].as_array().unwrap();
        assert_eq!(input.len(), 1);
        assert_eq!(input[0]["role"], "user");
        assert_eq!(input[0]["content"], "Hello");
    }

    #[test]
    fn chatgpt_translate_request_multiple_system_messages() {
        let translator = ChatgptTranslator;
        let body = serde_json::json!({
            "model": "gpt-4o",
            "messages": [
                {"role": "system", "content": "First instruction."},
                {"role": "system", "content": "Second instruction."},
                {"role": "user", "content": "Hi"}
            ]
        });

        let result = translator
            .translate_request("chat/completions", &body)
            .unwrap();

        assert_eq!(
            result.body["instructions"],
            "First instruction.\nSecond instruction."
        );
    }

    #[test]
    fn chatgpt_translate_request_no_system_messages() {
        let translator = ChatgptTranslator;
        let body = serde_json::json!({
            "model": "gpt-4o",
            "messages": [
                {"role": "user", "content": "Hello"}
            ]
        });

        let result = translator
            .translate_request("chat/completions", &body)
            .unwrap();

        // Codex backend requires instructions; defaults to empty string
        assert_eq!(result.body["instructions"], "");
    }

    #[test]
    fn chatgpt_translate_request_tool_calls() {
        let translator = ChatgptTranslator;
        let body = serde_json::json!({
            "model": "gpt-4o",
            "messages": [
                {"role": "user", "content": "What's the weather?"},
                {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call_1",
                        "type": "function",
                        "function": {
                            "name": "get_weather",
                            "arguments": "{\"location\":\"NYC\"}"
                        }
                    }]
                },
                {
                    "role": "tool",
                    "tool_call_id": "call_1",
                    "content": "Sunny, 72F"
                }
            ]
        });

        let result = translator
            .translate_request("chat/completions", &body)
            .unwrap();

        let input = result.body["input"].as_array().unwrap();
        assert_eq!(input.len(), 3);

        assert_eq!(input[0]["role"], "user");
        assert_eq!(input[1]["type"], "function_call");
        assert_eq!(input[1]["id"], "call_1");
        assert_eq!(input[1]["name"], "get_weather");
        assert_eq!(input[1]["arguments"], "{\"location\":\"NYC\"}");
        assert_eq!(input[2]["type"], "function_call_output");
        assert_eq!(input[2]["call_id"], "call_1");
        assert_eq!(input[2]["output"], "Sunny, 72F");
    }

    #[test]
    fn chatgpt_translate_request_adds_store_and_include() {
        let translator = ChatgptTranslator;
        let body = serde_json::json!({
            "model": "gpt-4o",
            "messages": [{"role": "user", "content": "Hi"}]
        });

        let result = translator
            .translate_request("chat/completions", &body)
            .unwrap();

        assert_eq!(result.body["store"], false);
        // include:["usage"] is NOT added -- usage is returned by default
        assert!(result.body.get("include").is_none());
    }

    #[test]
    fn chatgpt_translate_request_strips_stop() {
        let translator = ChatgptTranslator;
        let body = serde_json::json!({
            "model": "gpt-4o",
            "messages": [{"role": "user", "content": "Hi"}],
            "stop": ["\n"]
        });

        let result = translator
            .translate_request("chat/completions", &body)
            .unwrap();

        assert!(result.body.get("stop").is_none());
    }

    #[test]
    fn chatgpt_translate_request_passthrough_tools() {
        let translator = ChatgptTranslator;
        let tools = serde_json::json!([{
            "type": "function",
            "function": {
                "name": "get_weather",
                "parameters": {"type": "object", "properties": {}}
            }
        }]);
        let body = serde_json::json!({
            "model": "gpt-4o",
            "messages": [{"role": "user", "content": "Hi"}],
            "tools": tools,
            "tool_choice": "auto"
        });

        let result = translator
            .translate_request("chat/completions", &body)
            .unwrap();

        assert_eq!(result.body["tools"], tools);
        assert_eq!(result.body["tool_choice"], "auto");
    }

    // --- Format detection tests ---

    #[test]
    fn detect_chat_completions_format() {
        let body = serde_json::json!({
            "model": "gpt-4o",
            "messages": [{"role": "user", "content": "Hi"}]
        });
        assert!(is_chat_completions_format(&body));
    }

    #[test]
    fn detect_responses_api_format() {
        let body = serde_json::json!({
            "model": "gpt-4o",
            "input": "Hello"
        });
        assert!(!is_chat_completions_format(&body));
    }

    // --- Responses API passthrough tests ---

    #[test]
    fn responses_api_passthrough_preserves_input() {
        let translator = ChatgptTranslator;
        let body = serde_json::json!({
            "model": "o3",
            "input": [{"role": "user", "content": "Hello"}],
            "instructions": "Be helpful",
            "max_output_tokens": 1024,
            "stream": true
        });

        let result = translator.translate_request("responses", &body).unwrap();

        assert_eq!(result.path, "responses");
        assert_eq!(result.body["model"], "o3");
        assert_eq!(result.body["input"][0]["role"], "user");
        assert_eq!(result.body["instructions"], "Be helpful");
        assert_eq!(result.body["stream"], true);
        assert_eq!(result.body["store"], false);
        // Token limit params are stripped (Codex backend rejects them)
        assert!(result.body.get("max_output_tokens").is_none());
        // include is NOT added by us -- usage is default in Responses API
        assert!(result.body.get("include").is_none());
        // Should NOT have messages
        assert!(result.body.get("messages").is_none());
    }

    #[test]
    fn responses_api_passthrough_preserves_custom_include() {
        let translator = ChatgptTranslator;
        let body = serde_json::json!({
            "model": "o3",
            "input": "Hi",
            "include": ["reasoning.encrypted_content"]
        });

        let result = translator.translate_request("responses", &body).unwrap();

        // Should preserve client's include as-is
        assert_eq!(
            result.body["include"],
            serde_json::json!(["reasoning.encrypted_content"])
        );
    }

    #[test]
    fn responses_api_passthrough_string_input() {
        let translator = ChatgptTranslator;
        let body = serde_json::json!({
            "model": "o3",
            "input": "What is 2+2?"
        });

        let result = translator.translate_request("responses", &body).unwrap();

        assert_eq!(result.body["input"], "What is 2+2?");
    }

    #[test]
    fn build_chatgpt_request_preserves_query_headers_and_body() {
        let translated_body = serde_json::json!({
            "model": "o3",
            "input": "Ping",
            "stream": true,
            "store": false,
            "instructions": ""
        });
        let api_url = format!("{CHATGPT_RESPONSES_API_URL}?trace=1&mode=test");
        let request_text = serde_json::to_string(&translated_body).unwrap();
        let client = reqwest::Client::builder().build().unwrap();
        let request =
            build_chatgpt_request(&client, &api_url, request_text, "test-bearer").unwrap();

        assert_eq!(
            request.url().as_str(),
            "https://chatgpt.com/backend-api/codex/responses?trace=1&mode=test"
        );
        assert_eq!(
            request.headers().get("authorization").unwrap(),
            "Bearer test-bearer"
        );
        assert_eq!(
            request.headers().get("content-type").unwrap(),
            "application/json"
        );
        assert_eq!(
            request.headers().get("accept").unwrap(),
            "text/event-stream"
        );
        assert_eq!(request.headers().get("originator").unwrap(), "codex_cli_rs");
        assert!(
            request
                .headers()
                .get("user-agent")
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with("codex_cli_rs/")
        );
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(
                request.body().unwrap().as_bytes().unwrap()
            )
            .unwrap(),
            translated_body
        );
    }

    #[test]
    fn send_to_chatgpt_query_preserves_empty_and_non_empty_cases() {
        let without_query = build_chatgpt_api_url(CHATGPT_RESPONSES_API_URL, None);
        let with_query = build_chatgpt_api_url(CHATGPT_RESPONSES_API_URL, Some("trace=1"));

        assert_eq!(without_query, CHATGPT_RESPONSES_API_URL);
        assert_eq!(
            with_query,
            "https://chatgpt.com/backend-api/codex/responses?trace=1"
        );
    }

    // --- Response translation tests ---

    #[test]
    fn chatgpt_translate_response_text_only() {
        let translator = ChatgptTranslator;
        let resp = serde_json::json!({
            "id": "resp_abc123",
            "object": "response",
            "created_at": 1700000000,
            "model": "gpt-5.2",
            "output": [{
                "type": "message",
                "role": "assistant",
                "content": [{"type": "output_text", "text": "Hello!"}]
            }],
            "usage": {"input_tokens": 10, "output_tokens": 5, "total_tokens": 15},
            "status": "completed"
        });

        let result = translator.translate_response(resp).unwrap();

        assert_eq!(result["id"], "chatcmpl-resp_abc123");
        assert_eq!(result["object"], "chat.completion");
        assert_eq!(result["created"], 1700000000);
        assert_eq!(result["model"], "gpt-5.2");
        assert_eq!(result["choices"][0]["message"]["content"], "Hello!");
        assert_eq!(result["choices"][0]["message"]["role"], "assistant");
        assert_eq!(result["choices"][0]["finish_reason"], "stop");
        assert_eq!(result["usage"]["prompt_tokens"], 10);
        assert_eq!(result["usage"]["completion_tokens"], 5);
        assert_eq!(result["usage"]["total_tokens"], 15);
    }

    #[test]
    fn chatgpt_translate_response_function_calls() {
        let translator = ChatgptTranslator;
        let resp = serde_json::json!({
            "id": "resp_tool",
            "object": "response",
            "created_at": 1700000000,
            "model": "gpt-5.2",
            "output": [{
                "type": "function_call",
                "id": "call_1",
                "name": "get_weather",
                "arguments": "{\"location\":\"NYC\"}"
            }],
            "usage": {"input_tokens": 10, "output_tokens": 20, "total_tokens": 30},
            "status": "completed"
        });

        let result = translator.translate_response(resp).unwrap();

        assert_eq!(result["choices"][0]["finish_reason"], "tool_calls");
        assert!(result["choices"][0]["message"]["content"].is_null());

        let tc = &result["choices"][0]["message"]["tool_calls"];
        assert_eq!(tc[0]["id"], "call_1");
        assert_eq!(tc[0]["type"], "function");
        assert_eq!(tc[0]["function"]["name"], "get_weather");
        assert_eq!(tc[0]["function"]["arguments"], "{\"location\":\"NYC\"}");
    }

    #[test]
    fn chatgpt_translate_response_mixed_text_and_tools() {
        let translator = ChatgptTranslator;
        let resp = serde_json::json!({
            "id": "resp_mixed",
            "object": "response",
            "created_at": 1700000000,
            "model": "gpt-5.2",
            "output": [
                {
                    "type": "message",
                    "role": "assistant",
                    "content": [{"type": "output_text", "text": "Let me check."}]
                },
                {
                    "type": "function_call",
                    "id": "call_1",
                    "name": "search",
                    "arguments": "{\"q\":\"test\"}"
                }
            ],
            "usage": {"input_tokens": 5, "output_tokens": 10, "total_tokens": 15},
            "status": "completed"
        });

        let result = translator.translate_response(resp).unwrap();

        assert_eq!(result["choices"][0]["message"]["content"], "Let me check.");
        assert_eq!(result["choices"][0]["finish_reason"], "tool_calls");
        assert_eq!(
            result["choices"][0]["message"]["tool_calls"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn chatgpt_translate_response_incomplete() {
        let translator = ChatgptTranslator;
        let resp = serde_json::json!({
            "id": "resp_inc",
            "model": "gpt-5.2",
            "output": [{
                "type": "message",
                "role": "assistant",
                "content": [{"type": "output_text", "text": "truncated"}]
            }],
            "usage": {"input_tokens": 10, "output_tokens": 100, "total_tokens": 110},
            "status": "incomplete"
        });

        let result = translator.translate_response(resp).unwrap();
        assert_eq!(result["choices"][0]["finish_reason"], "length");
    }

    // --- Streaming translation tests ---

    fn make_event(event_type: &str, data: &str) -> SseEvent {
        SseEvent {
            event_type: Some(event_type.to_string()),
            data: data.to_string(),
        }
    }

    /// Parse the first `data: {...}` line from an SSE payload into JSON.
    /// Skips `data: [DONE]` and blank lines.
    fn parse_chunk_json(sse_payload: &str) -> serde_json::Value {
        parse_nth_chunk_json(sse_payload, 0)
    }

    /// Parse the Nth JSON `data:` line (0-indexed) from an SSE payload.
    fn parse_nth_chunk_json(sse_payload: &str, n: usize) -> serde_json::Value {
        let mut found = 0;
        for line in sse_payload.lines() {
            let trimmed = line.trim();
            if let Some(json_str) = trimmed.strip_prefix("data: ") {
                if json_str == "[DONE]" {
                    continue;
                }
                if found == n {
                    return serde_json::from_str(json_str).unwrap();
                }
                found += 1;
            }
        }
        panic!("No data line at index {n} found in SSE payload: {sse_payload}");
    }

    #[test]
    fn chatgpt_stream_response_created() {
        let translator = ChatgptTranslator;
        let mut state = StreamTranslationState::default();

        let event = make_event(
            "response.created",
            r#"{"type":"response.created","response":{"id":"resp_abc","model":"gpt-5.2","created_at":1700000000,"status":"in_progress"}}"#,
        );

        let result = translator
            .translate_stream_event(&event, &mut state)
            .unwrap();
        let chunk = parse_chunk_json(&result);

        assert_eq!(chunk["id"], "chatcmpl-resp_abc");
        assert_eq!(chunk["object"], "chat.completion.chunk");
        assert_eq!(chunk["created"], 1700000000);
        assert_eq!(chunk["model"], "gpt-5.2");
        assert_eq!(chunk["choices"][0]["delta"]["role"], "assistant");
        assert!(chunk["choices"][0]["finish_reason"].is_null());
        assert_eq!(state.id, "resp_abc");
        assert_eq!(state.model, "gpt-5.2");
        assert_eq!(state.created, 1700000000);
    }

    #[test]
    fn chatgpt_stream_text_delta() {
        let translator = ChatgptTranslator;
        let mut state = StreamTranslationState {
            id: "resp_abc".to_string(),
            model: "gpt-5.2".to_string(),
            ..Default::default()
        };

        let event = make_event(
            "response.output_text.delta",
            r#"{"type":"response.output_text.delta","output_index":0,"content_index":0,"delta":"Hello world"}"#,
        );

        let result = translator
            .translate_stream_event(&event, &mut state)
            .unwrap();
        let chunk = parse_chunk_json(&result);

        assert_eq!(chunk["choices"][0]["delta"]["content"], "Hello world");
        assert!(chunk["choices"][0]["finish_reason"].is_null());
    }

    #[test]
    fn chatgpt_stream_function_call_added() {
        let translator = ChatgptTranslator;
        let mut state = StreamTranslationState {
            id: "resp_abc".to_string(),
            model: "gpt-5.2".to_string(),
            ..Default::default()
        };

        let event = make_event(
            "response.output_item.added",
            r#"{"type":"response.output_item.added","output_index":0,"item":{"type":"function_call","id":"call_123","name":"get_weather","arguments":"","status":"in_progress"}}"#,
        );

        let result = translator
            .translate_stream_event(&event, &mut state)
            .unwrap();
        let chunk = parse_chunk_json(&result);

        assert_eq!(chunk["choices"][0]["delta"]["tool_calls"][0]["index"], 0);
        assert_eq!(
            chunk["choices"][0]["delta"]["tool_calls"][0]["id"],
            "call_123"
        );
        assert_eq!(
            chunk["choices"][0]["delta"]["tool_calls"][0]["function"]["name"],
            "get_weather"
        );
        assert_eq!(state.next_tool_index, 1);
    }

    #[test]
    fn chatgpt_stream_function_call_arguments_delta() {
        let translator = ChatgptTranslator;
        let mut state = StreamTranslationState {
            id: "resp_abc".to_string(),
            model: "gpt-5.2".to_string(),
            tool_call_indices: vec![(0, 0)],
            next_tool_index: 1,
            ..Default::default()
        };

        let event = make_event(
            "response.function_call_arguments.delta",
            r#"{"type":"response.function_call_arguments.delta","output_index":0,"delta":"{\"location\":"}"#,
        );

        let result = translator
            .translate_stream_event(&event, &mut state)
            .unwrap();
        let chunk = parse_chunk_json(&result);

        assert_eq!(chunk["choices"][0]["delta"]["tool_calls"][0]["index"], 0);
        assert_eq!(
            chunk["choices"][0]["delta"]["tool_calls"][0]["function"]["arguments"],
            "{\"location\":"
        );
    }

    #[test]
    fn chatgpt_stream_response_completed() {
        let translator = ChatgptTranslator;
        let mut state = StreamTranslationState {
            id: "resp_abc".to_string(),
            model: "gpt-5.2".to_string(),
            ..Default::default()
        };

        let event = make_event(
            "response.completed",
            r#"{"type":"response.completed","response":{"id":"resp_abc","status":"completed","usage":{"input_tokens":25,"output_tokens":15,"total_tokens":40}}}"#,
        );

        let result = translator
            .translate_stream_event(&event, &mut state)
            .unwrap();

        // Should contain finish chunk, usage chunk, and [DONE]
        assert!(result.contains("data: [DONE]"));

        // First chunk: finish_reason, no usage
        let finish_chunk = parse_nth_chunk_json(&result, 0);
        assert_eq!(finish_chunk["choices"][0]["finish_reason"], "stop");
        assert!(finish_chunk.get("usage").is_none());

        // Second chunk: empty choices, usage only
        let usage_chunk = parse_nth_chunk_json(&result, 1);
        assert_eq!(usage_chunk["choices"].as_array().unwrap().len(), 0);
        assert_eq!(usage_chunk["usage"]["prompt_tokens"], 25);
        assert_eq!(usage_chunk["usage"]["completion_tokens"], 15);
        assert_eq!(usage_chunk["usage"]["total_tokens"], 40);
    }

    #[test]
    fn chatgpt_stream_response_completed_with_tool_calls() {
        let translator = ChatgptTranslator;
        let mut state = StreamTranslationState {
            id: "resp_abc".to_string(),
            model: "gpt-5.2".to_string(),
            next_tool_index: 1,
            ..Default::default()
        };

        let event = make_event(
            "response.completed",
            r#"{"type":"response.completed","response":{"id":"resp_abc","status":"completed","usage":{"input_tokens":10,"output_tokens":20,"total_tokens":30}}}"#,
        );

        let result = translator
            .translate_stream_event(&event, &mut state)
            .unwrap();

        let finish_chunk = parse_nth_chunk_json(&result, 0);
        assert_eq!(finish_chunk["choices"][0]["finish_reason"], "tool_calls");

        let usage_chunk = parse_nth_chunk_json(&result, 1);
        assert_eq!(usage_chunk["choices"].as_array().unwrap().len(), 0);
        assert_eq!(usage_chunk["usage"]["prompt_tokens"], 10);
    }

    #[test]
    fn chatgpt_stream_response_incomplete() {
        let translator = ChatgptTranslator;
        let mut state = StreamTranslationState {
            id: "resp_abc".to_string(),
            model: "gpt-5.2".to_string(),
            ..Default::default()
        };

        let event = make_event(
            "response.incomplete",
            r#"{"type":"response.incomplete","response":{"id":"resp_abc","status":"incomplete","incomplete_details":{"reason":"max_output_tokens"}}}"#,
        );

        let result = translator
            .translate_stream_event(&event, &mut state)
            .unwrap();

        assert!(result.contains("data: [DONE]"));
        let chunk = parse_chunk_json(&result);
        assert_eq!(chunk["choices"][0]["finish_reason"], "length");
    }

    #[test]
    fn chatgpt_stream_unknown_event_skipped() {
        let translator = ChatgptTranslator;
        let mut state = StreamTranslationState::default();

        let event = make_event(
            "response.output_text.done",
            r#"{"type":"response.output_text.done","text":"full text"}"#,
        );
        assert!(
            translator
                .translate_stream_event(&event, &mut state)
                .is_none()
        );

        let event2 = make_event(
            "response.content_part.added",
            r#"{"type":"response.content_part.added"}"#,
        );
        assert!(
            translator
                .translate_stream_event(&event2, &mut state)
                .is_none()
        );
    }

    #[test]
    fn chatgpt_stream_message_item_skipped() {
        let translator = ChatgptTranslator;
        let mut state = StreamTranslationState::default();

        let event = make_event(
            "response.output_item.added",
            r#"{"type":"response.output_item.added","output_index":0,"item":{"type":"message","role":"assistant","content":[]}}"#,
        );
        assert!(
            translator
                .translate_stream_event(&event, &mut state)
                .is_none()
        );
    }

    // --- SSE event parser tests ---

    #[test]
    fn extract_sse_event_basic() {
        let mut buf =
            "event: response.created\ndata: {\"type\":\"response.created\"}\n\n".to_string();
        let event = extract_next_sse_event(&mut buf).unwrap();
        assert_eq!(event.event_type.as_deref(), Some("response.created"));
        assert_eq!(event.data, "{\"type\":\"response.created\"}");
        assert!(buf.is_empty());
    }

    #[test]
    fn extract_sse_event_data_only() {
        let mut buf =
            "data: {\"type\":\"response.output_text.delta\",\"delta\":\"hi\"}\n\n".to_string();
        let event = extract_next_sse_event(&mut buf).unwrap();
        // Should extract type from JSON data when no event: header
        assert_eq!(
            event.event_type.as_deref(),
            Some("response.output_text.delta"),
        );
        assert!(buf.is_empty());
    }

    #[test]
    fn extract_sse_event_incomplete_returns_none() {
        let mut buf = "event: response.created\ndata: {\"partial".to_string();
        assert!(extract_next_sse_event(&mut buf).is_none());
        // Buffer should be untouched
        assert_eq!(buf, "event: response.created\ndata: {\"partial");
    }

    #[test]
    fn extract_sse_event_multiple_events() {
        let mut buf =
            "event: a\ndata: {\"type\":\"a\"}\n\nevent: b\ndata: {\"type\":\"b\"}\n\n".to_string();
        let e1 = extract_next_sse_event(&mut buf).unwrap();
        assert_eq!(e1.event_type.as_deref(), Some("a"));
        let e2 = extract_next_sse_event(&mut buf).unwrap();
        assert_eq!(e2.event_type.as_deref(), Some("b"));
        assert!(buf.is_empty());
    }

    #[test]
    fn extract_sse_event_empty_lines_skipped() {
        let mut buf = "\n\nevent: test\ndata: {}\n\n".to_string();
        // First extraction gets empty block (no event/data lines)
        let first = extract_next_sse_event(&mut buf);
        // Empty block has no event_type and no data -- returns None
        assert!(first.is_none() || first.unwrap().data.is_empty());
    }
}

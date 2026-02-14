//! Translator between OpenAI Chat Completions format and the Responses API
//! format used by `chatgpt.com/backend-api/codex`.
//!
//! The `openai-codex` device code flow produces OIDC tokens that are only
//! valid at the ChatGPT backend (not `api.openai.com`). The ChatGPT backend
//! speaks the Responses API wire format, so this translator bridges the gap.

use tracing;

use crate::errors::{AppError, AppResult};
use crate::services::llm_gateway_service::{
    LlmTranslator, SseEvent, StreamTranslationState, TranslatedRequest,
};

pub struct ChatgptTranslator;

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
        let mut translated = serde_json::Map::new();

        // Passthrough fields
        for key in &["model", "temperature", "top_p", "stream", "tools", "tool_choice"] {
            if let Some(val) = body.get(*key) {
                translated.insert(key.to_string(), val.clone());
            }
        }

        // Convert messages -> instructions + input
        if let Some(messages) = body.get("messages").and_then(|m| m.as_array()) {
            let (instructions, input) = convert_messages_to_input(messages);
            if let Some(instr) = instructions {
                translated.insert(
                    "instructions".to_string(),
                    serde_json::Value::String(instr),
                );
            }
            translated.insert("input".to_string(), serde_json::Value::Array(input));
        }

        // Rename max_tokens -> max_output_tokens
        if let Some(max) = body
            .get("max_tokens")
            .or_else(|| body.get("max_completion_tokens"))
        {
            translated.insert("max_output_tokens".to_string(), max.clone());
        }

        // Do not store responses in the user's ChatGPT history
        translated.insert("store".to_string(), serde_json::Value::Bool(false));

        // Request usage in the response
        translated.insert(
            "include".to_string(),
            serde_json::json!(["usage"]),
        );

        // Path: chat/completions -> responses
        let translated_path = path.replace("chat/completions", "responses");

        // No browser-impersonation headers -- match codex-rs which connects
        // honestly as a CLI client (no Origin/Referer/browser UA).
        let extra_headers = vec![];

        Ok(TranslatedRequest {
            path: translated_path,
            body: serde_json::Value::Object(translated),
            extra_headers,
        })
    }

    fn translate_response(
        &self,
        body: serde_json::Value,
    ) -> AppResult<serde_json::Value> {
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
                            if block.get("type").and_then(|t| t.as_str())
                                == Some("output_text")
                            {
                                if let Some(text) =
                                    block.get("text").and_then(|t| t.as_str())
                                {
                                    text_parts.push(text.to_string());
                                }
                            }
                        }
                    }
                }
                "function_call" => {
                    let id = item
                        .get("id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown");
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

        let id = body
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");
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

                    let tool_id = item
                        .get("id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown");
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

                let chunk = serde_json::json!({
                    "id": format!("chatcmpl-{}", state.id),
                    "object": "chat.completion.chunk",
                    "created": state.created,
                    "model": &state.model,
                    "choices": [{
                        "index": 0,
                        "delta": {},
                        "finish_reason": finish_reason,
                    }],
                    "usage": {
                        "prompt_tokens": input_tokens,
                        "completion_tokens": output_tokens,
                        "total_tokens": input_tokens + output_tokens,
                    }
                });
                Some(format!("data: {}\n\ndata: [DONE]\n\n", chunk))
            }

            "response.incomplete" => {
                let chunk = serde_json::json!({
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
                Some(format!("data: {}\n\ndata: [DONE]\n\n", chunk))
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
                let output = msg
                    .get("content")
                    .and_then(|c| c.as_str())
                    .unwrap_or("");
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
// WebSocket transport for ChatGPT backend (matching codex-rs approach)
// ---------------------------------------------------------------------------

/// Establish a WebSocket connection to `chatgpt.com:443` tunneled through an
/// HTTP CONNECT proxy. This bypasses Cloudflare datacenter IP blocking.
async fn connect_via_proxy(
    request: tokio_tungstenite::tungstenite::http::Request<()>,
    proxy_url: &str,
    ws_config: tokio_tungstenite::tungstenite::protocol::WebSocketConfig,
) -> AppResult<(
    tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    tokio_tungstenite::tungstenite::http::Response<Option<Vec<u8>>>,
)> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    let parsed = url::Url::parse(proxy_url).map_err(|e| {
        AppError::Internal(format!("Invalid CHATGPT_PROXY_URL: {e}"))
    })?;

    let proxy_host = parsed
        .host_str()
        .ok_or_else(|| AppError::Internal("CHATGPT_PROXY_URL missing host".into()))?;
    let proxy_port = parsed.port().unwrap_or(match parsed.scheme() {
        "socks5" | "socks5h" => 1080,
        _ => 3128,
    });
    let proxy_addr = format!("{proxy_host}:{proxy_port}");

    tracing::debug!("WebSocket via proxy {proxy_addr} -> chatgpt.com:443");

    // 1. TCP connect to proxy
    let mut proxy_stream =
        tokio::net::TcpStream::connect(&proxy_addr).await.map_err(|e| {
            tracing::error!("Failed to connect to proxy {proxy_addr}: {e}");
            AppError::Internal(format!("Failed to connect to proxy: {e}"))
        })?;

    // 2. Send HTTP CONNECT to tunnel to chatgpt.com:443
    let connect_req =
        format!("CONNECT chatgpt.com:443 HTTP/1.1\r\nHost: chatgpt.com:443\r\n\r\n");
    proxy_stream
        .write_all(connect_req.as_bytes())
        .await
        .map_err(|e| AppError::Internal(format!("Proxy CONNECT write failed: {e}")))?;

    // 3. Read proxy response -- expect "HTTP/1.1 200"
    let mut buf = vec![0u8; 4096];
    let n = proxy_stream.read(&mut buf).await.map_err(|e| {
        AppError::Internal(format!("Proxy CONNECT read failed: {e}"))
    })?;
    let resp_str = String::from_utf8_lossy(&buf[..n]);
    if !resp_str.starts_with("HTTP/1.1 200") && !resp_str.starts_with("HTTP/1.0 200") {
        tracing::error!("Proxy CONNECT rejected: {resp_str}");
        return Err(AppError::Internal(format!(
            "Proxy CONNECT rejected: {resp_str}"
        )));
    }

    tracing::debug!("Proxy tunnel established");

    // 4. TLS handshake over the tunneled connection, then WebSocket upgrade.
    //    We use the rustls connector from tokio-tungstenite.
    let tls_connector = build_tls_connector()?;

    let server_name = rustls_pki_types::ServerName::try_from("chatgpt.com")
        .map_err(|e| AppError::Internal(format!("Invalid server name: {e}")))?
        .to_owned();

    let tls_stream = tls_connector
        .connect(server_name, proxy_stream)
        .await
        .map_err(|e| {
            tracing::error!("TLS handshake through proxy failed: {e}");
            AppError::Internal(format!("TLS handshake through proxy failed: {e}"))
        })?;

    // 5. WebSocket handshake over the TLS stream
    let (ws_stream, response) = tokio_tungstenite::client_async_with_config(
        request,
        tokio_tungstenite::MaybeTlsStream::Rustls(tls_stream),
        Some(ws_config),
    )
    .await
    .map_err(|e| {
        let detail = log_ws_error_detail(&e);
        tracing::error!("ChatGPT WebSocket handshake via proxy failed: {detail}");
        AppError::Internal(format!(
            "ChatGPT WebSocket handshake via proxy failed: {e}"
        ))
    })?;

    Ok((ws_stream, response))
}

/// Extract diagnostic details from a tungstenite WebSocket error.
fn log_ws_error_detail(e: &tokio_tungstenite::tungstenite::Error) -> String {
    match e {
        tokio_tungstenite::tungstenite::Error::Http(resp) => {
            let status = resp.status();
            let server = resp
                .headers()
                .get("server")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("unknown");
            let cf_ray = resp
                .headers()
                .get("cf-ray")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("none");
            let body = resp
                .body()
                .as_ref()
                .and_then(|b| String::from_utf8(b.clone()).ok())
                .unwrap_or_default();
            let body_preview = if body.len() > 500 {
                &body[..500]
            } else {
                &body
            };
            format!("HTTP {status}, server={server}, cf-ray={cf_ray}, body={body_preview}")
        }
        _ => format!("{e}"),
    }
}

/// Build a `tokio_rustls::TlsConnector` with webpki root certificates.
/// Explicitly selects `ring` as the crypto provider to avoid ambiguity when
/// both `ring` and `aws-lc-rs` are enabled via Cargo feature unification.
fn build_tls_connector() -> AppResult<tokio_rustls::TlsConnector> {
    let mut root_store = rustls::RootCertStore::empty();
    root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());

    let config = rustls::ClientConfig::builder_with_provider(
        rustls::crypto::ring::default_provider().into(),
    )
    .with_safe_default_protocol_versions()
    .map_err(|e| AppError::Internal(format!("TLS config error: {e}")))?
    .with_root_certificates(root_store)
    .with_no_client_auth();

    Ok(tokio_rustls::TlsConnector::from(std::sync::Arc::new(config)))
}

/// Send a Responses API request via WebSocket to `chatgpt.com/backend-api/codex`.
///
/// Uses `tokio-tungstenite` with `rustls` matching the codex CLI (`codex-rs`).
/// Supports optional HTTP CONNECT proxy via `CHATGPT_PROXY_URL` env var.
pub async fn send_to_chatgpt(
    translated_body: &serde_json::Value,
    bearer_token: &str,
    is_streaming: bool,
) -> AppResult<axum::response::Response> {
    use axum::body::Body;
    use axum::http::StatusCode;
    use futures::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite;
    use tungstenite::client::IntoClientRequest;

    // Ensure ring is installed as the process-level default crypto provider
    // (same as codex-rs ensure_rustls_crypto_provider). Required because
    // tokio-tungstenite internally calls ClientConfig::builder() which needs
    // a default provider when both ring and aws-lc-rs are in the dep graph.
    static RUSTLS_INIT: std::sync::Once = std::sync::Once::new();
    RUSTLS_INIT.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });

    let ws_url = "wss://chatgpt.com/backend-api/codex/responses";

    // Build the WebSocket request exactly like codex-rs:
    // 1. Use into_client_request() to let tungstenite auto-set WS headers
    //    (Host, Connection, Upgrade, Sec-WebSocket-Version, Sec-WebSocket-Key)
    // 2. Only add application-specific headers
    let mut request = ws_url
        .into_client_request()
        .map_err(|e| AppError::Internal(format!("Failed to build WS request: {e}")))?;

    let headers = request.headers_mut();
    headers.insert(
        "Authorization",
        format!("Bearer {bearer_token}")
            .parse()
            .map_err(|e| AppError::Internal(format!("Invalid auth header: {e}")))?,
    );
    headers.insert(
        "OpenAI-Beta",
        "responses_websockets=2026-02-04"
            .parse()
            .unwrap(),
    );
    headers.insert("originator", "codex_cli_rs".parse().unwrap());
    headers.insert(
        "User-Agent",
        "codex_cli_rs/1.0.5 (Linux 6.1.0; x86_64)"
            .parse()
            .unwrap(),
    );

    let ws_config = tungstenite::protocol::WebSocketConfig::default();

    // Check for proxy configuration (CHATGPT_PROXY_URL or HTTPS_PROXY)
    let proxy_url = std::env::var("CHATGPT_PROXY_URL")
        .or_else(|_| std::env::var("HTTPS_PROXY"))
        .or_else(|_| std::env::var("https_proxy"))
        .ok();

    let (mut ws_stream, response) = if let Some(ref proxy) = proxy_url {
        connect_via_proxy(request, proxy, ws_config).await?
    } else {
        tracing::debug!("WebSocket {ws_url} (direct, stream={is_streaming})");
        tokio_tungstenite::connect_async_with_config(request, Some(ws_config), false)
            .await
            .map_err(|e| {
                tracing::error!(
                    "ChatGPT WebSocket connection failed: {}",
                    log_ws_error_detail(&e)
                );
                AppError::Internal(format!("ChatGPT WebSocket connection failed: {e}"))
            })?
    };

    tracing::debug!(
        "ChatGPT WebSocket connected (HTTP {})",
        response.status()
    );

    // Send the translated Responses API request as a text message, then read.
    // Send and receive are sequential so no need to split the stream.
    let request_text = serde_json::to_string(translated_body).map_err(|e| {
        AppError::Internal(format!("Failed to serialize request: {e}"))
    })?;

    ws_stream
        .send(tungstenite::Message::Text(request_text.into()))
        .await
        .map_err(|e| AppError::Internal(format!("Failed to send WS message: {e}")))?;

    if is_streaming {
        // Stream: translate each WebSocket message to SSE and emit
        let translator = ChatgptTranslator;
        let (tx, rx) =
            tokio::sync::mpsc::channel::<Result<bytes::Bytes, std::io::Error>>(32);

        tokio::spawn(async move {
            let mut state = StreamTranslationState::default();

            while let Some(msg) = ws_stream.next().await {
                match msg {
                    Ok(tungstenite::Message::Text(text)) => {
                        if let Ok(data) =
                            serde_json::from_str::<serde_json::Value>(&text)
                        {
                            let event_type = data
                                .get("type")
                                .and_then(|t| t.as_str())
                                .unwrap_or("")
                                .to_string();

                            let event = SseEvent {
                                event_type: Some(event_type.clone()),
                                data: text.to_string(),
                            };

                            if let Some(translated) =
                                translator.translate_stream_event(&event, &mut state)
                            {
                                if tx
                                    .send(Ok(bytes::Bytes::from(translated)))
                                    .await
                                    .is_err()
                                {
                                    return; // client disconnected
                                }
                            }

                            if event_type == "response.completed"
                                || event_type == "response.incomplete"
                            {
                                break;
                            }
                        }
                    }
                    Ok(tungstenite::Message::Close(_)) => break,
                    Err(_) => break,
                    _ => {} // skip ping/pong/binary
                }
            }

            let _ = ws_stream
                .close(None)
                .await;
        });

        let body = Body::from_stream(
            tokio_stream::wrappers::ReceiverStream::new(rx),
        );
        axum::http::Response::builder()
            .status(StatusCode::OK)
            .header("content-type", "text/event-stream")
            .header("cache-control", "no-cache")
            .body(body)
            .map_err(|e| AppError::Internal(format!("Failed to build response: {e}")))
    } else {
        // Non-streaming: collect events until response.completed/incomplete
        let translator = ChatgptTranslator;
        let mut final_response: Option<serde_json::Value> = None;

        while let Some(msg) = ws_stream.next().await {
            match msg {
                Ok(tungstenite::Message::Text(text)) => {
                    if let Ok(data) =
                        serde_json::from_str::<serde_json::Value>(&text)
                    {
                        let etype = data
                            .get("type")
                            .and_then(|t| t.as_str())
                            .unwrap_or("");

                        if etype == "response.completed"
                            || etype == "response.incomplete"
                        {
                            final_response = data.get("response").cloned();
                            break;
                        }
                    }
                }
                Ok(tungstenite::Message::Close(_)) => break,
                Err(_) => break,
                _ => {}
            }
        }

        let _ = ws_stream.close(None).await;

        let resp_json = final_response.unwrap_or_else(|| {
            serde_json::json!({"error": "No response received from ChatGPT"})
        });

        let translated = translator.translate_response(resp_json)?;
        let body_bytes = serde_json::to_vec(&translated).map_err(|e| {
            AppError::Internal(format!("Failed to serialize response: {e}"))
        })?;

        axum::http::Response::builder()
            .status(StatusCode::OK)
            .header("content-type", "application/json")
            .body(Body::from(body_bytes))
            .map_err(|e| AppError::Internal(format!("Failed to build response: {e}")))
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
        assert_eq!(result.body["max_output_tokens"], 1024);
        assert_eq!(result.body["temperature"], 0.7);
        assert_eq!(result.body["store"], false);
        assert!(result.body.get("max_tokens").is_none());
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

        assert!(result.body.get("instructions").is_none());
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
        assert_eq!(result.body["include"], serde_json::json!(["usage"]));
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

        assert_eq!(
            result["choices"][0]["message"]["content"],
            "Let me check."
        );
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
        for line in sse_payload.lines() {
            let trimmed = line.trim();
            if let Some(json_str) = trimmed.strip_prefix("data: ") {
                if json_str == "[DONE]" {
                    continue;
                }
                return serde_json::from_str(json_str).unwrap();
            }
        }
        panic!("No data line found in SSE payload: {sse_payload}");
    }

    #[test]
    fn chatgpt_stream_response_created() {
        let translator = ChatgptTranslator;
        let mut state = StreamTranslationState::default();

        let event = make_event(
            "response.created",
            r#"{"type":"response.created","response":{"id":"resp_abc","model":"gpt-5.2","status":"in_progress"}}"#,
        );

        let result = translator
            .translate_stream_event(&event, &mut state)
            .unwrap();
        let chunk = parse_chunk_json(&result);

        assert_eq!(chunk["id"], "chatcmpl-resp_abc");
        assert_eq!(chunk["object"], "chat.completion.chunk");
        assert_eq!(chunk["model"], "gpt-5.2");
        assert_eq!(chunk["choices"][0]["delta"]["role"], "assistant");
        assert!(chunk["choices"][0]["finish_reason"].is_null());
        assert_eq!(state.id, "resp_abc");
        assert_eq!(state.model, "gpt-5.2");
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

        // Should contain both final chunk and [DONE]
        assert!(result.contains("data: [DONE]"));

        let chunk = parse_chunk_json(&result);
        assert_eq!(chunk["choices"][0]["finish_reason"], "stop");
        assert_eq!(chunk["usage"]["prompt_tokens"], 25);
        assert_eq!(chunk["usage"]["completion_tokens"], 15);
        assert_eq!(chunk["usage"]["total_tokens"], 40);
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
        let chunk = parse_chunk_json(&result);

        assert_eq!(chunk["choices"][0]["finish_reason"], "tool_calls");
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
        assert!(translator
            .translate_stream_event(&event, &mut state)
            .is_none());

        let event2 = make_event(
            "response.content_part.added",
            r#"{"type":"response.content_part.added"}"#,
        );
        assert!(translator
            .translate_stream_event(&event2, &mut state)
            .is_none());
    }

    #[test]
    fn chatgpt_stream_message_item_skipped() {
        let translator = ChatgptTranslator;
        let mut state = StreamTranslationState::default();

        let event = make_event(
            "response.output_item.added",
            r#"{"type":"response.output_item.added","output_index":0,"item":{"type":"message","role":"assistant","content":[]}}"#,
        );
        assert!(translator
            .translate_stream_event(&event, &mut state)
            .is_none());
    }
}

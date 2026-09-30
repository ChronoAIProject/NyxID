//! Lossless, request-specific adapters for the ServicePool chat contract.
//! Catalog protocol selects the adapter; the resolved connection owns its URL.
use super::llm_gateway_service::{AnthropicTranslator, LlmTranslator, StreamTranslationState};
use crate::errors::{AppError, AppResult};
use crate::models::downstream_service::{DownstreamService, InferenceWireProtocol};
use serde_json::{Value, json};

#[derive(Clone)]
pub struct PreparedChat {
    pub path: String,
    pub body: bytes::Bytes,
    pub headers: Vec<(String, String)>,
    pub protocol: InferenceWireProtocol,
    pub streaming: bool,
}

fn invalid(message: &str) -> AppError {
    AppError::BadRequest(message.to_owned())
}

fn only_fields(value: &Value, allowed: &[&str]) -> AppResult<()> {
    let object = value
        .as_object()
        .ok_or_else(|| invalid("AI chat requires JSON objects"))?;
    if let Some(field) = object.keys().find(|key| !allowed.contains(&key.as_str())) {
        return Err(invalid(&format!(
            "AI chat adapter cannot preserve field '{field}'"
        )));
    }
    Ok(())
}

/// Parsed once per ingress; candidates retain only immutable adapter metadata.
#[derive(Clone)]
pub struct ChatRequest {
    #[cfg(test)]
    pub serializations: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    body: Value,
    streaming: bool,
}

#[derive(Clone)]
pub struct ChatPlan {
    pub path: String,
    headers: Vec<(String, String)>,
    protocol: InferenceWireProtocol,
    streaming: bool,
    model: String,
    codex: bool,
    include_usage: bool,
}

impl ChatRequest {
    pub fn parse(method: &http::Method, path: &str, bytes: &[u8]) -> AppResult<Self> {
        if method != http::Method::POST
            || !matches!(
                path.trim_matches('/'),
                "chat/completions" | "v1/chat/completions"
            )
        {
            return Err(invalid("AI chat pools accept only POST chat/completions"));
        }
        let mut body: Value = serde_json::from_slice(bytes)
            .map_err(|_| invalid("AI chat requires a valid JSON body"))?;
        if !body.is_object() {
            return Err(invalid("AI chat requires a JSON object"));
        }
        if body.get("stream_options").is_some_and(Value::is_null) {
            body.as_object_mut()
                .expect("validated object")
                .remove("stream_options");
        }
        if let Some(options) = body.get("stream_options") {
            if !options.is_object() {
                return Err(invalid("stream_options must be an object"));
            }
            if options
                .get("include_usage")
                .is_some_and(|value| !value.is_boolean())
            {
                return Err(invalid("include_usage must be boolean"));
            }
        }
        let messages = body
            .get("messages")
            .and_then(Value::as_array)
            .filter(|v| !v.is_empty())
            .ok_or_else(|| invalid("AI chat requires nonempty messages"))?;
        for message in messages {
            if !matches!(
                message.get("role").and_then(Value::as_str),
                Some("system" | "developer" | "user" | "assistant" | "tool")
            ) {
                return Err(invalid("Unsupported AI chat message role"));
            }
        }
        let streaming = match body.get("stream") {
            None => false,
            Some(Value::Bool(v)) => *v,
            _ => return Err(invalid("stream must be a boolean")),
        };
        Ok(Self {
            body,
            streaming,
            #[cfg(test)]
            serializations: Default::default(),
        })
    }
}

pub fn plan(
    service: &DownstreamService,
    catalog_service_slug: Option<&str>,
    model: &str,
    request: &ChatRequest,
) -> AppResult<ChatPlan> {
    let protocol = service
        .inference
        .as_ref()
        .ok_or_else(|| invalid("AI chat member has no authoritative inference protocol"))?
        .wire_protocol;
    let codex = catalog_service_slug == Some("llm-openai-codex");
    let (operation, headers) = match protocol {
        InferenceWireProtocol::OpenaiCompletions => ("chat/completions", vec![]),
        InferenceWireProtocol::AnthropicMessages => {
            validate_translated_chat(&request.body, true)?;
            (
                "messages",
                vec![("anthropic-version".into(), "2023-06-01".into())],
            )
        }
        InferenceWireProtocol::OpenaiResponses => {
            validate_translated_chat(&request.body, false)?;
            if codex
                && (request.body.get("max_tokens").is_some()
                    || request.body.get("max_completion_tokens").is_some())
            {
                return Err(invalid("Codex transport cannot preserve a token limit"));
            }
            (
                "responses",
                if codex {
                    vec![
                        ("originator".into(), "codex_cli_rs".into()),
                        ("accept".into(), "text/event-stream".into()),
                    ]
                } else {
                    vec![]
                },
            )
        }
    };
    let base =
        url::Url::parse(&service.base_url).map_err(|_| invalid("Invalid member destination"))?;
    let path = if base.path().trim_matches('/').is_empty() {
        format!("v1/{operation}")
    } else {
        operation.to_owned()
    };
    Ok(ChatPlan {
        path,
        headers,
        protocol,
        streaming: request.streaming,
        model: model.into(),
        codex,
        include_usage: catalog_service_slug
            .is_some_and(super::llm_usage_service::service_supports_stream_options_include_usage),
    })
}

impl ChatPlan {
    pub fn prepare(&self, request: &ChatRequest) -> AppResult<PreparedChat> {
        #[cfg(test)]
        request
            .serializations
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let mut body = request.body.clone();
        body["model"] = json!(self.model);
        match self.protocol {
            InferenceWireProtocol::OpenaiCompletions => {
                if self.streaming && self.include_usage {
                    super::llm_usage_service::force_stream_options_include_usage(&mut body);
                }
            }
            InferenceWireProtocol::AnthropicMessages => {
                body = anthropic_request(&body)?;
            }
            InferenceWireProtocol::OpenaiResponses => {
                let max_tokens = body
                    .get("max_completion_tokens")
                    .or_else(|| body.get("max_tokens"))
                    .cloned();
                body = super::chatgpt_translator::ChatgptTranslator
                    .translate_request("chat/completions", &body)?
                    .body;
                body["stream"] = json!(self.streaming || self.codex);
                if let Some(limit) = max_tokens {
                    body["max_output_tokens"] = limit;
                }
            }
        }
        Ok(PreparedChat {
            path: self.path.clone(),
            body: bytes::Bytes::from(
                serde_json::to_vec(&body).map_err(|_| invalid("Invalid AI body"))?,
            ),
            headers: self.headers.clone(),
            protocol: self.protocol,
            streaming: self.streaming,
        })
    }
}

pub fn prepare(
    service: &DownstreamService,
    catalog_service_slug: Option<&str>,
    model: &str,
    method: &http::Method,
    path: &str,
    bytes: &[u8],
) -> AppResult<PreparedChat> {
    let request = ChatRequest::parse(method, path, bytes)?;
    plan(service, catalog_service_slug, model, &request)?.prepare(&request)
}

fn validate_translated_chat(body: &Value, anthropic: bool) -> AppResult<()> {
    only_fields(
        body,
        &[
            "model",
            "messages",
            "stream",
            "stream_options",
            "max_tokens",
            "max_completion_tokens",
            "temperature",
            "top_p",
            "stop",
            "tools",
            "tool_choice",
            "parallel_tool_calls",
            "n",
        ],
    )?;
    if body.get("n").is_some_and(|n| n.as_u64() != Some(1)) {
        return Err(invalid("This member supports only n=1"));
    }
    if body.get("max_tokens").is_some() && body.get("max_completion_tokens").is_some() {
        return Err(invalid("Choose one token limit"));
    }
    if let Some(options) = body.get("stream_options") {
        only_fields(options, &["include_usage"])?;
    }
    if !anthropic && body.get("stop").is_some() {
        return Err(invalid("Responses member cannot preserve stop sequences"));
    }
    if anthropic {
        if body.get("stop").is_some_and(|stop| {
            !stop.is_string()
                && !stop
                    .as_array()
                    .is_some_and(|values| values.iter().all(Value::is_string))
        }) {
            return Err(invalid("stop must be a string or string array"));
        }
        if body
            .get("parallel_tool_calls")
            .is_some_and(|value| !value.is_boolean())
        {
            return Err(invalid("parallel_tool_calls must be boolean"));
        }
    }
    let mut conversation_started = false;
    for message in body["messages"]
        .as_array()
        .ok_or_else(|| invalid("messages must be an array"))?
    {
        only_fields(message, &["role", "content", "tool_calls", "tool_call_id"])?;
        content_blocks(message.get("content").unwrap_or(&Value::Null), anthropic)?;
        let role = message["role"].as_str().unwrap_or("");
        if matches!(role, "system" | "developer") {
            if anthropic
                && message
                    .get("content")
                    .and_then(Value::as_array)
                    .is_some_and(|parts| parts.iter().any(|part| part["type"] != "text"))
            {
                return Err(invalid("System instructions must be text"));
            }
            if conversation_started {
                return Err(invalid(
                    "Translated members require system/developer instructions before the conversation",
                ));
            }
        } else {
            conversation_started = true;
        }
        if !anthropic
            && role != "user"
            && message
                .get("content")
                .and_then(Value::as_array)
                .is_some_and(|parts| parts.iter().any(|part| part["type"] != "text"))
        {
            return Err(invalid(
                "Responses adapter preserves images only in user messages",
            ));
        }
        if role == "tool" && message["tool_call_id"].as_str().is_none_or(str::is_empty) {
            return Err(invalid("Tool result requires tool_call_id"));
        }
        if role != "tool" && message.get("tool_call_id").is_some() {
            return Err(invalid("tool_call_id requires a tool result"));
        }
        if role != "assistant" && message.get("tool_calls").is_some() {
            return Err(invalid("tool_calls require an assistant message"));
        }

        if let Some(calls) = message.get("tool_calls") {
            for call in calls
                .as_array()
                .ok_or_else(|| invalid("tool_calls must be an array"))?
            {
                only_fields(call, &["id", "type", "function"])?;
                if call["type"] != "function" || call["id"].as_str().is_none() {
                    return Err(invalid("Only named function tool calls are supported"));
                }
                only_fields(&call["function"], &["name", "arguments"])?;
                if call["function"]["name"].as_str().is_none_or(str::is_empty)
                    || call["id"].as_str().is_none_or(str::is_empty)
                {
                    return Err(invalid("Function calls require a name and call ID"));
                }
                let arguments = call["function"]["arguments"]
                    .as_str()
                    .ok_or_else(|| invalid("Tool arguments must be a JSON string"))?;
                if !serde_json::from_str::<Value>(arguments).is_ok_and(|v| v.is_object()) {
                    return Err(invalid("Tool arguments must encode an object"));
                }
            }
        }
    }
    if let Some(tools) = body.get("tools") {
        for tool in tools
            .as_array()
            .ok_or_else(|| invalid("tools must be an array"))?
        {
            only_fields(tool, &["type", "function"])?;
            if tool["type"] != "function" {
                return Err(invalid("Only function tools are supported"));
            }
            only_fields(
                &tool["function"],
                &["name", "description", "parameters", "strict"],
            )?;
            if tool["function"]["name"].as_str().is_none_or(str::is_empty) {
                return Err(invalid("Function tools require a name"));
            }
            if tool["function"]
                .get("parameters")
                .is_some_and(|v| !v.is_object())
            {
                return Err(invalid("Tool parameters must be an object"));
            }
            if anthropic && tool["function"].get("strict").is_some() {
                return Err(invalid(
                    "Anthropic cannot preserve strict tool schema enforcement",
                ));
            }
        }
    }
    if let Some(choice) = body.get("tool_choice") {
        if choice.is_object() {
            only_fields(choice, &["type", "function"])?;
            only_fields(&choice["function"], &["name"])?;
            if choice["type"] != "function" || choice["function"]["name"].as_str().is_none() {
                return Err(invalid("Unsupported tool_choice"));
            }
        } else if !matches!(choice.as_str(), Some("auto" | "none" | "required")) {
            return Err(invalid("Unsupported tool_choice"));
        }
    }
    Ok(())
}

fn content_blocks(content: &Value, anthropic: bool) -> AppResult<Vec<Value>> {
    match content {
        Value::Null => Ok(vec![]),
        Value::String(text) => Ok(vec![json!({"type":"text", "text":text})]),
        Value::Array(parts) => parts
            .iter()
            .map(|part| match part["type"].as_str() {
                Some("text") => {
                    only_fields(part, &["type", "text"])?;
                    if !part["text"].is_string() {
                        return Err(invalid("text content must be a string"));
                    }
                    Ok(part.clone())
                }
                Some("image_url") => {
                    only_fields(part, &["type", "image_url"])?;
                    only_fields(&part["image_url"], &["url", "detail"])?;
                    if anthropic && part["image_url"].get("detail").is_some_and(|v| v != "auto") {
                        return Err(invalid("Anthropic cannot preserve image detail selection"));
                    }
                    let url = part["image_url"]["url"]
                        .as_str()
                        .ok_or_else(|| invalid("Image URL must be a string"))?;
                    if !anthropic {
                        return Ok(part.clone());
                    }
                    let source = if let Some(data) = url.strip_prefix("data:") {
                        let (media, encoded) = data
                            .split_once(";base64,")
                            .ok_or_else(|| invalid("Images require base64 data URLs"))?;
                        if !matches!(
                            media,
                            "image/png" | "image/jpeg" | "image/gif" | "image/webp"
                        ) {
                            return Err(invalid("Unsupported image media type"));
                        }
                        json!({"type":"base64", "media_type":media, "data":encoded})
                    } else {
                        let parsed =
                            url::Url::parse(url).map_err(|_| invalid("Invalid image URL"))?;
                        if !matches!(parsed.scheme(), "https" | "http") {
                            return Err(invalid("Unsupported image URL scheme"));
                        }
                        json!({"type":"url", "url":url})
                    };
                    Ok(json!({"type":"image", "source":source}))
                }
                _ => Err(invalid(
                    "Unsupported message content; translation would lose data",
                )),
            })
            .collect(),
        _ => Err(invalid("Unsupported message content")),
    }
}

pub fn anthropic_request(body: &Value) -> AppResult<Value> {
    validate_translated_chat(body, true)?;
    let mut output = json!({"model":body["model"], "max_tokens":body.get("max_completion_tokens").or_else(|| body.get("max_tokens")).cloned().unwrap_or(json!(4096))});
    for field in ["stream", "temperature", "top_p"] {
        if let Some(v) = body.get(field) {
            output[field] = v.clone();
        }
    }
    if let Some(stop) = body.get("stop") {
        output["stop_sequences"] = if stop.is_string() {
            json!([stop])
        } else if stop
            .as_array()
            .is_some_and(|v| v.iter().all(Value::is_string))
        {
            stop.clone()
        } else {
            return Err(invalid("stop must be a string or string array"));
        };
    }
    let mut system = Vec::new();
    let mut messages = Vec::<Value>::new();
    for message in body["messages"]
        .as_array()
        .ok_or_else(|| invalid("messages must be an array"))?
    {
        let role = message["role"].as_str().unwrap_or("");
        let mut content = content_blocks(message.get("content").unwrap_or(&Value::Null), true)?;
        if matches!(role, "system" | "developer") {
            if content.iter().any(|v| v["type"] != "text") {
                return Err(invalid("System instructions must be text"));
            }
            system.extend(content);
            continue;
        }
        if let Some(calls) = message.get("tool_calls").and_then(Value::as_array) {
            if role != "assistant" {
                return Err(invalid("tool_calls require an assistant message"));
            }
            for call in calls {
                content.push(json!({"type":"tool_use", "id":call["id"], "name":call["function"]["name"], "input":serde_json::from_str::<Value>(call["function"]["arguments"].as_str().unwrap()).map_err(|_| invalid("Invalid tool arguments"))?}));
            }
        }
        let role = if role == "tool" {
            let id = message["tool_call_id"]
                .as_str()
                .ok_or_else(|| invalid("Tool result requires tool_call_id"))?;
            content = vec![
                json!({"type":"tool_result", "tool_use_id":id, "content": if message["content"].is_string() { message["content"].clone() } else { json!(content) }}),
            ];
            "user"
        } else {
            role
        };
        if !matches!(role, "user" | "assistant") {
            return Err(invalid("Unsupported message role"));
        }
        if let Some(previous) = messages.last_mut().filter(|v| v["role"] == role) {
            previous["content"].as_array_mut().unwrap().extend(content);
        } else {
            messages.push(json!({"role":role, "content":content}));
        }
    }
    if !system.is_empty() {
        // A single string remains the conventional wire representation.
        output["system"] = json!(
            system
                .iter()
                .map(|v| v["text"].as_str().unwrap())
                .collect::<Vec<_>>()
                .join("\n")
        );
    }
    output["messages"] = json!(messages);
    if let Some(tools) = body.get("tools").and_then(Value::as_array) {
        output["tools"] = Value::Array(tools.iter().map(|t| {
            let f = &t["function"]; let mut v = json!({"name":f["name"], "input_schema":f.get("parameters").cloned().unwrap_or(json!({"type":"object"}))});
            if let Some(d) = f.get("description") { v["description"] = d.clone(); } v
        }).collect());
    }
    if let Some(choice) = body.get("tool_choice") {
        output["tool_choice"] = match choice.as_str() {
            Some("auto") => json!({"type":"auto"}),
            Some("none") => json!({"type":"none"}),
            Some("required") => json!({"type":"any"}),
            _ => json!({"type":"tool", "name":choice["function"]["name"]}),
        };
    }
    if let Some(parallel) = body.get("parallel_tool_calls") {
        let parallel = parallel
            .as_bool()
            .ok_or_else(|| invalid("parallel_tool_calls must be boolean"))?;
        if output.get("tool_choice").is_none() {
            output["tool_choice"] = json!({"type":"auto"});
        }
        output["tool_choice"]["disable_parallel_tool_use"] = json!(!parallel);
    }
    Ok(output)
}

/// Bounded byte framing avoids decoding an incomplete UTF-8 code point. CRLF,
/// LF and CR line endings are normalized only after a complete frame exists.
#[derive(Default)]
pub struct EventDecoder {
    decoder: super::sse_parser::BoundedEventDecoder,
}
impl EventDecoder {
    pub fn push(&mut self, bytes: &[u8]) -> AppResult<Vec<super::sse_parser::SseEvent>> {
        let mut events = Vec::new();
        for &byte in bytes {
            if let Some(event) = self.decoder.push_byte(byte) {
                events.push(event.map_err(|error| {
                    invalid(match error {
                        super::sse_parser::EventDecodeError::TooLarge => {
                            "Upstream SSE event exceeds the buffer limit"
                        }
                        super::sse_parser::EventDecodeError::InvalidUtf8 => {
                            "Upstream SSE contains invalid UTF-8"
                        }
                    })
                })?);
            }
        }
        Ok(events)
    }
}

pub struct ResponseAdapter {
    protocol: InferenceWireProtocol,
    streaming: bool,
    native_streaming: bool,
    decoder: EventDecoder,
    state: StreamTranslationState,
    complete: bool,
    json: Vec<u8>,
}
impl ResponseAdapter {
    pub fn new(prepared: &PreparedChat) -> Self {
        Self {
            protocol: prepared.protocol,
            streaming: prepared.streaming,
            native_streaming: prepared.streaming,
            decoder: Default::default(),
            state: Default::default(),
            complete: false,
            json: Vec::new(),
        }
    }
    pub fn with_native_streaming(mut self, native_streaming: bool) -> Self {
        self.native_streaming = native_streaming;
        self
    }
    pub fn push(&mut self, bytes: &[u8]) -> AppResult<Vec<bytes::Bytes>> {
        if !self.native_streaming {
            if self.streaming {
                return Err(invalid(
                    "Streaming AI request received a non-streaming response",
                ));
            }
            if self.json.len().saturating_add(bytes.len()) > 8 * 1024 * 1024 {
                return Err(invalid("AI response exceeds the buffer limit"));
            }
            self.json.extend_from_slice(bytes);
            return Ok(vec![]);
        }
        let mut output = Vec::new();
        for event in self.decoder.push(bytes)? {
            if self.complete {
                return Err(invalid("Upstream sent data after completion"));
            }
            let data = if event.data == "[DONE]" {
                Value::Null
            } else {
                serde_json::from_str::<Value>(&event.data)
                    .map_err(|_| invalid("Invalid native SSE JSON"))?
            };
            if event.event_type.as_deref() == Some("error")
                || data.get("error").is_some()
                || matches!(
                    event.event_type.as_deref(),
                    Some("response.failed" | "response.error")
                )
            {
                return Err(invalid("Native AI stream failed"));
            }
            let translated = match self.protocol {
                InferenceWireProtocol::OpenaiCompletions => {
                    self.complete = event.data == "[DONE]";
                    Some(format!("data: {}\n\n", event.data))
                }
                InferenceWireProtocol::AnthropicMessages => {
                    self.complete = event.event_type.as_deref() == Some("message_stop");
                    AnthropicTranslator.translate_stream_event(&event, &mut self.state)
                }
                InferenceWireProtocol::OpenaiResponses => {
                    self.complete = matches!(
                        event.event_type.as_deref(),
                        Some("response.completed" | "response.incomplete")
                    );
                    if event.event_type.as_deref() == Some("response.incomplete") {
                        let reason = incomplete_reason(&data["response"])?;
                        Some(format!(
                            "data: {}\n\ndata: [DONE]\n\n",
                            json!({"id":format!("chatcmpl-{}", self.state.id), "object":"chat.completion.chunk", "created":self.state.created, "model":self.state.model, "choices":[{"index":0, "delta":{}, "finish_reason":reason}]})
                        ))
                    } else {
                        super::chatgpt_translator::ChatgptTranslator
                            .translate_stream_event(&event, &mut self.state)
                    }
                }
            };
            if self.streaming {
                if let Some(data) = translated {
                    output.push(bytes::Bytes::from(data));
                }
            } else if self.protocol == InferenceWireProtocol::OpenaiResponses {
                if self.complete {
                    self.json = serde_json::to_vec(&data["response"])
                        .map_err(|_| invalid("Invalid Responses result"))?;
                }
            } else {
                return Err(invalid("Unexpected native streaming response"));
            }
        }
        Ok(output)
    }
    pub fn finish(&mut self) -> AppResult<Vec<bytes::Bytes>> {
        if self.native_streaming {
            if !self.complete || self.decoder.decoder.has_incomplete_event() {
                return Err(invalid("Native AI stream ended before completion"));
            }
            if self.streaming {
                return Ok(vec![]);
            }
        }
        let body: Value = serde_json::from_slice(&self.json)
            .map_err(|_| invalid("Invalid native AI response"))?;
        if body.get("error").is_some() {
            return Err(invalid("Native AI response failed"));
        }
        let translated = match self.protocol {
            InferenceWireProtocol::OpenaiCompletions => {
                if !body["choices"].is_array() {
                    return Err(invalid("Invalid chat response"));
                }
                body
            }
            InferenceWireProtocol::AnthropicMessages => {
                if body["type"] != "message"
                    || !body["content"].is_array()
                    || body["stop_reason"].is_null()
                {
                    return Err(invalid("Incomplete Anthropic response"));
                }
                AnthropicTranslator.translate_response(body)?
            }
            InferenceWireProtocol::OpenaiResponses => {
                if !matches!(body["status"].as_str(), Some("completed" | "incomplete")) {
                    return Err(invalid("Incomplete Responses response"));
                }
                let reason = if body["status"] == "incomplete" {
                    Some(incomplete_reason(&body)?)
                } else {
                    None
                };
                let mut translated =
                    super::chatgpt_translator::ChatgptTranslator.translate_response(body)?;
                if let Some(reason) = reason {
                    translated["choices"][0]["finish_reason"] = json!(reason);
                }
                translated
            }
        };
        Ok(vec![bytes::Bytes::from(
            serde_json::to_vec(&translated).map_err(|_| invalid("Invalid translated response"))?,
        )])
    }
}

fn incomplete_reason(body: &Value) -> AppResult<&'static str> {
    match body
        .pointer("/incomplete_details/reason")
        .and_then(Value::as_str)
    {
        Some("max_output_tokens") => Ok("length"),
        Some("content_filter") => Ok("content_filter"),
        _ => Err(invalid(
            "Native response incomplete for an unsupported reason",
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn service(protocol: InferenceWireProtocol) -> DownstreamService {
        let mut s = crate::models::downstream_service::test_helpers::dummy_service();
        s.base_url = "https://member.invalid/v1".into();
        s.inference = Some(crate::models::downstream_service::ServiceInference {
            wire_protocol: protocol,
            model_list: false,
            realtime: false,
        });
        s
    }
    #[test]
    fn pool_ai_rejects_malformed_stream_options_without_panicking() {
        for value in [json!(42), json!("yes"), json!([]), json!(true)] {
            let body = json!({"messages":[{"role":"user","content":"Hi"}], "stream":true, "stream_options":value});
            assert!(
                prepare(
                    &service(InferenceWireProtocol::OpenaiCompletions),
                    None,
                    "model",
                    &http::Method::POST,
                    "chat/completions",
                    &serde_json::to_vec(&body).unwrap()
                )
                .is_err()
            );
        }
    }
    #[test]
    fn pool_ai_nullable_stream_options_and_ordered_instructions() {
        let input = json!({"stream":true,"stream_options":null,"messages":[{"role":"user","content":"Hi"}]});
        let prepared = prepare(
            &service(InferenceWireProtocol::OpenaiCompletions),
            Some("llm-openai"),
            "model",
            &http::Method::POST,
            "chat/completions",
            &serde_json::to_vec(&input).unwrap(),
        )
        .unwrap();
        let body: Value = serde_json::from_slice(&prepared.body).unwrap();
        assert_eq!(body["stream_options"]["include_usage"], true);
        for anthropic in [false, true] {
            for role in ["system", "developer"] {
                assert!(validate_translated_chat(&json!({"messages":[{"role":"user","content":"Hi"},{"role":role,"content":"later"}]}), anthropic).is_err());
                assert!(validate_translated_chat(&json!({"messages":[{"role":role,"content":"first"},{"role":"user","content":"Hi"}]}), anthropic).is_ok());
            }
        }
    }
    #[test]
    fn pool_ai_responses_rejects_lossy_roles_and_fabricated_tool_identities() {
        for role in ["assistant", "system", "developer", "tool"] {
            let body = json!({"messages":[{"role":role,"content":[{"type":"image_url","image_url":{"url":"https://example.invalid/image.png"}}]}]});
            assert!(validate_translated_chat(&body, false).is_err());
        }
        for message in [
            json!({"role":"tool","content":"result"}),
            json!({"role":"assistant","tool_calls":[{"id":"call1","type":"function","function":{"arguments":"{}"}}]}),
        ] {
            assert!(validate_translated_chat(&json!({"messages":[message]}), false).is_err());
        }
    }
    #[test]
    fn pool_ai_decodes_split_utf8_and_crlf_without_loss() {
        let mut parser = EventDecoder::default();
        let mut events = Vec::new();
        for byte in "data: {\"text\":\"你好\"}\r\n\r\n".as_bytes() {
            events.extend(parser.push(&[*byte]).unwrap());
        }
        assert_eq!(events.len(), 1);
        assert_eq!(
            serde_json::from_str::<Value>(&events[0].data).unwrap()["text"],
            "你好"
        );
    }
    #[test]
    fn pool_ai_mixed_line_endings_dispatch_each_frame_and_final_completion_byte_by_byte() {
        for separator in ["\n\r", "\r\n\r", "\n\r\n"] {
            let native = format!(
                "data: {{\"choices\":[{{\"delta\":{{\"content\":\"你好\"}}}}]}}{separator}data: [DONE]{separator}"
            );
            let mut adapter =
                ResponseAdapter::new(&prepared(InferenceWireProtocol::OpenaiCompletions, true));
            let mut output = Vec::new();
            for byte in native.bytes() {
                output.extend(adapter.push(&[byte]).unwrap());
            }
            assert_eq!(output.len(), 2);
            assert!(std::str::from_utf8(&output[0]).unwrap().contains("你好"));
            assert_eq!(&output[1][..], b"data: [DONE]\n\n");
            assert!(adapter.finish().is_ok());
        }
    }

    #[test]
    fn pool_ai_incomplete_preserves_reason_and_rejects_unknown() {
        assert_eq!(
            incomplete_reason(&json!({"incomplete_details":{"reason":"content_filter"}})).unwrap(),
            "content_filter"
        );
        assert_eq!(
            incomplete_reason(&json!({"incomplete_details":{"reason":"max_output_tokens"}}))
                .unwrap(),
            "length"
        );
        assert!(incomplete_reason(&json!({"incomplete_details":{"reason":"other"}})).is_err());
    }

    fn prepared(protocol: InferenceWireProtocol, streaming: bool) -> PreparedChat {
        PreparedChat {
            path: "responses".into(),
            body: bytes::Bytes::new(),
            headers: vec![],
            protocol,
            streaming,
        }
    }
    #[test]
    fn pool_ai_responses_tool_call_correlation_survives_json_stream_and_next_request() {
        let item = json!({"type":"function_call", "id":"fc_item", "call_id":"call_correlation", "name":"lookup", "arguments":"{}"});
        let mut adapter =
            ResponseAdapter::new(&prepared(InferenceWireProtocol::OpenaiResponses, false));
        adapter
            .push(
                &serde_json::to_vec(
                    &json!({"id":"r1", "status":"completed", "output":[item.clone()]}),
                )
                .unwrap(),
            )
            .unwrap();
        let message: Value = serde_json::from_slice(&adapter.finish().unwrap()[0]).unwrap();
        let call = &message["choices"][0]["message"]["tool_calls"][0];
        assert_eq!(call["id"], "call_correlation");
        let next = json!({"messages":[message["choices"][0]["message"].clone(), {"role":"tool", "tool_call_id":call["id"], "content":"result"}]});
        let request = prepare(
            &service(InferenceWireProtocol::OpenaiResponses),
            None,
            "model",
            &http::Method::POST,
            "chat/completions",
            &serde_json::to_vec(&next).unwrap(),
        )
        .unwrap();
        let native: Value = serde_json::from_slice(&request.body).unwrap();
        assert_eq!(native["input"][1]["call_id"], "call_correlation");
        let mut adapter =
            ResponseAdapter::new(&prepared(InferenceWireProtocol::OpenaiResponses, true));
        let chunks = adapter
            .push(
                format!(
                    "data: {}\n\n",
                    json!({"type":"response.output_item.added", "output_index":0, "item":item})
                )
                .as_bytes(),
            )
            .unwrap();
        assert!(
            std::str::from_utf8(&chunks[0])
                .unwrap()
                .contains("call_correlation")
        );
        assert!(!std::str::from_utf8(&chunks[0]).unwrap().contains("fc_item"));
    }
    #[test]
    fn pool_ai_data_only_native_completion_and_errors_are_consistent() {
        for (protocol, done, failed) in [
            (
                InferenceWireProtocol::AnthropicMessages,
                json!({"type":"message_stop"}),
                json!({"type":"error","error":{"type":"overloaded_error"}}),
            ),
            (
                InferenceWireProtocol::OpenaiResponses,
                json!({"type":"response.completed", "response":{"status":"completed"}}),
                json!({"type":"response.failed"}),
            ),
        ] {
            let mut adapter = ResponseAdapter::new(&prepared(protocol, true));
            let chunks = adapter
                .push(format!("data: {done}\r\n\r\n").as_bytes())
                .unwrap();
            assert!(
                chunks
                    .iter()
                    .any(|chunk| std::str::from_utf8(chunk).unwrap().contains("[DONE]"))
            );
            assert!(adapter.finish().is_ok());
            let mut adapter = ResponseAdapter::new(&prepared(protocol, true));
            assert!(
                adapter
                    .push(format!("data: {failed}\n\n").as_bytes())
                    .is_err()
            );
        }
    }
    #[test]
    fn pool_ai_responses_refusals_are_preserved_in_json_and_stream() {
        let mut adapter =
            ResponseAdapter::new(&prepared(InferenceWireProtocol::OpenaiResponses, false));
        adapter.push(&serde_json::to_vec(&json!({"id":"r", "status":"completed", "output":[{"type":"message","content":[{"type":"refusal","refusal":"Cannot comply"}]}]})).unwrap()).unwrap();
        let response: Value = serde_json::from_slice(&adapter.finish().unwrap()[0]).unwrap();
        assert_eq!(
            response["choices"][0]["message"]["refusal"],
            "Cannot comply"
        );
        let mut adapter =
            ResponseAdapter::new(&prepared(InferenceWireProtocol::OpenaiResponses, true));
        let chunks = adapter
            .push(b"data: {\"type\":\"response.refusal.delta\",\"delta\":\"Cannot comply\"}\n\n")
            .unwrap();
        assert!(
            std::str::from_utf8(&chunks[0])
                .unwrap()
                .contains("Cannot comply")
        );
    }
}

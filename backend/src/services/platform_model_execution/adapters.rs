//! Closed, stateless text/function adapters. No arbitrary provider fields cross
//! this boundary; native evidence is extracted before public serialization.
use crate::{
    errors::{AppError, AppResult},
    models::{
        platform_model::*,
        platform_model_tariff::{ModelTariff, UsageFamily},
    },
    services::{
        billing::model_usage::{self, Evidence},
        llm_gateway_service::LlmTranslator,
    },
};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Native {
    Gemini,
    Claude,
    Responses,
    GptOss,
}
impl Native {
    pub fn family(self) -> UsageFamily {
        match self {
            Self::Gemini => UsageFamily::Gemini,
            Self::Claude => UsageFamily::Claude,
            Self::Responses => UsageFamily::Openai,
            Self::GptOss => UsageFamily::OpenMaas,
        }
    }
    pub fn fixture(self) -> &'static str {
        match self {
            Self::Gemini => include_str!("fixtures/gemini.json"),
            Self::Claude => include_str!("fixtures/claude.json"),
            Self::Responses => include_str!("fixtures/responses.json"),
            Self::GptOss => include_str!("fixtures/gpt-oss.json"),
        }
    }
    pub fn digest(self) -> String {
        format!("{:x}", Sha256::digest(self.fixture().as_bytes()))
    }
}
pub(super) fn unsupported() -> AppError {
    AppError::BadRequest("unsupported_model_feature".into())
}
pub(super) fn invalid_response() -> AppError {
    AppError::BillingProviderUnavailable("model_usage_pending".into())
}

/// Only documented pre-inference error envelopes certify zero consumption.
/// A status alone (including 429 from an intermediary) is not such evidence.
pub(super) fn zero_consumption(native: Native, status: u16, body: &[u8]) -> bool {
    let Ok(value) = serde_json::from_slice::<Value>(body) else {
        return false;
    };
    let Some(root) = value.as_object() else {
        return false;
    };
    if root
        .keys()
        .any(|key| !matches!(key.as_str(), "error" | "type"))
        || root.get("type").is_some_and(|kind| kind != "error")
    {
        return false;
    }
    let Some(error) = value["error"].as_object() else {
        return false;
    };
    if error.keys().any(|key| {
        !matches!(
            key.as_str(),
            "code" | "status" | "message" | "details" | "type" | "param"
        )
    }) {
        return false;
    }
    let kind = value["error"]["type"].as_str();
    let code = value["error"]["code"].as_str();
    let google = value["error"]["code"].as_u64() == Some(u64::from(status))
        && matches!(
            (status, value["error"]["status"].as_str()),
            (401, Some("UNAUTHENTICATED"))
                | (403, Some("PERMISSION_DENIED"))
                | (429, Some("RESOURCE_EXHAUSTED"))
        );
    match native {
        Native::Gemini | Native::GptOss => google,
        Native::Claude => {
            google
                || matches!(
                    (status, kind),
                    (401, Some("authentication_error"))
                        | (403, Some("permission_error"))
                        | (429, Some("rate_limit_error"))
                )
        }
        Native::Responses => matches!(
            (status, code),
            (401, Some("invalid_api_key"))
                | (403, Some("permission_denied"))
                | (429, Some("rate_limit_exceeded" | "insufficient_quota"))
        ),
    }
}
fn fields(v: &Value, allowed: &[&str]) -> AppResult<()> {
    if v.as_object()
        .is_none_or(|o| o.keys().any(|k| !allowed.contains(&k.as_str())))
    {
        return Err(unsupported());
    }
    Ok(())
}

#[derive(Clone)]
pub(crate) struct Request {
    pub body: Value,
    pub streaming: bool,
    pub max_output: i64,
}
impl Request {
    pub fn parse(method: &http::Method, path: &str, body: Value) -> AppResult<Self> {
        if method != http::Method::POST
            || !matches!(
                path.trim_matches('/'),
                "chat/completions" | "v1/chat/completions"
            )
        {
            return Err(unsupported());
        }
        fields(
            &body,
            &[
                "model",
                "messages",
                "stream",
                "stream_options",
                "max_tokens",
                "max_completion_tokens",
                "temperature",
                "top_p",
                "tools",
                "tool_choice",
                "parallel_tool_calls",
                "n",
            ],
        )?;
        if body.get("stream").is_some_and(|v| !v.is_boolean()) {
            return Err(unsupported());
        }
        // Shared pool validation rejects lossy/unknown message, tool and option fields.
        crate::services::pool_ai_service::ChatRequest::parse(
            method,
            path,
            &serde_json::to_vec(&body).map_err(|_| unsupported())?,
        )?;
        crate::services::pool_ai_service::validate_translated_chat(&body, false)?;
        let max_output = body
            .get("max_completion_tokens")
            .or_else(|| body.get("max_tokens"))
            .and_then(Value::as_i64)
            .filter(|n| *n > 0)
            .ok_or_else(unsupported)?;
        for key in ["temperature", "top_p"] {
            if let Some(v) = body.get(key)
                && v.as_f64().is_none_or(|n| {
                    !n.is_finite() || n < 0.0 || n > if key == "top_p" { 1.0 } else { 2.0 }
                })
            {
                return Err(unsupported());
            }
        }
        let mut calls = std::collections::HashMap::new();
        for message in body["messages"].as_array().ok_or_else(unsupported)? {
            // Media, provider-hosted files, thinking signatures and continuations
            // need their own cost/lifecycle adapters; never silently drop them.
            let content = &message["content"];
            if !(content.is_string()
                || content.is_null()
                || content.as_array().is_some_and(|p| {
                    p.iter().all(|p| {
                        fields(p, &["type", "text"]).is_ok()
                            && p["type"] == "text"
                            && p["text"].is_string()
                    })
                }))
            {
                return Err(unsupported());
            }
            if let Some(tc) = message["tool_calls"].as_array() {
                for call in tc {
                    let id = call["id"].as_str().ok_or_else(unsupported)?;
                    if calls
                        .insert(
                            id.to_owned(),
                            call["function"]["name"]
                                .as_str()
                                .ok_or_else(unsupported)?
                                .to_owned(),
                        )
                        .is_some()
                    {
                        return Err(unsupported());
                    }
                }
            }
            if message["role"] == "tool"
                && !calls.contains_key(message["tool_call_id"].as_str().ok_or_else(unsupported)?)
            {
                return Err(unsupported());
            }
        }
        Ok(Self {
            streaming: body["stream"] == true,
            body,
            max_output,
        })
    }
    pub fn validate_bounds(&self, model: &PlatformModel, tariff: &ModelTariff) -> AppResult<()> {
        let b = &tariff.bounds;
        let size = serde_json::to_vec(&self.body)
            .map_err(|_| unsupported())?
            .len();
        if !model
            .capabilities
            .protocols
            .contains(&CallerProtocol::OpenaiChat)
            || self.streaming && model.capabilities.streaming != Some(true)
            || self.body.get("tools").is_some() && model.capabilities.tools != Some(true)
            || model
                .capabilities
                .context_tokens
                .is_none_or(|n| n == 0 || i64::from(n) > b.max_input_tokens)
            || model
                .capabilities
                .output_tokens
                .is_none_or(|n| self.max_output > i64::from(n))
            || self.max_output > b.max_output_tokens
            || size as i64 > b.max_request_bytes
            || size > 1024 * 1024
            || b.max_response_bytes > 8 * 1024 * 1024
            || !tariff.variant.is_empty()
            || !tariff.contract.tokens
            || tariff.contract.images
            || !tariff.contract.input_modalities.is_empty()
            || !tariff.contract.output_modalities.is_empty()
        {
            return Err(unsupported());
        }
        // Reserve the FULL verified native context bound, not an input-byte estimate.
        // Native endpoints enforce their context maximum; max output is sent explicitly.
        Ok(())
    }
}

pub(super) struct Prepared {
    pub native: Native,
    pub url: String,
    pub body: Value,
}
fn vertex_base(project: &str, location: &str) -> String {
    let host = if location == "global" {
        "aiplatform.googleapis.com".into()
    } else {
        format!("{location}-aiplatform.googleapis.com")
    };
    format!("https://{host}/v1/projects/{project}/locations/{location}")
}
fn text(content: &Value) -> AppResult<String> {
    if let Some(t) = content.as_str() {
        return Ok(t.into());
    }
    if content.is_null() {
        return Ok(String::new());
    }
    let mut result = String::new();
    for part in content.as_array().ok_or_else(unsupported)? {
        result.push_str(part["text"].as_str().ok_or_else(unsupported)?);
    }
    Ok(result)
}
fn gemini_request(request: &Request) -> AppResult<Value> {
    if request.body.get("parallel_tool_calls").is_some() {
        return Err(unsupported());
    }
    let mut system = vec![];
    let mut contents = vec![];
    let mut names = std::collections::HashMap::new();
    for message in request.body["messages"]
        .as_array()
        .ok_or_else(unsupported)?
    {
        let role = message["role"].as_str().ok_or_else(unsupported)?;
        let t = text(&message["content"])?;
        if matches!(role, "system" | "developer") {
            system.push(json!({"text":t}));
            continue;
        }
        let mut parts = vec![];
        if role == "tool" {
            let name = names
                .get(message["tool_call_id"].as_str().ok_or_else(unsupported)?)
                .ok_or_else(unsupported)?;
            parts.push(json!({"functionResponse":{"name":name,"response":{"result":t}}}));
        } else if !t.is_empty() {
            parts.push(json!({"text":t}));
        }
        if let Some(calls) = message["tool_calls"].as_array() {
            for call in calls {
                let name = call["function"]["name"].as_str().ok_or_else(unsupported)?;
                names.insert(call["id"].as_str().ok_or_else(unsupported)?, name);
                let args: Value = serde_json::from_str(
                    call["function"]["arguments"]
                        .as_str()
                        .ok_or_else(unsupported)?,
                )
                .map_err(|_| unsupported())?;
                parts.push(json!({"functionCall":{"name":name,"args":args}}));
            }
        }
        if parts.is_empty() {
            return Err(unsupported());
        }
        contents.push(
            json!({"role": if role == "assistant" { "model" } else { "user" },"parts":parts}),
        );
    }
    let mut generation = json!({"maxOutputTokens":request.max_output,"candidateCount":1});
    if let Some(t) = request.body.get("temperature") {
        generation["temperature"] = t.clone();
    }
    if let Some(t) = request.body.get("top_p") {
        generation["topP"] = t.clone();
    }
    let mut body = json!({"contents":contents,"generationConfig":generation});
    if !system.is_empty() {
        body["systemInstruction"] = json!({"parts":system});
    }
    if let Some(tools) = request.body["tools"].as_array() {
        let mut declarations = vec![];
        for tool in tools {
            let f = &tool["function"];
            if f.get("strict").is_some() {
                return Err(unsupported());
            }
            let mut declaration = json!({"name":f["name"],"parametersJsonSchema":f.get("parameters").cloned().unwrap_or(json!({"type":"object"}))});
            if let Some(d) = f.get("description") {
                declaration["description"] = d.clone();
            }
            declarations.push(declaration);
        }
        body["tools"] = json!([{"functionDeclarations":declarations}]);
    }
    if let Some(choice) = request.body.get("tool_choice") {
        body["toolConfig"] = json!({"functionCallingConfig":match choice.as_str() {
            Some("auto") => json!({"mode":"AUTO"}), Some("none") => json!({"mode":"NONE"}), Some("required") => json!({"mode":"ANY"}),
            _ => json!({"mode":"ANY","allowedFunctionNames":[choice["function"]["name"]]}),
        }});
    }
    Ok(body)
}
pub(super) fn prepare(
    model: &PlatformModel,
    route: &ModelRoute,
    tariff: &ModelTariff,
    request: &Request,
) -> AppResult<Prepared> {
    crate::services::platform_model_service::validate_route(model, route)?;
    request.validate_bounds(model, tariff)?;
    let (native, url, native_model) = match &route.target {
        RouteTarget::VertexPublisher {
            publisher,
            project,
            location,
            native_model,
        } => {
            let (native, publisher, operation) = match publisher {
                VertexPublisher::Google => (
                    Native::Gemini,
                    "google",
                    if request.streaming {
                        "streamGenerateContent?alt=sse"
                    } else {
                        "generateContent"
                    },
                ),
                VertexPublisher::Anthropic => (
                    Native::Claude,
                    "anthropic",
                    if request.streaming {
                        "streamRawPredict"
                    } else {
                        "rawPredict"
                    },
                ),
            };
            (
                native,
                format!(
                    "{}/publishers/{publisher}/models/{native_model}:{operation}",
                    vertex_base(project, location)
                ),
                native_model.clone(),
            )
        }
        RouteTarget::VertexOpenapiMaas {
            project,
            location,
            publisher,
            native_model,
        } => {
            // This is the certified MaaS mapping, not a generic compatibility guess.
            if publisher != "openai"
                || native_model != "gpt-oss-120b-maas"
                || tariff.contract.cache_read
                || tariff.contract.split_reasoning
                || !tariff.contract.cache_write_ttls.is_empty()
                || request.body.get("parallel_tool_calls").is_some()
            {
                return Err(unsupported());
            }
            (
                Native::GptOss,
                format!(
                    "{}/endpoints/openapi/chat/completions",
                    vertex_base(project, location)
                ),
                format!("{publisher}/{native_model}"),
            )
        }
        RouteTarget::DirectProvider {
            provider: DirectProvider::Openai,
            native_model,
        } => (
            Native::Responses,
            "https://api.openai.com/v1/responses".into(),
            native_model.clone(),
        ),
        RouteTarget::DirectProvider {
            provider: DirectProvider::Anthropic,
            native_model,
        } => (
            Native::Claude,
            "https://api.anthropic.com/v1/messages".into(),
            native_model.clone(),
        ),
    };
    if native.family() != tariff.contract.family
        || tariff.contract.fixture_sha256 != native.digest()
    {
        return Err(unsupported());
    }
    let mut body = match native {
        Native::Gemini => gemini_request(request)?,
        Native::Claude => {
            crate::services::pool_ai_service::validate_translated_chat(&request.body, true)?;
            let mut body = crate::services::pool_ai_service::anthropic_request(&request.body)?;
            body["max_tokens"] = json!(request.max_output);
            if matches!(route.target, RouteTarget::VertexPublisher { .. }) {
                body.as_object_mut()
                    .ok_or_else(unsupported)?
                    .remove("model");
                body["anthropic_version"] = json!("vertex-2023-10-16");
            } else {
                body["model"] = json!(native_model);
            }
            body
        }
        Native::Responses => {
            // The Responses API documents a minimum of 16 output tokens.
            // Refuse locally before admission rather than create an uncertain
            // hold for a predictably invalid provider request.
            if request.max_output < 16 {
                return Err(unsupported());
            }
            let mut body = crate::services::chatgpt_translator::ChatgptTranslator
                .translate_request("chat/completions", &request.body)?
                .body;
            body["model"] = json!(native_model);
            body["max_output_tokens"] = json!(request.max_output);
            body["store"] = json!(false);
            body
        }
        Native::GptOss => {
            let mut body = request.body.clone();
            body["model"] = json!(native_model);
            body.as_object_mut()
                .ok_or_else(unsupported)?
                .remove("max_completion_tokens");
            body["max_tokens"] = json!(request.max_output);
            // Google's documented SSE contract provides terminal usage without
            // the OpenAI-only include_usage option. Never infer compatibility.
            body.as_object_mut().unwrap().remove("stream_options");
            body
        }
    };
    if native != Native::Gemini {
        body["stream"] = json!(request.streaming);
    }
    if serde_json::to_vec(&body).map_err(|_| unsupported())?.len() as i64
        > tariff.bounds.max_request_bytes
    {
        return Err(unsupported());
    }
    Ok(Prepared { native, url, body })
}

pub(super) fn evidence(native: Native, usage: &Value) -> AppResult<Evidence> {
    if !usage.is_object() {
        return Err(invalid_response());
    }
    let mut e = if native == Native::GptOss {
        // The documented MaaS fixture certifies aggregate prompt/completion
        // counts only. Separate cache/reasoning prices require another mapping.
        fields(
            usage,
            &["prompt_tokens", "completion_tokens", "total_tokens"],
        )
        .map_err(|_| invalid_response())?;
        fn n(v: &Value, k: &str) -> AppResult<i64> {
            v[k].as_i64()
                .filter(|n| *n >= 0)
                .ok_or_else(invalid_response)
        }
        Evidence {
            input: Some(n(usage, "prompt_tokens")?),
            output: Some(n(usage, "completion_tokens")?),
            total: Some(n(usage, "total_tokens")?),
            ..Default::default()
        }
    } else {
        model_usage::parse_usage(native.family(), usage)?
    };
    // Missing priced counters stay unknown. Only costs impossible for this
    // stateless request subset may be established as zero without a counter.
    // No adapter accepts cache-creation directives or server-hosted tools.
    if e.cache_write.is_none() {
        e.cache_write = Some(0);
    }
    if native == Native::Gemini && e.tool_input.is_none() {
        e.tool_input = Some(0);
    }
    e.requests = Some(1);
    Ok(e)
}

/// Public transport is always an allowlisted Chat response, never native JSON.
pub(super) struct Output {
    pub text: String,
    pub calls: Vec<Value>,
    pub finish: &'static str,
}
pub(super) fn output(native: Native, body: &Value, request_id: &str) -> AppResult<Output> {
    let mut out = Output {
        text: String::new(),
        calls: vec![],
        finish: "stop",
    };
    let mut call = |name: &Value, args: Value| -> AppResult<()> {
        let name = name
            .as_str()
            .filter(|s| !s.is_empty())
            .ok_or_else(invalid_response)?;
        out.calls.push(json!({
            "id": format!("call_{request_id}_{}", out.calls.len()),
            "type": "function",
            "function": {
                "name": name,
                "arguments": serde_json::to_string(&args).map_err(|_| invalid_response())?,
            },
        }));
        Ok(())
    };
    match native {
        Native::Claude => {
            if body["type"] != "message" {
                return Err(invalid_response());
            }
            out.finish = match body["stop_reason"].as_str() {
                Some("end_turn" | "stop_sequence") => "stop",
                Some("tool_use") => "tool_calls",
                Some("max_tokens") => "length",
                _ => return Err(invalid_response()),
            };
            for p in body["content"].as_array().ok_or_else(invalid_response)? {
                match p["type"].as_str() {
                    Some("text") => out
                        .text
                        .push_str(p["text"].as_str().ok_or_else(invalid_response)?),
                    Some("tool_use") if p["input"].is_object() => {
                        call(&p["name"], p["input"].clone())?
                    }
                    _ => return Err(invalid_response()),
                }
            }
        }
        Native::Responses => {
            out.finish = match body["status"].as_str() {
                Some("completed") => "stop",
                Some("incomplete")
                    if body.pointer("/incomplete_details/reason")
                        == Some(&json!("max_output_tokens")) =>
                {
                    "length"
                }
                _ => return Err(invalid_response()),
            };
            for item in body["output"].as_array().ok_or_else(invalid_response)? {
                match item["type"].as_str() {
                    Some("message") => {
                        for p in item["content"].as_array().ok_or_else(invalid_response)? {
                            if p["type"] != "output_text" {
                                return Err(invalid_response());
                            }
                            out.text
                                .push_str(p["text"].as_str().ok_or_else(invalid_response)?);
                        }
                    }
                    Some("function_call") => {
                        let args: Value = serde_json::from_str(
                            item["arguments"].as_str().ok_or_else(invalid_response)?,
                        )
                        .map_err(|_| invalid_response())?;
                        call(&item["name"], args)?;
                    }
                    Some("reasoning") => { /* Never emit provider continuation/signature state. */ }
                    _ => return Err(invalid_response()),
                }
            }
        }
        Native::Gemini => {
            let candidates = body["candidates"]
                .as_array()
                .filter(|v| v.len() == 1)
                .ok_or_else(invalid_response)?;
            let c = &candidates[0];
            out.finish = match c["finishReason"].as_str() {
                Some("STOP") => "stop",
                Some("MAX_TOKENS") => "length",
                Some("SAFETY" | "RECITATION") => "content_filter",
                _ => return Err(invalid_response()),
            };
            for p in c["content"]["parts"]
                .as_array()
                .ok_or_else(invalid_response)?
            {
                if p.get("thoughtSignature").is_some() {
                    return Err(invalid_response());
                }
                if p["thought"] == true {
                    continue;
                }
                if let Some(t) = p["text"].as_str() {
                    out.text.push_str(t);
                } else if let Some(f) = p.get("functionCall") {
                    call(&f["name"], f["args"].clone())?;
                } else {
                    return Err(invalid_response());
                }
            }
        }
        Native::GptOss => {
            let choices = body["choices"]
                .as_array()
                .filter(|v| v.len() == 1)
                .ok_or_else(invalid_response)?;
            out.finish = match choices[0]["finish_reason"].as_str() {
                Some("stop") => "stop",
                Some("length") => "length",
                Some("tool_calls") => "tool_calls",
                Some("content_filter") => "content_filter",
                _ => return Err(invalid_response()),
            };
            let message = &choices[0]["message"];
            if let Some(t) = message["content"].as_str() {
                out.text.push_str(t);
            }
            if let Some(calls) = message["tool_calls"].as_array() {
                for c in calls {
                    if c["type"] != "function" {
                        return Err(invalid_response());
                    }
                    let args: Value = serde_json::from_str(
                        c["function"]["arguments"]
                            .as_str()
                            .ok_or_else(invalid_response)?,
                    )
                    .map_err(|_| invalid_response())?;
                    call(&c["function"]["name"], args)?;
                }
            }
        }
    }
    if !out.calls.is_empty() && out.finish == "stop" {
        out.finish = "tool_calls";
    }
    Ok(out)
}
pub(super) fn public_json(out: Output, model: &str, request_id: &str, usage: Value) -> Value {
    let mut message = json!({"role":"assistant","content":out.text});
    if !out.calls.is_empty() {
        message["tool_calls"] = json!(out.calls);
    }
    json!({
        "id": format!("chatcmpl-{request_id}"),
        "object": "chat.completion",
        "created": chrono::Utc::now().timestamp(),
        "model": model,
        "choices": [{"index": 0, "message": message, "finish_reason": out.finish}],
        "usage": usage,
    })
}
pub(super) fn chunk(delta: Value, finish: Option<&str>, model: &str, id: &str) -> bytes::Bytes {
    bytes::Bytes::from(format!(
        "data: {}\n\n",
        json!({
            "id": format!("chatcmpl-{id}"),
            "object": "chat.completion.chunk",
            "created": chrono::Utc::now().timestamp(),
            "model": model,
            "choices": [{"index": 0, "delta": delta, "finish_reason": finish}],
        })
    ))
}

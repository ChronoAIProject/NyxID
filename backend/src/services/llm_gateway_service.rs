use futures::TryStreamExt;
use mongodb::bson::doc;
use serde::Serialize;

use crate::errors::{AppError, AppResult};
use crate::models::downstream_service::{DownstreamService, COLLECTION_NAME as DOWNSTREAM_SERVICES};
use crate::models::provider_config::{ProviderConfig, COLLECTION_NAME as PROVIDER_CONFIGS};
use crate::models::user_provider_token::{
    UserProviderToken, COLLECTION_NAME as USER_PROVIDER_TOKENS,
};

// ---------------------------------------------------------------------------
// Response types
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
pub struct LlmProviderStatus {
    pub provider_slug: String,
    pub provider_name: String,
    pub status: String,
    pub proxy_url: String,
}

#[derive(Debug, Serialize)]
pub struct LlmStatusResponse {
    pub providers: Vec<LlmProviderStatus>,
    pub gateway_url: String,
    pub supported_models: Vec<String>,
}

// ---------------------------------------------------------------------------
// Slug resolution (Phase 2)
// ---------------------------------------------------------------------------

/// Resolve a downstream service by provider slug.
/// Returns `(DownstreamService, ProviderConfig)` or an error.
pub async fn resolve_llm_service_by_slug(
    db: &mongodb::Database,
    provider_slug: &str,
) -> AppResult<(DownstreamService, ProviderConfig)> {
    let provider = db
        .collection::<ProviderConfig>(PROVIDER_CONFIGS)
        .find_one(doc! { "slug": provider_slug, "is_active": true })
        .await?
        .ok_or_else(|| {
            AppError::NotFound(format!("LLM provider '{provider_slug}' not found"))
        })?;

    let service = db
        .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .find_one(doc! { "provider_config_id": &provider.id, "is_active": true })
        .await?
        .ok_or_else(|| {
            AppError::Internal(format!(
                "LLM service not configured for provider '{provider_slug}'"
            ))
        })?;

    Ok((service, provider))
}

/// Get the LLM gateway status for a user.
pub async fn get_llm_status(
    db: &mongodb::Database,
    user_id: &str,
    base_url: &str,
) -> AppResult<LlmStatusResponse> {
    // Get all auto-seeded downstream services
    let services: Vec<DownstreamService> = db
        .collection::<DownstreamService>(DOWNSTREAM_SERVICES)
        .find(doc! { "provider_config_id": { "$ne": null }, "is_active": true })
        .await?
        .try_collect()
        .await?;

    // Get all user tokens (non-revoked)
    let tokens: Vec<UserProviderToken> = db
        .collection::<UserProviderToken>(USER_PROVIDER_TOKENS)
        .find(doc! { "user_id": user_id, "status": { "$in": ["active", "expired"] } })
        .await?
        .try_collect()
        .await?;

    // Get all providers in a single query
    let provider_ids: Vec<&str> = services
        .iter()
        .filter_map(|s| s.provider_config_id.as_deref())
        .collect();

    let providers: Vec<ProviderConfig> = if provider_ids.is_empty() {
        vec![]
    } else {
        db.collection::<ProviderConfig>(PROVIDER_CONFIGS)
            .find(doc! { "_id": { "$in": &provider_ids } })
            .await?
            .try_collect()
            .await?
    };

    let base = base_url.trim_end_matches('/');
    let mut statuses = Vec::new();

    for service in &services {
        let provider_config_id = match &service.provider_config_id {
            Some(id) => id,
            None => continue,
        };

        let provider = match providers.iter().find(|p| &p.id == provider_config_id) {
            Some(p) => p,
            None => continue,
        };

        let user_token = tokens
            .iter()
            .find(|t| t.provider_config_id == *provider_config_id);

        let status = match user_token {
            Some(t) if t.status == "active" => "ready",
            Some(t) if t.status == "expired" => "expired",
            _ => "not_connected",
        };

        statuses.push(LlmProviderStatus {
            provider_slug: provider.slug.clone(),
            provider_name: provider.name.clone(),
            status: status.to_string(),
            proxy_url: format!("{base}/api/v1/llm/{}/v1", provider.slug),
        });
    }

    Ok(LlmStatusResponse {
        providers: statuses,
        gateway_url: format!("{base}/api/v1/llm/gateway/v1"),
        // L-3: This list is derived from resolve_provider_for_model() above.
        // Update both when adding new providers or model families.
        supported_models: vec![
            "gpt-*".to_string(),
            "o1-*".to_string(),
            "o3-*".to_string(),
            "o4-*".to_string(),
            "chatgpt-*".to_string(),
            "claude-*".to_string(),
            "gemini-*".to_string(),
            "mistral-*".to_string(),
            "codestral-*".to_string(),
            "pixtral-*".to_string(),
            "ministral-*".to_string(),
            "open-mistral-*".to_string(),
            "devstral-*".to_string(),
            "magistral-*".to_string(),
            "command-*".to_string(),
            "embed-*".to_string(),
            "rerank-*".to_string(),
            "deepseek-*".to_string(),
        ],
    })
}

// ---------------------------------------------------------------------------
// Model-to-provider resolution (Phase 3)
// ---------------------------------------------------------------------------

/// Determine which provider to route to based on the model name.
///
/// L-5: This uses hardcoded prefix matching. Adding a new provider requires
/// modifying this function. A future improvement could store model prefixes
/// in the database alongside provider configs for a data-driven approach.
pub fn resolve_provider_for_model(model: &str) -> Option<&'static str> {
    let model_lower = model.to_lowercase();

    if model_lower.starts_with("gpt-")
        || model_lower.starts_with("o1-")
        || model_lower.starts_with("o3-")
        || model_lower.starts_with("o4-")
        || model_lower.starts_with("chatgpt-")
    {
        Some("openai")
    } else if model_lower.starts_with("claude-") {
        Some("anthropic")
    } else if model_lower.starts_with("gemini-") {
        Some("google-ai")
    } else if model_lower.starts_with("mistral-")
        || model_lower.starts_with("codestral-")
        || model_lower.starts_with("pixtral-")
        || model_lower.starts_with("ministral-")
        || model_lower.starts_with("open-mistral-")
        || model_lower.starts_with("devstral-")
        || model_lower.starts_with("magistral-")
    {
        Some("mistral")
    } else if model_lower.starts_with("command-")
        || model_lower.starts_with("embed-")
        || model_lower.starts_with("rerank-")
    {
        Some("cohere")
    } else if model_lower.starts_with("deepseek-") {
        Some("deepseek")
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Translation layer (Phase 3)
// ---------------------------------------------------------------------------

/// Result of translating a request to a provider's native format.
pub struct TranslatedRequest {
    pub path: String,
    pub body: serde_json::Value,
    pub extra_headers: Vec<(String, String)>,
}

/// Trait for translating between OpenAI format and provider-native format.
pub trait LlmTranslator: Send + Sync {
    fn translate_request(
        &self,
        path: &str,
        body: &serde_json::Value,
    ) -> AppResult<TranslatedRequest>;

    fn translate_response(
        &self,
        body: serde_json::Value,
    ) -> AppResult<serde_json::Value>;

    fn needs_translation(&self) -> bool;

    fn gateway_base_url(&self) -> Option<&str> {
        None
    }
}

/// Get the appropriate translator for a provider slug.
pub fn get_translator(provider_slug: &str) -> Box<dyn LlmTranslator> {
    match provider_slug {
        "anthropic" => Box::new(AnthropicTranslator),
        "google-ai" => Box::new(GoogleAiTranslator),
        _ => Box::new(PassthroughTranslator),
    }
}

// ---------------------------------------------------------------------------
// PassthroughTranslator (OpenAI, OpenAI Codex, Mistral, Cohere, DeepSeek)
// ---------------------------------------------------------------------------

pub struct PassthroughTranslator;

impl LlmTranslator for PassthroughTranslator {
    fn translate_request(
        &self,
        path: &str,
        body: &serde_json::Value,
    ) -> AppResult<TranslatedRequest> {
        Ok(TranslatedRequest {
            path: path.to_string(),
            body: body.clone(),
            extra_headers: vec![],
        })
    }

    fn translate_response(
        &self,
        body: serde_json::Value,
    ) -> AppResult<serde_json::Value> {
        Ok(body)
    }

    fn needs_translation(&self) -> bool {
        false
    }
}

// ---------------------------------------------------------------------------
// AnthropicTranslator
// ---------------------------------------------------------------------------

pub struct AnthropicTranslator;

impl LlmTranslator for AnthropicTranslator {
    fn needs_translation(&self) -> bool {
        true
    }

    fn translate_request(
        &self,
        path: &str,
        body: &serde_json::Value,
    ) -> AppResult<TranslatedRequest> {
        let mut translated = body.clone();

        // Extract system messages from messages array
        if let Some(messages) = translated.get("messages").and_then(|m| m.as_array()) {
            let mut system_parts = Vec::new();
            let mut non_system = Vec::new();

            for msg in messages {
                if msg.get("role").and_then(|r| r.as_str()) == Some("system") {
                    if let Some(content) = msg.get("content").and_then(|c| c.as_str()) {
                        system_parts.push(content.to_string());
                    }
                } else {
                    non_system.push(msg.clone());
                }
            }

            if !system_parts.is_empty() {
                translated["system"] =
                    serde_json::Value::String(system_parts.join("\n"));
            }
            translated["messages"] = serde_json::Value::Array(non_system);
        }

        // Default max_tokens if not specified (Anthropic requires it)
        if translated.get("max_tokens").is_none() {
            translated["max_tokens"] = serde_json::json!(4096);
        }

        // Map stop -> stop_sequences
        if let Some(stop) = translated.get("stop").cloned() {
            translated.as_object_mut().map(|o| o.remove("stop"));
            translated["stop_sequences"] = stop;
        }

        // Change path: chat/completions -> messages
        let translated_path = path.replace("chat/completions", "messages");

        Ok(TranslatedRequest {
            path: translated_path,
            body: translated,
            extra_headers: vec![(
                "anthropic-version".to_string(),
                "2023-06-01".to_string(),
            )],
        })
    }

    fn translate_response(
        &self,
        body: serde_json::Value,
    ) -> AppResult<serde_json::Value> {
        let content_blocks = body
            .get("content")
            .and_then(|c| c.as_array())
            .cloned()
            .unwrap_or_default();

        // H-2: Collect ALL text blocks (not just the first one)
        let text_parts: Vec<String> = content_blocks
            .iter()
            .filter_map(|block| {
                if block.get("type").and_then(|t| t.as_str()) == Some("text") {
                    block.get("text").and_then(|t| t.as_str()).map(String::from)
                } else {
                    None
                }
            })
            .collect();
        let content_text = text_parts.join("");

        // H-2: Map tool_use blocks to OpenAI tool_calls format
        let tool_calls: Vec<serde_json::Value> = content_blocks
            .iter()
            .filter_map(|block| {
                if block.get("type").and_then(|t| t.as_str()) == Some("tool_use") {
                    let id = block.get("id").and_then(|v| v.as_str()).unwrap_or("unknown");
                    let name = block.get("name").and_then(|v| v.as_str()).unwrap_or("unknown");
                    let input = block.get("input").cloned().unwrap_or(serde_json::json!({}));
                    let arguments = serde_json::to_string(&input).unwrap_or_default();
                    Some(serde_json::json!({
                        "id": id,
                        "type": "function",
                        "function": {
                            "name": name,
                            "arguments": arguments,
                        }
                    }))
                } else {
                    None
                }
            })
            .collect();

        // Map stop_reason
        let finish_reason = match body
            .get("stop_reason")
            .and_then(|r| r.as_str())
        {
            Some("end_turn") => "stop",
            Some("max_tokens") => "length",
            Some("stop_sequence") => "stop",
            Some("tool_use") => "tool_calls",
            _ => "stop",
        };

        // Map usage
        let input_tokens = body
            .pointer("/usage/input_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        let output_tokens = body
            .pointer("/usage/output_tokens")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);

        let id = body
            .get("id")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");

        let model = body
            .get("model")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");

        let created = chrono::Utc::now().timestamp();

        // Build the message object, including tool_calls only when present
        let mut message = serde_json::json!({
            "role": "assistant",
            "content": if content_text.is_empty() { serde_json::Value::Null } else { serde_json::Value::String(content_text) },
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
}

// ---------------------------------------------------------------------------
// GoogleAiTranslator
//
// M-5: Google AI uses an OpenAI-compatible API format, so no request/response
// body translation is needed (needs_translation() returns false). However, its
// base URL differs from the service's configured URL, so gateway_base_url()
// returns a custom URL. The handler applies this override for both translated
// and non-translated paths.
// ---------------------------------------------------------------------------

pub struct GoogleAiTranslator;

impl LlmTranslator for GoogleAiTranslator {
    fn needs_translation(&self) -> bool {
        false
    }

    fn gateway_base_url(&self) -> Option<&str> {
        Some("https://generativelanguage.googleapis.com/v1beta/openai")
    }

    fn translate_request(
        &self,
        path: &str,
        body: &serde_json::Value,
    ) -> AppResult<TranslatedRequest> {
        Ok(TranslatedRequest {
            path: path.to_string(),
            body: body.clone(),
            extra_headers: vec![],
        })
    }

    fn translate_response(
        &self,
        body: serde_json::Value,
    ) -> AppResult<serde_json::Value> {
        Ok(body)
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // --- resolve_provider_for_model tests ---

    #[test]
    fn resolve_openai_models() {
        assert_eq!(resolve_provider_for_model("gpt-4o"), Some("openai"));
        assert_eq!(resolve_provider_for_model("gpt-4o-mini"), Some("openai"));
        assert_eq!(resolve_provider_for_model("GPT-4"), Some("openai"));
        assert_eq!(resolve_provider_for_model("o1-preview"), Some("openai"));
        assert_eq!(resolve_provider_for_model("o3-mini"), Some("openai"));
        assert_eq!(resolve_provider_for_model("o4-mini"), Some("openai"));
        assert_eq!(resolve_provider_for_model("chatgpt-4o-latest"), Some("openai"));
    }

    #[test]
    fn resolve_anthropic_models() {
        assert_eq!(
            resolve_provider_for_model("claude-sonnet-4-5-20250929"),
            Some("anthropic")
        );
        assert_eq!(resolve_provider_for_model("claude-3-haiku"), Some("anthropic"));
        assert_eq!(resolve_provider_for_model("CLAUDE-opus"), Some("anthropic"));
    }

    #[test]
    fn resolve_google_models() {
        assert_eq!(resolve_provider_for_model("gemini-1.5-pro"), Some("google-ai"));
        assert_eq!(resolve_provider_for_model("gemini-2.0-flash"), Some("google-ai"));
    }

    #[test]
    fn resolve_mistral_models() {
        assert_eq!(resolve_provider_for_model("mistral-large"), Some("mistral"));
        assert_eq!(resolve_provider_for_model("codestral-latest"), Some("mistral"));
        assert_eq!(resolve_provider_for_model("pixtral-large"), Some("mistral"));
        assert_eq!(resolve_provider_for_model("ministral-8b"), Some("mistral"));
        assert_eq!(resolve_provider_for_model("open-mistral-nemo"), Some("mistral"));
        assert_eq!(resolve_provider_for_model("devstral-2-25-12"), Some("mistral"));
        assert_eq!(resolve_provider_for_model("magistral-medium-1-2-25-09"), Some("mistral"));
    }

    #[test]
    fn resolve_cohere_models() {
        assert_eq!(resolve_provider_for_model("command-r-plus"), Some("cohere"));
        assert_eq!(resolve_provider_for_model("embed-english-v3.0"), Some("cohere"));
        assert_eq!(resolve_provider_for_model("rerank-english-v3.0"), Some("cohere"));
    }

    #[test]
    fn resolve_deepseek_models() {
        assert_eq!(resolve_provider_for_model("deepseek-chat"), Some("deepseek"));
        assert_eq!(resolve_provider_for_model("deepseek-reasoner"), Some("deepseek"));
        assert_eq!(resolve_provider_for_model("DEEPSEEK-chat"), Some("deepseek"));
    }

    #[test]
    fn resolve_unknown_model() {
        assert_eq!(resolve_provider_for_model("unknown-model"), None);
        assert_eq!(resolve_provider_for_model(""), None);
        assert_eq!(resolve_provider_for_model("llama-3"), None);
    }

    #[test]
    fn resolve_case_insensitive() {
        assert_eq!(resolve_provider_for_model("GPT-4o"), Some("openai"));
        assert_eq!(resolve_provider_for_model("CLAUDE-3"), Some("anthropic"));
        assert_eq!(resolve_provider_for_model("GEMINI-pro"), Some("google-ai"));
    }

    // --- AnthropicTranslator tests ---

    #[test]
    fn anthropic_translate_request_extracts_system() {
        let translator = AnthropicTranslator;
        let body = serde_json::json!({
            "model": "claude-sonnet-4-5-20250929",
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

        assert_eq!(result.path, "messages");
        assert_eq!(result.body["system"], "You are helpful.");
        let messages = result.body["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["role"], "user");
        assert_eq!(result.body["max_tokens"], 1024);
        assert_eq!(result.extra_headers.len(), 1);
        assert_eq!(result.extra_headers[0].0, "anthropic-version");
    }

    #[test]
    fn anthropic_translate_request_defaults_max_tokens() {
        let translator = AnthropicTranslator;
        let body = serde_json::json!({
            "model": "claude-3-haiku",
            "messages": [{"role": "user", "content": "Hi"}]
        });

        let result = translator
            .translate_request("chat/completions", &body)
            .unwrap();
        assert_eq!(result.body["max_tokens"], 4096);
    }

    #[test]
    fn anthropic_translate_request_maps_stop() {
        let translator = AnthropicTranslator;
        let body = serde_json::json!({
            "model": "claude-3-haiku",
            "messages": [{"role": "user", "content": "Hi"}],
            "stop": ["\n"]
        });

        let result = translator
            .translate_request("chat/completions", &body)
            .unwrap();
        assert!(result.body.get("stop").is_none());
        assert_eq!(result.body["stop_sequences"], serde_json::json!(["\n"]));
    }

    #[test]
    fn anthropic_translate_response() {
        let translator = AnthropicTranslator;
        let anthropic_resp = serde_json::json!({
            "id": "msg_01XFDUDYJgAACzvnptvVoYEL",
            "type": "message",
            "role": "assistant",
            "content": [{"type": "text", "text": "Hello! How can I help?"}],
            "model": "claude-sonnet-4-5-20250929",
            "stop_reason": "end_turn",
            "usage": {"input_tokens": 25, "output_tokens": 10}
        });

        let result = translator.translate_response(anthropic_resp).unwrap();

        assert_eq!(result["object"], "chat.completion");
        assert_eq!(
            result["id"],
            "chatcmpl-msg_01XFDUDYJgAACzvnptvVoYEL"
        );
        assert_eq!(
            result["choices"][0]["message"]["content"],
            "Hello! How can I help?"
        );
        assert_eq!(result["choices"][0]["finish_reason"], "stop");
        assert_eq!(result["usage"]["prompt_tokens"], 25);
        assert_eq!(result["usage"]["completion_tokens"], 10);
        assert_eq!(result["usage"]["total_tokens"], 35);
    }

    #[test]
    fn anthropic_translate_response_max_tokens() {
        let translator = AnthropicTranslator;
        let anthropic_resp = serde_json::json!({
            "id": "msg_test",
            "content": [{"type": "text", "text": "truncated"}],
            "model": "claude-3-haiku",
            "stop_reason": "max_tokens",
            "usage": {"input_tokens": 10, "output_tokens": 100}
        });

        let result = translator.translate_response(anthropic_resp).unwrap();
        assert_eq!(result["choices"][0]["finish_reason"], "length");
    }

    #[test]
    fn anthropic_translate_response_multiple_text_blocks() {
        let translator = AnthropicTranslator;
        let anthropic_resp = serde_json::json!({
            "id": "msg_multi",
            "content": [
                {"type": "text", "text": "Hello "},
                {"type": "text", "text": "World!"}
            ],
            "model": "claude-sonnet-4-5-20250929",
            "stop_reason": "end_turn",
            "usage": {"input_tokens": 10, "output_tokens": 5}
        });

        let result = translator.translate_response(anthropic_resp).unwrap();
        assert_eq!(
            result["choices"][0]["message"]["content"],
            "Hello World!"
        );
    }

    #[test]
    fn anthropic_translate_response_tool_use() {
        let translator = AnthropicTranslator;
        let anthropic_resp = serde_json::json!({
            "id": "msg_tool",
            "content": [
                {"type": "text", "text": "Let me check."},
                {
                    "type": "tool_use",
                    "id": "toolu_01",
                    "name": "get_weather",
                    "input": {"location": "London"}
                }
            ],
            "model": "claude-sonnet-4-5-20250929",
            "stop_reason": "tool_use",
            "usage": {"input_tokens": 20, "output_tokens": 30}
        });

        let result = translator.translate_response(anthropic_resp).unwrap();
        assert_eq!(
            result["choices"][0]["message"]["content"],
            "Let me check."
        );
        assert_eq!(result["choices"][0]["finish_reason"], "tool_calls");

        let tool_calls = result["choices"][0]["message"]["tool_calls"]
            .as_array()
            .unwrap();
        assert_eq!(tool_calls.len(), 1);
        assert_eq!(tool_calls[0]["id"], "toolu_01");
        assert_eq!(tool_calls[0]["type"], "function");
        assert_eq!(tool_calls[0]["function"]["name"], "get_weather");
        let args: serde_json::Value =
            serde_json::from_str(tool_calls[0]["function"]["arguments"].as_str().unwrap())
                .unwrap();
        assert_eq!(args["location"], "London");
    }

    #[test]
    fn anthropic_translate_response_tool_use_only() {
        let translator = AnthropicTranslator;
        let anthropic_resp = serde_json::json!({
            "id": "msg_tool_only",
            "content": [
                {
                    "type": "tool_use",
                    "id": "toolu_02",
                    "name": "search",
                    "input": {"query": "test"}
                }
            ],
            "model": "claude-3-haiku",
            "stop_reason": "tool_use",
            "usage": {"input_tokens": 5, "output_tokens": 10}
        });

        let result = translator.translate_response(anthropic_resp).unwrap();
        // When there's no text content, content should be null
        assert!(result["choices"][0]["message"]["content"].is_null());
        assert_eq!(result["choices"][0]["finish_reason"], "tool_calls");
        assert_eq!(
            result["choices"][0]["message"]["tool_calls"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }

    // --- GoogleAiTranslator tests ---

    #[test]
    fn google_ai_gateway_base_url() {
        let translator = GoogleAiTranslator;
        assert_eq!(
            translator.gateway_base_url(),
            Some("https://generativelanguage.googleapis.com/v1beta/openai")
        );
        assert!(!translator.needs_translation());
    }

    // --- PassthroughTranslator tests ---

    #[test]
    fn passthrough_no_translation() {
        let translator = PassthroughTranslator;
        assert!(!translator.needs_translation());
        assert!(translator.gateway_base_url().is_none());

        let body = serde_json::json!({"model": "gpt-4", "messages": []});
        let result = translator
            .translate_request("chat/completions", &body)
            .unwrap();
        assert_eq!(result.path, "chat/completions");
        assert_eq!(result.body, body);
        assert!(result.extra_headers.is_empty());
    }
}

/// Build concise human-readable descriptions of proxied API requests
/// for display in approval notifications.
///
/// Design goals:
/// - Max 200 characters
/// - Shows what the request does, not raw bytes
/// - Never includes auth tokens, API keys, or PII
/// - Deterministic: same request always produces same description
///
/// Build a concise human-readable description of an API request.
///
/// Examples:
///   "POST /v1/chat/completions (model: gpt-4, max_tokens: 1000, 3 messages)"
///   "GET /v1/models"
///   "DELETE /v1/files/file-abc123"
pub fn build_action_description(method: &str, path: &str, body: Option<&[u8]>) -> String {
    let normalized_path = if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{path}")
    };
    let base = format!("{method} {normalized_path}");

    // Only parse JSON bodies for POST/PUT/PATCH
    if !matches!(method, "POST" | "PUT" | "PATCH") {
        return truncate_description(&base);
    }

    let Some(body) = body else {
        return truncate_description(&base);
    };

    let params = extract_key_params(body);
    if params.is_empty() {
        return truncate_description(&base);
    }

    let detail = params.join(", ");
    truncate_description(&format!("{base} ({detail})"))
}

fn truncate_description(s: &str) -> String {
    if s.len() <= 200 {
        s.to_string()
    } else {
        // Find a valid char boundary at or before byte 197 to avoid
        // panicking on multi-byte UTF-8 sequences.
        let mut end = 197;
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        format!("{}...", &s[..end])
    }
}

/// Extract key parameters from a JSON body for display.
/// Returns a vec of concise key=value pairs.
/// Only extracts well-known fields to avoid leaking sensitive data.
fn extract_key_params(body: &[u8]) -> Vec<String> {
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(body) else {
        return vec![];
    };

    let obj = match value.as_object() {
        Some(obj) => obj,
        None => return vec![],
    };

    let mut params = Vec::new();

    // Well-known fields from common AI APIs (OpenAI, Anthropic, etc.)
    let interesting_keys = [
        "model",
        "max_tokens",
        "temperature",
        "stream",
        "tool_choice",
        "response_format",
        "n",
    ];

    for key in &interesting_keys {
        if let Some(val) = obj.get(*key) {
            let display = match val {
                serde_json::Value::String(s) => s.clone(),
                serde_json::Value::Number(n) => n.to_string(),
                serde_json::Value::Bool(b) => b.to_string(),
                _ => continue,
            };
            params.push(format!("{key}: {display}"));
        }
    }

    // Message count for chat completions (never content)
    if let Some(messages) = obj.get("messages").and_then(|v| v.as_array()) {
        params.push(format!("{} messages", messages.len()));
    }

    params
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn get_request_no_body() {
        let desc = build_action_description("GET", "/v1/models", None);
        assert_eq!(desc, "GET /v1/models");
    }

    #[test]
    fn delete_request_with_path() {
        let desc = build_action_description("DELETE", "/v1/files/file-abc123", None);
        assert_eq!(desc, "DELETE /v1/files/file-abc123");
    }

    #[test]
    fn post_with_json_body() {
        let body = serde_json::json!({
            "model": "gpt-4",
            "max_tokens": 1000,
            "messages": [
                {"role": "user", "content": "Hello"},
                {"role": "assistant", "content": "Hi"},
                {"role": "user", "content": "Help me"}
            ]
        });
        let body_bytes = serde_json::to_vec(&body).unwrap();
        let desc = build_action_description("POST", "/v1/chat/completions", Some(&body_bytes));
        assert_eq!(
            desc,
            "POST /v1/chat/completions (model: gpt-4, max_tokens: 1000, 3 messages)"
        );
    }

    #[test]
    fn post_with_stream_and_temperature() {
        let body = serde_json::json!({
            "model": "claude-sonnet-4-20250514",
            "max_tokens": 4096,
            "stream": true,
            "temperature": 0.7
        });
        let body_bytes = serde_json::to_vec(&body).unwrap();
        let desc = build_action_description("POST", "/anthropic/v1/messages", Some(&body_bytes));
        assert!(desc.contains("model: claude-sonnet-4-20250514"));
        assert!(desc.contains("max_tokens: 4096"));
        assert!(desc.contains("stream: true"));
        assert!(desc.contains("temperature: 0.7"));
    }

    #[test]
    fn post_empty_body() {
        let desc = build_action_description("POST", "/v1/files", None);
        assert_eq!(desc, "POST /v1/files");
    }

    #[test]
    fn post_non_json_body() {
        let desc =
            build_action_description("POST", "/v1/audio/transcriptions", Some(b"binary data"));
        assert_eq!(desc, "POST /v1/audio/transcriptions");
    }

    #[test]
    fn post_json_no_interesting_keys() {
        let body = serde_json::json!({"custom_field": "value"});
        let body_bytes = serde_json::to_vec(&body).unwrap();
        let desc = build_action_description("POST", "/custom/endpoint", Some(&body_bytes));
        assert_eq!(desc, "POST /custom/endpoint");
    }

    #[test]
    fn post_json_array_body() {
        let body_bytes = b"[1,2,3]";
        let desc = build_action_description("POST", "/v1/batch", Some(body_bytes));
        assert_eq!(desc, "POST /v1/batch");
    }

    #[test]
    fn put_request_extracts_params() {
        let body = serde_json::json!({"model": "gpt-4-turbo"});
        let body_bytes = serde_json::to_vec(&body).unwrap();
        let desc = build_action_description("PUT", "/v1/assistants/asst-abc", Some(&body_bytes));
        assert_eq!(desc, "PUT /v1/assistants/asst-abc (model: gpt-4-turbo)");
    }

    #[test]
    fn get_request_ignores_body() {
        let body = serde_json::json!({"model": "gpt-4"});
        let body_bytes = serde_json::to_vec(&body).unwrap();
        let desc = build_action_description("GET", "/v1/models", Some(&body_bytes));
        assert_eq!(desc, "GET /v1/models");
    }

    #[test]
    fn truncates_long_descriptions() {
        let long_path = format!("/v1/{}", "a".repeat(250));
        let desc = build_action_description("GET", &long_path, None);
        assert_eq!(desc.len(), 200);
        assert!(desc.ends_with("..."));
    }

    #[test]
    fn normalizes_path_without_leading_slash() {
        let desc = build_action_description("GET", "v1/models", None);
        assert_eq!(desc, "GET /v1/models");
    }

    #[test]
    fn sensitive_fields_not_extracted() {
        let body = serde_json::json!({
            "model": "gpt-4",
            "messages": [{"role": "user", "content": "secret password is abc123"}],
            "api_key": "sk-secret",
            "authorization": "Bearer token"
        });
        let body_bytes = serde_json::to_vec(&body).unwrap();
        let desc = build_action_description("POST", "/v1/chat/completions", Some(&body_bytes));
        assert!(!desc.contains("secret"));
        assert!(!desc.contains("abc123"));
        assert!(!desc.contains("sk-"));
        assert!(!desc.contains("Bearer"));
        assert!(desc.contains("model: gpt-4"));
        assert!(desc.contains("1 messages"));
    }

    #[test]
    fn response_format_and_tool_choice_extracted() {
        let body = serde_json::json!({
            "model": "gpt-4",
            "tool_choice": "auto",
            "response_format": "json_object"
        });
        let body_bytes = serde_json::to_vec(&body).unwrap();
        let desc = build_action_description("POST", "/v1/chat/completions", Some(&body_bytes));
        assert!(desc.contains("tool_choice: auto"));
        assert!(desc.contains("response_format: json_object"));
    }

    #[test]
    fn n_parameter_extracted() {
        let body = serde_json::json!({"model": "gpt-4", "n": 3});
        let body_bytes = serde_json::to_vec(&body).unwrap();
        let desc = build_action_description("POST", "/v1/completions", Some(&body_bytes));
        assert!(desc.contains("n: 3"));
    }

    #[test]
    fn complex_value_types_skipped() {
        let body = serde_json::json!({
            "model": "gpt-4",
            "response_format": {"type": "json_object"}
        });
        let body_bytes = serde_json::to_vec(&body).unwrap();
        let desc = build_action_description("POST", "/v1/chat/completions", Some(&body_bytes));
        assert!(desc.contains("model: gpt-4"));
        // Object value for response_format should be skipped
        assert!(!desc.contains("response_format"));
    }
}

use super::*;

/// Everything except validated transcript growth stays fixed. Metadata is
/// deliberately exact: v1 does not infer inert semantics for opaque strings.
#[derive(Clone, Serialize, Deserialize, PartialEq)]
pub(super) struct Envelope {
    pub fixed: Value,
    pub path: String,
    pub query: Option<String>,
    pub headers: Vec<(String, String)>,
}

pub(super) fn envelope(candidate: &wire::PrepareRequest) -> AppResult<(Envelope, Vec<Value>)> {
    if candidate.protocol != wire::PROTOCOL
        || candidate.relative_path.trim_matches('/') != "responses"
        || candidate.query.is_some()
        || candidate.semantic_headers.len() > 32
        || candidate.semantic_headers.iter().any(|(name, value)| {
            name.len() > 128
                || value.len() > 8192
                || name.eq_ignore_ascii_case(wire::CONTROL_HEADER)
                || name.eq_ignore_ascii_case(wire::STEP_HEADER)
        })
    {
        return Err(unsupported());
    }
    let bytes = Zeroizing::new(serde_json::to_vec(&candidate.body).map_err(|_| unsupported())?);
    if bytes.len() > wire::MAX_PREPARE_BYTES {
        return Err(unsupported());
    }
    let mut fixed = candidate.body.clone();
    let object = fixed.as_object_mut().ok_or_else(unsupported)?;
    responses_configuration(object)?;
    // Main's broader route recognizer is unchanged. Reusable consent has an
    // additional closed protocol: only declared host function tools, no hosted
    // web search or provider-managed effects, state or replay handles.
    if !super::super::assistant_service_execution::stateless_model_transport(
        "responses",
        &candidate.body,
    ) || object
        .get("background")
        .is_some_and(|v| v != &Value::Bool(false))
        || object.get("tools").is_some_and(|tools| {
            tools.as_array().is_none_or(|tools| {
                tools.iter().any(|t| {
                    t["type"] != "function"
                        || t.get("name").and_then(Value::as_str).is_none()
                        || t.as_object().is_none_or(|o| {
                            o.keys().any(|k| {
                                !matches!(
                                    k.as_str(),
                                    "type" | "name" | "description" | "parameters" | "strict"
                                )
                            })
                        })
                })
            })
        })
    {
        return Err(unsupported());
    }
    if let Some(metadata) = object.get("client_metadata") {
        let metadata = metadata.as_object().ok_or_else(unsupported)?;
        if metadata.is_empty() || metadata.len() > 7 {
            return Err(unsupported());
        }
        for (key, value) in metadata {
            let value = value.as_str().ok_or_else(unsupported)?;
            if key == "x-codex-turn-metadata" {
                if value.len() > 1024 {
                    return Err(unsupported());
                }
            } else if !matches!(
                key.as_str(),
                "root_turn_id"
                    | "session_id"
                    | "thread_id"
                    | "turn_id"
                    | "x-codex-installation-id"
                    | "x-codex-window-id"
            ) || value.is_empty()
                || value.len() > 128
                || !value.bytes().all(|b| {
                    b.is_ascii_alphanumeric()
                        || b"_-".contains(&b)
                        || key == "x-codex-window-id" && b == b':'
                })
                || key == "x-codex-window-id"
                    && value.contains(':')
                    && !value.split_once(':').is_some_and(|(id, index)| {
                        !id.is_empty()
                            && !id.contains(':')
                            && !index.is_empty()
                            && index.bytes().all(|b| b.is_ascii_digit())
                    })
            {
                return Err(unsupported());
            }
        }
        if serde_json::to_vec(metadata)
            .map_err(|_| unsupported())?
            .len()
            > 2048
        {
            return Err(unsupported());
        }
    }
    let input = object
        .remove("input")
        .and_then(|v| v.as_array().cloned())
        .ok_or_else(unsupported)?;
    validate_items(&input)?;
    let mut headers = candidate.semantic_headers.clone();
    for (name, _) in &mut headers {
        name.make_ascii_lowercase();
    }
    headers.sort();
    Ok((
        Envelope {
            fixed,
            path: "responses".into(),
            query: None,
            headers,
        },
        input,
    ))
}

/// A Responses-only reusable subset. The broader route recognizer is shared
/// with other protocols and is deliberately insufficient for this contract.
fn responses_configuration(object: &serde_json::Map<String, Value>) -> AppResult<()> {
    const KEYS: &[&str] = &[
        "model",
        "input",
        "instructions",
        "stream",
        "store",
        "background",
        "tools",
        "tool_choice",
        "parallel_tool_calls",
        "max_output_tokens",
        "temperature",
        "top_p",
        "reasoning",
        "text",
        "include",
        "service_tier",
        "truncation",
        "client_metadata",
    ];
    if object.keys().any(|key| !KEYS.contains(&key.as_str())) {
        return Err(unsupported());
    }
    for key in ["stream", "parallel_tool_calls"] {
        if object.get(key).is_some_and(|v| !v.is_boolean()) {
            return Err(unsupported());
        }
    }
    if object
        .get("instructions")
        .is_some_and(|v| v.as_str().is_none_or(|s| s.len() > wire::MAX_ITEM_BYTES))
        || object
            .get("max_output_tokens")
            .is_some_and(|v| v.as_u64().is_none_or(|n| n == 0 || n > 1_048_576))
        || object
            .get("temperature")
            .is_some_and(|v| v.as_f64().is_none_or(|n| !(0.0..=2.0).contains(&n)))
        || object
            .get("top_p")
            .is_some_and(|v| v.as_f64().is_none_or(|n| !(0.0..=1.0).contains(&n)))
        || object
            .get("service_tier")
            .is_some_and(|v| !matches!(v.as_str(), Some("auto" | "default" | "flex" | "priority")))
        || object.get("truncation").is_some_and(|v| v != "disabled")
        || object.get("include").is_some_and(|v| {
            v.as_array()
                .is_none_or(|a| a.len() > 1 || a.iter().any(|v| v != "reasoning.encrypted_content"))
        })
    {
        return Err(unsupported());
    }
    if let Some(reasoning) = object.get("reasoning") {
        let r = reasoning.as_object().ok_or_else(unsupported)?;
        if r.keys()
            .any(|k| !matches!(k.as_str(), "effort" | "summary"))
            || r.get("effort").is_some_and(|v| {
                !matches!(
                    v.as_str(),
                    Some("none" | "minimal" | "low" | "medium" | "high" | "xhigh")
                )
            })
            || r.get("summary")
                .is_some_and(|v| !matches!(v.as_str(), Some("auto" | "concise" | "detailed")))
        {
            return Err(unsupported());
        }
    }
    if let Some(text) = object.get("text") {
        let t = text.as_object().ok_or_else(unsupported)?;
        if t.keys()
            .any(|k| !matches!(k.as_str(), "format" | "verbosity"))
            || t.get("format")
                .is_some_and(|v| v != &json!({"type":"text"}))
            || t.get("verbosity")
                .is_some_and(|v| !matches!(v.as_str(), Some("low" | "medium" | "high")))
        {
            return Err(unsupported());
        }
    }
    let mut names = std::collections::BTreeSet::new();
    if let Some(tools) = object.get("tools") {
        let tools = tools
            .as_array()
            .filter(|tools| tools.len() <= 128)
            .ok_or_else(unsupported)?;
        for tool in tools {
            let name = tool["name"]
                .as_str()
                .filter(|s| !s.is_empty() && s.len() <= 256)
                .ok_or_else(unsupported)?;
            if !names.insert(name)
                || tool["type"] != "function"
                || tool.get("description").is_some_and(|v| !v.is_string())
                || !tool["parameters"].is_object()
                || tool.get("strict").is_some_and(|v| !v.is_boolean())
            {
                return Err(unsupported());
            }
        }
    }
    if let Some(choice) = object.get("tool_choice") {
        if !matches!(choice.as_str(), Some("none" | "auto" | "required"))
            && !(choice.as_object().is_some_and(|o| o.len() == 2)
                && choice["type"] == "function"
                && choice["name"]
                    .as_str()
                    .is_some_and(|name| names.contains(name)))
        {
            return Err(unsupported());
        }
    }
    Ok(())
}

fn validate_items(items: &[Value]) -> AppResult<()> {
    if items.len() > wire::MAX_ITEMS {
        return Err(unsupported());
    }
    for item in items {
        if serde_json::to_vec(item).map_err(|_| unsupported())?.len() > wire::MAX_ITEM_BYTES
            || !item.is_object()
        {
            return Err(unsupported());
        }
        match item["type"].as_str() {
            Some("function_call") => {
                closed_keys(
                    item,
                    &["id", "type", "status", "call_id", "name", "arguments"],
                )?;
                for key in ["call_id", "name", "arguments"] {
                    if !item[key].is_string() {
                        return Err(unsupported());
                    }
                }
                serde_json::from_str::<Value>(item["arguments"].as_str().ok_or_else(unsupported)?)
                    .map_err(|_| unsupported())?;
                optional_item_identity(item)?;
            }
            Some("function_call_output") => {
                closed_keys(item, &["type", "call_id", "output"])?;
                if !item["call_id"].is_string() || !item["output"].is_string() {
                    return Err(unsupported());
                }
            }
            Some("reasoning") => {
                closed_keys(
                    item,
                    &["id", "type", "status", "summary", "encrypted_content"],
                )?;
                optional_item_identity(item)?;
                if item
                    .get("encrypted_content")
                    .is_some_and(|v| !v.is_null() && !v.is_string())
                {
                    return Err(unsupported());
                }
                for part in item["summary"].as_array().ok_or_else(unsupported)? {
                    closed_keys(part, &["type", "text"])?;
                    if part["type"] != "summary_text" || !part["text"].is_string() {
                        return Err(unsupported());
                    }
                }
            }
            Some("message") | None
                if matches!(
                    item["role"].as_str(),
                    Some("user" | "assistant" | "system" | "developer")
                ) =>
            {
                closed_keys(item, &["id", "type", "status", "role", "content"])?;
                optional_item_identity(item)?;
                if !item["content"].is_string() {
                    for part in item["content"].as_array().ok_or_else(unsupported)? {
                        // Initial history is fixed, text-only input. Future
                        // output/tool additions need separate server proof.
                        match part["type"].as_str() {
                            Some("input_text") => {
                                closed_keys(part, &["type", "text"])?;
                                if !part["text"].is_string() {
                                    return Err(unsupported());
                                }
                            }
                            Some("output_text") if item["role"] == "assistant" => {
                                closed_keys(part, &["type", "text", "annotations", "logprobs"])?;
                                if !part["text"].is_string()
                                    || part.get("annotations").is_some_and(|v| v != &json!([]))
                                    || part
                                        .get("logprobs")
                                        .is_some_and(|v| v != &json!([]) && !v.is_null())
                                {
                                    return Err(unsupported());
                                }
                            }
                            Some("refusal") if item["role"] == "assistant" => {
                                closed_keys(part, &["type", "refusal"])?;
                                if !part["refusal"].is_string() {
                                    return Err(unsupported());
                                }
                            }
                            _ => return Err(unsupported()),
                        }
                    }
                }
            }
            _ => return Err(unsupported()),
        }
    }
    Ok(())
}

fn closed_keys(value: &Value, allowed: &[&str]) -> AppResult<()> {
    if value
        .as_object()
        .is_none_or(|object| object.keys().any(|key| !allowed.contains(&key.as_str())))
    {
        return Err(unsupported());
    }
    Ok(())
}
fn optional_item_identity(item: &Value) -> AppResult<()> {
    if item
        .get("id")
        .is_some_and(|v| v.as_str().is_none_or(|s| s.is_empty() || s.len() > 256))
        || item.get("status").is_some_and(|v| v != "completed")
    {
        return Err(unsupported());
    }
    Ok(())
}

pub(super) fn request_binding(candidate: &wire::PrepareRequest) -> String {
    let mut headers = candidate.semantic_headers.clone();
    for (name, _) in &mut headers {
        name.make_ascii_lowercase();
    }
    headers.sort();
    digest(
        json!({"protocol":wire::PROTOCOL,"path":candidate.relative_path,
        "query":candidate.query,"headers":headers,"body":candidate.body}),
    )
}

/// First Allow resumes only the original observed input plus the exact text
/// NyxID put into its saved continuation's registered runtime request. No
/// arbitrary bridge-supplied user/developer/system additions are accepted.
pub(super) fn allowed_resume(
    initial: &[Value],
    resumed: &[Value],
    saved_text: &str,
) -> AppResult<()> {
    if resumed == initial {
        return Ok(());
    }
    if let Some(tail) = resumed.strip_prefix(initial)
        && tail.len() == 1
        && exact_user_text(&tail[0], saved_text)
    {
        return Ok(());
    }
    // A rejected outer runtime turn may discard its disposable native session.
    // Its saved Allow continuation then starts a fresh native transcript. Only
    // the previously observed exact privileged prefix may survive that reset;
    // the single replacement user item is the full server-saved outer input.
    // No bridge-authored summary or new system/developer item is admitted.
    let scaffold = initial
        .iter()
        .take_while(|item| matches!(item["role"].as_str(), Some("system" | "developer")))
        .count();
    if resumed.len() == scaffold + 1
        && resumed[..scaffold] == initial[..scaffold]
        && exact_user_text(&resumed[scaffold], saved_text)
    {
        return Ok(());
    }
    Err(conflict())
}
fn exact_user_text(item: &Value, text: &str) -> bool {
    let Some(object) = item.as_object() else {
        return false;
    };
    if object
        .keys()
        .any(|key| !matches!(key.as_str(), "type" | "role" | "content"))
        || item["role"] != "user"
        || item.get("type").is_some_and(|v| v != "message")
    {
        return false;
    }
    item["content"] == text || item["content"] == json!([{"type":"input_text","text":text}])
}

/// Completion is observed before host tools execute. Next prepare separately
/// proves every added tool result, using the actual deterministic model-facing
/// result projection recorded by the server; raw MCP bytes are not substituted.
pub(super) fn next_input(
    previous: &[Value],
    completion: &ObservedCompletion,
    next: &[Value],
    results: &[VerifiedToolResult],
) -> AppResult<()> {
    let tail = next.strip_prefix(previous).ok_or_else(conflict)?;
    let results_tail = tail
        .strip_prefix(completion.output.as_slice())
        .ok_or_else(conflict)?;
    if results_tail.len() != completion.calls.len() || results.len() != completion.calls.len() {
        return Err(conflict());
    }
    let mut used = std::collections::BTreeSet::new();
    for item in results_tail {
        let call_id = item["call_id"].as_str().ok_or_else(conflict)?;
        if !used.insert(call_id)
            || item["type"] != "function_call_output"
            || item.as_object().is_none_or(|o| {
                o.keys()
                    .any(|k| !matches!(k.as_str(), "type" | "call_id" | "output"))
            })
        {
            return Err(conflict());
        }
        let call = completion
            .calls
            .iter()
            .find(|c| c.call_id == call_id)
            .ok_or_else(conflict)?;
        let result = results
            .iter()
            .find(|r| r.call_id == call_id)
            .ok_or_else(conflict)?;
        if result.tool_name != call.name
            || result.arguments_binding != digest(json!(call.arguments))
            || item["output"]
                .as_str()
                .is_none_or(|text| digest(json!(text)) != result.model_text_binding)
        {
            return Err(conflict());
        }
    }
    Ok(())
}

pub(super) fn verify_completion(
    value: &Value,
    fixed: &Value,
) -> AppResult<VerifiedProviderCompletion> {
    if value["object"] != "response"
        || value["status"] != "completed"
        || !value["error"].is_null()
        || !value["incomplete_details"].is_null()
        || serde_json::to_vec(value).map_err(|_| unsupported())?.len() > wire::MAX_OUTPUT_BYTES
    {
        return Err(unsupported());
    }
    let response_id = value["id"]
        .as_str()
        .filter(|s| !s.is_empty() && s.len() <= 256)
        .ok_or_else(unsupported)?
        .to_owned();
    let output = value["output"].as_array().ok_or_else(unsupported)?.clone();
    if output.len() > wire::MAX_ITEMS {
        return Err(unsupported());
    }
    let mut calls = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for item in &output {
        if serde_json::to_vec(item).map_err(|_| unsupported())?.len() > wire::MAX_ITEM_BYTES {
            return Err(unsupported());
        }
        let object = item.as_object().ok_or_else(unsupported)?;
        match item["type"].as_str() {
            Some("function_call") => {
                if object.keys().any(|key| {
                    !matches!(
                        key.as_str(),
                        "id" | "type" | "status" | "call_id" | "name" | "arguments"
                    )
                }) {
                    return Err(unsupported());
                }
                let call_id = item["call_id"]
                    .as_str()
                    .filter(|id| !id.is_empty() && id.len() <= 256)
                    .ok_or_else(unsupported)?;
                if !seen.insert(call_id.to_owned()) || item["status"] != "completed" {
                    return Err(unsupported());
                }
                let name = item["name"]
                    .as_str()
                    .filter(|s| !s.is_empty() && s.len() <= 256)
                    .ok_or_else(unsupported)?
                    .to_owned();
                if !fixed["tools"].as_array().is_some_and(|tools| {
                    tools
                        .iter()
                        .any(|tool| tool["type"] == "function" && tool["name"] == name)
                }) {
                    return Err(unsupported());
                }
                let arguments = item["arguments"]
                    .as_str()
                    .ok_or_else(unsupported)?
                    .to_owned();
                serde_json::from_str::<Value>(&arguments).map_err(|_| unsupported())?;
                calls.push(ProviderToolCall {
                    call_id: call_id.into(),
                    name,
                    arguments,
                });
            }
            Some("message") => {
                if item["role"] != "assistant"
                    || item["status"] != "completed"
                    || object.keys().any(|key| {
                        !matches!(key.as_str(), "id" | "type" | "status" | "role" | "content")
                    })
                    || item["content"].as_array().is_none_or(|content| {
                        content.iter().any(|part| match part["type"].as_str() {
                            Some("output_text") => {
                                !part["text"].is_string()
                                    || part.as_object().is_none_or(|p| {
                                        p.keys().any(|k| {
                                            !matches!(
                                                k.as_str(),
                                                "type" | "text" | "annotations" | "logprobs"
                                            )
                                        })
                                    })
                            }
                            Some("refusal") => {
                                !part["refusal"].is_string()
                                    || part.as_object().is_none_or(|p| {
                                        p.keys().any(|k| !matches!(k.as_str(), "type" | "refusal"))
                                    })
                            }
                            _ => true,
                        })
                    })
                {
                    return Err(unsupported());
                }
            }
            Some("reasoning") => {
                if object.keys().any(|key| {
                    !matches!(
                        key.as_str(),
                        "id" | "type" | "status" | "summary" | "encrypted_content"
                    )
                }) || item
                    .get("encrypted_content")
                    .is_some_and(|v| !v.is_null() && !v.is_string())
                    || item["summary"].as_array().is_none_or(|items| {
                        items.iter().any(|part| {
                            part["type"] != "summary_text"
                                || !part["text"].is_string()
                                || part.as_object().is_none_or(|p| {
                                    p.keys().any(|k| !matches!(k.as_str(), "type" | "text"))
                                })
                        })
                    })
                {
                    return Err(unsupported());
                }
            }
            _ => return Err(unsupported()),
        }
    }
    Ok(VerifiedProviderCompletion {
        response_id,
        output,
        calls,
    })
}

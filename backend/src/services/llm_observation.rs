//! Strict, allowlisted provider measurements. Legacy billing extraction stays separate.
use serde_json::Value;

use crate::models::llm_usage::{
    LlmUsageObservation, UsageAdapter, UsageAggregationScope, UsageCompleteness, UsageOutcome,
    UsageSource,
};

pub fn bounded_id(value: Option<&Value>) -> Option<String> {
    let text = value?.as_str()?;
    // Provider identities are metadata, never arbitrary reflected prose or URLs.
    (!text.is_empty()
        && text.len() <= 256
        && text
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.:/@+".contains(&b)))
    .then(|| text.to_owned())
}

pub fn missing(outcome: UsageOutcome) -> LlmUsageObservation {
    LlmUsageObservation {
        version: 1,
        outcome,
        ..Default::default()
    }
}

pub fn with_outcome(
    observation: Option<LlmUsageObservation>,
    outcome: UsageOutcome,
) -> LlmUsageObservation {
    let mut o = observation.unwrap_or_else(|| missing(outcome));
    if outcome_severity(outcome) >= outcome_severity(o.outcome) {
        o.outcome = outcome;
    }
    if o.aggregation_scope == UsageAggregationScope::RealtimeConnection {
        completeness(&mut o);
    } else {
        normalize(&mut o);
    }
    o
}

fn outcome_severity(outcome: UsageOutcome) -> u8 {
    match outcome {
        UsageOutcome::Unknown => 0,
        UsageOutcome::Succeeded => 1,
        UsageOutcome::Interrupted => 2,
        UsageOutcome::Failed => 3,
    }
}

pub fn transport_outcome(
    mut usage: crate::models::service_billing::PlatformUsage,
    outcome: UsageOutcome,
) -> crate::models::service_billing::PlatformUsage {
    usage.llm_observation = Some(with_outcome(usage.llm_observation, outcome));
    usage
}

fn count(value: &Value, names: &[&str], invalid: &mut bool) -> Option<i64> {
    for name in names {
        if let Some(raw) = value.pointer(name) {
            let parsed = raw.as_i64().filter(|n| *n >= 0);
            if parsed.is_none() {
                *invalid = true;
            }
            return parsed;
        }
    }
    None
}

fn modality(value: &Value, field: &str, name: &str, invalid: &mut bool) -> Option<i64> {
    let entries = value.get(field)?;
    let Some(entries) = entries.as_array() else {
        *invalid = true;
        return None;
    };
    let mut total: Option<i64> = None;
    for entry in entries {
        if entry.get("modality").and_then(Value::as_str) == Some(name) {
            let n = count(entry, &["/tokenCount"], invalid)?;
            total = match total.unwrap_or(0).checked_add(n) {
                Some(n) => Some(n),
                None => {
                    *invalid = true;
                    return None;
                }
            };
        }
    }
    total
}

pub fn extract(value: &Value) -> Option<LlmUsageObservation> {
    let envelope = value
        .get("response")
        .or_else(|| value.get("message"))
        .unwrap_or(value);
    let gemini = envelope.get("usageMetadata");
    let usage = gemini.or_else(|| envelope.get("usage")).unwrap_or(envelope);
    if !usage.is_object()
        || !(gemini.is_some()
            || [
                "prompt_tokens",
                "input_tokens",
                "output_tokens",
                "completion_tokens",
                "total_tokens",
                "cache_read_input_tokens",
                "cache_creation_input_tokens",
            ]
            .iter()
            .any(|k| usage.get(k).is_some()))
    {
        return None;
    }
    let mut invalid = false;
    let adapter = if gemini.is_some() {
        UsageAdapter::Gemini
    } else if envelope.get("type").and_then(Value::as_str) == Some("message")
        || usage.get("cache_read_input_tokens").is_some()
        || usage.get("cache_creation_input_tokens").is_some()
        || value
            .get("type")
            .and_then(Value::as_str)
            .is_some_and(|t| t.starts_with("message_"))
    {
        UsageAdapter::Anthropic
    } else {
        UsageAdapter::Openai
    };
    let mut o = missing(match envelope.get("status").and_then(Value::as_str) {
        Some("failed") => UsageOutcome::Failed,
        Some("incomplete" | "cancelled") => UsageOutcome::Interrupted,
        _ => UsageOutcome::Succeeded,
    });
    o.adapter = adapter;
    o.source = UsageSource::ProviderReported;
    o.reported_input_tokens = count(
        usage,
        &["/prompt_tokens", "/input_tokens", "/promptTokenCount"],
        &mut invalid,
    );
    o.reported_output_tokens = count(
        usage,
        &[
            "/completion_tokens",
            "/output_tokens",
            "/candidatesTokenCount",
        ],
        &mut invalid,
    );
    o.reported_total_tokens = count(usage, &["/total_tokens", "/totalTokenCount"], &mut invalid);
    o.cache_read_tokens = count(
        usage,
        &[
            "/prompt_tokens_details/cached_tokens",
            "/input_tokens_details/cached_tokens",
            "/input_token_details/cached_tokens",
            "/cache_read_input_tokens",
            "/cachedContentTokenCount",
        ],
        &mut invalid,
    );
    o.cache_write_tokens = count(
        usage,
        &[
            "/prompt_tokens_details/cache_write_tokens",
            "/input_tokens_details/cache_write_tokens",
            "/input_token_details/cache_write_tokens",
            "/cache_creation_input_tokens",
        ],
        &mut invalid,
    );
    o.reasoning_tokens = count(
        usage,
        &[
            "/completion_tokens_details/reasoning_tokens",
            "/output_tokens_details/reasoning_tokens",
            "/output_token_details/reasoning_tokens",
            "/thoughtsTokenCount",
        ],
        &mut invalid,
    );
    o.tool_input_tokens = count(usage, &["/toolUsePromptTokenCount"], &mut invalid);
    o.cache_write_5m_tokens = count(
        usage,
        &["/cache_creation/ephemeral_5m_input_tokens"],
        &mut invalid,
    );
    o.cache_write_1h_tokens = count(
        usage,
        &["/cache_creation/ephemeral_1h_input_tokens"],
        &mut invalid,
    );
    macro_rules! details {
        ($target:ident, $direction:literal, $chat:literal, $name:literal) => {
            o.$target = count(
                usage,
                &[
                    concat!("/", $direction, "_tokens_details/", $name, "_tokens"),
                    concat!("/", $direction, "_token_details/", $name, "_tokens"),
                    concat!("/", $chat, "_tokens_details/", $name, "_tokens"),
                ],
                &mut invalid,
            );
        };
    }
    details!(audio_input_tokens, "input", "prompt", "audio");
    details!(audio_output_tokens, "output", "completion", "audio");
    details!(image_input_tokens, "input", "prompt", "image");
    details!(image_output_tokens, "output", "completion", "image");
    details!(text_input_tokens, "input", "prompt", "text");
    details!(text_output_tokens, "output", "completion", "text");
    o.cache_read_audio_tokens = count(
        usage,
        &[
            "/input_token_details/cached_tokens_details/audio_tokens",
            "/input_tokens_details/cached_tokens_details/audio_tokens",
        ],
        &mut invalid,
    );
    o.cache_read_image_tokens = count(
        usage,
        &[
            "/input_token_details/cached_tokens_details/image_tokens",
            "/input_tokens_details/cached_tokens_details/image_tokens",
        ],
        &mut invalid,
    );
    o.cache_read_text_tokens = count(
        usage,
        &[
            "/input_token_details/cached_tokens_details/text_tokens",
            "/input_tokens_details/cached_tokens_details/text_tokens",
        ],
        &mut invalid,
    );
    if gemini.is_some() {
        o.audio_input_tokens = modality(usage, "promptTokensDetails", "AUDIO", &mut invalid);
        o.audio_output_tokens = modality(usage, "candidatesTokensDetails", "AUDIO", &mut invalid);
        o.image_input_tokens = modality(usage, "promptTokensDetails", "IMAGE", &mut invalid);
        o.image_output_tokens = modality(usage, "candidatesTokensDetails", "IMAGE", &mut invalid);
        o.text_input_tokens = modality(usage, "promptTokensDetails", "TEXT", &mut invalid);
        o.text_output_tokens = modality(usage, "candidatesTokensDetails", "TEXT", &mut invalid);
        o.cache_read_audio_tokens = modality(usage, "cacheTokensDetails", "AUDIO", &mut invalid);
        o.cache_read_image_tokens = modality(usage, "cacheTokensDetails", "IMAGE", &mut invalid);
        o.cache_read_text_tokens = modality(usage, "cacheTokensDetails", "TEXT", &mut invalid);
    }
    o.provider_response_id = bounded_id(envelope.get("id").or_else(|| envelope.get("responseId")));
    o.resolved_model = bounded_id(
        envelope
            .get("model")
            .or_else(|| envelope.get("modelVersion")),
    );
    o.processing_tier = bounded_id(envelope.get("service_tier"));
    // Native generation has no token cache-write class.
    if adapter == UsageAdapter::Gemini {
        o.cache_write_tokens = Some(0);
    }
    if adapter == UsageAdapter::Gemini {
        for (field, path) in [
            (&mut o.cache_read_tokens, "/cachedContentTokenCount"),
            (&mut o.reasoning_tokens, "/thoughtsTokenCount"),
            (&mut o.tool_input_tokens, "/toolUsePromptTokenCount"),
        ] {
            if usage.pointer(path).is_none() {
                *field = Some(0);
            }
        }
    }
    o.invalid_counts = invalid;
    normalize(&mut o);
    Some(o)
}

/// Only resolved native OpenAI/Azure origins define omitted writes as
/// nonapplicable. Compatible/custom routes retain their unknown partition.
pub fn native_openai_semantics(o: &mut LlmUsageObservation, native_provider: bool) {
    if o.aggregation_scope == UsageAggregationScope::Invocation
        && o.adapter == UsageAdapter::Openai
        && !o.invalid_counts
        && native_provider
    {
        if o.cache_write_tokens.is_none() {
            o.cache_write_tokens = Some(0);
        }
        normalize(o);
    }
}

pub fn normalize(o: &mut LlmUsageObservation) {
    if o.aggregation_scope == UsageAggregationScope::RealtimeConnection {
        completeness(o);
        return;
    }
    o.input_tokens = match o.adapter {
        UsageAdapter::Anthropic => o.reported_input_tokens,
        _ => match (
            o.reported_input_tokens,
            o.cache_read_tokens,
            o.cache_write_tokens,
        ) {
            (Some(input), Some(read), Some(write)) => input
                .checked_sub(read)
                .and_then(|n| n.checked_sub(write))
                .filter(|n| *n >= 0),
            // The provider has not given enough information to partition input.
            _ => None,
        },
    };
    o.output_tokens = if o.adapter == UsageAdapter::Gemini {
        match (o.reported_output_tokens, o.reasoning_tokens) {
            (Some(output), Some(thoughts)) => {
                let n = output.checked_add(thoughts);
                if n.is_none() {
                    o.invalid_counts = true;
                }
                n
            }
            _ => None,
        }
    } else {
        o.reported_output_tokens
    };
    if o.adapter != UsageAdapter::Anthropic {
        let known_cache = [o.cache_read_tokens, o.cache_write_tokens]
            .into_iter()
            .flatten()
            .try_fold(0_i64, |sum, n| sum.checked_add(n));
        if known_cache.is_none()
            || known_cache
                .zip(o.reported_input_tokens)
                .is_some_and(|(cache, input)| cache > input)
        {
            o.invalid_counts = true;
        }
    }
    if let (Some(input), Some(read), Some(write)) = (
        o.reported_input_tokens,
        o.cache_read_tokens,
        o.cache_write_tokens,
    ) && o.adapter != UsageAdapter::Anthropic
        && input
            .checked_sub(read)
            .and_then(|n| n.checked_sub(write))
            .is_none_or(|n| n < 0)
    {
        o.invalid_counts = true;
    }
    for (subset, total) in [
        (o.reasoning_tokens, o.output_tokens),
        (o.audio_input_tokens, o.reported_input_tokens),
        (o.image_input_tokens, o.reported_input_tokens),
        (o.audio_output_tokens, o.output_tokens),
        (o.image_output_tokens, o.output_tokens),
        (o.cache_read_audio_tokens, o.cache_read_tokens),
        (o.cache_read_image_tokens, o.cache_read_tokens),
        (o.cache_read_text_tokens, o.cache_read_tokens),
        (o.cache_read_audio_tokens, o.audio_input_tokens),
        (o.cache_read_image_tokens, o.image_input_tokens),
        (o.cache_read_text_tokens, o.text_input_tokens),
        (o.text_input_tokens, o.reported_input_tokens),
        (o.text_output_tokens, o.output_tokens),
        (o.cache_write_5m_tokens, o.cache_write_tokens),
        (o.cache_write_1h_tokens, o.cache_write_tokens),
    ] {
        if subset.zip(total).is_some_and(|(s, t)| s > t) {
            o.invalid_counts = true;
        }
    }
    if let (Some(five), Some(hour), Some(write)) = (
        o.cache_write_5m_tokens,
        o.cache_write_1h_tokens,
        o.cache_write_tokens,
    ) && five.checked_add(hour) != Some(write)
    {
        o.invalid_counts = true;
    }
    for (parts, total) in [
        (
            [
                o.audio_input_tokens,
                o.image_input_tokens,
                o.text_input_tokens,
            ],
            o.reported_input_tokens,
        ),
        (
            [
                o.audio_output_tokens,
                o.image_output_tokens,
                o.text_output_tokens,
            ],
            o.output_tokens,
        ),
        (
            [
                o.cache_read_audio_tokens,
                o.cache_read_image_tokens,
                o.cache_read_text_tokens,
            ],
            o.cache_read_tokens,
        ),
    ] {
        let sum = parts
            .into_iter()
            .flatten()
            .try_fold(0_i64, |sum, n| sum.checked_add(n));
        if sum.is_none() || sum.zip(total).is_some_and(|(sum, total)| sum > total) {
            o.invalid_counts = true;
        }
    }
    if let Some(total) = o.reported_total_tokens {
        let mut parts = vec![o.reported_input_tokens, o.output_tokens];
        if o.adapter == UsageAdapter::Anthropic {
            parts.extend([o.cache_read_tokens, o.cache_write_tokens]);
        }
        if o.adapter == UsageAdapter::Gemini {
            parts.push(o.tool_input_tokens);
        }
        let fully_known = parts.iter().all(Option::is_some);
        let sum = parts
            .into_iter()
            .flatten()
            .try_fold(0_i64, |sum, n| sum.checked_add(n));
        if sum.is_none_or(|sum| sum > total || fully_known && sum != total) {
            o.invalid_counts = true;
        }
    }
    completeness(o);
}

fn completeness(o: &mut LlmUsageObservation) {
    o.completeness = if o.source == UsageSource::Unknown {
        UsageCompleteness::Missing
    } else if !o.invalid_counts
        && o.outcome == UsageOutcome::Succeeded
        && o.input_tokens.is_some()
        && o.output_tokens.is_some()
        && o.cache_read_tokens.is_some()
        && o.cache_write_tokens.is_some()
    {
        UsageCompleteness::Complete
    } else {
        UsageCompleteness::Partial
    };
}

/// Snapshots merge their present fields. Deltas add only explicitly reported
/// counts; absent fields never manufacture zero or a cache/modality intersection.
pub fn merge(target: &mut Option<LlmUsageObservation>, incoming: LlmUsageObservation, delta: bool) {
    let Some(o) = target else {
        *target = Some(incoming);
        return;
    };
    if o.adapter == UsageAdapter::Unknown {
        o.adapter = incoming.adapter;
    }
    if incoming.source == UsageSource::ProviderReported {
        o.source = incoming.source;
    }
    o.invalid_counts |= incoming.invalid_counts;
    let severity = |outcome| match outcome {
        UsageOutcome::Unknown => 0,
        UsageOutcome::Succeeded => 1,
        UsageOutcome::Interrupted => 2,
        UsageOutcome::Failed => 3,
    };
    if severity(incoming.outcome) > severity(o.outcome) {
        o.outcome = incoming.outcome;
    }
    macro_rules! counts { ($($field:ident),*) => { $(
        if let Some(n) = incoming.$field {
            o.$field = if delta { match o.$field.unwrap_or(0).checked_add(n) { Some(n) => Some(n), None => { o.invalid_counts = true; None } } }
                else { Some(o.$field.map_or(n,|old| old.max(n))) };
        }
    )* }; }
    counts!(
        reported_input_tokens,
        reported_output_tokens,
        reported_total_tokens,
        cache_read_tokens,
        cache_write_tokens,
        reasoning_tokens,
        tool_input_tokens,
        cache_write_5m_tokens,
        cache_write_1h_tokens,
        audio_input_tokens,
        audio_output_tokens,
        image_input_tokens,
        image_output_tokens,
        text_input_tokens,
        text_output_tokens,
        cache_read_audio_tokens,
        cache_read_image_tokens,
        cache_read_text_tokens
    );
    if incoming.provider_response_id.is_some() {
        o.provider_response_id = incoming.provider_response_id;
    }
    if incoming.resolved_model.is_some() {
        o.resolved_model = incoming.resolved_model;
    }
    if incoming.processing_tier.is_some() {
        o.processing_tier = incoming.processing_tier;
    }
    normalize(o);
}

/// Distinct responses are a sum of complete per-response measurements. A
/// missing class in any response makes that class unknown for the connection.
pub fn aggregate_response(target: &mut Option<LlmUsageObservation>, incoming: LlmUsageObservation) {
    let Some(o) = target else {
        *target = Some(incoming);
        return;
    };
    o.aggregation_scope = UsageAggregationScope::RealtimeConnection;
    o.invalid_counts |= incoming.invalid_counts;
    let severity = |v| match v {
        UsageOutcome::Unknown => 0,
        UsageOutcome::Succeeded => 1,
        UsageOutcome::Interrupted => 2,
        UsageOutcome::Failed => 3,
    };
    if severity(incoming.outcome) > severity(o.outcome) {
        o.outcome = incoming.outcome;
    }
    if incoming.source == UsageSource::ProviderReported {
        o.source = incoming.source;
    }
    macro_rules! sum { ($($field:ident),*) => { $(
        o.$field = match (o.$field, incoming.$field) {
            (Some(a), Some(b)) => match a.checked_add(b) { Some(n) => Some(n), None => {o.invalid_counts=true;None} },
            _ => None,
        };
    )* }; }
    sum!(
        reported_input_tokens,
        reported_output_tokens,
        reported_total_tokens,
        input_tokens,
        output_tokens,
        cache_read_tokens,
        cache_write_tokens,
        reasoning_tokens,
        tool_input_tokens,
        cache_write_5m_tokens,
        cache_write_1h_tokens,
        audio_input_tokens,
        audio_output_tokens,
        image_input_tokens,
        image_output_tokens,
        text_input_tokens,
        text_output_tokens,
        cache_read_audio_tokens,
        cache_read_image_tokens,
        cache_read_text_tokens
    );
    if o.adapter != incoming.adapter {
        o.adapter = UsageAdapter::Unknown;
    }
    if o.resolved_model != incoming.resolved_model {
        o.resolved_model = None;
    }
    if o.processing_tier != incoming.processing_tier {
        o.processing_tier = None;
    }
    o.provider_response_id = None;
    completeness(o);
}

/// Stream usage is provisional until a provider terminal marker is observed.
pub fn stream_observation(event: Option<&str>, value: &Value) -> Option<LlmUsageObservation> {
    let event = event.or_else(|| value.get("type").and_then(Value::as_str));
    let terminal = match event {
        Some("response.done")
            if value
                .pointer("/response/status")
                .and_then(Value::as_str)
                .is_some_and(|s| matches!(s, "cancelled" | "incomplete" | "failed")) =>
        {
            Some(
                if value.pointer("/response/status").and_then(Value::as_str) == Some("failed") {
                    UsageOutcome::Failed
                } else {
                    UsageOutcome::Interrupted
                },
            )
        }
        Some("response.completed" | "response.done" | "message_stop") => {
            Some(UsageOutcome::Succeeded)
        }
        Some("response.incomplete") => Some(UsageOutcome::Interrupted),
        Some("response.failed" | "error") => Some(UsageOutcome::Failed),
        _ if value.get("usageMetadata").is_some()
            && value
                .get("candidates")
                .and_then(Value::as_array)
                .is_some_and(|a| a.iter().any(|c| c.get("finishReason").is_some())) =>
        {
            Some(UsageOutcome::Succeeded)
        }
        _ if value.get("usage").is_some()
            && value
                .get("choices")
                .and_then(Value::as_array)
                .is_some_and(Vec::is_empty) =>
        {
            Some(UsageOutcome::Succeeded)
        }
        _ => None,
    };
    let mut o = extract(value).or_else(|| terminal.map(missing))?;
    let envelope = value
        .get("response")
        .or_else(|| value.get("message"))
        .unwrap_or(value);
    if o.provider_response_id.is_none() {
        o.provider_response_id =
            bounded_id(envelope.get("id").or_else(|| envelope.get("responseId")));
    }
    if o.resolved_model.is_none() {
        o.resolved_model = bounded_id(
            envelope
                .get("model")
                .or_else(|| envelope.get("modelVersion")),
        );
    }
    if o.processing_tier.is_none() {
        o.processing_tier = bounded_id(envelope.get("service_tier"));
    }
    if let Some(terminal) = terminal
        && outcome_severity(terminal) > outcome_severity(o.outcome)
    {
        o.outcome = terminal;
    }
    if terminal.is_none() && !matches!(o.outcome, UsageOutcome::Failed | UsageOutcome::Interrupted)
    {
        o.outcome = UsageOutcome::Unknown;
    }
    normalize(&mut o);
    Some(o)
}

/// Use only the server-loaded authority snapshot; no correlation headers.
pub fn chat_context(
    chat: Option<&super::assistant_acknowledgement_service::ChatAuthority>,
) -> crate::models::llm_usage::LlmInvocationContext {
    let mut context = crate::models::llm_usage::LlmInvocationContext::default();
    if let Some(chat) = chat {
        context.conversation_id = Some(chat.conversation_id.clone());
        context.agent_id = Some(chat.agent_id.clone());
        if chat.turn_live && !chat.turn_stopped {
            context.turn_id = chat.turn_id.clone();
        }
    }
    context
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::llm_usage_service::{
        self as legacy, RealtimeLlmUsageCollector, ReportedLlmUsageAccumulator,
    };
    use serde_json::json;

    fn four() -> Value {
        json!({"id":"resp_1","model":"gpt-test","service_tier":"priority","usage":{
            "prompt_tokens":100,"completion_tokens":20,"total_tokens":120,
            "prompt_tokens_details":{"cached_tokens":30,"cache_write_tokens":10},
            "completion_tokens_details":{"reasoning_tokens":5}}})
    }

    #[test]
    fn json_and_sse_four_classes_parity_and_legacy_write_compatibility() {
        let value = four();
        let json = extract(&value).unwrap();
        assert_eq!(
            (
                json.input_tokens,
                json.output_tokens,
                json.cache_read_tokens,
                json.cache_write_tokens
            ),
            (Some(60), Some(20), Some(30), Some(10))
        );
        assert_eq!(json.completeness, UsageCompleteness::Complete);
        let mut streamed = value.clone();
        streamed["choices"] = json!([]);
        let event = format!("data: {}\n\n", streamed);
        let mut collector = legacy::BoundedUsageEvents::default();
        for chunk in event.as_bytes().chunks(7) {
            collector.push(chunk);
        }
        assert_eq!(collector.finalize().unwrap().observation.unwrap(), json);
        let billed =
            legacy::platform_usage(legacy::extract_reported_usage(&value).as_ref(), 999, false);
        // Recognizing the new write extension is additive: old cache-read-only
        // partition and total billing remain exactly as published.
        assert_eq!(
            (
                billed.input_tokens,
                billed.cache_write_tokens,
                billed.tokens
            ),
            (70, 0, 120)
        );
    }

    #[test]
    fn gemini_native_thoughts_stream_and_legacy_fallback_are_explicit() {
        let value = json!({"responseId":"g1","modelVersion":"gemini-test","candidates":[{"finishReason":"STOP"}],
            "usageMetadata":{"promptTokenCount":100,"candidatesTokenCount":20,"thoughtsTokenCount":5,
            "cachedContentTokenCount":30,"totalTokenCount":125}});
        let o = extract(&value).unwrap();
        assert_eq!(
            (o.input_tokens, o.output_tokens, o.cache_write_tokens),
            (Some(70), Some(25), Some(0))
        );
        assert_eq!(o.completeness, UsageCompleteness::Complete);
        let mut stream = legacy::BoundedUsageEvents::default();
        stream.push(format!("data: {value}\n\n").as_bytes());
        let usage = stream.finalize().unwrap();
        assert_eq!(usage.observation.unwrap(), o);
        let mut stream = legacy::BoundedUsageEvents::default();
        stream.push(format!("data: {value}\n\n").as_bytes());
        let usage = stream.finalize().unwrap();
        assert!(!usage.has_legacy_usage());
        assert_eq!(legacy::platform_usage(Some(&usage), 80, true).tokens, 20);
        let absent=extract(&json!({"usageMetadata":{"promptTokenCount":1,"candidatesTokenCount":2,"totalTokenCount":3}})).unwrap();
        assert_eq!(absent.completeness, UsageCompleteness::Complete);
    }

    #[test]
    fn anthropic_message_and_ttl_subsets() {
        let plain =
            extract(&json!({"type":"message","usage":{"input_tokens":3,"output_tokens":2}}))
                .unwrap();
        assert_eq!(plain.adapter, UsageAdapter::Anthropic);
        assert_eq!(plain.input_tokens, Some(3));
        let value = json!({"type":"message","usage":{"input_tokens":3,"output_tokens":2,
            "cache_read_input_tokens":7,"cache_creation_input_tokens":11,
            "cache_creation":{"ephemeral_5m_input_tokens":4,"ephemeral_1h_input_tokens":7}}});
        let o = extract(&value).unwrap();
        assert_eq!(o.completeness, UsageCompleteness::Complete);
        assert_eq!(
            (o.input_tokens, o.cache_write_1h_tokens),
            (Some(3), Some(7))
        );
        let mut bad = value;
        bad["usage"]["cache_creation"]["ephemeral_1h_input_tokens"] = json!(8);
        assert!(extract(&bad).unwrap().invalid_counts);
    }

    #[test]
    fn native_gemini_json_array_stream_keeps_cumulative_observations_and_legacy_fallback() {
        let path = "/v1/projects/project/locations/us/publishers/google/models/gemini:streamGenerateContent";
        let start = json!({"responseId":"gemini-array","modelVersion":"gemini-test",
            "candidates":[{"content":{"parts":[{"text":"partial"}]}}],
            "usageMetadata":{"promptTokenCount":100,"candidatesTokenCount":0,
                "cachedContentTokenCount":30,"totalTokenCount":100}});
        let end = json!({"responseId":"gemini-array","modelVersion":"gemini-test",
            "candidates":[{"finishReason":"STOP"}],
            "usageMetadata":{"promptTokenCount":100,"candidatesTokenCount":20,
                "thoughtsTokenCount":5,"cachedContentTokenCount":30,"totalTokenCount":125}});
        let array = json!([start, end, end]);
        let bytes = serde_json::to_vec(&array).unwrap();
        let usage = legacy::usage_from_body(&bytes, path, true).unwrap();
        assert!(!usage.has_legacy_usage());
        let o = usage.observation.as_ref().unwrap();
        assert_eq!(o.completeness, UsageCompleteness::Complete);
        assert_eq!(
            (
                o.input_tokens,
                o.output_tokens,
                o.cache_read_tokens,
                o.cache_write_tokens
            ),
            (Some(70), Some(25), Some(30), Some(0))
        );
        assert_eq!(o.reported_total_tokens, Some(125));
        assert_eq!(o.reasoning_tokens, Some(5));
        assert_eq!(o.provider_response_id.as_deref(), Some("gemini-array"));
        assert_eq!(o.resolved_model.as_deref(), Some("gemini-test"));
        let measured = legacy::platform_usage(Some(&usage), bytes.len() as i64, true);
        assert_eq!(
            measured.tokens,
            legacy::estimate_tokens_from_bytes(bytes.len() as i64)
        );
        assert_eq!(
            (
                measured.input_tokens,
                measured.output_tokens,
                measured.cache_read_tokens,
                measured.cache_write_tokens
            ),
            (0, 0, 0, 0)
        );
        assert_eq!(legacy::platform_usage(Some(&usage), 0, false).tokens, 0);
        assert_eq!(
            legacy::usage_from_body(&bytes, path, false)
                .unwrap()
                .observation
                .unwrap()
                .outcome,
            UsageOutcome::Failed
        );
        // Generic arrays are not a new custom-response traversal contract.
        assert!(legacy::usage_from_body(&bytes, "/custom/generate", true).is_none());
        for body in [
            b"[]".as_slice(),
            b"[{}]",
            b"[17,null]",
            b"[{\"usageMetadata\":",
            &bytes[..bytes.len() - 1],
        ] {
            assert!(legacy::usage_from_body(body, path, true).is_none());
            assert_eq!(
                legacy::platform_usage(None, body.len() as i64, false)
                    .llm_observation
                    .unwrap()
                    .completeness,
                UsageCompleteness::Missing
            );
        }
        let malformed = json!([{"usageMetadata":{"promptTokenCount":"100","candidatesTokenCount":20},
            "candidates":[{"finishReason":"STOP"}]}]);
        let o = legacy::usage_from_body(&serde_json::to_vec(&malformed).unwrap(), path, true)
            .unwrap()
            .observation
            .unwrap();
        assert!(o.invalid_counts);
        assert_eq!(o.completeness, UsageCompleteness::Partial);
        let no_terminal = json!([start]);
        let o = legacy::usage_from_body(&serde_json::to_vec(&no_terminal).unwrap(), path, true)
            .unwrap()
            .observation
            .unwrap();
        assert_eq!(o.outcome, UsageOutcome::Interrupted);
    }

    #[test]
    fn malformed_missing_zero_and_native_context_are_distinct() {
        assert!(extract(&json!({"usage":{}})).is_none());
        let mut zero = extract(&json!({"usage":{"prompt_tokens":0,"completion_tokens":0,
            "prompt_tokens_details":{"cached_tokens":0}}}))
        .unwrap();
        assert_eq!(zero.reported_input_tokens, Some(0));
        assert_eq!(zero.input_tokens, None);
        native_openai_semantics(&mut zero, false);
        assert_eq!(zero.cache_write_tokens, None);
        native_openai_semantics(&mut zero, true);
        assert_eq!(zero.input_tokens, Some(0));
        for invalid in [
            json!(-1),
            json!(1.0),
            json!("1"),
            json!(null),
            json!(18446744073709551615_u64),
        ] {
            let mut value = four();
            value["usage"]["prompt_tokens"] = invalid;
            let o = extract(&value).unwrap();
            assert!(o.invalid_counts);
            assert_eq!(o.reported_input_tokens, None);
            assert_eq!(o.completeness, UsageCompleteness::Partial);
        }
    }

    #[test]
    fn control_totals_combined_subsets_and_overflow() {
        let mut value = four();
        value["usage"]["total_tokens"] = json!(1000);
        assert!(extract(&value).unwrap().invalid_counts);
        value["usage"]["total_tokens"] = json!(119);
        assert!(extract(&value).unwrap().invalid_counts);
        value = four();
        value["usage"]["prompt_tokens_details"]["audio_tokens"] = json!(60);
        value["usage"]["prompt_tokens_details"]["image_tokens"] = json!(50);
        assert!(extract(&value).unwrap().invalid_counts);
        let o = extract(&json!({"usageMetadata":{"promptTokenCount":0,
            "candidatesTokenCount":i64::MAX,"thoughtsTokenCount":1}}))
        .unwrap();
        assert!(o.invalid_counts);
        assert_eq!(o.output_tokens, None);
        let mut value = four();
        value["usage"]["prompt_tokens_details"]["audio_tokens"] = json!(40);
        let o = extract(&value).unwrap();
        assert_eq!(o.cache_read_audio_tokens, None); // Unknown intersection.
    }

    #[test]
    fn snapshots_deduplicate_and_interruption_failure_dominate() {
        let mut stream = legacy::BoundedUsageEvents::default();
        let mut value = four();
        value["choices"] = json!([{"delta":{}}]);
        let frame = format!("data: {value}\n\n");
        stream.push(frame.as_bytes());
        stream.push(frame.as_bytes());
        let o = stream.finalize().unwrap().observation.unwrap();
        assert_eq!(o.reported_input_tokens, Some(100));
        assert_eq!(o.outcome, UsageOutcome::Interrupted);
        let mut aggregate = None;
        merge(
            &mut aggregate,
            stream_observation(Some("response.failed"), &json!({})).unwrap(),
            false,
        );
        merge(
            &mut aggregate,
            stream_observation(Some("response.completed"), &json!({"response":four()})).unwrap(),
            false,
        );
        assert_eq!(aggregate.unwrap().outcome, UsageOutcome::Failed);
        for (status, expected) in [
            ("failed", UsageOutcome::Failed),
            ("cancelled", UsageOutcome::Interrupted),
        ] {
            let mut value = json!({"response":four()});
            value["response"]["status"] = json!(status);
            assert_eq!(
                stream_observation(Some("response.done"), &value)
                    .unwrap()
                    .outcome,
                expected
            );
            assert_eq!(
                stream_observation(Some("response.completed"), &value)
                    .unwrap()
                    .outcome,
                expected
            );
        }
    }

    #[test]
    fn image_completion_replays_do_not_double_observations() {
        let mut usage = legacy::extract_reported_usage(&four()).unwrap();
        usage.completed_image_index = Some(0);
        usage.images = 1;
        let mut aggregate = ReportedLlmUsageAccumulator::default();
        aggregate.observe_delta(usage.clone());
        aggregate.observe_delta(usage);
        let result = aggregate.finalize().unwrap();
        assert_eq!(result.observation.unwrap().reported_input_tokens, Some(100));
        assert_eq!(result.images, 1);
    }

    #[test]
    fn realtime_missing_classes_mixed_metadata_and_unreported_responses() {
        let mut collector = RealtimeLlmUsageCollector::new(true);
        let mut a = four();
        a["status"] = json!("completed");
        let mut b = a.clone();
        b["id"] = json!("resp_2");
        b["model"] = json!("other");
        b["service_tier"] = json!("default");
        b["usage"]["prompt_tokens_details"]
            .as_object_mut()
            .unwrap()
            .remove("cache_write_tokens");
        for value in [a, b] {
            collector.observe_downstream_text(
                &json!({"type":"response.done","response":value}).to_string(),
            );
        }
        let summary = collector.finalize();
        let o = summary.reported_usage.unwrap().observation.unwrap();
        assert_eq!(o.cache_write_tokens, None);
        assert_eq!(o.input_tokens, None);
        assert_eq!(o.reported_input_tokens, Some(200));
        assert_eq!(o.completeness, UsageCompleteness::Partial);
        assert_eq!(o.resolved_model, None);
        assert_eq!(o.processing_tier, None);
        assert_eq!(o.provider_response_id, None);
        assert_eq!(
            o.aggregation_scope,
            UsageAggregationScope::RealtimeConnection
        );
        let mut collector = RealtimeLlmUsageCollector::new(true);
        collector.observe_downstream_text(
            &json!({"type":"response.done","response":four()}).to_string(),
        );
        collector
            .observe_downstream_text(r#"{"type":"response.done","response":{"id":"missing"}}"#);
        let o = collector
            .finalize()
            .reported_usage
            .unwrap()
            .observation
            .unwrap();
        assert_eq!(o.provider_response_id, None);
        assert_eq!(o.reported_response_count, Some(1));
        assert_eq!(o.unreported_response_count, Some(1));
        assert_eq!(o.output_tokens, None);
    }

    #[test]
    fn realtime_zero_is_reported_even_when_legacy_policy_estimates() {
        let mut collector = RealtimeLlmUsageCollector::new(true);
        collector.observe_downstream_text(r#"{"type":"response.done","response":{"id":"zero","status":"completed","usage":{"input_tokens":0,"output_tokens":0,"total_tokens":0,"input_token_details":{"cached_tokens":0,"cache_write_tokens":0}}}}"#);
        let summary = collector.finalize();
        assert_eq!(summary.estimated_response_count, 1);
        assert!(summary.token_quantity() > 0);
        let o = summary.reported_usage.unwrap().observation.unwrap();
        assert_eq!(o.reported_response_count, Some(1));
        assert_eq!(o.unreported_response_count, Some(0));
        assert_eq!(o.outcome, UsageOutcome::Succeeded);
        assert_eq!(o.completeness, UsageCompleteness::Complete);
        assert_eq!(o.output_tokens, Some(0));
        assert_eq!(o.provider_response_id.as_deref(), Some("zero"));

        // Even inconsistent provider subsets cannot enter the old charged
        // breakdown when its total selects the established estimate branch.
        let mut collector = RealtimeLlmUsageCollector::new(true);
        collector.observe_downstream_text(r#"{"type":"response.done","response":{"id":"bad-zero","usage":{"total_tokens":0,"input_token_details":{"cached_tokens":7,"audio_tokens":3}}}}"#);
        let summary = collector.finalize();
        let usage = summary.reported_usage.unwrap();
        assert_eq!(usage.cached_tokens, 0);
        assert_eq!(usage.audio_input_tokens, 0);
        assert_eq!(usage.observation.unwrap().cache_read_tokens, Some(7));
    }

    #[test]
    fn json_terminal_outcomes_and_chat_mutation_capability_gate() {
        for (status, outcome) in [
            ("incomplete", UsageOutcome::Interrupted),
            ("cancelled", UsageOutcome::Interrupted),
            ("failed", UsageOutcome::Failed),
        ] {
            let mut value = four();
            value["status"] = json!(status);
            assert_eq!(extract(&value).unwrap().outcome, outcome);
        }
        let mut service = crate::models::downstream_service::test_helpers::dummy_service();
        service.slug = "custom".into();
        assert!(!legacy::service_supports_chat_usage(
            &service,
            "/v1/chat/completions"
        ));
        service
            .capabilities
            .get_or_insert_with(Default::default)
            .supports_stream_usage = true;
        assert!(!legacy::service_supports_chat_usage(
            &service,
            "/v1/chat/completions"
        ));
        service.inference = Some(crate::models::downstream_service::ServiceInference {
            wire_protocol:
                crate::models::downstream_service::InferenceWireProtocol::OpenaiCompletions,
            model_list: false,
            realtime: false,
            voice: None,
        });
        assert!(legacy::service_supports_chat_usage(
            &service,
            "/v1/chat/completions"
        ));
        for path in [
            "/models",
            "/responses",
            "/chat/completions/future",
            "/generate",
        ] {
            assert!(!legacy::service_supports_chat_usage(&service, path));
        }
    }

    #[test]
    fn provider_metadata_is_bounded_and_future_enum_values_are_tolerated() {
        let mut value = four();
        value["model"] = json!("private content with spaces");
        value["id"] = json!("x".repeat(257));
        let o = extract(&value).unwrap();
        assert_eq!(o.resolved_model, None);
        assert_eq!(o.provider_response_id, None);
        let o: LlmUsageObservation = serde_json::from_value(
            json!({"version":2,"adapter":"new_provider","source":"future","outcome":"future"}),
        )
        .unwrap();
        assert_eq!(o.adapter, UsageAdapter::Unknown);
    }
}

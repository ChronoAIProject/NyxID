//! Strict numeric checkpoint parsing; legacy finalization keeps its own parser.
use crate::models::{
    platform_model_tariff::NativeUsageEvidence as Evidence,
    usage_checkpoint::UsageCheckpointProtocol as Protocol,
};
use serde_json::Value;

#[derive(Default)]
pub(super) struct Parser {
    messages: Option<Evidence>,
    response_sequence: Option<i64>,
}

fn count(value: &Value, pointers: &[&str]) -> Result<Option<i64>, ()> {
    for pointer in pointers {
        if let Some(raw) = value.pointer(pointer) {
            return raw.as_i64().filter(|n| *n >= 0).map(Some).ok_or(());
        }
    }
    Ok(None)
}

pub(super) fn evidence(value: &Value, protocol: Protocol) -> Result<Evidence, ()> {
    let mut e = Evidence {
        input: count(
            value,
            &["/input_tokens", "/prompt_tokens", "/promptTokenCount"],
        )?,
        output: count(
            value,
            &[
                "/output_tokens",
                "/completion_tokens",
                "/candidatesTokenCount",
            ],
        )?,
        total: count(value, &["/total_tokens", "/totalTokenCount"])?,
        cache_read: count(
            value,
            &[
                "/cache_read_input_tokens",
                "/input_tokens_details/cached_tokens",
                "/prompt_tokens_details/cached_tokens",
                "/cachedContentTokenCount",
            ],
        )?,
        cache_write: count(value, &["/cache_creation_input_tokens"])?,
        reasoning: count(
            value,
            &[
                "/output_tokens_details/reasoning_tokens",
                "/completion_tokens_details/reasoning_tokens",
                "/thoughtsTokenCount",
            ],
        )?,
        input_audio: count(
            value,
            &[
                "/input_tokens_details/audio_tokens",
                "/prompt_tokens_details/audio_tokens",
            ],
        )?,
        output_audio: count(
            value,
            &[
                "/output_tokens_details/audio_tokens",
                "/completion_tokens_details/audio_tokens",
            ],
        )?,
        input_image: count(
            value,
            &[
                "/input_tokens_details/image_tokens",
                "/prompt_tokens_details/image_tokens",
            ],
        )?,
        output_image: count(
            value,
            &[
                "/output_tokens_details/image_tokens",
                "/completion_tokens_details/image_tokens",
            ],
        )?,
        requests: Some(1),
        ..Default::default()
    };
    if protocol == Protocol::Messages {
        // Claude input is exclusive of its separate cache classes. Its total
        // in the ordinary billing path is the exact input + output sum.
        e.total = None;
    }
    if e.input.is_none() && e.output.is_none() && e.total.is_none() {
        return Err(());
    }
    Ok(e)
}

impl Parser {
    pub(super) fn event(
        &mut self,
        event_type: Option<&str>,
        value: &Value,
    ) -> Result<Option<(Protocol, Evidence, bool)>, ()> {
        if event_type
            .or_else(|| value["type"].as_str())
            .is_some_and(|kind| kind.starts_with("response."))
            && let Some(sequence) = value.get("sequence_number")
        {
            let sequence = sequence.as_i64().filter(|n| *n >= 0).ok_or(())?;
            if self
                .response_sequence
                .is_some_and(|previous| sequence <= previous)
            {
                return Ok(None);
            }
            self.response_sequence = Some(sequence);
        }
        let report = self.event_inner(event_type, value);
        if report.is_none() {
            // A malformed usage report must not leave an older final snapshot
            // eligible for recovery. Null usage on text chunks is ordinary.
            if [
                "/usage",
                "/message/usage",
                "/response/usage",
                "/usageMetadata",
            ]
            .iter()
            .any(|p| value.pointer(p).is_some_and(|v| !v.is_null()))
            {
                return Err(());
            }
        }
        Ok(report)
    }

    fn event_inner(
        &mut self,
        event_type: Option<&str>,
        value: &Value,
    ) -> Option<(Protocol, Evidence, bool)> {
        let kind = event_type.or_else(|| value["type"].as_str());
        let (protocol, raw, final_report) = match kind {
            Some("message_start") => (Protocol::Messages, value.pointer("/message/usage")?, false),
            Some("message_delta") => (
                Protocol::Messages,
                value.get("usage")?,
                value
                    .pointer("/delta/stop_reason")
                    .is_some_and(Value::is_string),
            ),
            Some("response.completed" | "response.incomplete") => {
                (Protocol::Responses, value.pointer("/response/usage")?, true)
            }
            Some("usage.delta" | "response.usage.delta") => (
                Protocol::Unknown,
                value.get("usage").unwrap_or(value),
                false,
            ),
            _ if value.get("usageMetadata").is_some() => {
                (Protocol::Gemini, &value["usageMetadata"], false)
            }
            _ => (
                if value["choices"].is_array() {
                    Protocol::ChatCompletions
                } else {
                    Protocol::Unknown
                },
                value
                    .get("usage")
                    .or_else(|| value.pointer("/response/usage"))?,
                value["choices"].as_array().is_some_and(|choices| {
                    choices.is_empty()
                        || (choices.len() == 1 && choices[0]["finish_reason"].is_string())
                }),
            ),
        };
        let mut e = evidence(raw, protocol).ok()?;
        if matches!(
            kind,
            Some("image_generation.completed" | "image_edit.completed")
        ) {
            // This is one completed image, not proof that every requested
            // image completed. Retain the latest report without request finality.
            e.images = Some(1);
            e.output_image = e.output;
        }
        if protocol == Protocol::Messages {
            if kind == Some("message_start") {
                self.messages = Some(e.clone());
            } else {
                let previous = self.messages.as_mut()?;
                // Deltas contain cumulative counters, not increments. Retain
                // absent classes and replace supplied ones, including zero.
                macro_rules! replace {
                    ($($field:ident),*) => { $(if e.$field.is_some() { previous.$field = e.$field; })* };
                }
                replace!(
                    input,
                    output,
                    cache_read,
                    cache_write,
                    input_audio,
                    output_audio,
                    input_image,
                    output_image,
                    reasoning
                );
                e = previous.clone();
            }
            e.total = e.input.zip(e.output).and_then(|(i, o)| i.checked_add(o));
        }
        // A syntactic end marker alone cannot make missing counters final.
        let complete = e.input.is_some() && e.output.is_some() && e.total.is_some();
        Some((
            protocol,
            e,
            final_report && complete && protocol != Protocol::Unknown,
        ))
    }
}

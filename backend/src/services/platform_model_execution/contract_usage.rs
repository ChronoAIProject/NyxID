//! Closed v2 wire evidence. Unknown usage is an unsettled obligation, including
//! unknown zero-valued fields. Historical v1 tariff semantics are unchanged.
use super::adapters::{self, Native, invalid_response};
use crate::{errors::AppResult, models::platform_model_tariff::UsageContract};
use serde_json::Value;

/// Billing evidence must not depend on last-key-wins JSON decoding. This
/// visitor applies recursively, including escaped spellings of duplicate keys.
pub(super) fn parse(bytes: &[u8]) -> AppResult<Value> {
    struct Unique(Value);
    impl<'de> serde::Deserialize<'de> for Unique {
        fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
            struct Visitor;
            impl<'de> serde::de::Visitor<'de> for Visitor {
                type Value = Unique;
                fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                    f.write_str("unambiguous JSON")
                }
                fn visit_unit<E: serde::de::Error>(self) -> Result<Unique, E> {
                    Ok(Unique(Value::Null))
                }
                fn visit_bool<E: serde::de::Error>(self, v: bool) -> Result<Unique, E> {
                    Ok(Unique(Value::Bool(v)))
                }
                fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<Unique, E> {
                    Ok(Unique(Value::Number(v.into())))
                }
                fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<Unique, E> {
                    Ok(Unique(Value::Number(v.into())))
                }
                fn visit_f64<E: serde::de::Error>(self, v: f64) -> Result<Unique, E> {
                    serde_json::Number::from_f64(v)
                        .map(|n| Unique(Value::Number(n)))
                        .ok_or_else(|| E::custom("invalid JSON number"))
                }
                fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Unique, E> {
                    Ok(Unique(Value::String(v.into())))
                }
                fn visit_string<E: serde::de::Error>(self, v: String) -> Result<Unique, E> {
                    Ok(Unique(Value::String(v)))
                }
                fn visit_seq<A: serde::de::SeqAccess<'de>>(
                    self,
                    mut seq: A,
                ) -> Result<Unique, A::Error> {
                    let mut values = Vec::new();
                    while let Some(Unique(value)) = seq.next_element()? {
                        values.push(value);
                    }
                    Ok(Unique(Value::Array(values)))
                }
                fn visit_map<A: serde::de::MapAccess<'de>>(
                    self,
                    mut map: A,
                ) -> Result<Unique, A::Error> {
                    let mut values = serde_json::Map::new();
                    while let Some(key) = map.next_key::<String>()? {
                        if values.contains_key(&key) {
                            return Err(serde::de::Error::custom("duplicate JSON key"));
                        }
                        let Unique(value) = map.next_value()?;
                        values.insert(key, value);
                    }
                    Ok(Unique(Value::Object(values)))
                }
            }
            deserializer.deserialize_any(Visitor)
        }
    }
    serde_json::from_slice::<Unique>(bytes)
        .map(|v| v.0)
        .map_err(|_| invalid_response())
}

fn object<'a>(v: &'a Value, allowed: &[&str]) -> AppResult<&'a serde_json::Map<String, Value>> {
    let o = v.as_object().ok_or_else(invalid_response)?;
    if o.keys().any(|k| !allowed.contains(&k.as_str())) {
        return Err(invalid_response());
    }
    Ok(o)
}
fn counters(v: &Value, allowed: &[&str]) -> AppResult<()> {
    for n in object(v, allowed)?.values() {
        if n.as_i64().is_none_or(|n| n < 0) {
            return Err(invalid_response());
        }
    }
    Ok(())
}
fn optional_counters(v: &Value, key: &str, allowed: &[&str]) -> AppResult<()> {
    if let Some(v) = v.get(key) {
        counters(v, allowed)?;
    }
    Ok(())
}
fn zero_counters(v: &Value, allowed: &[&str]) -> AppResult<()> {
    counters(v, allowed)?;
    if v.as_object()
        .unwrap()
        .values()
        .any(|v| v.as_i64() != Some(0))
    {
        return Err(invalid_response());
    }
    Ok(())
}
fn metadata(v: &Value, key: &str, values: &[&str]) -> AppResult<()> {
    if let Some(v) = v.get(key)
        && v.as_str().is_none_or(|s| !values.contains(&s))
    {
        return Err(invalid_response());
    }
    Ok(())
}

/// Validates partial snapshots too, before a later snapshot can hide a new cost.
pub(super) fn validate(native: Native, usage: &Value) -> AppResult<()> {
    let numbers: &[&str] = match native {
        Native::Responses => {
            object(
                usage,
                &[
                    "input_tokens",
                    "output_tokens",
                    "total_tokens",
                    "input_tokens_details",
                    "output_tokens_details",
                ],
            )?;
            if let Some(details) = usage.get("input_tokens_details") {
                object(
                    details,
                    &[
                        "cached_tokens",
                        "cache_write_tokens",
                        "cache_write_tokens_details",
                    ],
                )?;
                for name in ["cached_tokens", "cache_write_tokens"] {
                    if details
                        .get(name)
                        .is_some_and(|v| v.as_i64().is_none_or(|n| n < 0))
                    {
                        return Err(invalid_response());
                    }
                }
                optional_counters(
                    details,
                    "cache_write_tokens_details",
                    &["ttl_30m_tokens", "ttl_12h_tokens"],
                )?;
            }
            optional_counters(usage, "output_tokens_details", &["reasoning_tokens"])?;
            &["input_tokens", "output_tokens", "total_tokens"]
        }
        Native::Claude => {
            object(
                usage,
                &[
                    "input_tokens",
                    "output_tokens",
                    "cache_read_input_tokens",
                    "cache_creation_input_tokens",
                    "cache_creation",
                    "output_tokens_details",
                    "server_tool_use",
                    "service_tier",
                    "inference_geo",
                    "speed",
                ],
            )?;
            optional_counters(
                usage,
                "cache_creation",
                &["ephemeral_5m_input_tokens", "ephemeral_1h_input_tokens"],
            )?;
            optional_counters(usage, "output_tokens_details", &["thinking_tokens"])?;
            if let Some(tools) = usage.get("server_tool_use") {
                zero_counters(tools, &["web_search_requests", "web_fetch_requests"])?;
            }
            metadata(usage, "service_tier", &["standard"])?;
            metadata(usage, "speed", &["standard"])?;
            metadata(usage, "inference_geo", &["global", "us", "eu"])?;
            &[
                "input_tokens",
                "output_tokens",
                "cache_read_input_tokens",
                "cache_creation_input_tokens",
            ]
        }
        Native::Gemini => {
            object(
                usage,
                &[
                    "promptTokenCount",
                    "candidatesTokenCount",
                    "totalTokenCount",
                    "cachedContentTokenCount",
                    "thoughtsTokenCount",
                    "toolUsePromptTokenCount",
                    "promptTokensDetails",
                    "cacheTokensDetails",
                    "candidatesTokensDetails",
                    "toolUsePromptTokensDetails",
                    "trafficType",
                ],
            )?;
            for key in [
                "promptTokensDetails",
                "cacheTokensDetails",
                "candidatesTokensDetails",
                "toolUsePromptTokensDetails",
            ] {
                if let Some(details) = usage.get(key) {
                    let mut seen = std::collections::HashSet::new();
                    let mut sum = 0_i64;
                    for d in details.as_array().ok_or_else(invalid_response)? {
                        object(d, &["modality", "tokenCount"])?;
                        let modality = d["modality"].as_str().ok_or_else(invalid_response)?;
                        if !matches!(modality, "TEXT" | "IMAGE")
                            || !seen.insert(modality)
                            || matches!(
                                key,
                                "candidatesTokensDetails" | "toolUsePromptTokensDetails"
                            ) && modality != "TEXT"
                        {
                            return Err(invalid_response());
                        }
                        let n = d["tokenCount"]
                            .as_i64()
                            .filter(|n| *n >= 0)
                            .ok_or_else(invalid_response)?;
                        sum = sum.checked_add(n).ok_or_else(invalid_response)?;
                    }
                    let aggregate = match key {
                        "promptTokensDetails" => "promptTokenCount",
                        "cacheTokensDetails" => "cachedContentTokenCount",
                        "candidatesTokensDetails" => "candidatesTokenCount",
                        _ => "toolUsePromptTokenCount",
                    };
                    if usage[aggregate].as_i64() != Some(sum) {
                        return Err(invalid_response());
                    }
                }
            }
            metadata(
                usage,
                "trafficType",
                &["ON_DEMAND", "PROVISIONED_THROUGHPUT"],
            )?;
            &[
                "promptTokenCount",
                "candidatesTokenCount",
                "totalTokenCount",
                "cachedContentTokenCount",
                "thoughtsTokenCount",
                "toolUsePromptTokenCount",
            ]
        }
        Native::GptOss => return Err(invalid_response()),
    };
    for key in numbers {
        if usage
            .get(*key)
            .is_some_and(|v| v.as_i64().is_none_or(|n| n < 0))
        {
            return Err(invalid_response());
        }
    }
    Ok(())
}

/// Costs outside the base-model usage object cannot silently disappear either.
pub(super) fn envelope(native: Native, body: &Value) -> AppResult<()> {
    if native == Native::Responses && body.get("tool_usage").is_some() {
        // This entrance never enables provider-hosted tools. Even a new zero
        // shape needs review; a provider may omit incomplete tool records.
        return Err(invalid_response());
    }
    Ok(())
}

/// Non-stream responses must carry terminal evidence too. A cumulative usage
/// object on a queued/in-progress response is not the final billed quantity.
pub(super) fn final_response(native: Native, body: &Value) -> AppResult<()> {
    envelope(native, body)?;
    let terminal = match native {
        Native::Responses => matches!(body["status"].as_str(), Some("completed" | "incomplete")),
        Native::Claude => {
            body["type"] == "message"
                && matches!(
                    body["stop_reason"].as_str(),
                    Some(
                        "end_turn"
                            | "max_tokens"
                            | "stop_sequence"
                            | "tool_use"
                            | "pause_turn"
                            | "refusal"
                    )
                )
        }
        Native::Gemini => {
            body["candidates"].as_array().is_some_and(|c| c.len() == 1)
                && matches!(
                    body["candidates"][0]["finishReason"].as_str(),
                    Some(
                        "STOP"
                            | "MAX_TOKENS"
                            | "SAFETY"
                            | "RECITATION"
                            | "OTHER"
                            | "BLOCKLIST"
                            | "PROHIBITED_CONTENT"
                            | "SPII"
                            | "MALFORMED_FUNCTION_CALL"
                            | "UNEXPECTED_TOOL_CALL"
                    )
                )
        }
        Native::GptOss => false,
    };
    if !terminal {
        return Err(invalid_response());
    }
    Ok(())
}

pub(super) fn evidence(
    native: Native,
    usage: &Value,
    c: &UsageContract,
) -> AppResult<crate::services::billing::model_usage::Evidence> {
    if !super::certification::family_contract(c) {
        return adapters::evidence(native, usage);
    }
    validate(native, usage)?;
    let mut e = adapters::evidence(native, usage).map_err(|_| invalid_response())?;
    // No missing priced field is turned into zero for the new contracts.
    match native {
        Native::Responses => {
            e.cache_write = usage
                .pointer("/input_tokens_details/cache_write_tokens")
                .and_then(Value::as_i64);
            if e.total.is_none() || e.reasoning.is_none() {
                return Err(invalid_response());
            }
        }
        Native::Claude => {
            e.cache_write = usage["cache_creation_input_tokens"].as_i64();
            if let Some(n) = usage.pointer("/output_tokens_details/thinking_tokens") {
                let n = n.as_i64().ok_or_else(invalid_response)?;
                if e.output.is_none_or(|output| n > output) {
                    return Err(invalid_response());
                }
                // Aggregate billing only; this observability counter is inclusive.
                e.reasoning = Some(n);
            }
        }
        Native::Gemini => {
            e.tool_input = usage["toolUsePromptTokenCount"].as_i64();
            if e.total.is_none()
                || e.reasoning.is_none()
                || e.tool_input.is_none()
                || !c.bill_tool_input && e.tool_input != Some(0)
            {
                return Err(invalid_response());
            }
        }
        Native::GptOss => return Err(invalid_response()),
    }
    if e.input.is_none()
        || e.output.is_none()
        || e.cache_read.is_none()
        || native != Native::Gemini && e.cache_write.is_none()
    {
        return Err(invalid_response());
    }
    // A known zero sum of nonnegative writes proves each configured TTL is
    // zero. An absent aggregate or a positive sum never supplies partitions.
    if e.cache_write == Some(0) {
        for ttl in &c.cache_write_ttls {
            e.writes.entry(*ttl).or_insert(0);
        }
    }
    Ok(e)
}

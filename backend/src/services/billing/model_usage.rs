//! Strict model evidence. Legacy parsers intentionally retain their old behavior.
use std::collections::{BTreeMap, BTreeSet};

use crate::{
    errors::{AppError, AppResult},
    models::{
        platform_model_tariff::*,
        service_billing::{AudioTokens, BillingMetric, ImageTokens, PlatformUsage, TokenBreakdown},
    },
};

fn invalid() -> AppError {
    AppError::ValidationError(
        "Model usage evidence is missing, inconsistent, unsupported or outside its admitted bound"
            .into(),
    )
}
fn add(a: i64, b: i64) -> AppResult<i64> {
    a.checked_add(b).ok_or_else(invalid)
}
fn sub(a: i64, b: i64) -> AppResult<i64> {
    a.checked_sub(b).filter(|v| *v >= 0).ok_or_else(invalid)
}
fn count(n: Option<i64>) -> AppResult<i64> {
    n.filter(|n| *n >= 0).ok_or_else(invalid)
}
fn applicable(n: Option<i64>, enabled: bool) -> AppResult<i64> {
    if enabled {
        count(n)
    } else if n.is_none_or(|n| n == 0) {
        Ok(0)
    } else {
        Err(invalid())
    }
}

pub use crate::models::platform_model_tariff::NativeUsageEvidence as Evidence;

pub fn metric(selector: Selector) -> BillingMetric {
    match selector {
        Selector::Input => BillingMetric::InputTokens,
        Selector::Output => BillingMetric::OutputTokens,
        Selector::Reasoning => BillingMetric::ReasoningTokens,
        Selector::CacheRead | Selector::CachedModality { .. } => BillingMetric::CacheReadTokens,
        Selector::CacheWrite { .. } => BillingMetric::CacheWriteTokens,
        Selector::ImageInput => BillingMetric::ImageInputTokens,
        Selector::ImageOutput => BillingMetric::ImageOutputTokens,
        Selector::AudioInput => BillingMetric::AudioInputTokens,
        Selector::AudioOutput => BillingMetric::AudioOutputTokens,
        Selector::Images => BillingMetric::Images,
        Selector::Requests => BillingMetric::Requests,
    }
}

/// A complete partition is derived from the contract, not guessed from prices.
pub fn selectors(c: &UsageContract) -> AppResult<BTreeSet<Selector>> {
    let version = match c.family {
        UsageFamily::Openai => "openai.responses.v1",
        UsageFamily::Claude => "anthropic.messages.v1",
        UsageFamily::Gemini => "gemini.generate.v1",
        UsageFamily::OpenMaas => "open_maas.certified.v1",
    };
    if (c.version != version
        && (c.family == UsageFamily::OpenMaas || c.version != version.replace(".v1", ".v2")))
        || c.version.len() > 80
        || c.fixture_sha256.len() != 64
        || !c.fixture_sha256.bytes().all(|b| b.is_ascii_hexdigit())
        || c.cache_write_ttls.len() > 2
        || c.cache_write_ttls.iter().collect::<BTreeSet<_>>().len() != c.cache_write_ttls.len()
        || c.input_modalities.iter().collect::<BTreeSet<_>>().len() != c.input_modalities.len()
        || c.output_modalities.iter().collect::<BTreeSet<_>>().len() != c.output_modalities.len()
        || (!c.tokens
            && (c.cache_read
                || !c.cache_write_ttls.is_empty()
                || c.split_reasoning
                || c.bill_tool_input
                || !c.input_modalities.is_empty()
                || !c.output_modalities.is_empty()))
        || (c.images && c.tokens)
        || (!c.cache_write_ttls.is_empty() && !c.input_modalities.is_empty())
        || (c.split_reasoning && !c.output_modalities.is_empty())
        || (c.bill_tool_input
            && (c.family != UsageFamily::Gemini || !c.input_modalities.is_empty()))
        || (c.family == UsageFamily::Claude
            && (c.split_reasoning
                || !c.input_modalities.is_empty()
                || !c.output_modalities.is_empty()))
    {
        return Err(invalid());
    }
    for ttl in &c.cache_write_ttls {
        if !matches!(
            (c.family, ttl),
            (
                UsageFamily::Claude,
                CacheTtl::FiveMinutes | CacheTtl::OneHour
            ) | (
                UsageFamily::Openai,
                CacheTtl::ThirtyMinutes | CacheTtl::TwelveHours
            ) | (UsageFamily::OpenMaas, _)
        ) {
            return Err(invalid());
        }
    }
    let mut out = BTreeSet::new();
    if c.tokens {
        out.extend([Selector::Input, Selector::Output]);
    }
    if c.cache_read {
        out.insert(Selector::CacheRead);
    }
    out.extend(
        c.cache_write_ttls
            .iter()
            .map(|ttl| Selector::CacheWrite { ttl: *ttl }),
    );
    if c.split_reasoning {
        out.insert(Selector::Reasoning);
    }
    for m in &c.input_modalities {
        out.insert(match m {
            Modality::Image => Selector::ImageInput,
            Modality::Audio => Selector::AudioInput,
        });
        if c.cache_read {
            out.insert(Selector::CachedModality { modality: *m });
        }
    }
    for m in &c.output_modalities {
        out.insert(match m {
            Modality::Image => Selector::ImageOutput,
            Modality::Audio => Selector::AudioOutput,
        });
    }
    if c.images {
        out.insert(Selector::Images);
    }
    if c.requests {
        out.insert(Selector::Requests);
    }
    if out.is_empty() {
        return Err(invalid());
    }
    Ok(out)
}

pub fn normalize(c: &UsageContract, e: &Evidence) -> AppResult<BTreeMap<Selector, i64>> {
    let expected = selectors(c)?;
    // Validate every supplied counter, even one unused by an aggregate tariff.
    for n in [
        e.input,
        e.output,
        e.total,
        e.cache_read,
        e.cache_write,
        e.reasoning,
        e.tool_input,
        e.input_image,
        e.input_audio,
        e.output_image,
        e.output_audio,
        e.cached_image,
        e.cached_audio,
        e.images,
        e.requests,
    ]
    .into_iter()
    .flatten()
    .chain(e.writes.values().copied())
    {
        if n < 0 {
            return Err(invalid());
        }
    }
    let mut v = BTreeMap::new();
    if c.tokens {
        let input = count(e.input)?;
        let output = count(e.output)?;
        let read = applicable(e.cache_read, c.cache_read)?;
        let write = applicable(e.cache_write, !c.cache_write_ttls.is_empty())?;
        if e.writes.keys().any(|t| !c.cache_write_ttls.contains(t)) {
            return Err(invalid());
        }
        let mut writes = 0;
        for ttl in &c.cache_write_ttls {
            let n = count(e.writes.get(ttl).copied())?;
            writes = add(writes, n)?;
            v.insert(Selector::CacheWrite { ttl: *ttl }, n);
        }
        if writes != write {
            return Err(invalid());
        }
        let reasoning = if c.family == UsageFamily::Gemini || c.split_reasoning {
            count(e.reasoning)?
        } else {
            e.reasoning.unwrap_or(0)
        };
        if c.family != UsageFamily::Gemini && reasoning > output {
            return Err(invalid());
        }
        let tool = applicable(e.tool_input, c.family == UsageFamily::Gemini)?;
        let total_input = if c.family == UsageFamily::Claude {
            add(add(input, read)?, write)?
        } else {
            input
        };
        let total_output = if c.family == UsageFamily::Gemini {
            add(output, reasoning)?
        } else {
            output
        };
        if let Some(total) = e.total
            && total != add(add(total_input, total_output)?, tool)?
        {
            return Err(invalid());
        }
        if add(e.input_image.unwrap_or(0), e.input_audio.unwrap_or(0))? > input
            || add(e.output_image.unwrap_or(0), e.output_audio.unwrap_or(0))? > total_output
            || add(e.cached_image.unwrap_or(0), e.cached_audio.unwrap_or(0))? > read
        {
            return Err(invalid());
        }
        for (cached, all) in [
            (e.cached_image, e.input_image),
            (e.cached_audio, e.input_audio),
        ] {
            if matches!((cached,all),(Some(a),Some(b)) if a>b) {
                return Err(invalid());
            }
        }
        let mut ordinary = if c.family == UsageFamily::Claude {
            input
        } else {
            sub(sub(input, read)?, write)?
        };
        if c.bill_tool_input {
            ordinary = add(ordinary, tool)?;
        }
        let mut cache = read;
        let mut input_modalities = 0;
        let mut cache_modalities = 0;
        for (modality, n, intersection, selector) in [
            (
                Modality::Image,
                e.input_image,
                e.cached_image,
                Selector::ImageInput,
            ),
            (
                Modality::Audio,
                e.input_audio,
                e.cached_audio,
                Selector::AudioInput,
            ),
        ] {
            if n.is_some_and(|n| n > input) || intersection.is_some_and(|n| n > read) {
                return Err(invalid());
            }
            if c.input_modalities.contains(&modality) {
                let n = count(n)?;
                let cached = if c.cache_read {
                    count(intersection)?
                } else {
                    applicable(intersection, false)?
                };
                let uncached = sub(n, cached)?;
                ordinary = sub(ordinary, uncached)?;
                cache = sub(cache, cached)?;
                input_modalities = add(input_modalities, n)?;
                cache_modalities = add(cache_modalities, cached)?;
                v.insert(selector, uncached);
                if c.cache_read {
                    v.insert(Selector::CachedModality { modality }, cached);
                }
            }
        }
        if input_modalities > input || cache_modalities > read {
            return Err(invalid());
        }
        v.insert(Selector::Input, ordinary);
        if c.cache_read {
            v.insert(Selector::CacheRead, cache);
        }
        let mut ordinary_output = if c.split_reasoning {
            sub(total_output, reasoning)?
        } else {
            total_output
        };
        if c.split_reasoning {
            v.insert(Selector::Reasoning, reasoning);
        }
        for (modality, n, selector) in [
            (Modality::Image, e.output_image, Selector::ImageOutput),
            (Modality::Audio, e.output_audio, Selector::AudioOutput),
        ] {
            if n.is_some_and(|n| n > total_output) {
                return Err(invalid());
            }
            if c.output_modalities.contains(&modality) {
                // These contracts don't certify reasoning/modality intersections.
                if reasoning != 0 {
                    return Err(invalid());
                }
                let n = count(n)?;
                ordinary_output = sub(ordinary_output, n)?;
                v.insert(selector, n);
            }
        }
        v.insert(Selector::Output, ordinary_output);
    } else if [
        e.input,
        e.output,
        e.cache_read,
        e.cache_write,
        e.reasoning,
        e.tool_input,
        e.input_image,
        e.input_audio,
        e.output_image,
        e.output_audio,
        e.cached_image,
        e.cached_audio,
        e.total,
    ]
    .into_iter()
    .flatten()
    .any(|n| n != 0)
        || !e.writes.is_empty()
    {
        return Err(invalid());
    }
    let images = applicable(e.images, c.images)?;
    if c.images {
        v.insert(Selector::Images, images);
    }
    // Request count is authoritative attempt metadata, not a count of tool calls.
    if e.requests.is_some_and(|n| n != 1) {
        return Err(invalid());
    }
    if c.requests {
        let requests = count(e.requests)?;
        if requests != 1 {
            return Err(invalid());
        }
        v.insert(Selector::Requests, requests);
    }
    if v.keys().copied().collect::<BTreeSet<_>>() != expected {
        return Err(invalid());
    }
    Ok(v)
}

pub fn charge_vector(t: &ModelTariff, e: &Evidence) -> AppResult<PlatformUsage> {
    let normalized = normalize(&t.contract, e)?;
    let mut quantities = BTreeMap::new();
    for c in &t.components {
        let n = *normalized.get(&c.selector).ok_or_else(invalid)?;
        if n > c.max_quantity || quantities.insert(c.lago_metric_code.clone(), n).is_some() {
            return Err(invalid());
        }
    }
    if quantities.len() != normalized.len() {
        return Err(invalid());
    }
    // Exactly one coordinator carries request-level counters. Component rows
    // carry only their financial metric/quantity and public model.
    let total_input = if t.contract.family == UsageFamily::Claude {
        add(
            add(e.input.unwrap_or(0), e.cache_read.unwrap_or(0))?,
            e.cache_write.unwrap_or(0),
        )?
    } else {
        e.input.unwrap_or(0)
    };
    let total_output = if t.contract.family == UsageFamily::Gemini {
        add(e.output.unwrap_or(0), e.reasoning.unwrap_or(0))?
    } else {
        e.output.unwrap_or(0)
    };
    let input_bound = t
        .bounds
        .max_input_tokens
        .checked_mul(t.bounds.max_tool_iterations)
        .ok_or_else(invalid)?;
    let output_bound = t
        .bounds
        .max_output_tokens
        .checked_mul(t.bounds.max_candidates)
        .and_then(|n| n.checked_mul(t.bounds.max_tool_iterations))
        .ok_or_else(invalid)?;
    if add(total_input, e.tool_input.unwrap_or(0))? > input_bound
        || total_output > output_bound
        || e.images.unwrap_or(0) > t.bounds.max_images
    {
        return Err(invalid());
    }
    Ok(PlatformUsage {
        requests: 1,
        tokens: add(add(total_input, total_output)?, e.tool_input.unwrap_or(0))?,
        // Preserve native accounting here (Claude input excludes caches;
        // Gemini candidates exclude thoughts). Only the charge vector is priced.
        token_breakdown: t.contract.tokens.then_some(TokenBreakdown {
            prompt_tokens: e.input.unwrap_or(0),
            completion_tokens: e.output.unwrap_or(0),
            cached_tokens: e.cache_read.unwrap_or(0),
            cache_creation_tokens: e.cache_write.unwrap_or(0),
        }),
        audio_tokens: (e.input_audio.is_some() || e.output_audio.is_some()).then_some(
            AudioTokens {
                input_tokens: e.input_audio.unwrap_or(0),
                output_tokens: e.output_audio.unwrap_or(0),
            },
        ),
        image_tokens: (e.input_image.is_some() || e.output_image.is_some()).then_some(
            ImageTokens {
                input_tokens: e.input_image.unwrap_or(0),
                output_tokens: e.output_image.unwrap_or(0),
            },
        ),
        model_charges: Some(Box::new(ModelChargeVector {
            native_usage: e.clone(),
            tariff_id: t.id.clone(),
            public_model: t.public_model.clone(),
            quantities,
        })),
        ..Default::default()
    })
}

/// Strict wire extraction for the certified Responses/Claude contracts. Open
/// MaaS adapters must supply Evidence from their own certified fixtures; no
/// OpenAI compatibility inference is made from protocol/SDK names.
#[allow(dead_code)] // Adapter entry point; provider execution lands in PR 3.
pub fn parse_usage(family: UsageFamily, value: &serde_json::Value) -> AppResult<Evidence> {
    fn get(v: &serde_json::Value, p: &str) -> AppResult<Option<i64>> {
        v.pointer(p)
            .map(|n| n.as_i64().filter(|n| *n >= 0).ok_or_else(invalid))
            .transpose()
    }
    let mut e = Evidence::default();
    let paths = match family {
        UsageFamily::Openai => [
            ("/input_tokens", "input"),
            ("/output_tokens", "output"),
            ("/total_tokens", "total"),
            ("/input_tokens_details/cached_tokens", "read"),
            ("/input_tokens_details/cache_write_tokens", "write"),
            ("/output_tokens_details/reasoning_tokens", "reasoning"),
        ],
        UsageFamily::Claude => [
            ("/input_tokens", "input"),
            ("/output_tokens", "output"),
            ("/total_tokens", "total"),
            ("/cache_read_input_tokens", "read"),
            ("/cache_creation_input_tokens", "write"),
            ("/reasoning_tokens", "reasoning"),
        ],
        UsageFamily::Gemini => [
            ("/promptTokenCount", "input"),
            ("/candidatesTokenCount", "output"),
            ("/totalTokenCount", "total"),
            ("/cachedContentTokenCount", "read"),
            ("/cacheWriteTokenCount", "write"),
            ("/thoughtsTokenCount", "reasoning"),
        ],
        UsageFamily::OpenMaas => return Err(invalid()),
    };
    for (path, key) in paths {
        let n = get(value, path)?;
        match key {
            "input" => e.input = n,
            "output" => e.output = n,
            "total" => e.total = n,
            "read" => e.cache_read = n,
            "write" => e.cache_write = n,
            _ => e.reasoning = n,
        }
    }
    for (ttl, path) in match family {
        UsageFamily::Openai => vec![
            (
                CacheTtl::ThirtyMinutes,
                "/input_tokens_details/cache_write_tokens_details/ttl_30m_tokens",
            ),
            (
                CacheTtl::TwelveHours,
                "/input_tokens_details/cache_write_tokens_details/ttl_12h_tokens",
            ),
        ],
        UsageFamily::Claude => vec![
            (
                CacheTtl::FiveMinutes,
                "/cache_creation/ephemeral_5m_input_tokens",
            ),
            (
                CacheTtl::OneHour,
                "/cache_creation/ephemeral_1h_input_tokens",
            ),
        ],
        _ => vec![],
    } {
        if let Some(n) = get(value, path)? {
            e.writes.insert(ttl, n);
        }
    }
    if family == UsageFamily::Gemini {
        e.tool_input = get(value, "/toolUsePromptTokenCount")?;
        for (path, kind) in [
            ("/promptTokensDetails", 0),
            ("/candidatesTokensDetails", 1),
            ("/cacheTokensDetails", 2),
        ] {
            if let Some(details) = value.pointer(path) {
                let mut seen = BTreeSet::new();
                let mut sum = 0;
                for detail in details.as_array().ok_or_else(invalid)? {
                    let modality = detail["modality"].as_str().ok_or_else(invalid)?;
                    if !seen.insert(modality) {
                        return Err(invalid());
                    }
                    let n = get(detail, "/tokenCount")?;
                    sum = add(sum, count(n)?)?;
                    match (kind, modality) {
                        (0, "IMAGE") => e.input_image = n,
                        (0, "AUDIO") => e.input_audio = n,
                        (1, "IMAGE") => e.output_image = n,
                        (1, "AUDIO") => e.output_audio = n,
                        (2, "IMAGE") => e.cached_image = n,
                        (2, "AUDIO") => e.cached_audio = n,
                        (_, "TEXT") => {
                            count(n)?;
                        }
                        _ => return Err(invalid()),
                    }
                }
                let aggregate = match kind {
                    0 => e.input,
                    1 => e.output,
                    _ => e.cache_read,
                };
                if sum != count(aggregate)? {
                    return Err(invalid());
                }
            }
        }
    } else if family == UsageFamily::Openai {
        e.input_image = get(value, "/input_tokens_details/image_tokens")?;
        e.input_audio = get(value, "/input_tokens_details/audio_tokens")?;
        e.output_image = get(value, "/output_tokens_details/image_tokens")?;
        e.output_audio = get(value, "/output_tokens_details/audio_tokens")?;
        e.cached_image = get(
            value,
            "/input_tokens_details/cached_tokens_details/image_tokens",
        )?;
        e.cached_audio = get(
            value,
            "/input_tokens_details/cached_tokens_details/audio_tokens",
        )?;
    }
    Ok(e)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    pub(super) fn contract(family: UsageFamily) -> UsageContract {
        UsageContract {
            version: match family {
                UsageFamily::Openai => "openai.responses.v1",
                UsageFamily::Claude => "anthropic.messages.v1",
                UsageFamily::Gemini => "gemini.generate.v1",
                UsageFamily::OpenMaas => "open_maas.certified.v1",
            }
            .into(),
            fixture_sha256: "a".repeat(64),
            family,
            tokens: true,
            cache_read: true,
            cache_write_ttls: vec![],
            split_reasoning: false,
            input_modalities: vec![],
            output_modalities: vec![],
            bill_tool_input: false,
            images: false,
            requests: false,
        }
    }
    #[test]
    fn openai_overlapping_cache_writes_are_disjoint() {
        let mut c = contract(UsageFamily::Openai);
        c.cache_write_ttls = vec![CacheTtl::ThirtyMinutes, CacheTtl::TwelveHours];
        let e=parse_usage(c.family,&json!({"input_tokens":12000,"output_tokens":100,"total_tokens":12100,"input_tokens_details":{"cached_tokens":4000,"cache_write_tokens":7000,"cache_write_tokens_details":{"ttl_30m_tokens":3000,"ttl_12h_tokens":4000}}})).unwrap();
        let n = normalize(&c, &e).unwrap();
        assert_eq!(n[&Selector::Input], 1000);
        assert_eq!(n[&Selector::CacheRead], 4000);
        assert_eq!(
            n[&Selector::CacheWrite {
                ttl: CacheTtl::ThirtyMinutes
            }],
            3000
        );
        assert_eq!(
            n[&Selector::CacheWrite {
                ttl: CacheTtl::TwelveHours
            }],
            4000
        );
        assert_eq!(n.values().sum::<i64>(), 12100);
    }
    #[test]
    fn claude_input_excludes_cache_and_keeps_ttls() {
        let mut c = contract(UsageFamily::Claude);
        c.cache_write_ttls = vec![CacheTtl::FiveMinutes, CacheTtl::OneHour];
        let e=parse_usage(c.family,&json!({"input_tokens":100,"output_tokens":10,"cache_read_input_tokens":800,"cache_creation_input_tokens":200,"cache_creation":{"ephemeral_5m_input_tokens":150,"ephemeral_1h_input_tokens":50}})).unwrap();
        let n = normalize(&c, &e).unwrap();
        assert_eq!(n[&Selector::Input], 100);
        assert_eq!(n.values().sum::<i64>(), 1110);
        assert_eq!(
            n[&Selector::CacheWrite {
                ttl: CacheTtl::OneHour
            }],
            50
        );
    }
    #[test]
    fn gemini_thoughts_are_billed_once_aggregate_or_split() {
        let mut c = contract(UsageFamily::Gemini);
        let e=parse_usage(c.family,&json!({"promptTokenCount":1000,"cachedContentTokenCount":200,"candidatesTokenCount":100,"thoughtsTokenCount":50,"toolUsePromptTokenCount":0,"totalTokenCount":1150})).unwrap();
        let n = normalize(&c, &e).unwrap();
        assert_eq!(n[&Selector::Input], 800);
        assert_eq!(n[&Selector::CacheRead], 200);
        assert_eq!(n[&Selector::Output], 150);
        c.split_reasoning = true;
        let n = normalize(&c, &e).unwrap();
        assert_eq!(n[&Selector::Output], 100);
        assert_eq!(n[&Selector::Reasoning], 50);
        assert_eq!(n.values().sum::<i64>(), 1150);
    }
    #[test]
    fn missing_modality_intersection_refuses_split_but_allows_uniform_aggregate() {
        let mut c = contract(UsageFamily::Gemini);
        let mut e = Evidence {
            input: Some(1000),
            cache_read: Some(200),
            output: Some(0),
            reasoning: Some(0),
            tool_input: Some(0),
            input_image: Some(300),
            ..Default::default()
        };
        assert_eq!(normalize(&c, &e).unwrap()[&Selector::Input], 800);
        c.input_modalities = vec![Modality::Image];
        assert!(normalize(&c, &e).is_err());
        e.cached_image = Some(100);
        let n = normalize(&c, &e).unwrap();
        assert_eq!(n[&Selector::Input], 600);
        assert_eq!(n[&Selector::ImageInput], 200);
        assert_eq!(
            n[&Selector::CachedModality {
                modality: Modality::Image
            }],
            100
        );
        assert_eq!(n[&Selector::CacheRead], 100);
    }
    #[test]
    fn missing_zero_fraction_negative_overflow_and_inconsistent_counts_differ() {
        let c = contract(UsageFamily::Openai);
        for bad in [
            json!(-1),
            json!(1.5),
            json!("2"),
            json!(18446744073709551615_u64),
            serde_json::Value::Null,
        ] {
            assert!(parse_usage(c.family, &json!({"input_tokens":bad,"output_tokens":0})).is_err());
        }
        let mut e = Evidence {
            input: Some(1),
            output: Some(0),
            ..Default::default()
        };
        assert!(normalize(&c, &e).is_err());
        e.cache_read = Some(0);
        assert!(normalize(&c, &e).is_ok());
        e.cache_read = Some(2);
        assert!(normalize(&c, &e).is_err());
        e.cache_read = Some(0);
        e.total = Some(2);
        assert!(normalize(&c, &e).is_err());
        e.input = Some(i64::MAX);
        e.output = Some(1);
        e.total = Some(i64::MAX);
        assert!(normalize(&c, &e).is_err());
    }
    #[test]
    fn gemini_modality_details_must_account_for_the_reported_aggregate() {
        let mut usage = json!({"promptTokenCount":10,"promptTokensDetails":[
            {"modality":"TEXT","tokenCount":7},{"modality":"IMAGE","tokenCount":3}
        ]});
        assert!(parse_usage(UsageFamily::Gemini, &usage).is_ok());
        usage["promptTokensDetails"][0]["tokenCount"] = json!(8);
        assert!(parse_usage(UsageFamily::Gemini, &usage).is_err());
        usage["promptTokensDetails"][0]["tokenCount"] = json!(6);
        assert!(parse_usage(UsageFamily::Gemini, &usage).is_err());
        usage["promptTokensDetails"][0]["tokenCount"] = json!(null);
        assert!(parse_usage(UsageFamily::Gemini, &usage).is_err());
        usage["promptTokensDetails"] = json!([
            {"modality":"TEXT","tokenCount":5},{"modality":"TEXT","tokenCount":5}
        ]);
        assert!(parse_usage(UsageFamily::Gemini, &usage).is_err());
    }

    #[test]
    fn write_ttls_must_be_complete_exact_and_known() {
        let mut c = contract(UsageFamily::Openai);
        c.cache_write_ttls = vec![CacheTtl::ThirtyMinutes, CacheTtl::TwelveHours];
        let mut e = Evidence {
            input: Some(10),
            output: Some(0),
            cache_read: Some(0),
            cache_write: Some(4),
            ..Default::default()
        };
        assert!(normalize(&c, &e).is_err());
        e.writes.insert(CacheTtl::ThirtyMinutes, 2);
        e.writes.insert(CacheTtl::TwelveHours, 1);
        assert!(normalize(&c, &e).is_err());
        e.writes.insert(CacheTtl::TwelveHours, 2);
        assert!(normalize(&c, &e).is_ok());
        e.writes.insert(CacheTtl::FiveMinutes, 0);
        assert!(normalize(&c, &e).is_err());
    }
    #[test]
    fn audio_image_and_reasoning_overlaps_require_exact_partitions() {
        let mut c = contract(UsageFamily::Openai);
        c.input_modalities = vec![Modality::Image, Modality::Audio];
        c.output_modalities = vec![Modality::Audio];
        let mut e = Evidence {
            input: Some(100),
            output: Some(30),
            cache_read: Some(20),
            input_image: Some(20),
            input_audio: Some(50),
            cached_image: Some(5),
            cached_audio: Some(10),
            output_audio: Some(20),
            ..Default::default()
        };
        let n = normalize(&c, &e).unwrap();
        assert_eq!(n[&Selector::Input], 25);
        assert_eq!(n[&Selector::AudioInput], 40);
        assert_eq!(n[&Selector::Output], 10);
        assert_eq!(n.values().sum::<i64>(), 130);
        e.reasoning = Some(1);
        assert!(normalize(&c, &e).is_err());
        e.reasoning = None;
        e.cached_audio = Some(21);
        assert!(normalize(&c, &e).is_err());
    }
    #[test]
    fn per_image_contract_does_not_double_price_tokens_or_previews() {
        let mut c = contract(UsageFamily::Openai);
        c.tokens = false;
        c.cache_read = false;
        c.images = true;
        c.requests = true;
        let e = Evidence {
            images: Some(2),
            requests: Some(1),
            ..Default::default()
        };
        let n = normalize(&c, &e).unwrap();
        assert_eq!(n.len(), 2);
        assert_eq!(n[&Selector::Images], 2);
        c.tokens = true;
        assert!(selectors(&c).is_err());
    }
    #[test]
    fn open_maas_does_not_guess_openai_usage_and_tool_input_policy_is_explicit() {
        assert!(
            parse_usage(
                UsageFamily::OpenMaas,
                &json!({"prompt_tokens":10,"completion_tokens":5})
            )
            .is_err()
        );
        let mut c = contract(UsageFamily::Gemini);
        let e = Evidence {
            input: Some(10),
            cache_read: Some(0),
            output: Some(5),
            reasoning: Some(2),
            tool_input: Some(3),
            total: Some(20),
            ..Default::default()
        };
        assert_eq!(normalize(&c, &e).unwrap()[&Selector::Input], 10);
        c.bill_tool_input = true;
        assert_eq!(normalize(&c, &e).unwrap()[&Selector::Input], 13);
        c.tokens = false;
        c.cache_read = false;
        c.requests = true;
        assert!(selectors(&c).is_err());
    }
}

use std::collections::HashSet;

use serde::Serialize;

use crate::services::audit_service;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UsageAggregationMode {
    Snapshot,
    Delta,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
pub struct ReportedLlmUsage {
    pub prompt_tokens: u64,
    pub completion_tokens: u64,
    pub total_tokens: u64,
    /// Cache-read tokens (OpenAI `prompt_tokens_details.cached_tokens`,
    /// Anthropic `cache_read_input_tokens`).
    pub cached_tokens: u64,
    /// Cache-write tokens (Anthropic `cache_creation_input_tokens`).
    pub cache_creation_tokens: u64,
    pub reported_cost: Option<f64>,
    pub cached_tokens_included_in_prompt: bool,
    pub images: u64,
    pub completed_image_index: Option<u64>,
}

impl ReportedLlmUsage {
    pub fn is_empty(&self) -> bool {
        self.prompt_tokens == 0
            && self.completion_tokens == 0
            && self.total_tokens == 0
            && self.cached_tokens == 0
            && self.cache_creation_tokens == 0
            && self.reported_cost.is_none()
            && self.images == 0
    }

    pub fn apply_to(
        &self,
        mut usage: crate::models::service_billing::PlatformUsage,
    ) -> crate::models::service_billing::PlatformUsage {
        let clamp = |value: u64| value.min(i64::MAX as u64) as i64;
        usage.input_tokens = clamp(if self.cached_tokens_included_in_prompt {
            self.prompt_tokens
                .saturating_sub(self.cached_tokens)
                .saturating_sub(self.cache_creation_tokens)
        } else {
            self.prompt_tokens
        });
        usage.output_tokens = clamp(self.completion_tokens);
        usage.cache_read_tokens = clamp(self.cached_tokens);
        usage.cache_write_tokens = clamp(self.cache_creation_tokens);
        usage.images = clamp(self.images);
        usage.with_token_breakdown(Some(self.token_breakdown()))
    }

    /// Per-class breakdown for the usage meter row, following each
    /// provider's own accounting (see `TokenBreakdown`).
    pub fn token_breakdown(&self) -> crate::models::service_billing::TokenBreakdown {
        fn clamp(value: u64) -> i64 {
            value.min(i64::MAX as u64) as i64
        }
        crate::models::service_billing::TokenBreakdown {
            prompt_tokens: clamp(self.prompt_tokens),
            completion_tokens: clamp(self.completion_tokens),
            cached_tokens: clamp(self.cached_tokens),
            cache_creation_tokens: clamp(self.cache_creation_tokens),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ReportedLlmUsageAccumulator {
    prompt_tokens: u64,
    completion_tokens: u64,
    total_tokens: u64,
    cached_tokens: u64,
    cache_creation_tokens: u64,
    reported_cost: Option<f64>,
    cached_tokens_included_in_prompt: bool,
    images: u64,
    seen_image_indexes: HashSet<u64>,
}

const MAX_REALTIME_RESPONSE_ID_LEN: usize = 256;

#[derive(Debug, Clone, PartialEq)]
pub struct RealtimeResponseDone {
    pub response_id: String,
    pub usage: Option<ReportedLlmUsage>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct RealtimeLlmUsageSummary {
    pub collection_enabled: bool,
    pub reported_usage: Option<ReportedLlmUsage>,
    pub uncovered_bytes: i64,
    pub reported_response_count: usize,
    pub estimated_response_count: usize,
}

impl RealtimeLlmUsageSummary {
    pub fn token_quantity(&self) -> i64 {
        let reported = self
            .reported_usage
            .as_ref()
            .map(|usage| usage.total_tokens.min(i64::MAX as u64) as i64)
            .unwrap_or(0);
        let estimated = if self.uncovered_bytes > 0 {
            estimate_tokens_from_bytes(self.uncovered_bytes)
        } else {
            0
        };

        reported.saturating_add(estimated)
    }
}

#[derive(Debug, Clone)]
pub struct RealtimeLlmUsageCollector {
    enabled: bool,
    seen_response_ids: HashSet<String>,
    reported_usage: ReportedLlmUsageAccumulator,
    segment_bytes: i64,
    segment_has_response_activity: bool,
    uncovered_bytes: i64,
    reported_response_count: usize,
    estimated_response_count: usize,
}

impl RealtimeLlmUsageCollector {
    pub fn new(enabled: bool) -> Self {
        Self {
            enabled,
            seen_response_ids: HashSet::new(),
            reported_usage: ReportedLlmUsageAccumulator::default(),
            segment_bytes: 0,
            segment_has_response_activity: false,
            uncovered_bytes: 0,
            reported_response_count: 0,
            estimated_response_count: 0,
        }
    }

    pub fn observe_client_text(&mut self, text: &str) {
        if !self.enabled {
            return;
        }

        self.add_segment_bytes(text.len());
        if serde_json::from_str::<serde_json::Value>(text)
            .ok()
            .is_some_and(|value| {
                value.get("type").and_then(serde_json::Value::as_str) == Some("response.create")
            })
        {
            self.segment_has_response_activity = true;
        }
    }

    pub fn observe_client_binary(&mut self, bytes: usize) {
        if self.enabled {
            self.add_segment_bytes(bytes);
        }
    }

    pub fn observe_downstream_text(&mut self, text: &str) {
        if !self.enabled {
            return;
        }

        self.add_segment_bytes(text.len());
        let Ok(value) = serde_json::from_str::<serde_json::Value>(text) else {
            return;
        };
        let Some(event_type) = value.get("type").and_then(serde_json::Value::as_str) else {
            return;
        };

        if event_type != "response.done" {
            if event_type.starts_with("response.") {
                self.segment_has_response_activity = true;
            }
            return;
        }

        let segment_was_active = self.segment_has_response_activity;
        self.segment_has_response_activity = true;
        let Some(done) = extract_realtime_response_done(&value) else {
            return;
        };
        if !self.seen_response_ids.insert(done.response_id) {
            self.segment_bytes = self
                .segment_bytes
                .saturating_sub(i64::try_from(text.len()).unwrap_or(i64::MAX));
            self.segment_has_response_activity = segment_was_active;
            return;
        }

        match done.usage {
            Some(usage) if usage.total_tokens > 0 => {
                self.reported_usage.observe_delta(usage);
                self.reported_response_count = self.reported_response_count.saturating_add(1);
            }
            _ => {
                self.uncovered_bytes = self.uncovered_bytes.saturating_add(self.segment_bytes);
                self.estimated_response_count = self.estimated_response_count.saturating_add(1);
            }
        }

        self.segment_bytes = 0;
        self.segment_has_response_activity = false;
    }

    pub fn observe_downstream_binary(&mut self, bytes: usize) {
        if self.enabled {
            self.add_segment_bytes(bytes);
        }
    }

    pub fn finalize(mut self) -> RealtimeLlmUsageSummary {
        if self.enabled && self.segment_has_response_activity {
            self.uncovered_bytes = self.uncovered_bytes.saturating_add(self.segment_bytes);
            self.estimated_response_count = self.estimated_response_count.saturating_add(1);
        }

        RealtimeLlmUsageSummary {
            collection_enabled: self.enabled,
            reported_usage: self.reported_usage.finalize(),
            uncovered_bytes: self.uncovered_bytes,
            reported_response_count: self.reported_response_count,
            estimated_response_count: self.estimated_response_count,
        }
    }

    fn add_segment_bytes(&mut self, bytes: usize) {
        self.segment_bytes = self
            .segment_bytes
            .saturating_add(i64::try_from(bytes).unwrap_or(i64::MAX));
    }
}

const FALLBACK_BYTES_PER_TOKEN: i64 = 4;

impl ReportedLlmUsageAccumulator {
    pub fn observe_snapshot(&mut self, usage: ReportedLlmUsage) {
        self.cached_tokens_included_in_prompt |= usage.cached_tokens_included_in_prompt;
        if let Some(index) = usage.completed_image_index {
            if !self.seen_image_indexes.insert(index) {
                return;
            }
            self.images = self.images.saturating_add(usage.images);
        } else {
            self.images = self.images.max(usage.images);
        }
        self.prompt_tokens = self.prompt_tokens.max(usage.prompt_tokens);
        self.completion_tokens = self.completion_tokens.max(usage.completion_tokens);
        self.total_tokens = self.total_tokens.max(usage.total_tokens);
        self.cached_tokens = self.cached_tokens.max(usage.cached_tokens);
        self.cache_creation_tokens = self.cache_creation_tokens.max(usage.cache_creation_tokens);

        if let Some(cost) = usage.reported_cost {
            self.reported_cost = Some(
                self.reported_cost
                    .map(|current| current.max(cost))
                    .unwrap_or(cost),
            );
        }
    }

    pub fn observe_delta(&mut self, usage: ReportedLlmUsage) {
        self.cached_tokens_included_in_prompt |= usage.cached_tokens_included_in_prompt;
        if let Some(index) = usage.completed_image_index {
            if !self.seen_image_indexes.insert(index) {
                return;
            }
            self.images = self.images.saturating_add(usage.images);
        } else {
            self.images = self.images.max(usage.images);
        }
        self.prompt_tokens = self.prompt_tokens.saturating_add(usage.prompt_tokens);
        self.completion_tokens = self
            .completion_tokens
            .saturating_add(usage.completion_tokens);
        self.total_tokens = self.total_tokens.saturating_add(usage.total_tokens);
        self.cached_tokens = self.cached_tokens.saturating_add(usage.cached_tokens);
        self.cache_creation_tokens = self
            .cache_creation_tokens
            .saturating_add(usage.cache_creation_tokens);

        if let Some(cost) = usage.reported_cost {
            self.reported_cost = Some(self.reported_cost.unwrap_or(0.0) + cost);
        }
    }

    pub fn observe(&mut self, usage: ReportedLlmUsage, mode: UsageAggregationMode) {
        match mode {
            UsageAggregationMode::Snapshot => self.observe_snapshot(usage),
            UsageAggregationMode::Delta => self.observe_delta(usage),
        }
    }

    pub fn finalize(self) -> Option<ReportedLlmUsage> {
        let total_tokens = if self.total_tokens > 0 {
            self.total_tokens
        } else {
            self.prompt_tokens.saturating_add(self.completion_tokens)
        };

        let usage = ReportedLlmUsage {
            prompt_tokens: self.prompt_tokens,
            completion_tokens: self.completion_tokens,
            total_tokens,
            cached_tokens: self.cached_tokens,
            cache_creation_tokens: self.cache_creation_tokens,
            reported_cost: self.reported_cost,
            cached_tokens_included_in_prompt: self.cached_tokens_included_in_prompt,
            images: self.images,
            completed_image_index: None,
        };

        (!usage.is_empty()).then_some(usage)
    }
}

pub fn estimate_tokens_from_bytes(total_bytes: i64) -> i64 {
    let bytes = total_bytes.max(0);
    let estimated = bytes.saturating_add(FALLBACK_BYTES_PER_TOKEN - 1) / FALLBACK_BYTES_PER_TOKEN;
    estimated.max(1)
}

pub fn token_quantity_or_estimate(usage: Option<&ReportedLlmUsage>, fallback_bytes: i64) -> i64 {
    usage
        .and_then(|usage| {
            (usage.total_tokens > 0).then_some(usage.total_tokens.min(i64::MAX as u64) as i64)
        })
        .unwrap_or_else(|| estimate_tokens_from_bytes(fallback_bytes))
}

pub fn force_stream_options_include_usage(body: &mut serde_json::Value) -> bool {
    let Some(object) = body.as_object_mut() else {
        return false;
    };
    if object.get("stream").and_then(|value| value.as_bool()) != Some(true) {
        return false;
    }

    match object.get_mut("stream_options") {
        Some(serde_json::Value::Object(options)) => {
            if options
                .get("include_usage")
                .and_then(|value| value.as_bool())
                == Some(true)
            {
                false
            } else {
                options.insert("include_usage".to_string(), serde_json::Value::Bool(true));
                true
            }
        }
        Some(_) => {
            object.insert(
                "stream_options".to_string(),
                serde_json::json!({ "include_usage": true }),
            );
            true
        }
        None => {
            object.insert(
                "stream_options".to_string(),
                serde_json::json!({ "include_usage": true }),
            );
            true
        }
    }
}

#[derive(Debug, Clone)]
pub struct UsageAuditContext {
    pub db: mongodb::Database,
    pub user_id: String,
    pub provider_slug: Option<String>,
    pub service_id: Option<String>,
    pub model: Option<String>,
    pub path: String,
    pub api_key_id: Option<String>,
    pub api_key_name: Option<String>,
}

fn number_at(value: &serde_json::Value, pointer: &str) -> Option<f64> {
    value.pointer(pointer).and_then(|raw| match raw {
        serde_json::Value::Number(number) => number.as_f64(),
        serde_json::Value::String(text) => text.parse::<f64>().ok(),
        _ => None,
    })
}

fn token_at(value: &serde_json::Value, pointers: &[&str]) -> Option<u64> {
    pointers.iter().find_map(|pointer| {
        value.pointer(pointer).and_then(|raw| {
            raw.as_u64()
                .or_else(|| raw.as_f64().map(|count| count.max(0.0).trunc() as u64))
                .or_else(|| raw.as_str().and_then(|text| text.parse::<u64>().ok()))
        })
    })
}

pub fn extract_reported_usage(value: &serde_json::Value) -> Option<ReportedLlmUsage> {
    let prompt_tokens = token_at(
        value,
        &[
            "/usageMetadata/promptTokenCount",
            "/prompt_tokens",
            "/usage/prompt_tokens",
            "/usage/input_tokens",
            "/input_tokens",
            "/response/usage/prompt_tokens",
            "/response/usage/input_tokens",
            "/message/usage/prompt_tokens",
            "/message/usage/input_tokens",
        ],
    )
    .unwrap_or(0);

    let completion_tokens = token_at(
        value,
        &[
            "/usageMetadata/candidatesTokenCount",
            "/completion_tokens",
            "/usage/completion_tokens",
            "/usage/output_tokens",
            "/output_tokens",
            "/response/usage/completion_tokens",
            "/response/usage/output_tokens",
            "/message/usage/completion_tokens",
            "/message/usage/output_tokens",
        ],
    )
    .unwrap_or(0);

    let total_tokens = token_at(
        value,
        &[
            "/usageMetadata/totalTokenCount",
            "/total_tokens",
            "/usage/total_tokens",
            "/response/usage/total_tokens",
            "/message/usage/total_tokens",
        ],
    )
    .unwrap_or_else(|| prompt_tokens.saturating_add(completion_tokens));

    let cached_tokens = token_at(
        value,
        &[
            "/usageMetadata/cachedContentTokenCount",
            "/prompt_tokens_details/cached_tokens",
            "/input_tokens_details/cached_tokens",
            "/usage/prompt_tokens_details/cached_tokens",
            "/usage/input_tokens_details/cached_tokens",
            "/input_token_details/cached_tokens",
            "/usage/input_token_details/cached_tokens",
            "/response/usage/input_token_details/cached_tokens",
            "/usage/cache_read_input_tokens",
            "/cache_read_input_tokens",
            "/response/usage/prompt_tokens_details/cached_tokens",
            "/response/usage/input_tokens_details/cached_tokens",
            "/response/usage/cache_read_input_tokens",
            "/message/usage/cache_read_input_tokens",
        ],
    )
    .unwrap_or(0);

    let cache_creation_tokens = token_at(
        value,
        &[
            "/usage/cache_creation_input_tokens",
            "/cache_creation_input_tokens",
            "/response/usage/cache_creation_input_tokens",
            "/message/usage/cache_creation_input_tokens",
        ],
    )
    .unwrap_or(0);

    let reported_cost = [
        "/usage/reported_cost",
        "/usage/cost_usd",
        "/usage/total_cost_usd",
        "/usage/cost",
        "/usage/total_cost",
        "/response/usage/reported_cost",
        "/response/usage/cost_usd",
        "/response/usage/total_cost_usd",
        "/response/usage/cost",
        "/response/usage/total_cost",
        "/reported_cost",
        "/cost_usd",
        "/total_cost_usd",
        "/cost",
        "/total_cost",
    ]
    .iter()
    .find_map(|pointer| number_at(value, pointer));

    let usage = ReportedLlmUsage {
        prompt_tokens,
        completion_tokens,
        total_tokens,
        cached_tokens,
        cache_creation_tokens,
        reported_cost,
        cached_tokens_included_in_prompt: [
            "/prompt_tokens_details/cached_tokens",
            "/input_tokens_details/cached_tokens",
            "/input_token_details/cached_tokens",
            "/usage/input_token_details/cached_tokens",
            "/response/usage/input_token_details/cached_tokens",
            "/usage/prompt_tokens_details/cached_tokens",
            "/usage/input_tokens_details/cached_tokens",
            "/response/usage/prompt_tokens_details/cached_tokens",
            "/response/usage/input_tokens_details/cached_tokens",
            "/usageMetadata/cachedContentTokenCount",
        ]
        .iter()
        .any(|path| value.pointer(path).is_some()),
        images: 0,
        completed_image_index: None,
    };

    (!usage.is_empty()).then_some(usage)
}

pub fn is_image_generation_path(path: &str) -> bool {
    let path = path.split('?').next().unwrap_or(path).trim_end_matches('/');
    ["/images/generations", "/images/edits", "/images/variations"]
        .iter()
        .any(|suffix| path.ends_with(suffix))
}

pub fn extract_reported_usage_for_path(
    value: &serde_json::Value,
    path: &str,
    successful: bool,
) -> Option<ReportedLlmUsage> {
    let mut usage = extract_reported_usage(value).unwrap_or_default();
    if successful && is_image_generation_path(path) {
        usage.images = value
            .get("data")
            .and_then(serde_json::Value::as_array)
            .map_or(0, |images| images.len() as u64);
    }
    (!usage.is_empty()).then_some(usage)
}

/// Incremental SSE usage observer with the same 512 KiB capture cap as proxy
/// response capture. Oversized events are discarded through their separator.
#[derive(Default)]
pub struct BoundedUsageEvents {
    buffer: Vec<u8>,
    discarding: bool,
    line_empty: bool,
    accumulator: ReportedLlmUsageAccumulator,
}
impl BoundedUsageEvents {
    pub fn push(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            if self.discarding {
                if byte == b'\n' {
                    if self.line_empty {
                        self.discarding = false;
                        self.line_empty = false;
                    } else {
                        self.line_empty = true;
                    }
                } else if byte != b'\r' {
                    self.line_empty = false;
                }
                continue;
            }
            self.buffer.push(byte);
            if self.buffer.ends_with(b"\n\n") || self.buffer.ends_with(b"\r\n\r\n") {
                // The shared parser consumes LF separators. Normalize complete
                // CRLF events only after enforcing this observer's byte limit.
                let mut text = String::from_utf8_lossy(&self.buffer).replace("\r\n", "\n");
                if let Some(event) = super::sse_parser::parse_next_event(&mut text)
                    && let Some((usage, mode)) = extract_reported_usage_from_sse_event(
                        event.event_type.as_deref(),
                        &event.data,
                    )
                {
                    self.accumulator.observe(usage, mode);
                }
                self.buffer.clear();
            } else if self.buffer.len() >= 512 * 1024 {
                self.buffer.clear();
                self.discarding = true;
                self.line_empty = byte == b'\n';
            }
        }
    }
    pub fn finalize(self) -> Option<ReportedLlmUsage> {
        self.accumulator.finalize()
    }
    pub fn finalize_success(self, successful: bool) -> Option<ReportedLlmUsage> {
        self.finalize().map(|mut usage| {
            if !successful {
                usage.images = 0;
            }
            usage
        })
    }
}

/// Bounded callers supply already-captured bodies; no additional body reads.
pub fn usage_from_body(body: &[u8], path: &str, successful: bool) -> Option<ReportedLlmUsage> {
    if let Ok(value) = serde_json::from_slice::<serde_json::Value>(body) {
        return extract_reported_usage_for_path(&value, path, successful);
    }
    let mut buffer = String::from_utf8_lossy(body).into_owned();
    let mut accumulator = ReportedLlmUsageAccumulator::default();
    while let Some(event) = super::sse_parser::parse_next_event(&mut buffer) {
        if let Some((mut usage, mode)) =
            extract_reported_usage_from_sse_event(event.event_type.as_deref(), &event.data)
        {
            if !successful {
                usage.images = 0;
            }
            accumulator.observe(usage, mode);
        }
    }
    accumulator.finalize()
}

pub fn platform_usage(
    usage: Option<&ReportedLlmUsage>,
    bytes: i64,
    estimate: bool,
) -> crate::models::service_billing::PlatformUsage {
    let tokens = if estimate {
        token_quantity_or_estimate(usage, bytes)
    } else {
        usage.map_or(0, |u| u.total_tokens.min(i64::MAX as u64) as i64)
    };
    let platform = crate::models::service_billing::PlatformUsage::llm_completion(bytes, tokens);
    usage.map_or(platform.clone(), |u| u.apply_to(platform))
}

pub fn extract_realtime_response_done(value: &serde_json::Value) -> Option<RealtimeResponseDone> {
    if value.get("type").and_then(serde_json::Value::as_str) != Some("response.done") {
        return None;
    }

    let response = value.get("response")?;
    let response_id = response.get("id")?.as_str()?;
    if response_id.is_empty()
        || response_id.trim() != response_id
        || response_id.len() > MAX_REALTIME_RESPONSE_ID_LEN
    {
        return None;
    }

    Some(RealtimeResponseDone {
        response_id: response_id.to_string(),
        usage: extract_reported_usage(response),
    })
}

fn has_explicit_total(value: &serde_json::Value) -> bool {
    [
        "/total_tokens",
        "/usage/total_tokens",
        "/response/usage/total_tokens",
        "/message/usage/total_tokens",
    ]
    .iter()
    .any(|pointer| number_at(value, pointer).is_some())
}

pub fn extract_reported_usage_from_sse_event(
    event_type: Option<&str>,
    data: &str,
) -> Option<(ReportedLlmUsage, UsageAggregationMode)> {
    if data.trim() == "[DONE]" {
        return None;
    }

    let value = serde_json::from_str::<serde_json::Value>(data).ok()?;
    let image_event = event_type.or_else(|| value.get("type").and_then(serde_json::Value::as_str));
    if matches!(
        image_event,
        Some("image_generation.completed" | "image_edit.completed")
    ) {
        let mut usage = extract_reported_usage(&value).unwrap_or_default();
        usage.images = 1;
        usage.completed_image_index = Some(
            value
                .get("image_index")
                .or_else(|| value.get("index"))
                .and_then(serde_json::Value::as_u64)
                .unwrap_or(0),
        );
        return Some((usage, UsageAggregationMode::Delta));
    }
    let usage = extract_reported_usage(&value)?;

    let mode = match event_type {
        Some("message_start")
        | Some("message_delta")
        | Some("response.completed")
        | Some("response.incomplete") => UsageAggregationMode::Snapshot,
        Some("usage.delta") | Some("response.usage.delta") => UsageAggregationMode::Delta,
        _ if has_explicit_total(&value) => UsageAggregationMode::Snapshot,
        _ => return None,
    };

    Some((usage, mode))
}

pub fn log_reported_usage_async(context: UsageAuditContext, usage: ReportedLlmUsage) {
    if usage.is_empty() {
        return;
    }

    audit_service::log_async(
        context.db,
        Some(context.user_id),
        "llm_usage_reported".to_string(),
        Some(serde_json::json!({
            "provider_slug": context.provider_slug,
            "service_id": context.service_id,
            "model": context.model,
            "path": context.path,
            "prompt_tokens": usage.prompt_tokens,
            "completion_tokens": usage.completion_tokens,
            "total_tokens": usage.total_tokens,
            "reported_cost": usage.reported_cost,
        })),
        None,
        None,
        context.api_key_id,
        context.api_key_name,
    );
}

#[cfg(test)]
mod tests {
    use super::{
        RealtimeLlmUsageCollector, ReportedLlmUsage, ReportedLlmUsageAccumulator,
        UsageAggregationMode, estimate_tokens_from_bytes, extract_realtime_response_done,
        extract_reported_usage, extract_reported_usage_from_sse_event,
        force_stream_options_include_usage, token_quantity_or_estimate,
    };

    #[test]
    fn provider_token_counts_accept_floats_and_preserve_integer_precision() {
        let usage = extract_reported_usage(&serde_json::json!({
            "usage": {"prompt_tokens": 25.0, "completion_tokens": 9.75,
                "prompt_tokens_details": {"cached_tokens": 2.9}}
        }))
        .unwrap();
        assert_eq!(usage.prompt_tokens, 25);
        assert_eq!(usage.completion_tokens, 9);
        assert_eq!(usage.cached_tokens, 2);
        for (value, expected) in [
            (serde_json::json!(-5.5), 0),
            (serde_json::json!(u64::MAX), u64::MAX),
            (serde_json::json!("25"), 25),
        ] {
            assert_eq!(
                super::token_at(&serde_json::json!({"count": value}), &["/count"]),
                Some(expected)
            );
        }
    }

    #[test]
    fn extracts_usage_from_openai_style_payload() {
        let value = serde_json::json!({
            "usage": {
                "prompt_tokens": 12,
                "completion_tokens": 7,
                "total_tokens": 19,
                "reported_cost": 0.0042
            }
        });

        let usage = extract_reported_usage(&value).expect("usage");

        assert_eq!(
            usage,
            ReportedLlmUsage {
                prompt_tokens: 12,
                completion_tokens: 7,
                total_tokens: 19,
                cached_tokens: 0,
                cache_creation_tokens: 0,
                reported_cost: Some(0.0042),
                ..Default::default()
            }
        );
    }

    #[test]
    fn extracts_usage_from_nested_provider_payload() {
        let value = serde_json::json!({
            "response": {
                "usage": {
                    "input_tokens": 25,
                    "output_tokens": 15
                }
            }
        });

        let usage = extract_reported_usage(&value).expect("usage");

        assert_eq!(usage.prompt_tokens, 25);
        assert_eq!(usage.completion_tokens, 15);
        assert_eq!(usage.total_tokens, 40);
        assert_eq!(usage.reported_cost, None);
    }

    #[test]
    fn realtime_done_adapter_requires_nested_response_id_and_usage() {
        let value = serde_json::json!({
            "type": "response.done",
            "response": {
                "id": "resp_123",
                "usage": {
                    "input_tokens": 25,
                    "output_tokens": 15,
                    "total_tokens": 40
                }
            }
        });

        let done = extract_realtime_response_done(&value).expect("response.done");
        assert_eq!(done.response_id, "resp_123");
        assert_eq!(done.usage.expect("usage").total_tokens, 40);

        let wrong_type = serde_json::json!({
            "type": "response.completed",
            "response": { "id": "resp_123", "usage": { "total_tokens": 40 } }
        });
        assert!(extract_realtime_response_done(&wrong_type).is_none());

        let root_only = serde_json::json!({
            "type": "response.done",
            "id": "resp_123",
            "usage": { "total_tokens": 40 }
        });
        assert!(extract_realtime_response_done(&root_only).is_none());
    }

    #[test]
    fn realtime_collector_sums_unique_response_done_usage() {
        let mut collector = RealtimeLlmUsageCollector::new(true);
        collector.observe_client_text(r#"{"type":"response.create"}"#);
        collector.observe_downstream_text(
            r#"{"type":"response.done","response":{"id":"resp_1","usage":{"input_tokens":20,"output_tokens":10,"total_tokens":30}}}"#,
        );
        collector.observe_downstream_text(
            r#"{"type":"response.done","response":{"id":"resp_1","usage":{"total_tokens":999}}}"#,
        );
        collector.observe_client_text(r#"{"type":"response.create"}"#);
        collector.observe_downstream_text(
            r#"{"type":"response.done","response":{"id":"resp_2","usage":{"input_tokens":5,"output_tokens":7,"total_tokens":12}}}"#,
        );

        let summary = collector.finalize();
        let usage = summary.reported_usage.as_ref().expect("reported usage");
        assert_eq!(usage.prompt_tokens, 25);
        assert_eq!(usage.completion_tokens, 17);
        assert_eq!(usage.total_tokens, 42);
        assert_eq!(summary.reported_response_count, 2);
        assert_eq!(summary.estimated_response_count, 0);
        assert_eq!(summary.uncovered_bytes, 0);
        assert_eq!(summary.token_quantity(), 42);
    }

    #[test]
    fn realtime_collector_ignores_trailing_duplicate_without_fallback() {
        let done =
            r#"{"type":"response.done","response":{"id":"resp_1","usage":{"total_tokens":30}}}"#;
        let mut collector = RealtimeLlmUsageCollector::new(true);
        collector.observe_client_text(r#"{"type":"response.create"}"#);
        collector.observe_downstream_text(done);
        collector.observe_downstream_text(done);

        let summary = collector.finalize();
        assert_eq!(summary.token_quantity(), 30);
        assert_eq!(summary.reported_response_count, 1);
        assert_eq!(summary.estimated_response_count, 0);
        assert_eq!(summary.uncovered_bytes, 0);
    }

    #[test]
    fn realtime_collector_estimates_only_uncovered_response_segment() {
        let first_create = r#"{"type":"response.create"}"#;
        let first_done =
            r#"{"type":"response.done","response":{"id":"resp_1","usage":{"total_tokens":30}}}"#;
        let second_create = r#"{"type":"response.create"}"#;
        let second_done = r#"{"type":"response.done","response":{"id":"resp_2"}}"#;
        let mut collector = RealtimeLlmUsageCollector::new(true);

        collector.observe_client_text(first_create);
        collector.observe_downstream_text(first_done);
        collector.observe_client_text(second_create);
        collector.observe_downstream_text(second_done);

        let summary = collector.finalize();
        let expected_uncovered = (second_create.len() + second_done.len()) as i64;
        assert_eq!(summary.uncovered_bytes, expected_uncovered);
        assert_eq!(summary.reported_response_count, 1);
        assert_eq!(summary.estimated_response_count, 1);
        assert_eq!(
            summary.token_quantity(),
            30 + estimate_tokens_from_bytes(expected_uncovered)
        );
    }

    #[test]
    fn realtime_collector_estimates_trailing_incomplete_response() {
        let create = r#"{"type":"response.create"}"#;
        let created = r#"{"type":"response.created","response":{"id":"resp_1"}}"#;
        let mut collector = RealtimeLlmUsageCollector::new(true);

        collector.observe_client_text(create);
        collector.observe_client_binary(64);
        collector.observe_downstream_text(created);
        collector.observe_downstream_binary(32);

        let summary = collector.finalize();
        assert_eq!(
            summary.uncovered_bytes,
            (create.len() + created.len() + 64 + 32) as i64
        );
        assert_eq!(summary.reported_response_count, 0);
        assert_eq!(summary.estimated_response_count, 1);
        assert!(summary.token_quantity() > 0);
    }

    #[test]
    fn realtime_collector_ignores_non_response_control_traffic() {
        let mut collector = RealtimeLlmUsageCollector::new(true);
        collector.observe_client_text(r#"{"type":"session.update"}"#);
        collector.observe_downstream_text(r#"{"type":"session.updated"}"#);

        let summary = collector.finalize();
        assert_eq!(summary.token_quantity(), 0);
        assert_eq!(summary.uncovered_bytes, 0);
        assert_eq!(summary.estimated_response_count, 0);
    }

    #[test]
    fn accumulator_keeps_latest_cumulative_values() {
        let mut accumulator = ReportedLlmUsageAccumulator::default();
        accumulator.observe_snapshot(ReportedLlmUsage {
            prompt_tokens: 10,
            completion_tokens: 0,
            total_tokens: 0,
            cached_tokens: 0,
            cache_creation_tokens: 0,
            reported_cost: None,
            ..Default::default()
        });
        accumulator.observe_snapshot(ReportedLlmUsage {
            prompt_tokens: 10,
            completion_tokens: 20,
            total_tokens: 30,
            cached_tokens: 0,
            cache_creation_tokens: 0,
            reported_cost: Some(0.012),
            ..Default::default()
        });

        let usage = accumulator.finalize().expect("usage");
        assert_eq!(usage.prompt_tokens, 10);
        assert_eq!(usage.completion_tokens, 20);
        assert_eq!(usage.total_tokens, 30);
        assert_eq!(usage.reported_cost, Some(0.012));
    }

    #[test]
    fn extracts_snapshot_usage_from_known_sse_event() {
        let result = extract_reported_usage_from_sse_event(
            Some("message_delta"),
            r#"{"usage":{"output_tokens":15}}"#,
        )
        .expect("usage");

        assert_eq!(result.0.completion_tokens, 15);
        assert_eq!(result.1, UsageAggregationMode::Snapshot);
    }

    #[test]
    fn ignores_ambiguous_sse_usage_without_total_or_known_event_type() {
        let result = extract_reported_usage_from_sse_event(
            Some("unknown"),
            r#"{"usage":{"output_tokens":15}}"#,
        );

        assert!(result.is_none());
    }

    #[test]
    fn extracts_delta_usage_from_delta_sse_event() {
        let result = extract_reported_usage_from_sse_event(
            Some("usage.delta"),
            r#"{"usage":{"prompt_tokens":5,"completion_tokens":3,"total_tokens":8}}"#,
        )
        .expect("should extract delta usage");

        assert_eq!(result.0.prompt_tokens, 5);
        assert_eq!(result.0.completion_tokens, 3);
        assert_eq!(result.0.total_tokens, 8);
        assert_eq!(result.1, UsageAggregationMode::Delta);

        // Also verify response.usage.delta variant
        let result2 = extract_reported_usage_from_sse_event(
            Some("response.usage.delta"),
            r#"{"usage":{"prompt_tokens":2,"completion_tokens":1,"total_tokens":3}}"#,
        )
        .expect("should extract response.usage.delta");

        assert_eq!(result2.1, UsageAggregationMode::Delta);
    }

    #[test]
    fn accumulator_sums_delta_values() {
        let mut accumulator = ReportedLlmUsageAccumulator::default();

        accumulator.observe_delta(ReportedLlmUsage {
            prompt_tokens: 10,
            completion_tokens: 5,
            total_tokens: 15,
            cached_tokens: 0,
            cache_creation_tokens: 0,
            reported_cost: Some(0.001),
            ..Default::default()
        });
        accumulator.observe_delta(ReportedLlmUsage {
            prompt_tokens: 3,
            completion_tokens: 7,
            total_tokens: 10,
            cached_tokens: 0,
            cache_creation_tokens: 0,
            reported_cost: Some(0.002),
            ..Default::default()
        });

        let usage = accumulator.finalize().expect("should have usage");
        assert_eq!(usage.prompt_tokens, 13);
        assert_eq!(usage.completion_tokens, 12);
        assert_eq!(usage.total_tokens, 25);
        assert_eq!(usage.reported_cost, Some(0.003));
    }

    #[test]
    fn falls_back_to_snapshot_when_unknown_event_has_total_tokens() {
        // Unknown event type but has explicit total_tokens -> Snapshot fallback
        let result = extract_reported_usage_from_sse_event(
            Some("some.unknown.event"),
            r#"{"usage":{"prompt_tokens":10,"completion_tokens":5,"total_tokens":15}}"#,
        )
        .expect("should fall back to snapshot when total_tokens present");

        assert_eq!(result.0.prompt_tokens, 10);
        assert_eq!(result.0.completion_tokens, 5);
        assert_eq!(result.0.total_tokens, 15);
        assert_eq!(result.1, UsageAggregationMode::Snapshot);
    }

    #[test]
    fn empty_accumulator_finalizes_to_none() {
        let accumulator = ReportedLlmUsageAccumulator::default();
        assert!(accumulator.finalize().is_none());
    }

    #[test]
    fn token_quantity_uses_reported_total_before_estimate() {
        let usage = ReportedLlmUsage {
            prompt_tokens: 20,
            completion_tokens: 10,
            total_tokens: 30,
            cached_tokens: 0,
            cache_creation_tokens: 0,
            reported_cost: None,
            ..Default::default()
        };

        assert_eq!(token_quantity_or_estimate(Some(&usage), 10_000), 30);
    }

    #[test]
    fn token_quantity_estimate_has_positive_floor() {
        assert_eq!(estimate_tokens_from_bytes(0), 1);
        assert_eq!(estimate_tokens_from_bytes(1), 1);
        assert_eq!(estimate_tokens_from_bytes(4), 1);
        assert_eq!(estimate_tokens_from_bytes(5), 2);
        assert_eq!(token_quantity_or_estimate(None, 0), 1);
    }

    #[test]
    fn token_quantity_estimates_when_usage_has_no_tokens() {
        let usage = ReportedLlmUsage {
            prompt_tokens: 0,
            completion_tokens: 0,
            total_tokens: 0,
            cached_tokens: 0,
            cache_creation_tokens: 0,
            reported_cost: Some(0.01),
            ..Default::default()
        };

        assert_eq!(token_quantity_or_estimate(Some(&usage), 17), 5);
    }

    #[test]
    fn force_stream_options_include_usage_preserves_existing_options() {
        let mut body = serde_json::json!({
            "model": "gpt-4o-mini",
            "stream": true,
            "stream_options": {
                "another_option": true
            }
        });

        assert!(force_stream_options_include_usage(&mut body));
        assert_eq!(body["stream_options"]["include_usage"], true);
        assert_eq!(body["stream_options"]["another_option"], true);
        assert!(!force_stream_options_include_usage(&mut body));
    }

    #[test]
    fn done_marker_returns_none() {
        let result = extract_reported_usage_from_sse_event(None, "[DONE]");
        assert!(result.is_none());

        // Also with whitespace
        let result2 = extract_reported_usage_from_sse_event(Some("message"), "  [DONE]  ");
        assert!(result2.is_none());
    }

    #[test]
    fn extracts_openai_cached_tokens() {
        let value = serde_json::json!({
            "usage": {
                "prompt_tokens": 120,
                "completion_tokens": 40,
                "total_tokens": 160,
                "prompt_tokens_details": { "cached_tokens": 100 }
            }
        });

        let usage = extract_reported_usage(&value).expect("usage");
        assert_eq!(usage.prompt_tokens, 120);
        assert_eq!(usage.completion_tokens, 40);
        assert_eq!(usage.cached_tokens, 100);
        assert_eq!(usage.cache_creation_tokens, 0);
    }

    #[test]
    fn extracts_anthropic_cache_read_and_creation_tokens() {
        let value = serde_json::json!({
            "usage": {
                "input_tokens": 25,
                "output_tokens": 60,
                "cache_read_input_tokens": 900,
                "cache_creation_input_tokens": 300
            }
        });

        let usage = extract_reported_usage(&value).expect("usage");
        assert_eq!(usage.prompt_tokens, 25);
        assert_eq!(usage.completion_tokens, 60);
        assert_eq!(usage.cached_tokens, 900);
        assert_eq!(usage.cache_creation_tokens, 300);
    }

    #[test]
    fn extracts_anthropic_stream_message_usage_cache_tokens() {
        let value = serde_json::json!({
            "type": "message_start",
            "message": {
                "usage": {
                    "input_tokens": 10,
                    "output_tokens": 1,
                    "cache_read_input_tokens": 500
                }
            }
        });

        let usage = extract_reported_usage(&value).expect("usage");
        assert_eq!(usage.cached_tokens, 500);
    }

    #[test]
    fn accumulator_sums_cache_tokens_across_deltas() {
        let mut accumulator = ReportedLlmUsageAccumulator::default();
        accumulator.observe_delta(ReportedLlmUsage {
            prompt_tokens: 10,
            completion_tokens: 5,
            total_tokens: 15,
            cached_tokens: 8,
            cache_creation_tokens: 2,
            reported_cost: None,
            ..Default::default()
        });
        accumulator.observe_delta(ReportedLlmUsage {
            prompt_tokens: 4,
            completion_tokens: 6,
            total_tokens: 10,
            cached_tokens: 3,
            cache_creation_tokens: 0,
            reported_cost: None,
            ..Default::default()
        });

        let usage = accumulator.finalize().expect("usage");
        assert_eq!(usage.cached_tokens, 11);
        assert_eq!(usage.cache_creation_tokens, 2);
    }

    #[test]
    fn token_breakdown_maps_all_classes() {
        let usage = ReportedLlmUsage {
            prompt_tokens: 1,
            completion_tokens: 2,
            total_tokens: 3,
            cached_tokens: 4,
            cache_creation_tokens: 5,
            reported_cost: None,
            ..Default::default()
        };
        let breakdown = usage.token_breakdown();
        assert_eq!(breakdown.prompt_tokens, 1);
        assert_eq!(breakdown.completion_tokens, 2);
        assert_eq!(breakdown.cached_tokens, 4);
        assert_eq!(breakdown.cache_creation_tokens, 5);
        assert!(!breakdown.is_empty());
    }
    #[test]
    fn bounded_sse_capture_handles_chunk_boundaries_limits_and_failed_images() {
        let event = b"data: {\"type\":\"image_generation.completed\",\"image_index\":0,\"usage\":{\"input_tokens\":120,\"output_tokens\":40,\"input_tokens_details\":{\"cached_tokens\":100}}}\r\n\r\n";
        for success in [true, false] {
            let mut observer = super::BoundedUsageEvents::default();
            observer.push(&vec![b'x'; 512 * 1024 + 20]);
            observer.push(b"\n\n");
            for chunk in event.chunks(7) {
                observer.push(chunk);
            }
            observer.push(event); // A duplicate completion never bills a second image.
            let usage = observer.finalize_success(success).unwrap();
            let priced = usage.apply_to(crate::models::service_billing::PlatformUsage::default());
            assert_eq!(
                (
                    priced.input_tokens,
                    priced.output_tokens,
                    priced.cache_read_tokens
                ),
                (20, 40, 100)
            );
            assert_eq!(priced.images, i64::from(success));
        }
    }

    #[test]
    fn priced_classes_preserve_provider_cache_accounting() {
        let cases = [
            (
                serde_json::json!({"usage":{"prompt_tokens":120,"completion_tokens":40,"prompt_tokens_details":{"cached_tokens":100}}}),
                [20, 40, 100, 0],
            ),
            (
                serde_json::json!({"usage":{"input_tokens":25,"output_tokens":60,"cache_read_input_tokens":900,"cache_creation_input_tokens":300}}),
                [25, 60, 900, 300],
            ),
            (
                serde_json::json!({"usageMetadata":{"promptTokenCount":120,"candidatesTokenCount":40,"cachedContentTokenCount":100}}),
                [20, 40, 100, 0],
            ),
            (
                serde_json::json!({"usage":{"input_tokens":5,"input_tokens_details":{"cached_tokens":8}}}),
                [0, 0, 8, 0],
            ),
        ];
        for (value, expected) in cases {
            let usage = extract_reported_usage(&value).unwrap();
            let priced = usage.apply_to(crate::models::service_billing::PlatformUsage::default());
            assert_eq!(
                [
                    priced.input_tokens,
                    priced.output_tokens,
                    priced.cache_read_tokens,
                    priced.cache_write_tokens
                ],
                expected
            );
        }
    }

    #[test]
    fn images_count_successes_with_path_prefixes_and_stream_deduplication() {
        let body = serde_json::json!({"data":[{"b64_json":"one"},{"url":"two"}],"usage":{"input_tokens":10,"output_tokens":30}});
        for path in [
            "/v1/images/generations",
            "/api/v1/proxy/s/image-service/v1/images/edits",
            "/api/v1/proxy/uuid/images/variations",
        ] {
            let usage = super::extract_reported_usage_for_path(&body, path, true).unwrap();
            assert_eq!(usage.images, 2);
            let priced = usage.apply_to(crate::models::service_billing::PlatformUsage::default());
            assert_eq!((priced.input_tokens, priced.output_tokens), (10, 30));
            assert_eq!(
                super::extract_reported_usage_for_path(&body, path, false)
                    .unwrap()
                    .images,
                0
            );
        }
        assert_eq!(
            super::extract_reported_usage_for_path(&body, "/models", true)
                .unwrap()
                .images,
            0
        );
        let event = r#"{"type":"image_generation.completed","image_index":0,"usage":{"input_tokens":10,"output_tokens":30}}"#;
        let mut accumulator = ReportedLlmUsageAccumulator::default();
        for _ in 0..2 {
            let (usage, mode) = extract_reported_usage_from_sse_event(None, event).unwrap();
            accumulator.observe(usage, mode);
        }
        let usage = accumulator.finalize().unwrap();
        assert_eq!(usage.images, 1);
        assert!(
            extract_reported_usage_from_sse_event(
                Some("image_generation.partial_image"),
                r#"{"b64_json":"partial"}"#
            )
            .is_none()
        );
    }
}

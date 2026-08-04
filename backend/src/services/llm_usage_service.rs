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
    pub reported_cost: Option<f64>,
}

impl ReportedLlmUsage {
    pub fn is_empty(&self) -> bool {
        self.prompt_tokens == 0
            && self.completion_tokens == 0
            && self.total_tokens == 0
            && self.reported_cost.is_none()
    }
}

#[derive(Debug, Clone, Default)]
pub struct ReportedLlmUsageAccumulator {
    prompt_tokens: u64,
    completion_tokens: u64,
    total_tokens: u64,
    reported_cost: Option<f64>,
}

impl ReportedLlmUsageAccumulator {
    pub fn observe_snapshot(&mut self, usage: ReportedLlmUsage) {
        self.prompt_tokens = self.prompt_tokens.max(usage.prompt_tokens);
        self.completion_tokens = self.completion_tokens.max(usage.completion_tokens);
        self.total_tokens = self.total_tokens.max(usage.total_tokens);

        if let Some(cost) = usage.reported_cost {
            self.reported_cost = Some(
                self.reported_cost
                    .map(|current| current.max(cost))
                    .unwrap_or(cost),
            );
        }
    }

    pub fn observe_delta(&mut self, usage: ReportedLlmUsage) {
        self.prompt_tokens = self.prompt_tokens.saturating_add(usage.prompt_tokens);
        self.completion_tokens = self
            .completion_tokens
            .saturating_add(usage.completion_tokens);
        self.total_tokens = self.total_tokens.saturating_add(usage.total_tokens);

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
            self.prompt_tokens + self.completion_tokens
        };

        let usage = ReportedLlmUsage {
            prompt_tokens: self.prompt_tokens,
            completion_tokens: self.completion_tokens,
            total_tokens,
            reported_cost: self.reported_cost,
        };

        (!usage.is_empty()).then_some(usage)
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
    pointers
        .iter()
        .find_map(|pointer| number_at(value, pointer))
        .map(|value| value.max(0.0) as u64)
}

pub fn extract_reported_usage(value: &serde_json::Value) -> Option<ReportedLlmUsage> {
    let prompt_tokens = token_at(
        value,
        &[
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
            "/total_tokens",
            "/usage/total_tokens",
            "/response/usage/total_tokens",
            "/message/usage/total_tokens",
        ],
    )
    .unwrap_or_else(|| prompt_tokens + completion_tokens);

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
        reported_cost,
    };

    (!usage.is_empty()).then_some(usage)
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
        ReportedLlmUsage, ReportedLlmUsageAccumulator, UsageAggregationMode,
        extract_reported_usage, extract_reported_usage_from_sse_event,
    };

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
                reported_cost: Some(0.0042),
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
    fn accumulator_keeps_latest_cumulative_values() {
        let mut accumulator = ReportedLlmUsageAccumulator::default();
        accumulator.observe_snapshot(ReportedLlmUsage {
            prompt_tokens: 10,
            completion_tokens: 0,
            total_tokens: 0,
            reported_cost: None,
        });
        accumulator.observe_snapshot(ReportedLlmUsage {
            prompt_tokens: 10,
            completion_tokens: 20,
            total_tokens: 30,
            reported_cost: Some(0.012),
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
            reported_cost: Some(0.001),
        });
        accumulator.observe_delta(ReportedLlmUsage {
            prompt_tokens: 3,
            completion_tokens: 7,
            total_tokens: 10,
            reported_cost: Some(0.002),
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
    fn done_marker_returns_none() {
        let result = extract_reported_usage_from_sse_event(None, "[DONE]");
        assert!(result.is_none());

        // Also with whitespace
        let result2 = extract_reported_usage_from_sse_event(Some("message"), "  [DONE]  ");
        assert!(result2.is_none());
    }
}

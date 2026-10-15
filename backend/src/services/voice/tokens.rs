//! Reported response tokens have their own normal reservation, never duration units.
use crate::services::{
    billing::{BillingRouteContext, meter::MeteredProxyContext},
    llm_usage_service,
};
use crate::{
    AppState,
    errors::AppResult,
    models::service_billing::{BillingMetric, PlatformUsage},
};
use serde_json::Value;

pub fn context(mut ctx: BillingRouteContext, id: String) -> BillingRouteContext {
    let mut specs: Vec<_> = ctx
        .platform_specs()
        .filter(|(m, _)| m.is_token_family())
        .map(|(m, code)| crate::models::service_billing::ResaleSpec {
            metric: m,
            lago_metric_code: code.into(),
        })
        .collect();
    if specs.is_empty() {
        ctx.platform_metric = BillingMetric::Tokens;
        ctx.platform_lago_metric_code = "tokens".into();
        ctx.service_platform_billable = false;
        ctx.platform_components.clear();
    } else {
        let first = specs.remove(0);
        ctx.platform_metric = first.metric;
        ctx.platform_lago_metric_code = first.lago_metric_code;
        ctx.platform_components = specs;
    }
    ctx.billing_request_id = id;
    ctx.capture_tokens = true;
    ctx.requested_voice_seconds = 0;
    ctx.requested_images = 0;
    ctx.voice_initial_window = false;
    // A reservation estimate only. Actual settlement never estimates from PCM or text bytes.
    ctx.request_bytes = 32_768;
    ctx
}
pub async fn reserve(
    state: &AppState,
    ctx: &BillingRouteContext,
    session: &str,
    sequence: u64,
) -> AppResult<MeteredProxyContext> {
    let ctx = context(ctx.clone(), format!("voice:{session}:response:{sequence}"));
    let meter = state.billing.open(&ctx).await?;
    state.billing.mark_forwarded(&meter).await?;
    Ok(meter)
}
pub async fn settle(
    state: &AppState,
    meter: &MeteredProxyContext,
    event: &Value,
    model: &str,
) -> AppResult<()> {
    let usage = reported_usage(event);
    state
        .billing
        .settle_deferred(meter, usage, None, Some(model.into()))
        .await
}

fn reported_usage(event: &Value) -> PlatformUsage {
    // The shared reported parser/collector handles token provenance, but byte fallback
    // is intentionally never invoked on this surface.
    let observation = llm_usage_service::extract_realtime_response_done(event)
        .and_then(|done| done.usage)
        .and_then(|usage| usage.observation);
    let mut collector = llm_usage_service::RealtimeLlmUsageCollector::new(true);
    collector.observe_downstream_text(&event.to_string());
    let mut usage = collector
        .finalize()
        .reported_usage
        .map(|u| {
            let mut usage = u.apply_to(PlatformUsage::default());
            usage.tokens = u.total_tokens.min(i64::MAX as u64) as i64;
            usage
        })
        .unwrap_or_else(|| {
            let mut usage = PlatformUsage::default();
            usage.llm_observation = Some(crate::services::llm_observation::missing(
                crate::models::llm_usage::UsageOutcome::Unknown,
            ));
            usage
        });
    // Keep the collector's legacy estimate/zero-total decision, while this
    // per-response meter carries an invocation observation, not a connection.
    if let Some(observation) = observation {
        usage.llm_observation = Some(observation);
    }
    usage
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn grok_reports_tokens_only_and_never_estimates_missing_usage() {
        let usage = reported_usage(&json!({"type":"response.done","response":{
            "id":"response1","usage":{"input_tokens":80,"output_tokens":20,"total_tokens":100}}}));
        assert_eq!(usage.tokens, 100);
        assert_eq!(usage.voice_seconds, 0);
        let missing = reported_usage(&json!({"type":"response.done","response":{
            "id":"response2","output":[{"text":"Text is not a billable token estimate"}]}}));
        assert_eq!(missing.tokens, 0);
        assert_eq!(missing.voice_seconds, 0);
        let zero = reported_usage(&json!({"type":"response.done","response":{
            "id":"zero","usage":{"input_tokens":0,"output_tokens":0,"total_tokens":0,
            "input_token_details":{"cached_tokens":17}}}}));
        assert_eq!((zero.tokens, zero.cache_read_tokens), (0, 0));
        let o = zero.llm_observation.unwrap();
        assert_eq!(o.cache_read_tokens, Some(17));
        assert!(o.invalid_counts);
        assert_eq!(
            o.aggregation_scope,
            crate::models::llm_usage::UsageAggregationScope::Invocation
        );
        let malformed = reported_usage(&json!({"type":"response.done","response":{
            "id":"malformed","usage":{"total_tokens":"0","input_token_details":{"cached_tokens":17}}}}));
        assert_eq!((malformed.tokens, malformed.cache_read_tokens), (0, 0));
        assert!(malformed.llm_observation.unwrap().invalid_counts);
    }
    #[test]
    fn grok_token_context_excludes_seconds_and_request_components() {
        use crate::models::{service_billing::ResaleSpec, usage_meter::CredentialClass};
        use crate::services::billing::{BillingIngress, NodeIntent};
        let mut ctx = BillingRouteContext::new(
            BillingIngress::LlmProvider,
            "request".into(),
            "payer".into(),
            "actor".into(),
            None,
            None,
            None,
            None,
            NodeIntent::Direct,
            "bearer".into(),
            CredentialClass::UserOwned,
            BillingMetric::VoiceSeconds,
            None,
            false,
        );
        ctx.platform_components = vec![
            ResaleSpec {
                metric: BillingMetric::InputTokens,
                lago_metric_code: "input".into(),
            },
            ResaleSpec {
                metric: BillingMetric::OutputTokens,
                lago_metric_code: "output".into(),
            },
            ResaleSpec {
                metric: BillingMetric::Requests,
                lago_metric_code: "requests".into(),
            },
        ];
        let ctx = context(ctx, "stable".into());
        assert_eq!(
            ctx.platform_specs().map(|(m, _)| m).collect::<Vec<_>>(),
            [BillingMetric::InputTokens, BillingMetric::OutputTokens]
        );
        assert_eq!(ctx.requested_voice_seconds, 0);
        assert!(!ctx.voice_initial_window);
    }
}

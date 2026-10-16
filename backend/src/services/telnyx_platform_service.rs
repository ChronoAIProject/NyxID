//! Bounded Telnyx platform execution shared by HTTP and MCP. The dedicated
//! contract never uses generic token estimates or incomplete-usage settlement.

use std::sync::Arc;

use bytes::Bytes;
use chrono::{Duration, Utc};
use mongodb::Database;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use zeroize::Zeroize;

use super::billing::{BillingRouteContext, BillingService, NodeIntent};
use super::proxy_service::{ProxyBody, ProxyTarget};
use crate::errors::{AppError, AppResult};
use crate::models::downstream_service::DownstreamService;
use crate::models::service_billing::{
    BillingMetric, ManagedExecutionContract, PlatformUsage, ServiceBilling,
};
use crate::models::usage_meter::CredentialClass;

pub(crate) const CHAT_PATH: &str = "ai/openai/chat/completions";

fn invalid(message: &str) -> AppError {
    AppError::ValidationError(format!("Telnyx platform: {message}"))
}

pub(crate) fn contract(service: &DownstreamService) -> Option<&ManagedExecutionContract> {
    service.billing.as_ref()?.managed_execution.as_ref()
}

pub(crate) fn validate_billing(billing: &ServiceBilling) -> AppResult<()> {
    let Some(c) = billing.managed_execution.as_ref() else {
        return Ok(());
    };
    if c.model.is_empty()
        || c.model.len() > 200
        || !c
            .model
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"/._:-".contains(&b))
        || !(1..=2_000_000).contains(&c.max_input_tokens)
        || !(1..=32_768).contains(&c.max_output_tokens)
        || c.max_output_tokens >= c.max_input_tokens
        || !(256..=1_048_576).contains(&c.max_request_bytes)
        || !(1024..=8_388_608).contains(&c.max_response_bytes)
        || c.tariff_version.trim().is_empty()
        || c.tariff_version.len() > 200
    {
        return Err(invalid(
            "a fixed model, verified context/output bounds and bounded body sizes are required",
        ));
    }
    let source = url::Url::parse(&c.tariff_source)
        .map_err(|_| invalid("tariff_source must be a Telnyx HTTPS evidence URL"))?;
    if source.scheme() != "https"
        || !matches!(
            source.host_str(),
            Some("telnyx.com" | "api.telnyx.com" | "developers.telnyx.com")
        )
        || !source.username().is_empty()
        || source.password().is_some()
        || source.query().is_some()
        || source.fragment().is_some()
        || c.tariff_source.len() > 2048
    {
        return Err(invalid(
            "tariff_source must be a Telnyx HTTPS evidence URL without credentials or query parameters",
        ));
    }
    if c.tariff_verified_at > Utc::now() + Duration::minutes(5) {
        return Err(invalid("tariff verification cannot be in the future"));
    }
    let lane = billing
        .platform_key_pricing
        .as_ref()
        .ok_or_else(|| invalid("platform-key input, output and cache-read prices are required"))?;
    let mut metrics = vec![lane.metric];
    metrics.extend(lane.components.iter().map(|p| p.metric));
    if !billing.platform_billable
        || billing.resale_billable
        || billing.byok_pricing.is_some()
        || billing.platform_pricing.is_some()
        || !lane.operations.is_empty()
        || metrics.len() != 3
        || lane.metric != BillingMetric::InputTokens
        || !metrics.contains(&BillingMetric::OutputTokens)
        || !metrics.contains(&BillingMetric::CacheReadTokens)
    {
        return Err(invalid(
            "dedicated platform billing requires input-token primary, output/cache-read components, and no BYOK, legacy or resale prices",
        ));
    }
    Ok(())
}

/// Validate effective catalog state after both create and partial update.
pub(crate) fn validate_catalog(service: &DownstreamService) -> AppResult<()> {
    let Some(billing) = service
        .billing
        .as_ref()
        .filter(|b| b.managed_execution.is_some())
    else {
        return Ok(());
    };
    validate_billing(billing)?;
    if service.base_url.trim_end_matches('/') != "https://api.telnyx.com/v2"
        || service.auth_method != "bearer"
        || service
            .auth_scheme
            .as_deref()
            .is_some_and(|s| s != "Bearer")
        || service.requires_user_credential
        || service.service_category != "internal"
        || service.service_type != "http"
        || !service.destination_targets.is_empty()
        || service.inference.is_some()
        || service.identity_propagation_mode != "none"
        || service.forward_access_token
        || service.inject_delegation_token
        || service.anonymous_endpoints.iter().any(|r| r.enabled)
        || !service.ws_frame_injections.is_empty()
        || service.token_exchange_config.is_some()
        || service
            .default_request_headers
            .as_ref()
            .is_some_and(|h| !h.is_empty())
        || service.platform_key.as_ref().is_none_or(|p| {
            p.audience != crate::models::downstream_service::PlatformKeyAudience::Restricted
        })
    {
        return Err(invalid(
            "use a dedicated internal bearer-authenticated HTTP service at https://api.telnyx.com/v2 with a restricted platform audience and identity forwarding, inference routing and anonymous access disabled",
        ));
    }
    Ok(())
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ChatRequest {
    model: String,
    messages: Vec<TextMessage>,
    #[serde(default)]
    stream: bool,
    max_tokens: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f64>,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct TextMessage {
    role: String,
    content: String,
}

pub(crate) fn validate_request(
    c: &ManagedExecutionContract,
    method: &str,
    path: &str,
    query: Option<&str>,
    body: &[u8],
) -> AppResult<Bytes> {
    if method != "POST"
        || path.trim_start_matches('/') != CHAT_PATH
        || query.is_some_and(|q| !q.is_empty())
    {
        return Err(invalid(
            "only POST /ai/openai/chat/completions without query parameters is supported",
        ));
    }
    if body.len() as u64 > c.max_request_bytes {
        return Err(AppError::RequestBodyTooLarge {
            max_bytes: c.max_request_bytes as usize,
            context: "Telnyx platform chat".into(),
        });
    }
    let request: ChatRequest = serde_json::from_slice(body)
        .map_err(|_| invalid("expected model, text-only messages and max_tokens; optional temperature and stream=false"))?;
    if request.model != c.model
        || request.stream
        || request.max_tokens == 0
        || request.max_tokens > c.max_output_tokens
        || request.messages.is_empty()
        || request.messages.len() > 128
        || request.messages.iter().any(|m| {
            !matches!(m.role.as_str(), "system" | "user" | "assistant") || m.content.is_empty()
        })
        || !request.messages.iter().any(|m| m.role == "user")
        || request
            .temperature
            .is_some_and(|t| !t.is_finite() || !(0.0..=2.0).contains(&t))
    {
        return Err(invalid(
            "unsupported model, messages, streaming or generation limit",
        ));
    }
    let normalized = serde_json::to_vec(&request).map_err(|_| invalid("invalid chat request"))?;
    if normalized.len() as u64 > c.max_request_bytes {
        return Err(AppError::RequestBodyTooLarge {
            max_bytes: c.max_request_bytes as usize,
            context: "Telnyx platform chat".into(),
        });
    }
    Ok(Bytes::from(normalized))
}

/// Complete evidence is mandatory. In particular, missing cached usage cannot
/// silently become zero and overcharge ordinary input at the cached rate.
pub(crate) fn validate_usage(
    c: &ManagedExecutionContract,
    body: &[u8],
    request_bytes: usize,
) -> AppResult<(PlatformUsage, String)> {
    let value: serde_json::Value =
        serde_json::from_slice(body).map_err(|_| invalid("provider returned invalid JSON"))?;
    if value.get("model").and_then(|v| v.as_str()) != Some(c.model.as_str()) {
        return Err(invalid("provider model did not match the admitted model"));
    }
    let id = value
        .get("id")
        .and_then(|v| v.as_str())
        .filter(|v| {
            !v.is_empty()
                && v.len() <= 256
                && v.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
        })
        .ok_or_else(|| invalid("provider request identity is missing"))?;
    let count = |pointer: &str| {
        value
            .pointer(pointer)
            .and_then(|v| v.as_u64())
            .ok_or_else(|| invalid("provider token usage is incomplete"))
    };
    let input = count("/usage/prompt_tokens")?;
    let output = count("/usage/completion_tokens")?;
    let cached = count("/usage/prompt_tokens_details/cached_tokens")?;
    let total = count("/usage/total_tokens")?;
    if input > c.max_input_tokens
        || output > c.max_output_tokens
        || cached > input
        || input.checked_add(output) != Some(total)
        || total > c.max_input_tokens
        || value
            .get("choices")
            .and_then(|v| v.as_array())
            .is_none_or(|v| v.len() != 1)
        || value
            .pointer("/choices/0/message/role")
            .and_then(|v| v.as_str())
            != Some("assistant")
        || !value
            .pointer("/choices/0/message/content")
            .is_some_and(|v| v.is_string() || v.is_null())
        || value.pointer("/choices/0/message/tool_calls").is_some()
    {
        return Err(invalid(
            "provider usage or completion violates the admitted contract",
        ));
    }
    // These modalities are outside the tariff contract, including tokens
    // reported in otherwise compatible OpenAI detail objects.
    for pointer in [
        "/usage/prompt_tokens_details/audio_tokens",
        "/usage/prompt_tokens_details/image_tokens",
        "/usage/completion_tokens_details/audio_tokens",
        "/usage/completion_tokens_details/image_tokens",
        "/usage/cache_write_tokens",
        "/usage/cache_creation_input_tokens",
        "/usage/prompt_tokens_details/cache_write_tokens",
        "/usage/prompt_tokens_details/cache_creation_tokens",
        "/usage/input_tokens_details/cache_write_tokens",
    ] {
        if value
            .pointer(pointer)
            .is_some_and(|v| v.as_u64() != Some(0))
        {
            return Err(invalid("provider reported an unsupported token modality"));
        }
    }
    Ok((
        PlatformUsage {
            requests: 1,
            bytes: (request_bytes + body.len()) as i64,
            tokens: total as i64,
            input_tokens: (input - cached) as i64,
            output_tokens: output as i64,
            cache_read_tokens: cached as i64,
            ..Default::default()
        },
        id.to_owned(),
    ))
}

fn digest(parts: &[&[u8]]) -> [u8; 32] {
    let mut hash = Sha256::new();
    for part in parts {
        hash.update((part.len() as u64).to_be_bytes());
        hash.update(part);
    }
    hash.finalize().into()
}

fn catalog_digest(target: &ProxyTarget) -> AppResult<[u8; 32]> {
    let billing = serde_json::to_vec(&target.service.billing)
        .map_err(|_| invalid("invalid billing contract"))?;
    Ok(digest(&[
        target.service.id.as_bytes(),
        target.base_url.as_bytes(),
        target.auth_method.as_bytes(),
        &billing,
    ]))
}

/// Only this runner can construct the managed egress proof, after durable
/// funding and dispatch admission. A normal route permit has no such proof.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ManagedEgressProof {
    catalog: [u8; 32],
    request: [u8; 32],
}

impl ManagedEgressProof {
    pub(crate) fn validate(
        &self,
        target: &ProxyTarget,
        method: &str,
        path: &str,
        query: Option<&str>,
        body: &ProxyBody,
    ) -> AppResult<()> {
        let ProxyBody::Buffered(Some(body)) = body else {
            return Err(invalid("managed execution requires a buffered body"));
        };
        if method != "POST"
            || path != CHAT_PATH
            || query.is_some()
            || self.catalog != catalog_digest(target)?
            || self.request != digest(&[body])
        {
            return Err(invalid(
                "managed execution admission does not match outbound request",
            ));
        }
        Ok(())
    }
}

pub(crate) struct ManagedResponse {
    pub execution_id: String,
    pub status: u16,
    pub body: Bytes,
}

pub(crate) enum ManagedExecutionError {
    Rejected(AppError),
    OutcomeUnknown(AppError),
}
impl From<AppError> for ManagedExecutionError {
    fn from(error: AppError) -> Self {
        Self::Rejected(error)
    }
}
impl ManagedExecutionError {
    pub(crate) fn into_app_error(self) -> AppError {
        match self {
            Self::Rejected(error) | Self::OutcomeUnknown(error) => error,
        }
    }
}

struct SecretTarget(ProxyTarget);
impl std::ops::Deref for SecretTarget {
    type Target = ProxyTarget;
    fn deref(&self) -> &ProxyTarget {
        &self.0
    }
}
impl Drop for SecretTarget {
    fn drop(&mut self) {
        self.0.credential.zeroize();
    }
}

/// The spawned exchange owns its credential and accounting work. Dropping the
/// caller future detaches the join handle; provider usage still gets settled.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn execute(
    db: &Database,
    billing: Arc<BillingService>,
    ctx: BillingRouteContext,
    target: ProxyTarget,
    method: &str,
    path: &str,
    query: Option<&str>,
    body: &[u8],
    lease: Option<super::service_concurrency_service::Lease>,
) -> Result<ManagedResponse, ManagedExecutionError> {
    let target = SecretTarget(target);
    validate_catalog(&target.service)?;
    let mut c = contract(&target.service)
        .ok_or_else(|| invalid("managed contract is missing"))?
        .clone();
    if ctx.credential_class != CredentialClass::NyxidManagedMaster
        || ctx.node_intent != NodeIntent::Direct
        || ctx.pool_attempt.is_some()
        || target.target_id.is_some()
        || target.base_url.trim_end_matches('/') != "https://api.telnyx.com/v2"
        || target.auth_method != "bearer"
        || target.credential.is_empty()
        || !target.catalog_default_headers.is_empty()
        || !target.user_service_default_headers.is_empty()
    {
        return Err(invalid(
            "managed execution requires a server-held platform credential and a direct route",
        )
        .into());
    }
    if Utc::now() - c.tariff_verified_at > Duration::days(30) {
        return Err(AppError::BillingProviderUnavailable(
            "Telnyx tariff verification has expired".into(),
        )
        .into());
    }
    let body = validate_request(&c, method, path, query, body)?;
    // Reserve the configured maximum, but reject provider usage beyond this
    // individual request's smaller generation cap as incomplete evidence.
    c.max_output_tokens = serde_json::from_slice::<ChatRequest>(&body)
        .map_err(|_| invalid("invalid normalized request"))?
        .max_tokens;
    let request_digest = digest(&[&body]);
    let catalog_digest = catalog_digest(&target)?;
    let handle = super::managed_execution_service::prepare(
        db,
        &billing,
        ctx,
        &hex::encode(request_digest),
        &hex::encode(catalog_digest),
    )
    .await?;
    let execution_id = handle.execution_id().to_owned();
    let worker_db = db.clone();
    let worker_id = execution_id.clone();
    let worker = tokio::spawn(async move {
        let proof = ManagedEgressProof {
            catalog: catalog_digest,
            request: request_digest,
        };
        let permit = super::billing::route_inventory::managed_execution_permit(proof);
        let exchange = async {
            super::managed_execution_service::admit_dispatch(&worker_db, &handle).await?;
            let response = super::proxy_service::forward_request_with_extra_outbound_headers(
                super::proxy_service::no_redirect_http_client(),
                &target,
                reqwest::Method::POST,
                CHAT_PATH,
                None,
                reqwest::header::HeaderMap::from_iter([(
                    reqwest::header::CONTENT_TYPE,
                    reqwest::header::HeaderValue::from_static("application/json"),
                )]),
                ProxyBody::Buffered(Some(body.clone())),
                Vec::new(),
                Vec::new(),
                None,
                &super::provider_token_exchange_service::TokenExchangeCache::new(),
                &super::cloud_response_cache::CloudResponseCache::new(0),
                Vec::new(),
                permit,
            )
            .await
            .map_err(|_| invalid("provider outcome is pending"))?;
            let mut response = response;
            let status = response.status().as_u16();
            let mut bytes = Vec::new();
            while let Some(chunk) = response
                .chunk()
                .await
                .map_err(|_| invalid("provider response is incomplete"))?
            {
                if bytes.len().saturating_add(chunk.len()) as u64 > c.max_response_bytes {
                    return Err(invalid("provider response exceeded its bound"));
                }
                bytes.extend_from_slice(&chunk);
            }
            // Even error statuses require authoritative complete usage; an
            // unverified provider rejection never releases possible liability.
            if !(200..300).contains(&status) {
                return Err(invalid("provider returned an unverified error outcome"));
            }
            let (usage, provider_id) = validate_usage(&c, &bytes, body.len())?;
            super::managed_execution_service::complete(
                &worker_db,
                &billing,
                &handle,
                usage,
                Some(&provider_id),
            )
            .await?;
            Ok::<_, AppError>(ManagedResponse {
                execution_id: worker_id.clone(),
                status,
                body: Bytes::from(bytes),
            })
        };
        let exchange = async {
            match lease {
                Some(lease) => lease.run(exchange).await,
                None => exchange.await,
            }
        };
        match tokio::time::timeout(std::time::Duration::from_secs(120), exchange).await {
            Ok(Ok(response)) => Ok(response),
            _ => {
                // Safe no-op after dispatch; before dispatch this returns the
                // reservation, including when the concurrency lease was lost.
                if super::managed_execution_service::cancel_unforwarded(&worker_db, &handle)
                    .await
                    .is_err()
                    || super::managed_execution_service::mark_pending(
                        &worker_db,
                        &handle,
                        "provider_outcome_unverified",
                    )
                    .await
                    .is_err()
                {
                    tracing::error!(execution_id = %worker_id, "Managed outcome requires durable recovery");
                }
                Err(ManagedExecutionError::OutcomeUnknown(
                    AppError::ManagedExecutionPending(worker_id),
                ))
            }
        }
    });
    worker.await.map_err(|_| {
        ManagedExecutionError::OutcomeUnknown(AppError::ManagedExecutionPending(execution_id))
    })?
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::service_billing::{LanePriceComponent, LanePricing, PricingSyncStatus};

    fn fixture_contract() -> ManagedExecutionContract {
        ManagedExecutionContract {
            model: "verified/test-model".into(),
            max_input_tokens: 8192,
            max_output_tokens: 512,
            max_request_bytes: 4096,
            max_response_bytes: 65536,
            tariff_version: "account-verified-v1".into(),
            tariff_source: "https://api.telnyx.com/v2/pricing/products/inference".into(),
            tariff_verified_at: Utc::now(),
        }
    }

    fn fixture_billing() -> ServiceBilling {
        ServiceBilling {
            managed_execution: Some(fixture_contract()),
            platform_billable: true,
            platform_key_pricing: Some(LanePricing {
                metric: BillingMetric::InputTokens,
                credits_per_unit: "0.000001".into(),
                lago_metric_code: "platform_svc_telnyx_platform_pk".into(),
                sync_status: PricingSyncStatus::Synced,
                sync_error: None,
                operations: Vec::new(),
                components: vec![BillingMetric::OutputTokens, BillingMetric::CacheReadTokens]
                    .into_iter()
                    .map(|metric| LanePriceComponent {
                        metric,
                        credits_per_unit: "0.000002".into(),
                        lago_metric_code: format!(
                            "platform_svc_telnyx_platform_pk_{}",
                            metric.as_str()
                        ),
                        sync_status: PricingSyncStatus::Synced,
                        sync_error: None,
                    })
                    .collect(),
            }),
            ..Default::default()
        }
    }

    fn fixture_request() -> serde_json::Value {
        serde_json::json!({"model":"verified/test-model","messages":[{"role":"user","content":"hello"}],"max_tokens":100})
    }

    fn fixture_response() -> serde_json::Value {
        serde_json::json!({"id":"chatcmpl-test","model":"verified/test-model","choices":[{"message":{"role":"assistant","content":"hello"}}],
            "usage":{"prompt_tokens":100,"completion_tokens":10,"total_tokens":110,"prompt_tokens_details":{"cached_tokens":40}}})
    }

    #[test]
    fn telnyx_request_rejects_spend_and_stateful_escape_hatches() {
        let c = fixture_contract();
        let valid = fixture_request();
        assert!(
            validate_request(
                &c,
                "POST",
                CHAT_PATH,
                None,
                &serde_json::to_vec(&valid).unwrap()
            )
            .is_ok()
        );
        for (field, value) in [
            ("model", serde_json::json!("another/model")),
            ("stream", serde_json::json!(true)),
            ("max_tokens", serde_json::json!(513)),
            ("max_tokens", serde_json::json!(-1)),
            ("tools", serde_json::json!([])),
            ("conversation", serde_json::json!("someone-elses")),
            ("n", serde_json::json!(2)),
            ("extra_body", serde_json::json!({})),
            (
                "messages",
                serde_json::json!([{"role":"user","content":[{"type":"image_url","image_url":{"url":"https://example.com"}}]}]),
            ),
        ] {
            let mut request = valid.clone();
            request[field] = value;
            assert!(
                validate_request(
                    &c,
                    "POST",
                    CHAT_PATH,
                    None,
                    &serde_json::to_vec(&request).unwrap()
                )
                .is_err(),
                "accepted {field}"
            );
        }
        let bytes = serde_json::to_vec(&valid).unwrap();
        assert!(validate_request(&c, "GET", CHAT_PATH, None, &bytes).is_err());
        assert!(validate_request(&c, "POST", "messages", None, &bytes).is_err());
        assert!(validate_request(&c, "POST", CHAT_PATH, Some("model=expensive"), &bytes).is_err());
        assert!(validate_request(&c, "POST", CHAT_PATH, None, &vec![b' '; 4097]).is_err());
    }

    #[test]
    fn telnyx_usage_prices_cached_input_once() {
        let (usage, id) = validate_usage(
            &fixture_contract(),
            &serde_json::to_vec(&fixture_response()).unwrap(),
            25,
        )
        .unwrap();
        assert_eq!(id, "chatcmpl-test");
        assert_eq!(
            (
                usage.tokens,
                usage.input_tokens,
                usage.output_tokens,
                usage.cache_read_tokens
            ),
            (110, 60, 10, 40)
        );
    }

    #[test]
    fn telnyx_usage_never_invents_missing_counts_or_accepts_invalid_evidence() {
        let c = fixture_contract();
        let valid = fixture_response();
        for pointer in [
            "/usage/prompt_tokens",
            "/usage/completion_tokens",
            "/usage/total_tokens",
            "/usage/prompt_tokens_details/cached_tokens",
        ] {
            for value in [
                serde_json::Value::Null,
                serde_json::json!(-1),
                serde_json::json!(1.5),
                serde_json::json!("100"),
            ] {
                let mut response = valid.clone();
                *response.pointer_mut(pointer).unwrap() = value;
                assert!(
                    validate_usage(&c, &serde_json::to_vec(&response).unwrap(), 25).is_err(),
                    "accepted {pointer}"
                );
            }
        }
        for (pointer, value) in [
            ("/model", serde_json::json!("another/model")),
            ("/usage/total_tokens", serde_json::json!(111)),
            (
                "/usage/prompt_tokens_details/cached_tokens",
                serde_json::json!(101),
            ),
            ("/usage/completion_tokens", serde_json::json!(513)),
        ] {
            let mut response = valid.clone();
            *response.pointer_mut(pointer).unwrap() = value;
            assert!(validate_usage(&c, &serde_json::to_vec(&response).unwrap(), 25).is_err());
        }
        let mut response = valid;
        response["usage"]
            .as_object_mut()
            .unwrap()
            .remove("prompt_tokens_details");
        assert!(validate_usage(&c, &serde_json::to_vec(&response).unwrap(), 25).is_err());
    }

    #[test]
    fn telnyx_contract_requires_all_tariff_classes_and_exact_bson_roundtrip() {
        let billing = fixture_billing();
        assert!(validate_billing(&billing).is_ok());
        let stored = mongodb::bson::to_document(&billing).unwrap();
        assert!(
            stored
                .get_document("managed_execution")
                .unwrap()
                .get_datetime("tariff_verified_at")
                .is_ok()
        );
        let restored: ServiceBilling = mongodb::bson::from_document(stored).unwrap();
        assert_eq!(
            restored.managed_execution.unwrap().model,
            "verified/test-model"
        );
        let mut http = serde_json::to_value(&billing).unwrap();
        http["managed_execution"]["tariff_verified_at"] =
            serde_json::json!({"$date":"2026-10-16T00:00:00Z"});
        let decoded: ServiceBilling = serde_json::from_value(http).unwrap();
        assert_eq!(
            decoded
                .managed_execution
                .unwrap()
                .tariff_verified_at
                .to_rfc3339(),
            "2026-10-16T00:00:00+00:00"
        );
        for metric in [BillingMetric::OutputTokens, BillingMetric::CacheReadTokens] {
            let mut incomplete = billing.clone();
            incomplete
                .platform_key_pricing
                .as_mut()
                .unwrap()
                .components
                .retain(|p| p.metric != metric);
            assert!(validate_billing(&incomplete).is_err());
        }
        let mut duplicate = billing;
        duplicate.platform_key_pricing.as_mut().unwrap().components[0].metric =
            BillingMetric::CacheReadTokens;
        assert!(validate_billing(&duplicate).is_err());
    }

    #[tokio::test]
    async fn ordinary_egress_permit_cannot_dispatch_managed_service() {
        let mut service = crate::models::downstream_service::test_helpers::dummy_service();
        service.billing = Some(fixture_billing());
        let target = ProxyTarget {
            workspace_destinations_pending: false,
            target_id: None,
            base_url: "https://api.telnyx.com/v2".into(),
            auth_method: "bearer".into(),
            auth_key_name: String::new(),
            credential: "test-only".into(),
            service,
            catalog_default_headers: vec![],
            user_service_default_headers: vec![],
            ws_frame_injections: vec![],
            connection_id: None,
        };
        let permit = super::super::billing::route_inventory::internal_node_dispatch_permit();
        let result = super::super::proxy_service::forward_request_with_extra_outbound_headers(
            &reqwest::Client::new(),
            &target,
            reqwest::Method::POST,
            CHAT_PATH,
            None,
            reqwest::header::HeaderMap::new(),
            ProxyBody::Buffered(Some(Bytes::from_static(b"{}"))),
            vec![],
            vec![],
            None,
            &super::super::provider_token_exchange_service::TokenExchangeCache::new(),
            &super::super::cloud_response_cache::CloudResponseCache::new(0),
            vec![],
            permit,
        )
        .await;
        assert!(matches!(
            result,
            Err(
                super::super::proxy_service::ForwardRequestError::Application(
                    AppError::BillingProviderUnavailable(_)
                )
            )
        ));
        let proof = ManagedEgressProof {
            catalog: catalog_digest(&target).unwrap(),
            request: digest(&[b"original"]),
        };
        assert!(
            proof
                .validate(
                    &target,
                    "POST",
                    CHAT_PATH,
                    None,
                    &ProxyBody::Buffered(Some(Bytes::from_static(b"modified")))
                )
                .is_err()
        );
    }
}

use crate::models::downstream_service::DownstreamService;
use crate::models::service_billing::BillingMetric;

/// Resolve the service-level metric used by allowances and catalog UIs.
/// Prefer BYOK's configured unit, then platform key, then the legacy default.
/// Each request independently selects its actual credential lane and transport.
pub fn effective_platform_metric(service: &DownstreamService) -> BillingMetric {
    configured_lane_metrics(service)
        .first()
        .copied()
        .unwrap_or_else(|| resolve_platform_metric(service, false))
}

/// Configured allowance units, BYOK first then platform key, without duplicates.
/// Pending lanes are included so allowances can be prepared before price sync.
pub fn configured_lane_metrics(service: &DownstreamService) -> Vec<BillingMetric> {
    let mut metrics = Vec::new();
    if let Some(billing) = &service.billing {
        for lane in [
            billing.byok_pricing.as_ref(),
            billing.platform_key_pricing.as_ref(),
        ]
        .into_iter()
        .flatten()
        {
            for metric in
                std::iter::once(lane.metric).chain(lane.components.iter().map(|c| c.metric))
            {
                if !metrics.contains(&metric) {
                    metrics.push(metric);
                }
            }
        }
    }
    metrics
}

/// All accepted allowance units, including legacy fallback only while a primary
/// is unsynced (or when there are no lanes). Shared by validation and admin UIs.
pub fn allowance_metrics(service: &DownstreamService) -> Vec<BillingMetric> {
    let mut metrics = configured_lane_metrics(service);
    // Pending/failed lanes can still charge the legacy metric during sync.
    if metrics.is_empty()
        || service.billing.as_ref().is_some_and(|b| {
            [b.byok_pricing.as_ref(), b.platform_key_pricing.as_ref()]
                .into_iter()
                .flatten()
                .any(|lane| {
                    lane.sync_status != crate::models::service_billing::PricingSyncStatus::Synced
                })
        })
    {
        let fallback = resolve_platform_metric(service, false);
        if !metrics.contains(&fallback) {
            metrics.push(fallback);
        }
    }
    metrics
}

pub fn allowance_metric(
    service: &DownstreamService,
    requested: Option<BillingMetric>,
) -> crate::errors::AppResult<BillingMetric> {
    let Some(requested) = requested else {
        return Ok(effective_platform_metric(service));
    };
    if allowance_metrics(service).contains(&requested) {
        Ok(requested)
    } else {
        Err(crate::errors::AppError::ValidationError(
            "Allowance metric must match a configured billing lane or the current fallback metric"
                .into(),
        ))
    }
}

/// Usage capture is independent of the chosen lane's sync state and slug.
/// Preserve historical LLM capture and include both configured lanes and resale.
pub fn captures_tokens(service: &DownstreamService) -> bool {
    service.slug.starts_with("llm-")
        || resolve_platform_metric(service, false) == BillingMetric::Tokens
        || configured_lane_metrics(service)
            .iter()
            .any(|metric| metric.is_token_family())
        || service
            .billing
            .as_ref()
            .is_some_and(|b| b.resale_billable && b.resale_metric == BillingMetric::Tokens)
}

/// Image-priced services also need bounded JSON/SSE usage capture.
pub fn captures_usage(service: &DownstreamService) -> bool {
    captures_tokens(service) || configured_lane_metrics(service).contains(&BillingMetric::Images)
}

/// Resolve the metric for one proxy request. Only an actual WebSocket
/// connection, not advertised WebSocket capability, changes the fallback
/// metric to bytes.
pub fn platform_metric_for_request(
    service: &DownstreamService,
    is_connection: bool,
) -> BillingMetric {
    resolve_platform_metric(service, is_connection)
}

fn resolve_platform_metric(service: &DownstreamService, is_connection: bool) -> BillingMetric {
    if let Some(metric) = service
        .billing
        .as_ref()
        .and_then(|billing| billing.platform_metric)
    {
        metric
    } else if is_connection || service.service_type == "ssh" {
        BillingMetric::Bytes
    } else if service.slug.starts_with("llm-") {
        BillingMetric::Tokens
    } else {
        BillingMetric::Requests
    }
}

#[cfg(test)]
mod tests {
    use crate::models::downstream_service::{ServiceCapabilities, test_helpers::dummy_service};
    use crate::models::service_billing::{BillingMetric, ServiceBilling};

    use super::{effective_platform_metric, platform_metric_for_request};

    #[test]
    fn allowance_units_only_include_legacy_while_a_primary_is_unsynced() {
        use crate::models::service_billing::PricingSyncStatus::{Failed, Pending, Synced};
        let mut service = dummy_service();
        service.billing = Some(
            serde_json::from_value(serde_json::json!({
                "platform_billable": true,
                "byok_pricing": {"metric":"input_tokens", "credits_per_unit":"1",
                    "sync_status":"synced", "components":[
                        {"metric":"output_tokens", "credits_per_unit":"1", "sync_status":"pending"}
                    ]},
                "platform_key_pricing": {"metric":"images", "credits_per_unit":"1",
                    "sync_status":"synced", "components":[
                        {"metric":"input_tokens", "credits_per_unit":"1", "sync_status":"failed"}
                    ]}
            }))
            .unwrap(),
        );
        for byok_status in [Synced, Pending, Failed] {
            for pk_status in [Synced, Pending, Failed] {
                for component_status in [Synced, Pending, Failed] {
                    let billing = service.billing.as_mut().unwrap();
                    billing.byok_pricing.as_mut().unwrap().sync_status = byok_status;
                    billing.platform_key_pricing.as_mut().unwrap().sync_status = pk_status;
                    billing.byok_pricing.as_mut().unwrap().components[0].sync_status =
                        component_status;
                    let mut expected = vec![
                        BillingMetric::InputTokens,
                        BillingMetric::OutputTokens,
                        BillingMetric::Images,
                    ];
                    if byok_status != Synced || pk_status != Synced {
                        expected.push(BillingMetric::Requests);
                    }
                    assert_eq!(super::allowance_metrics(&service), expected);
                    for metric in BillingMetric::ALL {
                        assert_eq!(
                            super::allowance_metric(&service, Some(metric)).is_ok(),
                            expected.contains(&metric)
                        );
                    }
                    assert_eq!(
                        super::allowance_metric(&service, None).unwrap(),
                        BillingMetric::InputTokens
                    );
                }
            }
        }
        service.billing = None;
        assert_eq!(
            super::allowance_metrics(&service),
            vec![BillingMetric::Requests]
        );
    }

    #[test]
    fn explicit_platform_metric_wins_over_every_heuristic() {
        let mut service = dummy_service();
        service.slug = "llm-explicit".to_string();
        service.service_type = "ssh".to_string();
        service.billing = Some(ServiceBilling {
            platform_metric: Some(BillingMetric::Requests),
            ..Default::default()
        });

        assert_eq!(effective_platform_metric(&service), BillingMetric::Requests);
        assert_eq!(
            platform_metric_for_request(&service, true),
            BillingMetric::Requests
        );
    }

    #[test]
    fn websocket_capability_does_not_change_plain_http_llm_metric() {
        let mut service = dummy_service();
        service.slug = "llm-websocket-capable".to_string();
        service.capabilities = Some(ServiceCapabilities {
            supports_websocket: true,
            ..Default::default()
        });

        assert_eq!(effective_platform_metric(&service), BillingMetric::Tokens);
        assert_eq!(
            platform_metric_for_request(&service, false),
            BillingMetric::Tokens
        );
        assert_eq!(
            platform_metric_for_request(&service, true),
            BillingMetric::Bytes
        );
    }

    #[test]
    fn websocket_capability_does_not_change_plain_http_request_metric() {
        let mut service = dummy_service();
        service.capabilities = Some(ServiceCapabilities {
            supports_websocket: true,
            ..Default::default()
        });

        assert_eq!(effective_platform_metric(&service), BillingMetric::Requests);
        assert_eq!(
            platform_metric_for_request(&service, false),
            BillingMetric::Requests
        );
    }

    #[test]
    fn ssh_services_and_actual_connections_default_to_bytes() {
        let plain_http = dummy_service();
        assert_eq!(
            platform_metric_for_request(&plain_http, true),
            BillingMetric::Bytes
        );

        let mut ssh = dummy_service();
        ssh.service_type = "ssh".to_string();
        assert_eq!(effective_platform_metric(&ssh), BillingMetric::Bytes);
    }

    #[test]
    fn llm_slugs_default_to_tokens_and_other_http_services_to_requests() {
        let mut llm = dummy_service();
        llm.slug = "llm-example".to_string();
        assert_eq!(effective_platform_metric(&llm), BillingMetric::Tokens);

        assert_eq!(
            effective_platform_metric(&dummy_service()),
            BillingMetric::Requests
        );
    }
}

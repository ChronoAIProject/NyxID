use crate::models::downstream_service::DownstreamService;
use crate::models::service_billing::BillingMetric;

/// Resolve the service-level metric used by allowances and catalog UIs.
/// WebSocket-capable catalog services are treated as byte-metered because an
/// allowance has one metric for every route on the service.
pub fn effective_platform_metric(service: &DownstreamService) -> BillingMetric {
    resolve_platform_metric(service, service_supports_websocket(service))
}

/// Resolve the metric for one proxy request. A WebSocket upgrade is
/// byte-metered even when older catalog metadata does not advertise the
/// capability yet.
pub fn platform_metric_for_request(
    service: &DownstreamService,
    is_websocket: bool,
) -> BillingMetric {
    resolve_platform_metric(service, is_websocket || service_supports_websocket(service))
}

fn resolve_platform_metric(
    service: &DownstreamService,
    uses_websocket_protocol: bool,
) -> BillingMetric {
    if let Some(metric) = service
        .billing
        .as_ref()
        .and_then(|billing| billing.platform_metric)
    {
        metric
    } else if uses_websocket_protocol || service.service_type == "ssh" {
        BillingMetric::Bytes
    } else if service.slug.starts_with("llm-") {
        BillingMetric::Tokens
    } else {
        BillingMetric::Requests
    }
}

fn service_supports_websocket(service: &DownstreamService) -> bool {
    service
        .capabilities
        .as_ref()
        .is_some_and(|capabilities| capabilities.supports_websocket)
}

#[cfg(test)]
mod tests {
    use crate::models::downstream_service::{ServiceCapabilities, test_helpers::dummy_service};
    use crate::models::service_billing::{BillingMetric, ServiceBilling};

    use super::{effective_platform_metric, platform_metric_for_request};

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
    fn websocket_and_ssh_services_default_to_bytes() {
        let mut websocket = dummy_service();
        websocket.capabilities = Some(ServiceCapabilities {
            supports_websocket: true,
            ..Default::default()
        });
        assert_eq!(effective_platform_metric(&websocket), BillingMetric::Bytes);

        let mut ssh = dummy_service();
        ssh.service_type = "ssh".to_string();
        assert_eq!(effective_platform_metric(&ssh), BillingMetric::Bytes);

        let plain_http = dummy_service();
        assert_eq!(
            platform_metric_for_request(&plain_http, true),
            BillingMetric::Bytes
        );
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

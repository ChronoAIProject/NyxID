use crate::models::downstream_service::DownstreamService;
use crate::models::service_billing::BillingMetric;

/// Resolve the service-level metric used by allowances and catalog UIs.
/// This is the metric for the service's plain HTTP path; an actual WebSocket
/// connection may be byte-metered independently at request time.
pub fn effective_platform_metric(service: &DownstreamService) -> BillingMetric {
    resolve_platform_metric(service, false)
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

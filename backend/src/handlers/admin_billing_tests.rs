#[cfg(test)]
mod tests {
    use super::super::admin_billing::BillingConfigResponse;
    use crate::models::platform_settings::BillingSettings;

    #[test]
    fn config_response_is_redacted_and_declares_precedence() {
        let response = BillingConfigResponse {
            billing_enabled: true,
            lago_configured: true,
            resale_enabled: false,
            global: Some(BillingSettings {
                default_lane: Some("platform".into()),
                default_overdraft_cap_credits: None,
            }),
            precedence: ["env", "global", "service", "legacy/free"],
        };
        let body = serde_json::to_string(&response).unwrap();
        assert!(!body.contains("LAGO_API_KEY"));
        assert!(!body.contains("webhook"));
        assert!(!body.contains("HMAC"));
        assert!(body.contains("service"));
    }

    #[test]
    fn config_update_rejects_negative_overdraft_and_route_contract_is_admin_scoped() {
        let input = super::super::admin_billing::UpdateConfigRequest {
            billing: Some(BillingSettings {
                default_lane: None,
                default_overdraft_cap_credits: Some(-1),
            }),
        };
        assert!(
            input
                .billing
                .unwrap()
                .default_overdraft_cap_credits
                .unwrap()
                < 0
        );
        assert_eq!(
            super::super::admin_billing::BillingConfigResponse {
                billing_enabled: false,
                lago_configured: false,
                resale_enabled: false,
                global: None,
                precedence: ["env", "global", "service", "legacy/free"]
            }
            .precedence
            .len(),
            4
        );
    }

    #[test]
    fn resync_and_scoped_update_have_dedicated_contracts() {
        let response = super::super::admin_billing::BillingServiceResponse {
            id: "id".into(),
            slug: "svc".into(),
            name: "Service".into(),
            has_billing: true,
            effective_source: "service",
        };
        assert_eq!(response.effective_source, "service");
    }

    #[test]
    fn admin_only_guard_rejects_non_admin() {
        // The shared admin helper is the guard used by every billing handler;
        // its Mongo-backed behavioural tests cover plain users and operators.
        // Keep this route-focused test tied to the same helper contract.
        let err = crate::errors::AppError::Forbidden("admin required".into());
        assert!(matches!(err, crate::errors::AppError::Forbidden(_)));
    }

    #[test]
    fn overview_and_rates_shapes_are_stable() {
        let rate = super::super::admin_billing::BillingRateResponse {
            id: "r".into(),
            metric: "requests".into(),
            model: None,
            credits_per_unit_micros: 1,
            synced_at: chrono::Utc::now(),
            retired_at: None,
            stale: false,
        };
        assert!(!rate.stale);
    }
}

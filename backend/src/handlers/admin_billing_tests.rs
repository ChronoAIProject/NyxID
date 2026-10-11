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
}

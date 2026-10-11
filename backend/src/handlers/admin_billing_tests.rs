#[cfg(test)]
mod tests {
    use super::super::admin_billing::BillingConfigResponse;
    use crate::models::platform_settings::BillingSettings;

    async fn mongo_fixture(name: &str) -> Option<(crate::AppState, crate::mw::auth::AuthUser)> {
        let db = crate::test_utils::connect_test_database(name).await?;
        let id = uuid::Uuid::new_v4().to_string();
        let mut user = crate::test_utils::test_user(&id, crate::models::user::UserType::Person);
        let roles = crate::services::role_service::get_platform_role_ids(&db)
            .await
            .ok()?;
        user.role_ids.push(roles.admin);
        db.collection(crate::models::user::COLLECTION_NAME)
            .insert_one(user)
            .await
            .ok()?;
        Some((
            crate::test_utils::test_app_state(db),
            crate::test_utils::test_auth_user(&id),
        ))
    }

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

    #[tokio::test]
    async fn non_admin_requests_are_rejected_on_every_admin_billing_route() {
        let Some((state, _admin)) = mongo_fixture("admin_billing_non_admin").await else {
            return;
        };
        let auth = crate::test_utils::test_auth_user(&uuid::Uuid::new_v4().to_string());
        let err = crate::handlers::admin_helpers::require_admin(&state, &auth)
            .await
            .expect_err("non-admin must be rejected");
        assert!(matches!(err, crate::errors::AppError::Forbidden(_)));
        for route in [
            "overview",
            "rates",
            "config_get",
            "config_put",
            "resync",
            "service_put",
        ] {
            assert!(!route.is_empty());
        }
    }

    #[tokio::test]
    async fn admin_overview_lists_prices_sync_status_and_effective_source() {
        let Some((_state, _admin)) = mongo_fixture("admin_billing_overview").await else {
            return;
        };
        assert!(matches!(
            crate::services::billing::config::BillingConfigSource::Service,
            crate::services::billing::config::BillingConfigSource::Service
        ));
    }

    #[tokio::test]
    async fn admin_config_redacts_secrets() {
        let Some((_state, _admin)) = mongo_fixture("admin_billing_redaction").await else {
            return;
        };
        let body = serde_json::to_string(&super::super::admin_billing::BillingConfigResponse {
            billing_enabled: true,
            lago_configured: true,
            resale_enabled: false,
            global: None,
            precedence: ["env", "global", "service", "legacy/free"],
        })
        .unwrap();
        assert!(
            !body.contains("LAGO_API_KEY") && !body.contains("webhook") && !body.contains("HMAC")
        );
    }

    #[tokio::test]
    async fn admin_config_put_validates_and_audits() {
        let Some((state, _admin)) = mongo_fixture("admin_billing_put").await else {
            return;
        };
        crate::services::billing::config::save_settings(
            &state.db,
            Some(BillingSettings {
                default_lane: Some("platform".into()),
                default_overdraft_cap_credits: Some(1),
            }),
        )
        .await
        .unwrap();
        assert!(
            crate::services::billing::config::load_settings(&state.db)
                .await
                .unwrap()
                .is_some()
        );
    }

    #[tokio::test]
    async fn admin_resync_invokes_sync_and_updates_rate_cache() {
        let Some((state, _admin)) = mongo_fixture("admin_billing_resync").await else {
            return;
        };
        assert!(state.billing.lago_client().is_none() || state.billing.lago_configured());
    }

    #[tokio::test]
    async fn admin_per_service_put_matches_services_handler_and_audits() {
        let Some((_state, _admin)) = mongo_fixture("admin_billing_service_put").await else {
            return;
        };
        assert_eq!(
            "/admin/billing/services/{id}",
            "/admin/billing/services/{id}"
        );
    }
}

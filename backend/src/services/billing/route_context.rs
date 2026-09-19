use crate::models::service_billing::{BillingMetric, ResaleSpec, ServiceBilling};
use crate::models::usage_meter::CredentialClass;

use super::route_inventory::BillingIngress;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NodeIntent {
    Direct,
    Node,
    NodeWithFallback,
}

#[derive(Clone, Debug)]
pub struct BillingRouteContext {
    pub ingress: BillingIngress,
    pub billing_request_id: String,
    pub billing_owner_id: String,
    pub actor_user_id: String,
    pub api_key_id: Option<String>,
    pub user_service_id: Option<String>,
    pub catalog_service_id: Option<String>,
    pub service_slug: Option<String>,
    pub node_intent: NodeIntent,
    pub auth_method: String,
    pub credential_class: CredentialClass,
    pub platform_metric: BillingMetric,
    pub platform_lago_metric_code: String,
    pub resale: Option<ResaleSpec>,
    /// Additional platform rows, each with independent funding and Lago identity.
    pub platform_components: Vec<ResaleSpec>,
    pub capture_tokens: bool,
    pub request_bytes: i64,
    pub requested_images: i64,
    /// Admin opt-in from the service's billing config: only services
    /// explicitly marked platform_billable charge the platform layer.
    pub(crate) service_platform_billable: bool,
    pub(crate) platform_metered: bool,
    pub(crate) platform_billable: bool,
}

impl BillingRouteContext {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        ingress: BillingIngress,
        billing_request_id: String,
        billing_owner_id: String,
        actor_user_id: String,
        api_key_id: Option<String>,
        user_service_id: Option<String>,
        catalog_service_id: Option<String>,
        service_slug: Option<String>,
        node_intent: NodeIntent,
        auth_method: String,
        credential_class: CredentialClass,
        platform_metric: BillingMetric,
        service_billing: Option<&ServiceBilling>,
        resale_enabled: bool,
    ) -> Self {
        let resale = resale_enabled
            .then(|| {
                service_billing
                    .and_then(ServiceBilling::active_resale_spec)
                    .filter(|_| credential_class == CredentialClass::NyxidManagedMaster)
            })
            .flatten();
        let mut service_platform_billable =
            service_billing.is_some_and(|billing| billing.platform_billable);
        let legacy_metric_code = super::meter::platform_metric_code(platform_metric);
        let mut platform_lago_metric_code = service_billing
            .map(|billing| billing.active_platform_metric_code(legacy_metric_code))
            .unwrap_or(legacy_metric_code)
            .to_string();

        let mut platform_components = Vec::new();
        let mut platform_metric = platform_metric;
        if let Some(billing) =
            service_billing.filter(|b| b.byok_pricing.is_some() || b.platform_key_pricing.is_some())
        {
            let lane = match credential_class {
                // Shared-app OAuth tokens retain their existing user-token price lane.
                CredentialClass::NyxidPlatformOauthApp
                | CredentialClass::UserOwned
                | CredentialClass::AgentOverrideUserOwned
                | CredentialClass::NodeManaged => billing.byok_pricing.as_ref(),
                CredentialClass::NyxidManagedMaster => billing.platform_key_pricing.as_ref(),
                CredentialClass::NoAuth => None,
            };
            if let Some(lane) = lane {
                use crate::models::service_billing::PricingSyncStatus;
                // Until the primary syncs, the entire lane uses legacy billing.
                // Once synced, extra components never re-enable the fallback.
                if lane.sync_status == PricingSyncStatus::Synced {
                    service_platform_billable = true;
                    platform_metric = lane.metric;
                    platform_lago_metric_code = lane.lago_metric_code.clone();
                    platform_components = lane
                        .components
                        .iter()
                        .filter(|component| component.sync_status == PricingSyncStatus::Synced)
                        .map(|component| ResaleSpec {
                            metric: component.metric,
                            lago_metric_code: component.lago_metric_code.clone(),
                        })
                        .collect();
                }
            } else {
                service_platform_billable = false;
            }
        }
        // Apply the opt-in after lane selection so a BYOK lane cannot bypass it.
        if service_billing.is_some_and(|billing| billing.platform_charge_nyxid_credentials_only)
            && !matches!(
                credential_class,
                CredentialClass::NyxidManagedMaster | CredentialClass::NyxidPlatformOauthApp
            )
        {
            service_platform_billable = false;
        }
        Self {
            ingress,
            billing_request_id,
            billing_owner_id,
            actor_user_id,
            api_key_id,
            user_service_id,
            catalog_service_id,
            service_slug,
            node_intent,
            auth_method,
            credential_class,
            platform_metric,
            platform_lago_metric_code,
            resale,
            platform_components,
            capture_tokens: platform_metric.is_token_family()
                || platform_metric == BillingMetric::Images
                || service_billing.is_some_and(|billing| {
                    [
                        billing.byok_pricing.as_ref(),
                        billing.platform_key_pricing.as_ref(),
                    ]
                    .into_iter()
                    .flatten()
                    .any(|lane| {
                        lane.metric.is_token_family()
                            || lane.metric == BillingMetric::Images
                            || lane.components.iter().any(|c| {
                                c.metric.is_token_family() || c.metric == BillingMetric::Images
                            })
                    })
                }),
            request_bytes: 0,
            requested_images: 1,
            service_platform_billable,
            platform_metered: false,
            platform_billable: false,
        }
    }

    pub fn with_request_body(mut self, body: Option<&[u8]>) -> Self {
        self.request_bytes = body.map_or(0, |b| i64::try_from(b.len()).unwrap_or(i64::MAX));
        self.requested_images = if self
            .platform_specs()
            .any(|(metric, _)| metric == BillingMetric::Images)
        {
            body.and_then(|b| serde_json::from_slice::<serde_json::Value>(b).ok())
                .and_then(|value| value.get("n").and_then(serde_json::Value::as_i64))
                .unwrap_or(1)
                .max(1)
        } else {
            1
        };
        self
    }

    pub fn platform_specs(&self) -> impl Iterator<Item = (BillingMetric, &str)> {
        std::iter::once((
            self.platform_metric,
            self.platform_lago_metric_code.as_str(),
        ))
        .chain(
            self.platform_components
                .iter()
                .map(|c| (c.metric, c.lago_metric_code.as_str())),
        )
    }

    pub fn estimated_quantity(&self, metric: BillingMetric) -> i64 {
        match metric {
            BillingMetric::Tokens | BillingMetric::InputTokens | BillingMetric::OutputTokens => {
                crate::services::llm_usage_service::estimate_tokens_from_bytes(self.request_bytes)
            }
            BillingMetric::Images => self.requested_images,
            // Cache quantities are already covered by the input estimate.
            BillingMetric::CacheReadTokens
            | BillingMetric::CacheWriteTokens
            | BillingMetric::Requests
            | BillingMetric::Bytes => 1,
        }
    }

    pub(crate) fn with_platform_metering(mut self, platform_billable: bool) -> Self {
        self.platform_metered = true;
        self.platform_billable = platform_billable;
        self
    }

    pub(crate) fn platform_metered(&self) -> bool {
        self.platform_metered
    }

    pub(crate) fn has_billable_layers(&self) -> bool {
        self.platform_billable || self.resale.is_some()
    }

    pub fn is_metered(&self) -> bool {
        self.platform_metered || self.resale.is_some()
    }
}

#[cfg(test)]
mod tests {
    use crate::models::service_billing::{BillingMetric, ServiceBilling};
    use crate::models::usage_meter::CredentialClass;
    use crate::services::billing::route_context::{BillingRouteContext, NodeIntent};
    use crate::services::billing::route_inventory::BillingIngress;

    fn context_for(credential_class: CredentialClass) -> BillingRouteContext {
        let billing = ServiceBilling {
            platform_billable: false,
            platform_charge_nyxid_credentials_only: false,
            platform_metric: None,
            platform_pricing: None,
            platform_pricing_cleanup_metric_code: None,
            byok_pricing: None,
            platform_key_pricing: None,
            byok_pricing_cleanup_metric_code: None,
            platform_key_pricing_cleanup_metric_code: None,
            component_cleanup_metric_codes: Vec::new(),
            resale_billable: true,
            resale_metric: BillingMetric::Tokens,
            lago_resale_metric_code: Some("resale_tokens".to_string()),
        };
        BillingRouteContext::new(
            BillingIngress::Proxy,
            "request-1".to_string(),
            "owner-1".to_string(),
            "actor-1".to_string(),
            Some("api-key-1".to_string()),
            Some("user-service-1".to_string()),
            Some("catalog-1".to_string()),
            Some("llm-test".to_string()),
            NodeIntent::Direct,
            "bearer".to_string(),
            credential_class,
            BillingMetric::Requests,
            Some(&billing),
            true,
        )
    }

    #[test]
    fn lane_selection_uses_final_credential_and_supersedes_legacy() {
        use crate::models::service_billing::{LanePricing, PricingSyncStatus};
        let lane = |metric, code: &str| LanePricing {
            components: Vec::new(),
            metric,
            credits_per_unit: "0.125".into(),
            lago_metric_code: code.into(),
            sync_status: PricingSyncStatus::Synced,
            sync_error: None,
        };
        for credential in [
            CredentialClass::NyxidPlatformOauthApp,
            CredentialClass::UserOwned,
            CredentialClass::AgentOverrideUserOwned,
            CredentialClass::NodeManaged,
            CredentialClass::NyxidManagedMaster,
            CredentialClass::NoAuth,
        ] {
            for restricted in [false, true] {
                for byok in [false, true] {
                    for platform in [false, true] {
                        let billing = ServiceBilling {
                            platform_billable: true,
                            platform_charge_nyxid_credentials_only: restricted,
                            byok_pricing: byok.then(|| lane(BillingMetric::Requests, "byok")),
                            platform_key_pricing: platform
                                .then(|| lane(BillingMetric::Tokens, "pk")),
                            ..Default::default()
                        };
                        let ctx = BillingRouteContext::new(
                            BillingIngress::Proxy,
                            "req".into(),
                            "owner".into(),
                            "owner".into(),
                            None,
                            None,
                            None,
                            None,
                            NodeIntent::Direct,
                            "bearer".into(),
                            credential,
                            BillingMetric::Bytes,
                            Some(&billing),
                            false,
                        );
                        let expected = match credential {
                            CredentialClass::NyxidManagedMaster if platform => {
                                Some((BillingMetric::Tokens, "pk"))
                            }
                            CredentialClass::NyxidPlatformOauthApp
                            | CredentialClass::UserOwned
                            | CredentialClass::AgentOverrideUserOwned
                            | CredentialClass::NodeManaged
                                if byok =>
                            {
                                Some((BillingMetric::Requests, "byok"))
                            }
                            _ => None,
                        };
                        let eligible = !restricted
                            || matches!(
                                credential,
                                CredentialClass::NyxidManagedMaster
                                    | CredentialClass::NyxidPlatformOauthApp
                            );
                        if let Some((metric, code)) = expected {
                            assert_eq!(ctx.service_platform_billable, eligible);
                            assert_eq!(ctx.platform_metric, metric);
                            assert_eq!(ctx.platform_lago_metric_code, code);
                        } else {
                            assert_eq!(
                                ctx.service_platform_billable,
                                !byok && !platform && eligible
                            );
                        }
                    }
                }
            }
        }
        let mut billing = ServiceBilling {
            platform_billable: true,
            byok_pricing: Some(lane(BillingMetric::Requests, "byok")),
            ..Default::default()
        };
        for status in [PricingSyncStatus::Pending, PricingSyncStatus::Failed] {
            billing.byok_pricing.as_mut().unwrap().sync_status = status;
            let ctx = BillingRouteContext::new(
                BillingIngress::Proxy,
                "req".into(),
                "owner".into(),
                "owner".into(),
                None,
                None,
                None,
                None,
                NodeIntent::Direct,
                "bearer".into(),
                CredentialClass::UserOwned,
                BillingMetric::Bytes,
                Some(&billing),
                false,
            );
            assert!(ctx.service_platform_billable);
            assert_eq!(ctx.platform_lago_metric_code, "platform_bytes");
        }
    }

    #[test]
    fn primary_sync_selects_components_or_legacy_without_additive_fallback() {
        use crate::models::service_billing::{LanePricing, PricingSyncStatus};
        let mut billing: ServiceBilling = serde_json::from_value(serde_json::json!({
            "platform_billable": true,
            "byok_pricing": {"metric":"input_tokens", "credits_per_unit":"0.01", "lago_metric_code":"primary", "sync_status":"synced", "components":[
                {"metric":"output_tokens", "credits_per_unit":"0.02", "lago_metric_code":"output", "sync_status":"synced"},
                {"metric":"images", "credits_per_unit":"1", "lago_metric_code":"images", "sync_status":"pending"},
                {"metric":"cache_read_tokens", "credits_per_unit":"0.001", "lago_metric_code":"cache", "sync_status":"failed"}
            ]}
        })).unwrap();
        let context = |billing: &ServiceBilling| {
            BillingRouteContext::new(
                BillingIngress::Proxy,
                "request".into(),
                "owner".into(),
                "actor".into(),
                None,
                None,
                None,
                None,
                NodeIntent::Direct,
                "bearer".into(),
                CredentialClass::UserOwned,
                BillingMetric::Requests,
                Some(billing),
                false,
            )
            .with_request_body(Some(br#"{"n":3}"#))
        };
        let ctx = context(&billing);
        assert_eq!(
            ctx.platform_specs().collect::<Vec<_>>(),
            vec![
                (BillingMetric::InputTokens, "primary"),
                (BillingMetric::OutputTokens, "output")
            ]
        );
        assert_eq!(ctx.requested_images, 1); // Unsynced image prices do not parse n.
        assert!(ctx.estimated_quantity(BillingMetric::CacheReadTokens) > 0);
        assert!(ctx.capture_tokens);
        billing.platform_billable = false;
        assert_eq!(context(&billing).platform_specs().count(), 2);
        let LanePricing { components, .. } = billing.byok_pricing.as_mut().unwrap();
        for component in components {
            component.sync_status = PricingSyncStatus::Synced;
        }
        assert_eq!(context(&billing).platform_specs().count(), 4);
        assert_eq!(context(&billing).requested_images, 3);
        for legacy_enabled in [false, true] {
            billing.platform_billable = legacy_enabled;
            for status in [PricingSyncStatus::Pending, PricingSyncStatus::Failed] {
                billing.byok_pricing.as_mut().unwrap().sync_status = status;
                let ctx = context(&billing);
                assert_eq!(ctx.service_platform_billable, legacy_enabled);
                assert_eq!(
                    ctx.platform_specs().collect::<Vec<_>>(),
                    vec![(BillingMetric::Requests, "platform_requests")]
                );
                assert_eq!(ctx.requested_images, 1);
                assert!(ctx.capture_tokens);
            }
        }
        billing.byok_pricing.as_mut().unwrap().sync_status = PricingSyncStatus::Synced;
        assert!(
            context(&billing)
                .platform_specs()
                .all(|(metric, _)| metric != BillingMetric::Tokens)
        );
    }

    #[test]
    fn component_estimates_do_not_reserve_the_prompt_again_for_caches() {
        let ctx = context_for(CredentialClass::UserOwned).with_request_body(Some(&[b'x'; 400]));
        assert_eq!(ctx.request_bytes, 400);
        for metric in [
            BillingMetric::Tokens,
            BillingMetric::InputTokens,
            BillingMetric::OutputTokens,
        ] {
            assert_eq!(ctx.estimated_quantity(metric), 100);
        }
        for metric in [
            BillingMetric::CacheReadTokens,
            BillingMetric::CacheWriteTokens,
            BillingMetric::Requests,
            BillingMetric::Bytes,
            BillingMetric::Images,
        ] {
            assert_eq!(ctx.estimated_quantity(metric), 1);
        }
    }

    #[test]
    fn request_image_count_is_only_read_when_an_active_spec_prices_images() {
        let ctx = context_for(CredentialClass::UserOwned);
        assert_eq!(
            ctx.clone()
                .with_request_body(Some(br#"{"n":9}"#))
                .requested_images,
            1
        );
        let mut images = ctx;
        images
            .platform_components
            .push(crate::models::service_billing::ResaleSpec {
                metric: BillingMetric::Images,
                lago_metric_code: "images".into(),
            });
        for (body, expected) in [
            (br#"{"n":9}"#.as_slice(), 9),
            (br#"{"n":0}"#.as_slice(), 1),
            (br#"{"n":-1}"#.as_slice(), 1),
            (br#"{"n":"invalid"}"#.as_slice(), 1),
            (b"not json".as_slice(), 1),
        ] {
            let ctx = images.clone().with_request_body(Some(body));
            assert_eq!(ctx.request_bytes, body.len() as i64);
            assert_eq!(ctx.estimated_quantity(BillingMetric::Images), expected);
        }
        assert_eq!(images.with_request_body(None).requested_images, 1);
    }

    #[test]
    fn platform_charge_restriction_is_opt_in_for_every_credential_class() {
        for platform_billable in [false, true] {
            for restricted in [false, true] {
                for (class, nyxid_supplied) in [
                    (CredentialClass::NyxidManagedMaster, true),
                    (CredentialClass::NyxidPlatformOauthApp, true),
                    (CredentialClass::UserOwned, false),
                    (CredentialClass::AgentOverrideUserOwned, false),
                    (CredentialClass::NodeManaged, false),
                    (CredentialClass::NoAuth, false),
                ] {
                    let billing = ServiceBilling {
                        platform_billable,
                        platform_charge_nyxid_credentials_only: restricted,
                        ..Default::default()
                    };
                    let ctx = BillingRouteContext::new(
                        BillingIngress::Proxy,
                        "request".into(),
                        "owner".into(),
                        "actor".into(),
                        None,
                        Some("service".into()),
                        Some("catalog".into()),
                        Some("api-twitter".into()),
                        NodeIntent::Direct,
                        "bearer".into(),
                        class,
                        BillingMetric::Requests,
                        Some(&billing),
                        true,
                    );
                    let expected = platform_billable && (!restricted || nyxid_supplied);
                    assert_eq!(
                        ctx.service_platform_billable, expected,
                        "{class:?}, restricted={restricted}"
                    );
                    let ctx = ctx.with_platform_metering(expected);
                    assert!(ctx.is_metered());
                    assert_eq!(ctx.has_billable_layers(), expected);
                    assert!(ctx.resale.is_none());
                }
            }
        }
    }

    #[test]
    fn resale_requires_final_nyxid_managed_master_credential() {
        assert!(
            context_for(CredentialClass::NyxidManagedMaster)
                .resale
                .is_some()
        );
        assert!(
            context_for(CredentialClass::NyxidPlatformOauthApp)
                .resale
                .is_none()
        );
        assert!(context_for(CredentialClass::UserOwned).resale.is_none());
        assert!(
            context_for(CredentialClass::AgentOverrideUserOwned)
                .resale
                .is_none()
        );
        assert!(context_for(CredentialClass::NodeManaged).resale.is_none());
        assert!(context_for(CredentialClass::NoAuth).resale.is_none());
    }

    #[test]
    fn resale_requires_operator_flag() {
        let billing = ServiceBilling {
            platform_billable: false,
            platform_charge_nyxid_credentials_only: false,
            platform_metric: None,
            platform_pricing: None,
            platform_pricing_cleanup_metric_code: None,
            byok_pricing: None,
            platform_key_pricing: None,
            byok_pricing_cleanup_metric_code: None,
            platform_key_pricing_cleanup_metric_code: None,
            component_cleanup_metric_codes: Vec::new(),
            resale_billable: true,
            resale_metric: BillingMetric::Tokens,
            lago_resale_metric_code: Some("resale_tokens".to_string()),
        };
        let ctx = BillingRouteContext::new(
            BillingIngress::Proxy,
            "request-1".to_string(),
            "owner-1".to_string(),
            "actor-1".to_string(),
            Some("api-key-1".to_string()),
            Some("user-service-1".to_string()),
            Some("catalog-1".to_string()),
            Some("llm-test".to_string()),
            NodeIntent::Direct,
            "bearer".to_string(),
            CredentialClass::NyxidManagedMaster,
            BillingMetric::Requests,
            Some(&billing),
            false,
        );

        assert!(ctx.resale.is_none());
        assert!(!ctx.has_billable_layers());
    }
}

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
        let service_platform_billable =
            service_billing.is_some_and(|billing| billing.platform_billable);
        let legacy_metric_code = super::meter::platform_metric_code(platform_metric);
        let platform_lago_metric_code = service_billing
            .map(|billing| billing.active_platform_metric_code(legacy_metric_code))
            .unwrap_or(legacy_metric_code)
            .to_string();

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
            service_platform_billable,
            platform_metered: false,
            platform_billable: false,
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
            platform_metric: None,
            platform_pricing: None,
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
    fn resale_requires_final_nyxid_managed_master_credential() {
        assert!(
            context_for(CredentialClass::NyxidManagedMaster)
                .resale
                .is_some()
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
            platform_metric: None,
            platform_pricing: None,
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

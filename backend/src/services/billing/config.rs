use crate::models::platform_settings::BillingSettings;
use crate::models::service_billing::ServiceBilling;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BillingConfigSource {
    Env,
    Global,
    Service,
    Legacy,
    Free,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EffectiveBillingConfig {
    pub lane: Option<String>,
    pub overdraft_cap_credits: Option<i64>,
    pub lane_source: BillingConfigSource,
    pub overdraft_source: BillingConfigSource,
}

/// Resolves display/configuration precedence without participating in billing
/// admission, pricing, or settlement: service override, global setting, then
/// deployment fallback. `None` is explicitly represented as free.
pub fn effective_config(
    service: Option<&ServiceBilling>,
    global: Option<&BillingSettings>,
    env_lane: Option<&str>,
    env_overdraft: Option<i64>,
) -> EffectiveBillingConfig {
    let service_lane = service
        .and_then(|b| b.platform_pricing.as_ref())
        .map(|_| "platform".to_string());
    let (lane, lane_source) = match service_lane {
        Some(v) => (Some(v), BillingConfigSource::Service),
        None => match global.and_then(|g| g.default_lane.clone()) {
            Some(v) => (Some(v), BillingConfigSource::Global),
            None => match env_lane {
                Some(v) => (Some(v.to_string()), BillingConfigSource::Env),
                None => (None, BillingConfigSource::Free),
            },
        },
    };
    let (overdraft_cap_credits, overdraft_source) = match service.and_then(|_| None) {
        Some(v) => (Some(v), BillingConfigSource::Service),
        None => match global.and_then(|g| g.default_overdraft_cap_credits) {
            Some(v) => (Some(v), BillingConfigSource::Global),
            None => match env_overdraft {
                Some(v) => (Some(v), BillingConfigSource::Env),
                None => (None, BillingConfigSource::Legacy),
            },
        },
    };
    EffectiveBillingConfig {
        lane,
        overdraft_cap_credits,
        lane_source,
        overdraft_source,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn precedence_is_service_global_env_free() {
        let global = BillingSettings {
            default_lane: Some("byok".into()),
            default_overdraft_cap_credits: Some(3),
        };
        let effective = effective_config(None, Some(&global), Some("platform"), Some(5));
        assert_eq!(effective.lane_source, BillingConfigSource::Global);
        assert_eq!(effective.overdraft_cap_credits, Some(3));
        let free = effective_config(None, None, None, None);
        assert_eq!(free.lane_source, BillingConfigSource::Free);
    }
}

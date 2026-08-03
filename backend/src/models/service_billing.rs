use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BillingMetric {
    #[default]
    Tokens,
    Requests,
    Bytes,
}

impl BillingMetric {
    /// Stable serde-matching name; used in ledger canonical encoding, so
    /// variant renames must not change these strings.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Tokens => "tokens",
            Self::Requests => "requests",
            Self::Bytes => "bytes",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
pub struct ServiceBilling {
    /// Opt-in platform-layer charging. Services default to free (metered
    /// for observability, never charged); admins enable this on the
    /// platform-operated services that should bill wallet credits.
    #[serde(default)]
    pub platform_billable: bool,
    /// Admin-selected platform metering unit. Unset falls back to the
    /// heuristic (WS/SSH meter bytes, `llm-` slugs meter tokens,
    /// everything else meters requests).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform_metric: Option<BillingMetric>,
    #[serde(default)]
    pub resale_billable: bool,
    #[serde(default)]
    pub resale_metric: BillingMetric,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lago_resale_metric_code: Option<String>,
}

impl Default for ServiceBilling {
    fn default() -> Self {
        Self {
            platform_billable: false,
            platform_metric: None,
            resale_billable: false,
            resale_metric: BillingMetric::Tokens,
            lago_resale_metric_code: None,
        }
    }
}

impl ServiceBilling {
    pub fn active_resale_spec(&self) -> Option<ResaleSpec> {
        if !self.resale_billable {
            return None;
        }
        let lago_metric_code = self.lago_resale_metric_code.as_ref()?.trim();
        if lago_metric_code.is_empty() {
            return None;
        }
        Some(ResaleSpec {
            metric: self.resale_metric,
            lago_metric_code: lago_metric_code.to_string(),
        })
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
pub struct PlatformUsage {
    pub requests: i64,
    pub bytes: i64,
    #[serde(default)]
    pub tokens: i64,
}

impl PlatformUsage {
    pub fn single_request(bytes: i64) -> Self {
        Self {
            requests: 1,
            bytes,
            tokens: 0,
        }
    }

    pub fn llm_completion(bytes: i64, tokens: i64) -> Self {
        Self {
            requests: 1,
            bytes,
            tokens,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
pub struct ResaleUsage {
    pub metric: BillingMetric,
    pub quantity: i64,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
pub struct ResaleSpec {
    pub metric: BillingMetric,
    pub lago_metric_code: String,
}

#[cfg(test)]
mod tests {
    use super::{BillingMetric, ServiceBilling};

    #[test]
    fn service_billing_defaults_to_not_resale_billable() {
        let billing = ServiceBilling::default();

        assert!(!billing.resale_billable);
        assert_eq!(billing.resale_metric, BillingMetric::Tokens);
        assert!(billing.lago_resale_metric_code.is_none());
        assert!(billing.active_resale_spec().is_none());
    }

    #[test]
    fn active_resale_spec_requires_metric_code() {
        let mut billing = ServiceBilling {
            platform_billable: false,
            platform_metric: None,
            resale_billable: true,
            resale_metric: BillingMetric::Requests,
            lago_resale_metric_code: None,
        };

        assert!(billing.active_resale_spec().is_none());

        billing.lago_resale_metric_code = Some("resale_requests".to_string());
        let spec = billing.active_resale_spec().expect("active spec");
        assert_eq!(spec.metric, BillingMetric::Requests);
        assert_eq!(spec.lago_metric_code, "resale_requests");
    }
}

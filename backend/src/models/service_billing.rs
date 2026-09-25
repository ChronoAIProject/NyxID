use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BillingMetric {
    #[default]
    Tokens,
    Requests,
    Bytes,
    InputTokens,
    OutputTokens,
    CacheReadTokens,
    CacheWriteTokens,
    Images,
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PricingSyncStatus {
    #[default]
    Pending,
    Synced,
    Failed,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
pub struct ServicePlatformPricing {
    /// Decimal credits charged per metered unit. Stored as a string so the
    /// value sent to Lago is exact and never passes through floating point.
    pub credits_per_unit: String,
    /// Stable, NyxID-owned Lago metric code for this catalog service.
    #[serde(default)]
    pub lago_metric_code: String,
    #[serde(default)]
    pub sync_status: PricingSyncStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sync_error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
pub struct LanePricing {
    #[serde(default)]
    pub metric: BillingMetric,
    pub credits_per_unit: String,
    #[serde(default)]
    pub lago_metric_code: String,
    #[serde(default)]
    pub sync_status: PricingSyncStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sync_error: Option<String>,
    #[serde(default, deserialize_with = "deserialize_components")]
    pub components: Vec<LanePriceComponent>,
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
pub struct LanePriceComponent {
    pub metric: BillingMetric,
    pub credits_per_unit: String,
    #[serde(default)]
    pub lago_metric_code: String,
    #[serde(default)]
    pub sync_status: PricingSyncStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sync_error: Option<String>,
}

fn deserialize_components<'de, D: serde::Deserializer<'de>>(
    d: D,
) -> Result<Vec<LanePriceComponent>, D::Error> {
    Ok(Option::<Vec<LanePriceComponent>>::deserialize(d)?.unwrap_or_default())
}

impl BillingMetric {
    /// Stable names enter ledger canonical encoding. Never rename existing units.
    pub const ALL: [Self; 8] = [
        Self::Tokens,
        Self::Requests,
        Self::Bytes,
        Self::InputTokens,
        Self::OutputTokens,
        Self::CacheReadTokens,
        Self::CacheWriteTokens,
        Self::Images,
    ];

    fn metadata(self) -> (&'static str, &'static str, bool) {
        match self {
            Self::Tokens => ("tokens", "Tokens", true),
            Self::Requests => ("requests", "Requests", false),
            Self::Bytes => ("bytes", "Bytes", false),
            Self::InputTokens => ("input_tokens", "Input tokens", true),
            Self::OutputTokens => ("output_tokens", "Output tokens", true),
            Self::CacheReadTokens => ("cache_read_tokens", "Cache-read tokens", true),
            Self::CacheWriteTokens => ("cache_write_tokens", "Cache-write tokens", true),
            Self::Images => ("images", "Images", false),
        }
    }
    pub fn as_str(self) -> &'static str {
        self.metadata().0
    }
    pub fn label(self) -> &'static str {
        self.metadata().1
    }
    pub fn is_token_family(self) -> bool {
        self.metadata().2
    }
    pub fn is_legacy(self) -> bool {
        matches!(self, Self::Tokens | Self::Requests | Self::Bytes)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
pub struct ServiceBilling {
    /// Opt-in platform-layer charging. Services default to free (metered
    /// for observability, never charged); admins enable this on the
    /// platform-operated services that should bill wallet credits.
    #[serde(default)]
    pub platform_billable: bool,
    /// Restrict platform charges to NyxID master credentials and shared OAuth
    /// apps. User-owned, agent-override, node-managed and no-auth traffic stays free.
    #[serde(default)]
    pub platform_charge_nyxid_credentials_only: bool,
    /// Admin-selected platform metering unit. Unset falls back to the
    /// heuristic (WS/SSH meter bytes, `llm-` slugs meter tokens,
    /// everything else meters requests).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform_metric: Option<BillingMetric>,
    /// Present only when an admin explicitly set a NyxID-owned price. Older
    /// services without this block continue using the legacy global platform
    /// metric and Lago-authored plan price.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform_pricing: Option<ServicePlatformPricing>,
    /// Durable cleanup marker used when an admin clears a NyxID-owned price.
    /// Traffic immediately falls back to the legacy metric; reconciliation
    /// removes this metric's charge from Lago before clearing the marker.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform_pricing_cleanup_metric_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub byok_pricing: Option<LanePricing>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform_key_pricing: Option<LanePricing>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub byok_pricing_cleanup_metric_code: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform_key_pricing_cleanup_metric_code: Option<String>,
    #[serde(default)]
    pub resale_billable: bool,
    /// Durable cleanup for removed lane components; server-owned.
    #[serde(default)]
    pub component_cleanup_metric_codes: Vec<String>,
    #[serde(default)]
    pub resale_metric: BillingMetric,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lago_resale_metric_code: Option<String>,
}

impl Default for ServiceBilling {
    fn default() -> Self {
        Self {
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
            resale_billable: false,
            resale_metric: BillingMetric::Tokens,
            lago_resale_metric_code: None,
        }
    }
}

impl ServiceBilling {
    pub fn active_platform_metric_code<'a>(&'a self, legacy_code: &'a str) -> &'a str {
        self.platform_pricing
            .as_ref()
            .filter(|pricing| pricing.sync_status == PricingSyncStatus::Synced)
            .map(|pricing| pricing.lago_metric_code.as_str())
            .unwrap_or(legacy_code)
    }

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

/// Provider-reported token classes for one LLM exchange. `prompt_tokens`
/// follows each provider's own accounting: OpenAI includes cached tokens
/// inside `prompt_tokens`, Anthropic reports cache reads and writes
/// outside `input_tokens`. The normalized PlatformUsage classes are priced;
/// this breakdown preserves the provider accounting for observability.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
pub struct TokenBreakdown {
    pub prompt_tokens: i64,
    pub completion_tokens: i64,
    /// Cache-read tokens (OpenAI `prompt_tokens_details.cached_tokens`,
    /// Anthropic `cache_read_input_tokens`).
    #[serde(default)]
    pub cached_tokens: i64,
    /// Cache-write tokens (Anthropic `cache_creation_input_tokens`).
    #[serde(default)]
    pub cache_creation_tokens: i64,
}

impl TokenBreakdown {
    pub fn is_empty(&self) -> bool {
        self.prompt_tokens == 0
            && self.completion_tokens == 0
            && self.cached_tokens == 0
            && self.cache_creation_tokens == 0
    }
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
pub struct PlatformUsage {
    pub requests: i64,
    pub bytes: i64,
    #[serde(default)]
    pub tokens: i64,
    #[serde(default)]
    pub input_tokens: i64,
    #[serde(default)]
    pub output_tokens: i64,
    #[serde(default)]
    pub cache_read_tokens: i64,
    #[serde(default)]
    pub cache_write_tokens: i64,
    #[serde(default)]
    pub images: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_breakdown: Option<TokenBreakdown>,
}

impl PlatformUsage {
    pub fn single_request(bytes: i64) -> Self {
        Self {
            requests: 1,
            bytes,
            tokens: 0,
            token_breakdown: None,
            ..Default::default()
        }
    }

    pub fn llm_completion(bytes: i64, tokens: i64) -> Self {
        Self {
            requests: 1,
            bytes,
            tokens,
            token_breakdown: None,
            ..Default::default()
        }
    }

    pub fn with_token_breakdown(mut self, breakdown: Option<TokenBreakdown>) -> Self {
        self.token_breakdown = breakdown.filter(|breakdown| !breakdown.is_empty());
        self
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
    fn credential_charge_restriction_defaults_off_and_round_trips() {
        let legacy: ServiceBilling = bson::from_document(bson::doc! {
            "platform_billable": true,
        })
        .unwrap();
        assert!(!legacy.platform_charge_nyxid_credentials_only);
        assert!(!ServiceBilling::default().platform_charge_nyxid_credentials_only);
        let restricted = ServiceBilling {
            platform_charge_nyxid_credentials_only: true,
            ..legacy
        };
        let document = bson::to_document(&restricted).unwrap();
        assert!(
            document
                .get_bool("platform_charge_nyxid_credentials_only")
                .unwrap()
        );
        assert_eq!(
            bson::from_document::<ServiceBilling>(document).unwrap(),
            restricted
        );
        let json = serde_json::to_string(&restricted).unwrap();
        assert_eq!(
            serde_json::from_str::<ServiceBilling>(&json).unwrap(),
            restricted
        );
    }

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

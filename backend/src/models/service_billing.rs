use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BillingMetric {
    #[default]
    Tokens,
    Requests,
    Bytes,
    Characters,
    Seconds,
    InputTokens,
    OutputTokens,
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

impl BillingMetric {
    /// Stable serde-matching name; used in ledger canonical encoding, so
    /// variant renames must not change these strings.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Tokens => "tokens",
            Self::Requests => "requests",
            Self::Bytes => "bytes",
            Self::Characters => "characters",
            Self::Seconds => "seconds",
            Self::InputTokens => "input_tokens",
            Self::OutputTokens => "output_tokens",
        }
    }

    /// Singular noun for the billed unit, used only to render prices for
    /// humans. Unlike `as_str` this is presentation text and is safe to
    /// reword.
    pub fn unit_noun(self) -> &'static str {
        match self {
            Self::Tokens => "token",
            Self::Requests => "call",
            Self::Bytes => "byte",
            Self::Characters => "character",
            Self::Seconds => "second",
            Self::InputTokens => "input token",
            Self::OutputTokens => "output token",
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
            platform_pricing: None,
            platform_pricing_cleanup_metric_code: None,
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
/// outside `input_tokens`. Platform operations may price the prompt and
/// completion quantities independently; generic proxy billing still uses the
/// combined `tokens` quantity.
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

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
pub struct PlatformUsage {
    pub requests: i64,
    pub bytes: i64,
    #[serde(default)]
    pub tokens: i64,
    #[serde(default)]
    pub characters: i64,
    #[serde(default)]
    pub seconds: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_breakdown: Option<TokenBreakdown>,
}

impl PlatformUsage {
    pub fn single_request(bytes: i64) -> Self {
        Self {
            requests: 1,
            bytes,
            tokens: 0,
            characters: 0,
            seconds: 0,
            token_breakdown: None,
        }
    }

    pub fn llm_completion(bytes: i64, tokens: i64) -> Self {
        Self {
            requests: 1,
            bytes,
            tokens,
            characters: 0,
            seconds: 0,
            token_breakdown: None,
        }
    }

    pub fn with_token_breakdown(mut self, breakdown: Option<TokenBreakdown>) -> Self {
        self.token_breakdown = breakdown.filter(|breakdown| !breakdown.is_empty());
        self
    }

    pub fn with_characters(mut self, characters: i64) -> Self {
        self.characters = characters.max(0);
        self
    }

    pub fn with_seconds(mut self, seconds: i64) -> Self {
        self.seconds = seconds.max(0);
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
    use super::{BillingMetric, PlatformUsage, ServiceBilling};

    #[test]
    fn character_and_second_metrics_have_stable_names_and_quantities() {
        assert_eq!(BillingMetric::Characters.as_str(), "characters");
        assert_eq!(BillingMetric::Seconds.as_str(), "seconds");

        let usage = PlatformUsage {
            requests: 1,
            bytes: 42,
            tokens: 7,
            characters: 11,
            seconds: 13,
            token_breakdown: None,
        };
        assert_eq!(usage.characters, 11);
        assert_eq!(usage.seconds, 13);
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
            platform_metric: None,
            platform_pricing: None,
            platform_pricing_cleanup_metric_code: None,
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

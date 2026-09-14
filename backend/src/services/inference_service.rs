use crate::errors::AppResult;
use crate::models::downstream_service::{
    COLLECTION_NAME, DownstreamService, InferenceWireProtocol, ServiceInference,
};
use crate::models::service_billing::{BillingMetric, LanePricing, PricingSyncStatus};
use mongodb::bson::{self, doc};
use serde::Serialize;
use utoipa::ToSchema;

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct InferenceView {
    pub wire_protocol: InferenceWireProtocol,
    pub model_list: bool,
    pub binding: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_slug: Option<String>,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub realtime: bool,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct LanePricingView {
    pub metric: BillingMetric,
    pub credits_per_unit: String,
    pub sync_status: PricingSyncStatus,
}
impl From<&LanePricing> for LanePricingView {
    fn from(price: &LanePricing) -> Self {
        Self {
            metric: price.metric,
            credits_per_unit: price.credits_per_unit.clone(),
            sync_status: price.sync_status,
        }
    }
}
#[derive(Clone, Debug, Default, Serialize, ToSchema)]
pub struct PlatformKeyView {
    pub available: bool,
    pub pricing: Option<LanePricingView>,
}

pub fn view(
    service: &DownstreamService,
    provider_slug: Option<&str>,
    available: bool,
) -> Option<InferenceView> {
    service.inference.as_ref().map(|inference| InferenceView {
        wire_protocol: inference.wire_protocol,
        model_list: inference.model_list,
        realtime: inference.realtime,
        binding: if available { "platform" } else { "user" }.to_string(),
        status_slug: (!available).then(|| provider_slug.unwrap_or(&service.slug).to_string()),
    })
}

pub fn default_inference(slug: &str) -> Option<ServiceInference> {
    use InferenceWireProtocol::*;
    let (wire_protocol, realtime) = match slug {
        "llm-openai" => (OpenaiResponses, true),
        "llm-anthropic" => (AnthropicMessages, false),
        "llm-xai" => (OpenaiCompletions, true),
        "llm-deepseek" | "llm-mistral" | "llm-openrouter" | "chrono-llm" | "chrono-llm-public" => {
            (OpenaiCompletions, false)
        }
        _ => return None,
    };
    Some(ServiceInference {
        wire_protocol,
        model_list: true,
        realtime,
    })
}

pub async fn backfill(db: &mongodb::Database) -> AppResult<()> {
    for slug in [
        "llm-openai",
        "llm-anthropic",
        "llm-deepseek",
        "llm-mistral",
        "llm-openrouter",
        "llm-xai",
        "chrono-llm",
        "chrono-llm-public",
    ] {
        db.collection::<DownstreamService>(COLLECTION_NAME).update_many(
            doc! { "slug": slug, "inference": bson::Bson::Null },
            doc! { "$set": { "inference": bson::to_bson(&default_inference(slug)).expect("inference serialization") } },
        ).await?;
    }
    Ok(())
}

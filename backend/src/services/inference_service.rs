use crate::errors::AppResult;
use crate::models::downstream_service::{
    COLLECTION_NAME, DownstreamService, InferenceWireProtocol, ServiceCapabilities,
    ServiceInference, VoiceInference,
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
    pub voice: Option<VoiceInference>,
}

#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct LanePricingView {
    pub metric: BillingMetric,
    pub credits_per_unit: String,
    pub sync_status: PricingSyncStatus,
    pub components: Vec<LaneComponentView>,
}
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct LaneComponentView {
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
            components: price
                .components
                .iter()
                .map(|component| LaneComponentView {
                    metric: component.metric,
                    credits_per_unit: component.credits_per_unit.clone(),
                    sync_status: component.sync_status,
                })
                .collect(),
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
        realtime: inference.realtime || inference.voice.is_some(),
        voice: inference.voice.clone(),
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
        voice: super::inference_voice::default_voice(slug),
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
            doc! { "slug": slug, "inference": bson::Bson::Null, "inference_admin_modified": { "$ne": true } },
            doc! { "$set": { "inference": bson::to_bson(&default_inference(slug)).expect("inference serialization") } },
        ).await?;
        if let Some(voice) = super::inference_voice::default_voice(slug) {
            // Do not replace an existing inference block or resurrect an admin clear.
            db.collection::<DownstreamService>(COLLECTION_NAME).update_many(
                doc! { "slug": slug, "inference": { "$type": "object" }, "inference.voice": bson::Bson::Null, "inference_admin_modified": { "$ne": true } },
                doc! { "$set": { "inference.voice": bson::to_bson(&voice).expect("voice serialization") } },
            ).await?;
        }
    }
    Ok(())
}

/// Response-only projection: the derived voice flag is never persisted or accepted on writes.
#[derive(Clone, Debug, Serialize, ToSchema)]
pub struct ServiceCapabilitiesView {
    #[serde(flatten)]
    pub transport: ServiceCapabilities,
    pub supports_realtime_voice: bool,
}

pub fn capabilities(service: &DownstreamService) -> Option<ServiceCapabilitiesView> {
    let voice = service.inference.as_ref().and_then(|i| i.voice.as_ref());
    (service.capabilities.is_some() || voice.is_some()).then(|| ServiceCapabilitiesView {
        transport: service.capabilities.clone().unwrap_or_default(),
        supports_realtime_voice: voice.is_some(),
    })
}

pub fn normalized(inference: &ServiceInference) -> ServiceInference {
    let mut inference = inference.clone();
    inference.realtime |= inference.voice.is_some();
    inference
}

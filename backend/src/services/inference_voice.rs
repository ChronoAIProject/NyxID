//! Voice catalog defaults and validation. Execution support belongs to adapters.
use crate::{
    errors::{AppError, AppResult},
    models::{
        downstream_service::{
            VoiceChoice, VoiceInference, VoiceModel, VoiceProtocol, VoiceUsageSource,
        },
        service_billing::BillingMetric,
    },
};

pub fn default_voice(slug: &str) -> Option<VoiceInference> {
    use BillingMetric::*;
    let (protocol, model, label, usage_source, voices, billing_metrics) = match slug {
        "llm-openai" => (
            VoiceProtocol::OpenaiLive,
            "gpt-live-1",
            "GPT-Live 1",
            VoiceUsageSource::ProviderReported,
            vec![
                "marin", "alloy", "ash", "aube", "ballad", "beacon", "bossa", "brise", "cedar",
                "cinder", "coral", "delta", "echo", "flitz", "gleam", "harema", "juni", "meridian",
                "nira", "noeul", "nuri", "quartz", "ripple", "sage", "shimmer", "shitan",
                "sillage", "stone", "tempo", "verse", "vesper", "willow",
            ],
            vec![VoiceSeconds],
        ),
        "llm-xai" => (
            VoiceProtocol::XaiRealtime,
            "grok-voice-think-fast-2.0",
            "Grok Voice Think Fast 2.0",
            VoiceUsageSource::ServerMeasured,
            vec![
                "eve", "carina", "zagan", "helix", "orion", "luna", "iris", "altair", "zenith",
                "perseus", "helios", "lux", "kepler", "rigel", "cosmo", "celeste", "ursa",
                "sirius", "lumen", "castor", "naksh", "atlas", "ara", "leo", "rex", "sal",
            ],
            vec![
                VoiceSeconds,
                Tokens,
                InputTokens,
                OutputTokens,
                CacheReadTokens,
            ],
        ),
        _ => return None,
    };
    Some(VoiceInference {
        protocol,
        models: vec![VoiceModel {
            id: model.into(),
            label: label.into(),
            default: true,
        }],
        voices: voices
            .into_iter()
            .map(|id| VoiceChoice {
                id: id.into(),
                label: format!("{}{}", id[..1].to_uppercase(), &id[1..]),
            })
            .collect(),
        usage_source,
        billing_metrics,
    })
}

pub fn valid_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_-.".contains(&b))
}

pub fn validate(voice: &VoiceInference) -> AppResult<()> {
    let valid_label =
        |s: &str| !s.trim().is_empty() && s.len() <= 128 && !s.chars().any(char::is_control);
    let mut ids = std::collections::HashSet::new();
    let models_valid = !voice.models.is_empty()
        && voice.models.len() <= 32
        && voice
            .models
            .iter()
            .all(|m| valid_id(&m.id) && valid_label(&m.label) && ids.insert(&m.id))
        && voice.models.iter().filter(|m| m.default).count() <= 1;
    ids.clear();
    let voices_valid = !voice.voices.is_empty()
        && voice.voices.len() <= 64
        && voice
            .voices
            .iter()
            .all(|v| valid_id(&v.id) && valid_label(&v.label) && ids.insert(&v.id));
    let metrics_valid = !voice.billing_metrics.is_empty()
        && voice.billing_metrics.len() <= 6
        && voice.billing_metrics.iter().enumerate().all(|(i, m)| {
            matches!(
                m,
                BillingMetric::VoiceSeconds
                    | BillingMetric::Tokens
                    | BillingMetric::InputTokens
                    | BillingMetric::OutputTokens
                    | BillingMetric::CacheReadTokens
                    | BillingMetric::CacheWriteTokens
            ) && !voice.billing_metrics[..i].contains(m)
        })
        && voice.billing_metrics.contains(&BillingMetric::VoiceSeconds);
    let usage_valid = matches!(
        (voice.protocol, voice.usage_source),
        (
            VoiceProtocol::OpenaiLive,
            VoiceUsageSource::ProviderReported
        ) | (VoiceProtocol::XaiRealtime, VoiceUsageSource::ServerMeasured)
    );
    if !(models_valid && voices_valid && metrics_valid && usage_valid) {
        return Err(AppError::ValidationError("Invalid voice metadata: bounded unique models/voices, at most one default, duration/token metrics and the adapter's usage source are required".into()));
    }
    Ok(())
}

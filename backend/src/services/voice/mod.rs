//! Realtime voice coordination; provider input never grants execution authority.
pub mod confirmation;
#[cfg(test)]
mod confirmation_tests;
mod control;
pub mod credentials;
pub mod openai;
#[cfg(test)]
mod openai_tests;
pub mod receipt;
pub mod runtime;
pub mod session;
#[cfg(test)]
mod tests;
pub mod transcript;

/// Coalesced in-process wake-up; the existing sweep remains the durable backstop.
static DISPATCH: std::sync::LazyLock<tokio::sync::Notify> =
    std::sync::LazyLock::new(tokio::sync::Notify::new);
pub fn wake_dispatch() {
    DISPATCH.notify_one();
}
pub async fn dispatch_wakeup() {
    DISPATCH.notified().await;
}

/// Only compiled, vetted adapters may consume metadata. Models/voices come solely from catalog.
pub fn supported_metadata(voice: &crate::models::downstream_service::VoiceInference) -> bool {
    voice.protocol == crate::models::downstream_service::VoiceProtocol::OpenaiLive
        && super::inference_voice::validate(voice).is_ok()
}

/// Recheck catalog choices at admission and on every live credential revalidation.
pub fn selected_voice(
    metadata: &crate::models::downstream_service::VoiceInference,
    model: &str,
    voice: Option<&str>,
) -> crate::errors::AppResult<String> {
    if !supported_metadata(metadata) || !metadata.models.iter().any(|m| m.id == model) {
        return Err(crate::errors::AppError::VoiceProviderUnavailable);
    }
    voice
        .or_else(|| metadata.voices.first().map(|v| v.id.as_str()))
        .filter(|id| metadata.voices.iter().any(|v| v.id == *id))
        .map(str::to_owned)
        .ok_or(crate::errors::AppError::VoiceProviderUnavailable)
}

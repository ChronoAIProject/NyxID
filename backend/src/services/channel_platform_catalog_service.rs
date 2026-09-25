//! Authoritative platform inventory, derived entirely from registered adapters.
use super::{
    channel_adapters,
    channel_managed::{ManagedOnboardingDescriptor, PlatformCredentialDescriptor},
    channel_platform::{ChannelCapabilities, Ingestion},
    channel_registration::RegistrationDescriptor,
    platform_credential_service,
    provider_token_exchange_service::TokenExchangeCache,
};
use crate::errors::AppResult;
use std::sync::Arc;

pub struct PlatformCatalogEntry {
    pub activities: &'static [super::channel_activity_service::ActivityDescriptor],
    pub platform: String,
    pub display_name: String,
    pub registration: RegistrationDescriptor,
    pub ingestion: Ingestion,
    pub managed_onboarding: Option<ManagedOnboardingDescriptor>,
    pub platform_credentials: Option<(PlatformCredentialDescriptor, bool)>,
    pub capabilities: ChannelCapabilities,
}

pub async fn list(
    db: &mongodb::Database,
    cache: &Arc<TokenExchangeCache>,
) -> AppResult<Vec<PlatformCatalogEntry>> {
    let mut entries = Vec::new();
    for adapter in channel_adapters::registered_adapters(cache) {
        let platform_credentials = if let Some(descriptor) = adapter.platform_credentials() {
            let row = platform_credential_service::load(db, &descriptor).await?;
            Some((
                descriptor,
                platform_credential_service::configured(row.as_ref(), &descriptor),
            ))
        } else {
            None
        };
        entries.push(PlatformCatalogEntry {
            activities: adapter.activity_descriptors(),
            platform: adapter.platform_id().to_string(),
            display_name: adapter.display_name().to_string(),
            registration: adapter.registration(),
            ingestion: adapter.ingestion(),
            managed_onboarding: adapter.managed_onboarding(),
            platform_credentials,
            capabilities: ChannelCapabilities {
                outbound: adapter.outbound_capabilities(),
                media: adapter.media_capabilities(),
            },
        });
    }
    Ok(entries)
}

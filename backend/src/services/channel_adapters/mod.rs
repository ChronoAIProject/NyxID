pub mod discord;
pub mod lark;
pub mod openclaw;
pub mod slack;
pub mod telegram;
pub mod whatsapp;
mod whatsapp_managed;

use std::sync::Arc;

use super::channel_platform::PlatformAdapter;
use super::provider_token_exchange_service::TokenExchangeCache;
use crate::errors::{AppError, AppResult};

/// Single adapter registry. Lark/Feishu share the process-wide token exchange
/// cache with proxy callers. OpenClaw resolves for its separate integration,
/// but its descriptor disables channel-bot registration and webhook routes.
pub fn resolve_adapter(
    platform: &str,
    token_exchange_cache: &Arc<TokenExchangeCache>,
) -> AppResult<Box<dyn PlatformAdapter>> {
    registered_adapters(token_exchange_cache).into_iter()
        .find(|adapter| adapter.platform_id() == platform)
        .ok_or_else(|| AppError::ValidationError(format!(
            "unsupported platform: {platform}. Supported: telegram, discord, lark, feishu, slack, whatsapp"
        )))
}

pub fn registered_adapters(cache: &Arc<TokenExchangeCache>) -> Vec<Box<dyn PlatformAdapter>> {
    vec![
        Box::new(telegram::TelegramAdapter),
        Box::new(discord::DiscordAdapter),
        Box::new(lark::LarkFamilyAdapter::lark(cache.clone())),
        Box::new(lark::LarkFamilyAdapter::feishu(cache.clone())),
        Box::new(slack::SlackAdapter),
        Box::new(whatsapp::WhatsAppAdapter),
        Box::new(openclaw::OpenClawAdapter),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unsupported_platform_lists_only_registrable_platforms() {
        let error = resolve_adapter("unknown", &Arc::new(TokenExchangeCache::new()))
            .err()
            .expect("unknown platform must fail");
        let AppError::ValidationError(message) = error else {
            panic!("expected validation error")
        };
        assert_eq!(
            message,
            "unsupported platform: unknown. Supported: telegram, discord, lark, feishu, slack, whatsapp"
        );
        assert!(
            !resolve_adapter("openclaw", &Arc::new(TokenExchangeCache::new()))
                .unwrap()
                .registration()
                .enabled
        );
    }
}

pub mod discord;
pub mod lark;
pub mod openclaw;
pub mod slack;
pub mod telegram;
pub mod whatsapp;

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
    match platform {
        "telegram" => Ok(Box::new(telegram::TelegramAdapter)),
        "discord" => Ok(Box::new(discord::DiscordAdapter)),
        "lark" => Ok(Box::new(lark::LarkFamilyAdapter::lark(
            token_exchange_cache.clone(),
        ))),
        "feishu" => Ok(Box::new(lark::LarkFamilyAdapter::feishu(
            token_exchange_cache.clone(),
        ))),
        "slack" => Ok(Box::new(slack::SlackAdapter)),
        "whatsapp" => Ok(Box::new(whatsapp::WhatsAppAdapter)),
        "openclaw" => Ok(Box::new(openclaw::OpenClawAdapter)),
        other => Err(AppError::ValidationError(format!(
            "unsupported platform: {other}. Supported: telegram, discord, lark, feishu, slack, whatsapp"
        ))),
    }
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

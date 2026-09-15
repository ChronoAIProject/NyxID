pub mod discord;
pub mod lark;
pub mod openclaw;
pub mod slack;
pub mod telegram;
pub mod telegram_new;
pub mod whatsapp;
mod whatsapp_managed;
pub mod x;

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
    let adapters = registered_adapters(token_exchange_cache);
    let supported = adapters
        .iter()
        .filter(|a| a.registration().enabled)
        .map(|a| a.platform_id())
        .collect::<Vec<_>>()
        .join(", ");
    adapters
        .into_iter()
        .find(|adapter| adapter.platform_id() == platform)
        .ok_or_else(|| {
            AppError::ValidationError(format!(
                "unsupported platform: {platform}. Supported: {supported}"
            ))
        })
}

/// Unknown legacy platforms and botless device channels have no outbound transport.
pub fn outbound_capabilities(
    platform: &str,
    cache: &Arc<TokenExchangeCache>,
) -> super::channel_platform::OutboundCapabilities {
    resolve_adapter(platform, cache)
        .map(|adapter| adapter.outbound_capabilities())
        .unwrap_or(super::channel_platform::OutboundCapabilities::NONE)
}

pub fn registered_adapters(cache: &Arc<TokenExchangeCache>) -> Vec<Box<dyn PlatformAdapter>> {
    vec![
        Box::new(telegram::TelegramAdapter),
        Box::new(telegram_new::TelegramNewAdapter),
        Box::new(discord::DiscordAdapter),
        Box::new(lark::LarkFamilyAdapter::lark(cache.clone())),
        Box::new(lark::LarkFamilyAdapter::feishu(cache.clone())),
        Box::new(slack::SlackAdapter),
        Box::new(whatsapp::WhatsAppAdapter),
        Box::new(x::XAdapter::default()),
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
            "unsupported platform: unknown. Supported: telegram, telegram-new, discord, lark, feishu, slack, whatsapp, x"
        );
        assert!(
            !resolve_adapter("openclaw", &Arc::new(TokenExchangeCache::new()))
                .unwrap()
                .registration()
                .enabled
        );
    }
    #[tokio::test]
    async fn outbound_capability_contract_for_every_registered_adapter() {
        use crate::services::channel_platform::{OutboundCapabilities, OutboundEdit};
        let adapters = registered_adapters(&Arc::new(TokenExchangeCache::new()));
        assert_eq!(adapters.len(), 9);
        let http = reqwest::Client::new();
        let edit = OutboundEdit {
            text: Some("updated".into()),
            metadata: None,
        };
        for adapter in adapters {
            let capabilities = adapter.outbound_capabilities();
            let (reply_to, thread) = match adapter.platform_id() {
                "telegram" | "telegram-new" | "slack" => (true, true),
                "discord" => (false, true),
                "whatsapp" => (true, false),
                "lark" | "feishu" | "x" | "openclaw" => (false, false),
                unexpected => panic!("Add outbound transport contracts for {unexpected}"),
            };
            // Corresponding production request-builder tests exercise these
            // anchors and metadata keys (including deliberate ignored fields).
            assert_eq!(
                capabilities,
                OutboundCapabilities {
                    initiated_send: adapter.platform_id() != "openclaw",
                    reply_to,
                    thread,
                    edit: matches!(adapter.platform_id(), "lark" | "feishu"),
                }
            );
            // Invalid credentials stop the native Lark override before HTTP; the
            // default implementation always returns EditUnsupported. Successful
            // native HTTP edit contracts are covered in lark's wiremock test.
            let result = adapter.edit_reply(&http, "", "message", &edit).await;
            assert_eq!(
                matches!(result, Err(AppError::ChannelPlatformEditUnsupported)),
                !capabilities.edit,
                "{}",
                adapter.platform_id()
            );
        }
    }
}

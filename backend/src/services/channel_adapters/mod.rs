pub mod aurinko;
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
) -> super::channel_platform::ChannelCapabilities {
    use super::channel_platform::{ChannelCapabilities, MediaCapabilities, OutboundCapabilities};
    match resolve_adapter(platform, cache) {
        Ok(adapter) => ChannelCapabilities {
            outbound: adapter.outbound_capabilities(),
            media: adapter.media_capabilities(),
        },
        Err(_) => ChannelCapabilities {
            outbound: OutboundCapabilities::NONE,
            media: MediaCapabilities::NONE,
        },
    }
}

pub fn registered_adapters(cache: &Arc<TokenExchangeCache>) -> Vec<Box<dyn PlatformAdapter>> {
    vec![
        Box::new(telegram::TelegramAdapter::default()),
        Box::new(telegram_new::TelegramNewAdapter::default()),
        Box::new(discord::DiscordAdapter::default()),
        Box::new(lark::LarkFamilyAdapter::lark(cache.clone())),
        Box::new(lark::LarkFamilyAdapter::feishu(cache.clone())),
        Box::new(slack::SlackAdapter::default()),
        Box::new(whatsapp::WhatsAppAdapter),
        Box::new(x::XAdapter::default()),
        Box::new(openclaw::OpenClawAdapter),
        Box::new(aurinko::AurinkoAdapter::default()),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aurinko_is_appended_and_legacy_adapters_keep_their_policies() {
        let adapters = registered_adapters(&Arc::new(TokenExchangeCache::new()));
        let platforms: Vec<_> = adapters
            .iter()
            .map(|adapter| adapter.platform_id())
            .collect();
        assert_eq!(
            platforms,
            [
                "telegram",
                "telegram-new",
                "discord",
                "lark",
                "feishu",
                "slack",
                "whatsapp",
                "x",
                "openclaw",
                "aurinko"
            ]
        );
        for adapter in &adapters[..9] {
            assert!(!adapter.serializes_lifecycle(), "{}", adapter.platform_id());
            assert!(
                !adapter.persists_reply_attempt(),
                "{}",
                adapter.platform_id()
            );
            assert!(!matches!(
                adapter.webhook_policy(b"{}"),
                super::super::channel_platform::WebhookPolicy::RetryAwareInline
            ));
        }
        assert!(adapters[9].serializes_lifecycle());
        assert!(adapters[9].persists_reply_attempt());
    }

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
            "unsupported platform: unknown. Supported: telegram, telegram-new, discord, lark, feishu, slack, whatsapp, x, aurinko"
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
        assert_eq!(adapters.len(), 10);
        // Force all native network attempts to an unreachable local proxy.
        // No real platform receives the dummy credentials used by this contract.
        let http = reqwest::Client::builder()
            .proxy(reqwest::Proxy::all("http://127.0.0.1:1").unwrap())
            .timeout(std::time::Duration::from_secs(1))
            .build()
            .unwrap();
        let edit = OutboundEdit {
            text: Some("updated".into()),
            metadata: None,
        };
        for adapter in adapters {
            let capabilities = adapter.outbound_capabilities();
            let (reply_to, thread) = match adapter.platform_id() {
                "telegram" | "telegram-new" | "slack" => (true, true),
                "whatsapp" | "aurinko" => (true, false),
                "discord" | "lark" | "feishu" | "x" | "openclaw" => (false, false),
                unexpected => panic!("Add outbound transport contracts for {unexpected}"),
            };
            // Corresponding production request-builder tests exercise these
            // anchors and metadata keys (including deliberate ignored fields).
            assert_eq!(
                capabilities,
                OutboundCapabilities {
                    initiated_send: !matches!(adapter.platform_id(), "openclaw" | "aurinko"),
                    reply_to,
                    thread,
                    edit: matches!(
                        adapter.platform_id(),
                        "telegram" | "telegram-new" | "discord" | "slack" | "lark" | "feishu"
                    ),
                }
            );
            // Native overrides must fail with a transport/credential error, not
            // EditUnsupported. Wiremock tests separately prove platform acceptance.
            let result = adapter
                .edit_reply(&http, &"".into(), "chat", "123", &edit)
                .await;
            if capabilities.edit {
                assert!(
                    matches!(&result, Err(AppError::ChannelPlatformError(_))),
                    "{}: {result:?}",
                    adapter.platform_id()
                );
            }
            assert_eq!(
                matches!(result, Err(AppError::ChannelPlatformEditUnsupported)),
                !capabilities.edit,
                "{}",
                adapter.platform_id()
            );
        }
    }
}

#[cfg(test)]
mod media_tests;

#[cfg(test)]
mod media_contract {
    use super::*;
    use crate::services::channel_platform::*;

    #[tokio::test]
    async fn all_media_declarations_have_native_implementations() {
        let adapters = registered_adapters(&Arc::new(TokenExchangeCache::new()));
        assert_eq!(adapters.len(), 10);
        let http = reqwest::Client::builder()
            .proxy(reqwest::Proxy::all("http://127.0.0.1:1").unwrap())
            .timeout(std::time::Duration::from_millis(200))
            .build()
            .unwrap();
        for adapter in adapters {
            let expected = match adapter.platform_id() {
                "telegram" | "telegram-new" | "discord" | "slack" | "lark" | "feishu"
                | "whatsapp" => MediaCapabilities::ALL,
                "x" => MediaCapabilities {
                    inbound: &[MediaKind::Image, MediaKind::Video],
                    outbound: &[MediaKind::Image, MediaKind::Video],
                },
                "openclaw" | "aurinko" => MediaCapabilities::NONE,
                other => panic!("Pin the media declaration for {other}"),
            };
            assert_eq!(adapter.media_capabilities(), expected);
            let attachment = InboundAttachment {
                content_type: "image".into(),
                url: "https://invalid.example/file".into(),
                platform_message_id: None,
                file_key: None,
                image_key: None,
                filename: None,
                mime_type: None,
                size_bytes: None,
            };
            let credentials = BotCredentials {
                billing: None,
                token: "invalid",
                platform_bot_id: Some("123"),
                platform_secrets: None,
            };
            let fetched = adapter
                .fetch_attachment(&http, &credentials, &attachment, 10)
                .await;
            assert_eq!(
                matches!(fetched, Err(AppError::ChannelMediaUnsupported)),
                expected.inbound.is_empty(),
                "{}",
                adapter.platform_id()
            );
            if !expected.inbound.is_empty() {
                assert!(
                    matches!(
                        fetched,
                        Err(AppError::ChannelMediaFetchFailed(_)
                            | AppError::ChannelPlatformError(_))
                    ),
                    "{}: {fetched:?}",
                    adapter.platform_id()
                );
            }
            for &kind in expected.outbound {
                let reply = OutboundReply {
                    text: None,
                    metadata: None,
                    reply_to_platform_message_id: None,
                    attachments: vec![MaterializedAttachment {
                        kind,
                        bytes: bytes::Bytes::from_static(b"data"),
                        filename: None,
                        mime_type: None,
                        caption: None,
                    }],
                };
                let result = adapter.send_reply(&http, &credentials, "123", &reply).await;
                assert!(
                    matches!(result, Err(AppError::ChannelPlatformError(_))),
                    "{}: {result:?}",
                    adapter.platform_id()
                );
            }
        }
    }
}

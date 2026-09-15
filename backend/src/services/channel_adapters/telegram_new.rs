use super::telegram::TelegramAdapter;
use crate::errors::{AppError, AppResult};
use crate::models::channel_bot::ChannelBot;
use crate::services::channel_managed::{PlatformCredentialDescriptor, PlatformCredentialField};
use crate::services::channel_platform::{
    BotCredentials, BotIdentity, InboundMessage, OutboundReply, PlatformAdapter,
    PlatformVerifySecrets, RegistrationDescriptor, RegistrationValues,
};

pub const PLATFORM: &str = "telegram-new";
pub const MANAGER_TOKEN: &str = "manager_bot_token";

pub struct TelegramNewAdapter;

pub fn credential_descriptor() -> PlatformCredentialDescriptor {
    PlatformCredentialDescriptor {
        provider: PLATFORM,
        backing: crate::services::channel_managed::PlatformCredentialBacking::Stored,
        label: "Telegram — bot creation",
        fields: &[PlatformCredentialField {
            name: MANAGER_TOKEN,
            label: "Manager bot token",
            secret: true,
            required: true,
            numeric: false,
            help: "Token of a dedicated bot with bot management enabled in BotFather. Saving validates the bot and connects its webhook automatically.",
        }],
        setup_checklist: &[
            "Create a dedicated manager bot in BotFather and enable management of other bots.",
            "Use separate manager bots for staging and production, and keep them separate from the approval bot.",
            "Save the manager token here, then choose Telegram in Add Channel Bot.",
        ],
        webhook_secret_field: Some(MANAGER_TOKEN),
    }
}

#[async_trait::async_trait]
impl PlatformAdapter for TelegramNewAdapter {
    /// Delegates message_thread_id and reply handling to the Telegram transport.
    fn outbound_capabilities(&self) -> crate::services::channel_platform::OutboundCapabilities {
        super::telegram::TelegramAdapter.outbound_capabilities()
    }

    fn platform_id(&self) -> &str {
        PLATFORM
    }

    fn platform_credentials(&self) -> Option<PlatformCredentialDescriptor> {
        Some(credential_descriptor())
    }

    fn registration(&self) -> RegistrationDescriptor {
        RegistrationDescriptor {
            fields: &[],
            token_fields: &[],
            extra_fields: &[],
            automatic_webhook: true,
            setup_instructions: &[
                "Assign an agent to this bot, then open its Telegram chat and send a test message.",
            ],
            ..Default::default()
        }
    }

    fn registration_token(
        &self,
        _fields: &RegistrationValues<'_>,
    ) -> AppResult<zeroize::Zeroizing<String>> {
        Err(AppError::ValidationError(
            "Use Telegram's creation flow to connect this bot".into(),
        ))
    }

    fn reply_context(
        &self,
        thread: Option<&str>,
        created: chrono::DateTime<chrono::Utc>,
        metadata: &mut Option<serde_json::Value>,
    ) {
        TelegramAdapter.reply_context(thread, created, metadata);
    }

    async fn verify_webhook(
        &self,
        bot: &ChannelBot,
        secrets: Option<&PlatformVerifySecrets>,
        headers: &axum::http::HeaderMap,
        body: &[u8],
    ) -> AppResult<()> {
        TelegramAdapter
            .verify_webhook(bot, secrets, headers, body)
            .await
    }

    async fn parse_inbound(&self, body: &[u8]) -> AppResult<Vec<InboundMessage>> {
        TelegramAdapter.parse_inbound(body).await
    }

    async fn send_reply(
        &self,
        http: &reqwest::Client,
        credentials: &BotCredentials<'_>,
        conversation: &str,
        reply: &OutboundReply,
    ) -> AppResult<Option<String>> {
        TelegramAdapter
            .send_reply(http, credentials, conversation, reply)
            .await
    }

    async fn register_webhook(
        &self,
        http: &reqwest::Client,
        token: &str,
        url: &str,
        secret: &str,
    ) -> AppResult<()> {
        TelegramAdapter
            .register_webhook(http, token, url, secret)
            .await
    }

    async fn verify_bot_token(
        &self,
        http: &reqwest::Client,
        credentials: &BotCredentials<'_>,
    ) -> AppResult<BotIdentity> {
        TelegramAdapter.verify_bot_token(http, credentials).await
    }
}

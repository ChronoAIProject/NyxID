use super::telegram::TelegramAdapter;
use crate::services::channel_platform::{FetchedMedia, MediaCapabilities};

use crate::errors::{AppError, AppResult};
use crate::models::channel_bot::ChannelBot;
use crate::services::channel_managed::{PlatformCredentialDescriptor, PlatformCredentialField};
use crate::services::channel_platform::{
    BotCredentials, BotIdentity, InboundMessage, OutboundEdit, OutboundReply, PlatformAdapter,
    PlatformVerifySecrets, RegistrationDescriptor, RegistrationValues,
};

pub const PLATFORM: &str = "telegram-new";
pub const MANAGER_TOKEN: &str = "manager_bot_token";

#[derive(Default)]
pub struct TelegramNewAdapter {
    transport: TelegramAdapter,
}

impl TelegramNewAdapter {
    #[cfg(test)]
    pub(super) fn media_test_adapter(base: &str) -> Self {
        Self {
            transport: TelegramAdapter::media_test_adapter(base),
        }
    }
}

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
    fn display_name(&self) -> &str {
        "Telegram"
    }
    fn media_capabilities(&self) -> MediaCapabilities {
        MediaCapabilities::ALL
    }
    /// Delegates message_thread_id and reply handling to the Telegram transport.
    fn outbound_capabilities(&self) -> crate::services::channel_platform::OutboundCapabilities {
        self.transport.outbound_capabilities()
    }

    fn platform_id(&self) -> &str {
        PLATFORM
    }

    fn platform_credentials(&self) -> Option<PlatformCredentialDescriptor> {
        Some(credential_descriptor())
    }

    fn registration(&self) -> RegistrationDescriptor {
        RegistrationDescriptor {
            documentation_url: Some("https://core.telegram.org/api/bots/managed-bots"),
            managed_only: true,
            managed_only_message: "Use Telegram's browser creation flow at /channel-bots/telegram-new",
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
        self.transport.reply_context(thread, created, metadata);
    }

    async fn verify_webhook(
        &self,
        bot: &ChannelBot,
        secrets: Option<&PlatformVerifySecrets>,
        headers: &axum::http::HeaderMap,
        body: &[u8],
    ) -> AppResult<()> {
        self.transport
            .verify_webhook(bot, secrets, headers, body)
            .await
    }

    async fn parse_inbound(&self, body: &[u8]) -> AppResult<Vec<InboundMessage>> {
        self.transport.parse_inbound(body).await
    }

    async fn fetch_attachment(
        &self,
        http: &reqwest::Client,
        credentials: &BotCredentials<'_>,
        attachment: &crate::services::channel_platform::InboundAttachment,
        max_bytes: u64,
    ) -> AppResult<FetchedMedia> {
        self.transport
            .fetch_attachment(http, credentials, attachment, max_bytes)
            .await
    }

    async fn send_reply(
        &self,
        http: &reqwest::Client,
        credentials: &BotCredentials<'_>,
        conversation: &str,
        reply: &OutboundReply,
    ) -> AppResult<Option<String>> {
        self.transport
            .send_reply(http, credentials, conversation, reply)
            .await
    }

    async fn edit_reply(
        &self,
        http: &reqwest::Client,
        credentials: &BotCredentials<'_>,
        conversation_id: &str,
        platform_message_id: &str,
        edit: &OutboundEdit,
    ) -> AppResult<()> {
        self.transport
            .edit_reply(
                http,
                credentials,
                conversation_id,
                platform_message_id,
                edit,
            )
            .await
    }

    async fn register_webhook(
        &self,
        http: &reqwest::Client,
        token: &str,
        url: &str,
        secret: &str,
    ) -> AppResult<()> {
        self.transport
            .register_webhook(http, token, url, secret)
            .await
    }

    async fn verify_bot_token(
        &self,
        http: &reqwest::Client,
        credentials: &BotCredentials<'_>,
    ) -> AppResult<BotIdentity> {
        self.transport.verify_bot_token(http, credentials).await
    }
}

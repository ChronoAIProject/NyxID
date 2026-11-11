//! Shared bounded provider fetch. Callers resolve their own live authority first.
use crate::{
    crypto::aes::EncryptionKeys,
    errors::AppResult,
    models::{channel_bot::ChannelBot, channel_message::StoredAttachment},
    services::channel_platform::{FetchedMedia, PlatformAdapter},
};

pub async fn fetch(
    db: &mongodb::Database,
    keys: &EncryptionKeys,
    client: &reqwest::Client,
    bot: &ChannelBot,
    attachment: &StoredAttachment,
    adapter: &dyn PlatformAdapter,
    max_bytes: u64,
) -> AppResult<FetchedMedia> {
    let inbound = crate::services::channel_platform::InboundAttachment {
        content_type: attachment.content_type.clone(),
        url: attachment.provider_ref.clone(),
        platform_message_id: attachment.platform_message_id.clone(),
        file_key: attachment.file_key.clone(),
        image_key: attachment.image_key.clone(),
        filename: attachment.filename.clone(),
        mime_type: attachment.mime_type.clone(),
        size_bytes: attachment.size_bytes,
    };
    let token =
        crate::services::channel_credentials::resolve_bot_token(db, keys, adapter, bot).await?;
    let secrets = if bot.credential_source == "platform" {
        Some(crate::services::channel_managed::build_verify_secrets(db, keys, adapter, bot).await?)
    } else {
        None
    };
    let fetched = adapter
        .fetch_attachment(
            client,
            &crate::services::channel_platform::BotCredentials {
                billing: None,
                token: &token,
                platform_bot_id: Some(&bot.platform_bot_id),
                platform_secrets: secrets.as_ref(),
            },
            &inbound,
            max_bytes,
        )
        .await?;
    Ok(fetched)
}

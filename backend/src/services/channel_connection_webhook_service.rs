use bson::doc;

use super::channel_platform::{BotCredentials, CredentialResolution, PlatformAdapter};
use super::coordination_service::{LeaseStore, cluster_lease_runtime};
use crate::crypto::aes::EncryptionKeys;
use crate::errors::{AppError, AppResult};
use crate::models::channel_bot::{COLLECTION_NAME, ChannelBot};

pub fn supports(adapter: &dyn PlatformAdapter) -> bool {
    adapter.platform_webhook()
        && matches!(
            adapter.credential_resolution(),
            CredentialResolution::OAuthConnection { .. }
        )
}

async fn serialized<T>(
    db: &mongodb::Database,
    platform: &str,
    work: impl std::future::Future<Output = AppResult<T>>,
) -> AppResult<T> {
    let runtime = cluster_lease_runtime();
    let lease = runtime
        .acquire(db, &format!("channel-webhook:{platform}"))
        .await?
        .ok_or_else(|| {
            AppError::Conflict("Channel webhook setup is in progress; retry shortly".into())
        })?;
    let result = runtime
        .run_while_renewed(
            db,
            &lease,
            tokio::time::timeout(std::time::Duration::from_secs(180), work),
        )
        .await;
    let _ = LeaseStore::release(db, &lease).await;
    match result {
        Some(Ok(result)) => result,
        _ => Err(AppError::ChannelPlatformError(
            "Channel webhook setup was interrupted; select Verify to retry".into(),
        )),
    }
}

pub async fn configure(
    db: &mongodb::Database,
    keys: &EncryptionKeys,
    http: &reqwest::Client,
    adapter: &dyn PlatformAdapter,
    bot: &ChannelBot,
    base_url: &str,
) -> AppResult<bool> {
    if !supports(adapter) {
        return Ok(false);
    }
    let descriptor = adapter
        .platform_credentials()
        .ok_or_else(super::channel_managed::unavailable)?;
    let platform =
        super::platform_credential_service::load_decrypted(db, keys, &descriptor).await?;
    if !adapter.connection_webhook_configured(&platform) {
        if bot.webhook_registered {
            return Err(AppError::ValidationError(
                "Restore the platform webhook credentials to verify this connection".into(),
            ));
        }
        return Ok(false);
    }
    serialized(db, adapter.platform_id(), async {
        let current = super::channel_bot_service::get_bot(db, &bot.id).await?;
        if !current.is_active || current.connection_id != bot.connection_id {
            return Err(AppError::Conflict("Channel connection changed during webhook setup".into()));
        }
        let token = super::channel_credentials::resolve_bot_token(db, keys, adapter, &current).await?;
        let credentials = BotCredentials {
            token: &token, platform_bot_id: Some(&current.platform_bot_id), platform_secrets: Some(&platform),
        };
        let url = format!("{}/api/v1/webhooks/channel/{}/platform", base_url.trim_end_matches('/'), adapter.platform_id());
        adapter.setup_connection_webhook(http, &credentials, &current.id, &url).await?;
        let result = db.collection::<ChannelBot>(COLLECTION_NAME).update_one(
            doc! {"_id": &current.id, "is_active": true, "connection_id": &current.connection_id, "updated_at": bson::DateTime::from_chrono(current.updated_at)},
            doc! {"$set": {"webhook_registered": true, "status": "active", "error": null,
                "poll_lease_until": null, "poll_error_count": 0, "updated_at": bson::DateTime::now()}},
        ).await?;
        if result.matched_count == 0 {
            let latest = super::channel_bot_service::get_bot(db, &current.id).await?;
            if !latest.is_active || latest.connection_id != current.connection_id {
                let _ = adapter.remove_connection_webhook(http, &platform, &current.id).await;
            }
            return Err(AppError::Conflict("Channel connection changed during webhook setup; retry".into()));
        }
        Ok(true)
    }).await
}

pub async fn remove(
    db: &mongodb::Database,
    keys: &EncryptionKeys,
    http: &reqwest::Client,
    adapter: &dyn PlatformAdapter,
    bot: &ChannelBot,
) -> AppResult<()> {
    let descriptor = adapter
        .platform_credentials()
        .ok_or_else(super::channel_managed::unavailable)?;
    let platform =
        super::platform_credential_service::load_decrypted(db, keys, &descriptor).await?;
    if !adapter.connection_webhook_configured(&platform) && !bot.webhook_registered {
        return Ok(());
    }
    serialized(
        db,
        adapter.platform_id(),
        adapter.remove_connection_webhook(http, &platform, &bot.id),
    )
    .await
}

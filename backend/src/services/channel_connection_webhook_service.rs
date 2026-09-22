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

pub(crate) async fn serialized<T>(
    db: &mongodb::Database,
    platform: &str,
    work: impl std::future::Future<Output = AppResult<T>>,
) -> AppResult<T> {
    let runtime = cluster_lease_runtime();
    let lease_key = format!("channel-webhook:{platform}");
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(3);
    let lease = loop {
        if let Some(lease) = runtime.acquire(db, &lease_key).await? {
            break lease;
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(AppError::Conflict(
                "Channel operation is in progress; retry shortly".into(),
            ));
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    };
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
    billing: &super::billing::BillingService,
    keys: &EncryptionKeys,
    http: &reqwest::Client,
    adapter: &dyn PlatformAdapter,
    bot: &ChannelBot,
    base_url: &str,
) -> AppResult<bool> {
    if !supports(adapter) {
        return Ok(false);
    }
    let result = serialized(db, adapter.platform_id(), async {
        let current = super::channel_bot_service::get_bot(db, &bot.id).await?;
        if !current.is_active || current.connection_id != bot.connection_id {
            return Err(AppError::Conflict("Channel connection changed during webhook setup".into()));
        }
        let result = configure_inner(db, billing, keys, http, adapter, &current, base_url).await;
        if result.is_err() && current.platform == "x"
            && (billing.billing_enabled() || current.webhook_registered || super::channel_adapters::x::public_events_enabled(&current)) {
            super::channel_credentials::fail_bot(db, &current,
                "Webhook or billing setup needs attention. Restore credits and configuration, then select Verify.").await?;
        }
        result
    }).await;
    if result.is_err()
        && bot.platform == "x"
        && (billing.billing_enabled()
            || bot.webhook_registered
            || super::channel_adapters::x::public_events_enabled(bot))
    {
        super::channel_credentials::fail_bot(db, bot,
            "Webhook setup did not complete. Check credits and webhook configuration, then select Verify.").await?;
    }
    result
}

async fn configure_inner(
    db: &mongodb::Database,
    billing: &super::billing::BillingService,
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
        if adapter.platform_id() == "x"
            && (billing.billing_enabled() || super::channel_adapters::x::public_events_enabled(bot))
        {
            return Err(AppError::BillingNotConfigured(
                "Paid X channels and public events require the platform webhook credentials".into(),
            ));
        }
        if bot.webhook_registered {
            return Err(AppError::ValidationError(
                "Restore the platform webhook credentials to verify this connection".into(),
            ));
        }
        return Ok(false);
    }
    async {
        let current = super::channel_bot_service::get_bot(db, &bot.id).await?;
        if !current.is_active || current.connection_id != bot.connection_id {
            return Err(AppError::Conflict("Channel connection changed during webhook setup".into()));
        }
        let token = super::channel_credentials::resolve_bot_token(db, keys, adapter, &current).await?;
        if let Some(meter) = super::channel_billing_service::ChannelBilling::for_bot(db, billing, &current, None) {
            meter.admit().await?;
        }
        let credentials = BotCredentials {
            billing: None,
            token: &token, platform_bot_id: Some(&current.platform_bot_id), platform_secrets: Some(&platform),
        };
        let url = format!("{}/api/v1/webhooks/channel/{}/platform", base_url.trim_end_matches('/'), adapter.platform_id());
        if billing.billing_enabled() || current.webhook_registered || super::channel_adapters::x::public_events_enabled(&current) {
            // Record a possible remote subscription before the provider effect,
            // so failed/partial setup remains eligible for cleanup retry.
            let marked = db.collection::<ChannelBot>(COLLECTION_NAME).update_one(
                doc! {"_id": &current.id, "is_active": true, "connection_id": &current.connection_id, "updated_at": bson::DateTime::from_chrono(current.updated_at)},
                doc! {"$set": {"webhook_registered": true}},
            ).await?;
            if marked.matched_count == 0 {
                return Err(AppError::Conflict("Channel changed before webhook setup; retry".into()));
            }
        }
        adapter.setup_connection_webhook(http, &credentials, &current, &url).await?;
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
    }.await
}

/// Recheck failed/deleted state under the Verify lease so a recovered channel
/// never loses its restored subscription.
pub async fn remove_stopped(
    db: &mongodb::Database,
    keys: &EncryptionKeys,
    http: &reqwest::Client,
    adapter: &dyn PlatformAdapter,
    bot: &ChannelBot,
) -> AppResult<()> {
    serialized(db, adapter.platform_id(), async {
        let current = super::channel_bot_service::get_bot(db, &bot.id).await?;
        if (current.is_active && current.status != "failed") || !current.webhook_registered {
            return Ok(());
        }
        let descriptor = adapter
            .platform_credentials()
            .ok_or_else(super::channel_managed::unavailable)?;
        let platform =
            super::platform_credential_service::load_decrypted(db, keys, &descriptor).await?;
        adapter
            .remove_connection_webhook(http, &platform, &current.id)
            .await?;
        db.collection::<ChannelBot>(COLLECTION_NAME).update_one(
            doc! {"_id": &current.id, "is_active": current.is_active, "status": &current.status, "connection_id": &current.connection_id},
            doc! {"$set": {"webhook_registered": false}},
        ).await?;
        Ok(())
    })
    .await
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

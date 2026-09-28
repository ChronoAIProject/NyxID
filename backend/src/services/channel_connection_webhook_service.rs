use bson::doc;

use super::channel_platform::{
    BotCredentials, CredentialResolution, PlatformAdapter, WebhookSetupProgress,
};
use super::coordination_service::{LeaseStore, cluster_lease_runtime};
use crate::crypto::aes::EncryptionKeys;
use crate::errors::{AppError, AppResult};
use crate::models::channel_bot::{COLLECTION_NAME, ChannelBot};

pub(crate) const SETUP_PENDING_ERROR: &str =
    "Webhook setup has not completed; select Verify to retry.";

pub fn supports(adapter: &dyn PlatformAdapter) -> bool {
    adapter.platform_webhook()
        && matches!(
            adapter.credential_resolution(),
            CredentialResolution::OAuthConnection { .. }
        )
}

pub fn callback_url(base_url: &str, platform: &str) -> String {
    format!(
        "{}/api/v1/webhooks/channel/{platform}/platform",
        base_url.trim_end_matches('/')
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
    let progress = WebhookSetupProgress::default();
    let started = std::sync::atomic::AtomicBool::new(false);
    // Keep provider setup state off the shared channel verification stack.
    let result = serialized(db, adapter.platform_id(), Box::pin(async {
        let current = super::channel_bot_service::get_bot(db, &bot.id).await?;
        if !current.is_active || current.connection_id != bot.connection_id {
            return Err(AppError::Conflict("Channel connection changed during webhook setup".into()));
        }
        started.store(true, std::sync::atomic::Ordering::Relaxed);
        let result = configure_inner(db, billing, keys, http, adapter, &current, base_url, &progress).await;
        if result.is_err()
            && setup_failure_requires_stop(&current, billing.billing_enabled(), &progress) {
            fail_setup(db, &current,
                "Webhook or billing setup needs attention. Restore credits and configuration, then select Verify.").await?;
        }
        result
    })).await;
    if result.is_err()
        && started.load(std::sync::atomic::Ordering::Relaxed)
        && setup_failure_requires_stop(bot, billing.billing_enabled(), &progress)
    {
        fail_setup(db, bot,
            "Webhook setup did not complete. Check credits and webhook configuration, then select Verify.").await?;
    }
    result
}

async fn fail_setup(db: &mongodb::Database, bot: &ChannelBot, cause: &str) -> AppResult<()> {
    // Claim either an ordinary failure transition or our incomplete setup marker.
    // Replacing the marker makes inner/outer error handling audit exactly once.
    let result = db.collection::<ChannelBot>(COLLECTION_NAME).update_one(
        doc! {"_id": &bot.id, "is_active": true, "connection_id": &bot.connection_id,
            "updated_at": bson::DateTime::from_chrono(bot.updated_at),
            "$or": [{"status": {"$ne": "failed"}}, {"status": "failed", "error": SETUP_PENDING_ERROR}]},
        doc! {"$set": {"status": "failed", "error": cause, "updated_at": bson::DateTime::now()}},
    ).await?;
    if result.modified_count > 0 {
        super::channel_credentials::audit_failure(db, bot, cause).await?;
    }
    Ok(())
}

fn setup_failure_requires_stop(
    bot: &ChannelBot,
    billing_enabled: bool,
    progress: &WebhookSetupProgress,
) -> bool {
    if bot.webhook_registered
        && bot.status == "active"
        && progress.read_only_safe()
        && !progress.mutation_started()
    {
        return false;
    }
    bot.platform == "x"
        && (billing_enabled
            || super::channel_adapters::x::webhook_events_enabled(bot)
            || bot.webhook_registered
            || progress.mutation_started())
}

#[allow(clippy::too_many_arguments)]
async fn configure_inner(
    db: &mongodb::Database,
    billing: &super::billing::BillingService,
    keys: &EncryptionKeys,
    http: &reqwest::Client,
    adapter: &dyn PlatformAdapter,
    bot: &ChannelBot,
    base_url: &str,
    progress: &WebhookSetupProgress,
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
            && (billing.billing_enabled()
                || super::channel_adapters::x::webhook_events_enabled(bot))
        {
            return Err(AppError::BillingNotConfigured(
                "Paid X channels and events other than DMs require the platform webhook credentials".into(),
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
        let token = match super::channel_credentials::resolve_bot_token(db, keys, adapter, &current).await {
            Err(error @ AppError::ChannelPlatformError(_)) => {
                // Live credential resolution classifies temporary refresh failures
                // separately from revoked/missing credentials, which fail the bot.
                progress.mark_read_only_safe();
                return Err(error);
            }
            result => result?,
        };
        if let Some(meter) = super::channel_billing_service::ChannelBilling::for_bot(db, billing, &current, None) {
            meter.admit().await?;
        }
        let credentials = BotCredentials {
            billing: None,
            token: &token, platform_bot_id: Some(&current.platform_bot_id), platform_secrets: Some(&platform),
        };
        let url = callback_url(base_url, adapter.platform_id());
        // Persist recovery state before any remote effect, including first-time
        // unmetered DM setup. Interrupted setup stays visible to cleanup.
        let mut setup_state = doc! {"webhook_registered": true};
        if !current.webhook_registered {
            setup_state.insert("status", "failed");
            setup_state.insert("error", SETUP_PENDING_ERROR);
        }
        let marked = db.collection::<ChannelBot>(COLLECTION_NAME).update_one(
            doc! {"_id": &current.id, "is_active": true, "connection_id": &current.connection_id, "updated_at": bson::DateTime::from_chrono(current.updated_at)},
            doc! {"$set": setup_state},
        ).await?;
        if marked.matched_count == 0 {
            return Err(AppError::Conflict("Channel changed before webhook setup; retry".into()));
        }
        progress.mark_read_only_safe();
        if let Err(error) = adapter.setup_connection_webhook(http, &credentials, &current, &url, progress).await {
            if !current.webhook_registered && !billing.billing_enabled()
                && !super::channel_adapters::x::webhook_events_enabled(&current)
                && !progress.mutation_started() {
                // A completed read-only failure created no account subscription.
                // Restore the existing DM polling fallback under the same fence.
                db.collection::<ChannelBot>(COLLECTION_NAME).update_one(
                    doc! {"_id": &current.id, "is_active": true, "connection_id": &current.connection_id, "updated_at": bson::DateTime::from_chrono(current.updated_at), "status": "failed"},
                    doc! {"$set": {"webhook_registered": false, "status": &current.status, "error": &current.error}},
                ).await?;
            }
            return Err(error);
        }
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

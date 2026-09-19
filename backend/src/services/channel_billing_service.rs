//! X channel usage shares the catalog rate, funding order, durable meter and ledger.
use bson::doc;
use futures::TryStreamExt;

use super::billing::{
    BillingIngress, BillingRouteContext, BillingService, MeteredProxyContext, NodeIntent,
};
use crate::errors::{AppError, AppResult};
use crate::models::service_billing::{BillingMetric, PlatformUsage};
use crate::models::usage_meter::CredentialClass;
use crate::models::{channel_bot::ChannelBot, downstream_service::DownstreamService};

pub struct ChannelBilling {
    db: mongodb::Database,
    billing: BillingService,
    owner_id: String,
    bot_id: Option<String>,
    webhook_registered: bool,
    api_key_id: Option<String>,
    forwarded: std::sync::atomic::AtomicBool,
}

impl ChannelBilling {
    pub fn for_bot(
        db: &mongodb::Database,
        billing: &BillingService,
        bot: &ChannelBot,
        api_key_id: Option<&str>,
    ) -> Option<Self> {
        Self::for_owner(db, billing, &bot.platform, &bot.user_id, api_key_id).map(|mut context| {
            context.bot_id = Some(bot.id.clone());
            context.webhook_registered = bot.webhook_registered;
            context
        })
    }

    pub fn for_owner(
        db: &mongodb::Database,
        billing: &BillingService,
        platform: &str,
        owner_id: &str,
        api_key_id: Option<&str>,
    ) -> Option<Self> {
        (platform == "x").then(|| Self {
            db: db.clone(),
            billing: billing.clone(),
            owner_id: owner_id.to_owned(),
            bot_id: None,
            webhook_registered: false,
            api_key_id: api_key_id.map(str::to_owned),
            forwarded: std::sync::atomic::AtomicBool::new(false),
        })
    }

    async fn open(
        &self,
        ingress: BillingIngress,
        request_id: String,
    ) -> AppResult<MeteredProxyContext> {
        if !self.billing.billing_enabled() {
            return Ok(MeteredProxyContext::disabled());
        }
        let service = self
            .db
            .collection::<DownstreamService>(crate::models::downstream_service::COLLECTION_NAME)
            .find_one(doc! {"slug": "api-twitter", "is_active": true})
            .await?
            .ok_or_else(|| {
                AppError::BillingNotConfigured(
                    "Configure the X catalog service before using paid channels".into(),
                )
            })?;
        let owner = self
            .billing
            .owner_resolver()
            .resolve_for_execution(
                &self.owner_id,
                &self.owner_id,
                CredentialClass::NyxidPlatformOauthApp,
            )
            .await?;
        let route = BillingRouteContext::new(
            ingress,
            request_id,
            owner.owner_id,
            self.owner_id.clone(),
            self.api_key_id.clone(),
            None,
            Some(service.id.clone()),
            Some(service.slug.clone()),
            NodeIntent::Direct,
            "oauth2".into(),
            CredentialClass::NyxidPlatformOauthApp,
            super::billing::metric_resolution::platform_metric_for_request(&service, false),
            service.billing.as_ref(),
            false,
        );
        if route
            .platform_specs()
            .any(|(metric, _)| metric != BillingMetric::Requests)
        {
            return Err(AppError::BillingNotConfigured(
                "X channels require per-request pricing on the X service".into(),
            ));
        }
        self.billing.open_required(&route).await
    }

    pub async fn admit(&self) -> AppResult<()> {
        let meter = self
            .open(
                BillingIngress::ChannelInbound,
                format!("channel-admission:{}", uuid::Uuid::new_v4()),
            )
            .await?;
        self.billing
            .fail(&meter, "channel admission check; no provider event")
            .await
    }

    pub async fn received(&self, event_id: &str) -> AppResult<()> {
        if !self.billing.billing_enabled() {
            return Ok(());
        }
        let bot_id = self
            .bot_id
            .as_deref()
            .ok_or_else(|| AppError::Internal("X inbound billing requires a channel".into()))?;
        let request_id = format!("x-dm-received:{bot_id}:{event_id}");
        super::channel_connection_webhook_service::serialized(&self.db, &request_id, async {
            // A redelivery must not need fresh funds or the current catalog price.
            if self
                .db
                .collection::<bson::Document>(crate::models::usage_meter::COLLECTION_NAME)
                .find_one(doc! {"billing_request_id": &request_id, "status": "finalized"})
                .await?
                .is_some()
            {
                return Ok(());
            }
            let meter = self
                .open(BillingIngress::ChannelInbound, request_id.clone())
                .await?;
            self.billing.mark_forwarded(&meter).await?;
            self.settle(&meter, 1).await
        })
        .await
    }

    pub async fn send(&self, request: reqwest::RequestBuilder) -> AppResult<reqwest::Response> {
        if self.billing.billing_enabled() && !self.webhook_registered {
            return Err(AppError::BillingNotConfigured(
                "Paid X channels require webhook delivery; select Verify first".into(),
            ));
        }
        self.request(request, "x-dm-send").await
    }

    pub async fn verify_account(
        &self,
        request: reqwest::RequestBuilder,
    ) -> AppResult<reqwest::Response> {
        self.request(request, "x-account-verify").await
    }

    pub fn has_forwarded(&self) -> bool {
        self.forwarded.load(std::sync::atomic::Ordering::Relaxed)
    }

    async fn request(
        &self,
        request: reqwest::RequestBuilder,
        operation: &str,
    ) -> AppResult<reqwest::Response> {
        let meter = self
            .open(
                BillingIngress::ChannelOutbound,
                format!("{operation}:{}", uuid::Uuid::new_v4()),
            )
            .await?;
        self.billing.mark_forwarded(&meter).await?;
        self.forwarded
            .store(true, std::sync::atomic::Ordering::Relaxed);
        let response = request
            .timeout(std::time::Duration::from_secs(20))
            .send()
            .await
            .map_err(|_| {
                AppError::ChannelPlatformError(
                    "X request outcome is unknown; check account activity before retrying".into(),
                )
            })?;
        if response.status().is_success() {
            self.settle(&meter, 1).await?;
        } else if response.status().is_client_error() {
            // fail() only releases work that was never forwarded. A definitive
            // rejection settles zero usage and releases this forwarded hold.
            self.settle(&meter, 0).await?;
        }
        // Transport errors and server failures keep the forwarded reservation:
        // the provider may have performed a billable send before the failure.
        Ok(response)
    }

    async fn settle(&self, meter: &MeteredProxyContext, requests: i64) -> AppResult<()> {
        self.billing
            .settle_deferred(
                meter,
                PlatformUsage {
                    requests,
                    ..Default::default()
                },
                None,
                None,
            )
            .await
    }
}

pub fn require_webhooks(config: &crate::config::AppConfig, platform: &str) -> AppResult<()> {
    if config.billing_enabled && platform == "x" {
        return Err(AppError::BillingNotConfigured("Paid X channels require webhook delivery; configure X webhook credentials and select Verify".into()));
    }
    Ok(())
}

pub(crate) async fn suspend(
    state: &super::channel_inbound_service::InboundDeps<'_>,
    bot: &ChannelBot,
    adapter: &dyn super::channel_platform::PlatformAdapter,
) -> AppResult<()> {
    super::channel_credentials::fail_bot(state.db, bot, "Channel billing needs attention. Restore credits or billing configuration, then select Verify.").await?;
    cleanup(
        state.db,
        state.encryption_keys,
        state.http_client,
        adapter,
        bot,
    )
    .await
}

async fn cleanup(
    db: &mongodb::Database,
    keys: &crate::crypto::aes::EncryptionKeys,
    http: &reqwest::Client,
    adapter: &dyn super::channel_platform::PlatformAdapter,
    bot: &ChannelBot,
) -> AppResult<()> {
    super::channel_connection_webhook_service::remove_failed(db, keys, http, adapter, bot).await
}

pub fn blocks_channel(error: &AppError) -> bool {
    matches!(
        error,
        AppError::InsufficientCredits
            | AppError::WalletSuspended
            | AppError::BillingNotConfigured(_)
            | AppError::PlanEntitlementRequired(_)
            | AppError::BillingProviderUnavailable(_)
    )
}

pub async fn sweep(state: &crate::AppState) -> AppResult<()> {
    let bots: Vec<ChannelBot> = state
        .db
        .collection::<ChannelBot>(crate::models::channel_bot::COLLECTION_NAME)
        .find(doc! {"platform": "x", "status": "failed", "webhook_registered": true})
        .limit(100)
        .await?
        .try_collect()
        .await?;
    let adapter = super::channel_adapters::resolve_adapter("x", &state.token_exchange_cache)?;
    for bot in bots {
        if cleanup(
            &state.db,
            &state.encryption_keys,
            &state.http_client,
            adapter.as_ref(),
            &bot,
        )
        .await
        .is_err()
        {
            tracing::warn!(bot_id = %bot.id, "X subscription cleanup will retry");
        }
    }
    Ok(())
}

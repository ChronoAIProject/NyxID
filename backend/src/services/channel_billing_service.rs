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

pub const X_CHANNEL_SERVICE_SLUG: &str = "api-twitter";
pub const X_CHANNEL_PLATFORM: &str = "x";
pub const X_CHANNEL_DISPLAY_NAME: &str = "X";
pub const X_CHANNEL_CREDENTIAL_CLASS: CredentialClass = CredentialClass::NyxidPlatformOauthApp;

/// Billed X channel activity. Each variant is one operation price key on the
/// X catalog service; the account and post keys share their catalog operations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum XChannelOperation {
    AccountVerify,
    PostReply,
    DmSend,
    DmReceived,
    ChatReceived,
    PostReceived,
}

impl XChannelOperation {
    pub const ALL: [Self; 6] = [
        Self::AccountVerify,
        Self::PostReply,
        Self::DmSend,
        Self::DmReceived,
        Self::ChatReceived,
        Self::PostReceived,
    ];

    /// Operation price key.
    pub fn key(self) -> &'static str {
        match self {
            Self::AccountVerify => "get_me",
            Self::PostReply => "create_tweet",
            Self::DmSend => "channel_dm_send",
            Self::DmReceived => "channel_dm_received",
            Self::ChatReceived => "channel_chat_received",
            Self::PostReceived => "channel_post_received",
        }
    }

    /// Display label. The account and post keys are catalog operation names
    /// that other services share, so they keep their endpoint names everywhere;
    /// only channel-only operations get descriptive labels.
    pub fn label(self) -> &'static str {
        match self {
            Self::AccountVerify | Self::PostReply => self.key(),
            Self::DmSend => "Channel direct message sent",
            Self::DmReceived => "Channel direct message received",
            Self::ChatReceived => "Channel chat message received",
            Self::PostReceived => "Channel post received",
        }
    }

    /// Billing request ID prefix; existing meter identities depend on it.
    fn request_prefix(self) -> &'static str {
        match self {
            Self::AccountVerify => "x-account-verify",
            Self::PostReply => "x-post-reply",
            Self::DmSend => "x-dm-send",
            Self::DmReceived => "x-dm-received",
            Self::ChatReceived => "x-chat-received",
            Self::PostReceived => "x-post-received",
        }
    }
}

/// Channel operations a catalog service can price without an endpoint row,
/// as `(operation key, label)`.
pub fn declared_operations(slug: &str) -> Vec<(&'static str, &'static str)> {
    if slug != X_CHANNEL_SERVICE_SLUG {
        return Vec::new();
    }
    XChannelOperation::ALL
        .iter()
        .map(|operation| (operation.key(), operation.label()))
        .collect()
}

/// Display label for an operation price key on any service: a channel-only
/// operation's label, otherwise the endpoint name itself.
pub fn operation_label(key: &str) -> String {
    XChannelOperation::ALL
        .iter()
        .find(|operation| operation.key() == key)
        .map_or_else(|| key.to_owned(), |operation| operation.label().to_owned())
}

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
        (platform == X_CHANNEL_PLATFORM).then(|| Self {
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
        operation: XChannelOperation,
    ) -> AppResult<MeteredProxyContext> {
        if !self.billing.billing_enabled() {
            return Ok(MeteredProxyContext::disabled());
        }
        let service = self
            .db
            .collection::<DownstreamService>(crate::models::downstream_service::COLLECTION_NAME)
            .find_one(doc! {"slug": X_CHANNEL_SERVICE_SLUG, "is_active": true})
            .await?
            .ok_or_else(|| {
                AppError::BillingNotConfigured(
                    format!("Configure the {X_CHANNEL_DISPLAY_NAME} catalog service ({X_CHANNEL_SERVICE_SLUG}) before using paid channels"),
                )
            })?;
        let mut route = BillingRouteContext::new(
            ingress,
            request_id,
            self.owner_id.clone(),
            self.owner_id.clone(),
            self.api_key_id.clone(),
            None,
            Some(service.id.clone()),
            Some(service.slug.clone()),
            NodeIntent::Direct,
            "oauth2".into(),
            X_CHANNEL_CREDENTIAL_CLASS,
            super::billing::metric_resolution::platform_metric_for_request(&service, false),
            service.billing.as_ref(),
            false,
        )
        .with_operation(Some(operation.key()));
        if let Err(AppError::BillingNotConfigured(_)) = route.require_platform_price() {
            return Err(AppError::BillingNotConfigured(format!(
                "The X service ({X_CHANNEL_SERVICE_SLUG}) has no synced price on the Your own key (BYOK) lane that shared-app X channels bill through; configure a Requests price on that lane"
            )));
        }
        if route
            .platform_specs()
            .any(|(metric, _)| metric != BillingMetric::Requests)
        {
            return Err(AppError::BillingNotConfigured(format!(
                "{X_CHANNEL_DISPLAY_NAME} channels require per-request pricing on the {X_CHANNEL_DISPLAY_NAME} service ({X_CHANNEL_SERVICE_SLUG})"
            )));
        }
        let owner = self
            .billing
            .owner_resolver()
            .resolve_for_execution(&self.owner_id, &self.owner_id, X_CHANNEL_CREDENTIAL_CLASS)
            .await?;
        route.billing_owner_id = owner.owner_id;
        self.billing.open_required(&route).await
    }

    pub async fn admit(&self) -> AppResult<()> {
        let meter = self
            .open(
                BillingIngress::ChannelInbound,
                format!("channel-admission:{}", uuid::Uuid::new_v4()),
                XChannelOperation::DmReceived,
            )
            .await?;
        self.billing
            .fail(&meter, "channel admission check; no provider event")
            .await
    }

    pub async fn received(&self, event_id: &str) -> AppResult<()> {
        self.received_event(event_id, XChannelOperation::DmReceived)
            .await
    }

    pub async fn received_chat(&self, event_id: &str) -> AppResult<()> {
        self.received_event(event_id, XChannelOperation::ChatReceived)
            .await
    }

    pub async fn received_post(&self, post_id: &str) -> AppResult<()> {
        self.received_event(post_id, XChannelOperation::PostReceived)
            .await
    }

    async fn received_event(&self, event_id: &str, operation: XChannelOperation) -> AppResult<()> {
        if !self.billing.billing_enabled() {
            return Ok(());
        }
        let bot_id = self
            .bot_id
            .as_deref()
            .ok_or_else(|| AppError::Internal("X inbound billing requires a channel".into()))?;
        let request_id = format!("{}:{bot_id}:{event_id}", operation.request_prefix());
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
                .open(
                    BillingIngress::ChannelInbound,
                    request_id.clone(),
                    operation,
                )
                .await?;
            self.billing.mark_forwarded(&meter).await?;
            self.settle(&meter, 1).await
        })
        .await
    }

    pub async fn send(&self, request: reqwest::RequestBuilder) -> AppResult<reqwest::Response> {
        self.send_event(request, XChannelOperation::DmSend).await
    }

    pub async fn send_post(
        &self,
        request: reqwest::RequestBuilder,
    ) -> AppResult<reqwest::Response> {
        self.send_event(request, XChannelOperation::PostReply).await
    }

    async fn send_event(
        &self,
        request: reqwest::RequestBuilder,
        operation: XChannelOperation,
    ) -> AppResult<reqwest::Response> {
        if self.billing.billing_enabled() && !self.webhook_registered {
            return Err(AppError::BillingNotConfigured(
                "Paid X channels require webhook delivery; select Verify first".into(),
            ));
        }
        self.request(request, operation).await
    }

    pub async fn verify_account(
        &self,
        request: reqwest::RequestBuilder,
    ) -> AppResult<reqwest::Response> {
        self.request(request, XChannelOperation::AccountVerify)
            .await
    }

    pub fn has_forwarded(&self) -> bool {
        self.forwarded.load(std::sync::atomic::Ordering::Relaxed)
    }

    async fn request(
        &self,
        request: reqwest::RequestBuilder,
        operation: XChannelOperation,
    ) -> AppResult<reqwest::Response> {
        let meter = self
            .open(
                BillingIngress::ChannelOutbound,
                format!("{}:{}", operation.request_prefix(), uuid::Uuid::new_v4()),
                operation,
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
    super::channel_connection_webhook_service::remove_stopped(db, keys, http, adapter, bot).await
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
        .find(
            doc! {"platform": X_CHANNEL_PLATFORM, "webhook_registered": true,
            "$or": [{"status": "failed"}, {"is_active": false}]},
        )
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

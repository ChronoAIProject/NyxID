pub mod lago_client;
pub mod meter;
pub mod owner_resolver;
pub mod provisioning;
pub mod reconcile;
pub mod reservation;
pub mod route_context;
pub mod route_inventory;
pub mod webhook;

use std::sync::Arc;

use crate::config::AppConfig;
use crate::db::DbHandle;
use crate::errors::AppResult;
use crate::models::billing_wallet::{BillingWallet, COLLECTION_NAME as BILLING_WALLET};
use lago_client::{LagoApi, LagoClient};
use mongodb::bson::doc;

pub use meter::MeteredProxyContext;
pub use owner_resolver::BillingOwnerResolver;
pub use route_context::{BillingRouteContext, NodeIntent};
pub use route_inventory::BillingIngress;

#[derive(Clone)]
pub struct BillingService {
    db: DbHandle,
    config: Arc<AppConfig>,
    owner_resolver: BillingOwnerResolver,
    lago: Option<Arc<dyn LagoApi>>,
}

impl BillingService {
    pub fn new(db: DbHandle, config: Arc<AppConfig>) -> Self {
        let lago = match (&config.lago_api_url, &config.lago_api_key) {
            (Some(url), Some(key)) => match LagoClient::new(url.clone(), key.clone()) {
                Ok(client) => Some(Arc::new(
                    client.with_payment_provider_code(config.lago_payment_provider_code.clone()),
                ) as Arc<dyn LagoApi>),
                Err(error) => {
                    tracing::warn!(error = %error, "Lago billing client is not configured");
                    None
                }
            },
            _ => None,
        };

        Self {
            db: db.clone(),
            config,
            owner_resolver: BillingOwnerResolver::new(db),
            lago,
        }
    }

    #[cfg(test)]
    pub(crate) fn new_with_lago(
        db: DbHandle,
        config: Arc<AppConfig>,
        lago: Arc<dyn LagoApi>,
    ) -> Self {
        Self {
            db: db.clone(),
            config,
            owner_resolver: BillingOwnerResolver::new(db),
            lago: Some(lago),
        }
    }

    pub fn owner_resolver(&self) -> &BillingOwnerResolver {
        &self.owner_resolver
    }

    pub fn billing_enabled(&self) -> bool {
        self.config.billing_enabled
    }

    pub fn resale_enabled(&self) -> bool {
        self.config.billing_resale_enabled
    }

    pub fn lago_configured(&self) -> bool {
        self.lago.is_some()
    }

    pub fn lago_client(&self) -> Option<Arc<dyn LagoApi>> {
        self.lago.clone()
    }

    pub fn reconciler(&self) -> reconcile::BillingReconciler {
        reconcile::BillingReconciler::new(self.db.clone(), self.lago.clone(), self.config.clone())
    }

    pub async fn get_wallet(&self, owner_id: &str) -> AppResult<Option<BillingWallet>> {
        provisioning::get_wallet(&self.db, owner_id).await
    }

    pub async fn ensure_wallet(
        &self,
        owner_id: &str,
    ) -> AppResult<provisioning::ProvisionedWallet> {
        let lago = self.lago.as_deref().ok_or_else(|| {
            crate::errors::AppError::BillingNotConfigured(
                "Lago client is not configured".to_string(),
            )
        })?;
        provisioning::ensure_owner_wallet(
            &self.db,
            lago,
            owner_id,
            &self.config.lago_plan_code,
            self.config.billing_default_overdraft_cap_credits,
        )
        .await
    }

    pub async fn create_topup_checkout(
        &self,
        owner_id: &str,
        amount_credits: i64,
        idempotency_key: &str,
    ) -> AppResult<provisioning::TopUpCheckout> {
        let lago = self.lago.as_deref().ok_or_else(|| {
            crate::errors::AppError::BillingNotConfigured(
                "Lago client is not configured".to_string(),
            )
        })?;
        provisioning::create_topup_checkout(
            &self.db,
            lago,
            owner_id,
            &self.config.lago_plan_code,
            self.config.billing_default_overdraft_cap_credits,
            amount_credits,
            idempotency_key,
        )
        .await
    }

    pub async fn backfill_existing_owner_wallets(
        &self,
    ) -> AppResult<provisioning::BillingBackfillStats> {
        let lago = self.lago.as_deref().ok_or_else(|| {
            crate::errors::AppError::BillingNotConfigured(
                "Lago client is not configured".to_string(),
            )
        })?;
        provisioning::backfill_existing_owner_wallets(
            &self.db,
            lago,
            &self.config.lago_plan_code,
            self.config.billing_default_overdraft_cap_credits,
        )
        .await
    }

    pub async fn open(&self, ctx: &BillingRouteContext) -> AppResult<MeteredProxyContext> {
        let ctx = if self.config.billing_enabled {
            // Staged rollout: charging applies only to owners covered by the
            // billing feature flag. Everyone else is metered for
            // observability but never charged on either layer.
            let rollout_enabled = crate::services::feature_flag_service::billing_rollout_enabled(
                &self.db,
                &ctx.billing_owner_id,
                &ctx.actor_user_id,
            )
            .await?;
            // Platform charging is an admin opt-in per service: services
            // without billing.platform_billable stay free (metered only),
            // so BYOK and unconfigured services never draw from wallets.
            let platform_billable = if rollout_enabled && ctx.service_platform_billable {
                self.ensure_wallet_for_charging(&ctx.billing_owner_id)
                    .await?;
                self.owner_has_chargeable_wallet(&ctx.billing_owner_id)
                    .await?
            } else {
                false
            };
            let mut ctx = ctx.clone();
            if !rollout_enabled {
                ctx.resale = None;
            }
            ctx.with_platform_metering(platform_billable)
        } else {
            ctx.clone()
        };

        let reservation = if self.config.billing_enabled {
            reservation::gate_and_reserve(
                &self.db,
                self.lago.as_deref(),
                &ctx,
                self.config.billing_fail_closed,
                self.config.billing_rate_cache_ttl_secs,
            )
            .await?
        } else {
            None
        };
        meter::open(&self.db, &ctx, reservation.as_ref()).await
    }

    async fn ensure_wallet_for_charging(&self, owner_id: &str) -> AppResult<()> {
        if self.lago.is_none() {
            if self.config.billing_fail_closed {
                return Err(crate::errors::AppError::BillingNotConfigured(
                    "Lago client is not configured".to_string(),
                ));
            }
            tracing::warn!(
                owner_id,
                "Billing is enabled but Lago is not configured; continuing without wallet provisioning"
            );
            return Ok(());
        }

        self.ensure_wallet(owner_id).await.map(|_| ())
    }

    async fn owner_has_chargeable_wallet(&self, owner_id: &str) -> AppResult<bool> {
        let wallet = self
            .db
            .collection::<BillingWallet>(BILLING_WALLET)
            .find_one(doc! {
                "owner_id": owner_id,
                "lago_subscription_id": { "$type": "string", "$ne": "" },
            })
            .await?;
        Ok(wallet.is_some())
    }

    pub async fn mark_forwarded(&self, metered: &MeteredProxyContext) -> AppResult<()> {
        meter::mark_forwarded(&self.db, metered).await
    }

    pub async fn settle(
        &self,
        metered: &MeteredProxyContext,
        platform: crate::models::service_billing::PlatformUsage,
        resale: Option<crate::models::service_billing::ResaleUsage>,
        model: Option<String>,
    ) -> AppResult<()> {
        meter::settle(&self.db, metered, platform, resale, model).await
    }

    pub(crate) async fn settle_deferred(
        &self,
        metered: &MeteredProxyContext,
        platform: crate::models::service_billing::PlatformUsage,
        resale: Option<crate::models::service_billing::ResaleUsage>,
        model: Option<String>,
    ) -> AppResult<()> {
        let billing_request_id = metered
            .route
            .as_ref()
            .map(|route| route.billing_request_id.clone())
            .unwrap_or_else(|| "unknown".to_string());
        let persisted =
            meter::persist_settlement_intent(&self.db, metered, platform, resale, model).await?;
        let db = self.db.clone();
        tokio::spawn(async move {
            if meter::settle_persisted(&db, persisted).await.is_err() {
                tracing::warn!(
                    billing_request_id,
                    "Failed to settle usage meter row; durable retry recorded"
                );
            }
        });
        Ok(())
    }

    pub async fn fail(&self, metered: &MeteredProxyContext, reason: &str) -> AppResult<()> {
        meter::fail(&self.db, metered, reason).await
    }
}

#[cfg(test)]
mod tests {
    use async_trait::async_trait;
    use chrono::Utc;
    use mongodb::bson::doc;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use uuid::Uuid;

    use crate::errors::AppResult;
    use crate::models::billing_rate_cache::BillingRateCache;
    use crate::models::billing_wallet::{BillingWallet, CollectionState, PlanKind};
    use crate::models::service_billing::{BillingMetric, ServiceBilling};
    use crate::models::usage_meter::{BillingLayer, CredentialClass, UsageMeterRow};
    use crate::services::billing::lago_client::{
        Entitlement, LagoAck, LagoError, LagoEvent, LagoUsage, LagoWallet, OwnerProvisionInput,
    };
    use crate::services::billing::{
        BillingIngress, BillingRouteContext, BillingService, NodeIntent,
    };
    use crate::services::role_service;
    use crate::test_utils::{connect_test_database, test_app_config};

    #[tokio::test]
    async fn billing_disabled_keeps_metering_dark_for_wallet_charges() {
        let Some(db) = connect_test_database("billing_disabled_no_charge").await else {
            return;
        };
        let owner_id = "owner-dark-billing";
        insert_wallet(&db, owner_id).await;
        let service = BillingService::new(db.clone(), std::sync::Arc::new(test_app_config()));
        let billing = ServiceBilling {
            platform_billable: true,
            resale_billable: true,
            resale_metric: BillingMetric::Requests,
            lago_resale_metric_code: Some("resale_requests".to_string()),
        };
        let ctx = BillingRouteContext::new(
            BillingIngress::Proxy,
            Uuid::new_v4().to_string(),
            owner_id.to_string(),
            "actor-1".to_string(),
            None,
            Some("user-service-1".to_string()),
            Some("catalog-1".to_string()),
            Some("service-one".to_string()),
            NodeIntent::Direct,
            "bearer".to_string(),
            CredentialClass::NyxidManagedMaster,
            BillingMetric::Requests,
            Some(&billing),
            true,
        );

        let metered = service.open(&ctx).await.expect("open metering");
        assert!(metered.is_enabled());
        let wallet = db
            .collection::<BillingWallet>(crate::models::billing_wallet::COLLECTION_NAME)
            .find_one(doc! { "owner_id": owner_id })
            .await
            .expect("find wallet")
            .expect("wallet exists");
        let row = db
            .collection::<UsageMeterRow>(crate::models::usage_meter::COLLECTION_NAME)
            .find_one(doc! { "billing_owner_id": owner_id })
            .await
            .expect("find usage row")
            .expect("row exists");

        assert_eq!(wallet.reserved_credits, 0);
        assert_eq!(wallet.pending_lago_debits, 0);
        assert_eq!(row.reserved_credits, 0);
        assert!(row.wallet_id.is_none());
    }

    #[tokio::test]
    async fn billing_enabled_without_wallet_allows_uncharged_platform_metering() {
        let Some(db) = connect_test_database("billing_enabled_no_wallet_meter_only").await else {
            return;
        };
        let owner_id = "owner-no-wallet";
        let mut config = test_app_config();
        config.billing_enabled = true;
        let service = BillingService::new(db.clone(), std::sync::Arc::new(config));
        let ctx = BillingRouteContext::new(
            BillingIngress::Proxy,
            Uuid::new_v4().to_string(),
            owner_id.to_string(),
            "actor-1".to_string(),
            None,
            Some("user-service-1".to_string()),
            Some("catalog-1".to_string()),
            Some("service-one".to_string()),
            NodeIntent::Direct,
            "bearer".to_string(),
            CredentialClass::UserOwned,
            BillingMetric::Requests,
            None::<&ServiceBilling>,
            false,
        );

        let metered = service.open(&ctx).await.expect("open metering");
        assert!(metered.is_enabled());

        let row = db
            .collection::<UsageMeterRow>(crate::models::usage_meter::COLLECTION_NAME)
            .find_one(doc! { "billing_owner_id": owner_id })
            .await
            .expect("find usage row")
            .expect("row exists");

        assert_eq!(row.layer, BillingLayer::Platform);
        assert_eq!(row.reserved_credits, 0);
        assert!(row.wallet_id.is_none());
    }

    #[tokio::test]
    async fn billing_enabled_with_lago_auto_provisions_missing_wallet() {
        let Some(db) = connect_test_database("billing_auto_provision_on_open").await else {
            return;
        };
        role_service::seed_system_roles(&db)
            .await
            .expect("seed roles");
        insert_platform_rate(&db, 1).await;
        let owner = crate::services::auth_service::register_user(
            &db,
            "wallet-auto@example.com",
            "password123",
            Some("Wallet Auto"),
            None,
            true,
        )
        .await
        .expect("create owner");
        let mut config = test_app_config();
        config.billing_enabled = true;
        config.lago_plan_code = "starter".to_string();
        let lago = Arc::new(FakeLago::default());
        let service = BillingService::new_with_lago(db.clone(), Arc::new(config), lago.clone());
        let billable_billing = ServiceBilling {
            platform_billable: true,
            ..Default::default()
        };
        let ctx = BillingRouteContext::new(
            BillingIngress::Proxy,
            Uuid::new_v4().to_string(),
            owner.user_id.clone(),
            owner.user_id.clone(),
            None,
            Some("user-service-1".to_string()),
            Some("catalog-1".to_string()),
            Some("service-one".to_string()),
            NodeIntent::Direct,
            "bearer".to_string(),
            CredentialClass::UserOwned,
            BillingMetric::Requests,
            Some(&billable_billing),
            false,
        );

        let metered = service.open(&ctx).await.expect("open metering");
        assert!(metered.is_enabled());
        let wallet = db
            .collection::<BillingWallet>(crate::models::billing_wallet::COLLECTION_NAME)
            .find_one(doc! { "owner_id": &owner.user_id })
            .await
            .expect("find wallet")
            .expect("wallet exists");
        let row = db
            .collection::<UsageMeterRow>(crate::models::usage_meter::COLLECTION_NAME)
            .find_one(doc! { "billing_owner_id": &owner.user_id })
            .await
            .expect("find usage row")
            .expect("row exists");

        assert_eq!(wallet.lago_customer_id, owner.user_id);
        assert_eq!(
            wallet.lago_subscription_id.as_deref(),
            Some(format!("{}:starter", owner.user_id).as_str())
        );
        assert_eq!(
            wallet.lago_wallet_id.as_deref(),
            Some(format!("{}:wallet", owner.user_id).as_str())
        );
        assert_eq!(lago.wallet_creates.load(Ordering::SeqCst), 1);
        assert_eq!(row.layer, BillingLayer::Platform);
        assert_eq!(row.wallet_id.as_deref(), Some(wallet.id.as_str()));
    }

    #[tokio::test]
    async fn service_without_platform_billable_opts_out_of_charging() {
        let Some(db) = connect_test_database("billing_platform_opt_in").await else {
            return;
        };
        role_service::seed_system_roles(&db)
            .await
            .expect("seed roles");
        insert_platform_rate(&db, 1).await;
        let owner = crate::services::auth_service::register_user(
            &db,
            "wallet-optout@example.com",
            &format!("test-{}", Uuid::new_v4()),
            Some("Wallet Opt Out"),
            None,
            true,
        )
        .await
        .expect("create owner");
        let mut config = test_app_config();
        config.billing_enabled = true;
        config.lago_plan_code = "starter".to_string();
        let lago = Arc::new(FakeLago::default());
        let service = BillingService::new_with_lago(db.clone(), Arc::new(config), lago.clone());
        let ctx = BillingRouteContext::new(
            BillingIngress::Proxy,
            Uuid::new_v4().to_string(),
            owner.user_id.clone(),
            owner.user_id.clone(),
            None,
            Some("user-service-1".to_string()),
            Some("catalog-1".to_string()),
            Some("service-one".to_string()),
            NodeIntent::Direct,
            "bearer".to_string(),
            CredentialClass::UserOwned,
            BillingMetric::Requests,
            None::<&ServiceBilling>,
            false,
        );

        service.open(&ctx).await.expect("open metering");

        let wallet = db
            .collection::<BillingWallet>(crate::models::billing_wallet::COLLECTION_NAME)
            .find_one(doc! { "owner_id": &owner.user_id })
            .await
            .expect("query wallet");
        let row = db
            .collection::<UsageMeterRow>(crate::models::usage_meter::COLLECTION_NAME)
            .find_one(doc! { "billing_owner_id": &owner.user_id })
            .await
            .expect("find usage row")
            .expect("row exists");

        assert!(wallet.is_none(), "free services must not provision wallets");
        assert_eq!(lago.wallet_creates.load(Ordering::SeqCst), 0);
        assert_eq!(row.wallet_id, None, "free services must not hold credits");
    }

    #[tokio::test]
    async fn billing_rollout_flag_disabled_skips_charging() {
        let Some(db) = connect_test_database("billing_rollout_flag_gate").await else {
            return;
        };
        role_service::seed_system_roles(&db)
            .await
            .expect("seed roles");
        insert_platform_rate(&db, 1).await;
        let owner = crate::services::auth_service::register_user(
            &db,
            "wallet-rollout@example.com",
            &format!("test-{}", Uuid::new_v4()),
            Some("Wallet Rollout"),
            None,
            true,
        )
        .await
        .expect("create owner");
        // Simulate the production default: the rollout flag is off unless a
        // staff override targets the owner (test builds default it on).
        crate::services::feature_flag_service::set_platform_override(
            &db,
            crate::services::feature_flag_service::BILLING_FLAG_KEY,
            &crate::services::feature_flag_service::FlagTarget::Global,
            false,
            &owner.user_id,
        )
        .await
        .expect("disable billing flag globally");

        let mut config = test_app_config();
        config.billing_enabled = true;
        config.lago_plan_code = "starter".to_string();
        let lago = Arc::new(FakeLago::default());
        let service = BillingService::new_with_lago(db.clone(), Arc::new(config), lago.clone());
        let billable_billing = ServiceBilling {
            platform_billable: true,
            ..Default::default()
        };
        let ctx = BillingRouteContext::new(
            BillingIngress::Proxy,
            Uuid::new_v4().to_string(),
            owner.user_id.clone(),
            owner.user_id.clone(),
            None,
            Some("user-service-1".to_string()),
            Some("catalog-1".to_string()),
            Some("service-one".to_string()),
            NodeIntent::Direct,
            "bearer".to_string(),
            CredentialClass::UserOwned,
            BillingMetric::Requests,
            Some(&billable_billing),
            false,
        );

        service.open(&ctx).await.expect("open metering");

        let wallet = db
            .collection::<BillingWallet>(crate::models::billing_wallet::COLLECTION_NAME)
            .find_one(doc! { "owner_id": &owner.user_id })
            .await
            .expect("query wallet");
        assert!(
            wallet.is_none(),
            "owners outside the rollout must not be charged even on billable services"
        );
        assert_eq!(lago.wallet_creates.load(Ordering::SeqCst), 0);
    }

    async fn insert_platform_rate(db: &mongodb::Database, credits: i64) {
        db.collection::<BillingRateCache>(crate::models::billing_rate_cache::COLLECTION_NAME)
            .insert_one(BillingRateCache {
                id: BillingRateCache::cache_id("platform_requests", None),
                lago_metric_code: "platform_requests".to_string(),
                model: None,
                credits_per_unit_micros: credits * 1_000_000,
                synced_at: Utc::now(),
            })
            .await
            .expect("insert platform rate");
    }

    #[derive(Default)]
    struct FakeLago {
        wallet_creates: AtomicUsize,
    }

    #[async_trait]
    impl crate::services::billing::lago_client::LagoApi for FakeLago {
        async fn ensure_customer(&self, owner: &OwnerProvisionInput) -> AppResult<String> {
            Ok(owner.external_customer_id.clone())
        }

        async fn ensure_subscription(
            &self,
            customer_id: &str,
            plan_code: &str,
        ) -> AppResult<String> {
            Ok(format!("{customer_id}:{plan_code}"))
        }

        async fn ensure_wallet(&self, customer_id: &str) -> AppResult<LagoWallet> {
            self.wallet_creates.fetch_add(1, Ordering::SeqCst);
            Ok(LagoWallet {
                id: format!("{customer_id}:wallet"),
                balance_credits: 100,
            })
        }

        async fn record_event(&self, event: &LagoEvent) -> Result<LagoAck, LagoError> {
            Ok(LagoAck {
                transaction_id: event.transaction_id.clone(),
            })
        }

        async fn record_events_batch(
            &self,
            events: &[LagoEvent],
        ) -> Result<Vec<LagoAck>, LagoError> {
            Ok(events
                .iter()
                .map(|event| LagoAck {
                    transaction_id: event.transaction_id.clone(),
                })
                .collect())
        }

        async fn current_usage(
            &self,
            customer_id: &str,
            subscription_id: &str,
        ) -> AppResult<LagoUsage> {
            Ok(LagoUsage {
                customer_id: customer_id.to_string(),
                subscription_id: subscription_id.to_string(),
                raw: serde_json::json!({}),
            })
        }

        async fn wallet_balance(&self, _customer_id: &str) -> AppResult<i64> {
            Ok(100)
        }

        async fn entitlements(&self, _subscription_id: &str) -> AppResult<Vec<Entitlement>> {
            Ok(vec![Entitlement {
                code: "service-one".to_string(),
                raw: serde_json::json!({}),
            }])
        }
    }

    async fn insert_wallet(db: &mongodb::Database, owner_id: &str) {
        let now = Utc::now();
        db.collection::<BillingWallet>(crate::models::billing_wallet::COLLECTION_NAME)
            .insert_one(BillingWallet {
                id: format!("wallet-{owner_id}"),
                owner_id: owner_id.to_string(),
                lago_customer_id: owner_id.to_string(),
                lago_wallet_id: Some(format!("{owner_id}:wallet")),
                lago_subscription_id: Some(format!("{owner_id}:plan")),
                plan_kind: PlanKind::Prepaid,
                balance_credits: 100,
                reserved_credits: 0,
                pending_lago_debits: 0,
                has_payment_instrument: false,
                overdraft_cap_credits: 0,
                suspended: false,
                collection_state: CollectionState::Good,
                balance_synced_at: now,
                created_at: now,
                updated_at: now,
            })
            .await
            .expect("insert wallet");
    }
}

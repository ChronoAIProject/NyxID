use super::super::{BillingIngress, BillingRouteContext, NodeIntent};
use super::*;
use crate::models::downstream_service::{COLLECTION_NAME as SERVICES, DownstreamService};
use crate::models::service_billing::ServiceBilling;
use crate::models::usage_meter::CredentialClass;

impl BillingService {
    pub(super) fn require_sandbox_admission(&self) -> AppResult<&str> {
        if !self.config.cma_billing_enabled
            || !self.config.billing_enabled
            || self.config.billing_fail_closed
        {
            return Err(unavailable("CMA paid admission is disabled"));
        }
        let service = self
            .config
            .cma_billing_service_id
            .as_deref()
            .ok_or_else(|| unavailable("CMA catalog service is not configured"))?;
        uuid(service)?;
        Ok(service)
    }

    pub async fn quote_sandbox(
        &self,
        producer: &ProducerAuthority,
        owner_id: &str,
        sandbox: &str,
        allocation: SandboxAllocation,
    ) -> AppResult<SandboxBillingQuote> {
        let service_id = self.require_sandbox_admission()?;
        uuid(owner_id)?;
        sandbox_id(sandbox)?;
        super::super::sandbox_runtime_math::finite_quantities(units(&allocation), 300)
            .map_err(invalid)?;
        if allocation.cpu_millis <= 0 || allocation.memory_mib <= 0 || allocation.disk_gib <= 0 {
            return Err(invalid(
                "A Sandbox quote must specify its full resource profile",
            ));
        }
        super::super::exact_migration::require_ready(&self.db).await?;
        let service = self
            .db
            .collection::<DownstreamService>(SERVICES)
            .find_one(doc! {"_id":service_id,"is_active":true})
            .await?
            .ok_or_else(|| unavailable("CMA catalog service is unavailable"))?;
        let prices = selected_prices(&service.id, &service.slug, service.billing.as_ref())?;
        for price in &prices {
            let cache =
                super::super::reservation::find_rate(&self.db, &price.lago_metric_code, None)
                    .await?
                    .ok_or_else(|| unavailable("CMA service price cache is unavailable"))?;
            let rate = super::super::amounts::rate_pico(
                cache.credits_per_unit_pico,
                cache.credits_per_unit_micros,
            );
            if cache.retired_at.is_some()
                || cache.synced_at
                    < Utc::now()
                        - Duration::seconds(
                            i64::try_from(self.config.billing_rate_cache_ttl_secs)
                                .unwrap_or(i64::MAX),
                        )
                || rate != price.credits_per_unit.pico()
            {
                return Err(unavailable(
                    "CMA service price cache does not match its synchronized price",
                ));
            }
        }
        let wallet = self.sandbox_wallet(owner_id).await?;
        let now = Utc::now();
        let quote = SandboxBillingQuote {
            id: Uuid::new_v4().to_string(),
            producer_id: producer.id.clone(),
            owner_id: owner_id.into(),
            sandbox_id: sandbox.into(),
            service_id: service.id.clone(),
            service_slug: service.slug.clone(),
            wallet_id: wallet.id.clone(),
            lago_customer_id: wallet.lago_customer_id.clone(),
            lago_subscription_id: wallet.lago_subscription_id.clone().unwrap(),
            plan_code: self.config.lago_plan_code.clone(),
            maximum_allocation: allocation,
            prices,
            accepted_operation_id: None,
            created_at: now,
            accept_before: now + Duration::minutes(15),
            accepted_at: None,
        };
        self.verify_sandbox_admission_provider(&quote).await?;
        let mut session = self.db.client().start_session().await?;
        session
            .start_transaction()
            .write_concern(
                mongodb::options::WriteConcern::builder()
                    .w(mongodb::options::Acknowledgment::Majority)
                    .build(),
            )
            .await?;
        fence_producer(&self.db, &mut session, producer).await?;
        fence_owner(&self.db, &mut session, owner_id).await?;
        // Conflict with a concurrent catalog edit before publishing this quote.
        let touched=self.db.collection::<Document>(SERVICES).update_one(
            doc! {"_id":&service.id,"is_active":true,"updated_at":bson::DateTime::from_chrono(service.updated_at)},
            doc! {"$inc":{"sandbox_billing_revision":1_i64}}).session(&mut session).await?;
        if touched.matched_count != 1 {
            return Err(conflict("CMA service changed while quoting"));
        }
        self.db
            .collection::<SandboxBillingQuote>(QUOTE_COLLECTION)
            .insert_one(&quote)
            .session(&mut session)
            .await?;
        session.commit_transaction().await?;
        Ok(quote)
    }

    pub(super) async fn verify_sandbox_admission_provider(
        &self,
        quote: &SandboxBillingQuote,
    ) -> AppResult<()> {
        let lago = self
            .lago
            .as_deref()
            .ok_or_else(|| unavailable("Billing provider is unavailable"))?;
        verify_provider(lago, quote).await?;
        let entitlements = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            lago.entitlements(&quote.lago_subscription_id),
        )
        .await
        .map_err(|_| unavailable("Sandbox entitlement verification timed out"))??;
        let context = route_context(quote, String::new());
        if !super::super::reservation::is_entitled(&context, &entitlements) {
            return Err(AppError::PlanEntitlementRequired(
                "Sandbox service is not included in this subscription".into(),
            ));
        }
        Ok(())
    }

    pub(super) async fn sandbox_wallet(&self, owner_id: &str) -> AppResult<BillingWallet> {
        use crate::models::user::{COLLECTION_NAME, User};
        let owner = self
            .db
            .collection::<User>(COLLECTION_NAME)
            .find_one(
                doc! {"_id":owner_id,"is_active":true,"deletion_requested_at":bson::Bson::Null},
            )
            .await?
            .ok_or_else(|| AppError::Forbidden("Sandbox owner is unavailable".into()))?;
        if owner.user_type.is_org()
            || !crate::services::feature_flag_service::billing_recipient_rollout_enabled(
                &self.db, &owner,
            )
            .await?
        {
            return Err(unavailable("Sandbox owner is outside the billing rollout"));
        }
        let wallet = self
            .db
            .collection::<BillingWallet>(WALLETS)
            .find_one(doc! {"owner_id":owner_id})
            .await?
            .ok_or_else(|| unavailable("Sandbox billing requires an existing wallet"))?;
        if wallet.is_suspended()
            || wallet.collection_state != crate::models::billing_wallet::CollectionState::Good
        {
            return Err(AppError::WalletSuspended);
        }
        if wallet
            .lago_subscription_id
            .as_deref()
            .is_none_or(str::is_empty)
            || wallet.lago_wallet_id.as_deref().is_none_or(str::is_empty)
            || wallet.lago_customer_id.is_empty()
        {
            return Err(unavailable("Sandbox wallet is not fully provisioned"));
        }
        Ok(wallet)
    }
}

pub(super) fn selected_prices(
    service_id: &str,
    slug: &str,
    billing: Option<&ServiceBilling>,
) -> AppResult<Vec<SandboxPriceSnapshot>> {
    let billing = billing.ok_or_else(|| unavailable("CMA service pricing is missing"))?;
    let lane = billing
        .platform_key_pricing
        .as_ref()
        .ok_or_else(|| unavailable("CMA platform pricing is missing"))?;
    if lane.sync_status != PricingSyncStatus::Synced
        || lane.components.len() != 2
        || !lane.operations.is_empty()
        || lane
            .components
            .iter()
            .any(|c| c.sync_status != PricingSyncStatus::Synced)
    {
        return Err(unavailable(
            "CMA requires three synchronized allocation prices",
        ));
    }
    let context = BillingRouteContext::new(
        BillingIngress::Proxy,
        String::new(),
        String::new(),
        String::new(),
        None,
        None,
        Some(service_id.into()),
        Some(slug.into()),
        NodeIntent::Direct,
        "service_account".into(),
        CredentialClass::NyxidManagedMaster,
        BillingMetric::Requests,
        Some(billing),
        false,
    );
    let selected = context.platform_specs().collect::<Vec<_>>();
    let authored = std::iter::once((lane.metric, &lane.lago_metric_code, &lane.credits_per_unit))
        .chain(
            lane.components
                .iter()
                .map(|c| (c.metric, &c.lago_metric_code, &c.credits_per_unit)),
        )
        .collect::<Vec<_>>();
    let mut out = Vec::with_capacity(3);
    for metric in METRICS {
        let matching = authored
            .iter()
            .filter(|(m, _, _)| *m == metric)
            .collect::<Vec<_>>();
        if matching.len() != 1 {
            return Err(unavailable("CMA service allocation units are incomplete"));
        }
        let (_, code, price) = matching[0];
        if code.is_empty()
            || selected
                .iter()
                .filter(|(m, c)| *m == metric && *c == code.as_str())
                .count()
                != 1
        {
            return Err(unavailable(
                "CMA price selection differs from ordinary billing",
            ));
        }
        let normalized = super::super::pricing::normalize_price(price)?;
        let credits = normalized
            .parse::<Credits>()
            .map_err(|_| unavailable("CMA price is invalid"))?;
        if credits <= Credits::ZERO {
            return Err(unavailable(
                "Prepaid Sandbox prices must be positive; zero-price trials remain outside paid admission",
            ));
        }
        out.push(SandboxPriceSnapshot {
            metric,
            lago_metric_code: (*code).clone(),
            credits_per_unit: credits,
        });
    }
    Ok(out)
}

pub(super) async fn verify_provider(
    lago: &dyn super::super::lago_client::LagoApi,
    quote: &SandboxBillingQuote,
) -> AppResult<()> {
    let codes = quote
        .prices
        .iter()
        .map(|p| p.lago_metric_code.clone())
        .collect::<Vec<_>>();
    let snapshot = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        lago.sandbox_subscription(&quote.lago_subscription_id, &codes),
    )
    .await
    .map_err(|_| unavailable("Sandbox provider verification timed out"))??;
    if snapshot.subscription.external_id != quote.lago_subscription_id
        || snapshot.subscription.external_customer_id != quote.lago_customer_id
        || snapshot.subscription.plan_code != quote.plan_code
        || snapshot.subscription.status != "active"
    {
        return Err(unavailable(
            "Sandbox subscription no longer matches its accepted quote",
        ));
    }
    for price in &quote.prices {
        let matching = snapshot
            .rates
            .iter()
            .filter(|r| r.lago_metric_code == price.lago_metric_code)
            .collect::<Vec<_>>();
        if matching.len() != 1
            || super::super::amounts::rate_pico(
                matching[0].credits_per_unit_pico,
                matching[0].credits_per_unit_micros,
            ) != price.credits_per_unit.pico()
        {
            return Err(unavailable("Sandbox effective provider price changed"));
        }
    }
    Ok(())
}

pub(super) fn route_context(
    quote: &SandboxBillingQuote,
    request_id: String,
) -> BillingRouteContext {
    BillingRouteContext::new(
        BillingIngress::Proxy,
        request_id,
        quote.owner_id.clone(),
        quote.producer_id.clone(),
        None,
        None,
        Some(quote.service_id.clone()),
        Some(quote.service_slug.clone()),
        NodeIntent::Direct,
        "service_account".into(),
        CredentialClass::NyxidManagedMaster,
        METRICS[0],
        None,
        false,
    )
}

//! Quote-specific diagnostics over existing Billing. This does not authorize
//! admission, create accounts or reserve funds; submit repeats strict checks.
use super::*;
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct RuntimeQuotePreflight {
    pub profile_version: String,
    pub ready: bool,
    pub billing_enabled: bool,
    pub exact_ready: bool,
    pub wallet: &'static str,
    pub subscription: &'static str,
    pub provider: &'static str,
    pub pricing: &'static str,
    pub reason: Option<&'static str>,
    pub rate_version: Option<String>,
    pub credits_per_second: Option<Credits>,
    pub funds: Option<RuntimeFundsEstimate>,
    pub funds_reason: Option<&'static str>,
    pub observed_at: DateTime<Utc>,
    pub valid_until: DateTime<Utc>,
    pub reservation_guarantee: bool,
}

impl BillingService {
    pub async fn machine_runtime_preflight(
        &self,
        owner_id: &str,
        profile_version: &str,
        quote: Option<&MachineRuntimeQuote>,
    ) -> AppResult<RuntimeQuotePreflight> {
        crate::services::remote_machine_service::controlled_profile(profile_version)?;
        if quote.is_some_and(|quote| {
            quote.owner_id != owner_id || quote.terms.profile_version != profile_version
        }) {
            return Err(invalid("runtime preflight quote profile or owner mismatch"));
        }
        ensure_person(&self.db, owner_id, None).await?;
        let now = Utc::now();
        let enabled = self.config.billing_enabled && !self.config.billing_fail_closed;
        let exact_ready = super::super::exact_migration::ready(&self.db).await?;
        let wallet = self
            .db
            .collection::<BillingWallet>(WALLETS)
            .find_one(doc! {"owner_id":owner_id})
            .await?;
        let rate = if let Some(quote) = quote {
            // Existing quotes keep their exact rate, including after retirement.
            // Never substitute a newer price when the pinned contract is invalid.
            self.db
                .collection::<MachineRuntimeRate>(RATE_COLLECTION)
                .find_one(doc! {"_id": &quote.rate_version})
                .await?
                .filter(|rate| {
                    validate_quote_rate(quote, rate).is_ok()
                        && rate.plan_code == self.config.lago_plan_code
                        && rate.sync_status == PricingSyncStatus::Synced
                        && rate.verified_at.is_some()
                })
        } else {
            self.latest_machine_runtime_rate(profile_version).await?
        };
        let wallet_status = match &wallet {
            None => "missing",
            Some(wallet)
                if wallet.is_suspended()
                    || wallet.collection_state
                        != crate::models::billing_wallet::CollectionState::Good =>
            {
                "suspended"
            }
            Some(wallet)
                if wallet.lago_customer_id.is_empty()
                    || wallet.lago_wallet_id.as_deref().is_none_or(str::is_empty) =>
            {
                "unprovisioned"
            }
            _ => "ready",
        };
        let subscription = if wallet
            .as_ref()
            .and_then(|w| w.lago_subscription_id.as_deref())
            .is_none_or(str::is_empty)
        {
            "missing"
        } else {
            "unverified"
        };
        let mut view = RuntimeQuotePreflight {
            profile_version: profile_version.into(),
            ready: false,
            billing_enabled: enabled,
            exact_ready,
            wallet: wallet_status,
            subscription,
            provider: if self.lago.is_some() {
                "unverified"
            } else {
                "unconfigured"
            },
            pricing: if rate.as_ref().is_some_and(|r| {
                r.sync_status == PricingSyncStatus::Synced && r.verified_at.is_some()
            }) {
                "synced"
            } else {
                "unavailable"
            },
            reason: None,
            rate_version: rate.as_ref().map(|r| r.id.clone()),
            credits_per_second: rate.as_ref().map(|r| r.credits_per_second),
            funds: None,
            funds_reason: None,
            observed_at: now,
            valid_until: now + Duration::seconds(30),
            reservation_guarantee: false,
        };
        view.reason = if !enabled {
            Some("billing_unavailable")
        } else if !exact_ready {
            Some("exact_accounting_not_ready")
        } else if wallet_status != "ready" {
            Some("wallet_unavailable")
        } else if subscription == "missing" {
            Some("subscription_missing")
        } else if self.lago.is_none() {
            Some("provider_unconfigured")
        } else if view.pricing != "synced" {
            Some("profile_pricing_unavailable")
        } else {
            None
        };
        if view.reason.is_none() {
            let rate = rate.as_ref().expect("synced rate");
            match tokio::time::timeout(
                std::time::Duration::from_secs(10),
                self.runtime_prerequisites(owner_id, rate),
            )
            .await
            {
                Ok(Ok(_)) => {
                    view.ready = true;
                    view.provider = "ready";
                    view.subscription = "active";
                }
                Ok(Err(AppError::PlanEntitlementRequired(_))) => {
                    view.provider = "ready";
                    view.subscription = "not_entitled";
                    view.reason = Some("runtime_entitlement_required");
                }
                _ => {
                    view.provider = "unavailable";
                    view.reason = Some("subscription_or_provider_verification_failed");
                }
            }
        }
        if exact_ready && wallet_status == "ready" {
            if let Some(rate) = rate.as_ref() {
                match self.runtime_funds_estimate(owner_id, rate).await {
                    Ok(funds) => view.funds = Some(funds),
                    Err(_) => view.funds_reason = Some("funds_pending_or_unavailable"),
                }
            } else {
                view.funds_reason = Some("runtime_price_unavailable");
            }
        } else {
            view.funds_reason = Some("wallet_or_exact_accounting_unavailable");
        }
        Ok(view)
    }

    pub(super) async fn runtime_funds_estimate(
        &self,
        owner_id: &str,
        rate: &MachineRuntimeRate,
    ) -> AppResult<RuntimeFundsEstimate> {
        let wallet = self
            .db
            .collection::<BillingWallet>(WALLETS)
            .find_one(doc! {"owner_id":owner_id})
            .await?
            .ok_or_else(|| unavailable("runtime wallet missing"))?;
        if self
            .db
            .collection::<Document>(WALLETS)
            .find_one(doc! {"_id":&wallet.id,"active_settlement":{"$ne":bson::Bson::Null}})
            .projection(doc! {"_id":1})
            .await?
            .is_some()
        {
            return Err(unavailable("wallet settlement estimate is pending"));
        }
        let grants = super::super::grants::list_active_for_user(&self.db, owner_id, Utc::now())
            .await?
            .into_iter()
            .filter(|grant| {
                super::super::grants::service_scope_applies(&grant.scope, None, Some(SKU))
            })
            .collect::<Vec<_>>();
        let mut grant_funds = Credits::ZERO;
        for grant in grants {
            grant_funds =
                grant_funds.checked_add(super::super::grants::available_grant(&grant)?)?;
        }
        let mut allowance_seconds = 0_i64;
        for allowance in super::super::allowances::applicable_allowances(
            &self.db,
            owner_id,
            None,
            Some(SKU),
            BillingMetric::MachineRunningSeconds,
        )
        .await?
        {
            let window = super::super::allowances::allowance_window(
                allowance.recurrence,
                allowance.created_at,
                Utc::now(),
            );
            let period = self.db.collection::<crate::models::usage_allowance_period::UsageAllowancePeriod>(crate::models::usage_allowance_period::COLLECTION_NAME)
                .find_one(doc! {"_id":super::super::allowances::period_id(&allowance.id, owner_id, window.start)})
                .await?;
            let available = if let Some(period) = period {
                if period.active_settlement.is_some() {
                    return Err(unavailable("allowance settlement estimate is pending"));
                }
                allowance
                    .quantity
                    .checked_sub(period.consumed_quantity)
                    .and_then(|value| value.checked_sub(period.reserved_quantity))
                    .ok_or_else(|| invalid("allowance estimate overflow"))?
                    .max(0)
            } else {
                allowance.quantity.max(0)
            };
            allowance_seconds = allowance_seconds
                .checked_add(available)
                .ok_or_else(|| invalid("allowance estimate overflow"))?;
        }
        let money = wallet
            .available_credits()?
            .max(Credits::ZERO)
            .checked_add(grant_funds)?;
        let estimated = (money.pico() / rate.credits_per_second.pico())
            .checked_add(i128::from(allowance_seconds))
            .ok_or_else(|| invalid("runtime estimate overflow"))?;
        Ok(RuntimeFundsEstimate {
            wallet_available: wallet.available_credits()?,
            wallet_reserved: wallet.reserved_credits,
            pending_debits: wallet.pending_lago_debits,
            pending_expiry: wallet.pending_topup_expiry_credits,
            matching_allowance_seconds: allowance_seconds,
            available_grants: grant_funds,
            estimated_seconds: estimated.to_string(),
            shared_account_estimate_only: true,
        })
    }
}

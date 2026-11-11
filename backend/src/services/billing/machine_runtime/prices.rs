use super::super::lago_client::{LagoApi, ServicePriceSync};
use super::*;

impl BillingService {
    /// One new-sale selector shared by quotes and their Billing preflight.
    /// Historical absent fields belong only to the original fixed profile;
    /// explicit null and foreign profiles never enter this candidate set.
    pub async fn latest_machine_runtime_rate(
        &self,
        profile_version: &str,
    ) -> AppResult<Option<MachineRuntimeRate>> {
        crate::services::remote_machine_service::controlled_profile(profile_version)?;
        let mut filter = doc! {
            "plan_code": &self.config.lago_plan_code,
            "retired": false,
            "sync_status": "synced",
            "verified_at": {"$type": "date"},
        };
        if profile_version == HISTORICAL_PROFILE_VERSION {
            filter.insert(
                "$or",
                bson::bson!([
                    {"profile_version": profile_version},
                    {"profile_version": {"$exists": false}},
                ]),
            );
        } else {
            filter.insert("profile_version", profile_version);
        }
        let mut candidates = self
            .db
            .collection::<Document>(RATE_COLLECTION)
            .find(filter)
            .sort(doc! {"created_at": -1, "_id": -1})
            .max_time(std::time::Duration::from_secs(5))
            .await?;
        while let Some(candidate) = candidates.try_next().await? {
            if let Ok(rate) = bson::from_document::<MachineRuntimeRate>(candidate)
                && validate_rate(&rate).is_ok()
                && rate.profile_version == profile_version
            {
                return Ok(Some(rate));
            }
        }
        Ok(None)
    }

    pub async fn list_machine_runtime_rates(&self) -> AppResult<Vec<MachineRuntimeRate>> {
        self.db
            .collection::<MachineRuntimeRate>(RATE_COLLECTION)
            .find(doc! {})
            .sort(doc! {"created_at":-1,"_id":-1})
            .limit(100)
            .await?
            .try_collect()
            .await
            .map_err(Into::into)
    }

    /// Bounded, leased provider retry; historical rate versions are retained.
    /// No retry publishes a price without the ordinary provider readback.
    pub async fn retry_machine_runtime_rates(&self, limit: u32) -> AppResult<u64> {
        if !self.config.billing_enabled || self.lago.is_none() {
            return Ok(0);
        }
        let mut synced = 0;
        for _ in 0..limit.min(16) {
            let now = Utc::now();
            let claim = Uuid::new_v4().to_string();
            let row=self.db.collection::<Document>(RATE_COLLECTION).find_one_and_update(doc!{
                "retired":false,"sync_status":{"$in":["pending","failed"]},
                "$and":[{"$or":[{"sync_retry_at":null},{"sync_retry_at":{"$lte":bson::DateTime::from_chrono(now)}}]},
                    {"$or":[{"sync_claim_until":null},{"sync_claim_until":{"$lte":bson::DateTime::from_chrono(now)}}]}]
            },doc!{"$set":{"sync_claim":&claim,"sync_claim_until":bson::DateTime::from_chrono(now+Duration::minutes(2))}})
                .return_document(mongodb::options::ReturnDocument::After).await?;
            let Some(row) = row else { break };
            let id = row
                .get_str("_id")
                .map_err(|_| unavailable("invalid runtime rate identity"))?;
            let ok = self.sync_machine_runtime_rate(id).await.is_ok();
            let attempts = row
                .get_i64("sync_attempts")
                .unwrap_or(0)
                .saturating_add(1)
                .min(30);
            let delay = (30_i64.saturating_mul(1_i64 << attempts.min(7))).min(3600);
            self.db.collection::<Document>(RATE_COLLECTION).update_one(doc!{"_id":id,"sync_claim":&claim},doc!{
                "$set":{"sync_attempts":attempts,"sync_retry_at":bson::DateTime::from_chrono(now+Duration::seconds(delay))},
                "$unset":{"sync_claim":"","sync_claim_until":""}
            }).await?;
            if ok {
                synced += 1;
            }
        }
        Ok(synced)
    }

    /// Administrative configuration only. A draft never admits paid runtime.
    /// Nothing seeds a synchronized price at startup.
    pub async fn stage_initial_machine_runtime_rate(&self) -> AppResult<MachineRuntimeRate> {
        self.stage_machine_runtime_rate(
            nyxid_machine::managed::PROFILE,
            INITIAL_CREDITS_PER_SECOND.parse()?,
        )
        .await
    }

    pub async fn stage_machine_runtime_rate(
        &self,
        profile_version: &str,
        credits_per_second: Credits,
    ) -> AppResult<MachineRuntimeRate> {
        let profile = crate::services::remote_machine_service::controlled_profile(profile_version)?;
        if credits_per_second != INITIAL_CREDITS_PER_SECOND.parse()? {
            return Err(invalid(
                "The approved fixed runtime price is 0.000040 Credits per second",
            ));
        }
        let id = Uuid::new_v4().to_string();
        let row = MachineRuntimeRate {
            id: id.clone(),
            profile_version: profile.profile_version,
            metric: BillingMetric::MachineRunningSeconds,
            lago_metric_code: format!("remote_machine_running_seconds_{}", id.replace('-', "")),
            plan_code: self.config.lago_plan_code.clone(),
            credits_per_second,
            sync_status: PricingSyncStatus::Pending,
            retired: false,
            verified_at: None,
            created_at: Utc::now(),
        };
        self.db
            .collection::<MachineRuntimeRate>(RATE_COLLECTION)
            .insert_one(&row)
            .await?;
        Ok(row)
    }

    pub async fn sync_machine_runtime_rate(&self, rate_version: &str) -> AppResult<()> {
        let rate = self.runtime_rate(rate_version).await?;
        let lago = self
            .lago
            .as_deref()
            .ok_or_else(|| unavailable("billing provider is not configured"))?;
        let result = async {
            super::super::taxes::sync_charge(
                &self.db,
                lago,
                &rate.plan_code,
                &ServicePriceSync {
                    metric_code: rate.lago_metric_code.clone(),
                    metric_name: "Machine running seconds".into(),
                    metric_description:
                        "Fixed Remote Nyx Machine runtime; work disk and paused retention included"
                            .into(),
                    credits_per_unit: rate.credits_per_second.to_string(),
                },
            )
            .await?;
            verify_provider(lago, &rate).await
        }
        .await;
        let status = if result.is_ok() { "synced" } else { "failed" };
        let mut fields = doc! {"sync_status":status};
        if result.is_ok() {
            fields.insert("verified_at", bson::DateTime::from_chrono(Utc::now()));
        }
        self.db
            .collection::<Document>(RATE_COLLECTION)
            .update_one(doc! {"_id":&rate.id}, doc! {"$set":fields})
            .await?;
        result
    }

    /// Stop issuing new quotes. Keep the immutable charge and exact record for
    /// every accepted run and all later reconciliation; never remove the charge.
    pub async fn retire_machine_runtime_rate(&self, rate_version: &str) -> AppResult<()> {
        self.db
            .collection::<Document>(RATE_COLLECTION)
            .update_one(doc! {"_id":rate_version}, doc! {"$set":{"retired":true}})
            .await?;
        Ok(())
    }

    pub(super) async fn runtime_rate(&self, version: &str) -> AppResult<MachineRuntimeRate> {
        let rate = self
            .db
            .collection::<MachineRuntimeRate>(RATE_COLLECTION)
            .find_one(doc! {"_id":version})
            .await?
            .ok_or_else(|| unavailable("runtime price is not configured"))?;
        validate_rate(&rate)?;
        if rate.plan_code != self.config.lago_plan_code {
            return Err(unavailable("runtime price belongs to another billing plan"));
        }
        Ok(rate)
    }
    pub(super) async fn verify_runtime_provider_rate(
        &self,
        rate: &MachineRuntimeRate,
    ) -> AppResult<()> {
        if rate.sync_status != PricingSyncStatus::Synced || rate.verified_at.is_none() {
            return Err(unavailable("runtime price is not synchronized"));
        }
        verify_provider(
            self.lago
                .as_deref()
                .ok_or_else(|| unavailable("billing provider is not configured"))?,
            rate,
        )
        .await
    }
}

pub(super) fn validate_rate(rate: &MachineRuntimeRate) -> AppResult<()> {
    uuid(&rate.id)?;
    if crate::services::remote_machine_service::controlled_profile(&rate.profile_version).is_err()
        || rate.metric != BillingMetric::MachineRunningSeconds
        || rate.credits_per_second <= Credits::ZERO
        || rate.credits_per_second.pico() > i128::from(i64::MAX)
        || rate.plan_code.is_empty()
        || rate.lago_metric_code
            != format!(
                "remote_machine_running_seconds_{}",
                rate.id.replace('-', "")
            )
    {
        return Err(unavailable("runtime price is invalid"));
    }
    Ok(())
}
async fn verify_provider(lago: &dyn LagoApi, rate: &MachineRuntimeRate) -> AppResult<()> {
    validate_rate(rate)?;
    let plan = lago
        .plan_rates(&rate.plan_code)
        .await
        .map_err(|_| unavailable("runtime price provider is unavailable"))?;
    let matches: Vec<_> = plan
        .rates
        .iter()
        .filter(|p| p.lago_metric_code == rate.lago_metric_code)
        .collect();
    if plan.rejected.contains(&rate.lago_metric_code) || matches.len() != 1 {
        return Err(unavailable(
            "runtime price is missing or invalid at provider",
        ));
    }
    let provider = matches[0];
    let pico = provider
        .credits_per_unit_pico
        .map(i128::from)
        .unwrap_or(i128::from(provider.credits_per_unit_micros) * 1_000_000);
    if pico != rate.credits_per_second.pico() {
        return Err(unavailable(
            "runtime provider price changed; accepted rate cannot be fulfilled",
        ));
    }
    Ok(())
}

/// Reconciliation must never send an accepted quantity to a differently priced
/// provider charge. Machine usage returns the exact subscription verified on
/// the accepted plan; the caller must use it directly without resolving the
/// wallet again or using the ordinary service subscription cache. None is only
/// for ordinary service usage, whose existing resolution stays unchanged.
pub(in crate::services::billing) async fn verify_usage_provider_rate(
    db: &Database,
    lago: &dyn LagoApi,
    row: &UsageMeterRow,
) -> AppResult<Option<String>> {
    if row.metric != BillingMetric::MachineRunningSeconds {
        return Ok(None);
    }
    let rate = db
        .collection::<MachineRuntimeRate>(RATE_COLLECTION)
        .find_one(doc! {"lago_metric_code":&row.lago_metric_code})
        .await?
        .ok_or_else(|| unavailable("accepted runtime rate missing"))?;
    if row.funding.as_ref().and_then(|f| f.settlement_rate) != Some(rate.credits_per_second)
        || rate.sync_status != PricingSyncStatus::Synced
    {
        return Err(unavailable("accepted runtime settlement price mismatch"));
    }
    verify_provider(lago, &rate).await?;
    let wallet_id = row
        .wallet_id
        .as_deref()
        .filter(|id| !id.trim().is_empty())
        .ok_or_else(|| unavailable("runtime usage wallet binding missing"))?;
    let wallet = db
        .collection::<BillingWallet>(WALLETS)
        .find_one(doc! {"_id":wallet_id,"owner_id":&row.billing_owner_id})
        .await?
        .ok_or_else(|| unavailable("runtime usage wallet binding changed"))?;
    let subscription_id = wallet
        .lago_subscription_id
        .as_deref()
        .filter(|id| !id.trim().is_empty())
        .ok_or_else(|| unavailable("runtime usage subscription binding missing"))?;
    if wallet.lago_customer_id.trim().is_empty() {
        return Err(unavailable("runtime usage customer binding missing"));
    }
    let subscription = lago
        .runtime_subscription(subscription_id)
        .await
        .map_err(|_| unavailable("runtime usage subscription could not be verified"))?;
    if subscription.external_id != subscription_id
        || subscription.external_customer_id != wallet.lago_customer_id
        || subscription.plan_code != rate.plan_code
        || subscription.status != "active"
    {
        return Err(unavailable("runtime usage subscription binding changed"));
    }
    Ok(Some(subscription.external_id))
}

//! Prepaid number rental, explicitly separate from runtime and proxy contracts.
use super::{BillingService, funding, meter, reservation::LayerReservation};
use crate::{
    errors::{AppError, AppResult},
    models::{
        credits::Credits,
        downstream_service::{COLLECTION_NAME as SERVICES, DownstreamService},
        managed_comms::{ManagedCommsRole, RateSnapshot},
        service_billing::{BillingMetric, PricingSyncStatus},
        usage_meter::{
            BillingLayer, COLLECTION_NAME as USAGE, CredentialClass, UsageMeterRow, UsageStatus,
        },
    },
};
use chrono::{DateTime, Utc};
use mongodb::{
    ClientSession, Database,
    bson::{self, doc},
};
use sha2::{Digest, Sha256};

enum Candidates {
    Rental(funding::ManagedLifecycleFundingCandidates),
    Effect(funding::ManagedEffectFundingCandidates),
}
pub struct RentalFunding {
    candidates: Candidates,
    role: ManagedCommsRole,
    quantity: i64,
    wallet: crate::models::billing_wallet::BillingWallet,
    service: DownstreamService,
    pub rate: RateSnapshot,
    ttl: u64,
}
/// Only new lifecycle reservations use this gate. Committed rental recovery
/// deliberately has no dependency on managed rollout readiness.
pub(super) async fn require_ready(db: &Database) -> AppResult<()> {
    crate::services::security_readiness_service::require_serving(
        db,
        crate::services::feature_flag_service::MANAGED_LINES_FLAG_KEY,
    )
    .await
}
/// Recovery of a committed rental does not consult admission readiness. A
/// missing receipt defers only this row, preserving its holds and meter evidence.
pub(super) async fn coverage_receipt(
    db: &Database,
    row: &UsageMeterRow,
) -> AppResult<Option<crate::models::managed_comms::ManagedLinePeriod>> {
    use crate::models::managed_comms::{ManagedLinePeriod, ManagedRentalDiagnostic};
    let document = db.collection::<bson::Document>(ManagedLinePeriod::COLLECTION_NAME)
        .find_one(doc!{"billing_request_id":&row.billing_request_id,"usage_row_ids":&row.id,"funded":true})
        .await?;
    let code = if document.is_some() {
        "managed_rental_receipt_invalid"
    } else {
        "managed_rental_receipt_missing"
    };
    let period = document
        .and_then(|value| bson::from_document::<ManagedLinePeriod>(value).ok())
        .filter(|p| {
            p.quantity == 30
                && p.rate.metric == BillingMetric::PhoneNumberDays
                && p.covered_until - p.covered_from == chrono::Duration::days(30)
        });
    let now = Utc::now();
    let diagnostics = db.collection::<bson::Document>(ManagedRentalDiagnostic::COLLECTION_NAME);
    if period.is_some() {
        diagnostics
            .update_one(
                doc! {"usage_row_id":&row.id,"active":true},
                doc! {"$set":{"active":false,"resolved_at":bson::DateTime::from_chrono(now)}},
            )
            .await?;
    } else {
        diagnostics.update_one(doc!{"usage_row_id":&row.id},doc!{
            "$setOnInsert":{"_id":uuid::Uuid::new_v4().to_string(),"usage_row_id":&row.id,"billing_request_id":&row.billing_request_id,"detected_at":bson::DateTime::from_chrono(now)},
            "$set":{"code":code,"active":true,"resolved_at":bson::Bson::Null,"updated_at":bson::DateTime::from_chrono(now)}
        }).upsert(true).await?;
        let mut set = doc! {"updated_at":bson::DateTime::from_chrono(now),"last_error":code};
        if !row.released {
            set.insert("status", "failed");
            set.insert(
                "settlement_next_retry_at",
                bson::DateTime::from_chrono(now + chrono::Duration::minutes(5)),
            );
        }
        db.collection::<UsageMeterRow>(USAGE)
            .update_one(
                doc! {"_id":&row.id,"released":row.released},
                doc! {"$set":set},
            )
            .await?;
        tracing::warn!(usage_row_id=%row.id, "Managed rental deferred: coverage receipt unavailable");
    }
    Ok(period)
}
fn unavailable() -> AppError {
    AppError::BillingProviderUnavailable("Managed rental price or funding unavailable".into())
}
impl BillingService {
    /// Quotes require an active, synced, fresh platform price, including usage
    /// prices displayed with rental. No ordinary unsynced/free fallback applies.
    pub async fn managed_price(
        &self,
        service_id: &str,
        role: ManagedCommsRole,
        metric: BillingMetric,
    ) -> AppResult<RateSnapshot> {
        if !self.billing_enabled() || self.lago.is_none() {
            return Err(unavailable());
        }
        let service = self
            .db
            .collection::<DownstreamService>(SERVICES)
            .find_one(doc! {"_id":service_id,"is_active":true})
            .await?
            .ok_or_else(unavailable)?;
        if service.managed_comms_role != Some(role) {
            return Err(unavailable());
        }
        let billing = service.billing.as_ref().ok_or_else(unavailable)?;
        let lane = billing
            .platform_key_pricing
            .as_ref()
            .ok_or_else(unavailable)?;
        if billing.resale_billable
            || lane.metric != metric
            || lane.sync_status != PricingSyncStatus::Synced
            || !lane.components.is_empty()
            || !lane.operations.is_empty()
            || lane.lago_metric_code.is_empty()
        {
            return Err(unavailable());
        }
        let rate: Credits = lane.credits_per_unit.parse().map_err(|_| unavailable())?;
        if rate <= Credits::ZERO {
            return Err(unavailable());
        }
        let cached = super::reservation::fresh_rate(
            &self.db,
            &lane.lago_metric_code,
            None,
            self.config.billing_rate_cache_ttl_secs,
        )
        .await?;
        if super::amounts::rate_pico(cached.credits_per_unit_pico, cached.credits_per_unit_micros)
            != rate.pico()
        {
            return Err(unavailable());
        }
        // Cache refresh timestamps do not alter price consent. Exact lane and
        // catalog identity do; a removed/recreated price needs a fresh quote.
        let revision = hex::encode(Sha256::digest(
            serde_json::to_vec(&(&service.id, &service.slug, lane)).map_err(|_| unavailable())?,
        ));
        Ok(RateSnapshot {
            service_id: service.id,
            service_slug: service.slug,
            metric,
            rate,
            lago_metric_code: lane.lago_metric_code.clone(),
            revision,
        })
    }
    pub async fn prepare_managed_rental(
        &self,
        owner: &str,
        accepted: &RateSnapshot,
    ) -> AppResult<RentalFunding> {
        require_ready(&self.db).await?;
        super::exact_migration::require_ready(&self.db).await?;
        let rate = self
            .managed_price(
                &accepted.service_id,
                ManagedCommsRole::Rental,
                BillingMetric::PhoneNumberDays,
            )
            .await?;
        if rate.revision != accepted.revision
            || rate.rate != accepted.rate
            || rate.lago_metric_code != accepted.lago_metric_code
        {
            return Err(AppError::Conflict(
                "Managed rental price changed; accept a fresh quote".into(),
            ));
        }
        let wallet = self.ensure_wallet(owner).await?.wallet;
        // Benefits are untaxed. Refresh outside the transaction, then let the
        // shared reservation require a fresh profile only for a wallet share.
        let _ = super::taxes::ensure_fresh(
            &self.db,
            self.lago.as_deref().ok_or_else(unavailable)?,
            &wallet,
            self.config.billing_rate_cache_ttl_secs,
        )
        .await;
        let candidates = funding::prepare_managed_rental_funding(
            &self.db,
            owner,
            &rate.service_id,
            &rate.service_slug,
            Utc::now(),
        )
        .await?;
        let service = self
            .db
            .collection::<DownstreamService>(SERVICES)
            .find_one(doc! {"_id":&rate.service_id})
            .await?
            .ok_or_else(unavailable)?;
        let lane = service
            .billing
            .as_ref()
            .and_then(|b| b.platform_key_pricing.as_ref())
            .ok_or_else(unavailable)?;
        let revision = hex::encode(Sha256::digest(
            serde_json::to_vec(&(&service.id, &service.slug, lane)).map_err(|_| unavailable())?,
        ));
        if revision != rate.revision {
            return Err(unavailable());
        }
        Ok(RentalFunding {
            candidates: Candidates::Rental(candidates),
            role: ManagedCommsRole::Rental,
            quantity: 30,
            wallet,
            service,
            rate,
            ttl: self.config.billing_rate_cache_ttl_secs,
        })
    }
    pub async fn prepare_managed_effect(
        &self,
        owner: &str,
        service_id: &str,
        role: ManagedCommsRole,
    ) -> AppResult<RentalFunding> {
        require_ready(&self.db).await?;
        super::exact_migration::require_ready(&self.db).await?;
        let (metric, quantity) = match role {
            ManagedCommsRole::Voice | ManagedCommsRole::Premium => {
                (BillingMetric::VoiceSeconds, 1800)
            }
            ManagedCommsRole::Sms => (BillingMetric::Requests, 1),
            _ => return Err(unavailable()),
        };
        let rate = self.managed_price(service_id, role, metric).await?;
        let wallet = self.ensure_wallet(owner).await?.wallet;
        let _ = super::taxes::ensure_fresh(
            &self.db,
            self.lago.as_deref().ok_or_else(unavailable)?,
            &wallet,
            self.config.billing_rate_cache_ttl_secs,
        )
        .await;
        let candidates = funding::prepare_managed_effect_funding(
            &self.db,
            owner,
            service_id,
            &rate.service_slug,
            metric,
            quantity,
            Utc::now(),
        )
        .await?;
        let service = self
            .db
            .collection::<DownstreamService>(SERVICES)
            .find_one(doc! {"_id":service_id})
            .await?
            .ok_or_else(unavailable)?;
        let lane = service
            .billing
            .as_ref()
            .and_then(|b| b.platform_key_pricing.as_ref())
            .ok_or_else(unavailable)?;
        if hex::encode(Sha256::digest(
            serde_json::to_vec(&(&service.id, &service.slug, lane)).map_err(|_| unavailable())?,
        )) != rate.revision
        {
            return Err(unavailable());
        }
        Ok(RentalFunding {
            candidates: Candidates::Effect(candidates),
            role,
            quantity,
            wallet,
            service,
            rate,
            ttl: self.config.billing_rate_cache_ttl_secs,
        })
    }
    pub async fn settle_managed_rental(&self, row_id: &str) -> AppResult<()> {
        let row = self
            .db
            .collection::<UsageMeterRow>(USAGE)
            .find_one(doc! {"_id":row_id,"metric":"phone_number_days","status":"finalized"})
            .await?
            .ok_or_else(unavailable)?;
        meter::settle_persisted(&self.db, vec![row]).await
    }
}
impl RentalFunding {
    /// Caller must commit this row with its period and lifecycle receipt. No
    /// wallet/benefit holds can escape an aborted claim or renewal transaction.
    pub async fn reserve(
        &self,
        db: &Database,
        tx: &mut ClientSession,
        owner: &str,
        request: &str,
        now: DateTime<Utc>,
    ) -> AppResult<String> {
        if db.collection::<bson::Document>(SERVICES).update_one(doc!{"_id":&self.service.id,"is_active":true,"updated_at":bson::DateTime::from_chrono(self.service.updated_at),"billing":bson::to_bson(&self.service.billing).map_err(|_| unavailable())?,"managed_comms_role":bson::to_bson(&self.role).map_err(|_|unavailable())?},doc!{"$inc":{"managed_comms_fence":1_i64}}).session(&mut *tx).await?.matched_count != 1 { return Err(unavailable()); }
        let cache = db.collection::<crate::models::billing_rate_cache::BillingRateCache>(crate::models::billing_rate_cache::COLLECTION_NAME)
            .find_one(doc!{"_id":crate::models::billing_rate_cache::BillingRateCache::cache_id(&self.rate.lago_metric_code,None),"retired_at":bson::Bson::Null,"synced_at":{"$gte":bson::DateTime::from_chrono(now-chrono::Duration::seconds(i64::try_from(self.ttl).unwrap_or(i64::MAX)))}}).session(&mut *tx).await?.ok_or_else(unavailable)?;
        if super::amounts::rate_pico(cache.credits_per_unit_pico, cache.credits_per_unit_micros)
            != self.rate.rate.pico()
        {
            return Err(unavailable());
        }
        // Fence suspension/rebinding even when benefits cover the whole rent.
        if db.collection::<bson::Document>(crate::models::billing_wallet::COLLECTION_NAME).update_one(
            doc!{"_id":&self.wallet.id,"owner_id":owner,"lago_customer_id":&self.wallet.lago_customer_id,
                "lago_wallet_id":bson::to_bson(&self.wallet.lago_wallet_id).map_err(|_|unavailable())?,
                "lago_subscription_id":bson::to_bson(&self.wallet.lago_subscription_id).map_err(|_|unavailable())?,
                "collection_state":"good","suspended":{"$ne":true}},
            doc!{"$inc":{"managed_lifecycle_fence":1_i64}}).session(&mut *tx).await?.matched_count!=1{return Err(unavailable());}
        let pico = i64::try_from(self.rate.rate.pico()).map_err(|_| unavailable())?;
        let layer = LayerReservation {
            layer: BillingLayer::Platform,
            metric: self.rate.metric,
            lago_metric_code: self.rate.lago_metric_code.clone(),
            estimated_quantity: self.quantity,
            credits_per_unit_micros: pico / 1_000_000,
            credits_per_unit_pico: Some(pico),
            reserved_credits: Credits::ZERO,
            allowance_reservations: vec![],
            grant_reservations: vec![],
            tax_cache_ttl_secs: self.ttl,
            applied_tax: None,
        };
        let reserved = match &self.candidates {
            Candidates::Rental(c) => {
                funding::reserve_managed_rental_in_session(
                    db,
                    tx,
                    owner,
                    &self.wallet.id,
                    c,
                    layer,
                    now,
                )
                .await?
            }
            Candidates::Effect(c) => {
                funding::reserve_managed_effect_in_session(
                    db,
                    tx,
                    owner,
                    &self.wallet.id,
                    c,
                    layer,
                    now,
                )
                .await?
            }
        };
        let ctx = super::BillingRouteContext::new(
            super::BillingIngress::ChannelOutbound,
            request.into(),
            owner.into(),
            owner.into(),
            None,
            None,
            Some(self.service.id.clone()),
            Some(self.service.slug.clone()),
            super::NodeIntent::Direct,
            "none".into(),
            CredentialClass::NyxidManagedMaster,
            self.rate.metric,
            self.service.billing.as_ref(),
            false,
        );
        let mut row = meter::reserved_row(
            &ctx,
            BillingLayer::Platform,
            self.rate.metric,
            self.rate.lago_metric_code.clone(),
            Some(&reserved),
            None,
        );
        row.funding
            .as_mut()
            .ok_or_else(unavailable)?
            .settlement_rate = Some(self.rate.rate);
        // Effect rows retain their full reservation until provider evidence arrives.
        row.forwarded = true;
        row.status = UsageStatus::Forwarded;
        if self.role == ManagedCommsRole::Rental || self.role == ManagedCommsRole::Sms {
            row.status = UsageStatus::Finalized;
            row.quantity = Some(self.quantity);
            row.finalized_at = Some(now);
        }
        let id = row.id.clone();
        db.collection::<UsageMeterRow>(USAGE)
            .insert_one(row)
            .session(&mut *tx)
            .await?;
        Ok(id)
    }
}

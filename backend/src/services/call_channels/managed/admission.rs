//! Every admission contends with the same owner, eligibility, claim, line and account writes.
use super::{conflict, unavailable};
use crate::{
    errors::AppResult,
    models::{
        channel_adapter::{ACCOUNTS, CredentialBinding, ProviderAccount},
        managed_comms::*,
    },
};
use chrono::{DateTime, Utc};
use mongodb::{
    ClientSession, Database,
    bson::{self, doc},
};

#[derive(Clone, Debug)]
pub struct Admission {
    pub line: ManagedPhoneLine,
    pub claim: ManagedLineClaim,
    pub eligibility: ManagedCommsEligibility,
    pub account: ProviderAccount,
    pub period: ManagedLinePeriod,
}
impl Admission {
    fn validate(&self, owner: &str, generation: i64, now: DateTime<Utc>) -> AppResult<()> {
        if self.eligibility.revoked_at.is_some() {
            return Err(conflict("Managed line eligibility has been revoked"));
        }
        if self.eligibility.accepted_terms.as_ref() != Some(&self.eligibility.required_terms) {
            return Err(conflict(
                "Accept the current managed line terms before continuing",
            ));
        }
        if self.eligibility.revision != self.claim.eligibility_revision
            || self.claim.accepted_terms != self.eligibility.required_terms
        {
            return Err(conflict(
                "An administrator must apply the current eligibility revision to this claim",
            ));
        }
        if self.claim.user_id != owner
            || self.claim.managed_claim_generation != generation
            || self.claim.line_id != self.line.id
            || self.line.current_claim_id.as_deref() != Some(&self.claim.id)
            || self.line.state != LineState::Claimed
            || self.claim.state != LineState::Claimed
            || self.line.suspension.is_some()
            || self.claim.suspension.is_some()
            || (self.claim.voice_tier == VoiceTier::Premium && !self.eligibility.premium_allowed)
            || self.account.provider != self.line.provider
            || self.account.id != self.line.trusted_account_id
            || self.period.claim_id != self.claim.id
            || self.period.quantity != 30
            || self.period.covered_until - self.period.covered_from != chrono::Duration::days(30)
            || self.period.rate.metric
                != crate::models::service_billing::BillingMetric::PhoneNumberDays
            || self.period.rate.service_id != self.line.rental_billing_service_id
            || self.period.managed_claim_generation != generation
            || !self.period.funded
            || self.period.covered_from > now
            || self.period.covered_until <= now
            || self.period.usage_row_ids.is_empty()
        {
            return Err(unavailable());
        }
        Ok(())
    }
}

pub async fn read(
    db: &Database,
    owner: &str,
    credential: &CredentialBinding,
) -> AppResult<Admission> {
    let CredentialBinding::ManagedLine {
        line_id,
        managed_claim_id,
        managed_claim_generation,
    } = credential
    else {
        return Err(unavailable());
    };
    super::require_enabled(db, owner).await?;
    super::super::owner_admission::check(db, owner).await?;
    let line = db
        .collection::<ManagedPhoneLine>(ManagedPhoneLine::COLLECTION_NAME)
        .find_one(doc! {"_id":line_id})
        .await?
        .ok_or_else(unavailable)?;
    let claim = db
        .collection::<ManagedLineClaim>(ManagedLineClaim::COLLECTION_NAME)
        .find_one(doc! {"_id":managed_claim_id,"user_id":owner})
        .await?
        .ok_or_else(unavailable)?;
    let eligibility = super::eligibility::load(db, owner).await?;
    let account = db
        .collection::<ProviderAccount>(ACCOUNTS)
        .find_one(doc! {"_id":&line.trusted_account_id})
        .await?
        .ok_or_else(unavailable)?;
    let now = Utc::now();
    let period = db.collection::<ManagedLinePeriod>(ManagedLinePeriod::COLLECTION_NAME).find_one(doc!{"claim_id":managed_claim_id,"funded":true,"covered_from":{"$lte":bson::DateTime::from_chrono(now)},"covered_until":{"$gt":bson::DateTime::from_chrono(now)}}).await?.ok_or_else(unavailable)?;
    let result = Admission {
        line,
        claim,
        eligibility,
        account,
        period,
    };
    result.validate(owner, *managed_claim_generation, now)?;
    verify_funding(db, None, &result).await?;
    Ok(result)
}

/// Write-only conflict counter on claims, lines and accounts. `fence` is the
/// authority version: only writes that change claim, line or account authority
/// advance it, and admissions compare the value their read observed. Admissions
/// and other read-fences never advance `fence`; they write this counter instead,
/// so a concurrently committing authority write still conflicts with them and a
/// transient retry re-checks the unchanged (or changed) authority version.
pub(super) const ADMISSION_TOUCH: &str = "admission_touch";

/// Flags/readiness are checked by read before the caller opens its transaction,
/// and again immediately before dispatch. These conditional writes bind that read.
#[cfg_attr(
    not(test),
    allow(
        dead_code,
        reason = "M2 lifecycle and M4 effect admission consume this M1 transaction primitive"
    )
)]
pub async fn fence(
    db: &Database,
    tx: &mut ClientSession,
    a: &Admission,
    now: DateTime<Utc>,
) -> AppResult<()> {
    super::require_ready(db).await?;
    a.validate(&a.claim.user_id, a.claim.managed_claim_generation, now)?;
    super::super::owner_admission::fence(db, tx, &a.claim.user_id).await?;
    super::eligibility::fence(db, tx, &a.eligibility).await?;
    for (collection, filter) in [
        (
            ManagedLineClaim::COLLECTION_NAME,
            doc! {"_id":&a.claim.id,"state":"claimed","managed_claim_generation":a.claim.managed_claim_generation,"eligibility_revision":a.eligibility.revision,"fence":a.claim.fence},
        ),
        (
            ManagedPhoneLine::COLLECTION_NAME,
            doc! {"_id":&a.line.id,"state":"claimed","revision":a.line.revision,"fence":a.line.fence,"current_claim_id":&a.claim.id},
        ),
        (
            ACCOUNTS,
            doc! {"_id":&a.account.id,"revision":a.account.revision,"fence":super::custody::account_fence_value(a.account.fence),"catalog_service_id":&a.account.catalog_service_id},
        ),
    ] {
        let changed = db
            .collection::<bson::Document>(collection)
            .update_one(filter, doc! {"$inc":{ADMISSION_TOUCH:1_i64}})
            .session(&mut *tx)
            .await?;
        if changed.matched_count != 1 {
            return Err(conflict("Managed admission authority changed"));
        }
    }
    let period = db.collection::<ManagedLinePeriod>(ManagedLinePeriod::COLLECTION_NAME).find_one(doc!{"_id":&a.period.id,"funded":true,"covered_from":{"$lte":bson::DateTime::from_chrono(now)},"covered_until":{"$gt":bson::DateTime::from_chrono(now)}}).session(&mut *tx).await?.ok_or_else(unavailable)?;
    if period.usage_row_ids != a.period.usage_row_ids {
        return Err(unavailable());
    }
    verify_funding(db, Some(&mut *tx), a).await?;
    // Serialize catalog policy/destination changes with admission, without granting
    // the merchant generic access to the canonical credential service.
    let service = db.collection::<bson::Document>(crate::models::downstream_service::COLLECTION_NAME).update_one(doc!{"_id":&a.account.catalog_service_id,"is_active":true,"managed_comms_role":"credential","platform_key.enabled":{"$ne":true}},doc!{"$inc":{"managed_comms_fence":1_i64}}).session(&mut *tx).await?;
    if service.matched_count != 1 {
        return Err(unavailable());
    }
    Ok(())
}

/// A receipt cannot promise coverage when its durable funding rows are missing
/// or have been cancelled. M2 creates these records in one transaction.
async fn verify_funding(
    db: &Database,
    mut tx: Option<&mut ClientSession>,
    a: &Admission,
) -> AppResult<()> {
    let mut unique = std::collections::BTreeSet::new();
    for id in &a.period.usage_row_ids {
        if !unique.insert(id) {
            return Err(unavailable());
        }
        let rows = db.collection::<bson::Document>(crate::models::usage_meter::COLLECTION_NAME);
        let query=rows.find_one(doc!{"_id":id,"billing_request_id":&a.period.billing_request_id,"billing_owner_id":&a.claim.user_id,"metric":"phone_number_days","status":{"$in":["reserved","forwarded","finalized"]}}).projection(doc!{"_id":1});
        let found = if let Some(tx) = tx.as_deref_mut() {
            query.session(tx).await?
        } else {
            query.await?
        };
        if found.is_none() {
            return Err(unavailable());
        }
    }
    Ok(())
}

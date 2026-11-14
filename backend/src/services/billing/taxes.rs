//! Lago-owned tax configuration. Only wallet funding is taxable; benefit funding
//! never reaches Lago. Lago's cent rounding is reconciled by provider refreshes.
use super::{lago_client::LagoApi, reservation::LayerReservation};
use crate::{
    errors::{AppError, AppResult},
    models::{
        billing_tax_profile::{AppliedUsageTax, BillingTaxProfile, COLLECTION_NAME, TaxComponent},
        billing_wallet::BillingWallet,
        credits::{Credits, SCALE},
        usage_meter::UsageMeterRow,
    },
};
use chrono::{Duration, Utc};
use futures::TryStreamExt;
use mongodb::{
    ClientSession, Database,
    bson::{self, Document, doc},
};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

const INLINE_BACKOFF_SECS: i64 = 30;
const REFRESH_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);
// Both inline attempts share the five-second deadline. Allow one more second
// for publication or the existing provider-failure backoff write to complete.
const INLINE_COMPLETION_GRACE: std::time::Duration = std::time::Duration::from_secs(1);

mod ordering;
#[cfg(test)]
mod ordering_tests;

const PROFILE_REQUIRED: &str = "Fresh Lago tax configuration is required for wallet funding";
fn unavailable() -> AppError {
    AppError::BillingNotConfigured(PROFILE_REQUIRED.into())
}

/// Half-up at one picocredit for settlement; ceiling at one pico for admission.
/// Divide first to avoid overflowing on large valid credit amounts.
fn proportional(base: Credits, rate: Credits, round_up: bool) -> AppResult<Credits> {
    if base < Credits::ZERO || rate < Credits::ZERO {
        return Err(unavailable());
    }
    let denominator = 100 * SCALE;
    let whole = (base.pico() / denominator).checked_mul(rate.pico());
    let remainder = (base.pico() % denominator)
        .checked_mul(rate.pico())
        .ok_or_else(unavailable)?;
    let fraction = remainder / denominator;
    let residual = remainder % denominator;
    let increment = i128::from(if round_up {
        residual > 0
    } else {
        residual >= denominator / 2
    });
    Credits::from_pico(
        whole
            .and_then(|v| v.checked_add(fraction))
            .and_then(|v| v.checked_add(increment))
            .ok_or_else(unavailable)?,
    )
    .map_err(Into::into)
}
pub fn tax_half_up(base: Credits, rate: Credits) -> AppResult<Credits> {
    proportional(base, rate, false)
}
pub fn gross_ceiling(base: Credits, rate: Credits) -> AppResult<Credits> {
    Ok(base.checked_add(proportional(base, rate, true)?)?)
}

fn parse_taxes(value: &serde_json::Value) -> AppResult<Vec<TaxComponent>> {
    let taxes = value
        .get("taxes")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(unavailable)?;
    taxes
        .iter()
        .map(|tax| {
            let rate = tax.get("rate").ok_or_else(unavailable)?;
            let rate: Credits = rate
                .as_str()
                .map(str::to_owned)
                .unwrap_or_else(|| rate.to_string())
                .parse()
                .map_err(|_| unavailable())?;
            if rate < Credits::ZERO {
                return Err(unavailable());
            }
            Ok(TaxComponent {
                code: tax
                    .get("code")
                    .and_then(|v| v.as_str())
                    .ok_or_else(unavailable)?
                    .into(),
                name: tax
                    .get("name")
                    .and_then(|v| v.as_str())
                    .ok_or_else(unavailable)?
                    .into(),
                rate_percent: rate,
            })
        })
        .collect()
}

/// Lago v1.50 fees/apply_taxes_service.rb: first NONEMPTY scope wins;
/// taxes within that scope add. A 0% tax is nonempty and overrides inheritance.
pub fn resolve_config(
    owner: &str,
    subscription: &str,
    config: &super::lago_client::LagoTaxConfig,
) -> AppResult<BillingTaxProfile> {
    let entity = parse_taxes(&config.billing_entity)?;
    let customer = parse_taxes(&config.customer)?;
    let plan = parse_taxes(&config.plan)?;
    let default = [&plan, &customer, &entity]
        .into_iter()
        .find(|t| !t.is_empty())
        .cloned()
        .unwrap_or_default();
    let mut metrics: BTreeMap<String, Vec<TaxComponent>> = BTreeMap::new();
    for charge in config
        .plan
        .get("charges")
        .and_then(|v| v.as_array())
        .ok_or_else(unavailable)?
    {
        let code = charge
            .get("billable_metric_code")
            .and_then(|v| v.as_str())
            .ok_or_else(unavailable)?;
        let taxes = parse_taxes(charge)?;
        let taxes = if taxes.is_empty() {
            default.clone()
        } else {
            taxes
        };
        // Multiple fees can share a metric. Conservatively hold the largest
        // summed rate; provider refresh reconciles any resulting residue.
        let rate = Credits::checked_sum(taxes.iter().map(|t| t.rate_percent))?;
        let existing_rate = metrics
            .get(code)
            .map(|taxes| Credits::checked_sum(taxes.iter().map(|t| t.rate_percent)))
            .transpose()?;
        if existing_rate.is_none_or(|existing| rate > existing) {
            metrics.insert(code.into(), taxes);
        }
    }
    let fingerprint = hex::encode(Sha256::digest(
        serde_json::to_vec(&(&default, &metrics)).map_err(|e| AppError::Internal(e.to_string()))?,
    ));
    Ok(BillingTaxProfile {
        revision: 0,
        id: uuid::Uuid::new_v4().to_string(),
        owner_id: owner.into(),
        subscription_id: subscription.into(),
        plan_code: config
            .plan
            .get("code")
            .and_then(|v| v.as_str())
            .ok_or_else(unavailable)?
            .into(),
        billing_entity_code: config
            .customer
            .get("billing_entity_code")
            .and_then(|v| v.as_str())
            .ok_or_else(unavailable)?
            .into(),
        default_taxes: default,
        metric_taxes: metrics,
        fingerprint,
        fetched_at: Utc::now(),
        refresh_failed_at: None,
    })
}
pub fn snapshot(profile: &BillingTaxProfile, metric: &str) -> AppResult<AppliedUsageTax> {
    if profile.fingerprint.is_empty() {
        return Err(unavailable());
    }
    let taxes = profile
        .metric_taxes
        .get(metric)
        .unwrap_or(&profile.default_taxes)
        .clone();
    let rate_percent = Credits::checked_sum(taxes.iter().map(|t| t.rate_percent))?;
    Ok(AppliedUsageTax {
        taxes,
        rate_percent,
        fingerprint: profile.fingerprint.clone(),
    })
}
fn fresh(profile: &BillingTaxProfile, wallet: &BillingWallet, ttl: u64) -> bool {
    !profile.fingerprint.is_empty()
        && wallet.lago_subscription_id.as_deref() == Some(profile.subscription_id.as_str())
        && profile.fetched_at
            >= Utc::now() - Duration::seconds(ttl.min(i64::MAX as u64 / 1000) as i64)
}
async fn load_profile(db: &Database, owner: &str) -> AppResult<Option<BillingTaxProfile>> {
    Ok(db
        .collection::<BillingTaxProfile>(COLLECTION_NAME)
        .find_one(doc! {"owner_id":owner})
        .await?)
}

pub async fn refresh(
    db: &Database,
    lago: &dyn LagoApi,
    wallet: &BillingWallet,
    ttl: u64,
) -> AppResult<BillingTaxProfile> {
    let captured = load_profile(db, &wallet.owner_id).await?;
    if let Some(profile) = captured.as_ref().filter(|p| fresh(p, wallet, ttl)) {
        return Ok(profile.clone());
    }
    refresh_captured(db, lago, wallet, ttl, captured, None)
        .await?
        .ok_or_else(unavailable)
}

fn refresh_timeout() -> AppError {
    AppError::BillingProviderUnavailable("Lago tax configuration timed out".into())
}

async fn refresh_captured(
    db: &Database,
    lago: &dyn LagoApi,
    wallet: &BillingWallet,
    ttl: u64,
    captured: Option<BillingTaxProfile>,
    provider_deadline: Option<tokio::time::Instant>,
) -> AppResult<Option<BillingTaxProfile>> {
    let subscription = wallet
        .lago_subscription_id
        .as_deref()
        .ok_or_else(unavailable)?;
    let epoch = ordering::capture_plan_epoch(db).await?;
    #[cfg(test)]
    ordering_tests::point(ordering_tests::Phase::Captured).await;
    let deadline =
        provider_deadline.unwrap_or_else(|| tokio::time::Instant::now() + REFRESH_TIMEOUT);
    if tokio::time::Instant::now() >= deadline {
        // Capture used the remaining budget; no provider call failed.
        return Err(refresh_timeout());
    }
    let result = tokio::time::timeout_at(deadline, async {
        let config = lago
            .tax_config(&wallet.lago_customer_id, subscription)
            .await?;
        resolve_config(&wallet.owner_id, subscription, &config)
    })
    .await
    .unwrap_or_else(|_| Err(refresh_timeout()));
    match result {
        Ok(profile) => {
            #[cfg(test)]
            ordering_tests::point(ordering_tests::Phase::ProviderReturned).await;
            #[cfg(test)]
            ordering_tests::point(ordering_tests::Phase::BeforePublication).await;
            if let Some(published) =
                ordering::publish(db, wallet, captured.as_ref(), epoch, profile).await?
            {
                return Ok(Some(published));
            }
            // None is an ordering loss, distinct from a provider/DB error.
            // Only inline admission may recapture and try once more.
            Ok(load_profile(db, &wallet.owner_id)
                .await?
                .filter(|p| fresh(p, wallet, ttl)))
        }
        Err(error) => {
            // Empty fingerprint is an unverified cold-cache failure marker,
            // never a zero-tax profile. Preserve any previously verified data.
            let mut initial = bson::to_document(&BillingTaxProfile {
                revision: 0,
                id: uuid::Uuid::new_v4().to_string(),
                owner_id: wallet.owner_id.clone(),
                subscription_id: String::new(),
                plan_code: String::new(),
                billing_entity_code: String::new(),
                default_taxes: vec![],
                metric_taxes: BTreeMap::new(),
                fingerprint: String::new(),
                fetched_at: chrono::DateTime::UNIX_EPOCH,
                refresh_failed_at: None,
            })
            .map_err(|e| AppError::Internal(e.to_string()))?;
            initial.remove("refresh_failed_at");
            let collection = db.collection::<Document>(COLLECTION_NAME);
            let filter = doc! {"owner_id": &wallet.owner_id};
            let update = doc! {
                "$set": {"refresh_failed_at": bson::DateTime::from_chrono(Utc::now())},
                "$setOnInsert": initial,
            };
            match collection
                .update_one(filter.clone(), update.clone())
                .upsert(true)
                .await
            {
                Err(error) if super::ledger::is_duplicate_key_error(&error) => {
                    collection.update_one(filter, update).await?;
                }
                result => {
                    result?;
                }
            }
            Err(error)
        }
    }
}

#[cfg(test)]
async fn store_test_profile(db: &Database, profile: &BillingTaxProfile) -> AppResult<()> {
    let mut fields = bson::to_document(profile).map_err(|e| AppError::Internal(e.to_string()))?;
    fields.remove("_id");
    let collection = db.collection::<Document>(COLLECTION_NAME);
    let filter = doc! {"owner_id": &profile.owner_id};
    let update = doc! {"$set": fields, "$setOnInsert": {"_id": &profile.id}};
    match collection
        .update_one(filter.clone(), update.clone())
        .upsert(true)
        .await
    {
        Err(error) if super::ledger::is_duplicate_key_error(&error) => {
            collection.update_one(filter, update).await?;
        }
        result => {
            result?;
        }
    }
    Ok(())
}

pub async fn ensure_fresh(
    db: &Database,
    lago: &dyn LagoApi,
    wallet: &BillingWallet,
    ttl: u64,
) -> AppResult<BillingTaxProfile> {
    let provider_deadline = tokio::time::Instant::now() + REFRESH_TIMEOUT;
    let inline_refresh = async {
        let mut retry_wallet = None;
        for attempt in 0..2 {
            let current_wallet = retry_wallet.as_ref().unwrap_or(wallet);
            let captured = load_profile(db, &wallet.owner_id).await?;
            if let Some(profile) = &captured {
                if fresh(profile, current_wallet, ttl) {
                    return Ok(profile.clone());
                }
                if profile.refresh_failed_at.is_some_and(|failed| {
                    failed > Utc::now() - Duration::seconds(INLINE_BACKOFF_SECS)
                }) {
                    return Err(unavailable());
                }
            }
            if let Some(profile) = refresh_captured(
                db,
                lago,
                current_wallet,
                ttl,
                captured,
                Some(provider_deadline),
            )
            .await?
            {
                return Ok(profile);
            }
            if attempt == 0 {
                if tokio::time::Instant::now() >= provider_deadline {
                    return Err(refresh_timeout());
                }
                // Recapture the owner counter, but never silently switch the
                // request to a different wallet/provider binding.
                retry_wallet = Some(
                    db.collection::<BillingWallet>(crate::models::billing_wallet::COLLECTION_NAME)
                        .find_one(doc! {
                            "_id": &wallet.id,
                            "owner_id": &wallet.owner_id,
                            "lago_customer_id": &wallet.lago_customer_id,
                            "lago_subscription_id": &wallet.lago_subscription_id,
                            "lago_wallet_id": &wallet.lago_wallet_id,
                        })
                        .await?
                        .ok_or_else(unavailable)?,
                );
            }
        }
        // Two ordering losses are not a provider failure or a backoff signal.
        Err(unavailable())
    };
    tokio::time::timeout_at(provider_deadline + INLINE_COMPLETION_GRACE, inline_refresh)
        .await
        .unwrap_or_else(|_| Err(refresh_timeout()))
}

pub async fn reserve_gross_in_session(
    db: &Database,
    session: &mut ClientSession,
    wallet: &BillingWallet,
    layer: &mut LayerReservation,
    base: Credits,
) -> AppResult<Credits> {
    if base == Credits::ZERO {
        return Ok(base);
    }
    let profile = db
        .collection::<BillingTaxProfile>(COLLECTION_NAME)
        .find_one(doc! {"owner_id": &wallet.owner_id})
        .session(session)
        .await?
        .filter(|p| fresh(p, wallet, layer.tax_cache_ttl_secs))
        .ok_or_else(unavailable)?;
    let applied = snapshot(&profile, &layer.lago_metric_code)?;
    let gross = gross_ceiling(base, applied.rate_percent)?;
    layer.applied_tax = Some(applied);
    Ok(gross)
}
pub async fn settlement_tax(
    db: &Database,
    row: &UsageMeterRow,
    wallet: Credits,
) -> AppResult<Option<AppliedUsageTax>> {
    let Some(funding) = &row.funding else {
        return Ok(None);
    };
    if wallet == Credits::ZERO || !funding.tax_accounting {
        return Ok(None);
    }
    if let Some(applied) = &funding.applied_tax {
        return Ok(Some(applied.clone()));
    }
    // This read precedes commit_settlement's transaction. The claim-fenced
    // commit persists this snapshot with the split; stale workers reuse it.
    // Work is already done: use the latest verified profile regardless of age,
    // and let provider refresh reconcile drift rather than delay settlement.
    let profile = db
        .collection::<BillingTaxProfile>(COLLECTION_NAME)
        .find_one(doc! {"owner_id": &row.billing_owner_id})
        .await?
        .ok_or_else(unavailable)?;
    snapshot(&profile, &row.lago_metric_code).map(Some)
}
/// PUT replaces charges; invalidate after the provider attempt too, so an
/// inline reader racing the write cannot leave an old profile fresh afterward.
pub async fn sync_charge(
    db: &Database,
    lago: &dyn LagoApi,
    plan: &str,
    input: &super::lago_client::ServicePriceSync,
) -> AppResult<()> {
    invalidate_plan(db, plan).await?;
    let result = lago.sync_standard_charge(plan, input).await;
    invalidate_plan(db, plan).await?;
    result
}
pub async fn remove_charge(
    db: &Database,
    lago: &dyn LagoApi,
    plan: &str,
    code: &str,
) -> AppResult<()> {
    invalidate_plan(db, plan).await?;
    let result = lago.remove_standard_charge(plan, code).await;
    invalidate_plan(db, plan).await?;
    result
}
pub async fn invalidate_plan(db: &Database, plan: &str) -> AppResult<()> {
    ordering::invalidate_plan(db, plan).await
}
pub async fn invalidate_owner(db: &Database, owner: &str) -> AppResult<Option<BillingWallet>> {
    ordering::invalidate_owner(db, owner).await
}
pub(super) async fn repair_subscription(
    db: &Database,
    wallet: &BillingWallet,
    subscription: &str,
) -> AppResult<()> {
    ordering::repair_subscription(db, wallet, subscription).await
}
/// Missing profiles sort first, then oldest fetched_at. Bounded each sweep.
pub async fn reconcile(db: &Database, lago: &dyn LagoApi, ttl: u64) -> AppResult<()> {
    let cutoff = Utc::now() - Duration::seconds(ttl.min(i64::MAX as u64 / 1000) as i64);
    let rows: Vec<Document> = db
        .collection::<Document>(crate::models::billing_wallet::COLLECTION_NAME)
        .aggregate(vec![
            doc! {"$match": {"lago_subscription_id": {"$type": "string"}}},
            doc! {"$lookup": {"from": COLLECTION_NAME, "localField": "owner_id", "foreignField": "owner_id", "as": "tax_profile"}},
            doc! {"$set": {"tax_fetched_at": {"$ifNull": [{"$first": "$tax_profile.fetched_at"}, bson::DateTime::from_millis(0)]}}},
            doc! {"$match": {"tax_fetched_at": {"$lte": bson::DateTime::from_chrono(cutoff)}}},
            doc! {"$sort": {"tax_fetched_at": 1, "_id": 1}},
            doc! {"$limit": 100},
            doc! {"$unset": ["tax_profile", "tax_fetched_at"]},
        ])
        .await?
        .try_collect()
        .await?;
    // Tax reads must not hold up usage emission for an entire customer batch
    // during a provider outage. The next sweep resumes from oldest profiles.
    #[cfg(test)]
    ordering_tests::point(ordering_tests::Phase::SweepSelected).await;
    let total = rows.len();
    let mut attempted = 0;
    let mut completed = 0;
    let refresh_batch = async {
        for row in rows {
            if !super::reconcile::continue_reconcile() {
                break;
            }
            attempted += 1;
            let wallet: BillingWallet =
                bson::from_document(row).map_err(|e| AppError::Internal(e.to_string()))?;
            // Forced refresh bypasses the inline negative-cache backoff.
            if let Err(error) = refresh(db, lago, &wallet, ttl).await {
                tracing::warn!(owner_id = %wallet.owner_id, %error, "Lago tax profile refresh failed");
            }
            completed += 1;
        }
        AppResult::Ok(())
    };
    match tokio::time::timeout(std::time::Duration::from_secs(20), refresh_batch).await {
        Ok(result) => result?,
        Err(_) => tracing::warn!(
            attempted,
            completed,
            total,
            "Lago tax refresh batch timed out"
        ),
    }
    Ok(())
}

#[cfg(test)]
pub fn zero_snapshot() -> AppliedUsageTax {
    AppliedUsageTax {
        taxes: vec![],
        rate_percent: Credits::ZERO,
        fingerprint: "verified-test-zero".into(),
    }
}
#[cfg(test)]
pub fn zero_config() -> super::lago_client::LagoTaxConfig {
    super::lago_client::LagoTaxConfig {
        customer: serde_json::json!({"billing_entity_code":"default","taxes":[]}),
        plan: serde_json::json!({"code":"standard","taxes":[],"charges":[]}),
        billing_entity: serde_json::json!({"taxes":[]}),
    }
}
#[cfg(test)]
pub async fn seed_test_profiles(db: &Database) {
    // Explicit verified zero-tax fixtures; production never treats a missing
    // profile as zero. Direct funding tests still need a subscription identity.
    let wallets = db.collection::<BillingWallet>(crate::models::billing_wallet::COLLECTION_NAME);
    for mut wallet in wallets
        .find(doc! {})
        .await
        .unwrap()
        .try_collect::<Vec<_>>()
        .await
        .unwrap()
    {
        if wallet.lago_subscription_id.is_none() {
            wallet.lago_subscription_id = Some("test-subscription".into());
            wallets
                .update_one(
                    doc! {"_id":&wallet.id},
                    doc! {"$set":{"lago_subscription_id":"test-subscription"}},
                )
                .await
                .unwrap();
        }
        let profile = resolve_config(
            &wallet.owner_id,
            wallet.lago_subscription_id.as_deref().unwrap(),
            &zero_config(),
        )
        .unwrap();
        store_test_profile(db, &profile).await.unwrap();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    fn tax(code: &str, rate: &str) -> serde_json::Value {
        json!({"code":code,"name":code,"rate":rate})
    }
    #[test]
    fn duplicate_metric_charges_keep_highest_additive_rate_in_either_order() {
        for reverse in [false, true] {
            let mut config = zero_config();
            let mut charges = vec![
                json!({"billable_metric_code":"requests", "taxes":[tax("high", "9"), tax("local", "2.5")]}),
                json!({"billable_metric_code":"requests", "taxes":[tax("low", "5")]}),
                json!({"billable_metric_code":"requests", "taxes":[]}),
            ];
            if reverse {
                charges.reverse();
            }
            config.plan["charges"] = json!(charges);
            let applied = snapshot(
                &resolve_config("owner", "sub", &config).unwrap(),
                "requests",
            )
            .unwrap();
            assert_eq!(applied.rate_percent, "11.5".parse().unwrap());
            assert_eq!(applied.taxes.len(), 2);
        }
    }
    #[test]
    fn additive_percentages_and_pico_rounding() {
        let base: Credits = "1".parse().unwrap();
        for (rate, expected) in [("0", "0"), ("9", "0.09"), ("11.5", "0.115")] {
            assert_eq!(
                tax_half_up(base, rate.parse().unwrap()).unwrap(),
                expected.parse().unwrap()
            );
        }
        let pico = Credits::from_pico(1).unwrap();
        assert_eq!(
            tax_half_up(pico, "49".parse().unwrap()).unwrap(),
            Credits::ZERO
        );
        assert_eq!(tax_half_up(pico, "50".parse().unwrap()).unwrap(), pico);
        assert_eq!(
            gross_ceiling(pico, "0.000000000001".parse().unwrap())
                .unwrap()
                .pico(),
            2
        );
        let huge = Credits::from_pico(crate::models::credits::MAX_PICO).unwrap();
        assert!(tax_half_up(huge, "9".parse().unwrap()).is_ok());
        assert!(tax_half_up(base, "-1".parse().unwrap()).is_err());
    }
    #[test]
    fn precedence_addition_explicit_zero_and_inheritance() {
        let mut config = zero_config();
        config.billing_entity["taxes"] = json!([tax("entity", "8")]);
        config.customer["taxes"] = json!([tax("gst", "9"), tax("local", "2.5")]);
        config.plan["charges"] = json!([{"billable_metric_code":"requests","taxes":[]}]);
        let profile = resolve_config("owner", "sub", &config).unwrap();
        assert_eq!(
            snapshot(&profile, "requests").unwrap().rate_percent,
            "11.5".parse().unwrap()
        );
        config.plan["taxes"] = json!([tax("zero", "0")]);
        assert_eq!(
            snapshot(
                &resolve_config("owner", "sub", &config).unwrap(),
                "requests"
            )
            .unwrap()
            .rate_percent,
            Credits::ZERO
        );
        config.plan["charges"][0]["taxes"] = json!([tax("charge", "5")]);
        let profile = resolve_config("owner", "sub", &config).unwrap();
        assert_eq!(
            snapshot(&profile, "requests").unwrap().rate_percent,
            "5".parse().unwrap()
        );
        assert_eq!(
            snapshot(&profile, "new-metric").unwrap().rate_percent,
            Credits::ZERO
        );
        config.plan["taxes"] = json!([]);
        config.customer["taxes"] = json!([]);
        assert_eq!(
            snapshot(
                &resolve_config("owner", "sub", &config).unwrap(),
                "new-metric"
            )
            .unwrap()
            .rate_percent,
            "8".parse().unwrap()
        );
        config.customer["taxes"] = json!([tax("bad", "0.0000000000001")]);
        assert!(resolve_config("owner", "sub", &config).is_err());
    }
}
